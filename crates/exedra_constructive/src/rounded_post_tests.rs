// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::discretize::DiscretizePolicy;
use crate::evaluate::{Fidelity, evaluate};
use crate::ir::{CapMode, CsgOp, NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};
use crate::profile::{Loop2, Profile2, Seg2};
use crate::tessellate::EvalPolicy;
use alloc::{vec, vec::Vec};

#[test]
fn cylinder_with_spherical_top_is_an_exact_union() {
    let mut b = RecipeBuilder::new();
    let cylinder = b
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Cylinder {
                radius: 0.02,
                height: 0.5,
                segments: 16,
            },
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    // Construct cylinders extend along Z; ODB cylinders extend along Y.
    let cylinder = b
        .add(NodeKind::Transform {
            child: cylinder,
            xf: Placement3 {
                rows: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, -1.0, 0.0, 0.0],
                ],
            },
        })
        .unwrap();
    let profile = b.add_profile(
        Profile2::simple(
            Loop2::new(vec![Seg2::arc((0.0, 0.02), 1.0), Seg2::line((0.0, -0.02))]).unwrap(),
        )
        .unwrap(),
    );
    let sphere = b
        .add(NodeKind::Revolve {
            profile,
            placement: Placement3::IDENTITY,
            sweep: core::f64::consts::TAU,
            caps: CapMode::Both,
        })
        .unwrap();
    let sphere = b
        .add(NodeKind::Transform {
            child: sphere,
            xf: Placement3::translate(0.0, 0.5, 0.0),
        })
        .unwrap();
    let union = b
        .add(NodeKind::Csg {
            op: CsgOp::Union,
            operands: vec![cylinder, sphere],
        })
        .unwrap();
    let recipe = b.finish(union).unwrap();
    let policy = EvalPolicy {
        discretize: DiscretizePolicy {
            chord_tolerance: 0.0005,
            max_segment_edges: 4096,
            ..DiscretizePolicy::default()
        },
        ..EvalPolicy::default()
    };
    let result = evaluate(&recipe, &policy).unwrap();
    assert!(
        result
            .report
            .fidelity
            .iter()
            .all(|(_, f)| matches!(f, Fidelity::Exact)),
        "{:#?}",
        result.report
    );
    assert_eq!(result.bodies.len(), 1);
    assert_closed_rounded_post(&result.bodies[0].body.mesh);
}

// Independent volume of a 16-sided cylinder plus four polygonal frusta
// forming the sampled upper hemisphere (eight meridian edges per semicircle).
fn rounded_post_volume() -> f64 {
    let radius = 0.02;
    let polygon_factor = 8.0 * libm::sin(core::f64::consts::TAU / 16.0);
    let mut volume = polygon_factor * radius * radius * 0.5;
    for i in 0..4 {
        let a = f64::from(i) * core::f64::consts::PI / 8.0;
        let b = f64::from(i + 1) * core::f64::consts::PI / 8.0;
        let r0 = radius * libm::sin(a);
        let r1 = radius * libm::sin(b);
        let height = radius * (libm::cos(a) - libm::cos(b));
        volume += polygon_factor * height * (r0 * r0 + r0 * r1 + r1 * r1) / 3.0;
    }
    volume
}

fn assert_closed_rounded_post(mesh: &exedra_mesh::Mesh) {
    use exedra_math::{cross, dot, sub};
    use exedra_mesh::{FaceId, FaceTriangulation};
    assert!(mesh.validate_deep().is_empty());
    let mut visited = Vec::new();
    let mut pending = vec![mesh.faces().next().expect("nonempty solid")];
    let mut volume = 0.0;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    while let Some(face) = pending.pop() {
        if visited.contains(&face) {
            continue;
        }
        visited.push(face);
        for edge in mesh.face_loop(face) {
            let neighbor = mesh.face(mesh.twin(edge).expect("twin")).expect("face");
            assert_ne!(neighbor, FaceId::OUTSIDE, "solid must be closed");
            pending.push(neighbor);
            let p = mesh
                .vertex_position(mesh.to_vertex(edge).unwrap())
                .unwrap()
                .map(f64::from);
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
        let mut triangles = Vec::new();
        assert!(
            !mesh.face_triangles_into(face, FaceTriangulation::Robust, &mut triangles),
            "no triangulation fallback"
        );
        assert!(!triangles.is_empty());
        for corners in triangles {
            let p = corners.map(|c| {
                mesh.vertex_position(mesh.to_vertex(c).unwrap())
                    .unwrap()
                    .map(f64::from)
            });
            assert_ne!(cross(sub(p[1], p[0]), sub(p[2], p[0])), [0.0; 3]);
            volume += dot(p[0], cross(p[1], p[2])) / 6.0;
        }
    }
    assert_eq!(visited.len(), mesh.faces().count(), "one connected shell");
    let expected = rounded_post_volume();
    assert!(
        (volume - expected).abs() < expected * 1e-5,
        "outward volume {volume}, expected {expected}"
    );
    for (actual, expected) in min
        .into_iter()
        .zip([-0.02, 0.0, -0.02])
        .chain(max.into_iter().zip([0.02, 0.52, 0.02]))
    {
        assert!(
            (actual - expected).abs() < 1e-7,
            "bound {actual}, expected {expected}"
        );
    }
}
