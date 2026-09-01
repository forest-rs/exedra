// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use exedra::{FaceBuildAttrs, Mesh, MeshBuilder};

    use super::exact::ExactStretchPlan;
    use super::mesh::{cross3, dot3};
    use super::exact_plan;
    use crate::evaluate::{Aabb3, Fidelity, Severity, evaluate, evaluate_with_cache, mesh_bounds};
    use crate::ir::{
        CapMode, CsgOp, NodeKind, Placement3, Plane3, PrimitiveSpec, Recipe, RecipeBuilder,
    };
    use crate::tessellate::EvalPolicy;

    fn stretched_box(normal: [f64; 3], distance: f64, length: f64) -> Recipe {
        let mut builder = RecipeBuilder::new();
        let child = builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box {
                    size: [10.0, 4.0, 2.0],
                },
                placement: Placement3::IDENTITY,
            })
            .expect("box is valid");
        let stretch = builder
            .add(NodeKind::Stretch {
                child,
                plane: Plane3 { normal, distance },
                length,
            })
            .expect("stretch is valid");
        builder.finish(stretch).expect("recipe is valid")
    }

    fn evaluate_bounds(recipe: &Recipe) -> (Aabb3, crate::evaluate::Evaluation) {
        let result = evaluate(recipe, &EvalPolicy::default()).expect("stretch evaluation is total");
        assert_eq!(result.bodies.len(), 1, "{:?}", result.report.diagnostics);
        let bounds = mesh_bounds(&result.bodies[0].body.mesh);
        (bounds, result)
    }

    fn mesh_volume(mesh: &Mesh) -> f64 {
        let mut six_volume = 0.0;
        for face in mesh.faces() {
            for corners in mesh.face_triangles(face, exedra::FaceTriangulation::Fan) {
                let points = corners.map(|corner| {
                    let vertex = mesh.to_vertex(corner).expect("corner has a vertex");
                    mesh.vertex_position(vertex)
                        .expect("vertex has a position")
                        .map(f64::from)
                });
                six_volume += dot3(points[0], cross3(points[1], points[2]));
            }
        }
        six_volume.abs() / 6.0
    }

    fn imported_box() -> Mesh {
        imported_box_with_faces(6)
    }

    fn imported_box_with_faces(face_count: usize) -> Mesh {
        let mut builder = MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 4.0, 0.0],
            [0.0, 4.0, 0.0],
            [0.0, 0.0, 2.0],
            [10.0, 0.0, 2.0],
            [10.0, 4.0, 2.0],
            [0.0, 4.0, 2.0],
        ] {
            builder.push_vertex(position);
        }
        // These loops point outward and carry distinct regions so a later
        // test can tell whether a stretched wall retained its material slot.
        for (region, face) in [
            (10, [0, 3, 2, 1]),
            (11, [4, 5, 6, 7]),
            (12, [0, 1, 5, 4]),
            (13, [1, 2, 6, 5]),
            (14, [2, 3, 7, 6]),
            (15, [3, 0, 4, 7]),
        ]
        .into_iter()
        .take(face_count)
        {
            builder
                .add_face_with_attrs(
                    &face,
                    &FaceBuildAttrs {
                        region: Some(region),
                        ..FaceBuildAttrs::default()
                    },
                )
                .expect("box face is manifold");
        }
        builder.build().expect("box mesh is valid").mesh
    }

    fn tapered_import() -> Mesh {
        let mut builder = MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 3.0, 0.0],
            [0.0, 4.0, 0.0],
            [0.0, 0.0, 2.0],
            [10.0, 0.0, 2.0],
            [10.0, 3.0, 2.0],
            [0.0, 4.0, 2.0],
        ] {
            builder.push_vertex(position);
        }
        for face in [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ] {
            builder.add_face(&face).expect("tapered face is manifold");
        }
        builder.build().expect("tapered mesh is valid").mesh
    }

    fn region_split_imported_box() -> Mesh {
        let mut builder = MeshBuilder::new();
        for x in [0.0, 5.0, 10.0] {
            for position in [
                [x, 0.0, 0.0],
                [x, 4.0, 0.0],
                [x, 4.0, 2.0],
                [x, 0.0, 2.0],
            ] {
                builder.push_vertex(position);
            }
        }
        for (region, face) in [
            (10, [1, 0, 3, 2]),
            (11, [8, 9, 10, 11]),
            (100, [0, 1, 5, 4]),
            (101, [3, 7, 6, 2]),
            (102, [0, 4, 7, 3]),
            (103, [1, 2, 6, 5]),
            (200, [4, 5, 9, 8]),
            (201, [7, 11, 10, 6]),
            (202, [4, 8, 11, 7]),
            (203, [5, 6, 10, 9]),
        ] {
            builder
                .add_face_with_attrs(
                    &face,
                    &FaceBuildAttrs {
                        region: Some(region),
                        ..FaceBuildAttrs::default()
                    },
                )
                .expect("segmented box face is manifold");
        }
        builder.build().expect("segmented box is valid").mesh
    }

    fn concave_import() -> Mesh {
        let outline = [
            [0.0, 0.0],
            [6.0, 0.0],
            [6.0, 1.0],
            [2.0, 1.0],
            [2.0, 3.0],
            [6.0, 3.0],
            [6.0, 4.0],
            [0.0, 4.0],
        ];
        let mut builder = MeshBuilder::new();
        for z in [0.0, 2.0] {
            for [x, y] in outline {
                builder.push_vertex([x, y, z]);
            }
        }
        builder
            .add_face(&(0_u32..8).rev().collect::<Vec<_>>())
            .expect("lower concave cap is valid");
        builder
            .add_face(&(8_u32..16).collect::<Vec<_>>())
            .expect("upper concave cap is valid");
        for current in 0_u32..8 {
            let next = (current + 1) % 8;
            builder
                .add_face(&[current, next, next + 8, current + 8])
                .expect("concave prism wall is manifold");
        }
        builder.build().expect("concave prism is valid").mesh
    }

    fn mapped_imported_box() -> Mesh {
        let mut mesh = imported_box();
        let faces = mesh.faces().collect::<Vec<_>>();
        let mut assignments = Vec::new();
        for (face_index, face) in faces.into_iter().enumerate() {
            for corner in mesh.face_loop(face) {
                let vertex = mesh.to_vertex(corner).expect("corner has a vertex");
                let [x, y, z] = *mesh.vertex_position(vertex).expect("vertex has position");
                let uv = match face_index {
                    0 | 1 => [x, y],
                    2 | 4 => [x, z],
                    3 | 5 => [y, z],
                    _ => unreachable!("box has six faces"),
                };
                assignments.push((corner, uv));
            }
        }
        {
            let mut edit = mesh.edit();
            for (corner, uv) in assignments {
                exedra::op::set_corner_uv(&mut edit, corner, uv).expect("corner is live");
            }
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                edit.finish();
            }
        }
        mesh
    }

    fn edge_authored_imported_box() -> Mesh {
        let mut mesh = imported_box();
        let edge = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|edge| {
                let from = mesh.from_vertex(*edge).unwrap();
                let to = mesh.to_vertex(*edge).unwrap();
                mesh.vertex_position(from).unwrap()[0] == 0.0
                    && mesh.vertex_position(to).unwrap()[0] == 0.0
            })
            .expect("box has an edge wholly in x=0");
        let vertex = mesh.from_vertex(edge).expect("edge has an origin");
        {
            let mut edit = mesh.edit();
            exedra::op::set_edge_seam(&mut edit, edge, true).expect("edge is live");
            exedra::op::set_edge_sharpness(&mut edit, edge, 2.5).expect("edge is live");
            exedra::op::set_vertex_sharpness(&mut edit, vertex, 3.25)
                .expect("vertex is live");
            exedra::op::set_corner_normal_override(
                &mut edit,
                edge,
                Some([0.0, 0.6, 0.8]),
            )
            .expect("corner is live");
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                edit.finish();
            }
        }
        mesh
    }

    fn stretched_import(length: f64) -> Recipe {
        stretched_mesh_import(imported_box(), 4.0, length)
    }

    fn stretched_mesh_import(mesh: Mesh, distance: f64, length: f64) -> Recipe {
        stretched_mesh_import_plane(mesh, [1.0, 0.0, 0.0], distance, length)
    }

    fn stretched_mesh_import_plane(
        mesh: Mesh,
        normal: [f64; 3],
        distance: f64,
        length: f64,
    ) -> Recipe {
        let mut builder = RecipeBuilder::new();
        let import = builder
            .add_import(mesh)
            .expect("deep-valid mesh is a valid import");
        let child = builder
            .add(NodeKind::MeshImport {
                import,
                placement: Placement3::IDENTITY,
            })
            .expect("import node is valid");
        let stretch = builder
            .add(NodeKind::Stretch {
                child,
                plane: Plane3 { normal, distance },
                length,
            })
            .expect("stretch node is valid");
        builder.finish(stretch).expect("recipe is valid")
    }

    #[test]
    fn normalized_plane_orientation_selects_the_positive_half_space() {
        // Scaling a plane encoding cannot change geometry. Reversing it must
        // change which rigid half moves: +X grows the right side, while -X
        // moves the left side toward -X and leaves the right side fixed.
        let (positive, result) = evaluate_bounds(&stretched_box([2.0, 0.0, 0.0], 8.0, 3.0));
        assert_eq!(
            positive,
            Aabb3 {
                min: [0.0, 0.0, 0.0],
                max: [13.0, 4.0, 2.0],
            }
        );
        assert_eq!(result.report.counters.unimplemented, 0);
        assert!(result.report.clean_at(Severity::Warning));

        let (negative, _) = evaluate_bounds(&stretched_box([-2.0, 0.0, 0.0], -8.0, 3.0));
        assert_eq!(
            negative,
            Aabb3 {
                min: [-3.0, 0.0, 0.0],
                max: [10.0, 4.0, 2.0],
            }
        );

        let (imported_positive, _) = evaluate_bounds(&stretched_mesh_import_plane(
            imported_box(),
            [2.0, 0.0, 0.0],
            8.0,
            3.0,
        ));
        let (imported_negative, _) = evaluate_bounds(&stretched_mesh_import_plane(
            imported_box(),
            [-2.0, 0.0, 0.0],
            -8.0,
            3.0,
        ));
        assert_eq!(imported_positive, positive);
        assert_eq!(imported_negative, negative);
    }

    #[test]
    fn contraction_removes_the_positive_offset_slab() {
        // A -3 stretch at x=4 removes input x=[4, 7], then moves x>=7 back
        // by three. It must not scale the ten-unit box or fold x in (4, 7).
        let (bounds, result) = evaluate_bounds(&stretched_box([1.0, 0.0, 0.0], 4.0, -3.0));
        assert_eq!(
            bounds,
            Aabb3 {
                min: [0.0, 0.0, 0.0],
                max: [7.0, 4.0, 2.0],
            }
        );
        assert_eq!(
            result.report.fidelity_of(result.bodies[0].node),
            Some(Fidelity::Exact)
        );
    }

    #[test]
    fn profile_plane_and_extrusion_axis_stretches_are_exact() {
        // The two algebraic cases exercise different rewrites: X edits the
        // rectangular Profile2, while Z edits the extrusion height. Both
        // keep exact fidelity and are independent of chord tolerance.
        for (normal, distance, expected) in [
            ([1.0, 0.0, 0.0], 4.0, [13.0, 4.0, 10.0]),
            ([0.0, 0.0, 1.0], 4.0, [10.0, 4.0, 13.0]),
        ] {
            let mut builder = RecipeBuilder::new();
            let profile = builder.add_profile(crate::builders::rect(10.0, 4.0).unwrap());
            let child = builder
                .add(NodeKind::Extrude {
                    profile,
                    placement: Placement3::IDENTITY,
                    height: 10.0,
                    caps: CapMode::Both,
                })
                .unwrap();
            let stretch = builder
                .add(NodeKind::Stretch {
                    child,
                    plane: Plane3 { normal, distance },
                    length: 3.0,
                })
                .unwrap();
            let recipe = builder.finish(stretch).unwrap();
            let (bounds, result) = evaluate_bounds(&recipe);
            assert_eq!(bounds.max, expected);
            assert_eq!(result.report.fidelity_of(stretch), Some(Fidelity::Exact));
        }
    }

    #[test]
    fn profile_rewrite_preserves_tags_and_untouched_arcs() {
        // A middle cut crosses only the rounded rectangle's straight top and
        // bottom segments. Those split pieces keep their original tags; right
        // corner arcs translate rigidly, left arcs remain bit-identical, and
        // no arc is flattened into policy-dependent lines.
        let profile = crate::builders::rounded_rect(400.0, 300.0, 50.0).unwrap();
        let source = profile.outer().segs().to_vec();
        let mut builder = RecipeBuilder::new();
        let profile_id = builder.add_profile(profile);
        let child = builder
            .add(NodeKind::Extrude {
                profile: profile_id,
                placement: Placement3::IDENTITY,
                height: 20.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let recipe = builder.finish(child).unwrap();
        let plan = exact_plan(
            &recipe,
            child,
            &Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 200.0,
            },
            100.0,
            &Placement3::IDENTITY,
        )
        .unwrap()
        .expect("middle-band stretch has an exact profile rewrite");
        let ExactStretchPlan::Extrude { profile, .. } = plan else {
            panic!("profile stretch remains an extrusion");
        };
        let rewritten = profile.outer().segs();
        for tag in 0..8 {
            assert!(
                rewritten
                    .iter()
                    .any(|segment| segment.tag == Some(crate::profile::SegTag(tag))),
                "source segment tag {tag} survives"
            );
        }
        for tag in [1_u32, 3, 5, 7] {
            let before = source
                .iter()
                .find(|segment| segment.tag == Some(crate::profile::SegTag(tag)))
                .unwrap();
            let after = rewritten
                .iter()
                .find(|segment| segment.tag == Some(crate::profile::SegTag(tag)))
                .unwrap();
            assert_eq!(after.kind, before.kind, "arc {tag} keeps its bulge");
            let expected_x = before.to.x + if tag == 1 || tag == 3 { 100.0 } else { 0.0 };
            assert_eq!(after.to.x, expected_x, "arc {tag} translates as one curve");
            assert_eq!(after.to.y, before.to.y);
        }
    }

    #[test]
    fn profile_rewrite_preserves_wall_regions_and_features() {
        // Splitting two profile lines inserts four additional analytic
        // segments. Those pieces extend their source walls: they must reuse
        // the original eight region and feature identities rather than
        // renumbering every wall after a cut.
        let mut builder = RecipeBuilder::new();
        let profile = builder
            .add_profile(crate::builders::rounded_rect(400.0, 300.0, 50.0).unwrap());
        let child = builder
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 20.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let stretch = builder
            .add(NodeKind::Stretch {
                child,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 200.0,
                },
                length: 100.0,
            })
            .unwrap();
        let result = evaluate(
            &builder.finish(stretch).unwrap(),
            &EvalPolicy::default(),
        )
        .unwrap();
        assert_eq!(result.report.counters.stretch_exact, 1);
        let body = &result.bodies[0].body;
        let regions = body
            .mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("extrusion walls have regions");
        let mut seen_regions = Vec::new();
        let mut seen_segments = Vec::new();
        for face in body.mesh.faces() {
            if let Some(crate::tessellate::Feature::Wall { seg, .. }) =
                body.source_map.face_feature(face)
            {
                seen_regions.push(regions.get(face.into()).copied().unwrap());
                seen_segments.push(seg);
            }
        }
        seen_regions.sort_unstable();
        seen_regions.dedup();
        seen_segments.sort_unstable();
        seen_segments.dedup();
        assert_eq!(seen_regions, (2..10).collect::<Vec<_>>());
        assert_eq!(seen_segments, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn profile_plane_contraction_removes_a_band_without_flattening() {
        // The rectangular profile's two X-directed boundary lines span the
        // removed slab. The algebraic path clips those lines at x=4 and x=7,
        // drops the intervening pieces, and preserves their segment tags and
        // original wall-region identities.
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(crate::builders::rect(10.0, 4.0).unwrap());
        let child = builder
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 2.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let stretch = builder
            .add(NodeKind::Stretch {
                child,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 4.0,
                },
                length: -3.0,
            })
            .unwrap();
        let recipe = builder.finish(stretch).unwrap();
        let (bounds, result) = evaluate_bounds(&recipe);
        assert_eq!(bounds.max, [7.0, 4.0, 2.0]);
        assert_eq!(result.report.counters.stretch_exact, 1);
        assert_eq!(result.report.counters.stretch_mesh, 0);
        let body = &result.bodies[0].body;
        let regions = body
            .mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("extrusion walls have regions");
        let mut seen = body
            .mesh
            .faces()
            .filter(|face| {
                matches!(
                    body.source_map.face_feature(*face),
                    Some(crate::tessellate::Feature::Wall { .. })
                )
            })
            .map(|face| regions.get(face.into()).copied().unwrap())
            .collect::<Vec<_>>();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen, (2..6).collect::<Vec<_>>());
    }

    #[test]
    fn imported_closed_mesh_gets_a_watertight_inserted_band() {
        // MeshImport cannot use a recipe rewrite. This pins the general path:
        // the section is closed, its band carries the source wall regions,
        // and the result remains deeply valid rather than becoming a soup.
        let (bounds, result) = evaluate_bounds(&stretched_import(3.0));
        assert_eq!(bounds.max, [13.0, 4.0, 2.0]);
        let body = &result.bodies[0].body;
        assert!(body.mesh.validate_deep().is_empty());
        assert_eq!(body.source_map.face_count(), body.mesh.faces().count());
        assert_eq!(
            result.report.counters.faces as usize,
            body.mesh.faces().count(),
            "temporary child faces are work, not emitted output"
        );
        assert_eq!(
            result.report.counters.vertices as usize,
            body.mesh.vertices().count(),
            "temporary child vertices are work, not emitted output"
        );
        assert_eq!(
            result.report.counters.source_map_bytes,
            body.source_map.stats().approx_bytes as u64,
            "temporary child source maps are not retained output"
        );
        let regions = body
            .mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("stretch keeps face regions");
        for region in 10..=15 {
            assert!(
                body.mesh
                    .faces()
                    .any(|face| regions.get(face.into()).copied() == Some(region)),
                "source region {region} survives"
            );
        }
        assert!(
            body.mesh.vertices().any(|vertex| matches!(
                body.source_map.vertex_feature(vertex),
                Some(crate::tessellate::Feature::StretchSeam { .. })
            )),
            "the section boundary is addressable as a stretch seam"
        );
    }

    #[test]
    fn planar_uvs_extend_across_the_inserted_band() {
        // The imported box maps each face affinely in its own plane. Stretch
        // shifts the positive-side UVs by that affine gradient and maps the
        // band between x=4 and x=7, so its U interval grows by exactly three.
        let recipe = stretched_mesh_import(mapped_imported_box(), 4.0, 3.0);
        let (_, result) = evaluate_bounds(&recipe);
        assert_eq!(result.report.counters.stretch_uv_unmapped_faces, 0);
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "eval.stretch.uv_unmapped")
        );
        let mesh = &result.bodies[0].body.mesh;
        let uvs = mesh
            .attrs()
            .sparse(exedra::attr::CORNER_UV)
            .expect("mapped input retains corner UVs");
        let mut found_extended_band = false;
        for face in mesh.faces() {
            let corners = mesh.face_loop(face).collect::<Vec<_>>();
            let xs = corners
                .iter()
                .map(|corner| {
                    let vertex = mesh.to_vertex(*corner).unwrap();
                    mesh.vertex_position(vertex).unwrap()[0]
                })
                .collect::<Vec<_>>();
            if xs.iter().copied().fold(f32::INFINITY, f32::min) == 4.0
                && xs.iter().copied().fold(f32::NEG_INFINITY, f32::max) == 7.0
            {
                let us = corners
                    .iter()
                    .filter_map(|corner| uvs.get((*corner).into()).map(|uv| uv[0]))
                    .collect::<Vec<_>>();
                if !us.is_empty()
                    && us.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                        - us.iter().copied().fold(f32::INFINITY, f32::min)
                        == 3.0
                {
                    found_extended_band = true;
                }
            }
        }
        assert!(found_extended_band, "a band face extends U from 4 to 7");
    }

    #[test]
    fn imported_closed_mesh_contraction_restitched_matching_sections() {
        // The mesh fallback must implement removal as well as insertion. The
        // two x sections of this prismatic import are identical after the far
        // section moves by -3, so they weld into one seam without a band.
        let (bounds, result) = evaluate_bounds(&stretched_import(-3.0));
        assert_eq!(bounds.max, [7.0, 4.0, 2.0]);
        let body = &result.bodies[0].body;
        assert!(body.mesh.validate_deep().is_empty());
        assert_eq!(result.report.counters.stretch_mesh, 1);
        assert_eq!(result.report.counters.stretch_band_faces, 0);
        assert_eq!(result.report.counters.stretch_refusals, 0);
    }

    #[test]
    fn contraction_allows_provenance_to_change_inside_the_removed_slab() {
        // The near cut crosses regions 100..103 while the far cut crosses
        // 200..203. Those material boundaries are metadata, not topology:
        // geometrically matching sections can stitch while each surviving
        // face keeps the region authored on its side of the new seam.
        let recipe = stretched_mesh_import(region_split_imported_box(), 4.0, -3.0);
        let (bounds, result) = evaluate_bounds(&recipe);
        assert_eq!(bounds.max, [7.0, 4.0, 2.0]);
        assert_eq!(result.report.counters.stretch_refusals, 0);
        let regions = result.bodies[0]
            .body
            .mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("contraction retains regions");
        for expected in [100, 101, 102, 103, 200, 201, 202, 203] {
            assert!(
                result.bodies[0]
                    .body
                    .mesh
                    .faces()
                    .any(|face| regions.get(face.into()).copied() == Some(expected)),
                "region {expected} survives on its original side of the seam"
            );
        }
    }

    #[test]
    fn plane_miss_is_a_rigid_noop_or_translation() {
        // A plane strictly beyond the positive end selects no material and
        // leaves the body alone; a plane before the negative end selects the
        // whole body and applies one rigid translation. Neither needs a cut.
        let (unchanged, unchanged_result) =
            evaluate_bounds(&stretched_box([1.0, 0.0, 0.0], 20.0, 3.0));
        assert_eq!(unchanged.min, [0.0, 0.0, 0.0]);
        assert_eq!(unchanged.max, [10.0, 4.0, 2.0]);
        assert_eq!(unchanged_result.report.counters.stretch_exact, 1);

        let (translated, _) = evaluate_bounds(&stretched_box([1.0, 0.0, 0.0], -5.0, 3.0));
        assert_eq!(translated.min, [3.0, 0.0, 0.0]);
        assert_eq!(translated.max, [13.0, 4.0, 2.0]);
    }

    #[test]
    fn contraction_past_the_movable_half_is_typed() {
        // Removing x=[4, 12] from a box ending at x=10 leaves no positive
        // component to re-stitch. The evaluator must preserve only the child
        // envelope and name the refusal instead of collapsing the box.
        let recipe = stretched_box([1.0, 0.0, 0.0], 4.0, -8.0);
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("refusal is total");
        assert!(result.bodies.is_empty());
        assert_eq!(
            result.report.fidelity_of(recipe.root()),
            Some(Fidelity::EnvelopeOnly)
        );
        assert!(result.report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "eval.stretch.contraction_half"
                && diagnostic.severity == Severity::Error
        }));
        assert_eq!(result.report.counters.stretch_refusals, 1);
    }

    #[test]
    fn contraction_refuses_nonmatching_sections() {
        // This prism narrows in Y along X. Its x=4 and x=7 sections differ,
        // so translating the latter back by three cannot make the boundary
        // edge maps coincide. Restitching would invent or fold a surface.
        let recipe = stretched_mesh_import(tapered_import(), 4.0, -3.0);
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("refusal is total");
        assert!(result.bodies.is_empty());
        assert_eq!(
            result.report.fidelity_of(recipe.root()),
            Some(Fidelity::EnvelopeOnly)
        );
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "eval.stretch.contraction_stitch")
        );
    }

    #[test]
    fn tangent_vertex_and_open_crossing_shell_are_typed() {
        // General mesh cutting deliberately has no ownership convention for
        // a plane through stored vertices/coplanar faces, and it cannot close
        // an open shell. The exact box lane follows the same contact contract
        // so equivalent constructive and imported geometry cannot disagree.
        let tangent = stretched_mesh_import(imported_box(), 0.0, 2.0);
        let tangent = evaluate(&tangent, &EvalPolicy::default()).expect("refusal is total");
        assert!(tangent.bodies.is_empty());
        assert!(
            tangent
                .report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "eval.stretch.ambiguous_contact")
        );

        let exact_tangent = stretched_box([1.0, 0.0, 0.0], 0.0, 2.0);
        let exact_tangent =
            evaluate(&exact_tangent, &EvalPolicy::default()).expect("refusal is total");
        assert!(exact_tangent.bodies.is_empty());
        assert!(exact_tangent.report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "eval.stretch.ambiguous_contact"
        }));

        let open = stretched_mesh_import(imported_box_with_faces(5), 4.0, 2.0);
        let open = evaluate(&open, &EvalPolicy::default()).expect("refusal is total");
        assert!(open.bodies.is_empty());
        assert!(
            open.report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "eval.stretch.open_shell")
        );
    }

    #[test]
    fn disconnected_face_sections_are_named_separately_from_tangency() {
        // The x=4 plane crosses each concave cap in two disjoint intervals.
        // V1 does not decompose one face into several section segments, but
        // this is neither tangent contact nor non-manifold input and must be
        // reported under its own stable diagnostic code.
        let recipe = stretched_mesh_import(concave_import(), 4.0, 2.0);
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("refusal is total");
        assert!(result.bodies.is_empty());
        assert!(result.report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "eval.stretch.disconnected_face_section"
        }));
    }

    #[test]
    fn stretch_composes_above_and_below_csg() {
        // The first recipe proves a stretched operand reaches Boolean CSG.
        // The second sections through the Boolean result's aperture, proving
        // both its outer and inner loops can be banded by an outer stretch
        // without either layer silently emitting envelopes.
        fn build(stretch_inside: bool) -> Recipe {
            let mut builder = RecipeBuilder::new();
            let host = builder
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box {
                        size: [10.0, 4.0, 2.0],
                    },
                    placement: Placement3::IDENTITY,
                })
                .unwrap();
            let cutter = builder
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box {
                        size: [2.0, 2.0, 4.0],
                    },
                    placement: Placement3::translate(2.0, 1.0, -1.0),
                })
                .unwrap();
            if stretch_inside {
                let stretched = builder
                    .add(NodeKind::Stretch {
                        child: host,
                        plane: Plane3 {
                            normal: [1.0, 0.0, 0.0],
                            distance: 7.0,
                        },
                        length: 2.0,
                    })
                    .unwrap();
                let root = builder
                    .add(NodeKind::Csg {
                        op: CsgOp::Difference,
                        operands: vec![stretched, cutter],
                    })
                    .unwrap();
                builder.finish(root).unwrap()
            } else {
                let difference = builder
                    .add(NodeKind::Csg {
                        op: CsgOp::Difference,
                        operands: vec![host, cutter],
                    })
                    .unwrap();
                let root = builder
                    .add(NodeKind::Stretch {
                        child: difference,
                        plane: Plane3 {
                            normal: [1.0, 0.0, 0.0],
                            distance: 3.0,
                        },
                        length: 2.0,
                    })
                    .unwrap();
                builder.finish(root).unwrap()
            }
        }
        for stretch_inside in [true, false] {
            let recipe = build(stretch_inside);
            let (bounds, result) = evaluate_bounds(&recipe);
            assert_eq!(bounds.max[0], 12.0);
            assert!(
                result.report.clean_at(Severity::Warning),
                "{:?}",
                result.report.diagnostics
            );
            assert!(result.bodies[0].body.mesh.validate_deep().is_empty());
        }
    }

    #[test]
    fn mesh_stretch_is_bit_identical_and_cacheable() {
        // The general path must not depend on map iteration or fresh arena
        // generations. A warm cache reuses its result, and both signatures
        // equal two independent pure evaluations.
        let recipe = stretched_import(3.0);
        let policy = EvalPolicy::default();
        let signature = |evaluation: &crate::evaluate::Evaluation| {
            let (triangles, _) = evaluation.bodies[0]
                .body
                .mesh
                .to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&triangles)
        };
        let first = evaluate(&recipe, &policy).unwrap();
        let second = evaluate(&recipe, &policy).unwrap();
        assert_eq!(signature(&first), signature(&second));

        let mut cache = crate::cache::EvalCache::new();
        let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        assert_eq!(signature(&first), signature(&cold));
        assert_eq!(signature(&cold), signature(&warm));
        assert_eq!(warm.report.counters.tessellations, 0);
        assert_eq!(cold.report.counters.stretch_faces_split, 4);
        assert_eq!(cold.report.counters.stretch_band_faces, 4);
        assert_eq!(warm.report.counters.stretch_faces_split, 0);
        assert_eq!(warm.report.counters.stretch_band_faces, 0);
        assert!(
            warm.report.counters.cache_hits >= 2,
            "child and stretch hit"
        );
    }

    #[test]
    fn mesh_path_matches_the_exact_box_oracle_across_bands() {
        // Axis-aligned boxes have a closed-form stretch and an independent
        // primitive rewrite. Sweep several non-vertex planes and insert/remove
        // distances; the imported-mesh path must match exact bounds and the
        // analytic volume `(10 + length) * 4 * 2` in every case.
        for (distance, length) in [
            (0.5, 0.25),
            (2.25, 1.0),
            (4.0, 3.0),
            (6.5, 0.75),
            (9.5, 2.0),
            (0.5, -0.25),
            (2.25, -1.0),
            (4.0, -3.0),
            (6.5, -1.5),
        ] {
            let exact = stretched_box([1.0, 0.0, 0.0], distance, length);
            let imported = stretched_mesh_import(imported_box(), distance, length);
            let (exact_bounds, _) = evaluate_bounds(&exact);
            let (mesh_bounds, evaluated) = evaluate_bounds(&imported);
            assert_eq!(
                mesh_bounds, exact_bounds,
                "distance={distance} length={length}"
            );
            let expected = (10.0 + length) * 4.0 * 2.0;
            let actual = mesh_volume(&evaluated.bodies[0].body.mesh);
            assert!(
                (actual - expected).abs() < 1.0e-5,
                "distance={distance} length={length}: {actual} vs {expected}"
            );
        }
    }

    #[test]
    fn seam_creasing_obeys_the_policy_threshold() {
        // An oblique cut creates band faces that are not tangent to every
        // source face. Threshold zero marks those section rims sharp, while
        // threshold one accepts every possible sine angle as smooth.
        let recipe = stretched_mesh_import_plane(imported_box(), [1.0, 0.3, 0.2], 4.1, 2.0);
        let sharp_count = |threshold| {
            let policy = EvalPolicy {
                sharp_sin_threshold: threshold,
                ..EvalPolicy::default()
            };
            let result = evaluate(&recipe, &policy).expect("stretch evaluates");
            assert_eq!(result.bodies.len(), 1, "{:?}", result.report.diagnostics);
            let mesh = &result.bodies[0].body.mesh;
            mesh.faces()
                .flat_map(|face| mesh.face_loop(face))
                .filter(|edge| mesh.edge_sharpness(*edge).is_some_and(|value| value > 0.0))
                .count()
        };
        assert!(sharp_count(0.0) > 0);
        assert_eq!(sharp_count(1.0), 0);
    }

    #[test]
    fn authored_mesh_attributes_survive_away_from_the_cut() {
        // Stretch rebuilds topology, but the rigid halves must retain authored
        // attributes. The x=0 source corner never meets the x=4 section, so
        // its edge tags, vertex sharpness, and normal override stay unchanged.
        let recipe = stretched_mesh_import(edge_authored_imported_box(), 4.0, 2.0);
        let (_, result) = evaluate_bounds(&recipe);
        let mesh = &result.bodies[0].body.mesh;
        assert!(
            mesh.faces()
                .flat_map(|face| mesh.face_loop(face))
                .any(|edge| {
                    mesh.edge_seam(edge) == Some(true) && mesh.edge_sharpness(edge) == Some(2.5)
                })
        );
        assert!(
            mesh.vertices()
                .any(|vertex| mesh.vertex_sharpness(vertex) == Some(3.25))
        );
        let normals = mesh
            .attrs()
            .sparse(exedra::attr::CORNER_NORMAL_OVERRIDE)
            .expect("normal override layer survives");
        assert!(mesh.faces().flat_map(|face| mesh.face_loop(face)).any(
            |corner| normals.get(corner.into()).copied() == Some([0.0, 0.6, 0.8])
        ));
    }

    #[test]
    fn nested_planes_are_read_after_inner_deformation() {
        // The inner edit grows 0..10 to 0..12. The outer plane x=11 is then
        // evaluated in that deformed input and grows only the final band,
        // yielding 0..15. Reading both planes in the original space would
        // select a different zone and violate sequential composition.
        let mut builder = RecipeBuilder::new();
        let child = builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box {
                    size: [10.0, 4.0, 2.0],
                },
                placement: Placement3::IDENTITY,
            })
            .unwrap();
        let inner = builder
            .add(NodeKind::Stretch {
                child,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 4.0,
                },
                length: 2.0,
            })
            .unwrap();
        let outer = builder
            .add(NodeKind::Stretch {
                child: inner,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 11.0,
                },
                length: 3.0,
            })
            .unwrap();
        let recipe = builder.finish(outer).unwrap();
        let (bounds, result) = evaluate_bounds(&recipe);
        assert_eq!(bounds.max, [15.0, 4.0, 2.0]);
        assert_eq!(result.report.counters.unimplemented, 0);
        assert_eq!(result.report.counters.stretch_exact, 2);
        assert_eq!(result.report.counters.stretch_mesh, 0);
    }

    #[test]
    fn exact_rewrite_pulls_the_plane_through_a_rigid_transform() {
        // A 90-degree child transform maps local +X to parent +Y. The stretch
        // plane is authored in the transformed input at y=6 (local x=4), so
        // the exact box rewrite must grow Y while retaining the rotated AABB.
        let mut builder = RecipeBuilder::new();
        let box_node = builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box {
                    size: [10.0, 4.0, 2.0],
                },
                placement: Placement3::IDENTITY,
            })
            .unwrap();
        let transformed = builder
            .add(NodeKind::Transform {
                child: box_node,
                xf: Placement3::rotate_z_then_translate(
                    core::f64::consts::FRAC_PI_2,
                    5.0,
                    2.0,
                    0.0,
                ),
            })
            .unwrap();
        let stretch = builder
            .add(NodeKind::Stretch {
                child: transformed,
                plane: Plane3 {
                    normal: [0.0, 1.0, 0.0],
                    distance: 6.0,
                },
                length: 3.0,
            })
            .unwrap();
        let recipe = builder.finish(stretch).unwrap();
        let (bounds, result) = evaluate_bounds(&recipe);
        assert!((bounds.min[0] - 1.0).abs() < 1.0e-6, "{bounds:?}");
        assert!((bounds.max[0] - 5.0).abs() < 1.0e-6, "{bounds:?}");
        assert!((bounds.min[1] - 2.0).abs() < 1.0e-6, "{bounds:?}");
        assert!((bounds.max[1] - 15.0).abs() < 1.0e-6, "{bounds:?}");
        assert_eq!(result.report.counters.stretch_exact, 1);
    }

    #[test]
    fn affine_ancestor_transforms_the_local_displacement_after_stretch() {
        // Stretch is node-local: insert three units along local X, then the
        // ancestor scales X by two. Both the exact primitive and mesh import
        // must therefore end at world x=26, not at 23 from a normalized
        // world-space displacement applied in the wrong order.
        fn build(imported: bool) -> Recipe {
            let mut builder = RecipeBuilder::new();
            let child = if imported {
                let import = builder.add_import(imported_box()).unwrap();
                builder
                    .add(NodeKind::MeshImport {
                        import,
                        placement: Placement3::IDENTITY,
                    })
                    .unwrap()
            } else {
                builder
                    .add(NodeKind::Primitive {
                        spec: PrimitiveSpec::Box {
                            size: [10.0, 4.0, 2.0],
                        },
                        placement: Placement3::IDENTITY,
                    })
                    .unwrap()
            };
            let stretch = builder
                .add(NodeKind::Stretch {
                    child,
                    plane: Plane3 {
                        normal: [1.0, 0.0, 0.0],
                        distance: 4.0,
                    },
                    length: 3.0,
                })
                .unwrap();
            let root = builder
                .add(NodeKind::Transform {
                    child: stretch,
                    xf: Placement3 {
                        rows: [
                            [2.0, 0.0, 0.0, 0.0],
                            [0.0, 1.0, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                        ],
                    },
                })
                .unwrap();
            builder.finish(root).unwrap()
        }
        for imported in [false, true] {
            let (bounds, result) = evaluate_bounds(&build(imported));
            assert_eq!(bounds.max, [26.0, 4.0, 2.0]);
            assert!(result.report.clean_at(Severity::Warning));
        }
    }
}
