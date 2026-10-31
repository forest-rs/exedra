// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A plan as plain arrays for a shader to tessellate.

use alloc::vec::Vec;
use core::ops::Range;

use exedra_math::{add, scale};

use super::{RealizationPlan, RealizeError};
use crate::vector::WeightedPoint;

/// One span's record in a [`GpuCurve`]: 16 bytes, four `u32`s with no
/// padding (`#[repr(C)]`, a `uvec4` in WGSL or GLSL `std430`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuSpan {
    /// The span's first control in [`GpuCurve::controls`], and its first
    /// coefficient in [`GpuCurve::widths`] and [`GpuCurve::normals`]:
    /// `span * order`.
    pub first_control: u32,
    /// The frame at the span's `u = 0` in [`GpuCurve::frames`]; the span's
    /// frames are this one and the next [`GpuCurve::frames_per_span`].
    pub first_frame: u32,
    /// The piece the span belongs to ([`super::RealizationPlan::pieces`]).
    pub piece: u32,
    /// [`Self::PIECE_START`], [`Self::PIECE_END`], [`Self::CREASE`] and
    /// [`Self::GAP`], or'ed.
    pub flags: u32,
}

impl GpuSpan {
    /// The span starts a piece: the curve's first span, or the first after
    /// a break. A strip of triangles starts here and does not join the span
    /// before.
    pub const PIECE_START: u32 = 1;
    /// The span ends a piece.
    pub const PIECE_END: u32 = 2;
    /// The joint at the span's start is a [crease](super#joints): the
    /// tangent turns there. For a closed curve's first span, the seam.
    pub const CREASE: u32 = 4;
    /// The joint at the span's start is a [gap](super#joints): the span
    /// does not start where the one before it ends.
    pub const GAP: u32 = 8;
}

/// A [`RealizationPlan`] as plain arrays, for a shader to tessellate on
/// the GPU: the analytic curve as per-span Bezier pieces, its widths and
/// ribbon normals as per-span polynomials, and its rotation-minimizing
/// frames, which are sequential to compute and so come from the CPU.
///
/// # Layout
///
/// Every array is in span order; `order` is [`Self::order`] (4 for a
/// cubic chain), `F` is [`Self::frames_per_span`].
///
/// | array | element | stride | count | holds |
/// |---|---|---|---|---|
/// | [`Self::spans`] | [`GpuSpan`] | 16 bytes | spans | where each span's data starts, its piece and flags |
/// | [`Self::controls`] | `[f32; 4]` | 16 bytes | `spans * order` | homogeneous Bezier controls `m (w (x - ox), w (y - oy), w (z - oz), w)` |
/// | [`Self::widths`] | `f32` | 4 bytes | `spans * order` | Bezier coefficients of the full width |
/// | [`Self::normals`] | `[f32; 4]` | 16 bytes | `spans * order`, or 0 | Bezier coefficients of the ribbon normal, `w = 0` |
/// | [`Self::frames`] | `[f32; 4]` | 16 bytes | `stations`, plus 1 when closed | the unit frame normal at each station, `w = 0` |
///
/// Positions and widths are in the curve's units; positions are relative
/// to [`Self::origin`], chosen by the caller, so `f32` keeps their
/// precision near it. `w` is the NURBS weight, positive (1 for a cubic
/// chain), and `m` a power of two per span that brings the span's largest
/// weight into `[1, 2)`, so weights far from 1 stay within `f32`; scaling
/// a span's homogeneous controls together moves neither its points nor
/// its tangents. Widths and normals are polynomial in `u`, not rational: a
/// curve's weights shape only its positions ([the module
/// documentation](super#attributes)).
///
/// Every `m w` is at least `2^-32`, and every control's point relative to
/// the origin (the span's points lie in their hull), width and normal
/// coefficient is at most `2^64` in magnitude, so the sums, differences
/// and quotients below stay far inside `f32`'s range. A shader normalizes
/// a vector by dividing it by its largest component first, as this crate
/// does, so a short vector's squared length cannot underflow. Points
/// within about `2^-94` of the origin may flush to it on a GPU that
/// flushes subnormals.
///
/// # Tessellating
///
/// Span `s` at `u` in `[0, 1]`, with `i` in `0..order`, `p = order - 1`
/// and `b = spans[s]`:
///
/// 1. **Position.** `H(u) = sum B_i(u) controls[b.first_control + i]` in
///    the Bernstein basis of degree `p` (de Casteljau's scheme), and
///    `P = H.xyz / H.w` is the point relative to the origin: the point is
///    `origin + P`.
/// 2. **Tangent.** The direction of `H'.xyz - P H'.w`, from the
///    derivative `H'` of the same scheme: the curve's derivative times
///    `H.w` (Piegl and Tiller, *The NURBS Book*, 2nd ed., equation 4.8),
///    with no product of two homogeneous values to underflow. Where it
///    vanishes, the curve stops and has no direction of its own; as the
///    CPU frames do (counting such stations in
///    [`FrameReport::estimated`](crate::FrameReport::estimated)), take the
///    bisector of the directions from the previous station's point and to
///    the next one's, or the one of them that exists.
/// 3. **Width.** `sum B_i(u) widths[b.first_control + i]`; a tube's
///    radius is half of it.
/// 4. **Frame.** With `k = min(floor(u F), F - 1)` and `f = u F - k`,
///    blend `frames[b.first_frame + k]` and `frames[b.first_frame + k + 1]`
///    by `f`, remove the part along the tangent and normalize: that is the
///    normal `N`; the binormal is `T x N`. At a station (`f = 0`) this is
///    the CPU frame up to `f32` rounding; between stations it follows the
///    rotation-minimizing frame to within the change of the tangent over
///    one station interval.
/// 5. **Section.** A tube's ring vertex at angle `a` is
///    `point + radius (cos a N + sin a B)`. A ribbon faces `N`, or the
///    normals' `sum B_i(u) normals[b.first_control + i]` made perpendicular
///    to the tangent (`N` again where that normal is parallel to the
///    tangent, as the CPU ribbon falls back,
///    [`SweepReport::unoriented`](crate::SweepReport::unoriented)), and
///    spans half the width each side along `T x facing`.
///
/// Spans flagged [`GpuSpan::PIECE_START`] start a new strip. A closed
/// curve has one piece; its last span's last frame is the first frame
/// carried once around the loop, so the seam's twist
/// ([`super::PlanReport::frames`]) appears there rather than inside a
/// span.
///
/// A caller reuses one through [`Default`] and
/// [`RealizationPlan::gpu_into`]. Non-exhaustive, so it can carry more
/// arrays later.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GpuCurve {
    /// Controls (and width and normal coefficients) per span: degree + 1.
    pub order: u32,
    /// Frame intervals per span, [`super::PlanOptions::per_span`].
    pub frames_per_span: u32,
    /// Whether the curve closes.
    pub closed: bool,
    /// The point positions are relative to.
    pub origin: [f64; 3],
    /// One record per span.
    pub spans: Vec<GpuSpan>,
    /// Homogeneous Bezier controls, `order` per span.
    pub controls: Vec<[f32; 4]>,
    /// Width coefficients, `order` per span.
    pub widths: Vec<f32>,
    /// Ribbon normal coefficients, `order` per span, or empty without
    /// normals.
    pub normals: Vec<[f32; 4]>,
    /// Frame normals at the stations, piece after piece, and for a closed
    /// curve the first frame carried around the loop.
    pub frames: Vec<[f32; 4]>,
}

/// Which parts of a [`GpuCurve`] [`RealizationPlan::gpu_into`] changed:
/// its header, and per array the range from the first changed element to
/// one past the last, `None` where nothing changed. A renderer that keeps
/// buffers uploads only these ranges, and its uniforms when the header
/// changed. The comparison is of the bits of
/// the `f32` values written, element by element, as they are written.
///
/// Non-exhaustive, so it can report more arrays later.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GpuReport {
    /// Whether [`GpuCurve::order`], [`GpuCurve::frames_per_span`],
    /// [`GpuCurve::closed`] or [`GpuCurve::origin`] changed (an origin by
    /// its bits).
    pub header: bool,
    /// Whether any array's length changed (as on a first fill, or a
    /// curve of another plan): its buffer is to be reallocated, and its
    /// range is the whole array.
    pub resized: bool,
    /// Changed [`GpuCurve::spans`].
    pub spans: Option<Range<usize>>,
    /// Changed [`GpuCurve::controls`].
    pub controls: Option<Range<usize>>,
    /// Changed [`GpuCurve::widths`].
    pub widths: Option<Range<usize>>,
    /// Changed [`GpuCurve::normals`].
    pub normals: Option<Range<usize>>,
    /// Changed [`GpuCurve::frames`].
    pub frames: Option<Range<usize>>,
}

impl GpuReport {
    /// Whether nothing changed, so there is nothing to upload.
    #[must_use]
    pub fn is_unchanged(&self) -> bool {
        *self == Self::default()
    }
}

impl RealizationPlan {
    /// Writes the plan into `out` as [`GpuCurve`]'s layout describes, with
    /// positions relative to `origin` (the center of [`Self::bounds`] is a
    /// good choice, kept while the curve moves), and reports what changed.
    /// `out`'s buffers are reused, and nothing is allocated once they have
    /// grown.
    ///
    /// # Errors
    ///
    /// [`RealizeError::F32Range`] for the first span whose data `f32`
    /// cannot hold; `out` is unchanged.
    pub fn gpu_into(
        &self,
        origin: [f64; 3],
        out: &mut GpuCurve,
    ) -> Result<GpuReport, RealizeError> {
        let order = self.order;
        // Every value checked before any is written, so a refused plan
        // leaves `out`, and what a renderer uploaded from it, as it was.
        let controls = |s: usize| self.gpu_controls(s, origin);
        let fits = |v: &[f32]| v.iter().all(|x| x.abs() <= LARGEST);
        // Each span's points lie in the hull of its controls' points, so
        // the controls as written, with their weights divided out, bound
        // every `P` a shader forms from them.
        let control_fits = |c: [f32; 4]| {
            c[3] >= LIGHTEST
                && c[..3]
                    .iter()
                    .all(|x| (f64::from(*x) / f64::from(c[3])).abs() <= f64::from(LARGEST))
        };
        let width = |c: &WeightedPoint<1>| narrow(c.weighted[0]);
        let normal = |c: &WeightedPoint<3>| {
            let [x, y, z] = c.weighted;
            [narrow(x), narrow(y), narrow(z), 0.0]
        };
        let normals = &self.normal_coefficients;
        if let Some(span) = (0..self.spans()).find(|&s| {
            let range = s * order..(s + 1) * order;
            !(controls(s).all(control_fits)
                && self.width_coefficients[range.clone()]
                    .iter()
                    .all(|c| fits(&[width(c)]))
                && normals
                    .get(range)
                    .unwrap_or_default()
                    .iter()
                    .all(|c| fits(&normal(c))))
        }) {
            return Err(RealizeError::F32Range { span });
        }
        let per_span = self.options.per_span;
        let closed = self.description.closed;
        let header = (
            u32::try_from(order).unwrap_or(u32::MAX),
            per_span,
            closed,
            origin.map(f64::to_bits),
        );
        let before = (
            out.order,
            out.frames_per_span,
            out.closed,
            out.origin.map(f64::to_bits),
        );
        (out.order, out.frames_per_span, out.closed) = (header.0, header.1, header.2);
        out.origin = origin;
        let mut report = GpuReport {
            header: header != before,
            ..GpuReport::default()
        };
        let spans = self.spans();
        report.spans = write(
            &mut out.spans,
            spans,
            (0..spans).map(|s| {
                let piece = self.pieces[self.span_piece(s) as usize];
                let mut flags = 0;
                if s == piece.first_span as usize {
                    flags |= GpuSpan::PIECE_START;
                }
                if s + 1 == (piece.first_span + piece.spans) as usize {
                    flags |= GpuSpan::PIECE_END;
                }
                if self.joints[s].crease {
                    flags |= GpuSpan::CREASE;
                }
                if self.joints[s].gap {
                    flags |= GpuSpan::GAP;
                }
                GpuSpan {
                    first_control: u32::try_from(s * order).unwrap_or(u32::MAX),
                    first_frame: self.span_first_station(s),
                    piece: self.span_piece(s),
                    flags,
                }
            }),
            &mut report.resized,
        );
        report.controls = write(
            &mut out.controls,
            self.bezier.len(),
            (0..spans).flat_map(controls),
            &mut report.resized,
        );
        report.widths = write(
            &mut out.widths,
            self.width_coefficients.len(),
            self.width_coefficients.iter().map(width),
            &mut report.resized,
        );
        report.normals = write(
            &mut out.normals,
            normals.len(),
            normals.iter().map(normal),
            &mut report.resized,
        );
        // A closed curve's last frame: the first carried around the loop,
        // its normal turned by the seam's twist about the first tangent.
        let around = closed.then(|| {
            let first = self.frames[0];
            let seam = self.report().frames;
            add(
                scale(first.normal, seam.seam_cos),
                scale(first.binormal, seam.seam_sin),
            )
        });
        report.frames = write(
            &mut out.frames,
            self.frames.len() + usize::from(closed),
            self.frames
                .iter()
                .map(|frame| frame.normal)
                .chain(around)
                .map(|[x, y, z]| [narrow(x), narrow(y), narrow(z), 0.0]),
            &mut report.resized,
        );
        Ok(report)
    }

    /// [`Self::gpu_into`] into a new [`GpuCurve`].
    ///
    /// # Errors
    ///
    /// As [`Self::gpu_into`].
    pub fn gpu(&self, origin: [f64; 3]) -> Result<GpuCurve, RealizeError> {
        let mut out = GpuCurve::default();
        self.gpu_into(origin, &mut out)?;
        Ok(out)
    }

    /// Span `s`'s homogeneous controls relative to `origin`, scaled by the
    /// power of two that brings its largest weight into `[1, 2)`, narrowed.
    fn gpu_controls(&self, s: usize, origin: [f64; 3]) -> impl Iterator<Item = [f32; 4]> {
        let controls = &self.bezier[s * self.order..(s + 1) * self.order];
        let heaviest = controls.iter().fold(0.0_f64, |m, c| m.max(c.weight));
        let m = unit_scale(heaviest);
        controls.iter().map(move |control| {
            // From the point relative to the origin, not `c - w origin`,
            // which far from zero cancels to its rounding.
            let w = control.weight * m;
            let point = control.point();
            [
                narrow((point[0] - origin[0]) * w),
                narrow((point[1] - origin[1]) * w),
                narrow((point[2] - origin[2]) * w),
                narrow(w),
            ]
        })
    }
}

/// The lightest scaled weight a span may have, `2^-32`: a span's weights
/// differ by at most `2^33`, so homogeneous coordinates stay within that
/// of the curve's own.
const LIGHTEST: f32 = f32::from_bits(0x2F80_0000);

/// The largest magnitude of a point relative to the origin, a width or a
/// normal coefficient, `2^64`: with [`LIGHTEST`], what a shader forms
/// from them stays below `2^100` for any order a plan can count.
const LARGEST: f32 = f32::from_bits(0x5F80_0000);

/// The power of two that brings the positive normal `x` into `[1, 2)`:
/// `x`'s exponent negated. Plan weights lie within `[2^-256, 2^256]`, so
/// it is normal too, and multiplying by it rounds nothing that does not
/// over- or underflow.
fn unit_scale(x: f64) -> f64 {
    let exponent = (x.to_bits() >> 52) & 0x7FF;
    f64::from_bits((2046 - exponent) << 52)
}

/// `x` rounded to the nearest `f32`, as the GPU arrays hold it.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the GPU layout is f32 by design; rounding to nearest is the conversion"
)]
fn narrow(x: f64) -> f32 {
    x as f32
}

/// Writes `len` `values` into `out`, comparing each with what is there,
/// and returns the range that changed; a change of length sets `resized`
/// and changes the whole array.
fn write<T: Copy + Bits>(
    out: &mut Vec<T>,
    len: usize,
    values: impl Iterator<Item = T>,
    resized: &mut bool,
) -> Option<Range<usize>> {
    if out.len() != len {
        *resized = true;
        out.clear();
        out.extend(values);
        return (len > 0).then_some(0..len);
    }
    let mut changed: Option<Range<usize>> = None;
    for (i, (slot, value)) in out.iter_mut().zip(values).enumerate() {
        if !slot.same(&value) {
            *slot = value;
            changed = Some(changed.map_or(i..i + 1, |r| r.start..i + 1));
        }
    }
    changed
}

/// A GPU element compared by its bits, so a sign of zero or a NaN payload
/// changing is a change.
trait Bits {
    fn same(&self, other: &Self) -> bool;
}

impl Bits for GpuSpan {
    fn same(&self, other: &Self) -> bool {
        self == other
    }
}

impl Bits for f32 {
    fn same(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl Bits for [f32; 4] {
    fn same(&self, other: &Self) -> bool {
        self.iter().zip(other).all(|(a, b)| a.same(b))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use exedra_math::{cross, dot, norm, normalize, sub};

    use super::super::{Attribute, Centerline, CurveDescription, PlanOptions};
    use super::*;
    use crate::{CubicBasis, NurbsCurve, Workspace};

    /// What a shader computes at span `s` and `u`, following
    /// [`GpuCurve`]'s documented steps and reading nothing else: the
    /// point, tangent direction, width and frame normal.
    struct Tessellated {
        point: [f64; 3],
        tangent: [f64; 3],
        width: f64,
        normal: [f64; 3],
    }

    /// Bernstein combination by de Casteljau's scheme down to two points:
    /// the value at `u` and `p` times the difference, the derivative.
    fn casteljau<const D: usize>(controls: &[[f64; D]], u: f64) -> ([f64; D], [f64; D]) {
        let mut level = controls.to_vec();
        while level.len() > 2 {
            for j in 0..level.len() - 1 {
                level[j] = core::array::from_fn(|c| (1.0 - u) * level[j][c] + u * level[j + 1][c]);
            }
            level.pop();
        }
        let p = (controls.len() - 1) as f64;
        let value = core::array::from_fn(|c| (1.0 - u) * level[0][c] + u * level[1][c]);
        (
            value,
            core::array::from_fn(|c| p * (level[1][c] - level[0][c])),
        )
    }

    fn tessellate(gpu: &GpuCurve, s: usize, u: f64) -> Tessellated {
        let order = gpu.order as usize;
        let span = gpu.spans[s];
        let first = span.first_control as usize;
        let controls: Vec<[f64; 4]> = gpu.controls[first..first + order]
            .iter()
            .map(|c| c.map(f64::from))
            .collect();
        let (h, d) = casteljau(&controls, u);
        let relative: [f64; 3] = core::array::from_fn(|c| h[c] / h[3]);
        let point = core::array::from_fn(|c| gpu.origin[c] + relative[c]);
        let tangent = core::array::from_fn(|c| d[c] - relative[c] * d[3]);
        let widths: Vec<[f64; 1]> = gpu.widths[first..first + order]
            .iter()
            .map(|w| [f64::from(*w)])
            .collect();
        let width = casteljau(&widths, u).0[0];
        let f_count = gpu.frames_per_span;
        let k = (0..f_count)
            .rev()
            .find(|&k| f64::from(k) <= u * f64::from(f_count))
            .unwrap_or(0);
        let f = u * f64::from(f_count) - f64::from(k);
        let frame = |i: u32| gpu.frames[(span.first_frame + i) as usize].map(f64::from);
        let (a, b) = (frame(k), frame(k + 1));
        let blend: [f64; 3] = core::array::from_fn(|c| (1.0 - f) * a[c] + f * b[c]);
        // Where the tangent vanishes, the caller supplies one; the blend
        // stands in here.
        let normal = normalize(tangent)
            .and_then(|t| normalize(sub(blend, scale(t, dot(blend, t)))))
            .unwrap_or(blend);
        Tessellated {
            point,
            tangent,
            width,
            normal,
        }
    }

    /// The centerline's point and derivative at span `s` and `u`, from
    /// the crate's own evaluators rather than the plan.
    fn analytic(description: &CurveDescription, s: usize, u: f64) -> ([f64; 3], [f64; 3]) {
        match &description.centerline {
            Centerline::Cubic {
                basis,
                points,
                spans,
            } => {
                let controls = spans[s].map(|c| points[c as usize]);
                (basis.evaluate(controls, u), basis.derivative(controls, u))
            }
            Centerline::Nurbs(curve) => {
                let prepared = curve.view().prepare();
                let [a, b] = prepared.span_domain(s).unwrap();
                let e = prepared.evaluation_in(s, a + (b - a) * u).unwrap();
                (e.point, e.first)
            }
        }
    }

    fn wander(n: usize) -> Vec<[f64; 3]> {
        (0..n)
            .map(|i| {
                [
                    100.0 + i as f64,
                    ((i * 7) % 5) as f64 * 0.3,
                    ((i * 3) % 4) as f64 * 0.2,
                ]
            })
            .collect()
    }

    fn descriptions() -> [CurveDescription; 4] {
        let knots = vec![
            0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 3.0, 3.0, 3.0, 3.0, 4.0, 4.0, 4.0, 4.0,
        ];
        let weights = (0..11).map(|i| 1.0 + 0.5 * (i % 3) as f64).collect();
        let nurbs = NurbsCurve::new(4, knots, wander(11), Some(weights)).unwrap();
        [
            CurveDescription {
                centerline: Centerline::Nurbs(nurbs),
                widths: Attribute::Vertex((0..11).map(|i| 0.1 + 0.01 * i as f64).collect()),
                normals: None,
                closed: false,
            },
            CurveDescription {
                centerline: Centerline::Cubic {
                    basis: CubicBasis::CatmullRom,
                    points: wander(8),
                    spans: (0..5).map(|i| [i, i + 1, i + 2, i + 3]).collect(),
                },
                widths: Attribute::Varying(vec![0.1, 0.2, 0.3, 0.2, 0.1, 0.3]),
                normals: Some(Attribute::Constant([0.0, 1.0, 1.0])),
                closed: false,
            },
            CurveDescription {
                centerline: Centerline::Cubic {
                    basis: CubicBasis::BSpline,
                    points: wander(6),
                    spans: (0..6)
                        .map(|i| core::array::from_fn(|k| (i + u32::try_from(k).unwrap()) % 6))
                        .collect(),
                },
                widths: Attribute::Constant(0.2),
                normals: None,
                closed: true,
            },
            // Hermite: interleaved points and tangents, step 2.
            CurveDescription {
                centerline: Centerline::Cubic {
                    basis: CubicBasis::Hermite,
                    points: vec![
                        [100.0, 0.0, 0.0],
                        [1.0, 2.0, 0.0],
                        [101.0, 1.0, 0.5],
                        [2.0, -1.0, 1.0],
                        [103.0, 0.0, 0.0],
                        [0.5, 0.5, -2.0],
                    ],
                    spans: vec![[0, 1, 2, 3], [2, 3, 4, 5]],
                },
                widths: Attribute::Vertex(vec![0.1, 0.0, 0.2, 0.0, 0.3, 0.0]),
                normals: None,
                closed: false,
            },
        ]
    }

    #[test]
    fn the_gpu_data_tessellates_to_the_curve_and_its_cpu_frames() {
        for description in descriptions() {
            let plan = RealizationPlan::new(
                description.clone(),
                PlanOptions::new(4),
                &mut Workspace::new(),
            )
            .unwrap();
            let [lo, hi] = plan.bounds();
            let origin = core::array::from_fn(|c| 0.5 * (lo[c] + hi[c]));
            let gpu = plan.gpu(origin).unwrap();
            assert_eq!(gpu.spans.len(), plan.spans());
            assert_eq!(gpu.controls.len(), plan.spans() * gpu.order as usize);
            assert_eq!(
                gpu.frames.len(),
                plan.frames().len() + usize::from(description.closed)
            );
            // f32 positions near the origin: a few units in the last place
            // of the curve's extent.
            let tolerance = 1e-6;
            // At every station: the CPU point, width, tangent and frame.
            for (i, station) in plan.stations().iter().enumerate() {
                let at = tessellate(&gpu, station.span as usize, station.u);
                assert!(norm(sub(at.point, plan.points()[i])) < tolerance);
                assert!((at.width - plan.widths()[i]).abs() < tolerance);
                let t = normalize(at.tangent).unwrap();
                assert!(norm(sub(t, plan.frames()[i].tangent)) < tolerance);
                assert!(norm(sub(at.normal, plan.frames()[i].normal)) < tolerance);
            }
            // Between stations: on the curve, and with a normal near both
            // neighbouring frames'.
            for s in 0..plan.spans() {
                for k in 0..16 {
                    let u = (f64::from(k) + 0.5) / 16.0;
                    let at = tessellate(&gpu, s, u);
                    let (point, tangent) = analytic(&description, s, u);
                    assert!(norm(sub(at.point, point)) < tolerance, "span {s} u {u}");
                    let (a, b) = (normalize(at.tangent).unwrap(), normalize(tangent).unwrap());
                    assert!(norm(cross(a, b)) < tolerance);
                    assert!(dot(at.normal, a).abs() < tolerance);
                }
            }
            // A tube's ring vertex from the GPU data is the CPU tube's.
            let sides = 6;
            let mesh = plan.tube(sides).unwrap();
            let first_piece_station = plan.stations()[3];
            let at = tessellate(
                &gpu,
                first_piece_station.span as usize,
                first_piece_station.u,
            );
            let binormal = normalize(cross(at.tangent, at.normal)).unwrap();
            let (sin, cos) = exedra_math::Real::sin_cos(core::f64::consts::TAU * 2.0 / 6.0);
            let vertex = add(
                at.point,
                scale(
                    add(scale(at.normal, cos), scale(binormal, sin)),
                    at.width * 0.5,
                ),
            );
            assert!(norm(sub(vertex, mesh.positions[3 * sides as usize + 2])) < tolerance);
        }
    }

    #[test]
    fn a_closed_curve_ends_on_its_first_frame_carried_around() {
        let plan = RealizationPlan::new(
            descriptions()[2].clone(),
            PlanOptions::new(3),
            &mut Workspace::new(),
        )
        .unwrap();
        let gpu = plan.gpu([0.0; 3]).unwrap();
        let last = gpu.spans.last().unwrap();
        assert_eq!(last.first_frame as usize + 3, gpu.frames.len() - 1);
        let seam = plan.report().frames;
        let first = plan.frames()[0];
        let expected = add(
            scale(first.normal, seam.seam_cos),
            scale(first.binormal, seam.seam_sin),
        );
        let end = gpu.frames[gpu.frames.len() - 1].map(f64::from);
        assert!(norm(sub([end[0], end[1], end[2]], expected)) < 1e-6);
        assert_eq!(gpu.spans[0].flags, GpuSpan::PIECE_START);
    }

    #[test]
    fn rewriting_reports_only_what_changed() {
        let mut workspace = Workspace::new();
        let mut plan = RealizationPlan::new(
            descriptions()[1].clone(),
            PlanOptions::new(4),
            &mut workspace,
        )
        .unwrap();
        let mut gpu = GpuCurve::default();
        let first = plan.gpu_into([100.0, 0.0, 0.0], &mut gpu).unwrap();
        assert!(first.resized);
        assert_eq!(first.controls, Some(0..20));
        assert!(
            plan.gpu_into([100.0, 0.0, 0.0], &mut gpu)
                .unwrap()
                .is_unchanged()
        );
        // Control 7 shapes only the last span (span 4), and of its Bezier
        // controls only the third, `C2 - (C3 - C1) / 6`; the frames change
        // from that span's first station on.
        let mut points = wander(8);
        points[7][2] += 1.0;
        plan.set_points(&points, &mut workspace).unwrap();
        let report = plan.gpu_into([100.0, 0.0, 0.0], &mut gpu).unwrap();
        assert!(!report.resized);
        assert_eq!(report.spans, None);
        assert_eq!(report.controls, Some(18..19));
        assert_eq!(report.widths, None);
        assert_eq!(report.normals, None);
        let frames = report.frames.unwrap();
        assert!(frames.start >= 16 && frames.end == 21, "{frames:?}");
        // New widths: only the width coefficients.
        plan.set_widths(Attribute::Constant(0.5)).unwrap();
        let report = plan.gpu_into([100.0, 0.0, 0.0], &mut gpu).unwrap();
        assert_eq!(
            report,
            GpuReport {
                widths: Some(0..20),
                ..GpuReport::default()
            }
        );
        assert_eq!(gpu, plan.gpu([100.0, 0.0, 0.0]).unwrap());
    }

    fn line(weights: [f64; 2]) -> CurveDescription {
        CurveDescription {
            centerline: Centerline::Nurbs(
                NurbsCurve::new(
                    2,
                    vec![0.0, 0.0, 1.0, 1.0],
                    vec![[0.0; 3], [2.0, 0.0, 0.0]],
                    Some(weights.into()),
                )
                .unwrap(),
            ),
            widths: Attribute::Constant(0.1),
            normals: None,
            closed: false,
        }
    }

    #[test]
    fn the_weights_validation_accepts_fit_f32() {
        let light = libm::scalbn(1.0, -256);
        let heavy = libm::scalbn(1.0, 256);
        for weights in [
            [light; 2],
            [heavy; 2],
            [light, 2.0 * light],
            [heavy, heavy / 3.0],
        ] {
            let plan =
                RealizationPlan::new(line(weights), PlanOptions::new(2), &mut Workspace::new())
                    .unwrap();
            let gpu = plan.gpu([0.0; 3]).unwrap();
            for (i, station) in plan.stations().iter().enumerate() {
                let at = tessellate(&gpu, station.span as usize, station.u);
                assert!(norm(sub(at.point, plan.points()[i])) < 1e-6, "{weights:?}");
            }
        }
    }

    #[test]
    fn data_f32_cannot_hold_is_refused_and_leaves_the_buffers() {
        let mut workspace = Workspace::new();
        let fine =
            RealizationPlan::new(line([1.0; 2]), PlanOptions::new(2), &mut workspace).unwrap();
        let mut gpu = fine.gpu([0.0; 3]).unwrap();
        let before = gpu.clone();
        let spread = [libm::scalbn(1.0, -256), libm::scalbn(1.0, 256)];
        let plan = RealizationPlan::new(line(spread), PlanOptions::new(2), &mut workspace).unwrap();
        assert_eq!(
            plan.gpu_into([0.0; 3], &mut gpu),
            Err(RealizeError::F32Range { span: 0 })
        );
        assert_eq!(
            fine.gpu_into([1e300, 0.0, 0.0], &mut gpu),
            Err(RealizeError::F32Range { span: 0 })
        );
        let mut wide = line([1.0; 2]);
        wide.widths = Attribute::Constant(1e300);
        let plan = RealizationPlan::new(wide, PlanOptions::new(2), &mut workspace).unwrap();
        assert_eq!(
            plan.gpu_into([0.0; 3], &mut gpu),
            Err(RealizeError::F32Range { span: 0 })
        );
        assert_eq!(gpu, before);
    }

    #[test]
    fn a_change_of_sign_of_zero_is_reported() {
        let mut workspace = Workspace::new();
        let mut description = descriptions()[1].clone();
        description.normals = Some(Attribute::Constant([1e-50, 1.0, 1.0]));
        let mut plan =
            RealizationPlan::new(description, PlanOptions::new(2), &mut workspace).unwrap();
        let mut gpu = plan.gpu([0.0; 3]).unwrap();
        plan.set_normals(Some(Attribute::Constant([-1e-50, 1.0, 1.0])))
            .unwrap();
        let report = plan.gpu_into([0.0; 3], &mut gpu).unwrap();
        assert_eq!(report.normals, Some(0..20));
        assert_eq!(gpu.normals[0][0].to_bits(), (-0.0_f32).to_bits());
    }

    #[test]
    fn the_tangent_recipe_holds_in_f32_for_light_weights() {
        // A quadratic whose last two weights are `2^-30` of the first: the
        // derivative at its end is `(2, 0, 0)`.
        let light = libm::scalbn(1.0, -30);
        let curve = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            Some(vec![1.0, light, light]),
        )
        .unwrap();
        let mut description = line([1.0; 2]);
        description.centerline = Centerline::Nurbs(curve);
        let plan =
            RealizationPlan::new(description, PlanOptions::new(2), &mut Workspace::new()).unwrap();
        let gpu = plan.gpu([0.0; 3]).unwrap();
        // The documented steps, in `f32` as a shader runs them.
        let mut level = gpu.controls.clone();
        let u = 1.0_f32;
        while level.len() > 2 {
            for j in 0..level.len() - 1 {
                level[j] = core::array::from_fn(|c| (1.0 - u) * level[j][c] + u * level[j + 1][c]);
            }
            level.pop();
        }
        let h: [f32; 4] = core::array::from_fn(|c| (1.0 - u) * level[0][c] + u * level[1][c]);
        let d: [f32; 4] = core::array::from_fn(|c| 2.0 * (level[1][c] - level[0][c]));
        let p: [f32; 3] = core::array::from_fn(|c| h[c] / h[3]);
        let t: [f32; 3] = core::array::from_fn(|c| d[c] - p[c] * d[3]);
        assert_eq!(p, [2.0, 0.0, 0.0]);
        assert!(t[0] > 0.0 && t[1] == 0.0 && t[2] == 0.0, "{t:?}");
        // Weights `2^-40` apart are refused rather than left to underflow.
        let lighter = libm::scalbn(1.0, -40);
        let curve = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            Some(vec![1.0, lighter, lighter]),
        )
        .unwrap();
        let mut description = line([1.0; 2]);
        description.centerline = Centerline::Nurbs(curve);
        let plan =
            RealizationPlan::new(description, PlanOptions::new(2), &mut Workspace::new()).unwrap();
        assert_eq!(plan.gpu([0.0; 3]), Err(RealizeError::F32Range { span: 0 }));
    }

    #[test]
    fn points_a_shader_cannot_difference_are_refused() {
        // Both ends fit `f32`, but their difference, the derivative, would
        // not.
        let mut description = line([1.0; 2]);
        let Centerline::Nurbs(curve) = &mut description.centerline else {
            unreachable!()
        };
        curve.points = vec![[-1e38, 0.0, 0.0], [1e38, 0.0, 0.0]];
        let plan =
            RealizationPlan::new(description, PlanOptions::new(2), &mut Workspace::new()).unwrap();
        assert_eq!(plan.gpu([0.0; 3]), Err(RealizeError::F32Range { span: 0 }));
    }

    #[test]
    fn a_new_origin_is_a_header_change() {
        let plan = RealizationPlan::new(
            descriptions()[1].clone(),
            PlanOptions::new(2),
            &mut Workspace::new(),
        )
        .unwrap();
        let mut gpu = plan.gpu([0.0; 3]).unwrap();
        assert!(plan.gpu_into([0.0; 3], &mut gpu).unwrap().is_unchanged());
        // Too slight a move to change any `f32`, but the origin is new.
        let report = plan.gpu_into([1e-9, 0.0, 0.0], &mut gpu).unwrap();
        assert_eq!(
            report,
            GpuReport {
                header: true,
                ..GpuReport::default()
            }
        );
        assert!(!report.is_unchanged());
    }

    #[test]
    fn a_stopping_curve_takes_its_tangent_from_its_stations() {
        // A cubic Bezier whose first two controls coincide: its derivative
        // vanishes at its start, where the CPU frames take the direction to
        // the next station.
        let description = CurveDescription {
            centerline: Centerline::Cubic {
                basis: CubicBasis::Bezier,
                points: vec![[0.0; 3], [0.0; 3], [1.0, 1.0, 0.0], [2.0, 0.0, 0.0]],
                spans: vec![[0, 1, 2, 3]],
            },
            widths: Attribute::Constant(0.1),
            normals: None,
            closed: false,
        };
        let plan =
            RealizationPlan::new(description, PlanOptions::new(4), &mut Workspace::new()).unwrap();
        assert_eq!(plan.report().frames.estimated, 1);
        let gpu = plan.gpu([0.0; 3]).unwrap();
        let at = tessellate(&gpu, 0, 0.0);
        assert_eq!(at.tangent, [0.0; 3]);
        // The documented fallback: only a next station exists.
        let next = tessellate(&gpu, 0, 0.25);
        let fallback = normalize(sub(next.point, at.point)).unwrap();
        assert!(norm(sub(fallback, plan.frames()[0].tangent)) < 1e-6);
    }

    #[test]
    fn controls_far_from_zero_are_written_relative_to_the_origin() {
        // Four equal points near 2^179 with uneven weights: forming
        // `c - w origin` cancels to rounding as large as 2^127, which a
        // shader's derivative would overflow. Relative to the origin they
        // are all at it, or refused; never written as noise.
        let x = f64::from_bits(0x4b20_b21f_ba9d_f792);
        let a = f64::from_bits(0x3ffb_410d_93b2_5495);
        let b = f64::from_bits(0x3fff_bb23_0b3a_c878);
        let curve = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0],
            vec![[x, 0.0, 0.0]; 4],
            Some(vec![a, a, b, b]),
        )
        .unwrap();
        let mut description = line([1.0; 2]);
        description.centerline = Centerline::Nurbs(curve);
        let plan =
            RealizationPlan::new(description, PlanOptions::new(2), &mut Workspace::new()).unwrap();
        match plan.gpu([x, 0.0, 0.0]) {
            Ok(gpu) => {
                for c in &gpu.controls {
                    for x in &c[..3] {
                        assert!((f64::from(*x) / f64::from(c[3])).abs() <= 1.8e19, "{c:?}");
                    }
                }
            }
            Err(error) => assert!(matches!(error, RealizeError::F32Range { .. })),
        }
    }
}
