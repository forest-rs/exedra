// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact selected native edge finishing with existing pocket revalidation.

use exedra_analytic::extrusion::{End, Extrusion, Pocket};
use exedra_constructive::{builders, ir::Placement3};
use exedra_curves::profile::{CornerTreatment, Loop, ProfilePolicy, SegTag, Segment};
use std::{hash::Hasher, hint::black_box, time::Instant};
const POLICY: ProfilePolicy = ProfilePolicy::new(1e-7);
fn main() {
    let mut cases = Vec::new();
    for count in [0, 16, 64] {
        let mut solid = Extrusion::from_profile(
            &builders::rect_centered(200.0, 200.0)
                .unwrap()
                .tagged(SegTag(1)),
            Placement3::IDENTITY,
            10.0,
            &POLICY,
        )
        .unwrap();
        for i in 0..count {
            let x = -70.0 + 20.0 * f64::from(i % 8);
            let y = -70.0 + 20.0 * f64::from(i / 8);
            let disk = Loop::new(vec![
                Segment::arc([x - 2.0, y], 1.0),
                Segment::arc([x + 2.0, y], 1.0),
            ])
            .unwrap();
            solid = solid
                .with_pocket(Pocket::from_region(&disk, End::End, 2.0), &POLICY)
                .unwrap();
        }
        let selected: Vec<_> = solid
            .profile_axial_edges_tagged(SegTag(1))
            .map(|(id, _)| id)
            .collect();
        let action = || {
            solid
                .with_axial_corners(
                    selected.iter().copied(),
                    CornerTreatment::fillet(5.0).tagged(SegTag(2)),
                    &POLICY,
                )
                .unwrap()
        };
        let expected = action();
        assert_eq!(
            expected,
            action(),
            "repeatable exact finishing before timing"
        );
        assert_eq!(
            expected.transitions.len(),
            4,
            "four selected native transitions"
        );
        assert!(expected.solid.is_closed(), "closed native boundary");
        let mut signature = std::collections::hash_map::DefaultHasher::new();
        signature.write(format!("{expected:?}").as_bytes());
        for _ in 0..3 {
            black_box(action());
        }
        let samples: Vec<_> = (0..31)
            .map(|_| {
                let start = Instant::now();
                let result = action();
                black_box(&result);
                let elapsed = start.elapsed().as_nanos();
                drop(result);
                elapsed
            })
            .collect();
        cases.push(serde_json::json!({"pockets":count,"native_faces":expected.solid.faces().len(),"signature":format!("{:016x}",signature.finish()),"samples_ns":samples}));
    }
    println!("{}", serde_json::json!({"cases":cases}));
}
