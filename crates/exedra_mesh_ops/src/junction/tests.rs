// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use exedra_mesh::{ExtractParams, FaceId, HalfEdgeId, Mesh, MeshBuilder, attr, op};

use super::*;

/// One open tube: `count` vertices per ring, `length` long, starting `start`
/// from the origin along `direction`.
#[derive(Copy, Clone, Debug)]
struct Tube {
    direction: [f64; 3],
    start: f64,
    radius: f64,
    count: usize,
}

const LENGTH: f64 = 1.0;

fn unit(v: [f64; 3]) -> [f64; 3] {
    normalize(v).expect("nonzero")
}

fn frame(direction: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let helper = if direction[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let e1 = unit(sub(helper, scale(direction, dot(helper, direction))));
    (e1, cross(direction, e1))
}

/// Builds outward-oriented open tubes. Returns the mesh and one inner-end
/// boundary half-edge per tube.
fn tubes(specs: &[Tube]) -> (Mesh, Vec<HalfEdgeId>) {
    let mut builder = MeshBuilder::new();
    let mut inner_rings = Vec::new();
    for tube in specs {
        let d = unit(tube.direction);
        let (e1, e2) = frame(d);
        let ring = |distance: f64, builder: &mut MeshBuilder| {
            (0..tube.count)
                .map(|k| {
                    let angle = TAU * k as f64 / tube.count as f64;
                    let p = add(
                        scale(d, distance),
                        add(
                            scale(e1, tube.radius * libm::cos(angle)),
                            scale(e2, tube.radius * libm::sin(angle)),
                        ),
                    );
                    builder.push_vertex(narrow(p))
                })
                .collect::<Vec<_>>()
        };
        let inner = ring(tube.start, &mut builder);
        let outer = ring(tube.start + LENGTH, &mut builder);
        for k in 0..tube.count {
            let k1 = (k + 1) % tube.count;
            builder
                .add_face(&[inner[k], inner[k1], outer[k1], outer[k]])
                .expect("tube face");
        }
        inner_rings.push(inner);
    }
    let built = builder.build().expect("tubes build");
    let mesh = built.mesh;
    let seeds = inner_rings
        .iter()
        .map(|ring| {
            let (a, b) = (ring[1], ring[0]);
            mesh.half_edges()
                .find(|&h| {
                    mesh.face(h) == Some(FaceId::OUTSIDE)
                        && mesh.from_vertex(h).map(|v| v.index()) == Some(a)
                        && mesh.to_vertex(h).map(|v| v.index()) == Some(b)
                })
                .expect("inner boundary half-edge")
        })
        .collect();
    (mesh, seeds)
}

fn tube(direction: [f64; 3], count: usize) -> Tube {
    Tube {
        direction,
        start: 0.6,
        radius: 0.2,
        count,
    }
}

fn params(seeds: Vec<HalfEdgeId>) -> JunctionParams {
    JunctionParams {
        center: [0.0; 3],
        rings: seeds,
        chart: None,
        region: None,
    }
}

/// Adds the junction, caps the tubes' outer ends, and checks the solid.
fn close_and_check(specs: &[Tube]) -> (Mesh, JunctionOutput) {
    let (mut mesh, seeds) = tubes(specs);
    let mut edit = mesh.edit();
    let output = add_junction(&mut edit, &params(seeds)).expect("junction");
    let _: () = edit.finish();

    // Only the outer ends stay open; cap them.
    let loops = mesh.boundary_loops().expect("boundary loops");
    assert_eq!(loops.len(), specs.len(), "every inner ring is closed");
    let mut edit = mesh.edit();
    for boundary in &loops {
        let vertices = boundary
            .iter()
            .map(|&h| edit.mesh().from_vertex(h).expect("vertex"))
            .collect::<Vec<_>>();
        op::add_face(&mut edit, &vertices).expect("cap");
    }
    let _: () = edit.finish();

    assert!(mesh.boundary_loops().expect("loops").is_empty(), "closed");
    assert!(mesh.validate_deep().is_empty(), "valid topology");
    let v = mesh.vertices().count();
    let f = mesh.faces().count();
    let e = mesh.half_edges().count() / 2;
    assert_eq!(v + f, e + 2, "genus zero");
    assert!(signed_volume(&mesh) > 0.0, "outward orientation");
    (mesh, output)
}

fn signed_volume(mesh: &Mesh) -> f64 {
    let (tri, _) = mesh.to_trimesh(&ExtractParams::default());
    tri.indices
        .chunks_exact(3)
        .map(|t| {
            let p = |i: u32| tri.positions[i as usize].map(f64::from);
            dot(p(t[0]), cross(p(t[1]), p(t[2]))) / 6.0
        })
        .sum()
}

fn fork(angle_degrees: f64) -> [f64; 3] {
    let a = angle_degrees.to_radians();
    [libm::sin(a), 0.0, libm::cos(a)]
}

#[test]
fn y_junction_closes_a_three_tube_fork() {
    let specs = [
        tube([0.0, 0.0, -1.0], 12),
        tube(fork(-35.0), 12),
        tube(fork(35.0), 12),
    ];
    let (_, output) = close_and_check(&specs);
    assert_eq!(output.arm_edges, vec![[0, 1], [0, 2], [1, 2]]);
    assert_eq!(output.stats.crotches, 2);
    assert_eq!(output.center_vertices.len(), 2);
    assert!(!output.stats.virtual_arm);
    assert!(output.stats.bridge_quads >= output.stats.bridge_triangles);
}

#[test]
fn t_junction_and_mixed_ring_counts_close() {
    close_and_check(&[
        tube([-1.0, 0.0, 0.0], 8),
        tube([1.0, 0.0, 0.0], 12),
        tube([0.0, 0.0, 1.0], 16),
    ]);
}

#[test]
fn three_to_five_children_close() {
    for children in 3..=5 {
        let mut specs = vec![tube([0.0, 0.0, -1.0], 16)];
        for c in 0..children {
            let azimuth = TAU * c as f64 / children as f64 + 0.3;
            let tilt = 50.0_f64.to_radians();
            specs.push(Tube {
                radius: 0.14,
                ..tube(
                    [
                        libm::sin(tilt) * libm::cos(azimuth),
                        libm::sin(tilt) * libm::sin(azimuth),
                        libm::cos(tilt),
                    ],
                    12,
                )
            });
        }
        let (_, output) = close_and_check(&specs);
        assert_eq!(output.origins.len(), output.faces.len());
        assert!(output.stats.crotches >= 2);
    }
}

#[test]
fn planar_cross_closes_with_two_polygonal_crotches() {
    let (_, output) = close_and_check(&[
        tube([1.0, 0.0, 0.0], 12),
        tube([0.0, 1.0, 0.0], 12),
        tube([-1.0, 0.0, 0.0], 12),
        tube([0.0, -1.0, 0.0], 12),
    ]);
    assert_eq!(output.stats.crotches, 2);
    assert_eq!(output.stats.crotch_quads, 8, "two four-arm crotches");
}

#[test]
fn coplanar_downward_arms_close_with_two_crotches() {
    let tilt = 60.0_f64.to_radians();
    let specs = (0..4)
        .map(|c| {
            let azimuth = TAU * c as f64 / 4.0;
            Tube {
                radius: 0.15,
                ..tube(
                    [
                        libm::sin(tilt) * libm::cos(azimuth),
                        libm::sin(tilt) * libm::sin(azimuth),
                        -libm::cos(tilt),
                    ],
                    12,
                )
            }
        })
        .collect::<Vec<_>>();
    let (_, output) = close_and_check(&specs);
    // The four directions share a bit-identical z only because the azimuths
    // are exactly symmetric; perturbed azimuths take the virtual-arm path,
    // which also yields two crotches.
    assert!(!output.stats.virtual_arm);
    assert_eq!(output.stats.crotches, 2);
}

#[test]
fn arms_in_one_hemisphere_merge_a_virtual_crotch() {
    let specs = [(0.0, 50.0), (1.7, 65.0), (3.3, 55.0), (4.8, 70.0)]
        .iter()
        .map(|&(azimuth, tilt_degrees): &(f64, f64)| {
            let tilt = tilt_degrees.to_radians();
            Tube {
                radius: 0.12,
                ..tube(
                    [
                        libm::sin(tilt) * libm::cos(azimuth),
                        libm::sin(tilt) * libm::sin(azimuth),
                        -libm::cos(tilt),
                    ],
                    12,
                )
            }
        })
        .collect::<Vec<_>>();
    let (_, output) = close_and_check(&specs);
    assert!(output.stats.virtual_arm);
}

#[test]
fn two_rings_join_with_one_closed_strip() {
    let (_, output) = close_and_check(&[tube([0.0, 0.0, -1.0], 10), tube(fork(30.0), 14)]);
    assert_eq!(output.stats.bridges, 1);
    assert_eq!(output.stats.crotches, 0);
    assert_eq!(
        output.stats.bridge_quads, 10,
        "one quad per shorter-ring step"
    );
    assert_eq!(output.stats.bridge_triangles, 4);
}

#[test]
fn planning_is_deterministic() {
    let specs = [
        tube([0.0, 0.0, -1.0], 12),
        tube(fork(-30.0), 10),
        tube(fork(40.0), 14),
        tube([0.0, 1.0, 0.3], 8),
    ];
    let (a, _) = close_and_check(&specs);
    let (b, _) = close_and_check(&specs);
    let (ta, _) = a.to_trimesh(&ExtractParams::default());
    let (tb, _) = b.to_trimesh(&ExtractParams::default());
    assert_eq!(ta, tb);
}

#[test]
fn chart_continues_the_parent_ring() {
    let specs = [
        tube([0.0, 0.0, -1.0], 12),
        tube(fork(-35.0), 12),
        tube(fork(35.0), 12),
    ];
    let (mut mesh, seeds) = tubes(&specs);
    let parent_loop = mesh.boundary_loop(seeds[0]).expect("parent loop");
    let parent_first = mesh.from_vertex(parent_loop[0]).expect("first");
    let chart = JunctionChart {
        parent: 0,
        u_origin: 0.25,
        u_per_turn: 3.0,
        v_origin: 5.0,
        v_per_unit: 2.0,
    };
    let mut edit = mesh.edit();
    let output = add_junction(
        &mut edit,
        &JunctionParams {
            chart: Some(chart),
            region: Some(7),
            ..params(seeds)
        },
    )
    .expect("junction");
    let _: () = edit.finish();

    let uv_layer = mesh.attrs().sparse(attr::CORNER_UV).expect("uv layer");
    let regions = mesh.attrs().dense(attr::FACE_REGION).expect("regions");
    for (k, &edge) in parent_loop.iter().enumerate() {
        let vertex = mesh.from_vertex(edge).expect("vertex");
        let expected_u = 0.25 + 3.0 * k as f64 / 12.0;
        for &face in &output.faces {
            for corner in mesh.face_loop(face) {
                if mesh.to_vertex(corner) != Some(vertex) {
                    continue;
                }
                let uv = uv_layer.get(corner.into()).expect("corner uv");
                assert!(
                    (f64::from(uv[1]) - 5.0).abs() < 1e-5,
                    "V at the parent ring"
                );
                // U at the parent ring follows its boundary order; the one
                // face crossing the seam may carry the next turn.
                let u = f64::from(uv[0]);
                let turns = (u - expected_u) / 3.0;
                assert!(
                    (turns - libm::round(turns)).abs() < 1e-5,
                    "U {u} at vertex {k}"
                );
                if vertex == parent_first {
                    assert!(libm::round(turns) >= 0.0);
                }
            }
        }
    }
    let parent_vertices = parent_loop
        .iter()
        .map(|&e| mesh.from_vertex(e).expect("vertex"))
        .collect::<Vec<_>>();
    for &face in &output.faces {
        assert_eq!(regions.get(face.into()), Some(&7));
        // The cylindrical chart wraps near the parent axis, so continuity is
        // checked where the chart continues the parent tube.
        if !mesh
            .face_loop(face)
            .any(|c| parent_vertices.contains(&mesh.to_vertex(c).expect("vertex")))
        {
            continue;
        }
        let us = mesh
            .face_loop(face)
            .map(|c| f64::from(uv_layer.get(c.into()).expect("uv")[0]))
            .collect::<Vec<_>>();
        let spread = us.iter().copied().fold(f64::MIN, f64::max)
            - us.iter().copied().fold(f64::MAX, f64::min);
        assert!(spread < 1.5, "U stays continuous within a face");
    }
}

#[test]
fn refusals_are_typed_and_leave_the_mesh_unchanged() {
    // Children 6 degrees apart overlap.
    let (mut mesh, seeds) = tubes(&[
        tube([0.0, 0.0, -1.0], 12),
        tube(fork(-3.0), 12),
        tube(fork(3.0), 12),
    ]);
    let faces_before = mesh.faces().count();
    let mut edit = mesh.edit();
    assert_eq!(
        add_junction(&mut edit, &params(seeds)),
        Err(JunctionError::OverlappingCollars { a: 1, b: 2 })
    );
    let _: () = edit.finish();
    assert_eq!(mesh.faces().count(), faces_before);

    // A ring whose tube points back through the center.
    let (mut mesh, seeds) = tubes(&[
        tube([0.0, 0.0, -1.0], 12),
        Tube {
            start: -2.0,
            ..tube([0.0, 0.0, 1.0], 12)
        },
    ]);
    let mut edit = mesh.edit();
    assert_eq!(
        add_junction(&mut edit, &params(seeds)),
        Err(JunctionError::RingFacesCenter { ring: 1 })
    );
    let _: () = edit.finish();

    // A triangle ring cannot face five neighbours.
    let mut specs = vec![tube([0.0, 0.0, -1.0], 3)];
    for c in 0..5 {
        let azimuth = TAU * c as f64 / 5.0;
        specs.push(Tube {
            radius: 0.1,
            ..tube([libm::cos(azimuth), libm::sin(azimuth), 0.2], 8)
        });
    }
    let (mut mesh, seeds) = tubes(&specs);
    let mut edit = mesh.edit();
    assert!(matches!(
        add_junction(&mut edit, &params(seeds)),
        Err(JunctionError::RingTooCoarse { ring: 0, .. })
    ));
    let _: () = edit.finish();

    // One ring, and a ring passed twice.
    let (mut mesh, seeds) = tubes(&[tube([0.0, 0.0, -1.0], 8), tube([0.0, 0.0, 1.0], 8)]);
    let mut edit = mesh.edit();
    assert_eq!(
        add_junction(&mut edit, &params(vec![seeds[0]])),
        Err(JunctionError::TooFewRings)
    );
    assert_eq!(
        add_junction(&mut edit, &params(vec![seeds[0], seeds[0]])),
        Err(JunctionError::SharedVertex { ring: 1, other: 0 })
    );
    let bad_chart = JunctionParams {
        chart: Some(JunctionChart {
            parent: 2,
            u_origin: 0.0,
            u_per_turn: 1.0,
            v_origin: 0.0,
            v_per_unit: 1.0,
        }),
        ..params(seeds)
    };
    assert_eq!(
        add_junction(&mut edit, &bad_chart),
        Err(JunctionError::InvalidChart)
    );
    let _: () = edit.finish();
}

#[test]
fn plans_work_without_a_mesh() {
    let ring = |direction: [f64; 3], count: usize| {
        let d = unit(direction);
        let (e1, e2) = frame(d);
        // Boundary order: clockwise seen from outside along the arm.
        (0..count)
            .map(|k| {
                let angle = -TAU * k as f64 / count as f64;
                add(
                    scale(d, 0.5),
                    add(
                        scale(e1, 0.1 * libm::cos(angle)),
                        scale(e2, 0.1 * libm::sin(angle)),
                    ),
                )
            })
            .collect::<Vec<_>>()
    };
    let plan = plan_junction(
        &[
            ring([0.0, 0.0, -1.0], 8),
            ring(fork(-40.0), 8),
            ring(fork(40.0), 8),
        ],
        [0.0; 3],
        None,
    )
    .expect("plan");
    assert_eq!(plan.centers.len(), 2);
    assert!(plan.faces.iter().all(|face| face.uvs.is_none()));
    // Every ring edge is used exactly once, in boundary order.
    for ring in 0..3 {
        for index in 0..8 {
            let from = JunctionVertex::Ring { ring, index };
            let to = JunctionVertex::Ring {
                ring,
                index: (index + 1) % 8,
            };
            let uses = plan
                .faces
                .iter()
                .filter(|face| {
                    let n = face.vertices.len();
                    (0..n).any(|k| face.vertices[k] == from && face.vertices[(k + 1) % n] == to)
                })
                .count();
            assert_eq!(uses, 1, "ring {ring} edge {index}");
        }
    }
}

#[test]
fn planned_faces_never_repeat_a_vertex() {
    let ring = |direction: [f64; 3], count: usize| {
        let d = unit(direction);
        let (e1, e2) = frame(d);
        (0..count)
            .map(|k| {
                let angle = -TAU * k as f64 / count as f64;
                add(
                    scale(d, 0.6),
                    add(
                        scale(e1, 0.2 * libm::cos(angle)),
                        scale(e2, 0.2 * libm::sin(angle)),
                    ),
                )
            })
            .collect::<Vec<_>>()
    };
    let plan = plan_junction(
        &[
            ring([0.0, 0.0, -1.0], 12),
            ring(fork(-35.0), 12),
            ring(fork(35.0), 12),
        ],
        [0.0; 3],
        None,
    )
    .expect("plan");
    for face in &plan.faces {
        for (k, v) in face.vertices.iter().enumerate() {
            assert!(
                !face.vertices[k + 1..].contains(v),
                "repeated vertex in {face:?}"
            );
        }
    }
}

// Exact self-intersection checks.

/// Pairs of mesh triangles, from the mesh's own robust triangulation, that
/// intersect other than along the vertices and edges they share.
fn self_intersections(mesh: &Mesh) -> Vec<(usize, usize)> {
    let mut tris = Vec::new();
    for face in mesh.faces() {
        for corners in mesh.face_triangles(face, exedra_mesh::FaceTriangulation::Robust) {
            let ids = corners.map(|c| mesh.to_vertex(c).expect("corner vertex"));
            let points = ids.map(|v| mesh.vertex_position(v).expect("position").map(f64::from));
            tris.push((ids, points));
        }
    }
    let mut hits = Vec::new();
    for i in 0..tris.len() {
        for j in i + 1..tris.len() {
            if intersect::triangles_intersect(tris[i].0, &tris[i].1, tris[j].0, &tris[j].1) {
                hits.push((i, j));
            }
        }
    }
    hits
}

#[test]
fn intersection_checker_sees_crossings_and_ignores_shared_features() {
    let (mesh, _) = close_and_check(&[
        tube([0.0, 0.0, -1.0], 12),
        tube(fork(-35.0), 12),
        tube(fork(35.0), 12),
    ]);
    assert!(self_intersections(&mesh).is_empty(), "a clean Y fork");

    // Two separate quads crossing each other.
    let mut builder = MeshBuilder::new();
    for p in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.5, 0.5, -0.5],
        [0.5, 0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, -0.5, -0.5],
    ] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2, 3]).expect("first quad");
    builder.add_face(&[4, 5, 6, 7]).expect("second quad");
    let crossing = builder.build().expect("build").mesh;
    assert!(!self_intersections(&crossing).is_empty());
}

/// Quads of the mesh folded along both diagonals: their rungs cross.
fn twisted_quads(mesh: &Mesh) -> Vec<FaceId> {
    mesh.faces()
        .filter(|&face| {
            let points = mesh
                .face_loop(face)
                .map(|c| {
                    let v = mesh.to_vertex(c).expect("corner vertex");
                    mesh.vertex_position(v).expect("position").map(f64::from)
                })
                .collect::<Vec<_>>();
            let [a, b, c, d] = points.as_slice() else {
                return false;
            };
            let (a, b, c, d) = (*a, *b, *c, *d);
            let normal = |p: [f64; 3], q: [f64; 3], r: [f64; 3]| cross(sub(q, p), sub(r, p));
            dot(normal(a, b, c), normal(a, c, d)) < 0.0
                && dot(normal(a, b, d), normal(b, c, d)) < 0.0
        })
        .collect()
}

/// Tiny deterministic generator for the fuzz.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / (1_u64 << 53) as f64)
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    fn direction(&mut self) -> [f64; 3] {
        let z = self.range(-1.0, 1.0);
        let azimuth = self.range(0.0, TAU);
        let r = libm::sqrt(1.0 - z * z);
        [r * libm::cos(azimuth), r * libm::sin(azimuth), z]
    }
}

/// Adds the junction when it is accepted, caps the outer ends, and returns
/// the closed mesh.
fn try_close(specs: &[Tube]) -> Result<Mesh, JunctionError> {
    let (mut mesh, seeds) = tubes(specs);
    let mut edit = mesh.edit();
    add_junction(&mut edit, &params(seeds))?;
    let _: () = edit.finish();
    let loops = mesh.boundary_loops().expect("boundary loops");
    let mut edit = mesh.edit();
    for boundary in &loops {
        let vertices = boundary
            .iter()
            .map(|&h| edit.mesh().from_vertex(h).expect("vertex"))
            .collect::<Vec<_>>();
        op::add_face(&mut edit, &vertices).expect("cap");
    }
    let _: () = edit.finish();
    Ok(mesh)
}

#[test]
fn accepted_random_junctions_are_clean_solids() {
    let mut accepted = 0;
    let trials = 400;
    for trial in 0..trials {
        let Ok(mesh) = try_close(&random_specs(trial)) else {
            continue;
        };
        accepted += 1;
        assert!(
            mesh.boundary_loops().expect("loops").is_empty(),
            "trial {trial} closed"
        );
        assert!(mesh.validate_deep().is_empty(), "trial {trial} valid");
        let v = mesh.vertices().count();
        let f = mesh.faces().count();
        let e = mesh.half_edges().count() / 2;
        assert_eq!(v + f, e + 2, "trial {trial} genus zero");
        assert!(signed_volume(&mesh) > 0.0, "trial {trial} outward");
        let hits = self_intersections(&mesh);
        assert!(
            hits.is_empty(),
            "trial {trial}: {} intersecting pairs",
            hits.len()
        );
        assert!(twisted_quads(&mesh).is_empty(), "trial {trial} twisted");
    }
    assert!(
        accepted >= trials / 8,
        "only {accepted} of {trials} accepted"
    );
}

/// The tubes of fuzz trial `trial`; a seed per trial keeps every case
/// reproducible on its own.
fn random_specs(trial: u64) -> Vec<Tube> {
    let mut rng = Rng(0xABCD_0000 + trial);
    let arms = 2 + (rng.next() % 5) as usize;
    (0..arms)
        .map(|_| {
            let start = rng.range(0.3, 1.2);
            Tube {
                direction: rng.direction(),
                start,
                radius: start * rng.range(0.03, 0.6),
                count: 5 + (rng.next() % 14) as usize,
            }
        })
        .collect()
}

#[test]
fn twisted_bridge_quads_are_refused() {
    // Fuzz trials whose bridge quads between a tiny and a large ring have
    // crossing rungs: no face-to-face test sees them.
    for trial in [7_u64, 1026] {
        assert!(
            matches!(
                try_close(&random_specs(trial)),
                Err(JunctionError::SkinTwisted { .. })
            ),
            "trial {trial}"
        );
    }
}

#[test]
fn a_skin_crossing_a_tube_wall_is_refused_before_any_edit() {
    // Two coaxial rings facing each other plan one clean strip. Folding one
    // wall quad of the upper tube back through the gap leaves the rings, and
    // so the plan, unchanged; only the wall test sees the crossing.
    let (mut mesh, seeds) = tubes(&[tube([0.0, 0.0, -1.0], 8), tube([0.0, 0.0, 1.0], 8)]);
    let outer = mesh
        .vertices()
        .find(|&v| mesh.vertex_position(v) == Some(&narrow([0.2, 0.0, 1.6])))
        .expect("upper tube's first outer vertex");
    let mut edit = mesh.edit();
    op::set_vertex_position(&mut edit, outer, [-0.5, 0.0, -0.3]).expect("move");
    let _: () = edit.finish();
    let faces = mesh.faces().count();
    let mut edit = mesh.edit();
    let result = add_junction(&mut edit, &params(seeds));
    let _: () = edit.finish();
    assert!(
        matches!(result, Err(JunctionError::SkinCrossesTube { .. })),
        "{result:?}"
    );
    assert_eq!(mesh.faces().count(), faces, "mesh unchanged");
}

#[test]
fn degenerate_triangles_are_their_edges() {
    let line = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let ids = [0_u32, 1, 2];
    // Crossing the line only in an axis-aligned projection is no contact.
    let above = [[1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [3.0, 0.0, 5.0]];
    assert!(!intersect::triangles_intersect(
        ids,
        &line,
        [3, 4, 5],
        &above
    ));
    assert!(!intersect::triangles_intersect(
        [3, 4, 5],
        &above,
        ids,
        &line
    ));
    // A triangle the line pierces.
    let pierced = [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 0.0, 1.0]];
    assert!(intersect::triangles_intersect(
        ids,
        &line,
        [3, 4, 5],
        &pierced
    ));
    // Sharing the line's end vertex: contact only there, or running inside.
    let beside = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    assert!(!intersect::triangles_intersect(
        ids,
        &line,
        [0, 4, 5],
        &beside
    ));
    let ahead = [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, -1.0, 0.0]];
    assert!(intersect::triangles_intersect(
        ids,
        &line,
        [0, 4, 5],
        &ahead
    ));
    // Two degenerate triangles on one line overlap or not.
    let overlapping = [[1.5, 0.0, 0.0], [3.0, 0.0, 0.0], [4.0, 0.0, 0.0]];
    assert!(intersect::triangles_intersect(
        ids,
        &line,
        [3, 4, 5],
        &overlapping
    ));
    let apart = [[2.5, 0.0, 0.0], [3.0, 0.0, 0.0], [4.0, 0.0, 0.0]];
    assert!(!intersect::triangles_intersect(
        ids,
        &line,
        [3, 4, 5],
        &apart
    ));
    // A parallel line in a common plane never meets.
    let parallel = [[0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [2.0, 1.0, 0.0]];
    assert!(!intersect::triangles_intersect(
        ids,
        &line,
        [3, 4, 5],
        &parallel
    ));
}

#[test]
fn a_fan_of_three_arms_is_refused_before_a_bridge_crosses_the_middle() {
    let specs = [
        Tube {
            radius: 0.05,
            ..tube(fork(0.0), 12)
        },
        Tube {
            radius: 0.05,
            ..tube(fork(30.0), 12)
        },
        Tube {
            radius: 0.05,
            ..tube(fork(58.0), 12)
        },
    ];
    assert_eq!(
        try_close(&specs).err(),
        Some(JunctionError::BridgeCrossesArm {
            a: 0,
            b: 2,
            through: 1
        })
    );
}

#[test]
fn an_acute_branch_beyond_the_trunk_ring_is_refused() {
    let specs = [
        tube([0.0, 0.0, -1.0], 12),
        tube([0.0, 0.0, 1.0], 12),
        Tube {
            start: 1.2,
            radius: 0.12,
            ..tube(fork(40.0), 12)
        },
    ];
    assert!(matches!(
        try_close(&specs),
        Err(JunctionError::RingBeyondNeighbour { .. })
    ));
}

#[test]
fn nearly_touching_collars_are_refused() {
    // Arms 32 degrees apart whose collars sum to about 27 degrees.
    let radius = 0.6 * libm::tan(13.5_f64.to_radians());
    let specs = [
        tube([0.0, 0.0, -1.0], 12),
        Tube {
            radius,
            ..tube(fork(-16.0), 12)
        },
        Tube {
            radius,
            ..tube(fork(16.0), 12)
        },
    ];
    assert_eq!(
        try_close(&specs).err(),
        Some(JunctionError::OverlappingCollars { a: 1, b: 2 })
    );
}

/// A planar ring around `center` with outward normal `normal`, in boundary
/// order: clockwise seen from outside.
fn tilted_ring(center: [f64; 3], normal: [f64; 3], radius: f64, count: usize) -> Vec<[f64; 3]> {
    let n = unit(normal);
    let (e1, e2) = frame(n);
    (0..count)
        .map(|k| {
            let angle = -TAU * k as f64 / count as f64;
            add(
                center,
                add(
                    scale(e1, radius * libm::cos(angle)),
                    scale(e2, radius * libm::sin(angle)),
                ),
            )
        })
        .collect()
}

#[test]
fn tilted_rings_report_the_cone_they_occupy() {
    let direction = [0.0, 0.0, 1.0];
    let tilt = 50.0_f64.to_radians();
    let ring = tilted_ring(
        scale(direction, 0.3),
        [libm::sin(tilt), 0.0, libm::cos(tilt)],
        0.2,
        16,
    );
    let arm = Arm::new(0, &ring, [0.0; 3]).expect("arm");
    let brute = ring
        .iter()
        .map(|p| angle_between(*p, arm.direction))
        .fold(0.0, f64::max);
    assert_eq!(arm.angular_radius, brute);
    assert!(
        arm.angular_radius > atan2(0.2, 0.3) + 0.1,
        "a tilted ring near the node occupies more than its perpendicular estimate"
    );
}

/// Open tubes whose inner rings sit at `centers` with outward `normals`, so
/// a ring may lean against its arm (the direction from the node to its
/// center). Each tube extends along its normal.
fn leaning_tubes(rings: &[([f64; 3], [f64; 3], f64, usize)]) -> (Mesh, Vec<HalfEdgeId>) {
    let mut builder = MeshBuilder::new();
    let mut inner_rings = Vec::new();
    for &(center, normal, radius, count) in rings {
        let n = unit(normal);
        let (e1, e2) = frame(n);
        let ring = |offset: f64, builder: &mut MeshBuilder| {
            (0..count)
                .map(|k| {
                    let angle = TAU * k as f64 / count as f64;
                    let p = add(
                        add(center, scale(n, offset)),
                        add(
                            scale(e1, radius * libm::cos(angle)),
                            scale(e2, radius * libm::sin(angle)),
                        ),
                    );
                    builder.push_vertex(narrow(p))
                })
                .collect::<Vec<_>>()
        };
        let inner = ring(0.0, &mut builder);
        let outer = ring(LENGTH, &mut builder);
        for k in 0..count {
            let k1 = (k + 1) % count;
            builder
                .add_face(&[inner[k], inner[k1], outer[k1], outer[k]])
                .expect("tube face");
        }
        inner_rings.push(inner);
    }
    let mesh = builder.build().expect("tubes build").mesh;
    let seeds = inner_rings
        .iter()
        .map(|ring| {
            mesh.half_edges()
                .find(|&h| {
                    mesh.face(h) == Some(FaceId::OUTSIDE)
                        && mesh.from_vertex(h).map(|v| v.index()) == Some(ring[1])
                        && mesh.to_vertex(h).map(|v| v.index()) == Some(ring[0])
                })
                .expect("inner boundary half-edge")
        })
        .collect();
    (mesh, seeds)
}

#[test]
fn rings_leaning_across_their_fork_are_refused_or_clean() {
    // The parent ring leans toward one child by increasing angles. Each
    // configuration is either refused or yields a clean closed shell; the
    // steepest leans are refused.
    let children = [fork(-35.0), fork(35.0)];
    let mut refused = 0;
    for tilt in [0.0_f64, 20.0, 40.0, 60.0, 70.0, 80.0] {
        let t = tilt.to_radians();
        let rings = [
            (
                [0.0, 0.0, -0.6],
                [libm::sin(t), 0.0, -libm::cos(t)],
                0.15,
                12,
            ),
            (scale(children[0], 0.6), children[0], 0.15, 12),
            (scale(children[1], 0.6), children[1], 0.15, 12),
        ];
        let (mut mesh, seeds) = leaning_tubes(&rings);
        let mut edit = mesh.edit();
        let result = add_junction(&mut edit, &params(seeds));
        let _: () = edit.finish();
        if result.is_err() {
            refused += 1;
            continue;
        }
        let loops = mesh.boundary_loops().expect("loops");
        let mut edit = mesh.edit();
        for boundary in &loops {
            let vertices = boundary
                .iter()
                .map(|&h| edit.mesh().from_vertex(h).expect("vertex"))
                .collect::<Vec<_>>();
            op::add_face(&mut edit, &vertices).expect("cap");
        }
        let _: () = edit.finish();
        assert!(mesh.validate_deep().is_empty(), "tilt {tilt} valid");
        assert!(self_intersections(&mesh).is_empty(), "tilt {tilt} clean");
    }
    assert!(refused >= 1, "the steepest leans are refused");
}

#[test]
fn t_junction_runs_face_their_neighbours() {
    let ring = |direction: [f64; 3], count: usize| {
        tilted_ring(scale(direction, 0.6), direction, 0.2, count)
    };
    let rings = [
        ring([-1.0, 0.0, 0.0], 12),
        ring([1.0, 0.0, 0.0], 12),
        ring([0.0, 0.0, 1.0], 12),
    ];
    let plan = plan_junction(&rings, [0.0; 3], None).expect("plan");
    for face in &plan.faces {
        let JunctionFaceOrigin::Bridge { rings: pair } = face.origin else {
            continue;
        };
        for vertex in &face.vertices {
            let JunctionVertex::Ring { ring: 0, index } = *vertex else {
                continue;
            };
            let z = rings[0][index][2];
            match pair {
                // Toward the upright arm: the ring's upper half.
                [0, 2] => assert!(z >= -1e-9, "vertex {index} at z {z}"),
                // Toward the opposite arm: underneath.
                [0, 1] => assert!(z <= 1e-9, "vertex {index} at z {z}"),
                _ => {}
            }
        }
    }
}
