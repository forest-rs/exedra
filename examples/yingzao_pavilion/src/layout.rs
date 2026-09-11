// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact module and roof datums; floating geometry starts at the adapter.

use setout::{
    Count, EvaluationScenarioBuilder, Knowledge, Length, NetworkBuilder, Offset, Quantity,
    QuantityPolicy, Rational, RootClaimSetBuilder, ScaleLength, Sum, compile_plan, evaluate,
};
use setout_generate::{InvocationKey, LinearBayDistribution, distribute_linear_bays};
use setout_joiner::{lower_length, lower_rational_iotas};

use crate::Result;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Parameters {
    pub(crate) bays: u32,
    pub(crate) span_mm: u32,
    pub(crate) depth_mm: u32,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            bays: 1,
            span_mm: 3_600,
            depth_mm: 3_600,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Layout {
    pub(crate) frames: Vec<f64>,
    pub(crate) span: f64,
    pub(crate) depth: f64,
    pub(crate) width: f64,
    pub(crate) fen: f64,
    pub(crate) arm_width: f64,
    pub(crate) arm_depth: f64,
    pub(crate) column_height: f64,
    /// Authored flat-seat depth, retained exactly for the timber rule.
    pub(crate) seat_depth: Length,
    /// Eave to ridge: horizontal distance from ridge, and height above ground.
    pub(crate) roof: [[f64; 2]; 5],
}

impl Layout {
    pub(crate) fn purlin_bottom(&self, level: usize) -> f64 {
        self.roof[level][1] - 0.205
    }

    pub(crate) fn bearing_height(&self, level: usize) -> f64 {
        self.purlin_bottom(level) + lower_length(self.seat_depth)
    }

    pub(crate) fn resolve(params: Parameters) -> Result<Self> {
        if !(1..=5).contains(&params.bays)
            || !(2_400..=5_400).contains(&params.span_mm)
            || !(2_400..=5_400).contains(&params.depth_mm)
        {
            return Err("use 1–5 bays and spans/depths between 2400 and 5400 mm".into());
        }
        let mut network = NetworkBuilder::new();
        let fen = network.declare::<Length>("module/fen", QuantityPolicy::positive())?;
        let span = network.declare::<Length>("plan/span", QuantityPolicy::positive())?;
        let depth = network.declare::<Length>("plan/depth", QuantityPolicy::positive())?;
        let arm_width = scaled(&mut network, "timber/width", &fen, 10, 1)?;
        let arm_depth = scaled(&mut network, "timber/depth", &fen, 15, 1)?;
        let seat_depth = scaled(&mut network, "purlin/seat-depth", &fen, 1, 1)?;
        let column = scaled(&mut network, "column/height", &fen, 170, 1)?;
        let width = scaled(
            &mut network,
            "plan/width",
            &span,
            i128::from(params.bays),
            1,
        )?;
        let half_depth = scaled(&mut network, "roof/half-depth", &depth, 1, 2)?;
        let overhang = scaled(&mut network, "roof/eave-overhang", &fen, 50, 1)?;
        let run = network.declare::<Length>("roof/half-run", QuantityPolicy::positive())?;
        network.relate(Sum::new("roof/run", half_depth, overhang, run.clone())?)?;
        let rise = scaled(&mut network, "roof/rise", &run, 1, 3)?;
        // Juzhe with four equal rafter runs: starting at the ridge, depress
        // the next working line by R/10, R/20, then R/40. Written here as
        // exact ratios of R, ordered from eave toward ridge.
        let lift1 = scaled(&mut network, "roof/purlin-1-lift", &rise, 1, 6)?;
        let lift2 = scaled(&mut network, "roof/purlin-2-lift", &rise, 23, 60)?;
        let lift3 = scaled(&mut network, "roof/purlin-3-lift", &rise, 13, 20)?;
        let definition = network.finish()?;
        let mut roots = RootClaimSetBuilder::new(&definition);
        for (key, quantity, mm) in [
            ("root/fen", &fen, 15),
            ("root/span", &span, params.span_mm),
            ("root/depth", &depth, params.depth_mm),
        ] {
            roots.author(
                key,
                quantity,
                Knowledge::exact(Length::millimeters(u64::from(mm)).ok_or("invalid dimension")?),
            )?;
        }
        let roots = roots.finish()?;
        let scenario = EvaluationScenarioBuilder::new("pavilion")?
            .activate_all(&roots)
            .finish(&roots)?;
        let plan = compile_plan(&definition, &roots, &scenario)?;
        let evaluation = evaluate(&definition, &roots, &scenario, &plan)?;
        let read = |quantity: &Quantity<Length>| -> Result<f64> {
            Ok(lower_length(evaluation.exact(quantity)?))
        };
        let invocation = InvocationKey::new("pavilion/bays")?;
        let bays = distribute_linear_bays(&LinearBayDistribution {
            invocation: &invocation,
            start: Offset::ZERO,
            end: Offset::from_iota(i64::try_from(evaluation.exact(&width)?.iota())?),
            bays: Count::new(u64::from(params.bays)),
            overrides: &[],
        })?;
        let width = read(&width)?;
        let mut frames: Vec<_> = bays
            .items()
            .iter()
            .map(|bay| lower_rational_iotas(bay.start()) - width * 0.5)
            .collect();
        frames.push(width * 0.5);
        let run = read(&run)?;
        let column_height = read(&column)?;
        let eave = 0.54 + column_height + 0.78;
        Ok(Self {
            frames,
            width,
            span: read(&span)?,
            depth: read(&depth)?,
            fen: read(&fen)?,
            arm_width: read(&arm_width)?,
            arm_depth: read(&arm_depth)?,
            column_height,
            seat_depth: evaluation.exact(&seat_depth)?,
            roof: [
                [run, eave],
                [run * 0.75, eave + read(&lift1)?],
                [run * 0.5, eave + read(&lift2)?],
                [run * 0.25, eave + read(&lift3)?],
                [0.0, eave + read(&rise)?],
            ],
        })
    }
}

fn scaled(
    network: &mut NetworkBuilder,
    key: &str,
    input: &Quantity<Length>,
    numerator: i128,
    denominator: u128,
) -> Result<Quantity<Length>> {
    let output = network.declare::<Length>(key, QuantityPolicy::positive())?;
    network.relate(ScaleLength::new(
        key,
        input.clone(),
        output.clone(),
        Rational::new(numerator, denominator)?,
    )?)?;
    Ok(output)
}
