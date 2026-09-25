// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use exedra_mesh::{ExtractParams, FaceId, HalfEdgeId, Mesh, MeshBuilder, attr, op};

use super::*;
use alloc::format;

/// One open tube: `count` vertices per ring, `length` long, starting `start`
/// from the origin along `direction`.
#[derive(Copy, Clone, Debug)]
struct Tube {
    direction: [f64; 3],
    start: f64,
    radius: f64,
    count: usize,
    /// Offset of the outer ring's center, so the tube leans and its wall
    /// meets the inner ring off its axis.
    lean: [f64; 3],
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
        let ring = |distance: f64, offset: [f64; 3], builder: &mut MeshBuilder| {
            (0..tube.count)
                .map(|k| {
                    let angle = TAU * k as f64 / tube.count as f64;
                    let p = add(
                        add(scale(d, distance), offset),
                        add(
                            scale(e1, tube.radius * libm::cos(angle)),
                            scale(e2, tube.radius * libm::sin(angle)),
                        ),
                    );
                    builder.push_vertex(narrow(p))
                })
                .collect::<Vec<_>>()
        };
        let inner = ring(tube.start, [0.0; 3], &mut builder);
        let outer = ring(tube.start + LENGTH, tube.lean, &mut builder);
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
        lean: [0.0; 3],
    }
}

fn params(seeds: Vec<HalfEdgeId>) -> JunctionParams {
    JunctionParams {
        center: [0.0; 3],
        rings: seeds,
        continue_uvs: false,
        smoothing: None,
        region: None,
    }
}

fn smoothed(seeds: Vec<HalfEdgeId>) -> JunctionParams {
    JunctionParams {
        smoothing: Some(JunctionSmoothing::default()),
        ..params(seeds)
    }
}

/// Adds the junction, caps the tubes' outer ends, and checks the solid.
fn close_and_check(specs: &[Tube]) -> (Mesh, JunctionOutput) {
    close_and_check_with(specs, params)
}

fn close_and_check_with(
    specs: &[Tube],
    params: impl Fn(Vec<HalfEdgeId>) -> JunctionParams,
) -> (Mesh, JunctionOutput) {
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
        .as_chunks::<3>()
        .0
        .iter()
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
    assert_eq!(output.skin_vertices.len(), 2);
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

/// Writes a cylindrical chart on every tube face: `U` turns `turns` times
/// around each tube (jumping across one seam edge), `V` runs along it.
/// Angles are measured about `axis` when given, else about each tube's own
/// direction.
fn chart_tubes(mesh: &mut Mesh, specs: &[Tube], turns: f32, axis: Option<[f64; 3]>) {
    let faces = mesh.faces().collect::<Vec<_>>();
    let tube_of = specs
        .iter()
        .flat_map(|spec| core::iter::repeat_n(spec, spec.count))
        .collect::<Vec<_>>();
    let mut edit = mesh.edit();
    for (face, spec) in faces.into_iter().zip(tube_of) {
        let corners = edit.mesh().face_loop(face).collect::<Vec<_>>();
        let points = corners
            .iter()
            .map(|&c| {
                let v = edit.mesh().to_vertex(c).expect("vertex");
                *edit.mesh().vertex_position(v).expect("position")
            })
            .collect::<Vec<_>>();
        let (e1, e2) = frame(axis.unwrap_or_else(|| unit(spec.direction)));
        let angles = points
            .iter()
            .map(|p| {
                let p = p.map(f64::from);
                let a = atan2(dot(p, e2), dot(p, e1));
                if a < 0.0 { a + TAU } else { a }
            })
            .collect::<Vec<_>>();
        let wraps = angles.iter().copied().fold(f64::MIN, f64::max)
            - angles.iter().copied().fold(f64::MAX, f64::min)
            > PI;
        for ((corner, angle), p) in corners.into_iter().zip(angles).zip(&points) {
            let angle = if wraps && angle < PI {
                angle + TAU
            } else {
                angle
            };
            let along = norm(p.map(f64::from));
            let uv = [angle / TAU * f64::from(turns), along];
            op::set_corner_uv(&mut edit, corner, narrow_uv(uv)).expect("tube uv");
        }
    }
    let _: () = edit.finish();
}

/// Checks that each skin corner on a ring edge holds exactly the UV the tube
/// face across that edge holds at the same vertex. With `period`, a ring
/// edge may instead be off by whole multiples of it in U, as a coarse bridge
/// face that a seam cut crosses is.
fn assert_uvs_continue_tubes(mesh: &Mesh, output: &JunctionOutput, period: Option<f32>) {
    let layer = mesh.attrs().sparse(attr::CORNER_UV).expect("uv layer");
    let mut checked = 0;
    for &face in &output.faces {
        for edge in mesh.face_loop(face) {
            let twin = mesh.twin(edge).expect("twin");
            if output.faces.contains(&mesh.face(twin).expect("face")) {
                continue;
            }
            // Ring edge a -> b in the skin; the tube's twin runs b -> a.
            let skin_prev = mesh.prev(edge).expect("prev");
            let tube_prev = mesh.prev(twin).expect("prev");
            let skin_b = layer.get(edge.into()).expect("skin uv");
            let skin_a = layer.get(skin_prev.into()).expect("skin uv");
            let tube_a = layer.get(twin.into()).expect("tube uv");
            let tube_b = layer.get(tube_prev.into()).expect("tube uv");
            let offset = skin_a[0] - tube_a[0];
            let whole = period.map_or(0.0, |period| libm::roundf(offset / period) * period);
            if whole == 0.0 {
                assert_eq!((skin_a, skin_b), (tube_a, tube_b), "ring edge {edge:?}");
            } else {
                for (skin, tube) in [(skin_a, tube_a), (skin_b, tube_b)] {
                    assert!(
                        (skin[0] - whole - tube[0]).abs() < 1e-4 && skin[1] == tube[1],
                        "ring edge {edge:?}: {skin:?} vs {tube:?}"
                    );
                }
            }
            checked += 1;
        }
    }
    assert!(checked > 0);
    for &face in &output.faces {
        for corner in mesh.face_loop(face) {
            let uv = layer.get(corner.into()).expect("skin uv");
            assert!(uv.iter().all(|c| c.is_finite()));
        }
    }
}

#[test]
fn uvs_continue_every_tube_across_the_rings() {
    let specs = [
        tube([0.0, 0.0, -1.0], 12),
        tube(fork(-35.0), 10),
        tube(fork(35.0), 12),
    ];
    for (smoothing, period) in [
        (None, Some(3.0)),
        (Some(JunctionSmoothing::default()), None),
    ] {
        let (mut mesh, seeds) = tubes(&specs);
        chart_tubes(&mut mesh, &specs, 3.0, None);
        let mut edit = mesh.edit();
        let output = add_junction(
            &mut edit,
            &JunctionParams {
                continue_uvs: true,
                smoothing,
                region: Some(7),
                ..params(seeds)
            },
        )
        .expect("junction");
        let _: () = edit.finish();
        assert_uvs_continue_tubes(&mesh, &output, period);
        let regions = mesh.attrs().dense(attr::FACE_REGION).expect("regions");
        for &face in &output.faces {
            assert_eq!(regions.get(face.into()), Some(&7));
        }
    }
}

#[test]
fn balanced_tube_seams_leave_no_jump_inside_a_straight_joint() {
    // Two coaxial tubes whose charts continue each other: the skin's U must
    // stay within one face's reach of the tubes' values, with no vortex.
    let specs = [tube([0.0, 0.0, -1.0], 12), tube([0.0, 0.0, 1.0], 12)];
    let (mut mesh, seeds) = tubes(&specs);
    chart_tubes(&mut mesh, &specs, 1.0, Some([0.0, 0.0, 1.0]));
    let mut edit = mesh.edit();
    let output = add_junction(
        &mut edit,
        &JunctionParams {
            continue_uvs: true,
            ..smoothed(seeds)
        },
    )
    .expect("junction");
    let _: () = edit.finish();
    assert_uvs_continue_tubes(&mesh, &output, None);
    let layer = mesh.attrs().sparse(attr::CORNER_UV).expect("uv layer");
    for &face in &output.faces {
        let us = mesh
            .face_loop(face)
            .map(|c| layer.get(c.into()).expect("uv")[0])
            .collect::<Vec<_>>();
        let spread = us.iter().copied().fold(f32::MIN, f32::max)
            - us.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread < 0.25, "U spread {spread} in one face");
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
    // Tubes without UVs cannot be continued.
    let continue_uvs = JunctionParams {
        continue_uvs: true,
        ..params(seeds.clone())
    };
    assert_eq!(
        add_junction(&mut edit, &continue_uvs),
        Err(JunctionError::MissingTubeUvs { ring: 0 })
    );
    // Tangents must leave each ring toward the junction.
    let outward = JunctionParams {
        smoothing: Some(JunctionSmoothing {
            tangents: Some(vec![vec![[0.0, 0.0, 1.0]; 8], vec![[0.0, 0.0, 1.0]; 8]]),
            ..JunctionSmoothing::default()
        }),
        ..params(seeds.clone())
    };
    assert_eq!(
        add_junction(&mut edit, &outward),
        Err(JunctionError::InvalidSmoothing { ring: Some(1) })
    );
    let missing = JunctionParams {
        smoothing: Some(JunctionSmoothing {
            tangents: Some(vec![vec![[0.0, 0.0, 1.0]; 8]]),
            ..JunctionSmoothing::default()
        }),
        ..params(seeds)
    };
    assert_eq!(
        add_junction(&mut edit, &missing),
        Err(JunctionError::InvalidSmoothing { ring: None })
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
        &JunctionOptions::default(),
    )
    .expect("plan");
    assert_eq!(plan.skin_vertices.len(), 2);
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
        &JunctionOptions::default(),
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
    try_close_with(specs, params)
}

fn try_close_with(
    specs: &[Tube],
    params: impl Fn(Vec<HalfEdgeId>) -> JunctionParams,
) -> Result<Mesh, JunctionError> {
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
                lean: [0.0; 3],
            }
        })
        .collect()
}

/// Fuzz trial `trial` with leaning tubes: each outer ring is offset
/// sideways by up to half the tube length, so the walls meet the rings off
/// their axes and smoothing follows measured tangents.
fn random_bent_specs(trial: u64) -> Vec<Tube> {
    let mut rng = Rng(0x5EED_0000 + trial);
    let arms = 2 + (rng.next() % 5) as usize;
    (0..arms)
        .map(|_| {
            let start = rng.range(0.3, 1.2);
            let direction = rng.direction();
            let (e1, e2) = frame(unit(direction));
            let (lean, turn) = (rng.range(0.0, 0.5) * LENGTH, rng.range(0.0, TAU));
            Tube {
                direction,
                start,
                radius: start * rng.range(0.03, 0.6),
                count: 5 + (rng.next() % 14) as usize,
                lean: add(
                    scale(e1, lean * libm::cos(turn)),
                    scale(e2, lean * libm::sin(turn)),
                ),
            }
        })
        .collect()
}

fn smoothed_rows(rows: u32) -> impl Fn(Vec<HalfEdgeId>) -> JunctionParams {
    move |seeds| JunctionParams {
        smoothing: Some(JunctionSmoothing {
            rows: NonZeroU32::new(rows).expect("nonzero"),
            tangents: None,
        }),
        ..params(seeds)
    }
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
    let plan = plan_junction(&rings, [0.0; 3], &JunctionOptions::default()).expect("plan");
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

/// Angles between the normals of each skin face and the tube face across
/// one of its ring edges, sorted.
fn ring_creases(mesh: &Mesh, output: &JunctionOutput) -> Vec<f64> {
    let normal = |face: FaceId| {
        let points = mesh
            .face_loop(face)
            .map(|c| {
                let v = mesh.to_vertex(c).expect("vertex");
                mesh.vertex_position(v).expect("position").map(f64::from)
            })
            .collect::<Vec<_>>();
        let mut n = [0.0; 3];
        for (k, p) in points.iter().enumerate() {
            n = add(n, cross(*p, points[(k + 1) % points.len()]));
        }
        unit(n)
    };
    let mut creases = Vec::new();
    for &face in &output.faces {
        for edge in mesh.face_loop(face) {
            let other = mesh.face(mesh.twin(edge).expect("twin")).expect("face");
            if !output.faces.contains(&other) {
                creases.push(angle_between(normal(face), normal(other)).to_degrees());
            }
        }
    }
    creases.sort_by(f64::total_cmp);
    creases
}

#[test]
fn smoothing_leaves_each_ring_along_its_tube_wall() {
    let specs = [
        tube([0.0, 0.0, -1.0], 16),
        tube(fork(-32.0), 12),
        tube(fork(38.0), 12),
    ];
    let (coarse_mesh, coarse) = close_and_check(&specs);
    let (smooth_mesh, smooth) = close_and_check_with(&specs, smoothed);
    let before = ring_creases(&coarse_mesh, &coarse);
    let after = ring_creases(&smooth_mesh, &smooth);
    let median = |c: &[f64]| c[c.len() / 2];
    let high = |c: &[f64]| c[c.len() * 9 / 10];
    // The coarse skin creases along every ring; smoothing halves the
    // typical crease. The largest remain at tight crotches, where a few rows
    // must turn between close collars, and are of the order of the tubes'
    // own facet angle (30 degrees for twelve sides).
    assert!(
        median(&after) < 0.6 * median(&before),
        "median crease {:.1} -> {:.1} degrees",
        median(&before),
        median(&after)
    );
    assert!(
        high(&after) < 0.7 * high(&before),
        "90th percentile crease {:.1} -> {:.1} degrees",
        high(&before),
        high(&after)
    );
    assert!(smooth.faces.len() > coarse.faces.len());
    assert_eq!(smooth.stats, coarse.stats, "stats describe the coarse skin");
}

#[test]
fn smoothed_plans_keep_ring_edges_and_add_rows() {
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
    let rings = [
        ring([0.0, 0.0, -1.0], 12),
        ring(fork(-35.0), 10),
        ring(fork(35.0), 12),
    ];
    let coarse = plan_junction(&rings, [0.0; 3], &JunctionOptions::default()).expect("plan");
    for rows in [1, 2, 5] {
        let plan = plan_junction(
            &rings,
            [0.0; 3],
            &JunctionOptions {
                smoothing: Some(JunctionSmoothing {
                    rows: NonZeroU32::new(rows).expect("nonzero"),
                    tangents: None,
                }),
                ..JunctionOptions::default()
            },
        )
        .expect("smoothed plan");
        if rows == 1 {
            assert_eq!(plan.faces.len(), coarse.faces.len());
        } else {
            assert!(plan.faces.len() > coarse.faces.len() * (rows as usize - 1));
        }
        // Crotch centers stay first and move only by fairing.
        assert!(plan.skin_vertices.len() >= coarse.skin_vertices.len());
        for (ring, points) in rings.iter().enumerate() {
            for index in 0..points.len() {
                let from = JunctionVertex::Ring { ring, index };
                let to = JunctionVertex::Ring {
                    ring,
                    index: (index + 1) % points.len(),
                };
                let uses = plan
                    .faces
                    .iter()
                    .filter(|face| {
                        let n = face.vertices.len();
                        (0..n).any(|k| face.vertices[k] == from && face.vertices[(k + 1) % n] == to)
                    })
                    .count();
                assert_eq!(uses, 1, "rows {rows}: ring {ring} edge {index}");
            }
        }
    }
}

/// Checks a closed junction solid: closed, manifold, genus zero, outward,
/// and free of self-intersections and twisted quads.
fn assert_clean_solid(mesh: &Mesh, label: &str) {
    assert!(
        mesh.boundary_loops().expect("loops").is_empty(),
        "{label} closed"
    );
    assert!(mesh.validate_deep().is_empty(), "{label} valid");
    let v = mesh.vertices().count();
    let f = mesh.faces().count();
    let e = mesh.half_edges().count() / 2;
    assert_eq!(v + f, e + 2, "{label} genus zero");
    assert!(signed_volume(mesh) > 0.0, "{label} outward");
    let hits = self_intersections(mesh);
    assert!(
        hits.is_empty(),
        "{label}: {} intersecting pairs",
        hits.len()
    );
    assert!(twisted_quads(mesh).is_empty(), "{label} twisted");
}

#[test]
fn accepted_random_smoothed_junctions_are_clean_solids() {
    // Straight and leaning tubes, with every row count from one to six.
    // A junction the coarse skin accepts is either smoothed cleanly or
    // refused with a smoothed-skin variant; nothing else may change.
    let mut accepted = 0;
    let trials = 120;
    for (label, specs) in [
        ("straight", random_specs as fn(u64) -> Vec<Tube>),
        ("bent", random_bent_specs),
    ] {
        for trial in 0..trials {
            let specs = specs(trial);
            let rows = 1 + (trial % 6) as u32;
            let coarse = try_close(&specs);
            match try_close_with(&specs, smoothed_rows(rows)) {
                Ok(mesh) => {
                    assert!(coarse.is_ok(), "{label} trial {trial}: smoothing only adds");
                    accepted += 1;
                    assert_clean_solid(&mesh, &format!("{label} trial {trial} rows {rows}"));
                }
                Err(
                    JunctionError::SmoothedSkinTwisted { .. }
                    | JunctionError::SmoothedSkinSelfIntersects { .. },
                ) => assert!(coarse.is_ok()),
                Err(error) => assert_eq!(
                    coarse.err(),
                    Some(error),
                    "{label} trial {trial}: smoothing refuses only smoothed skins"
                ),
            }
        }
    }
    assert!(accepted >= trials / 4, "only {accepted} accepted");
}

#[test]
fn leaning_tubes_close_cleanly_without_smoothing() {
    let mut accepted = 0;
    let trials = 200;
    for trial in 0..trials {
        let Ok(mesh) = try_close(&random_bent_specs(trial)) else {
            continue;
        };
        accepted += 1;
        assert_clean_solid(&mesh, &format!("bent trial {trial}"));
    }
    assert!(
        accepted >= trials / 8,
        "only {accepted} of {trials} accepted"
    );
}

/// Rings of a Y fork for plan-only smoothing tests, and tangents pointing
/// radially inward across each ring (with a little slope toward the
/// junction), which the fairing follows into the crotch.
/// Per-ring points or directions.
type RingVectors = Vec<Vec<[f64; 3]>>;

fn inward_tangent_fork(half_angle: f64) -> (RingVectors, RingVectors) {
    let ring = |direction: [f64; 3], radius: f64| {
        let d = unit(direction);
        let (e1, e2) = frame(d);
        (0..12)
            .map(|k| {
                let angle = -TAU * f64::from(k) / 12.0;
                add(
                    scale(d, 0.6),
                    add(
                        scale(e1, radius * libm::cos(angle)),
                        scale(e2, radius * libm::sin(angle)),
                    ),
                )
            })
            .collect::<Vec<_>>()
    };
    let rings = vec![
        ring([0.0, 0.0, -1.0], 0.2),
        ring(fork(-half_angle), 0.15),
        ring(fork(half_angle), 0.15),
    ];
    let tangents = rings
        .iter()
        .map(|points| {
            let centroid = points
                .iter()
                .fold([0.0; 3], |sum, p| add(sum, *p))
                .map(|c| c / points.len() as f64);
            let toward_junction = unit(scale(centroid, -1.0));
            points
                .iter()
                .map(|p| unit(add(unit(sub(centroid, *p)), scale(toward_junction, 0.1))))
                .collect()
        })
        .collect();
    (rings, tangents)
}

fn plan_smoothed(
    rings: &[Vec<[f64; 3]>],
    tangents: RingVectors,
    rows: u32,
) -> Result<JunctionPlan, JunctionError> {
    plan_junction(
        rings,
        [0.0; 3],
        &JunctionOptions {
            smoothing: Some(JunctionSmoothing {
                rows: NonZeroU32::new(rows).expect("nonzero"),
                tangents: Some(tangents),
            }),
            ..JunctionOptions::default()
        },
    )
}

#[test]
fn smoothed_skins_that_fold_are_refused() {
    // Tangents aimed across the rings pull the rows into the crotch: with a
    // wide fork and two rows, rows from both branches cross.
    let (rings, tangents) = inward_tangent_fork(45.0);
    assert!(plan_junction(&rings, [0.0; 3], &JunctionOptions::default()).is_ok());
    assert!(matches!(
        plan_smoothed(&rings, tangents, 2),
        Err(JunctionError::SmoothedSkinSelfIntersects { .. })
    ));
    // With a narrow fork and eight rows, the crotch rows fold inside a quad.
    let (rings, tangents) = inward_tangent_fork(20.0);
    assert!(matches!(
        plan_smoothed(&rings, tangents, 8),
        Err(JunctionError::SmoothedSkinTwisted { .. })
    ));

    // Measured tube tangents fold one fuzz junction at two rows; the refusal
    // leaves the mesh unchanged, and the coarse skin still closes it.
    let specs = random_specs(189);
    let (mut mesh, seeds) = tubes(&specs);
    let faces_before = mesh.faces().count();
    let mut edit = mesh.edit();
    assert!(matches!(
        add_junction(&mut edit, &smoothed_rows(2)(seeds)),
        Err(JunctionError::SmoothedSkinTwisted { .. })
    ));
    let _: () = edit.finish();
    assert_eq!(mesh.faces().count(), faces_before);
    assert!(try_close(&specs).is_ok());
}

#[test]
fn caller_layers_follow_the_harmonic_ring_weights() {
    use exedra_mesh::attributes::{AttrKey, Domain, Propagation};

    const HEAT: AttrKey<f32> = AttrKey::new(Domain::Vertex, "vertex.heat");
    const SHADE: AttrKey<f32> = AttrKey::new(Domain::HalfEdge, "corner.shade");
    let specs = [
        tube([0.0, 0.0, -1.0], 12),
        tube(fork(-35.0), 10),
        tube(fork(35.0), 12),
    ];
    // Builder vertices are laid out tube by tube, two rings each.
    let tube_of = |vertex: VertexId| {
        let mut index = vertex.index() as usize;
        specs
            .iter()
            .position(|spec| {
                let inside = index < 2 * spec.count;
                index = index.saturating_sub(2 * spec.count);
                inside
            })
            .expect("a tube vertex") as f32
    };
    for smoothing in [None, Some(JunctionSmoothing::default())] {
        let (mut mesh, seeds) = tubes(&specs);
        mesh.define_sparse_layer(HEAT).expect("heat");
        mesh.define_sparse_layer(SHADE).expect("shade");
        mesh.set_layer_propagation(HEAT, Propagation::Interpolate)
            .expect("heat rule");
        mesh.set_layer_propagation(SHADE, Propagation::Interpolate)
            .expect("shade rule");
        let vertices = mesh.vertices().collect::<Vec<_>>();
        let faces = mesh.faces().collect::<Vec<_>>();
        let corners = faces
            .iter()
            .flat_map(|&face| mesh.face_loop(face))
            .collect::<Vec<_>>();
        let mut edit = mesh.edit();
        // Each tube carries its own constants (heat 1, 2 and 5, so an even
        // blend of all three is not one of them), every blend stays within
        // the tubes' range, and a ring corner keeps its tube's value.
        for &vertex in &vertices {
            let tube = tube_of(vertex);
            op::set_attribute(&mut edit, HEAT, vertex, 1.0 + tube * tube).expect("heat");
        }
        for &corner in &corners {
            let vertex = edit.mesh().to_vertex(corner).expect("vertex");
            op::set_attribute(&mut edit, SHADE, corner, 10.0 + tube_of(vertex)).expect("shade");
        }
        let output = add_junction(
            &mut edit,
            &JunctionParams {
                smoothing,
                ..params(seeds)
            },
        )
        .expect("junction");
        let _: () = edit.finish();

        let heat = mesh.attrs().sparse(HEAT).expect("heat");
        let shade = mesh.attrs().sparse(SHADE).expect("shade");
        let mut blended = false;
        for &vertex in &output.skin_vertices {
            let value = *heat.get(vertex.as_id()).expect("skin vertex heat");
            assert!((1.0..=5.0).contains(&value), "heat {value}");
            blended |= ![1.0, 2.0, 5.0].contains(&value);
        }
        assert!(blended, "some skin vertex blends the tubes");
        for &face in &output.faces {
            for corner in mesh.face_loop(face) {
                let value = *shade.get(corner.as_id()).expect("skin corner shade");
                assert!((10.0..=12.0).contains(&value), "shade {value}");
                let vertex = mesh.to_vertex(corner).expect("vertex");
                if !output.skin_vertices.contains(&vertex) {
                    assert_eq!(value, 10.0 + tube_of(vertex), "ring corner");
                }
            }
        }
    }
}
