// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `_into` functions allocate nothing once their buffers and
//! workspace have grown, whatever the curve's order, and borrowing and
//! validating a curve allocates nothing at all, in the plane or in space.

#![expect(
    unsafe_code,
    reason = "counting allocations needs a global allocator, which forwards to the system's"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use exedra_curves::realize::{
    Attribute, Centerline, CurveDescription, GpuCurve, PlanOptions, RealizationPlan,
};
use exedra_curves::{
    CubicBasis, Frame, Nurbs, NurbsCurve, NurbsSamples, PreparedNurbs, SweptMesh, Tolerance,
    Workspace, adaptive_stations_into, rotation_minimizing_frames_into, tube_into,
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

#[test]
fn prepared_native_source_enclosures_reuse_all_query_storage() {
    use exedra_curves::enclosure::{
        EnclosureScratch, PreparationLimits, PreparedNurbsEnclosure, SourceRange,
    };
    for order in [4, 9, 17] {
        let points: Vec<_> = (0..order)
            .map(|i| [f64::from(i), f64::from(i % 3), 0.])
            .collect();
        let knots = [vec![2.; order as usize], vec![5.; order as usize]].concat();
        let curve = NurbsCurve::new(order, knots, points, None).unwrap();
        let prepared = PreparedNurbsEnclosure::prepare(
            curve.view(),
            SourceRange {
                parameters: [2., 5.],
                reversed: true,
            },
            PreparationLimits {
                max_controls: 100,
                max_work: 100_000,
            },
        )
        .unwrap();
        let mut scratch = EnclosureScratch::new(order as usize).unwrap();
        let count = allocations(|| {
            for range in [[0., 1.], [0.2, 0.8], [0.5, 0.5], [1., 1.]] {
                std::hint::black_box(prepared.enclose(range, &mut scratch, || true).unwrap());
                std::hint::black_box(
                    prepared
                        .project(range, [1., 2., 3.], [0., 1., 0.], &mut scratch, || true)
                        .unwrap(),
                );
            }
        });
        assert_eq!(count, 0, "source enclosure order {order}");
    }
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
    let controls: Vec<[f64; 3]> = (0..8).map(|i| [i as f64, (i % 2) as f64, 0.0]).collect();
    let spans: Vec<[u32; 4]> = (0..5).map(|s| [s, s + 1, s + 2, s + 3]).collect();
    for order in [4, 9, 17] {
        let prepared = curve(order, order + 6);
        let mut run = || {
            prepared.adaptive_sample_into(tolerance, 4000, &mut workspace, &mut samples);
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

/// Borrows and validates a curve of `order` in `N` dimensions, evaluates
/// it, which keeps its scratch on the stack up to order 8, then runs every
/// `_into` on it twice, counting the second run.
fn borrowed<const N: usize>(order: usize) {
    let controls = order + 5;
    let points: Vec<[f64; N]> = (0..controls)
        .map(|i| core::array::from_fn(|axis| ((i * (axis + 2)) as f64 * 0.7).sin()))
        .collect();
    let weights: Vec<f64> = (0..controls).map(|i| 1.0 + 0.25 * (i % 3) as f64).collect();
    let interior = controls - order;
    let knots: Vec<f64> = std::iter::repeat_n(0.0, order)
        .chain((1..=interior).map(|k| k as f64))
        .chain(std::iter::repeat_n(interior as f64 + 1.0, order))
        .collect();
    let order = u32::try_from(order).unwrap();
    let counted = allocations(|| {
        let valid = Nurbs {
            order,
            knots: &knots,
            points: &points,
            weights: Some(&weights),
        }
        .validate()
        .unwrap();
        if order <= 8 {
            core::hint::black_box(valid.evaluation(0.5));
        }
    });
    assert_eq!(counted, 0, "borrowing at order {order} in {N}D");
    let valid = Nurbs {
        order,
        knots: &knots,
        points: &points,
        weights: Some(&weights),
    }
    .validate()
    .unwrap();
    let tolerance = Tolerance {
        chord: 1e-3,
        angle: 0.2,
    };
    let mut workspace = Workspace::new();
    let mut prepared = valid.prepare();
    let mut samples = NurbsSamples::default();
    let mut lengths = Vec::new();
    let mut intervals = Vec::new();
    let mut run = || {
        valid.prepare_into(&mut workspace, &mut prepared);
        valid
            .sample_into(None, 8, &mut workspace, &mut samples)
            .unwrap();
        prepared.adaptive_sample_into(tolerance, 4000, &mut workspace, &mut samples);
        prepared.adaptive_intervals_into(tolerance, 4000, &mut workspace, &mut intervals);
        prepared.lengths_into(4, &mut workspace, &mut lengths);
        if order <= 8 {
            core::hint::black_box(prepared.evaluation(0.5));
        }
    };
    run();
    assert_eq!(allocations(run), 0, "warmed calls at order {order} in {N}D");
}

#[test]
fn borrowed_curves_allocate_nothing_in_the_plane_or_in_space() {
    for order in [2, 4, 9, 17] {
        borrowed::<2>(order);
        borrowed::<3>(order);
    }
}

/// A realization plan's updates and realizations, once warmed, allocate
/// nothing: moving controls, sweeping and writing GPU data, for a cubic
/// chain and for NURBS curves of low and high order.
#[test]
fn warmed_realization_allocates_nothing() {
    let wavy = |n: usize, lift: f64| -> Vec<[f64; 3]> {
        (0..n)
            .map(|i| [i as f64, ((i * 7) % 5) as f64 * 0.3 + lift, (i % 3) as f64])
            .collect()
    };
    let mut descriptions = vec![CurveDescription {
        centerline: Centerline::Cubic {
            basis: CubicBasis::CatmullRom,
            points: wavy(12, 0.0),
            spans: (0..9).map(|i| [i, i + 1, i + 2, i + 3]).collect(),
        },
        widths: Attribute::Vertex(vec_of(12)),
        normals: Some(Attribute::Constant([0.0, 0.0, 1.0])),
        closed: false,
    }];
    for order in [4_u32, 17] {
        let n = 2 * order as usize;
        let interior = n - order as usize;
        let mut knots = vec![0.0; order as usize];
        knots.extend((1..=interior).map(|k| k as f64));
        knots.extend(vec![(interior + 1) as f64; order as usize]);
        let curve = NurbsCurve::new(order, knots, wavy(n, 0.0), Some(vec![1.5; n])).unwrap();
        descriptions.push(CurveDescription {
            centerline: Centerline::Nurbs(curve),
            widths: Attribute::Constant(0.1),
            normals: None,
            closed: false,
        });
    }
    for description in descriptions {
        let n = description.centerline.controls();
        let mut workspace = Workspace::new();
        let mut plan =
            RealizationPlan::new(description, PlanOptions::new(6), &mut workspace).unwrap();
        let (a, b) = (wavy(n, 0.5), wavy(n, 0.0));
        let mut mesh = SweptMesh::default();
        let mut gpu = GpuCurve::default();
        let mut run = |points: &[[f64; 3]]| {
            plan.set_points(points, &mut workspace).unwrap();
            plan.tube_into(8, &mut workspace, &mut mesh).unwrap();
            plan.ribbon_into(&mut workspace, &mut mesh).unwrap();
            core::hint::black_box(plan.gpu_into([0.0; 3], &mut gpu).unwrap());
        };
        run(&a);
        run(&b);
        assert_eq!(
            allocations(|| run(&a)),
            0,
            "warmed realization of {n} controls"
        );
    }
}

/// Radii for `n` stations; allocated outside the counted calls.
fn vec_of(n: usize) -> Vec<f64> {
    vec![0.1; n]
}
