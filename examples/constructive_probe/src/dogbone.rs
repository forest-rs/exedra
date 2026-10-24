// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared geometry for the plain and textured dogbone probes.

use exedra_constructive::{
    chart::{ChartTransform, SurfaceChart},
    ir::{
        CapMode, CsgOp, Node, NodeKind, Path3, PrimitiveSpec, Recipe, RecipeBuilder, SectionLaw,
        UvFrame, UvMapping, UvProjection, UvTransform,
    },
    profile::{Loop2, Profile2, ProfileError, Seg2},
    tessellate::{EvalPolicy, SweepFrameNormal, sample_sweep_frames},
};

pub(crate) const RING_RADIUS: f64 = 50.0;
const END_OFFSET: f64 = 7.5;
const END_RADIUS: f64 = 1.5;
const WEB_HALF_WIDTH: f64 = 1.0;
const CUT_RADIUS: f64 = 5.0;
const CUT_DEPTH: f64 = 8.0;
pub(crate) const CUT_COUNT: usize = 21;

fn dogbone() -> Result<Profile2, ProfileError> {
    // Boundary of the union of a 15 x 2 rectangle and two radius-1.5 discs.
    // Each exposed circular lobe is a major arc between the web intersections.
    let inset = (END_RADIUS * END_RADIUS - WEB_HALF_WIDTH * WEB_HALF_WIDTH).sqrt();
    let inner = END_OFFSET - inset;
    let lobe_angle = core::f64::consts::TAU - 2.0 * (WEB_HALF_WIDTH / END_RADIUS).asin();
    let bulge = (lobe_angle / 4.0).tan();
    Profile2::simple(Loop2::new(vec![
        Seg2::line((inner, -WEB_HALF_WIDTH)),
        Seg2::arc((inner, WEB_HALF_WIDTH), bulge),
        Seg2::line((-inner, WEB_HALF_WIDTH)),
        Seg2::arc((-inner, -WEB_HALF_WIDTH), bulge),
    ])?)
}

pub(crate) fn ring_recipe(
    cut_count: usize,
    first_cut: usize,
    policy: &EvalPolicy,
    native_charts: bool,
    projection: Option<UvMapping>,
) -> Result<Recipe, Box<dyn std::error::Error>> {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(dogbone()?);
    let finish = builder.material_slot("finish");
    let path = Path3::xy_circle(RING_RADIUS)?;
    let section = SectionLaw::IDENTITY.with_turns(4);
    let cutter_frames = sample_sweep_frames(&path, &section, &policy.sweep_path)?
        .closed_stations(CUT_COUNT, 0.5)?;
    let labeled = native_charts || projection.is_some();
    let source = labeled.then(|| builder.source_ref("swept-body"));
    let sweep = builder.add_node(Node {
        surface_chart: native_charts.then_some(SurfaceChart::Sweep {
            wall: ChartTransform {
                matrix: [[1.0 / 40.0, 0.0], [0.0, 1.0 / 40.0]],
                offset: [0.0; 2],
            },
            caps: ChartTransform::IDENTITY,
        }),
        source,
        ..Node::new(NodeKind::Sweep {
            profile,
            path,
            section,
            caps: CapMode::None,
        })
        .with_material(finish)
    })?;
    if cut_count == 0 {
        let root = if let Some(mapping) = projection {
            builder.map_uv(sweep, mapping)?
        } else {
            sweep
        };
        return Ok(builder.finish(root)?);
    }
    let mut operands = Vec::with_capacity(cut_count + 1);
    operands.push(sweep);
    for (index, frame) in cutter_frames[first_cut..first_cut + cut_count]
        .iter()
        .enumerate()
    {
        let placement = frame.normal_placement(
            SweepFrameNormal::SectionY,
            -0.5 * CUT_DEPTH,
            core::f64::consts::PI / 64.0,
        )?;
        let source =
            labeled.then(|| builder.source_ref(&format!("cut-tool-{}", first_cut + index)));
        let cutter = builder.add_node(Node {
            source,
            ..Node::new(NodeKind::Primitive {
                spec: PrimitiveSpec::Cylinder {
                    radius: CUT_RADIUS,
                    height: CUT_DEPTH,
                    segments: 64,
                },
                placement,
            })
            .with_material(finish)
        })?;
        let cutter = if native_charts {
            builder.map_uv(
                cutter,
                UvMapping {
                    projection: UvProjection::Cylindrical(UvFrame::XY.placed(&placement)?),
                    transform: UvTransform::scale([
                        core::f64::consts::TAU * CUT_RADIUS / 40.0,
                        1.0 / 40.0,
                    ])?,
                },
            )?
        } else {
            cutter
        };
        operands.push(cutter);
    }
    let root = builder.add_node(NodeKind::Csg {
        op: CsgOp::Difference,
        operands,
    })?;
    let root = if let Some(mapping) = projection {
        builder.map_uv(root, mapping)?
    } else {
        root
    };
    Ok(builder.finish(root)?)
}
