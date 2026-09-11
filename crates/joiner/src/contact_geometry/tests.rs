// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

use exedra_assembly::{CompilePolicy, PartCompiler};
use exedra_constructive::builders::{circle, rect};
use exedra_constructive::ir::{CapMode, CsgOp, NodeKind, Placement3, Recipe, RecipeBuilder};

use crate::{Anchor, Evidence, EvidenceClass, OrientedBox, Part, lower, measure_contact};

const TOLERANCE: f64 = 0.0002;

enum SupportShape {
    Full,
    Narrow,
    Holed,
    Gap,
}

struct Fixture {
    construction: Construction,
    contact: ContactPatch,
    carried: CompiledPart,
    carrier: CompiledPart,
}

impl Fixture {
    fn measure(&self) -> Result<ContactGeometryMeasurement, ContactGeometryError> {
        measure_contact_geometry(
            &self.construction,
            &self.contact,
            &self.carried,
            &self.carrier,
            TOLERANCE,
        )
    }
}

fn fixture(seat: bool, support: SupportShape) -> Fixture {
    let depth = if seat { 0.02 } else { 0.0 };
    let evidence = Evidence::new("fixture", EvidenceClass::ModernEngineeringInference);
    let mut construction = Construction::new();
    construction
        .add_element(
            Element::new(
                "purlin",
                "purlin",
                "wood",
                OrientedBox::axis_aligned([0.0, 0.0, 0.2 - depth], [1.0, 0.2, 0.2]),
                evidence.clone(),
            )
            .with_part(Part::new(purlin(seat))),
        )
        .unwrap();
    construction
        .add_element(
            Element::new(
                "support",
                "beam",
                "wood",
                OrientedBox::axis_aligned([0.4, 0.0, 0.0], [0.2; 3]),
                evidence.clone(),
            )
            .with_part(Part::new(beam(support))),
        )
        .unwrap();
    // At depth 20 mm a circle of radius 100 mm has a 120 mm chord.
    // An uncut circle is offered the same finite claim to expose the OBB error.
    let contact = ContactPatch::new(
        "seat",
        Anchor::new("purlin", [0.5, 0.1, depth]),
        Anchor::new("support", [0.1, 0.1, 0.2]),
        [0.0, 0.0, 1.0],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        ContactMeaning::Bearing,
        evidence,
    )
    .with_footprint_meters([0.2, 0.12])
    .with_minimum_overlap_meters([0.1, 0.08]);
    let assembly = lower(&construction).unwrap();
    let mut policy = CompilePolicy::default();
    policy.evaluation.discretize.chord_tolerance = 0.00002;
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &policy)
        .unwrap();
    Fixture {
        construction,
        contact,
        carried: (*compiled.parts()[0]).clone(),
        carrier: (*compiled.parts()[1]).clone(),
    }
}

fn purlin(seat: bool) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let slot = builder.material_slot("surface");
    let profile = builder.add_profile(circle(0.1).unwrap());
    let cylinder = builder
        .with_material(slot)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::from_axes(
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
                [0.0, 0.1, 0.1],
            ),
            height: 1.0,
            caps: CapMode::Both,
        })
        .unwrap();
    if !seat {
        return builder.finish(cylinder).unwrap();
    }
    let cutter_profile = builder.add_profile(rect(0.2, 0.4).unwrap());
    let cutter = builder
        .with_material(slot)
        .add(NodeKind::Extrude {
            profile: cutter_profile,
            placement: Placement3::translate(0.4, -0.1, -0.1),
            height: 0.12,
            caps: CapMode::Both,
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![cylinder, cutter],
        })
        .unwrap();
    builder.finish(root).unwrap()
}

fn beam(shape: SupportShape) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let slot = builder.material_slot("surface");
    let (width, offset) = if matches!(shape, SupportShape::Narrow) {
        (0.08, 0.06)
    } else {
        (0.2, 0.0)
    };
    let profile = builder.add_profile(rect(0.2, width).unwrap());
    let z = if matches!(shape, SupportShape::Gap) {
        -0.002
    } else {
        0.0
    };
    let block = builder
        .with_material(slot)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::translate(0.0, offset, z),
            height: 0.2,
            caps: CapMode::Both,
        })
        .unwrap();
    if !matches!(shape, SupportShape::Holed) {
        return builder.finish(block).unwrap();
    }
    let hole = builder.add_profile(circle(0.01).unwrap());
    let cutter = builder
        .with_material(slot)
        .add(NodeKind::Extrude {
            profile: hole,
            placement: Placement3::translate(0.1, 0.1, -0.1),
            height: 0.4,
            caps: CapMode::Both,
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![block, cutter],
        })
        .unwrap();
    builder.finish(root).unwrap()
}

#[test]
fn tangent_cylinder_does_not_fill_the_bearing_area_its_bounds_claim() {
    let fixture = fixture(false, SupportShape::Full);
    let bounds = measure_contact(&fixture.construction, &fixture.contact).unwrap();
    assert!(bounds.anchors_coincide());
    assert!(bounds.overlap[0] >= 0.19 && bounds.overlap[1] >= 0.19);
    let surface = fixture.measure().unwrap();
    assert!(!surface.is_covered());
    // The distance tolerance admits a thin strip of near-tangent facets, but
    // it cannot turn them into the claimed 120 mm wide bearing surface.
    assert!(surface.carried_uncovered_area > 0.8 * surface.checked_area);
    assert_eq!(surface.carrier_uncovered_area, 0.0);
}

#[test]
fn a_cut_seat_has_finite_surface_coverage_on_both_participants() {
    let fixture = fixture(true, SupportShape::Full);
    let surface = fixture.measure().unwrap();
    assert!(surface.is_covered(), "{surface:?}");
    assert!((surface.checked_area - 0.1996 * 0.1196).abs() < 1e-12);
}

#[test]
fn gaps_partial_support_and_interior_holes_are_not_bearing() {
    for support in [SupportShape::Gap, SupportShape::Narrow, SupportShape::Holed] {
        let fixture = fixture(true, support);
        let surface = fixture.measure().unwrap();
        assert!(!surface.is_covered(), "{surface:?}");
        assert_eq!(surface.carried_uncovered_area, 0.0);
        assert!(surface.carrier_uncovered_area > 0.0002);
    }
}

#[test]
fn duplicate_triangles_cannot_cover_a_hole() {
    let mut fixture = fixture(true, SupportShape::Holed);
    let before = fixture.measure().unwrap();
    fixture
        .carrier
        .bodies
        .extend(fixture.carrier.bodies.clone());
    let after = fixture.measure().unwrap();
    assert!(!after.is_covered());
    assert!((after.carrier_uncovered_area - before.carrier_uncovered_area).abs() < 1e-12);
}

#[test]
fn missing_and_omitted_geometry_cannot_be_verified() {
    let mut fixture = fixture(true, SupportShape::Full);
    fixture
        .construction
        .set_element_present("support", false)
        .unwrap();
    assert_eq!(fixture.measure(), Err(ContactGeometryError::InvalidContact));
    fixture
        .construction
        .set_element_present("support", true)
        .unwrap();
    fixture.carrier.bodies.clear();
    assert_eq!(
        fixture.measure(),
        Err(ContactGeometryError::MissingGeometry)
    );
}

#[test]
fn rotated_and_reflected_frames_preserve_surface_coverage() {
    for reflection in [1.0, -1.0] {
        let mut fixture = fixture(true, SupportShape::Full);
        let axes = [[0.8, 0.0, 0.6], [0.0, reflection, 0.0], [-0.6, 0.0, 0.8]];
        let transform = |point: Vec3| {
            core::array::from_fn(|i| {
                axes[0][i] * point[0] + axes[1][i] * point[1] + axes[2][i] * point[2]
            })
        };
        for key in ["purlin", "support"] {
            let mut extent = fixture.construction.element(key).unwrap().extent.clone();
            extent.origin = transform(extent.origin);
            extent.axes = axes;
            fixture
                .construction
                .set_element_extent(key, extent)
                .unwrap();
        }
        fixture.contact.normal = transform(fixture.contact.normal);
        fixture.contact.tangents = fixture.contact.tangents.map(transform);
        let surface = fixture.measure().unwrap();
        assert!(surface.is_covered(), "reflection={reflection}: {surface:?}");
    }
}

#[test]
fn invalid_tolerances_are_errors_and_actual_plane_gaps_respect_tolerance() {
    let mut fixture = fixture(true, SupportShape::Full);
    for tolerance in [0.0, -0.001, f64::NAN, 0.06] {
        assert_eq!(
            measure_contact_geometry(
                &fixture.construction,
                &fixture.contact,
                &fixture.carried,
                &fixture.carrier,
                tolerance,
            ),
            Err(ContactGeometryError::InvalidTolerance),
        );
    }
    let original = fixture.carrier.clone();
    for (shift, covered) in [(0.0001_f32, true), (0.0004, false)] {
        fixture.carrier = original.clone();
        for body in &mut fixture.carrier.bodies {
            for point in &mut body.tri.positions {
                point[2] -= shift;
            }
        }
        assert_eq!(fixture.measure().unwrap().is_covered(), covered);
    }
}

#[test]
fn pathological_clipping_stops_with_an_explicit_error() {
    // Two sets of narrow strips leave a grid of thousands of separate holes.
    let mut remaining = vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]];
    let mut clips = 0;
    for axis in 0..2 {
        for index in 1..100 {
            let position = f64::from(index) / 100.0;
            let a = [position - 0.001, -1.0];
            let b = [position + 0.001, -1.0];
            let c = [position + 0.001, 2.0];
            let d = [position - 0.001, 2.0];
            for mut triangle in [[a, b, c], [a, c, d]] {
                if axis == 1 {
                    triangle = triangle.map(|point| [point[1], point[0]]);
                    triangle.swap(1, 2);
                }
                let mut next = Vec::new();
                for polygon in remaining {
                    if let Err(error) =
                        subtract_triangle(polygon, triangle, &mut next, &mut clips, 1e-12)
                    {
                        assert_eq!(error, ContactGeometryError::BudgetExceeded);
                        return;
                    }
                }
                remaining = next;
            }
        }
    }
    panic!("the dense grid must exhaust the bounded coverage budget");
}

#[test]
fn inward_facing_surfaces_do_not_witness_bearing() {
    let mut fixture = fixture(true, SupportShape::Full);
    for body in &mut fixture.carrier.bodies {
        for triangle in body.tri.indices.chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
    }
    let surface = fixture.measure().unwrap();
    assert!(!surface.is_covered());
    assert_eq!(surface.carrier_uncovered_area, surface.checked_area);
}
