// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Seeded fuzz over the boolean pipeline (test-only).
//!
//! A fixed `SplitMix64` corpus (the same generator as
//! `exedra_triangulate`'s torture suite) drives random convex solids —
//! jittered boxes and polygonal prisms at random offsets, scales, and
//! exact rational rotations — through every boolean operation. The
//! contract under fuzz:
//!
//! - the pipeline never panics;
//! - `Ok` results are deeply valid with plausible volumes (bounded by the
//!   operands' inclusion-exclusion envelope);
//! - `Err` results are typed and leave diagnostics behind;
//! - reruns are bit-identical.
//!
//! Coordinates avoid trig entirely (rotations come from Pythagorean
//! ratios), so the corpus is bit-identical across platforms.
//!
//! The same corpus also drives random `split_edge`/`split_face` op
//! sequences with deep validation after every op — the topology-edit
//! half of the fuzz ticket (collapse/flip ops don't exist yet).

use alloc::vec::Vec;

use super::{
    BooleanDiagnostics, BooleanError, BooleanOp, BooleanOutput, BooleanScratch, boolean_mesh,
};
use crate::{FaceTriangulation, Mesh, MeshBuilder};

/// `SplitMix64`: tiny deterministic PRNG for corpus generation.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `[lo, hi)` with 20 bits of entropy.
    fn unit(&mut self, lo: f64, hi: f64) -> f64 {
        let t = (self.next() >> 44) as f64 / (1_u64 << 20) as f64;
        lo + t * (hi - lo)
    }

    fn range(&mut self, n: usize) -> usize {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "n is a small corpus bound; the modulo keeps the value below it"
        )]
        {
            (self.next() % n as u64) as usize
        }
    }
}

/// Exact unit rotations from Pythagorean ratios: `(cos, sin)` pairs that
/// need no trig and stay deterministic across platforms.
const ROTATIONS: [[f64; 2]; 6] = [
    [1.0, 0.0],
    [0.0, 1.0],
    [0.6, 0.8],
    [5.0 / 13.0, 12.0 / 13.0],
    [15.0 / 17.0, 8.0 / 17.0],
    [21.0 / 29.0, 20.0 / 29.0],
];

/// Convex prism cross-section templates (counter-clockwise, around the
/// origin); scaled, rotated, and translated per solid.
const SECTIONS: [&[[f64; 2]]; 4] = [
    &[[1.0, 0.0], [-0.5, 0.9], [-0.5, -0.9]],
    &[[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]],
    &[
        [1.0, 0.0],
        [0.3, 0.95],
        [-0.8, 0.6],
        [-0.8, -0.6],
        [0.3, -0.95],
    ],
    &[
        [1.0, 0.0],
        [0.5, 0.85],
        [-0.5, 0.85],
        [-1.0, 0.0],
        [-0.5, -0.85],
        [0.5, -0.85],
    ],
];

/// The single documented f64 -> f32 narrowing for fuzz geometry.
fn narrow(p: [f64; 3]) -> [f32; 3] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "deliberate narrowing of generated fuzz positions"
    )]
    {
        [p[0] as f32, p[1] as f32, p[2] as f32]
    }
}

/// A random convex prism: a cross-section template scaled, rotated about
/// z, and translated. Watertight and outward-oriented by construction.
/// A quarter of the corpus snaps to a coarse grid so operand pairs hit
/// touching/coplanar configurations (which must fail typed, not panic).
fn random_solid(rng: &mut Rng) -> Mesh {
    let section = SECTIONS[rng.range(SECTIONS.len())];
    let [c, s] = ROTATIONS[rng.range(ROTATIONS.len())];
    let scale_x = rng.unit(0.4, 1.6);
    let scale_y = rng.unit(0.4, 1.6);
    let height = rng.unit(0.5, 2.5);
    let center = [
        rng.unit(-1.0, 1.0),
        rng.unit(-1.0, 1.0),
        rng.unit(-1.0, 1.0),
    ];
    let snap = rng.range(4) == 0;
    let grid = |v: f64| if snap { (v * 4.0).round() / 4.0 } else { v };

    let n = section.len();
    let count = u32::try_from(n).expect("small template");
    let mut builder = MeshBuilder::new();
    for (z_sign, z_offset) in [(-0.5, 0.0), (0.5, 0.0)] {
        let z = center[2] + z_sign * height + z_offset;
        for p in section {
            let (x, y) = (p[0] * scale_x, p[1] * scale_y);
            let position = [
                grid(center[0] + c * x - s * y),
                grid(center[1] + s * x + c * y),
                grid(z),
            ];
            builder.push_vertex(narrow(position));
        }
    }
    let bottom: Vec<u32> = (0..count).rev().collect();
    builder.add_face(&bottom).expect("bottom cap");
    let top: Vec<u32> = (count..2 * count).collect();
    builder.add_face(&top).expect("top cap");
    for i in 0..count {
        let j = (i + 1) % count;
        builder
            .add_face(&[i, j, count + j, count + i])
            .expect("side wall");
    }
    builder.build().expect("valid fuzz solid").mesh
}

/// Signed volume via the divergence theorem over fan triangles.
fn signed_volume(mesh: &Mesh) -> f64 {
    let mut volume = 0.0;
    for face in mesh.faces() {
        let corners: Vec<[f64; 3]> = mesh
            .face_loop(face)
            .filter_map(|he| mesh.to_vertex(he))
            .filter_map(|v| mesh.vertex_position(v))
            .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
            .collect();
        for i in 1..corners.len().saturating_sub(1) {
            let (a, b, c) = (corners[0], corners[i], corners[i + 1]);
            volume += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
        }
    }
    volume / 6.0
}

/// Structural snapshot for bit-identity comparison.
type Snapshot = (Vec<Vec<u32>>, Vec<[u32; 3]>);

fn snapshot(mesh: &Mesh) -> Snapshot {
    let faces: Vec<Vec<u32>> = mesh
        .faces()
        .map(|face| {
            mesh.face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .map(|v| v.index())
                .collect()
        })
        .collect();
    let positions: Vec<[u32; 3]> = mesh
        .vertices()
        .filter_map(|v| mesh.vertex_position(v))
        .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
        .collect();
    (faces, positions)
}

fn run_once(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    op: BooleanOp,
) -> (Result<BooleanOutput, BooleanError>, BooleanDiagnostics) {
    let mut scratch = BooleanScratch::new();
    let mut diagnostics = BooleanDiagnostics::default();
    let result = boolean_mesh(
        mesh_a,
        mesh_b,
        op,
        FaceTriangulation::Fan,
        &mut scratch,
        &mut diagnostics,
    );
    (result, diagnostics)
}

#[test]
fn boolean_pipeline_fuzz_never_panics_and_honors_its_contract() {
    let mut rng = Rng(0x00E1_D8A5_EED6_A501);
    const PAIRS: usize = 48;
    const TOLERANCE: f64 = 1e-4;

    let mut ok_results = 0_u64;
    let mut typed_failures = 0_u64;
    for pair in 0..PAIRS {
        let mesh_a = random_solid(&mut rng);
        let mesh_b = random_solid(&mut rng);
        let volume_a = signed_volume(&mesh_a);
        let volume_b = signed_volume(&mesh_b);
        assert!(volume_a > 0.0, "generator produces positive solids");
        assert!(volume_b > 0.0, "generator produces positive solids");

        for op in [
            BooleanOp::Union,
            BooleanOp::Intersection,
            BooleanOp::Difference,
        ] {
            let (first, first_diagnostics) = run_once(&mesh_a, &mesh_b, op);
            let (second, _) = run_once(&mesh_a, &mesh_b, op);

            match (&first, &second) {
                (Ok(a), Ok(b)) => {
                    ok_results += 1;
                    let errors = a.mesh.validate_deep();
                    assert!(errors.is_empty(), "pair {pair} {op:?}: {errors:?}");
                    let volume = signed_volume(&a.mesh);
                    assert!(
                        volume >= -TOLERANCE,
                        "pair {pair} {op:?}: negative volume {volume}"
                    );
                    assert!(
                        volume <= volume_a + volume_b + TOLERANCE,
                        "pair {pair} {op:?}: volume {volume} exceeds operand sum"
                    );
                    match op {
                        BooleanOp::Intersection => assert!(
                            volume <= volume_a.min(volume_b) + TOLERANCE,
                            "pair {pair}: intersection volume {volume} exceeds an operand"
                        ),
                        BooleanOp::Difference => assert!(
                            volume <= volume_a + TOLERANCE,
                            "pair {pair}: difference volume {volume} exceeds A"
                        ),
                        BooleanOp::Union => assert!(
                            volume >= volume_a.max(volume_b) - TOLERANCE,
                            "pair {pair}: union volume {volume} below an operand"
                        ),
                    }
                    // Determinism: bit-identical rerun.
                    assert_eq!(a.stats, b.stats, "pair {pair} {op:?}");
                    assert_eq!(a.face_provenance, b.face_provenance, "pair {pair} {op:?}");
                    assert_eq!(snapshot(&a.mesh), snapshot(&b.mesh), "pair {pair} {op:?}");
                }
                (Err(error), Err(second_error)) => {
                    typed_failures += 1;
                    assert_eq!(error, second_error, "pair {pair} {op:?}: rerun diverged");
                    match error {
                        BooleanError::SuspectPatches { count } => {
                            assert!(*count > 0, "pair {pair} {op:?}");
                            assert!(
                                !first_diagnostics.is_clean(),
                                "pair {pair} {op:?}: suspect failure left no diagnostics"
                            );
                        }
                        BooleanError::Build(e) => {
                            panic!("pair {pair} {op:?}: internal build failure {e:?}")
                        }
                        BooleanError::InvariantViolation { count } => {
                            panic!("pair {pair} {op:?}: {count} invariant violations")
                        }
                    }
                }
                _ => panic!("pair {pair} {op:?}: Ok/Err diverged between reruns"),
            }
        }
    }
    // The corpus must produce real results to mean anything. Typed
    // failures are allowed but no longer guaranteed: coplanar-contact
    // support and collinear-vertex reinsertion cleared every deferral this
    // corpus used to hit (the typed-failure contract stays covered by the
    // dedicated deferral tests in `split`/`classify`/`stitch`).
    assert!(ok_results > 0, "corpus produced no successful booleans");
    let _ = typed_failures;
}

/// One seeded run of random `split_edge`/`split_face` ops over a random
/// solid, deep-validating after every op. Bad picks (adjacent corners,
/// identical corners) must fail typed, never panic or corrupt.
fn run_split_ops(seed: u64) -> (Snapshot, u64, u64) {
    use crate::op::{split_edge, split_face};
    use crate::{CornerId, HalfEdgeId, PropagatePolicy};

    let mut rng = Rng(seed);
    let mut mesh = random_solid(&mut rng);
    let policy = PropagatePolicy::default();
    let mut applied = 0_u64;
    let mut rejected = 0_u64;
    for step in 0..24 {
        let corners: Vec<HalfEdgeId> = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face).collect::<Vec<_>>())
            .collect();
        if corners.is_empty() {
            break;
        }
        let succeeded = {
            let mut session = mesh.edit();
            let succeeded = if rng.range(2) == 0 {
                let half_edge = corners[rng.range(corners.len())];
                split_edge(&mut session, half_edge, &policy).is_ok()
            } else {
                let corner_a: CornerId = corners[rng.range(corners.len())];
                let corner_b: CornerId = corners[rng.range(corners.len())];
                split_face(&mut session, corner_a, corner_b, &policy).is_ok()
            };
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
            succeeded
        };
        if succeeded {
            applied += 1;
        } else {
            rejected += 1;
        }
        let errors = mesh.validate_deep();
        assert!(errors.is_empty(), "seed {seed:#x} step {step}: {errors:?}");
    }
    (snapshot(&mesh), applied, rejected)
}

#[test]
fn split_op_fuzz_keeps_meshes_deeply_valid() {
    let mut total_applied = 0_u64;
    let mut total_rejected = 0_u64;
    for round in 0..16_u64 {
        let seed = 0x5EED_0000_0000_0000 ^ (round.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (first, applied, rejected) = run_split_ops(seed);
        let (second, _, _) = run_split_ops(seed);
        assert_eq!(first, second, "seed {seed:#x}: rerun diverged");
        total_applied += applied;
        total_rejected += rejected;
    }
    assert!(total_applied > 0, "corpus applied no split ops");
    assert!(
        total_rejected > 0,
        "corpus never exercised typed rejections"
    );
}
