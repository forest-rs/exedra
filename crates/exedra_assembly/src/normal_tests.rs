// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_mesh::{Mesh, MeshBuilder, op};

fn triangle(normals: &[[f32; 3]]) -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2]).unwrap();
    let mut mesh = builder.build().unwrap().mesh;
    let corners: Vec<_> = mesh.faces().flat_map(|f| mesh.face_loop(f)).collect();
    let mut edit = mesh.edit();
    for (&corner, &normal) in corners.iter().zip(normals) {
        op::set_corner_normal_override(&mut edit, corner, Some(normal)).unwrap();
    }
    let _: () = edit.finish();
    mesh
}

#[test]
fn normal_policy_handles_partial_coverage_and_separates_cached_results() {
    let mut assembly = Assembly::new();
    let part = assembly
        .add_baked_part("partial", triangle(&[[0.6, 0.0, 0.8]]), &[])
        .unwrap();
    let mut compiler = PartCompiler::new();
    let default = CompilePolicy::default();
    assert_eq!(default.normals, NormalsSource::Derived);
    let derived = compiler.compile_parts(&assembly, &default).unwrap();
    assert_eq!(
        derived.part(part).unwrap().bodies[0].tri.normals,
        [[0.0, 0.0, 1.0]; 3]
    );

    let hybrid = CompilePolicy {
        normals: NormalsSource::CustomOrDerived,
        ..default
    };
    let preserved = compiler.compile_parts(&assembly, &hybrid).unwrap();
    let tri = &preserved.part(part).unwrap().bodies[0].tri;
    for (position, normal) in tri.positions.iter().zip(&tri.normals) {
        assert_eq!(
            *normal,
            if *position == [1.0, 0.0, 0.0] {
                [0.6, 0.0, 0.8]
            } else {
                [0.0, 0.0, 1.0]
            }
        );
    }

    let custom = CompilePolicy {
        normals: NormalsSource::CustomOnly,
        ..default
    };
    let only = compiler.compile_parts(&assembly, &custom).unwrap();
    let tri = &only.part(part).unwrap().bodies[0].tri;
    for (position, normal) in tri.positions.iter().zip(&tri.normals) {
        assert_eq!(
            *normal,
            if *position == [1.0, 0.0, 0.0] {
                [0.6, 0.0, 0.8]
            } else {
                [0.0; 3]
            }
        );
    }
    assert_eq!(compiler.counters().parts_compiled, 3);
    assert_eq!(compiler.counters().cache_hits, 0);
    assert_ne!(policy_fingerprint(&default), policy_fingerprint(&hybrid));
    assert_ne!(policy_fingerprint(&hybrid), policy_fingerprint(&custom));
    let reused = compiler.compile_parts(&assembly, &default).unwrap();
    assert!(Arc::ptr_eq(
        reused.part(part).unwrap(),
        derived.part(part).unwrap()
    ));
    assert_eq!(compiler.counters().parts_compiled, 3);
    assert_eq!(compiler.counters().cache_hits, 1);
}

#[test]
fn baked_attribute_changes_cannot_reuse_topology_only_geometry() {
    let mut assembly = Assembly::new();
    let a = assembly
        .add_baked_part("a", triangle(&[[0.6, 0.0, 0.8]; 3]), &[])
        .unwrap();
    let b = assembly
        .add_baked_part("b", triangle(&[[0.0, 0.6, 0.8]; 3]), &[])
        .unwrap();
    let policy = CompilePolicy {
        normals: NormalsSource::CustomOrDerived,
        ..CompilePolicy::default()
    };
    let mut compiler = PartCompiler::new();
    let compiled = compiler.compile_parts(&assembly, &policy).unwrap();
    let first = compiled.part(a).unwrap();
    let second = compiled.part(b).unwrap();
    assert_ne!(first.fingerprint, second.fingerprint);
    assert_eq!(first.bodies[0].tri.normals, [[0.6, 0.0, 0.8]; 3]);
    assert_eq!(second.bodies[0].tri.normals, [[0.0, 0.6, 0.8]; 3]);
    assert_eq!(compiler.counters().parts_compiled, 2);

    let mut previous = second.clone();
    for (uv, region) in [([0.25, 0.75], 0), ([0.25, 0.75], 17)] {
        let mut changed = triangle(&[[0.0, 0.6, 0.8]; 3]);
        let face = changed.faces().next().unwrap();
        let corners: Vec<_> = changed.face_loop(face).collect();
        let mut edit = changed.edit();
        for corner in corners {
            op::set_corner_uv(&mut edit, corner, uv).unwrap();
        }
        op::set_face_region(&mut edit, face, region).unwrap();
        let _: () = edit.finish();
        assembly
            .replace_part_source(b, PartSource::Baked(changed))
            .unwrap();
        // Change UVs, then only the region. Neither edit requires a dirty hint.
        let updated = compiler.compile_parts(&assembly, &policy).unwrap();
        let updated_b = updated.part(b).unwrap();
        assert_ne!(updated_b.fingerprint, previous.fingerprint);
        assert_eq!(
            updated_b.bodies[0].tri.normals,
            second.bodies[0].tri.normals
        );
        assert_eq!(updated_b.bodies[0].tri.uvs, [uv; 3]);
        assert_eq!(updated_b.bodies[0].regions[0].region, region);
        assert!(Arc::ptr_eq(updated.part(a).unwrap(), first));
        previous = updated_b.clone();
    }
    assert_eq!(compiler.counters().parts_compiled, 4);
}

#[test]
fn compile_identity_includes_constructive_refinement_settings() {
    let mut assembly = Assembly::new();
    assembly.add_baked_part("part", triangle(&[]), &[]).unwrap();
    let mut compiler = PartCompiler::new();
    let default = CompilePolicy::default();
    let mut refined = default;
    refined.evaluation.cap_refinement.get_or_insert_default();
    assert_ne!(policy_fingerprint(&default), policy_fingerprint(&refined));
    compiler.compile_parts(&assembly, &default).unwrap();
    compiler.compile_parts(&assembly, &refined).unwrap();
    assert_eq!(compiler.counters().parts_compiled, 2);
}

#[test]
fn rebuilt_assemblies_cannot_reuse_baked_attribute_fingerprints() {
    let policy = CompilePolicy {
        normals: NormalsSource::CustomOrDerived,
        ..CompilePolicy::default()
    };
    let mut compiler = PartCompiler::new();
    for normal in [[0.6, 0.0, 0.8], [0.0, 0.6, 0.8]] {
        let mut assembly = Assembly::new();
        let part = assembly
            .add_baked_part("part", triangle(&[normal; 3]), &[])
            .unwrap();
        let compiled = compiler.compile_parts(&assembly, &policy).unwrap();
        assert_eq!(
            compiled.part(part).unwrap().bodies[0].tri.normals,
            [normal; 3]
        );
    }
    assert_eq!(compiler.counters().parts_compiled, 2);
}

#[test]
fn baked_fingerprints_keep_positions_attached_to_sparse_vertex_ids() {
    let mut compiler = PartCompiler::new();
    for sparse in [false, true] {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        if sparse {
            builder.push_vertex([9.0; 3]);
        }
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 0.0, 1.0]);
        builder.add_face(&[0, 2, 3]).unwrap();
        let mut mesh = builder.build().unwrap().mesh;
        if sparse {
            let unused = mesh.vertices().nth(1).unwrap();
            let mut edit = mesh.edit();
            op::delete_vertices(&mut edit, &[unused]).unwrap();
            let _: () = edit.finish();
        }
        // Equal live-position sequences and face indices describe different
        // triangles when the vertex ids have different gaps.
        let mut assembly = Assembly::new();
        let part = assembly.add_baked_part("part", mesh, &[]).unwrap();
        let compiled = compiler
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        let expected = if sparse {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        assert_eq!(
            compiled.part(part).unwrap().bodies[0].tri.normals,
            [expected; 3]
        );
    }
    assert_eq!(compiler.counters().parts_compiled, 2);
}
