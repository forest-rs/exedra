// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `_into` functions allocate nothing once their buffers and
//! workspace have grown, whatever the curve's order.

#![expect(
    unsafe_code,
    reason = "counting allocations needs a global allocator, which forwards to the system's"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use exedra_curves::{
    CubicBasis, Frame, Nurbs, NurbsSamples, PreparedNurbs, SweptMesh, Tolerance, Workspace,
    adaptive_stations_into, rotation_minimizing_frames_into, tube_into,
};

/// Counts this thread's allocations, so tests on other threads do not
/// disturb the count.
struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: the caller's contract for `alloc` is passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract for `dealloc` is passed on.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: the caller's contract for `realloc` is passed on.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// How many allocations `f` makes on this thread.
fn allocations(f: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.with(Cell::get);
    f();
    ALLOCATIONS.with(Cell::get) - before
}

/// A clamped curve of `order` through a wavy row of controls.
fn curve(order: usize, controls: usize) -> PreparedNurbs {
    let points: Vec<[f64; 3]> = (0..controls)
        .map(|i| {
            let x = i as f64;
            [x, (x * 0.9).sin(), (x * 0.4).cos()]
        })
        .collect();
    let weights: Vec<f64> = (0..controls).map(|i| 1.0 + 0.25 * (i % 3) as f64).collect();
    let interior = controls - order;
    let knots: Vec<f64> = std::iter::repeat_n(0.0, order)
        .chain((1..=interior).map(|k| k as f64))
        .chain(std::iter::repeat_n(interior as f64 + 1.0, order))
        .collect();
    Nurbs {
        order: u32::try_from(order).unwrap(),
        knots: &knots,
        points: &points,
        weights: Some(&weights),
    }
    .validate()
    .unwrap()
    .prepare()
}

#[test]
fn warmed_calls_allocate_nothing() {
    let tolerance = Tolerance {
        chord: 1e-3,
        angle: 0.2,
    };
    let mut workspace = Workspace::new();
    let mut samples = NurbsSamples::default();
    let mut frames: Vec<Frame> = Vec::new();
    let mut mesh = SweptMesh::default();
    let mut stations = Vec::new();
    let mut lengths = Vec::new();
    let controls: Vec<[f64; 3]> = (0..8).map(|i| [i as f64, (i % 2) as f64, 0.0]).collect();
    let spans: Vec<[u32; 4]> = (0..5).map(|s| [s, s + 1, s + 2, s + 3]).collect();
    for order in [4, 9, 17] {
        let prepared = curve(order, order + 6);
        let mut run = || {
            prepared.adaptive_sample_into(tolerance, 4000, &mut workspace, &mut samples);
            prepared.lengths_into(4, &mut workspace, &mut lengths);
            rotation_minimizing_frames_into(
                &samples.points,
                None,
                false,
                None,
                &mut workspace,
                &mut frames,
            )
            .unwrap();
            let radii = vec_of(frames.len());
            tube_into(&frames, &radii, 6, false, &mut workspace, &mut mesh).unwrap();
            adaptive_stations_into(
                CubicBasis::CatmullRom,
                &controls,
                &spans,
                false,
                tolerance,
                4000,
                &mut workspace,
                &mut stations,
            )
            .unwrap();
        };
        run();
        run();
        let counted = allocations(|| {
            prepared.adaptive_sample_into(tolerance, 4000, &mut workspace, &mut samples);
        });
        assert_eq!(counted, 0, "adaptive sampling at order {order}");
        let counted = allocations(|| {
            prepared.lengths_into(4, &mut workspace, &mut lengths);
        });
        assert_eq!(counted, 0, "lengths at order {order}");
        let counted = allocations(|| {
            rotation_minimizing_frames_into(
                &samples.points,
                None,
                false,
                None,
                &mut workspace,
                &mut frames,
            )
            .unwrap();
        });
        assert_eq!(counted, 0, "frames at order {order}");
        let counted = allocations(|| {
            adaptive_stations_into(
                CubicBasis::CatmullRom,
                &controls,
                &spans,
                false,
                tolerance,
                4000,
                &mut workspace,
                &mut stations,
            )
            .unwrap();
        });
        assert_eq!(counted, 0, "adaptive stations");
    }
}

/// Radii for `n` stations; allocated outside the counted calls.
fn vec_of(n: usize) -> Vec<f64> {
    vec![0.1; n]
}
