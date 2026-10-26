// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Matched analytic construction and realization workloads for the external CAD comparison.

use exedra_analytic::End;
use exedra_analytic::extrusion::{Extrusion, Pocket, TessellationPolicy};
use exedra_analytic::realize::RealizationPolicy;
use exedra_analytic::revolve::Revolution;
use exedra_constructive::builders;
use exedra_constructive::ir::Placement3;
use exedra_constructive::profile::{Loop2, Profile2, Seg2};
use exedra_mesh::Mesh;
use exedra_profile::{Profile, ProfilePolicy, SegTag};
use serde_json::{Value, json};
use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const POLICY: ProfilePolicy = ProfilePolicy::new(1e-6);
const VENT: SegTag = SegTag(0x5645_4e54);
const WINDOW: SegTag = SegTag(0x5749_4e44);

fn number(spec: &Value, key: &str) -> f64 {
    spec[key].as_f64().expect("numeric fixture field")
}
fn integer(spec: &Value, key: &str) -> u32 {
    spec[key]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .expect("bounded fixture integer")
}
struct Prepared {
    profile: Profile,
    pockets: Vec<Pocket>,
    thickness: f64,
    sweep: Option<f64>,
}
fn shared(profile: &Profile2) -> Result<Profile> {
    Ok(profile.to_line_arc(&POLICY)?)
}
fn prepare(spec: &Value) -> Result<Prepared> {
    if spec["kind"] == "revolve" {
        let points = spec["points"].as_array().expect("profile points");
        let segments = points
            .iter()
            .map(|p| {
                Seg2::line((
                    p[0].as_f64().expect("radius"),
                    p[1].as_f64().expect("height"),
                ))
            })
            .collect();
        return Ok(Prepared {
            profile: shared(&Profile2::simple(Loop2::new(segments)?)?)?,
            pockets: Vec::new(),
            thickness: 0.0,
            sweep: Some(number(spec, "sweep_degrees").to_radians()),
        });
    }
    let outline = builders::rounded_rect_centered(
        number(spec, "width"),
        number(spec, "height"),
        number(spec, "radius"),
    )?;
    let mut holes = Vec::new();
    let mut pockets = Vec::new();
    if spec["kind"] == "plate" {
        for i in 0..integer(spec, "holes") {
            holes.push(builders::circle(1.5)?.translated((
                8.0 * f64::from(i / 4 + 1) - number(spec, "width") * 0.5,
                8.0 * f64::from(i % 4) - 12.0,
            ))?);
        }
    } else {
        let slot = builders::slot_centered(number(spec, "slot_length"), 2.5)?.tagged(VENT);
        for row in 0..integer(spec, "vents") {
            holes.push(slot.translated((-70.0, -30.0 + number(spec, "pitch") * f64::from(row)))?);
        }
        for (radius, x, y) in [(3.5, 60.0, -10.0), (1.5, 45.0, 35.0), (1.5, 75.0, 35.0)] {
            holes.push(builders::circle(radius)?.translated((x, y))?);
        }
        let window =
            shared(&builders::rounded_rect_centered(90.0, 50.0, 3.0)?.translated((-5.0, 5.0))?)?;
        pockets.push(Pocket::from_profile(&window, End::End, 2.0)?.tagged(WINDOW));
        for (x, y) in [(-88.0, -50.0), (88.0, -50.0), (-88.0, 50.0), (88.0, 50.0)] {
            let bore = shared(&builders::circle(2.5)?.translated((x, y))?)?;
            pockets.push(Pocket::from_profile(&bore, End::Start, 4.0)?);
        }
    }
    Ok(Prepared {
        profile: shared(&outline.with_cutouts(holes)?)?,
        pockets,
        thickness: number(spec, "thickness"),
        sweep: None,
    })
}
enum Model {
    Extruded(Extrusion),
    Revolved(Revolution),
}
impl Model {
    fn new(prepared: &Prepared) -> Result<Self> {
        if let Some(sweep) = prepared.sweep {
            return Ok(Self::Revolved(Revolution::new(
                prepared.profile.clone(),
                Placement3::IDENTITY,
                sweep,
            )?));
        }
        let mut model = Extrusion::new(
            prepared.profile.clone(),
            Placement3::IDENTITY,
            prepared.thickness,
        )?;
        for pocket in &prepared.pockets {
            model = model.with_pocket(pocket.clone(), &POLICY)?;
        }
        Ok(Self::Extruded(model))
    }
    fn measures(&self) -> [f64; 2] {
        match self {
            Self::Extruded(s) => [s.volume(), s.surface_area()],
            Self::Revolved(s) => [s.volume(), s.surface_area()],
        }
    }
    fn native_faces(&self) -> usize {
        match self {
            Self::Extruded(s) => s.body().faces().len(),
            Self::Revolved(s) => s.body().faces().len(),
        }
    }
    fn closed(&self) -> bool {
        match self {
            Self::Extruded(s) => s.is_closed(),
            Self::Revolved(s) => s.body().is_closed(),
        }
    }
    fn realize(&self, tolerance: f64) -> Result<Realization> {
        let (mesh, deviation, within) = match self {
            Self::Extruded(s) => {
                let r = s.tessellate(&TessellationPolicy {
                    chord_tolerance: tolerance,
                    ..TessellationPolicy::default()
                })?;
                let within = r.report.within_tolerance();
                (r.mesh, r.report.max_deviation, within)
            }
            Self::Revolved(s) => {
                let r = s.realize(&RealizationPolicy {
                    tolerance,
                    ..RealizationPolicy::default()
                })?;
                let within = r.report.within_tolerance();
                (r.mesh, r.report.max_deviation, within)
            }
        };
        if !within {
            return Err("realization failed requested tolerance".into());
        }
        Ok(Realization { mesh, deviation })
    }
}
struct Realization {
    mesh: Mesh,
    deviation: f64,
}
struct Indexed {
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
}
fn extract(mesh: &Mesh) -> Indexed {
    let mut positions = Vec::with_capacity(mesh.vertices().count());
    let mut remap = Vec::new();
    for id in mesh.vertices() {
        let index = id.index() as usize;
        if remap.len() <= index {
            remap.resize(index + 1, u32::MAX);
        }
        remap[index] = u32::try_from(positions.len()).expect("bounded mesh");
        positions.push(*mesh.vertex_position(id).expect("live vertex"));
    }
    // Closed fixtures: every half-edge is an interior face corner.
    let triangle_count = mesh.half_edge_count() - 2 * mesh.faces().count();
    let mut triangles = Vec::with_capacity(triangle_count);
    for face in mesh.faces() {
        let mut corners = mesh
            .face_loop(face)
            .map(|c| remap[mesh.from_vertex(c).expect("live corner").index() as usize]);
        let first = corners.next().expect("nonempty face");
        let mut previous = corners.next().expect("nonempty face");
        for next in corners {
            triangles.push([first, previous, next]);
            previous = next;
        }
    }
    Indexed {
        positions,
        triangles,
    }
}
fn measure<T>(repetitions: u32, mut action: impl FnMut() -> Result<T>) -> Result<Value> {
    let mut samples = Vec::with_capacity(repetitions as usize);
    for iteration in 0..repetitions + 2 {
        let start = Instant::now();
        let result = action()?;
        black_box(&result);
        let ns = start.elapsed().as_secs_f64() * 1e9;
        drop(result); // Every creation timer stops while its output is still alive.
        if iteration >= 2 {
            samples.push(ns);
        }
    }
    Ok(json!(samples))
}
fn measure_queries(repetitions: u32, model: &Model) -> Value {
    // A retained revolution's measures are two cached fields. Amortize clock
    // resolution instead of reporting individual sub-clock reads as zero cost.
    const BATCH: u32 = 1024;
    let mut samples = Vec::with_capacity(repetitions as usize);
    for iteration in 0..repetitions + 2 {
        let start = Instant::now();
        for _ in 0..BATCH {
            black_box(black_box(model).measures());
        }
        if iteration >= 2 {
            samples.push(start.elapsed().as_secs_f64() * 1e9 / f64::from(BATCH));
        }
    }
    json!(samples)
}
fn case(spec: &Value, tolerance: f64, repetitions: u32) -> Result<Value> {
    let prepared = prepare(spec)?;
    let model = Model::new(&prepared)?;
    let realized = model.realize(tolerance)?;
    if !model.closed()
        || !realized.mesh.boundary_loops()?.is_empty()
        || !realized.mesh.validate_deep().is_empty()
    {
        return Err("comparison requires valid closed output".into());
    }
    let flat = extract(&realized.mesh);
    let mut timings = serde_json::Map::new();
    timings.insert("author".into(), measure(repetitions, || prepare(spec))?);
    timings.insert(
        "construct".into(),
        measure(repetitions, || Model::new(&prepared))?,
    );
    timings.insert("measures".into(), measure_queries(repetitions, &model));
    timings.insert(
        "mesh".into(),
        measure(repetitions, || model.realize(tolerance))?,
    );
    timings.insert(
        "extract".into(),
        measure(repetitions, || Ok(extract(&realized.mesh)))?,
    );
    timings.insert(
        "end_to_end".into(),
        measure(repetitions, || {
            let prepared = prepare(spec)?;
            let model = Model::new(&prepared)?;
            let realized = model.realize(tolerance)?;
            let flat = extract(&realized.mesh);
            Ok((prepared, model, realized, flat))
        })?,
    );
    Ok(json!({
        "id":spec["id"],"measures":model.measures(),"native_faces":model.native_faces(),
        "positions":flat.positions,"triangles":flat.triangles,
        "reported_deviation":realized.deviation,"timings_ns":timings,"query_batch_size":1024
    }))
}
fn main() -> Result<()> {
    let mut repetitions = 9;
    let mut tolerance: f64 = 0.02;
    let mut only = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repetitions" => repetitions = args.next().ok_or("missing repetitions")?.parse()?,
            "--tolerance" => tolerance = args.next().ok_or("missing tolerance")?.parse()?,
            "--case" => only = Some(args.next().ok_or("missing case")?),
            "--info" => {
                println!(
                    "{}",
                    json!({"backend":"exedra","runner_version":env!("CARGO_PKG_VERSION"),
                    "debug_assertions":cfg!(debug_assertions)})
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    if repetitions == 0 || !(tolerance > 0.0 && tolerance.is_finite()) {
        return Err("positive repetitions and finite tolerance required".into());
    }
    let specs: Vec<Value> = serde_json::from_str(include_str!("../../build123d/fixtures.json"))?;
    let mut cases = Vec::new();
    for spec in specs {
        if only
            .as_ref()
            .is_none_or(|id| spec["id"].as_str() == Some(id))
        {
            cases.push(case(&spec, tolerance, repetitions)?);
        }
    }
    if cases.is_empty() {
        return Err("no matching case".into());
    }
    println!(
        "{}",
        json!({"backend":"exedra","tolerance":tolerance,"cases":cases})
    );
    Ok(())
}
