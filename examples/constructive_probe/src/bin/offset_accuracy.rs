// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Interactive comparison of actual offset profiles and their work charges.
//!
//! Run `cargo run -p constructive_probe --bin offset_accuracy` to write the
//! in-conversation visual fragment to `target/offset-accuracy.html`.

use std::fmt::Write as _;
use std::path::Path;

use exedra_constructive::offset::{CornerPolicy, OffsetPolicy};
use exedra_constructive::profile::{Loop2, Profile2, Seg2, SegKind};

fn rectangle(x: f64, y: f64, width: f64, height: f64) -> Loop2 {
    Loop2::new(vec![
        Seg2::line((x + width, y)),
        Seg2::line((x + width, y + height)),
        Seg2::line((x, y + height)),
        Seg2::line((x, y)),
    ])
    .expect("rectangle")
}

fn quoted(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => write!(out, "\\u{:04x}", u32::from(c)).expect("string"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn geometry(out: &mut String, profile: &Profile2) {
    let mut path = String::new();
    let mut points = Vec::new();
    for ring in std::iter::once(profile.outer()).chain(profile.holes()) {
        let start = ring.segs().last().expect("closed ring").to;
        write!(path, "M{:.9},{:.9}", start.x, start.y).expect("string");
        for (from, seg) in ring.iter_with_starts() {
            let to = seg.to;
            match &seg.kind {
                SegKind::Line => write!(path, "L{:.9},{:.9}", to.x, to.y),
                SegKind::Cubic { c1, c2 } => write!(
                    path,
                    "C{:.9},{:.9} {:.9},{:.9} {:.9},{:.9}",
                    c1.x, c1.y, c2.x, c2.y, to.x, to.y
                ),
                SegKind::Arc { bulge } => {
                    let radius = (to.x - from.x).hypot(to.y - from.y)
                        * (bulge.abs() + 1.0 / bulge.abs())
                        * 0.25;
                    write!(
                        path,
                        "A{radius:.9},{radius:.9} 0 {},{} {:.9},{:.9}",
                        u8::from(bulge.abs() > 1.0),
                        u8::from(*bulge > 0.0),
                        to.x,
                        to.y
                    )
                }
                _ => unreachable!("example profiles use concrete segments"),
            }
            .expect("string");
            points.push([to.x, to.y]);
        }
        path.push('Z');
    }
    out.push_str("{\"path\":");
    quoted(out, &path);
    write!(out, ",\"points\":{points:?}}}").expect("string");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frame = Profile2::new(
        rectangle(0.0, 0.0, 40.0, 24.0),
        vec![
            rectangle(5.0, 5.0, 8.0, 14.0).reversed(),
            rectangle(25.0, 5.0, 10.0, 14.0).reversed(),
        ],
    )?;
    let cubic = Profile2::simple(Loop2::new(vec![
        Seg2::line((10.0, 0.0)),
        Seg2::cubic((10.0, 10.0), (14.0, 3.0), (14.0, 7.0)),
        Seg2::line((0.0, 10.0)),
        Seg2::line((0.0, 0.0)),
    ])?)?;
    let mut data = String::from("[");
    for (i, (name, source)) in [
        ("Frame with two openings", frame),
        ("Cubic-sided section", cubic),
    ]
    .iter()
    .enumerate()
    {
        if i != 0 {
            data.push(',');
        }
        data.push_str("{\"name\":");
        quoted(&mut data, name);
        data.push_str(",\"source\":");
        geometry(&mut data, source);
        data.push_str(",\"states\":[");
        let mut first = true;
        for fit_tolerance in [0.01, 0.001, 1e-8] {
            for step in -15..=20 {
                if !first {
                    data.push(',');
                }
                first = false;
                let distance = f64::from(step) / 5.0;
                write!(data, "{{\"distance\":{distance},\"fit\":{fit_tolerance},").expect("string");
                let policy = OffsetPolicy {
                    fit_tolerance,
                    ..Default::default()
                };
                match source.offset_with_policy(distance, CornerPolicy::Round, &policy) {
                    Ok(result) => {
                        data.push_str("\"geometry\":");
                        geometry(&mut data, &result.profile);
                        write!(
                            data,
                            ",\"segments\":{},\"fits\":{},\"edges\":{},\"pairs\":{}",
                            result.work.result_segments,
                            result.work.cubic_fits,
                            result.work.check_edges,
                            result.work.check_pairs
                        )
                        .expect("string");
                    }
                    Err(error) => {
                        data.push_str("\"error\":");
                        quoted(&mut data, &error.to_string());
                    }
                }
                data.push('}');
            }
        }
        data.push_str("]}");
    }
    data.push(']');
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/offset-accuracy.html".into());
    if let Some(parent) = Path::new(&output).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &output,
        include_str!("offset_accuracy.html").replace("OFFSET_DATA", &data),
    )?;
    println!("Wrote {output}");
    Ok(())
}
