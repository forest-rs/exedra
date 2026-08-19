// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hostile-input torture: seeded random abuse of the public builder and
//! evaluation surfaces, asserting the never-panic contract.
//!
//! External spec compilers feed arbitrary spec data through this API;
//! whatever they produce, the answer must be a typed error or a valid
//! result — never a panic, never non-finite geometry. The corpus is
//! seeded and trig-free, so failures reproduce exactly.

use alloc::vec::Vec;

use crate::evaluate::evaluate;
use crate::ir::{
    CapMode, CsgOp, ImportId, NodeId, NodeKind, Path3, Placement3, Plane3, PolicyId, PrimitiveSpec,
    ProfileId, RecipeBuilder, SlotId, SourceId,
};
use crate::profile::{Loop2, Profile2, Seg2, SegKind};
use crate::tessellate::EvalPolicy;

/// Fuzzing policy: coarse discretization keeps hostile huge-radius arcs
/// from ballooning into thousands of edges — panic-freedom does not need
/// dense rings.
fn fuzz_policy() -> EvalPolicy {
    EvalPolicy {
        discretize: crate::discretize::DiscretizePolicy {
            chord_tolerance: 0.5,
            max_segment_edges: 24,
            min_arc_edges: 2,
        },
        ..EvalPolicy::default()
    }
}

/// `SplitMix64` (same generator as the triangulation torture suite).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self, lo: f64, hi: f64) -> f64 {
        let t = (self.next() >> 44) as f64 / (1_u64 << 20) as f64;
        lo + t * (hi - lo)
    }

    fn range(&mut self, n: usize) -> usize {
        #[expect(clippy::cast_possible_truncation, reason = "small test-corpus bound")]
        {
            (self.next() % n as u64) as usize
        }
    }

    /// Values chosen to be hostile: NaNs, infinities, huge magnitudes,
    /// zeros, denormals — mixed with enough reasonable values that some
    /// recipes genuinely build and evaluate.
    fn hostile_f64(&mut self) -> f64 {
        match self.range(14) {
            0 => f64::NAN,
            1 => f64::INFINITY,
            2 => f64::NEG_INFINITY,
            3 => 1e300,
            4 => -1e300,
            5 => 0.0,
            6 => f64::MIN_POSITIVE / 8.0,
            _ => self.unit(-100.0, 100.0),
        }
    }

    fn hostile_point(&mut self) -> (f64, f64) {
        (self.hostile_f64(), self.hostile_f64())
    }

    /// A hostile 32-bit id (truncation is the point: garbage in).
    fn hostile_u32(&mut self) -> u32 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "hostile ids are arbitrary garbage by design"
        )]
        {
            self.next() as u32
        }
    }
}

/// Narrows small test counters (bounded far below u32).
fn small_u32(n: usize) -> u32 {
    u32::try_from(n).expect("test counters are tiny")
}

fn hostile_seg(rng: &mut Rng) -> Seg2 {
    let to = rng.hostile_point();
    match rng.range(4) {
        0 => Seg2::line(to),
        1 => Seg2::arc(to, rng.hostile_f64()),
        2 => Seg2::cubic(to, rng.hostile_point(), rng.hostile_point()),
        _ => Seg2::policy(
            to,
            PolicyId(rng.hostile_u32()),
            SegKind::Arc {
                bulge: rng.hostile_f64(),
            },
        ),
    }
}

fn hostile_placement(rng: &mut Rng) -> Placement3 {
    if rng.range(3) == 0 {
        Placement3::IDENTITY
    } else {
        let mut rows = [[0.0; 4]; 3];
        for row in &mut rows {
            for cell in row.iter_mut() {
                *cell = rng.hostile_f64();
            }
        }
        Placement3 { rows }
    }
}

fn hostile_kind(rng: &mut Rng, profiles: usize, nodes: usize) -> NodeKind {
    let profile = ProfileId(rng.hostile_u32() % (small_u32(profiles.max(1)) + 3));
    let node = NodeId(rng.hostile_u32() % (small_u32(nodes.max(1)) + 3));
    match rng.range(12) {
        0 => NodeKind::Extrude {
            profile,
            placement: hostile_placement(rng),
            height: rng.hostile_f64(),
            caps: CapMode::Both,
        },
        1 => NodeKind::Revolve {
            profile,
            placement: hostile_placement(rng),
            sweep: rng.hostile_f64(),
            caps: CapMode::Start,
        },
        2 => NodeKind::Loft {
            sections: (0..rng.range(4))
                .map(|_| (hostile_placement(rng), profile))
                .collect(),
            policy: crate::ir::LoftPolicy::Ruled,
            caps: CapMode::Both,
        },
        3 => NodeKind::Sweep {
            profile,
            path: Path3::Polyline {
                points: (0..rng.range(5))
                    .map(|_| [rng.hostile_f64(), rng.hostile_f64(), rng.hostile_f64()])
                    .collect(),
                frame: crate::ir::FramePolicy::RotationMinimizing,
            },
            caps: CapMode::Both,
        },
        4 => NodeKind::PlanarFace {
            profile,
            placement: hostile_placement(rng),
        },
        5 => NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [rng.hostile_f64(), rng.hostile_f64(), rng.hostile_f64()],
            },
            placement: hostile_placement(rng),
        },
        6 => NodeKind::Csg {
            op: CsgOp::Difference,
            operands: (0..rng.range(4)).map(|_| node).collect(),
        },
        7 => NodeKind::Transform {
            child: node,
            xf: hostile_placement(rng),
        },
        8 => NodeKind::Mirror {
            child: node,
            plane: Plane3 {
                normal: [rng.hostile_f64(), rng.hostile_f64(), rng.hostile_f64()],
                distance: rng.hostile_f64(),
            },
        },
        9 => NodeKind::Instance {
            of: node,
            placement: hostile_placement(rng),
        },
        10 => NodeKind::Group {
            children: (0..rng.range(4)).map(|_| node).collect(),
        },
        _ => NodeKind::MeshImport {
            import: ImportId(rng.hostile_u32()),
            placement: hostile_placement(rng),
        },
    }
}

#[test]
fn hostile_profiles_never_panic() {
    let mut rng = Rng(0x40B1_0001);
    let mut accepted = 0;
    for _ in 0..500 {
        let count = rng.range(8);
        let segs: Vec<Seg2> = (0..count).map(|_| hostile_seg(&mut rng)).collect();
        // Every outcome must be a typed Result, never a panic.
        if let Ok(outer) = Loop2::new(segs) {
            let holes: Vec<Loop2> = (0..rng.range(3))
                .filter_map(|_| {
                    let segs: Vec<Seg2> =
                        (0..rng.range(6)).map(|_| hostile_seg(&mut rng)).collect();
                    Loop2::new(segs).ok()
                })
                .collect();
            if Profile2::new(outer, holes).is_ok() {
                accepted += 1;
            }
        }
    }
    // The corpus is hostile, but some inputs are genuinely valid.
    let _ = accepted;
}

#[test]
fn hostile_recipes_never_panic_and_evaluate_typed() {
    let mut rng = Rng(0x40B1_0002);
    let mut evaluated = 0;
    let mut eval_errors = 0;
    for _ in 0..120 {
        let mut b = RecipeBuilder::new();
        // Random interning, sometimes with odd content.
        for _ in 0..rng.range(3) {
            b.source_ref("hostile:src");
            b.material_slot("hostile-slot");
            b.curve_policy("hostile.policy@0");
        }
        // One guaranteed-valid profile keeps the corpus honest about the
        // success path; everything after it is hostile.
        b.add_profile(crate::builders::rect(2.0, 1.0).expect("rect"));
        let mut profiles = 1;
        for _ in 0..rng.range(4) {
            let segs: Vec<Seg2> = (0..rng.range(7)).map(|_| hostile_seg(&mut rng)).collect();
            if let Ok(outer) = Loop2::new(segs)
                && let Ok(profile) = Profile2::simple(outer)
            {
                b.add_profile(profile);
                profiles += 1;
            }
        }
        // One guaranteed-evaluable node keeps the success path exercised.
        let _ = b.add(NodeKind::Extrude {
            profile: ProfileId(0),
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        });
        let valid_source = b.source_ref("hostile:valid");
        let valid_slot = b.material_slot("hostile:valid-slot");
        let mut nodes: u32 = 1;
        for _ in 0..rng.range(8) {
            // Bindings alternate between valid interned ids and hostile
            // garbage; add() must answer garbage with typed errors, never
            // panic (a fuzz-caught panic lived exactly here).
            match rng.range(3) {
                0 => {
                    b.with_source(SourceId(rng.hostile_u32()))
                        .with_material(SlotId(rng.hostile_u32()));
                }
                1 => {
                    b.with_source(valid_source).with_material(valid_slot);
                }
                _ => {}
            }
            if b.add(hostile_kind(&mut rng, profiles, nodes as usize))
                .is_ok()
            {
                nodes += 1;
            }
        }
        // Roots alternate between hostile and valid so both finish paths
        // and evaluation get exercised.
        let root = if rng.range(2) == 0 {
            NodeId(rng.hostile_u32())
        } else {
            NodeId(nodes - 1)
        };
        let Ok(recipe) = b.finish(root) else {
            continue;
        };
        // Built recipes must evaluate to a typed outcome, and any emitted
        // geometry must be finite and valid.
        match evaluate(&recipe, &fuzz_policy()) {
            Ok(result) => {
                evaluated += 1;
                for placed in &result.bodies {
                    assert!(
                        placed.body.mesh.validate_deep().is_empty(),
                        "hostile input must never produce invalid meshes"
                    );
                    for vertex in placed.body.mesh.vertices() {
                        if let Some(p) = placed.body.mesh.vertex_position(vertex) {
                            assert!(
                                p.iter().all(|c| c.is_finite()),
                                "hostile input must never produce non-finite geometry"
                            );
                        }
                    }
                }
            }
            Err(_) => eval_errors += 1,
        }
    }
    assert!(evaluated > 0, "corpus must include evaluable recipes");
    let _ = eval_errors;
}

#[test]
fn hostile_ids_never_panic_on_lookups() {
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(crate::builders::rect(1.0, 1.0).expect("rect"));
    let n = b
        .add(NodeKind::Extrude {
            profile: p,
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        })
        .expect("valid");
    let recipe = b.finish(n).expect("valid recipe");
    // All accessors answer hostile ids with None, never a panic.
    assert!(recipe.node(NodeId(u32::MAX)).is_none());
    assert!(recipe.profile(ProfileId(u32::MAX)).is_none());
    assert!(recipe.source(SourceId(u32::MAX)).is_none());
    assert!(recipe.slot(SlotId(u32::MAX)).is_none());
    assert!(recipe.policy(PolicyId(u32::MAX)).is_none());
    assert!(recipe.import(ImportId(u32::MAX)).is_none());
    assert!(recipe.fingerprint(NodeId(u32::MAX)).is_none());
}

#[test]
fn extreme_finite_parameters_fail_typed_not_infinite() {
    // 1e300 is finite in f64 but overflows the f32 mesh boundary; the
    // narrowing guard must reject it rather than emit infinite geometry.
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(crate::builders::rect(1.0, 1.0).expect("rect"));
    let n = b
        .add(NodeKind::Extrude {
            profile: p,
            placement: Placement3::IDENTITY,
            height: 1e300,
            caps: CapMode::Both,
        })
        .expect("finite parameters pass IR validation");
    let recipe = b.finish(n).expect("valid recipe");
    let result = evaluate(&recipe, &EvalPolicy::default());
    assert!(
        matches!(
            result,
            Err(crate::evaluate::EvalError {
                error: crate::tessellate::TessellateError::NonFiniteGeometry,
                ..
            })
        ),
        "expected the narrowing guard, got {result:?}"
    );
}
