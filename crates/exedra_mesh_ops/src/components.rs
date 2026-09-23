// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit removal of small edge-connected face components.

use alloc::vec::Vec;
use exedra_mesh::{FaceId, Mesh, op};

/// Evidence from a component cleanup pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentCleanup {
    /// Source face IDs actually removed, in canonical order.
    pub removed_faces: Vec<FaceId>,
    /// Faces visited while finding components.
    pub faces_examined: u64,
    /// Edge-connected components visited.
    pub components_examined: u64,
    /// Caller-defined attribute values cleared with the removed elements
    /// ([`ChangeSet::cleared_attribute_values`](exedra_mesh::ChangeSet::cleared_attribute_values)).
    pub cleared_attribute_values: u64,
}

/// Removes entire edge-connected face components smaller than `min_faces`.
///
/// Traversal ignores region tags. A zero threshold removes nothing. Isolated
/// vertices left by removed faces are cleaned up. Deletion uses the kernel's
/// checked preflight and propagates its failure; success reports actual removals.
/// Caller-defined attribute values on removed elements are cleared and counted
/// in [`ComponentCleanup::cleared_attribute_values`]. Other source meanings
/// remain the caller's responsibility.
pub fn remove_small_components(
    mesh: &mut Mesh,
    min_faces: u32,
) -> Result<ComponentCleanup, op::DeleteFacesError> {
    let faces: Vec<FaceId> = mesh.faces().collect();
    // BTreeSet: deterministic and no extra dependency.
    let mut assigned: alloc::collections::BTreeSet<FaceId> = alloc::collections::BTreeSet::new();
    let mut doomed: Vec<FaceId> = Vec::new();
    let mut components_examined = 0_u64;

    for &seed in &faces {
        if assigned.contains(&seed) {
            continue;
        }
        components_examined += 1;
        // Flood-fill one component across shared edges.
        let mut component = alloc::vec![seed];
        assigned.insert(seed);
        let mut cursor = 0;
        while cursor < component.len() {
            let face = component[cursor];
            cursor += 1;
            for he in mesh.face_loop(face) {
                if let Some(twin) = mesh.twin(he)
                    && let Some(neighbor) = mesh.face(twin)
                    && neighbor != FaceId::OUTSIDE
                    && !assigned.contains(&neighbor)
                {
                    assigned.insert(neighbor);
                    component.push(neighbor);
                }
            }
        }
        if (component.len() as u64) < u64::from(min_faces) {
            doomed.extend(component);
        }
    }

    doomed.sort_unstable();
    let mut cleared_attribute_values = 0;
    if !doomed.is_empty() {
        let mut edit = mesh.edit_with(exedra_mesh::ChangeSetBuilder::new());
        let result = op::delete_faces(
            &mut edit,
            &doomed,
            exedra_mesh::DeletePolicy::CleanupIsolated,
        );
        cleared_attribute_values = edit.finish().cleared_attribute_values;
        result?;
    }
    Ok(ComponentCleanup {
        removed_faces: doomed,
        faces_examined: faces.len() as u64,
        components_examined,
        cleared_attribute_values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn cleanup_reports_removed_ids_and_ignores_region_boundaries() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [3.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2], &[0, 2, 3], &[4, 5, 6]],
        )
        .unwrap();
        let faces: Vec<_> = mesh.faces().collect();
        let mut edit = mesh.edit();
        op::set_face_region(&mut edit, faces[0], 5).unwrap();
        op::set_face_region(&mut edit, faces[1], 6).unwrap();
        let _: () = edit.finish();
        let revision = mesh.revision();
        let disabled = remove_small_components(&mut mesh, 0).unwrap();
        assert!(disabled.removed_faces.is_empty());
        assert_eq!(mesh.revision(), revision);
        let cleaned = remove_small_components(&mut mesh, 2).unwrap();
        assert_eq!(cleaned.faces_examined, 3);
        assert_eq!(cleaned.components_examined, 2);
        assert_eq!(cleaned.removed_faces, vec![faces[2]]);
        assert_eq!(mesh.faces().collect::<Vec<_>>(), faces[..2]);
        assert_eq!(mesh.vertices().count(), 4);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }
}
