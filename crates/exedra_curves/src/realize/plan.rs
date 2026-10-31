// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The realization plan: topology built once, controls and attributes
//! updated per field.

use alloc::vec::Vec;

use exedra_math::{cross, dot};

use super::{
    Attribute, Centerline, CurveDescription, Field, Generations, PlanOptions, PlanReport, PlanWork,
    RealizeError,
};
use crate::frames::{direction, seam, transported};
use crate::nurbs::{local_evaluation, representable, valid_weight};
use crate::sample::span_controls;
use crate::vector::{WeightedPoint, magnitude};
use crate::{
    CubicBasis, Frame, FrameReport, NurbsCurve, Station, SweepError, SweptMesh, Workspace,
    rotation_minimizing_frames_into,
};

/// `2^-26`: the crate's threshold for parallel directions and, here, for
/// a joint's points to coincide relative to their size.
const SQRT_EPSILON: f64 = f64::from_bits(0x3E50_0000_0000_0000);

/// The inverse of the Bezier basis matrix: it takes a cubic's power
/// coefficients `[a3, a2, a1, a0]` (rows of `M * C`) to its Bezier
/// controls. `Q0 = P(0)`, `Q1 = P(0) + P'(0) / 3`,
/// `Q2 = P(1) - P'(1) / 3`, `Q3 = P(1)`.
const BEZIER_INVERSE: [[f64; 4]; 4] = [
    [0.0, 0.0, 0.0, 1.0],
    [0.0, 0.0, 1.0 / 3.0, 1.0],
    [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0],
    [1.0, 1.0, 1.0, 1.0],
];

/// A run of a plan's spans between breaks, with its stations: framed and
/// swept as one strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    /// Its first span.
    pub first_span: u32,
    /// Its spans, at least one.
    pub spans: u32,
    /// Its first station.
    pub first_station: u32,
    /// Its stations: `spans * per_span`, plus 1 for its end unless the
    /// curve is closed.
    pub stations: u32,
}

/// One span's topology.
#[derive(Clone, Copy, Debug)]
struct SpanTopology {
    /// Its parameter interval: the knot interval for NURBS, `[0, 1]` for
    /// a cubic span.
    interval: [f64; 2],
    /// A cubic span's index into the chain's spans, or a NURBS span's
    /// knot index `k` (its controls are `k - p ..= k`).
    source: usize,
    /// Its piece.
    piece: u32,
    /// Its stations, `[first, end)`.
    stations: [u32; 2],
}

/// What a joint between two spans is, as [the module documents](super#joints).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Joint {
    pub(super) crease: bool,
    pub(super) gap: bool,
}

/// An analytic curve planned for drawing: its description, its topology,
/// and everything derived from its controls and attributes, kept current
/// by explicit updates. See [the module documentation](super) for the
/// layers and what each update redoes.
///
/// The plan owns its [`CurveDescription`]; read it with
/// [`Self::description`] and change it only through the `set_` methods,
/// which keep the derived state consistent. A change of topology (span
/// indices, basis, knots, order, control count, closure, options) is a new
/// plan.
#[derive(Clone, Debug)]
pub struct RealizationPlan {
    pub(super) description: CurveDescription,
    pub(super) options: PlanOptions,
    // Topology.
    /// Controls per span.
    pub(super) order: usize,
    spans: Vec<SpanTopology>,
    /// A cubic chain's one 4 by 4 extraction matrix, or `order^2` per
    /// NURBS span, row-major.
    extraction: Vec<f64>,
    /// The spans each control shapes, `control_spans[offsets[c]..offsets[c + 1]]`.
    control_spans: Vec<u32>,
    control_offsets: Vec<u32>,
    pub(super) pieces: Vec<Piece>,
    pub(super) stations: Vec<Station>,
    // Controls.
    /// `order` weighted Bezier controls per span.
    pub(super) bezier: Vec<WeightedPoint<3>>,
    span_bounds: Vec<[[f64; 3]; 2]>,
    /// The joint at each span's start: the previous span's end, or on a
    /// closed curve the last span's for span 0. Span 0 of an open curve,
    /// and a piece's first span, have none.
    pub(super) joints: Vec<Joint>,
    points: Vec<[f64; 3]>,
    tangents: Vec<[f64; 3]>,
    pub(super) frames: Vec<Frame>,
    frame_reports: Vec<FrameReport>,
    // Attributes.
    /// `order` width coefficients per span, weight 1.
    pub(super) width_coefficients: Vec<WeightedPoint<1>>,
    widths: Vec<f64>,
    radii: Vec<f64>,
    /// `order` normal coefficients per span, weight 1; empty without
    /// normals.
    pub(super) normal_coefficients: Vec<WeightedPoint<3>>,
    normals: Vec<[f64; 3]>,
    generations: Generations,
    built: PlanWork,
    /// Scratch: which spans an update touches.
    dirty: Vec<bool>,
}

impl RealizationPlan {
    /// Plans `description`: checks it, builds its topology, then its
    /// controls' and attributes' layers, with `workspace` as scratch. What
    /// that computed is [`PlanReport::built`].
    ///
    /// # Errors
    ///
    /// [`RealizeError::NoSpans`] or [`RealizeError::Control`] for a cubic
    /// chain's spans, [`RealizeError::ClosedWithBreaks`],
    /// [`RealizeError::TooLarge`] when its stations or controls do not fit
    /// a `u32`, [`RealizeError::Value`] for a cubic chain's non-finite point,
    /// [`RealizeError::Count`] or [`RealizeError::Value`] for an
    /// attribute.
    pub fn new(
        description: CurveDescription,
        options: PlanOptions,
        workspace: &mut Workspace,
    ) -> Result<Self, RealizeError> {
        let options = PlanOptions {
            per_span: options.per_span.max(1),
            ..options
        };
        let mut plan = Self {
            description,
            options,
            order: 0,
            spans: Vec::new(),
            extraction: Vec::new(),
            control_spans: Vec::new(),
            control_offsets: Vec::new(),
            pieces: Vec::new(),
            stations: Vec::new(),
            bezier: Vec::new(),
            span_bounds: Vec::new(),
            joints: Vec::new(),
            points: Vec::new(),
            tangents: Vec::new(),
            frames: Vec::new(),
            frame_reports: Vec::new(),
            width_coefficients: Vec::new(),
            widths: Vec::new(),
            radii: Vec::new(),
            normal_coefficients: Vec::new(),
            normals: Vec::new(),
            generations: Generations::default(),
            built: PlanWork::default(),
            dirty: Vec::new(),
        };
        let mut work = PlanWork::default();
        plan.build_topology(&mut work)?;
        if let Centerline::Cubic { points, .. } = &plan.description.centerline {
            check_points(points, None)?;
        }
        check_attribute(
            &plan.description.widths,
            plan.attribute_counts(),
            Field::Widths,
        )?;
        if let Some(normals) = &plan.description.normals {
            check_attribute(normals, plan.attribute_counts(), Field::Normals)?;
        }
        plan.dirty.clear();
        plan.dirty.resize(plan.spans.len(), true);
        plan.update_controls(&mut work, workspace);
        plan.update_widths(&mut work);
        plan.update_normals(&mut work);
        plan.built = work;
        Ok(plan)
    }

    /// The description the plan realizes, as last updated.
    #[must_use]
    pub fn description(&self) -> &CurveDescription {
        &self.description
    }

    /// The options the plan was built with, `per_span` at least 1.
    #[must_use]
    pub fn options(&self) -> PlanOptions {
        self.options
    }

    /// The plan's sizes, joints, frames, build work and generations.
    #[must_use]
    pub fn report(&self) -> PlanReport {
        let mut frames = FrameReport {
            seam_cos: 1.0,
            ..FrameReport::default()
        };
        for (piece, report) in self.pieces.iter().zip(&self.frame_reports) {
            frames.stations += report.stations;
            frames.repeated += report.repeated;
            frames.cusps += report.cusps;
            frames.estimated += report.estimated;
            // A piece's cusps count its own stations; the plan's, all.
            frames.first_cusp = frames
                .first_cusp
                .or(report.first_cusp.map(|i| i + piece.first_station));
        }
        if self.description.closed
            && let Some(piece) = self.frame_reports.first()
        {
            frames.seam_cos = piece.seam_cos;
            frames.seam_sin = piece.seam_sin;
        }
        PlanReport {
            order: count(self.order),
            spans: count(self.spans.len()),
            pieces: count(self.pieces.len()),
            stations: count(self.stations.len()),
            creases: count(self.joints.iter().filter(|j| j.crease).count()),
            gaps: count(self.joints.iter().filter(|j| j.gap).count()),
            frames,
            built: self.built,
            generations: self.generations,
        }
    }

    /// The plan's generations: which layers have changed, and how often.
    #[must_use]
    pub fn generations(&self) -> Generations {
        self.generations
    }

    /// The pieces, in order: one, or one more than the NURBS curve's
    /// breaks.
    #[must_use]
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// The stations, piece after piece: each names a plan span and the
    /// parameter `u` in `[0, 1]` across it.
    #[must_use]
    pub fn stations(&self) -> &[Station] {
        &self.stations
    }

    /// The curve's point at each station.
    #[must_use]
    pub fn points(&self) -> &[[f64; 3]] {
        &self.points
    }

    /// The curve's derivative at each station: with respect to the cubic
    /// span's `u`, or the NURBS curve's `t`.
    #[must_use]
    pub fn tangents(&self) -> &[[f64; 3]] {
        &self.tangents
    }

    /// The rotation-minimizing frame at each station, framed along the
    /// analytic tangents ([`crate::rotation_minimizing_frames_along`]),
    /// each piece on its own.
    #[must_use]
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// The width at each station.
    #[must_use]
    pub fn widths(&self) -> &[f64] {
        &self.widths
    }

    /// The ribbon normal at each station as interpolated, before it is
    /// made perpendicular to the tangent; `None` without normals.
    #[must_use]
    pub fn normals(&self) -> Option<&[[f64; 3]]> {
        self.description.normals.as_ref().map(|_| &*self.normals)
    }

    /// How many spans the plan has.
    #[must_use]
    pub fn spans(&self) -> usize {
        self.spans.len()
    }

    /// Span `span`'s weighted Bezier controls, [`PlanReport::order`] of
    /// them over `u` in `[0, 1]` (weight 1 for a cubic chain); `None` past
    /// the last span.
    #[must_use]
    pub fn span_bezier(&self, span: usize) -> Option<&[WeightedPoint<3>]> {
        (span < self.spans.len()).then(|| &self.bezier[span * self.order..(span + 1) * self.order])
    }

    /// A box holding span `span`'s piece of the centerline, its Bezier
    /// controls' box; `None` past the last span.
    #[must_use]
    pub fn span_bounds(&self, span: usize) -> Option<[[f64; 3]; 2]> {
        self.span_bounds.get(span).copied()
    }

    /// A box holding the whole centerline: its spans' boxes. A sweep
    /// reaches past it by up to half the largest width.
    #[must_use]
    pub fn bounds(&self) -> [[f64; 3]; 2] {
        let mut out = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        for [lo, hi] in &self.span_bounds {
            for c in 0..3 {
                out[0][c] = out[0][c].min(lo[c]);
                out[1][c] = out[1][c].max(hi[c]);
            }
        }
        out
    }

    /// Replaces the centerline's controls, one per control, and redoes the
    /// controls' layer for the spans the changed ones shape: their Bezier
    /// controls, bounds, station points and tangents and adjacent joints,
    /// and the frames from their first station to the end of its piece
    /// (the whole piece where a tangent had to be estimated). Topology and
    /// attributes are untouched. Controls are compared bit for bit, so an update that
    /// changes nothing does nothing.
    ///
    /// The result is the plan a fresh [`Self::new`] would build, bit for
    /// bit.
    ///
    /// # Errors
    ///
    /// [`RealizeError::Count`] unless there is one point per control,
    /// [`RealizeError::Value`] for a point [`crate::Nurbs::validate`]
    /// would refuse with its weight: not finite, or, weighted, overflowing
    /// or leaving the normal range. The plan is unchanged.
    pub fn set_points(
        &mut self,
        points: &[[f64; 3]],
        workspace: &mut Workspace,
    ) -> Result<PlanWork, RealizeError> {
        let expected = self.description.centerline.controls();
        if points.len() != expected {
            return Err(RealizeError::Count {
                field: Field::Points,
                given: points.len(),
                expected,
            });
        }
        let mut work = PlanWork::default();
        self.dirty.clear();
        self.dirty.resize(self.spans.len(), false);
        match &mut self.description.centerline {
            Centerline::Cubic { points: old, .. } => {
                check_points(points, None)?;
                for (c, (old, new)) in old.iter_mut().zip(points).enumerate() {
                    if !same(old, new) {
                        *old = *new;
                        work.changed_controls += 1;
                        mark(
                            &self.control_offsets,
                            &self.control_spans,
                            c,
                            &mut self.dirty,
                        );
                    }
                }
            }
            Centerline::Nurbs(curve) => {
                check_points(points, curve.weights.as_deref())?;
                for (c, (old, new)) in curve.points.iter_mut().zip(points).enumerate() {
                    if !same(old, new) {
                        *old = *new;
                        work.changed_controls += 1;
                        mark(
                            &self.control_offsets,
                            &self.control_spans,
                            c,
                            &mut self.dirty,
                        );
                    }
                }
            }
        }
        if work.changed_controls > 0 {
            self.update_controls(&mut work, workspace);
            self.generations.controls += 1;
        }
        Ok(work)
    }

    /// Replaces a NURBS centerline's weights (`None` for all 1), and
    /// redoes the controls' layer for the spans the changed ones shape, as
    /// [`Self::set_points`] does. Weights are compared as the weight each
    /// control takes, so `None` and all 1 are the same.
    ///
    /// # Errors
    ///
    /// [`RealizeError::NotRational`] for a cubic chain,
    /// [`RealizeError::Count`] unless there is one weight per control,
    /// [`RealizeError::Value`] for a weight outside `[2^-256, 2^256]`, or
    /// one that makes its point overflow. The plan is unchanged.
    pub fn set_weights(
        &mut self,
        weights: Option<&[f64]>,
        workspace: &mut Workspace,
    ) -> Result<PlanWork, RealizeError> {
        let Centerline::Nurbs(curve) = &mut self.description.centerline else {
            return Err(RealizeError::NotRational);
        };
        let n = curve.points.len();
        if let Some(weights) = weights {
            if weights.len() != n {
                return Err(RealizeError::Count {
                    field: Field::Weights,
                    given: weights.len(),
                    expected: n,
                });
            }
            if let Some(index) = weights.iter().position(|w| !valid_weight(*w)) {
                return Err(RealizeError::Value {
                    field: Field::Weights,
                    index,
                });
            }
            check_points(&curve.points, Some(weights)).map_err(|error| match error {
                RealizeError::Value { index, .. } => RealizeError::Value {
                    field: Field::Weights,
                    index,
                },
                other => other,
            })?;
        }
        let mut work = PlanWork::default();
        self.dirty.clear();
        self.dirty.resize(self.spans.len(), false);
        let weight = |w: Option<&[f64]>, c: usize| w.map_or(1.0, |w| w[c]);
        for c in 0..n {
            let (old, new) = (weight(curve.weights.as_deref(), c), weight(weights, c));
            if old.to_bits() != new.to_bits() {
                work.changed_controls += 1;
                mark(
                    &self.control_offsets,
                    &self.control_spans,
                    c,
                    &mut self.dirty,
                );
            }
        }
        if work.changed_controls > 0 {
            curve.weights = weights.map(<[f64]>::to_vec);
            self.update_controls(&mut work, workspace);
            self.generations.controls += 1;
        }
        Ok(work)
    }

    /// Replaces the widths and redoes their coefficients and station
    /// values; nothing else. Widths equal to the plan's do nothing.
    ///
    /// # Errors
    ///
    /// [`RealizeError::Count`] for the wrong number of values for their
    /// class, [`RealizeError::Value`] for a negative or non-finite width.
    /// The plan is unchanged.
    pub fn set_widths(&mut self, widths: Attribute<f64>) -> Result<PlanWork, RealizeError> {
        check_attribute(&widths, self.attribute_counts(), Field::Widths)?;
        let mut work = PlanWork::default();
        if !same_attribute(&widths, &self.description.widths) {
            self.description.widths = widths;
            self.update_widths(&mut work);
            self.generations.widths += 1;
        }
        Ok(work)
    }

    /// Replaces the ribbon normals (`None` to face the frames' normals) and
    /// redoes their coefficients and station values; nothing else. Normals
    /// equal to the plan's do nothing.
    ///
    /// # Errors
    ///
    /// [`RealizeError::Count`] for the wrong number of values for their
    /// class, [`RealizeError::Value`] for a non-finite normal. The plan is
    /// unchanged.
    pub fn set_normals(
        &mut self,
        normals: Option<Attribute<[f64; 3]>>,
    ) -> Result<PlanWork, RealizeError> {
        if let Some(normals) = &normals {
            check_attribute(normals, self.attribute_counts(), Field::Normals)?;
        }
        let mut work = PlanWork::default();
        let same = match (&normals, &self.description.normals) {
            (Some(new), Some(old)) => same_attribute(new, old),
            (new, old) => new.is_none() && old.is_none(),
        };
        if !same {
            self.description.normals = normals;
            self.update_normals(&mut work);
            self.generations.normals += 1;
        }
        Ok(work)
    }

    /// A tube of `sides` sides around the plan's frames, of radius half
    /// the width at each station, into `out`, with `workspace` as scratch:
    /// each piece swept by [`crate::tube_into`] and appended, its vertices
    /// after the previous piece's. `out.report` sums the pieces' reports.
    ///
    /// # Errors
    ///
    /// [`RealizeError::Sweep`] as [`crate::tube`] reports it, a station
    /// counted across pieces; `out` is left empty.
    pub fn tube_into(
        &self,
        sides: u32,
        workspace: &mut Workspace,
        out: &mut SweptMesh,
    ) -> Result<(), RealizeError> {
        self.sweep_pieces(workspace, out, |plan, piece, workspace, mesh| {
            let range =
                piece.first_station as usize..(piece.first_station + piece.stations) as usize;
            crate::tube_into(
                &plan.frames[range.clone()],
                &plan.radii[range],
                sides,
                plan.description.closed,
                workspace,
                mesh,
            )
        })
    }

    /// [`Self::tube_into`] into a new mesh.
    ///
    /// # Errors
    ///
    /// As [`Self::tube_into`].
    pub fn tube(&self, sides: u32) -> Result<SweptMesh, RealizeError> {
        let mut mesh = SweptMesh::default();
        self.tube_into(sides, &mut Workspace::new(), &mut mesh)?;
        Ok(mesh)
    }

    /// A ribbon along the plan's frames, as wide as the width at each
    /// station and facing its normal, or the frame's without normals, into
    /// `out`: each piece swept by [`crate::ribbon_into`] and appended.
    ///
    /// # Errors
    ///
    /// As [`Self::tube_into`], for [`crate::ribbon`]'s errors.
    pub fn ribbon_into(
        &self,
        workspace: &mut Workspace,
        out: &mut SweptMesh,
    ) -> Result<(), RealizeError> {
        self.sweep_pieces(workspace, out, |plan, piece, _, mesh| {
            let range =
                piece.first_station as usize..(piece.first_station + piece.stations) as usize;
            crate::ribbon_into(
                &plan.frames[range.clone()],
                plan.normals().map(|normals| &normals[range.clone()]),
                &plan.widths[range],
                plan.description.closed,
                mesh,
            )
        })
    }

    /// [`Self::ribbon_into`] into a new mesh.
    ///
    /// # Errors
    ///
    /// As [`Self::ribbon_into`].
    pub fn ribbon(&self) -> Result<SweptMesh, RealizeError> {
        let mut mesh = SweptMesh::default();
        self.ribbon_into(&mut Workspace::new(), &mut mesh)?;
        Ok(mesh)
    }

    /// Sweeps each piece with `sweep`, the first into `out` and the rest
    /// through the workspace's mesh, appended.
    fn sweep_pieces(
        &self,
        workspace: &mut Workspace,
        out: &mut SweptMesh,
        sweep: impl Fn(&Self, &Piece, &mut Workspace, &mut SweptMesh) -> Result<(), SweepError>,
    ) -> Result<(), RealizeError> {
        let result = self.sweep_pieces_with(workspace, out, sweep);
        if result.is_err() {
            out.positions.clear();
            out.normals.clear();
            out.indices.clear();
            out.ring = 0;
            out.report = SweptMesh::new().report;
        }
        result
    }

    fn sweep_pieces_with(
        &self,
        workspace: &mut Workspace,
        out: &mut SweptMesh,
        sweep: impl Fn(&Self, &Piece, &mut Workspace, &mut SweptMesh) -> Result<(), SweepError>,
    ) -> Result<(), RealizeError> {
        let (first, rest) = self
            .pieces
            .split_first()
            .expect("a plan has at least one piece");
        sweep(self, first, workspace, out)?;
        let mut piece_mesh = core::mem::take(&mut workspace.mesh);
        let result = rest.iter().try_for_each(|piece| {
            sweep(self, piece, workspace, &mut piece_mesh).map_err(|error| match error {
                SweepError::Size { station } => SweepError::Size {
                    station: station + piece.first_station as usize,
                },
                other => other,
            })?;
            append(out, &piece_mesh)
        });
        workspace.mesh = piece_mesh;
        result.map_err(RealizeError::Sweep)
    }

    /// The piece span `s` belongs to.
    pub(super) fn span_piece(&self, s: usize) -> u32 {
        self.spans[s].piece
    }

    /// Span `s`'s first station.
    pub(super) fn span_first_station(&self, s: usize) -> u32 {
        self.spans[s].stations[0]
    }

    /// Builds the spans, extraction matrices, pieces, stations and
    /// control-to-span index.
    fn build_topology(&mut self, work: &mut PlanWork) -> Result<(), RealizeError> {
        let per_span = self.options.per_span;
        let closed = self.description.closed;
        self.spans.clear();
        self.extraction.clear();
        let controls = self.description.centerline.controls();
        // Every index the plan keeps, and every count it reports, is a
        // `u32`: count the spans and pieces from the description alone,
        // and refuse a plan too large for them before allocating any of it.
        let (order, spans, pieces) = match &self.description.centerline {
            Centerline::Cubic { spans, .. } => (4, spans.len(), 1),
            Centerline::Nurbs(curve) => {
                let (spans, pieces) = nurbs_census(curve);
                (curve.order as usize, spans, pieces)
            }
        };
        let stations = spans
            .checked_mul(per_span as usize)
            .and_then(|n| n.checked_add(if closed { 0 } else { pieces }));
        let beziers = spans.checked_mul(order);
        // The largest arrays, in bytes, must be ones a `Vec` can hold:
        // frames per station, Bezier controls, and a NURBS curve's
        // extraction matrices, `order^2` per span.
        let extraction = match self.description.centerline {
            Centerline::Cubic { .. } => Some(16),
            Centerline::Nurbs(_) => beziers.and_then(|n| n.checked_mul(order)),
        };
        let bytes = |n: Option<usize>, size: usize| {
            n.and_then(|n| n.checked_mul(size))
                .filter(|&b| isize::try_from(b).is_ok())
        };
        if [Some(controls), beziers, stations]
            .iter()
            .any(|n| n.is_none_or(|n| u32::try_from(n).is_err()))
            || bytes(stations, size_of::<Frame>()).is_none()
            || bytes(beziers, size_of::<WeightedPoint<3>>()).is_none()
            || bytes(extraction, size_of::<f64>()).is_none()
        {
            return Err(RealizeError::TooLarge);
        }
        let mut shaped: Vec<(u32, u32)> = Vec::new();
        match &self.description.centerline {
            Centerline::Cubic {
                basis,
                points,
                spans,
            } => {
                if spans.is_empty() {
                    return Err(RealizeError::NoSpans);
                }
                for s in 0..spans.len() {
                    span_controls(points, spans, count(s)).map_err(RealizeError::Control)?;
                }
                self.order = 4;
                let m = basis.matrix();
                self.extraction.extend((0..16).map(|e| {
                    let (i, j) = (e / 4, e % 4);
                    (0..4).fold(0.0, |sum, r| sum + BEZIER_INVERSE[i][r] * m[r][j])
                }));
                for (s, span) in spans.iter().enumerate() {
                    self.spans.push(SpanTopology {
                        interval: [0.0, 1.0],
                        source: s,
                        piece: 0,
                        stations: [0, 0],
                    });
                    shaped.extend(span.iter().map(|&c| (c, count(s))));
                    work.extracted_spans += 1;
                }
                self.pieces.push(Piece {
                    first_span: 0,
                    spans: count(spans.len()),
                    first_station: 0,
                    stations: 0,
                });
            }
            Centerline::Nurbs(curve) => {
                let view = curve.view();
                let order = curve.order as usize;
                let p = order - 1;
                self.order = order;
                let knots = curve.knots();
                let mut matrix = Vec::new();
                let mut previous: Option<usize> = None;
                for k in p..controls {
                    if knots[k] >= knots[k + 1] {
                        continue;
                    }
                    let s = count(self.spans.len());
                    if previous.is_none_or(|last| k - last >= order) {
                        self.pieces.push(Piece {
                            first_span: s,
                            spans: 0,
                            first_station: 0,
                            stations: 0,
                        });
                    }
                    let piece = count(self.pieces.len() - 1);
                    self.pieces[piece as usize].spans += 1;
                    view.bezier_extraction_into(k, &mut matrix);
                    self.extraction.extend_from_slice(&matrix);
                    work.extracted_spans += 1;
                    self.spans.push(SpanTopology {
                        interval: [knots[k], knots[k + 1]],
                        source: k,
                        piece,
                        stations: [0, 0],
                    });
                    shaped.extend((k - p..=k).map(|c| (count(c), s)));
                    previous = Some(k);
                }
                if closed && self.pieces.len() > 1 {
                    return Err(RealizeError::ClosedWithBreaks {
                        breaks: self.pieces.len() - 1,
                    });
                }
            }
        }
        // The spans each control shapes, by counting sort.
        self.control_offsets.clear();
        self.control_offsets.resize(controls + 1, 0);
        for &(c, _) in &shaped {
            self.control_offsets[c as usize + 1] += 1;
        }
        for c in 0..controls {
            self.control_offsets[c + 1] += self.control_offsets[c];
        }
        self.control_spans.clear();
        self.control_spans.resize(shaped.len(), 0);
        let mut next = self.control_offsets.clone();
        for &(c, s) in &shaped {
            let slot = &mut next[c as usize];
            self.control_spans[*slot as usize] = s;
            *slot += 1;
        }
        // Stations: `per_span` per span at `u = k / per_span`, and each
        // open piece's end.
        self.stations.clear();
        for piece in &mut self.pieces {
            piece.first_station = count(self.stations.len());
            let spans = piece.first_span..piece.first_span + piece.spans;
            for s in spans.clone() {
                let first = count(self.stations.len());
                self.stations.extend((0..per_span).map(|k| Station {
                    span: s,
                    u: f64::from(k) / f64::from(per_span),
                }));
                self.spans[s as usize].stations = [first, count(self.stations.len())];
            }
            if !closed {
                let last = spans.end - 1;
                self.stations.push(Station { span: last, u: 1.0 });
                self.spans[last as usize].stations[1] += 1;
            }
            piece.stations = count(self.stations.len()) - piece.first_station;
        }
        Ok(())
    }

    /// Counts of values each attribute class needs: per control and per
    /// joint.
    fn attribute_counts(&self) -> [usize; 2] {
        let joints = self.spans.len() + usize::from(!self.description.closed);
        [self.description.centerline.controls(), joints]
    }

    /// Redoes the controls' layer for the spans marked in `dirty`.
    fn update_controls(&mut self, work: &mut PlanWork, workspace: &mut Workspace) {
        let spans = self.spans.len();
        let order = self.order;
        self.bezier.resize(spans * order, WeightedPoint::ZERO);
        self.span_bounds.resize(spans, [[0.0; 3]; 2]);
        self.joints.resize(spans, Joint::default());
        self.points.resize(self.stations.len(), [0.0; 3]);
        self.tangents.resize(self.stations.len(), [0.0; 3]);
        let mut scratch = [WeightedPoint::ZERO; 16];
        // Past order 16, de Casteljau's scheme runs in the workspace.
        let mut big = core::mem::take(&mut workspace.evaluation);
        for s in 0..spans {
            if !self.dirty[s] {
                continue;
            }
            work.bezier_spans += 1;
            self.fill_bezier(s);
            self.span_bounds[s] = bounds_of(&self.bezier[s * order..(s + 1) * order]);
            let [first, end] = self.spans[s].stations;
            for i in first..end {
                let station = self.stations[i as usize];
                let evaluation =
                    self.evaluate(station.span as usize, station.u, &mut scratch, &mut big);
                self.points[i as usize] = evaluation.0;
                self.tangents[i as usize] = evaluation.1;
                work.evaluated_stations += 1;
            }
        }
        // The joint at a span's start joins the span before it (the last,
        // for a closed curve's first span) to it.
        for s in 0..spans {
            if self.dirty[s] || self.dirty[(s + spans - 1) % spans] {
                self.check_joint(s, &mut scratch, &mut big, work);
            }
        }
        workspace.evaluation = big;
        self.reframe(work, workspace);
    }

    /// Frames each piece holding a dirty span: from its first dirty
    /// station on, carrying the frame before it, when the piece's frames
    /// exist and none of its tangents is estimated (so no station's frame
    /// depends on chords around it); the whole piece otherwise. Either way
    /// the frames are those framing the whole piece gives, bit for bit:
    /// the same step, [`crate::frames::transported`], from the same
    /// frame.
    fn reframe(&mut self, work: &mut PlanWork, workspace: &mut Workspace) {
        let framed = self.frames.len() == self.stations.len();
        let mut frames = core::mem::take(&mut workspace.frames);
        self.frames.resize(
            self.stations.len(),
            Frame {
                point: [0.0; 3],
                tangent: [0.0; 3],
                normal: [0.0; 3],
                binormal: [0.0; 3],
            },
        );
        self.frame_reports
            .resize(self.pieces.len(), FrameReport::default());
        for (index, piece) in self.pieces.iter().enumerate() {
            let spans = piece.first_span as usize..(piece.first_span + piece.spans) as usize;
            if !self.dirty[spans.clone()].iter().any(|d| *d) {
                continue;
            }
            let range =
                piece.first_station as usize..(piece.first_station + piece.stations) as usize;
            let from = (spans.start..spans.end)
                .find(|&s| self.dirty[s])
                .map_or(range.start, |s| self.spans[s].stations[0] as usize);
            let report = &mut self.frame_reports[index];
            let resumable = framed
                && from > range.start
                && report.estimated == 0
                && self.tangents[from..range.end]
                    .iter()
                    .all(|t| direction(*t).is_some());
            if resumable {
                for i in from..range.end {
                    let tangent = direction(self.tangents[i]).expect("checked above");
                    self.frames[i] = transported(&self.frames[i - 1], self.points[i], tangent);
                }
                let points = &self.points[range.clone()];
                report.repeated = count(points.windows(2).filter(|w| w[0] == w[1]).count());
                if self.description.closed {
                    [report.seam_cos, report.seam_sin] =
                        seam(&self.frames[range.start], &self.frames[range.end - 1]);
                }
                work.framed_stations += (range.end - from) as u64;
                continue;
            }
            let report = rotation_minimizing_frames_into(
                &self.points[range.clone()],
                Some(&self.tangents[range.clone()]),
                self.description.closed,
                self.options.seed,
                workspace,
                &mut frames,
            )
            .expect("one tangent per point");
            self.frames[range].copy_from_slice(&frames);
            self.frame_reports[index] = report;
            work.framed_stations += u64::from(piece.stations);
        }
        workspace.frames = frames;
    }

    /// Span `s`'s weighted Bezier controls: its extraction matrix applied
    /// to its weighted controls.
    fn fill_bezier(&mut self, s: usize) {
        let order = self.order;
        for i in 0..order {
            let row = &self.span_extraction(s)[i * order..(i + 1) * order];
            let mut sum = WeightedPoint::ZERO;
            for (j, e) in row.iter().enumerate() {
                let control = self.weighted_control(self.control_of(s, j));
                sum = WeightedPoint::plus(sum, WeightedPoint::times(control, *e));
            }
            if let Centerline::Cubic { .. } = self.description.centerline {
                // A cubic chain is polynomial; a Hermite matrix's rows need
                // not sum to 1, so the weight is set, not summed.
                sum.weight = 1.0;
            }
            self.bezier[s * order + i] = sum;
        }
    }

    /// Span `s`'s extraction matrix, `order` by `order`, row-major: the
    /// cubic chain's one, or the NURBS span's own.
    fn span_extraction(&self, s: usize) -> &[f64] {
        let size = self.order * self.order;
        match self.description.centerline {
            Centerline::Cubic { .. } => &self.extraction,
            Centerline::Nurbs(_) => &self.extraction[s * size..(s + 1) * size],
        }
    }

    /// The control index of span `s`'s `j`-th control: the one a cubic
    /// span names, or a NURBS span `k`'s `k - p + j`.
    fn control_of(&self, s: usize, j: usize) -> usize {
        let source = self.spans[s].source;
        match &self.description.centerline {
            Centerline::Cubic { spans, .. } => spans[source][j] as usize,
            Centerline::Nurbs(_) => source + 1 - self.order + j,
        }
    }

    /// Control `c` in homogeneous form, weight 1 for a cubic chain.
    fn weighted_control(&self, c: usize) -> WeightedPoint<3> {
        match &self.description.centerline {
            Centerline::Cubic { points, .. } => WeightedPoint {
                weighted: points[c],
                weight: 1.0,
            },
            Centerline::Nurbs(curve) => {
                let w = curve.weights.as_ref().map_or(1.0, |w| w[c]);
                WeightedPoint::new(curve.points[c], w)
            }
        }
    }

    /// The point and derivative of span `s` at `u`: a cubic span's basis
    /// on its controls, as [`crate::sample()`] evaluates it; a NURBS span's
    /// Bezier controls at `u` itself, its derivative with respect to the
    /// knot parameter.
    fn evaluate(
        &self,
        s: usize,
        u: f64,
        scratch: &mut [WeightedPoint<3>; 16],
        big: &mut Vec<WeightedPoint<3>>,
    ) -> ([f64; 3], [f64; 3]) {
        let span = self.spans[s];
        match &self.description.centerline {
            Centerline::Cubic {
                basis,
                points,
                spans,
            } => {
                let controls = spans[span.source].map(|c| points[c as usize]);
                (basis.evaluate(controls, u), basis.derivative(controls, u))
            }
            Centerline::Nurbs(_) => {
                let [a, b] = span.interval;
                let controls = &self.bezier[s * self.order..(s + 1) * self.order];
                let scratch = scratch_for(scratch, big, self.order);
                let e = local_evaluation(controls, b - a, u, scratch);
                (e.point, e.first)
            }
        }
    }

    /// Classifies the joint at span `s`'s start.
    fn check_joint(
        &mut self,
        s: usize,
        scratch: &mut [WeightedPoint<3>; 16],
        big: &mut Vec<WeightedPoint<3>>,
        work: &mut PlanWork,
    ) {
        let piece = self.pieces[self.spans[s].piece as usize];
        let previous = if s > piece.first_span as usize {
            s - 1
        } else if self.description.closed {
            self.spans.len() - 1
        } else {
            self.joints[s] = Joint::default();
            return;
        };
        work.checked_joints += 1;
        let (left, left_tangent) = self.evaluate(previous, 1.0, scratch, big);
        let (right, right_tangent) = self.evaluate(s, 0.0, scratch, big);
        let size = left
            .iter()
            .chain(&right)
            .fold(0.0_f64, |m, c| m.max(c.abs()));
        let apart = magnitude(exedra_math::sub(right, left));
        let crease = match (direction(left_tangent), direction(right_tangent)) {
            (Some(a), Some(b)) => {
                let sin = cross(a, b);
                dot(a, b) <= 0.0 || dot(sin, sin) > f64::EPSILON
            }
            _ => true,
        };
        self.joints[s] = Joint {
            crease,
            gap: apart > SQRT_EPSILON * size || apart.is_nan(),
        };
    }

    /// Redoes the widths' coefficients and station values.
    fn update_widths(&mut self, work: &mut PlanWork) {
        let mut coefficients = core::mem::take(&mut self.width_coefficients);
        let mut values = core::mem::take(&mut self.widths);
        values.clear();
        let widths = &self.description.widths;
        self.attribute(
            widths,
            |w| [w],
            &mut coefficients,
            |[w]| values.push(w),
            work,
        );
        self.radii.clear();
        self.radii.extend(values.iter().map(|w| w * 0.5));
        self.width_coefficients = coefficients;
        self.widths = values;
    }

    /// Redoes the normals' coefficients and station values.
    fn update_normals(&mut self, work: &mut PlanWork) {
        let mut coefficients = core::mem::take(&mut self.normal_coefficients);
        let mut values = core::mem::take(&mut self.normals);
        coefficients.clear();
        values.clear();
        if let Some(normals) = &self.description.normals {
            self.attribute(normals, |n| n, &mut coefficients, |n| values.push(n), work);
        }
        self.normal_coefficients = coefficients;
        self.normals = values;
    }

    /// An attribute's Bezier coefficients per span into `coefficients`, and
    /// its value at each station through `push`.
    fn attribute<T: Copy, const D: usize>(
        &self,
        attribute: &Attribute<T>,
        to: impl Fn(T) -> [f64; D],
        coefficients: &mut Vec<WeightedPoint<D>>,
        mut push: impl FnMut([f64; D]),
        work: &mut PlanWork,
    ) {
        let order = self.order;
        let p = (order - 1) as f64;
        let joints = self.attribute_counts()[1];
        coefficients.clear();
        let weighted = |value: [f64; D]| WeightedPoint {
            weighted: value,
            weight: 1.0,
        };
        let mut scratch = [WeightedPoint::ZERO; 16];
        let mut big = Vec::new();
        for (s, span) in self.spans.iter().enumerate() {
            work.attribute_spans += 1;
            let first = coefficients.len();
            match attribute {
                Attribute::Constant(value) => {
                    coefficients.extend((0..order).map(|_| weighted(to(*value))));
                }
                Attribute::Varying(values) => {
                    let (a, b) = (to(values[s]), to(values[(s + 1) % joints]));
                    coefficients.extend((0..order).map(|i| {
                        let u = i as f64 / p;
                        weighted(core::array::from_fn(|c| (1.0 - u) * a[c] + u * b[c]))
                    }));
                }
                Attribute::Vertex(values) => {
                    for i in 0..order {
                        let row = &self.span_extraction(s)[i * order..(i + 1) * order];
                        let mut sum = [0.0; D];
                        for (j, e) in row.iter().enumerate() {
                            let value = to(values[self.control_of(s, j)]);
                            for c in 0..D {
                                sum[c] += e * value[c];
                            }
                        }
                        coefficients.push(weighted(sum));
                    }
                }
            }
            for station in &self.stations[span.stations[0] as usize..span.stations[1] as usize] {
                let u = station.u;
                work.attribute_stations += 1;
                push(match attribute {
                    Attribute::Constant(value) => to(*value),
                    Attribute::Varying(values) => {
                        let (a, b) = (to(values[s]), to(values[(s + 1) % joints]));
                        core::array::from_fn(|c| (1.0 - u) * a[c] + u * b[c])
                    }
                    Attribute::Vertex(values) => match &self.description.centerline {
                        Centerline::Cubic { basis, spans, .. } => {
                            let picked = spans[span.source].map(|c| to(values[c as usize]));
                            CubicBasis::evaluate(*basis, picked, u)
                        }
                        Centerline::Nurbs(_) => {
                            let [a, b] = span.interval;
                            let scratch = scratch_for(&mut scratch, &mut big, order);
                            local_evaluation(&coefficients[first..first + order], b - a, u, scratch)
                                .point
                        }
                    },
                });
            }
        }
    }
}

/// `scratch`'s first `len` entries, or `big`'s when `len` passes 16.
fn scratch_for<'a, const N: usize>(
    scratch: &'a mut [WeightedPoint<N>; 16],
    big: &'a mut Vec<WeightedPoint<N>>,
    len: usize,
) -> &'a mut [WeightedPoint<N>] {
    if len <= scratch.len() {
        &mut scratch[..len]
    } else {
        big.clear();
        big.resize(len, WeightedPoint::ZERO);
        big
    }
}

/// A NURBS curve's nonempty knot spans and its pieces, the runs of them
/// between knots of full multiplicity, as the plan's topology takes them.
fn nurbs_census(curve: &NurbsCurve) -> (usize, usize) {
    let order = curve.order as usize;
    let knots = curve.knots();
    let (mut spans, mut pieces) = (0, 0);
    let mut previous: Option<usize> = None;
    for k in order - 1..curve.points().len() {
        if knots[k] < knots[k + 1] {
            spans += 1;
            pieces += usize::from(previous.is_none_or(|last| k - last >= order));
            previous = Some(k);
        }
    }
    (spans, pieces)
}

/// Marks the spans control `c` shapes.
fn mark(offsets: &[u32], spans: &[u32], c: usize, dirty: &mut [bool]) {
    for &s in &spans[offsets[c] as usize..offsets[c + 1] as usize] {
        dirty[s as usize] = true;
    }
}

/// Whether two attributes are the same class and the same bits.
fn same_attribute<T: AttributeValue>(a: &Attribute<T>, b: &Attribute<T>) -> bool {
    match (a, b) {
        (Attribute::Constant(a), Attribute::Constant(b)) => a.same(b),
        (Attribute::Varying(a), Attribute::Varying(b))
        | (Attribute::Vertex(a), Attribute::Vertex(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same(b))
        }
        _ => false,
    }
}

/// Whether two points are the same bits.
fn same(a: &[f64; 3], b: &[f64; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

/// Each point one [`crate::Nurbs::validate`] accepts with its weight (1
/// when unweighted).
fn check_points(points: &[[f64; 3]], weights: Option<&[f64]>) -> Result<(), RealizeError> {
    let weight = |c: usize| weights.map_or(1.0, |w| w[c]);
    match points
        .iter()
        .enumerate()
        .position(|(c, point)| !representable(point, weight(c)))
    {
        Some(index) => Err(RealizeError::Value {
            field: Field::Points,
            index,
        }),
        None => Ok(()),
    }
}

/// An attribute's count for its class, and its values: finite, and for
/// widths nonnegative.
fn check_attribute<T: AttributeValue>(
    attribute: &Attribute<T>,
    [controls, joints]: [usize; 2],
    field: Field,
) -> Result<(), RealizeError> {
    let (values, expected) = match attribute {
        Attribute::Constant(value) => (core::slice::from_ref(value), 1),
        Attribute::Varying(values) => (values.as_slice(), joints),
        Attribute::Vertex(values) => (values.as_slice(), controls),
    };
    if values.len() != expected {
        return Err(RealizeError::Count {
            field,
            given: values.len(),
            expected,
        });
    }
    match values.iter().position(|v| !v.valid()) {
        Some(index) => Err(RealizeError::Value { field, index }),
        None => Ok(()),
    }
}

/// A value an [`Attribute`] carries, when it is valid, and when two are
/// the same bits.
trait AttributeValue {
    fn valid(&self) -> bool;
    fn same(&self, other: &Self) -> bool;
}

impl AttributeValue for f64 {
    /// A width: finite and nonnegative.
    fn valid(&self) -> bool {
        self.is_finite() && *self >= 0.0
    }
    fn same(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl AttributeValue for [f64; 3] {
    /// A normal: finite.
    fn valid(&self) -> bool {
        self.iter().all(|c| c.is_finite())
    }
    fn same(&self, other: &Self) -> bool {
        same(self, other)
    }
}

/// The box of weighted controls' points.
fn bounds_of(controls: &[WeightedPoint<3>]) -> [[f64; 3]; 2] {
    let mut out = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for control in controls {
        let point = control.point();
        for c in 0..3 {
            out[0][c] = out[0][c].min(point[c]);
            out[1][c] = out[1][c].max(point[c]);
        }
    }
    out
}

/// Appends `piece` to `out`, its indices after `out`'s vertices, and adds
/// its report's counts.
fn append(out: &mut SweptMesh, piece: &SweptMesh) -> Result<(), SweepError> {
    let offset = u32::try_from(out.positions.len()).map_err(|_| SweepError::TooLarge)?;
    let vertices = out
        .report
        .vertices
        .checked_add(piece.report.vertices)
        .ok_or(SweepError::TooLarge)?;
    let triangles = out
        .report
        .triangles
        .checked_add(piece.report.triangles)
        .ok_or(SweepError::TooLarge)?;
    out.positions.extend_from_slice(&piece.positions);
    out.normals.extend_from_slice(&piece.normals);
    out.indices.extend(piece.indices.iter().map(|i| i + offset));
    out.report.stations += piece.report.stations;
    out.report.vertices = vertices;
    out.report.triangles = triangles;
    out.report.collapsed += piece.report.collapsed;
    out.report.unoriented += piece.report.unoriented;
    Ok(())
}

/// A count as `u32`, saturating; [`RealizationPlan::new`] refuses plans
/// whose counts do not fit.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;
    use crate::realize::GpuSpan;
    use crate::{
        NurbsCurve, Span, rotation_minimizing_frames_along, sample, sample_derivative, stations,
    };

    /// `n` deterministic controls wandering in space.
    fn wander(n: usize) -> Vec<[f64; 3]> {
        (0..n)
            .map(|i| {
                [
                    i as f64,
                    ((i * 7) % 5) as f64 * 0.3,
                    ((i * 3) % 4) as f64 * 0.2,
                ]
            })
            .collect()
    }

    /// Spans `[i, i + 1, i + 2, i + 3]` of a step-1 chain of `n` controls,
    /// wrapping when `closed`.
    fn step_one(n: usize, closed: bool) -> Vec<Span> {
        let spans = if closed { n } else { n - 3 };
        (0..spans)
            .map(|i| core::array::from_fn(|k| u32::try_from((i + k) % n).unwrap()))
            .collect()
    }

    fn widths(n: usize) -> Vec<f64> {
        (0..n).map(|i| 0.1 + 0.05 * ((i * 5) % 3) as f64).collect()
    }

    fn cubic(basis: CubicBasis, n: usize, closed: bool) -> CurveDescription {
        CurveDescription {
            centerline: Centerline::Cubic {
                basis,
                points: wander(n),
                spans: step_one(n, closed),
            },
            widths: Attribute::Vertex(widths(n)),
            normals: Some(Attribute::Vertex(
                (0..n).map(|i| [0.0, 0.3 * i as f64, 1.0]).collect(),
            )),
            closed,
        }
    }

    /// A weighted cubic NURBS of 9 controls with a double interior knot,
    /// and with `break_at_three` a full-multiplicity knot at 3 instead.
    fn nurbs(break_at_three: bool) -> CurveDescription {
        let knots = if break_at_three {
            vec![
                0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0, 4.0, 4.0, 4.0, 4.0,
            ]
        } else {
            vec![
                0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 3.0, 4.0, 5.0, 5.0, 5.0, 5.0,
            ]
        };
        let n = knots.len() - 4;
        let weights = (0..n).map(|i| 1.0 + 0.25 * (i % 3) as f64).collect();
        let curve = NurbsCurve::new(4, knots, wander(n), Some(weights)).unwrap();
        CurveDescription {
            centerline: Centerline::Nurbs(curve),
            widths: Attribute::Vertex(widths(n)),
            normals: None,
            closed: false,
        }
    }

    fn plan(description: CurveDescription, per_span: u32) -> RealizationPlan {
        let options = PlanOptions::new(per_span).with_seed([0.0, 0.0, 1.0]);
        RealizationPlan::new(description, options, &mut Workspace::new()).unwrap()
    }

    #[test]
    fn a_cubic_plan_sweeps_what_the_direct_functions_sweep() {
        for (basis, closed) in [(CubicBasis::CatmullRom, false), (CubicBasis::BSpline, true)] {
            let description = cubic(basis, 9, closed);
            let plan = plan(description.clone(), 5);
            let Centerline::Cubic { points, spans, .. } = &description.centerline else {
                unreachable!()
            };
            let at = stations(u32::try_from(spans.len()).unwrap(), 5, closed);
            assert_eq!(plan.stations(), &at[..]);
            let sampled = sample(basis, points, spans, &at).unwrap();
            let tangents = sample_derivative(basis, points, spans, &at).unwrap();
            let Attribute::Vertex(w) = &description.widths else {
                unreachable!()
            };
            let w: Vec<[f64; 1]> = w.iter().map(|w| [*w]).collect();
            let w: Vec<f64> = sample(basis, &w, spans, &at)
                .unwrap()
                .into_iter()
                .map(|[w]| w)
                .collect();
            let (frames, report) = rotation_minimizing_frames_along(
                &sampled,
                &tangents,
                closed,
                Some([0.0, 0.0, 1.0]),
            )
            .unwrap();
            assert_eq!(plan.points(), &sampled[..]);
            assert_eq!(plan.frames(), &frames[..]);
            assert_eq!(plan.report().frames, report);
            let radii: Vec<f64> = w.iter().map(|w| w * 0.5).collect();
            assert_eq!(
                plan.tube(7).unwrap(),
                crate::tube(&frames, &radii, 7, closed).unwrap()
            );
            let Some(Attribute::Vertex(n)) = &description.normals else {
                unreachable!()
            };
            let normals = sample(basis, n, spans, &at).unwrap();
            assert_eq!(
                plan.ribbon().unwrap(),
                crate::ribbon(&frames, Some(&normals), &w, closed).unwrap()
            );
        }
    }

    #[test]
    fn a_nurbs_plan_samples_and_evaluates_as_the_curve_does() {
        let description = nurbs(false);
        let Centerline::Nurbs(curve) = &description.centerline else {
            unreachable!()
        };
        let plan = plan(description.clone(), 4);
        let samples = curve.view().sample(None, 4).unwrap();
        let prepared = curve.view().prepare();
        assert_eq!(plan.stations().len(), samples.params.len());
        for (i, station) in plan.stations().iter().enumerate() {
            let [a, b] = prepared.span_domain(station.span as usize).unwrap();
            let t = if station.u >= 1.0 {
                b
            } else {
                a + (b - a) * station.u
            };
            assert_eq!(t, samples.params[i]);
            let expected = prepared.evaluation_in(station.span as usize, t).unwrap();
            for c in 0..3 {
                assert!((plan.points()[i][c] - expected.point[c]).abs() < 1e-13);
                assert!((plan.tangents()[i][c] - expected.first[c]).abs() < 1e-12);
                assert!((plan.points()[i][c] - samples.points[i][c]).abs() < 1e-13);
            }
        }
        // Widths follow the polynomial basis: a NURBS curve of the same
        // knots and no weights.
        let Attribute::Vertex(w) = &description.widths else {
            unreachable!()
        };
        let width_curve = NurbsCurve::<1>::new(
            4,
            curve.knots().to_vec(),
            w.iter().map(|w| [*w]).collect(),
            None,
        )
        .unwrap();
        for (station, width) in plan.stations().iter().zip(plan.widths()) {
            let [a, b] = prepared.span_domain(station.span as usize).unwrap();
            let t = if station.u >= 1.0 {
                b
            } else {
                a + (b - a) * station.u
            };
            let expected = width_curve
                .view()
                .prepare()
                .evaluation_in(station.span as usize, t);
            assert!((width - expected.unwrap().point[0]).abs() < 1e-14);
        }
        let report = plan.report();
        assert_eq!((report.order, report.spans, report.pieces), (4, 5, 1));
        assert_eq!(report.built.extracted_spans, 5);
        assert_eq!(report.creases + report.gaps, 0);
    }

    /// Every derived value of two plans, compared bit for bit.
    /// `a` and `b` hold the same bits: `Debug` writes each float's
    /// shortest round trip, so it tells `-0.0` from `0.0` where `==` does
    /// not.
    fn assert_same(a: &RealizationPlan, b: &RealizationPlan) {
        macro_rules! same {
            ($($field:expr),*) => {
                $(assert_eq!(
                    alloc::format!("{:?}", $field(a)),
                    alloc::format!("{:?}", $field(b)),
                    stringify!($field)
                );)*
            };
        }
        same!(
            |p: &RealizationPlan| p.description().clone(),
            |p: &RealizationPlan| p.points().to_vec(),
            |p: &RealizationPlan| p.tangents().to_vec(),
            |p: &RealizationPlan| p.frames().to_vec(),
            |p: &RealizationPlan| p.widths().to_vec(),
            |p: &RealizationPlan| p.normals().map(<[_]>::to_vec),
            |p: &RealizationPlan| p.bezier.clone(),
            |p: &RealizationPlan| p.span_bounds.clone(),
            |p: &RealizationPlan| p.joints.clone(),
            |p: &RealizationPlan| p.width_coefficients.clone(),
            |p: &RealizationPlan| p.normal_coefficients.clone(),
            |p: &RealizationPlan| p.report().frames,
            |p: &RealizationPlan| p.gpu([1.0, 2.0, 3.0]).unwrap()
        );
    }

    #[test]
    fn moving_a_control_redoes_only_its_spans_and_matches_a_fresh_plan() {
        let mut workspace = Workspace::new();
        // A Catmull-Rom chain of 12 controls: 9 spans of 6 stations.
        let mut cubic_plan = plan(cubic(CubicBasis::CatmullRom, 12, false), 6);
        let mut points = wander(12);
        points[5][1] += 0.5;
        let work = cubic_plan.set_points(&points, &mut workspace).unwrap();
        // Control 5 shapes spans 2 to 5.
        assert_eq!(
            work,
            PlanWork {
                changed_controls: 1,
                bezier_spans: 4,
                evaluated_stations: 4 * 6,
                checked_joints: 5,
                // From span 2's first station to the end.
                framed_stations: 9 * 6 + 1 - 2 * 6,
                ..PlanWork::default()
            }
        );
        let mut fresh = cubic(CubicBasis::CatmullRom, 12, false);
        if let Centerline::Cubic { points: p, .. } = &mut fresh.centerline {
            p.clone_from(&points);
        }
        assert_same(&cubic_plan, &plan(fresh, 6));
        assert_eq!(cubic_plan.generations().controls, 1);
        // The same points again: nothing to do, no new generation.
        let work = cubic_plan.set_points(&points, &mut workspace).unwrap();
        assert_eq!(work, PlanWork::default());
        assert_eq!(cubic_plan.generations().controls, 1);

        // A NURBS curve with a break: moving a control of the second piece
        // reframes that piece alone.
        let mut nurbs_plan = plan(nurbs(true), 4);
        assert_eq!(nurbs_plan.pieces().len(), 2);
        let second = nurbs_plan.pieces()[1];
        let mut points = wander(10);
        points[8][2] -= 1.0;
        let work = nurbs_plan.set_points(&points, &mut workspace).unwrap();
        assert_eq!(work.bezier_spans, 1);
        assert_eq!(work.framed_stations, u64::from(second.stations));
        assert_eq!(work.extracted_spans, 0);
        let mut fresh = nurbs(true);
        if let Centerline::Nurbs(curve) = &mut fresh.centerline {
            curve.points.clone_from(&points);
        }
        assert_same(&nurbs_plan, &plan(fresh, 4));

        // New weights: the same, by weight.
        let mut weights: Vec<f64> = (0..10).map(|i| 1.0 + 0.25 * (i % 3) as f64).collect();
        weights[0] = 2.0;
        let work = nurbs_plan
            .set_weights(Some(&weights), &mut workspace)
            .unwrap();
        assert_eq!((work.changed_controls, work.bezier_spans), (1, 1));
        let mut fresh = nurbs(true);
        if let Centerline::Nurbs(curve) = &mut fresh.centerline {
            curve.points.clone_from(&points);
            curve.weights = Some(weights.clone());
        }
        assert_same(&nurbs_plan, &plan(fresh, 4));
    }

    #[test]
    fn reframing_resumes_where_it_can_and_matches_a_fresh_plan() {
        let mut workspace = Workspace::new();
        // A closed B-spline loop: the seam is measured again after
        // resuming.
        let mut closed = plan(cubic(CubicBasis::BSpline, 10, true), 4);
        let mut points = wander(10);
        points[6] = [6.0, 2.0, -1.0];
        let work = closed.set_points(&points, &mut workspace).unwrap();
        // Control 6 shapes spans 3 to 6; framing resumes at span 3.
        assert_eq!(work.framed_stations, 40 - 3 * 4);
        let mut fresh = cubic(CubicBasis::BSpline, 10, true);
        if let Centerline::Cubic { points: p, .. } = &mut fresh.centerline {
            p.clone_from(&points);
        }
        assert_same(&closed, &plan(fresh, 4));

        // A Bezier chain whose second span starts with a zero tangent: that
        // tangent is estimated, so the whole piece is framed again.
        let mut points = wander(7);
        points[4] = points[3];
        let description = |points: Vec<[f64; 3]>| CurveDescription {
            centerline: Centerline::Cubic {
                basis: CubicBasis::Bezier,
                points,
                spans: vec![[0, 1, 2, 3], [3, 4, 5, 6]],
            },
            widths: Attribute::Constant(0.1),
            normals: None,
            closed: false,
        };
        let mut estimated = plan(description(points.clone()), 4);
        assert_eq!(estimated.report().frames.estimated, 1);
        points[6][2] += 1.0;
        let work = estimated.set_points(&points, &mut workspace).unwrap();
        assert_eq!(work.framed_stations, 9);
        assert_same(&estimated, &plan(description(points), 4));
    }

    #[test]
    fn attributes_update_without_touching_the_centerline() {
        let mut description = cubic(CubicBasis::Bezier, 7, false);
        if let Centerline::Cubic { spans, .. } = &mut description.centerline {
            *spans = vec![[0, 1, 2, 3], [3, 4, 5, 6]];
        }
        let mut plan = plan(description, 3);
        let frames = plan.frames().to_vec();
        let work = plan
            .set_widths(Attribute::Varying(vec![0.1, 0.2, 0.3]))
            .unwrap();
        assert_eq!(
            work,
            PlanWork {
                attribute_spans: 2,
                attribute_stations: 7,
                ..PlanWork::default()
            }
        );
        assert_eq!(plan.widths()[3], 0.2);
        assert_eq!(plan.frames(), &frames[..]);
        let generations = plan.generations();
        assert_eq!((generations.controls, generations.widths), (0, 1));
        let work = plan.set_normals(None).unwrap();
        assert_eq!(work.attribute_spans, 0);
        assert!(plan.normals().is_none());
        assert_eq!(plan.generations().normals, 1);
        // Unchanged widths do nothing.
        let work = plan
            .set_widths(Attribute::Varying(vec![0.1, 0.2, 0.3]))
            .unwrap();
        assert_eq!(work, PlanWork::default());
        assert_eq!(plan.generations().widths, 1);
    }

    #[test]
    fn breaks_split_pieces_and_sweeps_append_them() {
        let plan = plan(nurbs(true), 4);
        let pieces = plan.pieces();
        assert_eq!(pieces.len(), 2);
        assert_eq!((pieces[0].spans, pieces[1].spans), (3, 1));
        assert_eq!((pieces[0].stations, pieces[1].stations), (13, 5));
        let mesh = plan.tube(5).unwrap();
        assert_eq!(mesh.report.stations, 18);
        assert_eq!(mesh.report.vertices, 18 * 5);
        assert_eq!(mesh.report.triangles, (12 + 4) * 5 * 2);
        // The second strip's indices start past the first's vertices.
        let first_strip = 12 * 5 * 2 * 3;
        assert!(mesh.indices[first_strip..].iter().all(|&i| i >= 13 * 5));
        let mut closed = nurbs(true);
        closed.closed = true;
        assert_eq!(
            RealizationPlan::new(closed, PlanOptions::new(4), &mut Workspace::new()).unwrap_err(),
            RealizeError::ClosedWithBreaks { breaks: 1 }
        );
    }

    #[test]
    fn joints_report_creases_and_gaps() {
        // Two Bezier spans meeting at (3, 0, 0), the second leaving at a
        // right angle; then a third span going on in the same direction
        // from elsewhere.
        let points = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [3.0, 2.0, 0.0],
            [3.0, 3.0, 0.0],
            [5.0, 3.0, 0.0],
            [5.0, 4.0, 0.0],
            [5.0, 5.0, 0.0],
            [5.0, 6.0, 0.0],
        ];
        let description = CurveDescription {
            centerline: Centerline::Cubic {
                basis: CubicBasis::Bezier,
                points,
                spans: vec![[0, 1, 2, 3], [3, 4, 5, 6], [7, 8, 9, 10]],
            },
            widths: Attribute::Constant(0.1),
            normals: None,
            closed: false,
        };
        let plan = plan(description, 2);
        let report = plan.report();
        assert_eq!((report.creases, report.gaps), (1, 1));
        let gpu = plan.gpu([0.0; 3]).unwrap();
        assert_eq!(gpu.spans[1].flags, GpuSpan::CREASE);
        assert_eq!(gpu.spans[2].flags, GpuSpan::GAP | GpuSpan::PIECE_END);
        assert_eq!(gpu.spans[0].flags, GpuSpan::PIECE_START);
    }

    #[test]
    fn bad_input_is_named_and_leaves_the_plan_as_it_was() {
        let mut workspace = Workspace::new();
        let new = |description| {
            RealizationPlan::new(description, PlanOptions::new(2), &mut Workspace::new())
                .unwrap_err()
        };
        let mut no_spans = cubic(CubicBasis::BSpline, 5, false);
        if let Centerline::Cubic { spans, .. } = &mut no_spans.centerline {
            spans.clear();
        }
        assert_eq!(new(no_spans), RealizeError::NoSpans);
        let mut out_of_range = cubic(CubicBasis::BSpline, 5, false);
        if let Centerline::Cubic { spans, .. } = &mut out_of_range.centerline {
            spans[1][3] = 9;
        }
        assert!(matches!(new(out_of_range), RealizeError::Control(_)));
        let mut few_widths = cubic(CubicBasis::BSpline, 5, false);
        few_widths.widths = Attribute::Varying(vec![1.0; 2]);
        assert_eq!(
            new(few_widths),
            RealizeError::Count {
                field: Field::Widths,
                given: 2,
                expected: 3
            }
        );

        let mut plan = plan(cubic(CubicBasis::BSpline, 5, false), 2);
        let before = plan.clone();
        let mut points = wander(5);
        points[2][0] = f64::NAN;
        assert_eq!(
            plan.set_points(&points, &mut workspace),
            Err(RealizeError::Value {
                field: Field::Points,
                index: 2
            })
        );
        assert_eq!(
            plan.set_points(&points[..4], &mut workspace),
            Err(RealizeError::Count {
                field: Field::Points,
                given: 4,
                expected: 5
            })
        );
        assert_eq!(
            plan.set_weights(None, &mut workspace),
            Err(RealizeError::NotRational)
        );
        assert_eq!(
            plan.set_widths(Attribute::Constant(-1.0)),
            Err(RealizeError::Value {
                field: Field::Widths,
                index: 0
            })
        );
        assert_same(&plan, &before);
        assert_eq!(plan.generations(), before.generations());
    }

    #[test]
    fn joints_are_judged_alike_at_any_scale() {
        // A right-angled crease, then a gap, as tiny or as vast as `f64`
        // allows: squared lengths would under- or overflow.
        for scale in [1e-160, 1.0, 1e160] {
            let points = [
                [0.0, 0.0],
                [1.0, 0.0],
                [2.0, 0.0],
                [3.0, 0.0],
                [3.0, 1.0],
                [3.0, 2.0],
                [3.0, 3.0],
                [5.0, 3.0],
                [5.0, 4.0],
                [5.0, 5.0],
                [5.0, 6.0],
            ]
            .map(|[x, y]| [x * scale, y * scale, 0.0]);
            let plan = plan(
                CurveDescription {
                    centerline: Centerline::Cubic {
                        basis: CubicBasis::Bezier,
                        points: points.into(),
                        spans: vec![[0, 1, 2, 3], [3, 4, 5, 6], [7, 8, 9, 10]],
                    },
                    widths: Attribute::Constant(0.1 * scale),
                    normals: None,
                    closed: false,
                },
                2,
            );
            let joints = &plan.joints;
            assert_eq!(
                joints[1],
                Joint {
                    crease: true,
                    gap: false
                },
                "{scale}"
            );
            assert_eq!(
                joints[2],
                Joint {
                    crease: false,
                    gap: true
                },
                "{scale}"
            );
        }
    }

    #[test]
    fn plans_too_large_to_count_are_refused_before_allocating() {
        // Two cubic spans, or a NURBS curve's four spans in two pieces:
        // more stations than a `u32` counts.
        for (description, per_span) in [
            (cubic(CubicBasis::BSpline, 5, false), u32::MAX / 2 + 1),
            (cubic(CubicBasis::BSpline, 5, false), u32::MAX),
            (nurbs(true), u32::MAX / 4 + 1),
        ] {
            assert_eq!(
                RealizationPlan::new(
                    description,
                    PlanOptions::new(per_span),
                    &mut Workspace::new()
                )
                .unwrap_err(),
                RealizeError::TooLarge
            );
        }
    }

    /// On a 32-bit target, a single span of order `2^16` has extraction
    /// matrices of `2^32` entries, more than memory can address.
    #[cfg(target_pointer_width = "32")]
    #[test]
    fn extraction_larger_than_memory_is_refused() {
        let order = 1_usize << 16;
        let mut knots = vec![0.0; order];
        knots.extend(vec![1.0; order]);
        let curve = NurbsCurve::new(
            u32::try_from(order).unwrap(),
            knots,
            vec![[0.0; 3]; order],
            None,
        )
        .unwrap();
        let description = CurveDescription {
            centerline: Centerline::Nurbs(curve),
            widths: Attribute::Constant(0.1),
            normals: None,
            closed: false,
        };
        assert_eq!(
            RealizationPlan::new(description, PlanOptions::new(1), &mut Workspace::new())
                .unwrap_err(),
            RealizeError::TooLarge
        );
    }

    #[test]
    fn narrow_knot_intervals_far_from_zero_are_evaluated_at_their_own_u() {
        // `a + (b - a) / 2` is not representable: it rounds to `a` or `b`.
        let a = 1e16;
        let curve = NurbsCurve::new(
            2,
            vec![a, a, a + 2.0, a + 2.0],
            vec![[0.0; 3], [2.0, 0.0, 0.0]],
            None,
        )
        .unwrap();
        for widths in [
            Attribute::Vertex(vec![1.0, 3.0]),
            Attribute::Varying(vec![1.0, 3.0]),
        ] {
            let plan = plan(
                CurveDescription {
                    centerline: Centerline::Nurbs(curve.clone()),
                    widths,
                    normals: None,
                    closed: false,
                },
                2,
            );
            assert_eq!(plan.points()[1], [1.0, 0.0, 0.0]);
            assert_eq!(plan.tangents()[1], [1.0, 0.0, 0.0]);
            assert_eq!(plan.widths()[1], 2.0);
        }
    }

    #[test]
    fn signed_zeros_are_changes() {
        let mut description = cubic(CubicBasis::BSpline, 5, false);
        description.widths = Attribute::Constant(0.0);
        description.normals = Some(Attribute::Constant([0.0, 0.0, 1.0]));
        let mut plan = plan(description.clone(), 2);
        let work = plan.set_widths(Attribute::Constant(-0.0)).unwrap();
        assert_ne!(work, PlanWork::default());
        let work = plan
            .set_normals(Some(Attribute::Constant([-0.0, 0.0, 1.0])))
            .unwrap();
        assert_ne!(work, PlanWork::default());
        assert_eq!(plan.generations().widths, 1);
        assert_eq!(plan.generations().normals, 1);
        description.widths = Attribute::Constant(-0.0);
        description.normals = Some(Attribute::Constant([-0.0, 0.0, 1.0]));
        assert_same(&plan, &self::plan(description, 2));
    }

    #[test]
    fn points_and_weights_are_held_to_validation() {
        let mut workspace = Workspace::new();
        let n = nurbs(false).centerline.controls();
        let mut weights = vec![1.0; n];
        weights[4] = 1e-9;
        // A slight weight is fine, but not with a coordinate it would take
        // out of the normal range.
        let mut plan = plan(nurbs(false), 2);
        plan.set_weights(Some(&weights), &mut workspace).unwrap();
        let before = plan.clone();
        let mut points = wander(n);
        points[4][1] = 1e-300;
        assert_eq!(
            plan.set_points(&points, &mut workspace),
            Err(RealizeError::Value {
                field: Field::Points,
                index: 4
            })
        );
        assert_same(&plan, &before);
        // Nor the other way round.
        let mut description = nurbs(false);
        let Centerline::Nurbs(curve) = &mut description.centerline else {
            unreachable!()
        };
        curve.points[4][1] = 1e-300;
        let mut plan = self::plan(description, 2);
        assert_eq!(
            plan.set_weights(Some(&weights), &mut workspace),
            Err(RealizeError::Value {
                field: Field::Weights,
                index: 4
            })
        );
    }

    #[test]
    fn a_cusp_after_a_break_is_counted_in_the_plans_stations() {
        // Two quadratic pieces; the second doubles back at its middle,
        // where its tangent vanishes.
        let curve = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [5.0, 0.0, 0.0],
                [6.0, 0.0, 0.0],
                [5.0, 0.0, 0.0],
            ],
            None,
        )
        .unwrap();
        let plan = plan(
            CurveDescription {
                centerline: Centerline::Nurbs(curve),
                widths: Attribute::Constant(0.1),
                normals: None,
                closed: false,
            },
            2,
        );
        assert_eq!(plan.pieces().len(), 2);
        let frames = plan.report().frames;
        assert_eq!(frames.cusps, 1);
        assert_eq!(frames.first_cusp, Some(4));
    }
}
