// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;
use core::f64::consts::PI;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);

#[test]
fn refused_depth_reports_the_actual_remaining_material_and_requirement() {
    let policy = ProfilePolicy::new(0.5);
    let pocket = Pocket::from_region(
        &hole_rect([1.0, 1.0], [4.0, 4.0]).reversed(),
        End::End,
        1.75,
    );
    assert!(matches!(
        plate().with_pocket(pocket, &policy),
        Err(ExtrusionError::Pocket {
            pocket: 0,
            error: PocketError::InvalidDepth {
                depth: 1.75,
                length: 2.0,
                remaining: 0.25,
                min_separation: 0.5
            }
        })
    ));
    let first = Pocket::from_region(&hole_rect([1.0, 1.0], [4.0, 4.0]).reversed(), End::End, 1.0);
    let host = plate().with_pocket(first, &policy).unwrap();
    let second = Pocket::from_region(
        &hole_rect([2.0, 2.0], [5.0, 5.0]).reversed(),
        End::Start,
        1.0,
    );
    let error = host.with_pocket(second, &policy).unwrap_err();
    assert!(matches!(
        error,
        ExtrusionError::Pocket {
            pocket: 1,
            error: PocketError::BreaksThrough {
                other: 0,
                remaining: 0.0,
                min_separation: 0.5
            }
        }
    ));
    assert!(alloc::format!("{error}").contains("requiring at least 0.5"));
}

#[test]
fn whole_pocket_labels_survive_reordering_and_mesh_correspondence() {
    const RECESS: SegTag = SegTag(100);
    const INSERT: SegTag = SegTag(101);
    let recess = Pocket::from_region(&hole_rect([1.0, 1.0], [6.0, 5.0]).reversed(), End::End, 0.5)
        .tagged(RECESS);
    let bore = Pocket::from_region(&hole_circle([3.0, 3.0], 1.0).reversed(), End::Start, 1.0)
        .tagged(INSERT);
    for pockets in [[recess.clone(), bore.clone()], [bore, recess]] {
        let mut solid = plate();
        for pocket in pockets {
            solid = solid.with_pocket(pocket, &POLICY).unwrap();
        }
        let faces: Vec<_> = solid.faces_tagged(RECESS).collect();
        assert_eq!(faces.len(), 5, "four walls and the floor");
        let floor = faces
            .iter()
            .find(|(_, face)| matches!(face.role, FaceRole::Floor { .. }))
            .unwrap()
            .0;
        assert_eq!(solid.face_area(floor), Some(20.0));
        let edges: Vec<_> = solid.edges_tagged(RECESS).collect();
        assert_eq!(edges.len(), 12, "both four-edge rims and four seams");
        let realized = solid
            .tessellate_with_regions(&TessellationPolicy::default(), |id, _| {
                u32::from(solid.face_tag(id) == Some(RECESS))
            })
            .unwrap();
        let regions = realized
            .mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .unwrap();
        for &(mesh_face, native) in &realized.mesh_faces {
            assert_eq!(
                regions.get(mesh_face.as_id()),
                Some(&u32::from(solid.face_tag(native) == Some(RECESS)))
            );
        }
        for (edge, _) in edges {
            assert!(realized.edge_chains[edge.index()].len() >= 2);
        }
    }
}

#[test]
fn repeated_pocket_labels_select_all_instances_without_erasing_segment_sources() {
    const FEATURE: SegTag = SegTag(200);
    const SEGMENT: SegTag = SegTag(201);
    let region = hole_rect([1.0, 1.0], [2.0, 2.0]).reversed();
    let labelled = Loop::new(
        region
            .segments()
            .iter()
            .copied()
            .map(|s| s.tagged(SEGMENT))
            .collect(),
    )
    .unwrap();
    let solid = plate()
        .with_pocket(
            Pocket::from_region(&labelled, End::End, 0.5).tagged(FEATURE),
            &POLICY,
        )
        .unwrap()
        .with_pocket(
            Pocket::from_region(&hole_rect([4.0, 1.0], [5.0, 2.0]).reversed(), End::End, 0.5)
                .tagged(FEATURE),
            &POLICY,
        )
        .unwrap();
    assert_eq!(solid.faces_tagged(FEATURE).count(), 10);
    assert_eq!(solid.edges_tagged(FEATURE).count(), 24);
    assert!(solid.faces().iter().any(|face| matches!(
        face.role,
        FaceRole::Wall {
            tag: Some(SEGMENT),
            ..
        }
    )));
    assert_eq!(solid.face_tag(FaceId(u32::MAX)), None);
    assert_eq!(solid.edge_tag(EdgeId(u32::MAX)), None);
}

/// A clockwise rectangle, as a hole or pocket outline.
fn hole_rect(min: [f64; 2], max: [f64; 2]) -> Loop {
    Loop::new(vec![
        Segment::line([min[0], max[1]]),
        Segment::line(max),
        Segment::line([max[0], min[1]]),
        Segment::line(min),
    ])
    .unwrap()
}

/// A clockwise circle, as a hole or a blind bore.
fn hole_circle(center: [f64; 2], radius: f64) -> Loop {
    let east = [center[0] + radius, center[1]];
    let west = [center[0] - radius, center[1]];
    Loop::new(vec![Segment::arc(west, -1.0), Segment::arc(east, -1.0)]).unwrap()
}

/// A 10 x 6 plate, 2 thick, with a through hole of radius 0.5 at (8, 3).
fn plate() -> Extrusion {
    let outer = hole_rect([0.0, 0.0], [10.0, 6.0]).reversed();
    let profile = Profile::new(outer, vec![hole_circle([8.0, 3.0], 0.5)], &POLICY).unwrap();
    Extrusion::new(profile, Placement3::IDENTITY, 2.0).unwrap()
}

/// The plate with a recess in its end face and a blind bore in its start.
fn pocketed() -> Extrusion {
    plate()
        .with_pocket(
            Pocket {
                tag: None,
                outline: hole_rect([1.0, 1.0], [6.0, 5.0]),
                end: End::End,
                depth: 0.5,
            },
            &POLICY,
        )
        .unwrap()
        .with_pocket(
            Pocket {
                tag: None,
                outline: hole_circle([3.0, 3.0], 1.0),
                end: End::Start,
                depth: 1.0,
            },
            &POLICY,
        )
        .unwrap()
}

#[test]
fn pockets_add_walls_floors_and_cap_holes_and_stay_closed() {
    let solid = pocketed();
    assert!(solid.is_closed());
    // Two caps, 4 + 2 profile walls, 4 + 2 pocket walls, two floors.
    assert_eq!(solid.faces().len(), 2 + 6 + 6 + 2);
    let end_cap = &solid.faces()[1];
    assert_eq!(end_cap.role, FaceRole::Cap(End::End));
    assert_eq!(end_cap.loops.len(), 3, "outline, through hole and recess");
    assert_eq!(
        solid.faces()[0].loops.len(),
        3,
        "outline, through hole and bore"
    );
    for (index, expected_normal, expected_z) in
        [(14, [0.0, 0.0, 1.0], 1.5), (15, [0.0, 0.0, -1.0], 1.0)]
    {
        assert_eq!(
            solid.faces()[index].role,
            FaceRole::Floor { pocket: index - 14 }
        );
        let sample = solid
            .evaluate(FaceId(crate::usize_to_u32(index)), [2.0, 2.0])
            .unwrap();
        assert_eq!(sample.normal, expected_normal);
        assert_eq!(sample.point[2], expected_z);
    }
    // The bore's walls face into the bore, toward its axis.
    let bore_wall = solid
        .faces()
        .iter()
        .find(|face| {
            matches!(face.role, FaceRole::Wall { segment, .. } if segment.boundary == Boundary::Pocket(1))
        })
        .unwrap();
    assert!(
        matches!(&bore_wall.surface, Surface::Cylinder(c) if c.radius() == 1.0)
            && bore_wall.sense == Sense::Reverse
    );
}

#[test]
fn measures_subtract_each_pocket() {
    let solid = pocketed();
    let profile_area = 60.0 - 0.25 * PI;
    let expected = 2.0 * profile_area - 0.5 * 20.0 - 1.0 * PI;
    assert!((solid.volume() - expected).abs() < 1e-12);
    let area = |face: usize| solid.face_area(FaceId(crate::usize_to_u32(face))).unwrap();
    assert!((area(1) - (profile_area - 20.0)).abs() < 1e-12);
    assert!((area(0) - (profile_area - PI)).abs() < 1e-12);
    assert!((area(14) - 20.0).abs() < 1e-12);
    assert!((area(15) - PI).abs() < 1e-12);
    // Floors evaluate on their plane and give a frame there, facing out.
    let floor = solid.evaluate(FaceId(14), [2.0, 2.0]).unwrap();
    assert_eq!(
        floor,
        SurfacePoint {
            point: [2.0, 2.0, 1.5],
            normal: [0.0, 0.0, 1.0]
        }
    );
    let frame = solid.attachment_frame(FaceId(15)).unwrap();
    assert_eq!(frame.transform_point([0.0; 3]), [0.0, 0.0, 1.0]);
    assert_eq!(frame.rows.map(|row| row[2]), [0.0, 0.0, -1.0]);
    // Pocket walls span the pocket's depth only.
    let recess_wall = (0..solid.faces().len())
        .find(|&face| {
            matches!(solid.faces()[face].role, FaceRole::Wall { segment, .. } if segment.boundary == Boundary::Pocket(0))
        })
        .unwrap();
    let wall = FaceId(crate::usize_to_u32(recess_wall));
    assert!(solid.evaluate(wall, [0.5, 1.75]).is_some());
    assert!(solid.evaluate(wall, [0.5, 1.0]).is_none());
}

#[test]
fn pockets_are_refused_with_the_failed_check() {
    let cut = |solid: Extrusion, outline: Loop, end: End, depth: f64| {
        solid.with_pocket(
            Pocket {
                tag: None,
                outline,
                end,
                depth,
            },
            &POLICY,
        )
    };
    let refused = |result: Result<Extrusion, ExtrusionError>| match result {
        Err(ExtrusionError::Pocket { error, .. }) => error,
        other => panic!("expected a pocket refusal, got {other:?}"),
    };
    for depth in [0.0, 2.0, -1.0, f64::NAN] {
        let error = refused(cut(
            plate(),
            hole_rect([1.0, 1.0], [2.0, 2.0]),
            End::End,
            depth,
        ));
        let PocketError::InvalidDepth {
            depth: supplied,
            length,
            remaining,
            min_separation,
        } = error
        else {
            panic!("{error:?}");
        };
        assert_eq!(supplied.to_bits(), depth.to_bits());
        assert_eq!(length, 2.0);
        assert_eq!(remaining.to_bits(), (2.0 - depth).to_bits());
        assert_eq!(min_separation, POLICY.min_separation);
    }
    // Counter-clockwise, crossing the outline, over the through hole.
    for outline in [
        hole_rect([1.0, 1.0], [2.0, 2.0]).reversed(),
        hole_rect([9.0, 1.0], [11.0, 2.0]),
        hole_circle([8.0, 3.0], 1.0),
    ] {
        assert!(matches!(
            refused(cut(plate(), outline, End::End, 0.5)),
            PocketError::Arrangement(_)
        ));
    }
    // Overlapping another pocket on the same end.
    let once = cut(plate(), hole_rect([1.0, 1.0], [4.0, 4.0]), End::End, 0.5).unwrap();
    assert!(matches!(
        refused(cut(
            once.clone(),
            hole_rect([3.0, 3.0], [5.0, 5.0]),
            End::End,
            0.5
        )),
        PocketError::Arrangement(_)
    ));
    // From the other end, overlapping regions must leave material between.
    assert_eq!(
        refused(cut(
            once.clone(),
            hole_rect([3.0, 3.0], [5.0, 5.0]),
            End::Start,
            1.5
        )),
        PocketError::BreaksThrough {
            other: 0,
            remaining: 0.0,
            min_separation: POLICY.min_separation
        }
    );
    let web = cut(once, hole_rect([3.0, 3.0], [5.0, 5.0]), End::Start, 1.0).unwrap();
    assert!(web.is_closed());
}

#[test]
fn pocketed_solids_realize_closed_meshes_with_their_volume() {
    for solid in [pocketed(), {
        // Overlapping pockets from both ends leave a thin web.
        plate()
            .with_pocket(
                Pocket {
                    tag: None,
                    outline: hole_rect([1.0, 1.0], [4.0, 4.0]),
                    end: End::End,
                    depth: 0.75,
                },
                &POLICY,
            )
            .unwrap()
            .with_pocket(
                Pocket {
                    tag: None,
                    outline: hole_circle([3.0, 3.0], 1.5),
                    end: End::Start,
                    depth: 1.0,
                },
                &POLICY,
            )
            .unwrap()
    }] {
        let realized = solid.tessellate(&TessellationPolicy::default()).unwrap();
        let mesh = &realized.mesh;
        assert!(mesh.validate_deep().is_empty());
        assert!(mesh.boundary_loops().unwrap().is_empty());
        assert!(realized.report.within_tolerance());
        // Independent oracle: each boundary's sampled polygon area times its
        // span. Profile holes and pockets wind clockwise, so they subtract.
        let mut counts = realized.report.segment_edges.iter();
        let mut expected = 0.0;
        for band in solid.bands() {
            let loop_ = solid.boundary_loop(band.boundary);
            let mut points = Vec::new();
            for (index, (from, _)) in loop_.iter_with_starts().enumerate() {
                let count = *counts.next().unwrap();
                match loop_.arc(index) {
                    Some(arc) => points
                        .extend((0..count).map(|i| arc.point_at(f64::from(i) / f64::from(count)))),
                    None => points.push(from),
                }
            }
            let area: f64 = (0..points.len())
                .map(|i| {
                    let (a, b) = (points[i], points[(i + 1) % points.len()]);
                    0.5 * (a[0] * b[1] - a[1] * b[0])
                })
                .sum();
            expected += area * (band.heights[1] - band.heights[0]);
        }
        let mesh_volume: f64 = mesh
            .faces()
            .map(|face| {
                let corners: Vec<[f64; 3]> = mesh
                    .face_loop(face)
                    .map(|h| {
                        exedra_math::promote(
                            *mesh.vertex_position(mesh.to_vertex(h).unwrap()).unwrap(),
                        )
                    })
                    .collect();
                (1..corners.len() - 1)
                    .map(|i| {
                        exedra_math::dot(corners[0], exedra_math::cross(corners[i], corners[i + 1]))
                            / 6.0
                    })
                    .sum::<f64>()
            })
            .sum();
        assert!(
            (mesh_volume - expected).abs() <= 1e-5 * expected,
            "{mesh_volume} vs {expected}"
        );
        // Inscribed samples add volume at holes and pockets and remove it at
        // the outline; either way by at most the tolerance per unit length
        // of boundary and span.
        let exposed: f64 = solid
            .bands()
            .iter()
            .map(|band| {
                solid.boundary_loop(band.boundary).length() * (band.heights[1] - band.heights[0])
            })
            .sum();
        assert!((expected - solid.volume()).abs() <= 0.01 * exposed);
        // Every mesh face maps to a native face, floors included.
        let floors = realized
            .mesh_faces
            .iter()
            .filter(|(_, face)| matches!(solid.faces()[face.index()].role, FaceRole::Floor { .. }))
            .count();
        assert!(floors >= 2);
    }
}

#[test]
fn webs_thinner_than_the_separation_are_refused() {
    let policy = ProfilePolicy::new(1e-6);
    let block = || {
        let outer = hole_rect([0.0, 0.0], [10.0, 6.0]).reversed();
        let profile = Profile::new(outer, vec![], &policy).unwrap();
        Extrusion::new(profile, Placement3::IDENTITY, 1.0).unwrap()
    };
    let cut = |solid: Extrusion, end: End, depth: f64| {
        solid.with_pocket(
            Pocket {
                tag: None,
                outline: hole_rect([2.0, 2.0], [5.0, 4.0]),
                end,
                depth,
            },
            &policy,
        )
    };
    let refused = |result: Result<Extrusion, ExtrusionError>| match result {
        Err(ExtrusionError::Pocket { error, .. }) => error,
        other => panic!("expected a pocket refusal, got {other:?}"),
    };
    // A floor within the separation of the other end.
    assert!(matches!(
        refused(cut(block(), End::End, 1.0 - 1e-7)),
        PocketError::InvalidDepth {
            length: 1.0,
            min_separation: 1e-6,
            ..
        }
    ));
    // Opposite floors within the separation of each other.
    let once = cut(block(), End::End, 0.5).unwrap();
    assert!(
        matches!(refused(cut(once.clone(), End::Start, 0.5 - 1e-7)), PocketError::BreaksThrough { other: 0, min_separation:1e-6, remaining } if (remaining - 1e-7).abs()<1e-15)
    );
    assert!(cut(once, End::Start, 0.4).unwrap().is_closed());
}

#[test]
fn webs_that_collapse_in_f32_are_refused_at_realization() {
    // Exact validation under a tiny separation accepts a 1e-9 web, but
    // narrowing puts both floors at z = 0.5.
    let policy = ProfilePolicy::new(1e-12);
    let outer = hole_rect([0.0, 0.0], [10.0, 6.0]).reversed();
    let profile = Profile::new(outer, vec![], &policy).unwrap();
    let thin = [(End::End, 0.5), (End::Start, 0.499_999_999)]
        .into_iter()
        .fold(
            Extrusion::new(profile, Placement3::IDENTITY, 1.0).unwrap(),
            |solid, (end, depth)| {
                solid
                    .with_pocket(
                        Pocket {
                            tag: None,
                            outline: hole_rect([2.0, 2.0], [5.0, 4.0]),
                            end,
                            depth,
                        },
                        &policy,
                    )
                    .unwrap()
            },
        );
    assert_eq!(
        thin.tessellate(&TessellationPolicy::default()).unwrap_err(),
        TessellateExtrusionError::Web {
            pocket: 0,
            opposite: Some(1)
        }
    );
}

#[test]
fn native_clearance_checks_keep_separations_smaller_than_a_depth_ulp() {
    for (length, separation) in [(1.0, 1e-20), (1e9, 1e-9)] {
        let policy = ProfilePolicy::new(separation);
        let block = || {
            let outer = hole_rect([0.0, 0.0], [10.0, 6.0]).reversed();
            let profile = Profile::new(outer, vec![], &policy).unwrap();
            Extrusion::new(profile, Placement3::IDENTITY, length).unwrap()
        };
        let cut = |solid: Extrusion, end, depth| {
            solid.with_pocket(
                Pocket {
                    tag: None,
                    outline: hole_rect([2.0, 2.0], [5.0, 4.0]),
                    end,
                    depth,
                },
                &policy,
            )
        };
        for end in [End::Start, End::End] {
            assert!(matches!(
                cut(block(), end, length),
                Err(ExtrusionError::Pocket {
                    error: PocketError::InvalidDepth { .. },
                    ..
                })
            ));
            let once = cut(block(), end, 0.5 * length).unwrap();
            let opposite = if end == End::Start {
                End::End
            } else {
                End::Start
            };
            assert!(matches!(
                cut(once, opposite, 0.5 * length),
                Err(ExtrusionError::Pocket {
                    error: PocketError::BreaksThrough { other: 0, .. },
                    ..
                })
            ));
        }
    }
}
