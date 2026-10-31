// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Spans of controls, and where a chain of them is sampled.

use alloc::vec::Vec;

use crate::CubicBasis;

/// One cubic span: the indices of its four controls, in order, into the
/// chain's control array. A caller builds spans from its own indexing rules
/// (stepping, wrapping, repeated or reflected end controls); this crate
/// imposes none.
pub type Span = [u32; 4];

/// A place a chain is sampled: a span and the parameter in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Station {
    /// The span, an index into the chain's spans.
    pub span: u32,
    /// The parameter in the span, in `[0, 1]`.
    pub u: f64,
}

/// The stations of a chain of `spans` spans sampled `per_span` times each.
///
/// Span `s` is sampled at `u = k / per_span` for `k` in `0..per_span`
/// (each quotient computed as `f64` division, so the parameters are exact
/// for powers of two), and an open chain also at the end of its last span
/// (`u = 1`): `spans * per_span + 1` stations, or `spans * per_span` for a
/// closed chain, whose last station joins its first. Adjacent spans of the
/// [`CubicBasis`] chains meet (a Bezier chain shares its joint control, a
/// B-spline or Catmull-Rom chain is continuous by construction), so the end
/// of one span is the start of the next and is sampled once.
///
/// `per_span` of 0 is taken as 1. No stations for no spans.
#[must_use]
pub fn stations(spans: u32, per_span: u32, closed: bool) -> Vec<Station> {
    let mut out = Vec::new();
    stations_into(spans, per_span, closed, &mut out);
    out
}

/// [`stations`] into a caller's buffer, which is cleared first, so a
/// retained buffer is reused without allocating once it is large enough.
pub fn stations_into(spans: u32, per_span: u32, closed: bool, out: &mut Vec<Station>) {
    out.clear();
    let per_span = per_span.max(1);
    if spans == 0 {
        return;
    }
    // Counted in `usize`, where two `u32` factors cannot overflow on a
    // 64-bit target; on a 32-bit one a count past `usize` saturates and
    // the allocation fails as any `Vec`'s does.
    let count = (spans as usize)
        .saturating_mul(per_span as usize)
        .saturating_add(1);
    out.reserve(count);
    for span in 0..spans {
        for k in 0..per_span {
            out.push(Station {
                span,
                u: f64::from(k) / f64::from(per_span),
            });
        }
    }
    if !closed {
        out.push(Station {
            span: spans - 1,
            u: 1.0,
        });
    }
}

/// A span names a control the chain does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlOutOfRange {
    /// The span.
    pub span: u32,
    /// The control index it names.
    pub control: u32,
    /// How many controls there are.
    pub controls: u32,
}

impl core::fmt::Display for ControlOutOfRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "span {} names control {} of {}",
            self.span, self.control, self.controls
        )
    }
}

impl core::error::Error for ControlOutOfRange {}

/// Evaluates a chain at its stations: each station's span's four controls
/// weighted by `basis` at the station's `u`.
///
/// Controls may be of any dimension: sample positions and per-control
/// values (widths, colors) with the same spans and stations.
///
/// Values are not checked for finiteness: a non-finite control or `u`
/// gives non-finite samples where it is used. Sampling is evaluation, run
/// every time a curve is drawn, over values of any meaning, so it does not
/// scan its inputs on every call; a reader checks authored data once,
/// where it is read.
///
/// # Errors
///
/// [`ControlOutOfRange`] when a sampled span names a control past
/// `controls`. A station naming a span past `spans` is an error too, as a
/// control out of range of an empty span (`control: u32::MAX`).
pub fn sample<const N: usize>(
    basis: CubicBasis,
    controls: &[[f64; N]],
    spans: &[Span],
    stations: &[Station],
) -> Result<Vec<[f64; N]>, ControlOutOfRange> {
    let mut out = Vec::new();
    sample_into(basis, controls, spans, stations, &mut out)?;
    Ok(out)
}

/// [`sample()`] into a caller's buffer, which is cleared first.
///
/// # Errors
///
/// As [`sample()`]; `out` is left empty.
pub fn sample_into<const N: usize>(
    basis: CubicBasis,
    controls: &[[f64; N]],
    spans: &[Span],
    stations: &[Station],
    out: &mut Vec<[f64; N]>,
) -> Result<(), ControlOutOfRange> {
    sample_with(controls, spans, stations, out, |picked, u| {
        basis.evaluate(picked, u)
    })
}

/// The chain's derivative `dP/du` at its stations, with respect to each
/// station's span parameter `u` ([`CubicBasis::derivative`]): the analytic
/// tangents of a sampled chain, for framing it without estimating them
/// from chords.
///
/// At a station where two spans meet the derivative is the station's own
/// span's, so a chain that is only `C0` there (a Bezier chain with
/// unaligned handles) gives the derivative of the span the station names.
///
/// # Errors
///
/// As [`sample()`].
pub fn sample_derivative<const N: usize>(
    basis: CubicBasis,
    controls: &[[f64; N]],
    spans: &[Span],
    stations: &[Station],
) -> Result<Vec<[f64; N]>, ControlOutOfRange> {
    let mut out = Vec::new();
    sample_derivative_into(basis, controls, spans, stations, &mut out)?;
    Ok(out)
}

/// [`sample_derivative`] into a caller's buffer, which is cleared first.
///
/// # Errors
///
/// As [`sample()`]; `out` is left empty.
pub fn sample_derivative_into<const N: usize>(
    basis: CubicBasis,
    controls: &[[f64; N]],
    spans: &[Span],
    stations: &[Station],
    out: &mut Vec<[f64; N]>,
) -> Result<(), ControlOutOfRange> {
    sample_with(controls, spans, stations, out, |picked, u| {
        basis.derivative(picked, u)
    })
}

/// Gathers each station's span's controls and pushes `value(controls, u)`.
fn sample_with<const N: usize>(
    controls: &[[f64; N]],
    spans: &[Span],
    stations: &[Station],
    out: &mut Vec<[f64; N]>,
    value: impl Fn([[f64; N]; 4], f64) -> [f64; N],
) -> Result<(), ControlOutOfRange> {
    out.clear();
    out.reserve(stations.len());
    for station in stations {
        match span_controls(controls, spans, station.span) {
            Ok(picked) => out.push(value(picked, station.u)),
            Err(error) => {
                out.clear();
                return Err(error);
            }
        }
    }
    Ok(())
}

/// The four controls of span `span`.
pub(crate) fn span_controls<const N: usize>(
    controls: &[[f64; N]],
    spans: &[Span],
    span: u32,
) -> Result<[[f64; N]; 4], ControlOutOfRange> {
    let count = u32::try_from(controls.len()).unwrap_or(u32::MAX);
    let Some(indices) = spans.get(span as usize) else {
        return Err(ControlOutOfRange {
            span,
            control: u32::MAX,
            controls: count,
        });
    };
    let mut picked = [[0.0; N]; 4];
    for (slot, &control) in picked.iter_mut().zip(indices) {
        *slot = *controls.get(control as usize).ok_or(ControlOutOfRange {
            span,
            control,
            controls: count,
        })?;
    }
    Ok(picked)
}

/// Linearly interpolates per-joint values at a chain's stations: span `s`
/// runs from `values[joints[s][0]]` at `u = 0` to `values[joints[s][1]]` at
/// `u = 1`, as `(1 - u) * a + u * b`.
///
/// This is how data given once per span boundary (`OpenUSD`'s `varying`
/// curve primvars) follows a cubic chain.
///
/// # Errors
///
/// [`ControlOutOfRange`] when a joint index is past `values`, or a station
/// names a span past `joints`.
pub fn sample_linear<const N: usize>(
    values: &[[f64; N]],
    joints: &[[u32; 2]],
    stations: &[Station],
) -> Result<Vec<[f64; N]>, ControlOutOfRange> {
    let mut out = Vec::new();
    sample_linear_into(values, joints, stations, &mut out)?;
    Ok(out)
}

/// [`sample_linear`] into a caller's buffer, which is cleared first.
///
/// # Errors
///
/// As [`sample_linear`]; `out` is left empty.
pub fn sample_linear_into<const N: usize>(
    values: &[[f64; N]],
    joints: &[[u32; 2]],
    stations: &[Station],
    out: &mut Vec<[f64; N]>,
) -> Result<(), ControlOutOfRange> {
    let result = fill_linear(values, joints, stations, out);
    if result.is_err() {
        out.clear();
    }
    result
}

fn fill_linear<const N: usize>(
    values: &[[f64; N]],
    joints: &[[u32; 2]],
    stations: &[Station],
    out: &mut Vec<[f64; N]>,
) -> Result<(), ControlOutOfRange> {
    out.clear();
    out.reserve(stations.len());
    let count = u32::try_from(values.len()).unwrap_or(u32::MAX);
    for station in stations {
        let Some(&[a, b]) = joints.get(station.span as usize) else {
            return Err(ControlOutOfRange {
                span: station.span,
                control: u32::MAX,
                controls: count,
            });
        };
        let value = |index: u32| {
            values
                .get(index as usize)
                .copied()
                .ok_or(ControlOutOfRange {
                    span: station.span,
                    control: index,
                    controls: count,
                })
        };
        let (a, b) = (value(a)?, value(b)?);
        let u = station.u;
        out.push(core::array::from_fn(|k| (1.0 - u) * a[k] + u * b[k]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stations_of_open_and_closed_chains() {
        let open = stations(2, 4, false);
        assert_eq!(open.len(), 9);
        assert_eq!(open[4], Station { span: 1, u: 0.0 });
        assert_eq!(open[8], Station { span: 1, u: 1.0 });
        assert_eq!(stations(3, 2, true).len(), 6);
        assert!(stations(0, 4, false).is_empty());
        assert_eq!(stations(1, 0, false).len(), 2);
    }

    #[test]
    fn a_bezier_chain_meets_at_its_shared_control() {
        let controls: Vec<[f64; 2]> = (0..7).map(|i| [f64::from(i), 0.0]).collect();
        let spans = [[0, 1, 2, 3], [3, 4, 5, 6]];
        let at = stations(2, 2, false);
        let points = sample(CubicBasis::Bezier, &controls, &spans, &at).unwrap();
        assert_eq!(points.len(), 5);
        assert_eq!(points[0], [0.0, 0.0]);
        assert_eq!(points[2], [3.0, 0.0]);
        assert_eq!(points[4], [6.0, 0.0]);
        assert_eq!(
            sample(CubicBasis::Bezier, &controls[..6], &spans, &at),
            Err(ControlOutOfRange {
                span: 1,
                control: 6,
                controls: 6
            })
        );
    }

    #[test]
    fn varying_values_follow_the_joints() {
        let values = [[0.0], [2.0], [4.0]];
        let at = stations(2, 2, false);
        let out = sample_linear(&values, &[[0, 1], [1, 2]], &at).unwrap();
        assert_eq!(out, [[0.0], [1.0], [2.0], [3.0], [4.0]]);
    }

    #[test]
    fn buffers_are_reused_and_derivatives_follow_the_stations() {
        let controls: Vec<[f64; 1]> = (0..5).map(|i| [f64::from(i * i)]).collect();
        let spans = [[0, 1, 2, 3], [1, 2, 3, 4]];
        let mut at = Vec::new();
        stations_into(2, 4, false, &mut at);
        assert_eq!(at, stations(2, 4, false));
        let mut out = Vec::new();
        sample_into(CubicBasis::CatmullRom, &controls, &spans, &at, &mut out).unwrap();
        let capacity = out.capacity();
        sample_into(CubicBasis::CatmullRom, &controls, &spans, &at, &mut out).unwrap();
        assert_eq!(out.capacity(), capacity);
        assert_eq!(
            out,
            sample(CubicBasis::CatmullRom, &controls, &spans, &at).unwrap()
        );
        let derivative = sample_derivative(CubicBasis::CatmullRom, &controls, &spans, &at).unwrap();
        for (station, d) in at.iter().zip(&derivative) {
            let c = span_controls(&controls, &spans, station.span).unwrap();
            assert_eq!(*d, CubicBasis::CatmullRom.derivative(c, station.u));
        }
    }

    #[test]
    fn errors_leave_the_buffer_empty() {
        let controls = [[0.0], [1.0], [2.0], [3.0]];
        let at = stations(2, 2, false);
        let mut out = alloc::vec![[9.0]];
        let error = sample_into(
            CubicBasis::Bezier,
            &controls,
            &[[0, 1, 2, 3]],
            &at,
            &mut out,
        );
        assert!(error.is_err());
        assert!(out.is_empty());
        out.push([9.0]);
        assert!(sample_linear_into(&controls, &[[0, 1]], &at, &mut out).is_err());
        assert!(out.is_empty());
    }
}
