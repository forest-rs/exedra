// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::cache::EvalCache;
use crate::evaluate::{evaluate, evaluate_with_cache};
use crate::ir::{
    BoxUv, NodeKind, Placement3, Recipe, RecipeBuilder, UvFrame, UvMapping, UvProjection,
    UvTransform,
};
use crate::tessellate::{EvalPolicy, TessellateError};
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use exedra_mesh::{CornerId, ExtractParams, Mesh, attr, op};
use exedra_testkit::fixtures::{quad_mesh, unit_box};

fn uv(mesh: &Mesh, corner: CornerId) -> Option<[f32; 2]> {
    mesh.attrs()
        .sparse(attr::CORNER_UV)
        .and_then(|layer| layer.get(corner.into()))
        .copied()
}
fn mapping(projection: UvProjection) -> UvMapping {
    UvMapping {
        projection,
        transform: UvTransform::IDENTITY,
    }
}
fn import_recipe(mesh: Mesh, mapping: UvMapping) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let import = builder.add_import(mesh).unwrap();
    let child = builder
        .add(NodeKind::MeshImport {
            import,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let root = builder
        .add(NodeKind::UvMap {
            options: exedra_mesh::uv_mapping::UvMappingOptions::default(),
            child,
            mapping,
        })
        .unwrap();
    builder.finish(root).unwrap()
}

#[test]
fn mapping_is_local_preserves_provenance_and_keeps_shared_import_untouched() {
    let mut builder = RecipeBuilder::new();
    let import = builder.add_import(quad_mesh()).unwrap();
    let slot = builder.material_slot("finish");
    let source = builder.source_ref("loaded-panel");
    let child = builder
        .with_material(slot)
        .with_source(source)
        .add(NodeKind::MeshImport {
            import,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let mapped = builder
        .add(NodeKind::UvMap {
            options: exedra_mesh::uv_mapping::UvMappingOptions::default(),
            child,
            mapping: mapping(UvProjection::Planar(UvFrame::XY)),
        })
        .unwrap();
    let moved = builder
        .add(NodeKind::Transform {
            child: mapped,
            xf: Placement3::translate(10.0, 20.0, 30.0),
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Group {
            children: vec![child, mapped, moved],
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let policy = EvalPolicy::default();
    let pure = evaluate(&recipe, &policy).unwrap();
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    assert_eq!(cold.report.counters.uv_mapping_passes, 1);
    assert_eq!(warm.report.counters.uv_mapping_passes, 0);
    assert_eq!(cold.report.counters.uv_corners_written, 4);
    assert!(Rc::ptr_eq(&cold.bodies[1].body, &warm.bodies[1].body));
    assert!(!Rc::ptr_eq(&cold.bodies[0].body, &cold.bodies[1].body));
    for result in [&pure, &cold, &warm] {
        assert_eq!(result.bodies.len(), 3);
        for (index, placed) in result.bodies.iter().enumerate() {
            let mesh = &placed.body.mesh;
            placed.body.source_map.check(mesh).unwrap();
            assert_eq!(placed.material, Some(slot));
            assert!(mesh.validate_deep().is_empty());
            for face in mesh.faces() {
                assert!(placed.body.source_map.surface_origin(face).is_some());
                for corner in mesh.face_loop(face) {
                    if index == 0 {
                        assert_eq!(uv(mesh, corner), None);
                    } else {
                        let p = *mesh
                            .vertex_position(mesh.to_vertex(corner).unwrap())
                            .unwrap();
                        let offset = if index == 2 { [10.0, 20.0] } else { [0.0; 2] };
                        assert_eq!(uv(mesh, corner), Some([p[0] - offset[0], p[1] - offset[1]]));
                    }
                }
            }
        }
    }
}

#[test]
fn complete_groups_and_nested_existing_transforms_work() {
    let mut builder = RecipeBuilder::new();
    let import = builder.add_import(unit_box()).unwrap();
    let child = builder
        .add(NodeKind::MeshImport {
            import,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let second = builder
        .add(NodeKind::Instance {
            of: child,
            placement: Placement3::translate(3.0, 0.0, 0.0),
        })
        .unwrap();
    let group = builder
        .add(NodeKind::Group {
            children: vec![child, second],
        })
        .unwrap();
    let projected = builder
        .add(NodeKind::UvMap {
            options: exedra_mesh::uv_mapping::UvMappingOptions::default(),
            child: group,
            mapping: mapping(UvProjection::Box(BoxUv::default())),
        })
        .unwrap();
    let root = builder
        .add(NodeKind::UvMap {
            options: exedra_mesh::uv_mapping::UvMappingOptions::default(),
            child: projected,
            mapping: UvMapping {
                projection: UvProjection::Existing,
                transform: UvTransform::rotate_offset_scale(0.0, [0.25, 0.5], [-2.0, 3.0]).unwrap(),
            },
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
    assert_eq!(result.bodies.len(), 2);
    assert_eq!(result.report.counters.uv_mapping_passes, 4);
    assert_eq!(result.report.counters.uv_corners_written, 96);
    for placed in &result.bodies {
        placed.body.source_map.check(&placed.body.mesh).unwrap();
        for face in placed.body.mesh.faces() {
            for corner in placed.body.mesh.face_loop(face) {
                assert!(uv(&placed.body.mesh, corner).is_some());
            }
        }
    }
}

#[test]
fn missing_import_uvs_are_a_typed_evaluation_failure() {
    let recipe = import_recipe(quad_mesh(), mapping(UvProjection::Existing));
    let error = evaluate(&recipe, &EvalPolicy::default()).unwrap_err();
    assert_eq!(error.node, recipe.root());
    assert!(matches!(
        error.error,
        TessellateError::UvMapping(exedra_mesh_ops::uv::UvError::Mapping {
            corner: Some(_),
            error: exedra_mesh_ops::uv::UvMappingError::MissingUv,
        })
    ));
}

#[test]
fn generated_geometry_keeps_occurrence_material_defaults_out_of_the_cache() {
    use crate::ir::{Plane3, PrimitiveSpec};
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("a");
    let b = builder.material_slot("b");
    let child = builder
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [1.0; 3] },
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let mapped = builder
        .add(NodeKind::UvMap {
            options: exedra_mesh::uv_mapping::UvMappingOptions::default(),
            child,
            mapping: mapping(UvProjection::Box(BoxUv::default())),
        })
        .unwrap();
    let first = builder
        .with_material(a)
        .add(NodeKind::Transform {
            child: mapped,
            xf: Placement3::IDENTITY,
        })
        .unwrap();
    let second = builder
        .with_material(b)
        .add(NodeKind::Transform {
            child: mapped,
            xf: Placement3::IDENTITY,
        })
        .unwrap();
    let reflected = builder
        .with_material(b)
        .add(NodeKind::Mirror {
            child: mapped,
            plane: Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 0.0,
            },
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Group {
            children: vec![first, second, reflected],
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let mut cache = EvalCache::new();
    let policy = EvalPolicy::default();
    for _ in 0..2 {
        let result = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        assert_eq!(
            result
                .bodies
                .iter()
                .map(|body| body.material)
                .collect::<Vec<_>>(),
            vec![Some(a), Some(b), Some(b)]
        );
        assert!(Rc::ptr_eq(&result.bodies[0].body, &result.bodies[1].body));
        let (original, _) = result.bodies[0]
            .body
            .mesh
            .to_trimesh(&ExtractParams::default());
        let mut original_uvs: Vec<_> = original.uvs.iter().map(|p| p.map(f32::to_bits)).collect();
        original_uvs.sort_unstable();
        let reflected = &result.bodies[2].body;
        reflected.source_map.check(&reflected.mesh).unwrap();
        assert!(reflected.mesh.validate_deep().is_empty());
        let (render, _) = reflected.mesh.to_trimesh(&ExtractParams::default());
        let mut reflected_uvs: Vec<_> = render.uvs.iter().map(|p| p.map(f32::to_bits)).collect();
        reflected_uvs.sort_unstable();
        assert_eq!(original_uvs, reflected_uvs);
    }
}

#[test]
fn authored_mapping_changes_content_identity() {
    let first = import_recipe(quad_mesh(), mapping(UvProjection::Planar(UvFrame::XY)));
    let mut changed = mapping(UvProjection::Planar(UvFrame::XY));
    changed.transform.offset = [2.0, -3.0];
    let second = import_recipe(quad_mesh(), changed);
    assert_ne!(first.recipe_fingerprint(), second.recipe_fingerprint());
    let mut cache = EvalCache::new();
    let policy = EvalPolicy::default();
    let a = evaluate_with_cache(&first, &policy, &mut cache).unwrap();
    let b = evaluate_with_cache(&second, &policy, &mut cache).unwrap();
    let (a, _) = a.bodies[0].body.mesh.to_trimesh(&ExtractParams::default());
    let (b, _) = b.bodies[0].body.mesh.to_trimesh(&ExtractParams::default());
    assert_ne!(a.uvs, b.uvs);
}

#[test]
fn text_roundtrip_preserves_sparse_import_channels_and_mapping_variants() {
    let mut mesh = quad_mesh();
    let corners: Vec<_> = mesh.face_loop(mesh.faces().next().unwrap()).collect();
    op::set_corner_uv(&mut mesh.edit(), corners[1], [2.0, -3.0]).unwrap();
    op::set_corner_normal_override(&mut mesh.edit(), corners[2], Some([1.0, 0.0, 0.0])).unwrap();
    for projection in [
        UvProjection::Existing,
        UvProjection::Face,
        UvProjection::Cylindrical(UvFrame::XY),
        UvProjection::Spherical(UvFrame::XY),
        UvProjection::Toroidal {
            frame: UvFrame::XY,
            major_radius: 2.0,
        },
        UvProjection::Perspective(crate::ir::PerspectiveUv {
            frame: UvFrame {
                origin: [0.0, 0.0, -2.0],
                ..UvFrame::XY
            },
            focal: [1.0; 2],
            near: 0.01,
        }),
        UvProjection::Planar(UvFrame::from_normal([1.0; 3], [1.0, 2.0, 3.0]).unwrap()),
        UvProjection::Box(BoxUv {
            swap_axes: [true, false, true],
            flip_u: [false, true, false],
            flip_v: [true, false, true],
        }),
    ] {
        let recipe = import_recipe(
            mesh.clone(),
            UvMapping {
                projection,
                transform: UvTransform::rotate_offset_scale(0.3, [0.2, 0.5], [-2.0, 4.0]).unwrap(),
            },
        );
        let text = crate::text::dump_recipe(&recipe);
        let rebuilt = crate::text::parse_recipe(&text).unwrap();
        assert_eq!(recipe.recipe_fingerprint(), rebuilt.recipe_fingerprint());
        assert_eq!(text, crate::text::dump_recipe(&rebuilt));
        #[cfg(feature = "serde")]
        {
            let dto = crate::interchange::to_dto(&recipe);
            let json = serde_json::to_string(&dto).unwrap();
            let dto = serde_json::from_str(&json).unwrap();
            let rebuilt = crate::interchange::from_dto(&dto).unwrap();
            assert_eq!(recipe.recipe_fingerprint(), rebuilt.recipe_fingerprint());
            assert_eq!(crate::text::dump_recipe(&rebuilt), text);
        }
    }
}

#[test]
fn secondary_storage_defaults_and_propagation_survive_recipe_roundtrips() {
    use exedra_mesh::attributes::Propagation;
    for dense in [false, true] {
        for authored in [false, true] {
            for rule in [
                Propagation::Unspecified,
                Propagation::Copy,
                Propagation::Clear,
                Propagation::Interpolate,
            ] {
                let mut mesh = quad_mesh();
                if dense {
                    mesh.define_dense_layer(attr::CORNER_UV1, [0.2, 0.8])
                        .unwrap();
                } else {
                    mesh.define_sparse_layer(attr::CORNER_UV1).unwrap();
                }
                mesh.set_layer_propagation(attr::CORNER_UV1, rule).unwrap();
                if authored {
                    let corner = mesh.face_loop(mesh.faces().next().unwrap()).next().unwrap();
                    op::set_attribute(&mut mesh.edit(), attr::CORNER_UV1, corner, [3.0, -5.0])
                        .unwrap();
                }
                let recipe = import_recipe(mesh, mapping(UvProjection::Planar(UvFrame::XY)));
                let text = crate::text::dump_recipe(&recipe);
                let rebuilt = crate::text::parse_recipe(&text).unwrap();
                assert_eq!(recipe.recipe_fingerprint(), rebuilt.recipe_fingerprint());
                assert_eq!(text, crate::text::dump_recipe(&rebuilt));
                #[cfg(feature = "serde")]
                {
                    let dto = crate::interchange::to_dto(&recipe);
                    let json = serde_json::to_vec(&dto).unwrap();
                    let dto = serde_json::from_slice(&json).unwrap();
                    let rebuilt = crate::interchange::from_dto(&dto).unwrap();
                    assert_eq!(recipe.recipe_fingerprint(), rebuilt.recipe_fingerprint());
                    assert_eq!(text, crate::text::dump_recipe(&rebuilt));
                }
            }
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn interchange_rejects_malformed_corner_coverage_and_keeps_legacy_mesh_payloads() {
    use crate::interchange::MeshDto;
    let mesh = quad_mesh();
    let mut dto = MeshDto::from_mesh(&mesh);
    dto.corner_uvs = vec![vec![Some([0.0; 2])]];
    assert!(dto.to_mesh().is_err());
    dto.corner_uvs = vec![vec![Some([f32::NAN; 2]); 4]];
    assert!(dto.to_mesh().is_err());
    let legacy: MeshDto = serde_json::from_value(
        serde_json::json!({"positions": [[0,0,0], [1,0,0], [0,1,0]], "faces": [[0,1,2]]}),
    )
    .unwrap();
    assert_eq!(legacy.to_mesh().unwrap().faces().count(), 1);
}

#[test]
fn mapping_policies_roundtrip_and_participate_in_content_identity() {
    use crate::ir::{UvMappingOptions, UvPolePolicy, UvSeamPolicy, UvSet};
    let mut fingerprints = Vec::new();
    for options in [
        UvMappingOptions::default(),
        UvMappingOptions {
            source: UvSet::Secondary,
            ..UvMappingOptions::default()
        },
        UvMappingOptions {
            target: UvSet::Secondary,
            ..UvMappingOptions::default()
        },
        UvMappingOptions {
            seams: UvSeamPolicy::Preserve,
            ..UvMappingOptions::default()
        },
        UvMappingOptions {
            poles: UvPolePolicy::Refuse,
            ..UvMappingOptions::default()
        },
    ] {
        let mut builder = RecipeBuilder::new();
        let imported = builder.add_import(quad_mesh()).unwrap();
        let child = builder
            .add(NodeKind::MeshImport {
                import: imported,
                placement: Placement3::IDENTITY,
            })
            .unwrap();
        let root = builder
            .add(NodeKind::UvMap {
                child,
                mapping: mapping(UvProjection::Existing),
                options,
            })
            .unwrap();
        let recipe = builder.finish(root).unwrap();
        let fingerprint = recipe.recipe_fingerprint();
        assert!(!fingerprints.contains(&fingerprint));
        fingerprints.push(fingerprint);
        let text = crate::text::dump_recipe(&recipe);
        let rebuilt = crate::text::parse_recipe(&text).unwrap();
        assert_eq!(fingerprint, rebuilt.recipe_fingerprint());
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_vec(&crate::interchange::to_dto(&recipe)).unwrap();
            let dto = serde_json::from_slice(&json).unwrap();
            let rebuilt = crate::interchange::from_dto(&dto).unwrap();
            assert_eq!(fingerprint, rebuilt.recipe_fingerprint());
        }
    }
}
