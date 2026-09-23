// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Plans `EXT_mesh_gpu_instancing` batches: repeated leaf placements of one
//! part, with one material resolution, under one parent.

use std::collections::HashMap;

use exedra_assembly::{Assembly, CompiledParts, InstanceId, PartId};
use exedra_constructive::ir::Placement3;
use exedra_math::Quat;

use crate::{GltfInstancing, GltfStats, resolved_regions};

/// The glTF extension name.
pub(crate) const EXTENSION: &str = "EXT_mesh_gpu_instancing";

/// Largest `|cos|` between two scaled placement axes that still counts as
/// perpendicular.
///
/// Rotations built in `f64` sit near `1e-16`, and rigid placements that
/// passed through `f32` (imported scenes, editor data) near `1e-7`; both
/// batch. A deliberate shear is orders of magnitude larger. The residual
/// shear a batched instance can lose is below `f32` accessor precision
/// relative to its scale.
pub(crate) const SHEAR_TOLERANCE: f64 = 1e-6;

/// One instance transform decomposed for `TRANSLATION`, `ROTATION`, `SCALE`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Trs {
    pub(crate) translation: [f32; 3],
    /// Unit quaternion `x, y, z, w`.
    pub(crate) rotation: [f32; 4],
    pub(crate) scale: [f32; 3],
}

/// Why a placement cannot be written as a positive-scale TRS.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum Unbatchable {
    /// Negative determinant. glTF flips winding by the determinant of the
    /// node transform, which instanced renderers commonly ignore per
    /// instance, so reflections keep their own node.
    Mirrored,
    /// Non-perpendicular or degenerate axes: no TRS represents it.
    Sheared,
}

/// Decomposes a placement into translation, rotation and positive scale.
pub(crate) fn decompose(placement: &Placement3) -> Result<Trs, Unbatchable> {
    let rows = placement.rows;
    let column = |j: usize| [rows[0][j], rows[1][j], rows[2][j]];
    let axes = [column(0), column(1), column(2)];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let scale = axes.map(|axis| dot(axis, axis).sqrt());
    if scale.iter().any(|s| !s.is_finite() || *s == 0.0) {
        return Err(Unbatchable::Sheared);
    }
    for (i, j) in [(0, 1), (0, 2), (1, 2)] {
        if dot(axes[i], axes[j]).abs() > SHEAR_TOLERANCE * scale[i] * scale[j] {
            return Err(Unbatchable::Sheared);
        }
    }
    let [a, b, c] = axes;
    let cross = [
        b[1] * c[2] - b[2] * c[1],
        b[2] * c[0] - b[0] * c[2],
        b[0] * c[1] - b[1] * c[0],
    ];
    if dot(a, cross) < 0.0 {
        return Err(Unbatchable::Mirrored);
    }
    let unit = |r: usize| [0, 1, 2].map(|j| rows[r][j] / scale[j]);
    let rotation =
        Quat::from_rotation_rows([unit(0), unit(1), unit(2)]).ok_or(Unbatchable::Sheared)?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "EXT_mesh_gpu_instancing stores transforms as FLOAT accessors"
    )]
    Ok(Trs {
        translation: [rows[0][3] as f32, rows[1][3] as f32, rows[2][3] as f32],
        rotation: [
            rotation.x as f32,
            rotation.y as f32,
            rotation.z as f32,
            rotation.w as f32,
        ],
        scale: scale.map(|s| s as f32),
    })
}

/// Instances of one part and material resolution under one parent.
#[derive(Debug)]
pub(crate) struct Batch {
    pub(crate) parent: Option<InstanceId>,
    pub(crate) part: PartId,
    /// Members in instance order.
    pub(crate) members: Vec<(InstanceId, Trs)>,
}

/// Instances grouped for batching, each with its decomposition result.
type Candidates = (
    Option<InstanceId>,
    PartId,
    Vec<(InstanceId, Result<Trs, Unbatchable>)>,
);

/// The instancing decision for a whole export.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Per instance index: folded into a batch, so it gets no node of its own.
    pub(crate) batched: Vec<bool>,
    /// Batches of at least two members, ordered by their first member.
    pub(crate) batches: Vec<Batch>,
}

/// Groups batchable instances. Candidates are leaf instances with geometry,
/// grouped by parent, part and material resolution. Groups of one keep their
/// node. A candidate whose placement is mirrored or sheared keeps its node,
/// and is counted when its group has another member, so it would otherwise
/// have been batched.
pub(crate) fn plan(
    assembly: &Assembly,
    compiled: &CompiledParts,
    mode: GltfInstancing,
    stats: &mut GltfStats,
) -> Plan {
    let count = assembly.instances_with_ids().len();
    let mut plan = Plan {
        batched: vec![false; count],
        batches: Vec::new(),
    };
    if mode != GltfInstancing::GpuInstancing {
        return plan;
    }
    type Key = (Option<u32>, u32, Vec<Vec<Option<String>>>);
    let mut groups: HashMap<Key, usize> = HashMap::new();
    let mut candidates: Vec<Candidates> = Vec::new();
    for (id, instance) in assembly.instances_with_ids() {
        let Some(part) = instance.part() else {
            continue;
        };
        let entry = compiled.part(part).expect("matching compilation");
        if !instance.children().is_empty() || entry.bodies.iter().all(|b| b.tri.indices.is_empty())
        {
            continue;
        }
        let trs = decompose(instance.placement());
        let def = assembly.part(part).expect("validated instance part");
        let resolution = entry
            .bodies
            .iter()
            .map(|body| {
                resolved_regions(assembly, def, id, body)
                    .into_iter()
                    .map(|region| region.material)
                    .collect()
            })
            .collect();
        let key = (instance.parent().map(|p| p.0), part.0, resolution);
        let index = *groups.entry(key).or_insert_with(|| {
            candidates.push((instance.parent(), part, Vec::new()));
            candidates.len() - 1
        });
        candidates[index].2.push((id, trs));
    }
    for (parent, part, group) in candidates {
        if group.len() < 2 {
            continue;
        }
        let mut members = Vec::new();
        for (id, trs) in group {
            match trs {
                Ok(trs) => members.push((id, trs)),
                Err(Unbatchable::Mirrored) => stats.unbatched_mirrored_instances += 1,
                Err(Unbatchable::Sheared) => stats.unbatched_sheared_instances += 1,
            }
        }
        if members.len() < 2 {
            continue;
        }
        for (id, _) in &members {
            plan.batched[id.0 as usize] = true;
        }
        plan.batches.push(Batch {
            parent,
            part,
            members,
        });
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rebuilds the 3x4 rows `T * R * S` from a decomposition.
    fn compose(trs: &Trs) -> [[f64; 4]; 3] {
        let q = Quat::new(
            f64::from(trs.rotation[0]),
            f64::from(trs.rotation[1]),
            f64::from(trs.rotation[2]),
            f64::from(trs.rotation[3]),
        );
        let r = q.to_rotation_rows();
        core::array::from_fn(|i| {
            let mut row = [0.0; 4];
            for (j, cell) in row.iter_mut().take(3).enumerate() {
                *cell = r[i][j] * f64::from(trs.scale[j]);
            }
            row[3] = f64::from(trs.translation[i]);
            row
        })
    }

    #[test]
    fn decomposition_round_trips_general_rotations_and_scales() {
        let placement =
            Placement3::euler_extrinsic_xyz_then_translate(0.3, -1.1, 2.4, [1.5, -2.0, 0.25]);
        let mut rows = placement.rows;
        for row in &mut rows {
            row[0] *= 0.5;
            row[1] *= 2.0;
            row[2] *= 3.5;
        }
        let trs = decompose(&Placement3 { rows }).unwrap();
        let rebuilt = compose(&trs);
        for i in 0..3 {
            for j in 0..4 {
                assert!(
                    (rebuilt[i][j] - rows[i][j]).abs() < 1e-5,
                    "row {i} col {j}: {} vs {}",
                    rebuilt[i][j],
                    rows[i][j]
                );
            }
        }
    }

    #[test]
    fn rigid_placements_from_single_precision_batch() {
        let placement = Placement3::euler_extrinsic_xyz_then_translate(0.7, 0.2, -1.3, [0.0; 3]);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "simulates a placement that passed through f32"
        )]
        let rows = placement.rows.map(|row| row.map(|v| f64::from(v as f32)));
        assert!(decompose(&Placement3 { rows }).is_ok());
        let mut sheared = placement.rows;
        sheared[0][1] += 0.01;
        assert_eq!(
            decompose(&Placement3 { rows: sheared }),
            Err(Unbatchable::Sheared)
        );
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn decomposes_rotation_scale_and_translation() {
        let placement =
            Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, 0.0, 0.0, 0.0);
        let mut rows = placement.rows;
        for row in &mut rows {
            row[0] *= 2.0;
            row[1] *= 3.0;
            row[2] *= 4.0;
        }
        rows[0][3] = 5.0;
        rows[2][3] = -1.0;
        let trs = decompose(&Placement3 { rows }).unwrap();
        assert_eq!(trs.translation, [5.0, 0.0, -1.0]);
        assert!(
            trs.scale
                .iter()
                .zip([2.0, 3.0, 4.0])
                .all(|(a, b)| close(*a, b))
        );
        let half = core::f32::consts::FRAC_1_SQRT_2;
        let [x, y, z, w] = trs.rotation;
        assert!(close(x, 0.0) && close(y, 0.0) && close(z, half) && close(w, half));
    }

    #[test]
    fn refuses_mirrors_and_shears() {
        let mut mirrored = Placement3::IDENTITY.rows;
        mirrored[0][0] = -1.0;
        assert_eq!(
            decompose(&Placement3 { rows: mirrored }),
            Err(Unbatchable::Mirrored)
        );
        let mut sheared = Placement3::IDENTITY.rows;
        sheared[0][1] = 0.25;
        assert_eq!(
            decompose(&Placement3 { rows: sheared }),
            Err(Unbatchable::Sheared)
        );
        let mut flat = Placement3::IDENTITY.rows;
        flat[2][2] = 0.0;
        assert_eq!(
            decompose(&Placement3 { rows: flat }),
            Err(Unbatchable::Sheared)
        );
    }
}
