// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Splits of bodies placed far from the origin.

use exedra_math::{Placement3, Plane3, narrow};
use exedra_mesh::{Mesh, MeshBuilder};

use super::{SectionPolicy, split_mesh};

/// A size-2 box with a collinear vertex on its top/front edge, placed by
/// `placement`. Positions are computed in f64 and stored as f32.
fn placed_box(placement: &Placement3) -> Mesh {
    placed_box_with(placement, 0.5, 0.0)
}

/// [`placed_box`] with the top/front sample at `x = 2 t`, raised off the edge
/// by `bend` (zero keeps it collinear).
fn placed_box_with(placement: &Placement3, t: f64, bend: f64) -> Mesh {
    let place = |p: [f64; 3]| {
        let r = placement.rows;
        narrow([0, 1, 2].map(|i| r[i][0] * p[0] + r[i][1] * p[1] + r[i][2] * p[2] + r[i][3]))
    };
    let mut builder = MeshBuilder::new();
    for p in [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [0.0, 2.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 0.0, 2.0],
        [2.0, 0.0, 2.0],
        [0.0, 2.0, 2.0],
        [2.0, 2.0, 2.0],
        [2.0 * t, 0.0, 2.0 + bend],
    ] {
        builder.push_vertex(place(p));
    }
    for face in [
        &[0, 2, 3, 1][..],
        &[4, 8, 5, 7, 6],
        &[0, 1, 5, 8, 4],
        &[2, 6, 7, 3],
        &[0, 4, 6, 2],
        &[1, 3, 7, 5],
    ] {
        builder.add_face(face).expect("box face");
    }
    builder.build().expect("box").mesh
}

/// The plane `local z = height` of `placement`, in body coordinates.
fn placed_plane(placement: &Placement3, height: f64) -> Plane3 {
    let r = placement.rows;
    let normal = [r[0][2], r[1][2], r[2][2]];
    let origin = [
        r[0][2] * height + r[0][3],
        r[1][2] * height + r[1][3],
        r[2][2] * height + r[2][3],
    ];
    Plane3 {
        normal,
        distance: normal[0] * origin[0] + normal[1] * origin[1] + normal[2] * origin[2],
    }
}

/// A plane tolerance f32 storage can meet for coordinates up to `offset`
/// (plus the box's own extent).
fn storage_tolerance(offset: f64) -> f64 {
    (8.0 * f64::from(f32::EPSILON) * (offset + 4.0)).max(1e-6)
}

#[test]
fn rotated_boxes_split_far_from_the_origin() {
    for offset in [0.0, 10.0, 100.0, 300.0, 500.0, 1000.0] {
        let placement = Placement3::euler_extrinsic_xyz_then_translate(
            0.3,
            -0.7,
            1.1,
            [offset, -offset, offset],
        );
        let mesh = placed_box(&placement);
        let plane = placed_plane(&placement, 1.0);
        // Stored f32 cut vertices sit up to half an ulp off the plane, so the
        // absolute contact tolerance must cover the coordinate magnitude.
        let policy = SectionPolicy {
            distance_tolerance: storage_tolerance(offset),
            ..SectionPolicy::default()
        };
        let split = split_mesh(&mesh, plane, &policy, 99)
            .unwrap_or_else(|error| panic!("offset {offset}: {error:?}"));
        for half in [split.negative, split.positive] {
            let half = half.expect("both halves");
            let errors = half.mesh.validate_deep();
            assert!(errors.is_empty(), "offset {offset}: {errors:?}");
        }
    }
}

/// `SplitMix64`, for a seeded corpus.
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
        #[expect(
            clippy::cast_precision_loss,
            reason = "20 random bits convert to f64 exactly"
        )]
        let t = (self.next() >> 44) as f64 / f64::from(1_u32 << 20);
        lo + t * (hi - lo)
    }
}

/// A placed box whose top/front sample is the exact midpoint of its stored
/// edge endpoints in the top face's dominant projection, nudged `ulps` f32
/// steps along the dominant normal axis: collinear where the robust
/// triangulation looks, off the line only by storage rounding in 3-D. When
/// the projected midpoint is not exactly representable, the sample falls back
/// to the placed collinear point; the flag reports which one was built.
fn placed_box_nudged(placement: &Placement3, ulps: i32) -> (Mesh, bool) {
    let r = placement.rows;
    let place = |p: [f64; 3]| {
        narrow([0, 1, 2].map(|i| r[i][0] * p[0] + r[i][1] * p[1] + r[i][2] * p[2] + r[i][3]))
    };
    let corners = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [0.0, 2.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 0.0, 2.0],
        [2.0, 0.0, 2.0],
        [0.0, 2.0, 2.0],
        [2.0, 2.0, 2.0],
    ]
    .map(place);
    // The top face's normal is the placement's local z; its largest
    // component picks the axis the triangulation projects along.
    let normal = [r[0][2], r[1][2], r[2][2]];
    let axis = (0..3)
        .max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs()))
        .expect("three axes");
    let (a, b) = (corners[4], corners[5]);
    let mut sample = narrow([0, 1, 2].map(|k| (f64::from(a[k]) + f64::from(b[k])) / 2.0));
    let exact_midpoint = (0..3)
        .all(|k| k == axis || f64::from(sample[k]) == (f64::from(a[k]) + f64::from(b[k])) / 2.0);
    let mut bits = sample[axis].to_bits();
    for _ in 0..ulps.unsigned_abs() {
        let away = (ulps > 0) == (sample[axis] >= 0.0);
        bits = if away { bits + 1 } else { bits - 1 };
    }
    sample[axis] = f32::from_bits(bits);
    let mut builder = MeshBuilder::new();
    for p in corners {
        builder.push_vertex(p);
    }
    builder.push_vertex(if exact_midpoint {
        sample
    } else {
        place([1.0, 0.0, 2.0])
    });
    for face in [
        &[0, 2, 3, 1][..],
        &[4, 8, 5, 7, 6],
        &[0, 1, 5, 8, 4],
        &[2, 6, 7, 3],
        &[0, 4, 6, 2],
        &[1, 3, 7, 5],
    ] {
        builder.add_face(face).expect("box face");
    }
    (builder.build().expect("box").mesh, exact_midpoint)
}

/// Random rotations and offsets up to 2000, with the top/front sample exactly
/// collinear in the triangulation's projection but 1–2 ulps off the edge in
/// 3-D: the storage-rounding class that used to refuse as `Triangulation`.
/// Every split succeeds once the contact tolerance covers f32 storage.
#[test]
fn storage_rounded_collinear_samples_split_far_from_the_origin() {
    let mut rng = Rng(0xFA12_C0DE_5EED);
    let mut nudged = 0_u32;
    for trial in 0..1000 {
        let offset = rng.unit(0.0, 2000.0);
        let placement = Placement3::euler_extrinsic_xyz_then_translate(
            rng.unit(-3.0, 3.0),
            rng.unit(-3.0, 3.0),
            rng.unit(-3.0, 3.0),
            [offset, -0.5 * offset, 0.75 * offset],
        );
        let ulps = [-2, -1, 1, 2][usize::try_from(rng.next() % 4).expect("small")];
        let (mesh, exact) = placed_box_nudged(&placement, ulps);
        nudged += u32::from(exact);
        let plane = placed_plane(&placement, rng.unit(0.2, 1.8));
        let policy = SectionPolicy {
            distance_tolerance: storage_tolerance(offset),
            ..SectionPolicy::default()
        };
        let split = split_mesh(&mesh, plane, &policy, 99)
            .unwrap_or_else(|error| panic!("trial {trial} offset {offset}: {error:?}"));
        for half in [split.negative, split.positive] {
            let errors = half.expect("both halves").mesh.validate_deep();
            assert!(errors.is_empty(), "trial {trial}: {errors:?}");
        }
    }
    // Most trials must land in the rounding class, or the corpus tests little.
    assert!(
        nudged >= 200,
        "only {nudged} of 1000 trials built a nudged sample"
    );
}

/// A sample genuinely off the edge is not storage rounding: far from the
/// origin the nonplanar face is still refused rather than re-surfaced. The
/// box stays axis-aligned at an exactly representable offset so the bent
/// sample is exactly collinear in the triangulation's projection, which is
/// the case reinsertion has to judge.
#[test]
fn bent_samples_are_still_refused_far_from_the_origin() {
    let placement =
        Placement3::euler_extrinsic_xyz_then_translate(0.0, 0.0, 0.0, [1024.0, -1024.0, 1024.0]);
    let policy = SectionPolicy {
        distance_tolerance: storage_tolerance(1024.0),
        ..SectionPolicy::default()
    };
    // Coordinates near 1026 have an f32 ulp of 2^-13, so the reinsertion band
    // 4·ε·M is about four ulps. Bends of 8 and 16 ulps sit just outside it and
    // pin that the band is no looser than documented; 0.0625 is far outside.
    for bend in [0.000_976_562_5, 0.001_953_125, 0.0625] {
        let mesh = placed_box_with(&placement, 0.5, bend);
        assert_eq!(
            split_mesh(&mesh, placed_plane(&placement, 1.0), &policy, 99).err(),
            Some(super::SectionError::Triangulation),
            "bend {bend}"
        );
    }
}
