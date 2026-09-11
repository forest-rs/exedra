// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Scene-authored, pinned side laps over flat purlin bearings.
//!
//! Each short rafter has two real bearings. Half-width tongues meet at each
//! pitch change, including the ridge. This is an illustrative fit, not a
//! historical prescription or a structural capacity calculation.

use exedra_constructive::builders::circle;
use exedra_constructive::ir::Placement3;
use joiner::{
    Anchor, ContactMeaning, ContactPatch, Element, Evidence, Node, OrientedBox, Part, Relation,
    RelationKind, RuleOutput, TransferEdge, TransferKind, TransferTarget,
};

use crate::joinery::{self, FittedConstruction};
use crate::layout::{PURLIN_RADIUS, TOP_SEAT_DEPTH, interval_count};
use crate::roof_section::{
    BEARING_ABOVE_BASE, EAVE_EXTENSION, RAFTER_DEPTH, RoofSection, height, normal,
};
use crate::{Result, geometry, layout::Layout, seats};

pub(crate) const WIDTH: f64 = 0.085;
const HALF_LAP: f64 = 0.05;
pub(crate) const PIN_RADIUS: f64 = 0.006;
const BORE_RADIUS: f64 = PIN_RADIUS + 0.0005;
const PIN_LIFT: f64 = 0.052;

#[cfg(test)]
mod tests;

pub(crate) fn build(layout: &Layout) -> Result<FittedConstruction> {
    let width = layout.width + 1.2;
    let mut frame = seats::build(layout)?;
    add(layout, &mut frame, &stations(width))?;
    Ok(frame)
}

pub(crate) fn stations(width: f64) -> Vec<f64> {
    let intervals = interval_count(width - 0.20, 0.44);
    (0..=intervals)
        .map(|i| (f64::from(i) / f64::from(intervals) - 0.5) * (width - 0.20))
        .collect()
}

pub(crate) fn study(layout: &Layout) -> Result<FittedConstruction> {
    let mut frame = seats::study(layout)?;
    add(layout, &mut frame, &[0.0])?;
    Ok(frame)
}

fn add(layout: &Layout, frame: &mut FittedConstruction, stations: &[f64]) -> Result<()> {
    flatten_purlin_tops(frame)?;
    let section = RoofSection::new(layout);
    let points = section.rafter_points();
    let evidence = frame.construction.elements()[0].evidence.clone();
    for (lane, &x) in stations.iter().enumerate() {
        for segment in 0..RoofSection::SEGMENTS {
            let element = stock(&points, segment, x, lane, &evidence)?;
            let key = element.key.clone();
            joinery::member(&mut frame.construction, element.clone())?;
            frame
                .families
                .push((key.clone(), format!("roof-rafter-{segment}")));
            for end in [segment, segment + 1] {
                fit_end(frame, &element, points[end], end, end == segment)?;
            }
        }
        for (joint, &point) in points
            .iter()
            .enumerate()
            .take(RoofSection::SEGMENTS)
            .skip(1)
        {
            pin_lap(frame, lane, x, joint, point, &evidence)?;
        }
    }
    Ok(())
}

fn purlin_key(station: usize) -> String {
    let (level, side) = RoofSection::purlin_station(station);
    format!("roof-purlin-{level}-{side}")
}

fn flatten_purlin_tops(frame: &mut FittedConstruction) -> Result<()> {
    for station in 0..=RoofSection::SEGMENTS {
        let key = purlin_key(station);
        let purlin = frame.construction.element(&key).ok_or("missing purlin")?;
        let mut output = RuleOutput::new();
        let extent = &purlin.extent;
        joinery::remove_box(
            &mut output,
            purlin,
            "flat-rafter-bearing",
            OrientedBox::axis_aligned(
                [-0.001, -0.001, 2.0 * PURLIN_RADIUS - TOP_SEAT_DEPTH],
                [
                    extent.size[0] + 0.002,
                    2.0 * PURLIN_RADIUS + 0.002,
                    TOP_SEAT_DEPTH + 0.001,
                ],
            ),
        )?;
        // This preparation belongs to the same physical purlin already seated
        // on its first support; subsequent rafter relations declare its top contacts.
        let (level, side) = RoofSection::purlin_station(station);
        joinery::apply(
            &mut frame.construction,
            &format!("top-{key}"),
            &format!("seat-{level}-{side}-0"),
            output,
        )?;
    }
    Ok(())
}

fn stock(
    points: &[[f64; 2]],
    segment: usize,
    x: f64,
    lane: usize,
    evidence: &Evidence,
) -> Result<Element> {
    let [a, b] = [points[segment], points[segment + 1]];
    let start = a[0]
        - if segment == 0 {
            EAVE_EXTENSION
        } else {
            HALF_LAP
        };
    let end = b[0]
        + if segment == RoofSection::SEGMENTS - 1 {
            EAVE_EXTENSION
        } else {
            HALF_LAP
        };
    let top = |y| height(a, b, y) + RAFTER_DEPTH / normal(a, b)[1];
    let mut profile = vec![
        [start, height(a, b, start)],
        [end, height(a, b, end)],
        [end, top(end)],
        [start, top(start)],
    ];
    if segment == RoofSection::RIDGE - 1 || segment == RoofSection::RIDGE {
        let neighbor = if segment == RoofSection::RIDGE - 1 {
            RoofSection::RIDGE
        } else {
            RoofSection::RIDGE - 1
        };
        let [c, d] = [points[neighbor], points[neighbor + 1]];
        profile = clip_below(&profile, |y| {
            height(c, d, y) + RAFTER_DEPTH / normal(c, d)[1]
        });
    }
    let bottom = profile.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let top = profile
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max);
    let extent = OrientedBox {
        origin: [x - WIDTH * 0.5, start, bottom],
        axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        size: [end - start, WIDTH, top - bottom],
    };
    for p in &mut profile {
        p[0] -= start;
        p[1] -= bottom;
    }
    let recipe = geometry::extrude(
        geometry::polygon(&profile)?,
        WIDTH,
        Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [0.0, WIDTH, 0.0],
        ),
    )?;
    Ok(Element::new(
        &format!("rafter-{lane}-{segment}"),
        "common-rafter",
        "timber.end",
        extent,
        evidence.clone(),
    )
    .with_part(Part::new(recipe)))
}

fn clip_below(points: &[[f64; 2]], top: impl Fn(f64) -> f64) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    let mut a = points[points.len() - 1];
    for &b in points {
        let [da, db] = [top(a[0]) - a[1], top(b[0]) - b[1]];
        if (da < 0.0) != (db < 0.0) {
            let t = da / (da - db);
            out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
        }
        if db >= 0.0 {
            out.push(b);
        }
        a = b;
    }
    out
}

fn fit_end(
    frame: &mut FittedConstruction,
    rafter: &Element,
    point: [f64; 2],
    end: usize,
    start: bool,
) -> Result<()> {
    let purlin = frame
        .construction
        .element(&purlin_key(end))
        .ok_or("missing rafter support")?
        .clone();
    let x = rafter.extent.origin[0] + WIDTH * 0.5;
    let bearing = point[1] + BEARING_ABOVE_BASE;
    let joint = format!("{}-bearing-{end}", rafter.key);
    relation(
        frame,
        &joint,
        [x, point[0], bearing],
        &[&rafter.key, &purlin.key],
        &rafter.evidence,
    )?;
    let mut output = RuleOutput::new();
    // Local X follows the rafter; Y spans its width. Authoring tools here
    // makes repeated lanes identical without rounding world coordinates.
    let station = point[0] - rafter.extent.origin[1];
    let seat_height = bearing - rafter.extent.origin[2];
    let interior = end != 0 && end != RoofSection::SEGMENTS;
    if interior {
        // Joiner unions these cutters before subtracting them. Putting the
        // bore first avoids a known refusal in the alternate cutter-union order.
        joinery::remove(
            &mut output,
            rafter,
            "pin-bore",
            geometry::cylinder(BORE_RADIUS, WIDTH + 0.002)?,
            Placement3::from_axes(
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0],
                [station, -0.001, seat_height + PIN_LIFT],
            ),
        );
    }
    joinery::remove_box(
        &mut output,
        rafter,
        "birdsmouth",
        OrientedBox::axis_aligned(
            [station - PURLIN_RADIUS - 0.001, -0.001, -0.001],
            [
                2.0 * PURLIN_RADIUS + 0.002,
                WIDTH + 0.002,
                seat_height + 0.001,
            ],
        ),
    )?;
    if interior {
        let cut_x = station - HALF_LAP - if start { 0.001 } else { 0.0 };
        let cut_y = if start { -0.001 } else { WIDTH * 0.5 };
        joinery::remove_box(
            &mut output,
            rafter,
            "side-lap",
            OrientedBox::axis_aligned(
                [cut_x, cut_y, -0.001],
                [
                    2.0 * HALF_LAP + 0.001,
                    WIDTH * 0.5 + 0.001,
                    rafter.extent.size[2] + 0.002,
                ],
            ),
        )?;
    }
    let contact_x = x + if interior {
        if start { WIDTH * 0.25 } else { -WIDTH * 0.25 }
    } else {
        0.0
    };
    let at = [contact_x, point[0], bearing];
    output.contact(contact(
        &joint,
        rafter,
        &purlin,
        at,
        [0.0, 0.0, 1.0],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        [if interior { WIDTH * 0.5 } else { WIDTH }, 0.09],
        ContactMeaning::Bearing,
    ));
    output.transfer(TransferEdge::new(
        &joint,
        &rafter.key,
        TransferTarget::element(&purlin.key),
        TransferKind::Contact,
    ));
    joinery::apply(&mut frame.construction, &joint, &joint, output)
}

fn pin_lap(
    frame: &mut FittedConstruction,
    lane: usize,
    x: f64,
    joint: usize,
    point: [f64; 2],
    evidence: &Evidence,
) -> Result<()> {
    let previous = format!("rafter-{lane}-{}", joint - 1);
    let next = format!("rafter-{lane}-{joint}");
    let key = format!("rafter-lap-{lane}-{joint}");
    let bearing = point[1] + BEARING_ABOVE_BASE;
    let pin_key = format!("rafter-pin-{lane}-{joint}");
    let radius = PIN_RADIUS;
    let pin = Element::new(
        &pin_key,
        "wooden-pin",
        "timber.dark",
        OrientedBox {
            origin: [
                x - WIDTH * 0.5 - 0.008,
                point[0] - radius,
                bearing + PIN_LIFT - radius,
            ],
            axes: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            size: [2.0 * radius, 2.0 * radius, WIDTH + 0.016],
        },
        evidence.clone(),
    )
    .with_part(Part::new(geometry::extrude(
        circle(radius)?,
        WIDTH + 0.016,
        Placement3::translate(radius, radius, 0.0),
    )?));
    frame.families.push((pin_key, "roof-rafter-pin".into()));
    let at = [x, point[0], bearing + 0.022];
    relation(frame, &key, at, &[&previous, &next], evidence)?;
    let mut output = RuleOutput::new();
    output.generate(pin);
    output.contact(contact(
        &key,
        frame.construction.element(&next).ok_or("missing rafter")?,
        frame
            .construction
            .element(&previous)
            .ok_or("missing rafter")?,
        at,
        [1.0, 0.0, 0.0],
        [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [0.08, 0.02],
        ContactMeaning::SideFit,
    ));
    joinery::apply(&mut frame.construction, &key, &key, output)
}

fn relation(
    frame: &mut FittedConstruction,
    key: &str,
    at: [f64; 3],
    members: &[&str],
    evidence: &Evidence,
) -> Result<()> {
    frame.construction.add_node(Node::new(key, at))?;
    frame.construction.add_relation(Relation::new(
        key,
        RelationKind::member_member(key, members),
        "supported-rafter-fit",
        evidence.clone(),
    ))?;
    Ok(())
}

fn contact(
    key: &str,
    a: &Element,
    b: &Element,
    at: [f64; 3],
    normal: [f64; 3],
    tangents: [[f64; 3]; 2],
    size: [f64; 2],
    meaning: ContactMeaning,
) -> ContactPatch {
    ContactPatch::new(
        key,
        Anchor::new(&a.key, a.extent.local_point(at)),
        Anchor::new(&b.key, b.extent.local_point(at)),
        normal,
        tangents,
        meaning,
        a.evidence.clone(),
    )
    .with_footprint_meters(size)
    .with_minimum_overlap_meters(size)
}
