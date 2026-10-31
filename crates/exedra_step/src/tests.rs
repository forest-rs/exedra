// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::{format, vec};
use exedra_analytic::{
    body::{BodyParts, BoundaryLoop, Coedge, Edge, Face, Sense},
    geometry::{Curve, CurveSpan, ParameterDomain, ParameterRange, Surface},
};

fn cube() -> Body {
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 3.0, 0.0],
        [0.0, 3.0, 0.0],
        [0.0, 0.0, 4.0],
        [2.0, 0.0, 4.0],
        [2.0, 3.0, 4.0],
        [0.0, 3.0, 4.0],
    ];
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ]
    .into_iter()
    .map(|(a, b)| Edge {
        geometry: exedra_analytic::body::EdgeGeometry::Curve {
            curve: Curve::line(vertices[a], exedra_math::sub(vertices[b], vertices[a])).unwrap(),
            range: ParameterRange::UNIT,
        },
        start: VertexId(u32::try_from(a).unwrap()),
        end: VertexId(u32::try_from(b).unwrap()),
        role: (),
    })
    .collect();
    let loops = [
        [(3, true), (2, true), (1, true), (0, true)],
        [(4, false), (5, false), (6, false), (7, false)],
        [(0, false), (9, false), (4, true), (8, true)],
        [(1, false), (10, false), (5, true), (9, true)],
        [(2, false), (11, false), (6, true), (10, true)],
        [(3, false), (8, false), (7, true), (11, true)],
    ]
    .into_iter()
    .map(|ring| BoundaryLoop {
        coedges: ring
            .map(|(edge, reversed)| Coedge {
                edge: EdgeId(edge),
                reversed,
            })
            .to_vec(),
    })
    .collect();
    let frames = [
        Placement3::IDENTITY,
        Placement3::translate(0.0, 0.0, 4.0),
        Placement3::from_axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0; 3]),
        Placement3::from_axes(
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
        ),
        Placement3::from_axes(
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [2.0, 3.0, 0.0],
        ),
        Placement3::from_axes(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
            [0.0, 3.0, 0.0],
        ),
    ];
    let faces = frames
        .into_iter()
        .enumerate()
        .map(|(i, frame)| Face {
            surface: Surface::plane(frame).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(0.0, if i == 3 || i == 5 { 3.0 } else { 2.0 }).unwrap(),
                v: ParameterRange::new(0.0, if i < 2 { 3.0 } else { 4.0 }).unwrap(),
            },
            sense: if i == 0 {
                Sense::Reverse
            } else {
                Sense::Forward
            },
            loops: vec![LoopId(u32::try_from(i).unwrap())],
            role: (),
        })
        .collect();
    Body::try_from_parts(
        BodyParts {
            trims: Vec::new(),
            vertices,
            edges,
            loops,
            faces,
        },
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap()
}
#[test]
fn complete_document_is_deterministic_shared_and_fully_referenced() {
    let body = cube();
    let options = StepOptions::default();
    let a = export_body(&body, Placement3::IDENTITY, &options).unwrap();
    assert_eq!(
        a,
        export_body(&body, Placement3::IDENTITY, &options).unwrap()
    );
    assert!(a.text.starts_with("ISO-10303-21;\nHEADER;"));
    assert!(a.text.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    assert!(a.text.contains("FILE_SCHEMA(('AUTOMOTIVE_DESIGN'))"));
    assert_eq!(
        (
            a.report.counts.vertices,
            a.report.counts.edges,
            a.report.counts.coedges,
            a.report.counts.faces
        ),
        (8, 12, 24, 6)
    );
    assert_eq!(a.text.matches("EDGE_CURVE(").count(), 12);
    assert_eq!(a.text.matches("ORIENTED_EDGE(").count(), 24);
    let entities: Vec<_> = a
        .text
        .lines()
        .filter(|line| line.starts_with('#'))
        .collect();
    assert_eq!(entities.len(), a.report.counts.entities as usize);
    for (i, line) in entities.iter().enumerate() {
        assert!(line.starts_with(&format!("#{}=", i + 1)));
    }
    for tail in a.text.split('#').skip(1) {
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let id = tail[..digits].parse::<u32>().unwrap();
        assert!((1..=a.report.counts.entities).contains(&id));
    }
    for (i, &id) in a.report.correspondence.faces.iter().enumerate() {
        assert_eq!(
            a.report
                .correspondence
                .face(FaceId(u32::try_from(i).unwrap())),
            id
        );
        assert!(entities[id.unwrap().0 as usize - 1].contains("ADVANCED_FACE("));
    }
    assert!(a.report.max_deviation <= options.tolerance);
}
#[test]
fn unused_native_entries_do_not_change_exported_geometry() {
    let body = cube();
    let original = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    let mut parts = body.into_parts();
    parts.vertices.push([1000.0, 0.0, 0.0]);
    let body = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap();
    let output = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    assert_eq!(output.text, original.text);
    assert_eq!(output.report.counts, original.report.counts);
    assert_eq!(output.report.correspondence.vertex(VertexId(8)), None);
}
#[test]
fn budgets_refuse_complete_output_without_changing_the_source() {
    let body = cube();
    let before = body.clone();
    let options = StepOptions::default();
    let output = export_body(&body, Placement3::IDENTITY, &options).unwrap();
    for (entities, bytes, resource) in [
        (1, options.max_bytes, "input tables"),
        (options.max_entities, output.text.len() - 1, "bytes"),
        (
            output.report.counts.entities - 1,
            options.max_bytes,
            "entities",
        ),
    ] {
        let error = export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                max_entities: entities,
                max_bytes: bytes,
                ..options
            },
        )
        .unwrap_err();
        assert!(matches!(error,StepError::Budget {resource:r,..} if r==resource));
    }
    assert!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                max_entities: output.report.counts.entities,
                max_bytes: output.text.len(),
                ..options
            }
        )
        .is_ok()
    );
    assert_eq!(body, before);
}
#[test]
fn placement_accuracy_timestamp_and_options_are_explicit() {
    let body = cube();
    let options = StepOptions::default();
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                tolerance: f64::NAN,
                ..options
            }
        ),
        Err(StepError::InvalidOptions)
    ));
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                timestamp: "2023-02-29T00:00:00Z",
                ..options
            }
        ),
        Err(StepError::Timestamp)
    ));
    assert!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                timestamp: "2024-02-29T12:23:45.123Z",
                ..options
            }
        )
        .is_ok()
    );
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                tolerance: 1e-16,
                ..options
            }
        ),
        Err(StepError::Accuracy { .. })
    ));
    let scaled = Placement3::from_axes([2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]);
    assert!(matches!(
        export_body(&body, scaled, &options),
        Err(StepError::Placement(_))
    ));
    let placed = export_body(&body, Placement3::translate(100.0, 20.0, -3.0), &options).unwrap();
    assert!(placed.text.contains("1.00000000000000000E2"));
    assert_ne!(
        placed.text,
        export_body(&body, Placement3::IDENTITY, &options)
            .unwrap()
            .text
    );
}
#[test]
fn unsupported_supports_and_open_or_multiple_shells_have_distinct_refusals() {
    let body = cube();
    let mut parts = body.clone().into_parts();
    parts.faces.pop();
    let open = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap();
    assert!(matches!(
        export_body(&open, Placement3::IDENTITY, &StepOptions::default()),
        Err(StepError::EdgeIncidence { .. })
    ));
    let mut parts = body.clone().into_parts();
    let a = CurveSpan::new(
        Curve::line([0.0, 0.0, 4.0], [2.0, 0.0, 0.0]).unwrap(),
        ParameterRange::UNIT,
        false,
    )
    .unwrap();
    let b = CurveSpan::new(
        Curve::line([0.0, 3.0, 4.0], [2.0, 0.0, 0.0]).unwrap(),
        ParameterRange::UNIT,
        false,
    )
    .unwrap();
    parts.faces[1].surface = Surface::ruled(a, b).unwrap();
    parts.faces[1].domain = ParameterDomain {
        u: ParameterRange::UNIT,
        v: ParameterRange::UNIT,
    };
    let ruled = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap();
    // A patch ruled between two lines is a bilinear B-spline surface.
    let document = export_body(&ruled, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    assert!(
        document
            .text
            .contains("B_SPLINE_SURFACE_WITH_KNOTS('',1,1,((#"),
        "{}",
        document.text
    );
    // Between a line and an arc, the rulings join points at equal angle
    // and equal length, which no rational surface does with those rows.
    let line = CurveSpan::new(
        Curve::line([-1.0, 0.0, -2.0], [2.0, 0.0, 0.0]).unwrap(),
        ParameterRange::UNIT,
        false,
    )
    .unwrap();
    // A quarter of a circle of radius sqrt 2 above it, from x = -1 to 1.
    let arc = CurveSpan::new(
        Curve::circle(
            Placement3::from_axes(
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, -1.0],
            ),
            core::f64::consts::SQRT_2,
        )
        .unwrap(),
        ParameterRange::new(
            core::f64::consts::FRAC_PI_4,
            3.0 * core::f64::consts::FRAC_PI_4,
        )
        .unwrap(),
        false,
    )
    .unwrap();
    assert!(matches!(
        preflight::surface_support(&Surface::ruled(line, arc).unwrap(), FaceId(3)),
        Err(StepError::UnsupportedSurface {
            face: FaceId(3),
            family: "ruled patch no B-spline surface represents exactly"
        })
    ));
    let mut parts = body.clone().into_parts();
    let mut other = body.into_parts();
    for p in &mut other.vertices {
        p[0] += 10.0;
    }
    for e in &mut other.edges {
        let Curve::Line(l) = e.regular().unwrap().0 else {
            unreachable!()
        };
        *e.regular_mut().unwrap().0 = Curve::line(
            exedra_math::add(l.origin(), [10.0, 0.0, 0.0]),
            l.direction(),
        )
        .unwrap();
        e.start.0 += 8;
        e.end.0 += 8;
    }
    for l in &mut other.loops {
        for c in &mut l.coedges {
            c.edge.0 += 12;
        }
    }
    for f in &mut other.faces {
        f.surface =
            Surface::plane(Placement3::translate(10.0, 0.0, 0.0) * f.surface.frame()).unwrap();
        for l in &mut f.loops {
            l.0 += 6;
        }
    }
    parts.vertices.extend(other.vertices);
    parts.edges.extend(other.edges);
    parts.loops.extend(other.loops);
    parts.faces.extend(other.faces);
    let multiple = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap();
    assert!(matches!(
        export_body(&multiple, Placement3::IDENTITY, &StepOptions::default()),
        Err(StepError::ShellValidation(
            exedra_analytic::rectilinear::MaterialValidationError::MaterialComponents { found: 2 }
        ))
    ));
}

#[test]
fn structural_support_is_separate_from_final_options_and_budget_validation() {
    let body = cube();
    assert_eq!(check_geometry_support(&body), Ok(()));
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                tolerance: 0.0,
                ..StepOptions::default()
            }
        ),
        Err(StepError::InvalidOptions)
    ));
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                max_bytes: 1,
                ..StepOptions::default()
            }
        ),
        Err(StepError::Budget { .. })
    ));
}

fn sphere_with_poles(seam: bool) -> Body {
    let radius = 2.0;
    let mut edges = vec![
        Edge::collapsed(VertexId(0), ()),
        Edge::collapsed(VertexId(1), ()),
    ];
    let coedges = if seam {
        edges.push(Edge::new(
            Curve::circle(
                Placement3::from_axes(
                    [0.0, 0.0, -1.0],
                    [1.0, 0.0, 0.0],
                    [0.0, -1.0, 0.0],
                    [0.0; 3],
                ),
                radius,
            )
            .unwrap(),
            ParameterRange::new(0.0, core::f64::consts::PI).unwrap(),
            VertexId(0),
            VertexId(1),
            (),
        ));
        vec![
            Coedge {
                edge: EdgeId(0),
                reversed: false,
            },
            Coedge {
                edge: EdgeId(2),
                reversed: false,
            },
            Coedge {
                edge: EdgeId(1),
                reversed: true,
            },
            Coedge {
                edge: EdgeId(2),
                reversed: true,
            },
        ]
    } else {
        vec![Coedge {
            edge: EdgeId(0),
            reversed: false,
        }]
    };
    Body::try_from_parts(
        BodyParts {
            trims: Vec::new(),
            vertices: vec![[0.0, 0.0, -radius], [0.0, 0.0, radius]],
            edges,
            loops: vec![BoundaryLoop { coedges }],
            faces: vec![Face {
                surface: Surface::sphere(Placement3::IDENTITY, radius).unwrap(),
                domain: ParameterDomain {
                    u: ParameterRange::new(0.0, core::f64::consts::TAU).unwrap(),
                    v: ParameterRange::new(
                        -core::f64::consts::FRAC_PI_2,
                        core::f64::consts::FRAC_PI_2,
                    )
                    .unwrap(),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            }],
        },
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-10 },
    )
    .unwrap()
}
#[test]
fn certified_poles_map_to_vertices_and_regular_meridian_identity_survives() {
    for seam in [false, true] {
        let body = sphere_with_poles(seam);
        assert_eq!(check_geometry_support(&body), Ok(()));
        let file = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
        for options in [
            StepOptions {
                max_bytes: file.text.len() - 1,
                ..StepOptions::default()
            },
            StepOptions {
                max_entities: file.report.counts.entities - 1,
                ..StepOptions::default()
            },
        ] {
            assert!(matches!(
                export_body(&body, Placement3::IDENTITY, &options),
                Err(StepError::Budget { .. })
            ));
        }
        let map = &file.report.correspondence;
        assert!(file.text.contains("SPHERICAL_SURFACE("));
        assert!(!file.text.contains("UNSUPPORTED"));
        assert_eq!(
            map.edge(EdgeId(0)),
            Some(StepEdgeEntity::CollapsedVertex(
                map.vertex(VertexId(0)).unwrap()
            ))
        );
        assert_eq!(file.report.counts.edges, u32::from(seam));
        assert_eq!(file.report.counts.coedges, 2 * u32::from(seam));
        if seam {
            assert_eq!(
                map.edge(EdgeId(1)),
                Some(StepEdgeEntity::CollapsedVertex(
                    map.vertex(VertexId(1)).unwrap()
                ))
            );
            let Some(StepEdgeEntity::Curve(curve)) = map.edge(EdgeId(2)) else {
                panic!("ordinary meridian must remain an edge")
            };
            assert!(file.text.contains(&format!("#{}=EDGE_CURVE(", curve.0)));
            assert!(file.text.contains("EDGE_LOOP("));
            assert!(!file.text.contains("VERTEX_LOOP("));
        } else {
            assert_eq!(
                map.edge(EdgeId(1)),
                None,
                "detached pole stays explicitly unused"
            );
            assert!(file.text.contains("VERTEX_LOOP("));
            assert!(!file.text.contains("EDGE_CURVE("));
        }
    }
}

#[test]
fn elliptic_rims_export_shared_exact_arc_chains_with_complete_correspondence() {
    use exedra_analytic::machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder};
    let source = cube();
    let result = subtract_through_cylinder(
        &source,
        &CylinderTool::new([0.5, 0.5, 0.0], [0.1, 0.15, 1.0], 0.1).unwrap(),
        &MachiningPolicy::default(),
    )
    .unwrap();
    let document =
        export_body(&result.body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    let map = &document.report.correspondence;
    for rim in result.report.rims {
        let entity = map.edge(rim).unwrap();
        assert!(matches!(
            entity,
            StepEdgeEntity::CurveChain { count: 8, .. }
        ));
        let fragments = entity.curve_entities().collect::<Vec<_>>();
        assert_eq!(fragments.len(), 8);
        for fragment in fragments {
            assert_eq!(
                document
                    .text
                    .matches(&format!("*,*,#{},", fragment.0))
                    .count(),
                2,
                "every fragment is shared by exactly two face occurrences"
            );
            assert!(
                document
                    .text
                    .contains(&format!("#{}=EDGE_CURVE(", fragment.0))
            );
        }
    }
    assert_eq!(document.report.counts.vertices, 24);
    assert_eq!(document.report.counts.edges, 29);
    assert_eq!(document.report.counts.coedges, 58);
    assert_eq!(document.text.matches("=VERTEX_POINT(").count(), 24);
    assert_eq!(document.text.matches("=EDGE_CURVE(").count(), 29);
    assert_eq!(document.text.matches("=ORIENTED_EDGE(").count(), 58);
    assert_eq!(document.text.matches("=ELLIPSE(").count(), 2);
    // The additional exact topology participates in the ordinary atomic budget.
    let options = StepOptions {
        max_entities: document.report.counts.entities - 1,
        ..Default::default()
    };
    assert!(matches!(
        export_body(&result.body, Placement3::IDENTITY, &options),
        Err(StepError::Budget {
            resource: "entities",
            ..
        })
    ));
}

#[test]
fn ellipse_fragment_policy_tracks_physical_units_and_exact_equal_axes() {
    use exedra_analytic::machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder};
    let source = cube();
    let oblique = subtract_through_cylinder(
        &source,
        &CylinderTool::new([0.5, 0.5, 0.0], [0.1, 0.15, 1.0], 0.1).unwrap(),
        &MachiningPolicy::default(),
    )
    .unwrap();
    for (units, count) in [(LengthUnit::Millimetre, 8), (LengthUnit::Metre, 16)] {
        let doc = export_body(
            &oblique.body,
            Placement3::IDENTITY,
            &StepOptions {
                units,
                ..Default::default()
            },
        )
        .unwrap();
        for rim in oblique.report.rims {
            assert!(
                matches!(doc.report.correspondence.edge(rim),Some(StepEdgeEntity::CurveChain{count:n,..}) if n==count)
            );
        }
    }
    let axial = subtract_through_cylinder(
        &source,
        &CylinderTool::new([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], 0.1).unwrap(),
        &MachiningPolicy::default(),
    )
    .unwrap();
    let mut parts = axial.body.into_parts();
    // Exercise the exact ellipse identity even when a constructor selected Circle.
    for rim in axial.report.rims {
        let (curve, _) = parts.edges[rim.index()].regular_mut().unwrap();
        if let Curve::Circle(c) = curve {
            *curve = Curve::ellipse(c.frame(), c.radius(), c.radius()).unwrap();
        }
    }
    let body = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-9 },
    )
    .unwrap();
    let doc = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    for rim in axial.report.rims {
        assert!(matches!(
            doc.report.correspondence.edge(rim),
            Some(StepEdgeEntity::Curve(_))
        ));
    }
    assert_eq!(doc.text.matches("=ELLIPSE(").count(), 0);
    assert_eq!(doc.text.matches("=CIRCLE(").count(), 2);
    assert_eq!(doc.report.counts.edges, 15);
    assert_eq!(doc.report.counts.vertices, 10);
}

#[test]
fn cavities_preserve_shells_face_senses_and_material_accuracy_evidence() {
    let outer = cube();
    let mut inner = cube().into_parts();
    let transform =
        |p: [f64; 3]| core::array::from_fn(|axis| p[axis] * 0.25 + [0.75, 1.125, 1.5][axis]);
    for p in &mut inner.vertices {
        *p = transform(*p);
    }
    for edge in &mut inner.edges {
        let (curve, _) = edge.regular_mut().unwrap();
        let Curve::Line(line) = curve else {
            panic!("cube line");
        };
        *curve = Curve::line(transform(line.origin()), line.direction().map(|v| v * 0.25)).unwrap();
    }
    for face in &mut inner.faces {
        let mut frame = face.surface.frame();
        let origin = transform(frame.rows.map(|r| r[3]));
        for (row, value) in frame.rows.iter_mut().zip(origin) {
            row[3] = value;
        }
        face.surface = Surface::plane(frame).unwrap();
        let range = |range: ParameterRange| {
            let [a, b] = range.bounds();
            ParameterRange::new(a * 0.25, b * 0.25).unwrap()
        };
        face.domain = ParameterDomain {
            u: range(face.domain.u),
            v: range(face.domain.v),
        };
    }
    inner.trims.clear();
    let inner = Body::try_from_parts(
        inner,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-9 },
    )
    .unwrap();
    let body = exedra_analytic::rectilinear::boolean_rectilinear(
        &outer,
        &inner,
        exedra_analytic::rectilinear::BooleanOperation::Difference,
        &exedra_analytic::rectilinear::RectilinearPolicy::default(),
    )
    .unwrap()
    .body;
    let file = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    assert!(file.text.contains("BREP_WITH_VOIDS("));
    assert_eq!(file.text.matches("ORIENTED_CLOSED_SHELL('',*,").count(), 1);
    assert_eq!(file.report.counts.shells, 2);
    assert_eq!(file.report.counts.solids, 1);
    let certificate = file.report.material_validation.as_ref().unwrap();
    assert_eq!(certificate.volume, 24.0 - 24.0 / 64.0);
    assert!(
        file.report.max_deviation
            >= certificate.boundary.geometry.max_gap + certificate.arithmetic_allowance
    );
    assert!(certificate.plane_vertices <= 8 * certificate.cells);
    assert!(certificate.arrangement_scratch_bytes >= certificate.link_bytes);
    for (shell, entry) in certificate.boundary.shells.iter().enumerate() {
        for &face in &entry.faces {
            assert_eq!(
                file.report.correspondence.face_reversed(face),
                Some(shell != certificate.outer_shell)
            );
        }
    }
    let mut options = StepOptions::default();
    options.shell_validation.arrangement.max_cells = 1;
    assert!(matches!(
        export_body(&body, Placement3::IDENTITY, &options),
        Err(StepError::ShellValidation(
            exedra_analytic::rectilinear::MaterialValidationError::Arrangement(
                exedra_analytic::rectilinear::RectilinearError::CellBudget { .. }
            )
        ))
    ));
    let ordinary = export_body(&outer, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    assert!(ordinary.report.material_validation.is_none());
    assert!(ordinary.report.correspondence.reversed_faces.is_empty());
}

/// A prism of unit height over the face between a NURBS edge, over `range`
/// of `curve` from the origin to `(4, 0, 0)` bulging toward `+y`, and the
/// line back; its curved wall is ruled between the edge and its copy above.
fn spline_prism(curve: &exedra_analytic::geometry::NurbsCurve, range: ParameterRange) -> Body {
    use exedra_analytic::body::{EdgeGeometry, ValidationPolicy};
    let below = Curve::nurbs(curve.clone()).unwrap();
    let above = below
        .transformed(Placement3::translate(0.0, 0.0, 1.0))
        .unwrap();
    let wall = Surface::ruled(
        CurveSpan::new(below.clone(), range, false).unwrap(),
        CurveSpan::new(above.clone(), range, false).unwrap(),
    )
    .unwrap();
    let edge = |curve: Curve, range: ParameterRange, start: u32, end: u32| Edge {
        geometry: EdgeGeometry::Curve { curve, range },
        start: VertexId(start),
        end: VertexId(end),
        role: (),
    };
    let line = |a: [f64; 3], b: [f64; 3]| {
        let curve = Curve::line(a, core::array::from_fn(|k| b[k] - a[k])).unwrap();
        (curve, ParameterRange::UNIT)
    };
    let coedge = |edge: u32, reversed: bool| Coedge {
        edge: EdgeId(edge),
        reversed,
    };
    let plane = |frame: Placement3, v: [f64; 2], sense: Sense, loop_id: u32| Face {
        surface: Surface::plane(frame).unwrap(),
        domain: ParameterDomain {
            u: ParameterRange::new(-1.0, 5.0).unwrap(),
            v: ParameterRange::new(v[0], v[1]).unwrap(),
        },
        sense,
        loops: vec![LoopId(loop_id)],
        role: (),
    };
    let [a, b, c, d] = [[0.0; 3], [4.0, 0.0, 0.0], [0.0, 0.0, 1.0], [4.0, 0.0, 1.0]];
    let edges = vec![
        edge(below, range, 0, 1),
        edge(above, range, 2, 3),
        {
            let (curve, range) = line(b, a);
            edge(curve, range, 1, 0)
        },
        {
            let (curve, range) = line(d, c);
            edge(curve, range, 3, 2)
        },
        {
            let (curve, range) = line(a, c);
            edge(curve, range, 0, 2)
        },
        {
            let (curve, range) = line(b, d);
            edge(curve, range, 1, 3)
        },
    ];
    let parts: BodyParts<(), ()> = BodyParts {
        trims: Vec::new(),
        vertices: vec![a, b, c, d],
        edges,
        loops: vec![
            BoundaryLoop {
                coedges: vec![coedge(0, false), coedge(2, false)],
            },
            BoundaryLoop {
                coedges: vec![coedge(3, true), coedge(1, true)],
            },
            BoundaryLoop {
                coedges: vec![
                    coedge(4, false),
                    coedge(1, false),
                    coedge(5, true),
                    coedge(0, true),
                ],
            },
            BoundaryLoop {
                coedges: vec![
                    coedge(2, true),
                    coedge(5, false),
                    coedge(3, false),
                    coedge(4, true),
                ],
            },
        ],
        faces: vec![
            plane(Placement3::IDENTITY, [-1.0, 3.0], Sense::Reverse, 0),
            plane(
                Placement3::translate(0.0, 0.0, 1.0),
                [-1.0, 3.0],
                Sense::Forward,
                1,
            ),
            Face {
                surface: wall,
                domain: ParameterDomain {
                    u: ParameterRange::UNIT,
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Reverse,
                loops: vec![LoopId(2)],
                role: (),
            },
            plane(
                Placement3::from_axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0; 3]),
                [-1.0, 2.0],
                Sense::Forward,
                3,
            ),
        ],
    };
    Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
}

/// The `B_SPLINE_CURVE_WITH_KNOTS` instances a document writes.
fn spline_curves(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.contains("=B_SPLINE_CURVE_WITH_KNOTS("))
        .collect()
}

#[test]
fn nurbs_edges_are_written_over_their_own_range() {
    // A closed cubic: over [0, 2] from the origin out to (4, 0, 0) through
    // +y, and over [2, 4] back through -y. An edge over [0, 2] written as
    // the whole curve leaves a reader two ways from its start to its end.
    let loop_curve = exedra_analytic::geometry::NurbsCurve::new(
        4,
        vec![0.0, 0.0, 0.0, 0.0, 2.0, 2.0, 2.0, 4.0, 4.0, 4.0, 4.0],
        vec![
            [0.0; 3],
            [0.0, 3.0, 0.0],
            [4.0, 3.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, -3.0, 0.0],
            [0.0, -3.0, 0.0],
            [0.0; 3],
        ],
        None,
    )
    .unwrap();
    let body = spline_prism(&loop_curve, ParameterRange::new(0.0, 2.0).unwrap());
    let document = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    let curves = spline_curves(&document.text);
    assert_eq!(curves.len(), 2, "{}", document.text);
    for curve in curves {
        // One Bezier piece: four controls over [0, 2].
        assert!(curve.contains(",(4,4),("), "{curve}");
        assert_eq!(curve.split_once('=').unwrap().1.matches('#').count(), 4);
    }
}
