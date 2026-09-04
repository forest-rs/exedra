// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic 2D polygon triangulation for Exedra.
//!
//! The crate owns:
//! - triangulation of simple polygons with holes via ear clipping,
//! - deterministic hole bridging into a single ring,
//! - optional constrained-Delaunay edge legalization,
//! - the exact-sign planar predicates those algorithms rely on.
//!
//! Determinism is the contract: identical input bits produce identical output
//! triangles on every platform, in every build mode. The algorithms use only
//! f64 comparisons and exact-sign predicates — no transcendental functions, no
//! ambient epsilons, no hash-order iteration. Ambiguous choices (equally valid
//! ears) resolve by lowest stable input index.
//!
//! Every output vertex is an input vertex. The triangulator never invents
//! points (no Steiner insertion), so callers can carry per-vertex provenance
//! through triangulation unchanged.
//!
//! The crate intentionally does not know about meshes, curves, flattening
//! tolerances, or any Exedra ID type. Inputs are coordinate slices; outputs
//! are `u32` indices into the concatenation of those slices.
//!
//! ## Input model
//!
//! [`PolygonInput`] describes one planar polygon: an outer loop and zero or
//! more hole loops. Loops are index-free coordinate slices; the outer loop
//! must wind counter-clockwise and holes clockwise (validated, not assumed).
//! Output triangle indices address the virtual concatenation
//! `outer ++ holes[0] ++ holes[1] ++ …`, in order.
//!
//! # Example
//!
//! ```
//! use exedra_triangulate::{PolygonInput, TriParams, triangulate};
//!
//! let square = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]];
//! let input = PolygonInput {
//!     outer: &square,
//!     holes: &[],
//! };
//! let result = triangulate(&input, &TriParams::default()).expect("simple CCW polygon");
//!
//! assert_eq!(result.len(), 2);
//! assert!(result.triangles.iter().flatten().all(|&index| index < 4));
//! ```

#![no_std]
extern crate alloc;

mod bridge;
mod delaunay;
mod earclip;
pub mod predicates;
#[cfg(test)]
mod torture;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::bridge::{bridge_holes, prune_collinear_between};
use crate::delaunay::legalize_edges;
use crate::earclip::{earclip_ring, len_u32, twice_signed_area};
pub use crate::predicates::MAX_COORDINATE;

/// One planar polygon: an outer loop with zero or more hole loops.
///
/// Coordinates are f64 pairs. The outer loop must be a simple polygon in
/// counter-clockwise winding; each hole must be simple, clockwise, and
/// strictly inside the outer loop, and holes must not intersect each other.
/// [`validate`](Self::validate) checks the cheap structural subset of these
/// requirements; full geometric validation happens during triangulation,
/// which reports violations as typed [`TriError`]s rather than producing
/// garbage triangles.
#[derive(Copy, Clone, Debug)]
pub struct PolygonInput<'a> {
    /// Outer loop vertices in counter-clockwise order.
    pub outer: &'a [[f64; 2]],
    /// Hole loops, each in clockwise order.
    pub holes: &'a [&'a [[f64; 2]]],
}

impl PolygonInput<'_> {
    /// Total vertex count across the outer loop and all holes.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.outer.len() + self.holes.iter().map(|h| h.len()).sum::<usize>()
    }

    /// Validates structural requirements: loop sizes, finite coordinates,
    /// and the `u32` index budget.
    ///
    /// This is the cheap prefix of validation; winding, simplicity, and
    /// containment are verified during triangulation.
    ///
    /// # Errors
    ///
    /// Returns the first violation found, scanning the outer loop first and
    /// then holes in order.
    pub fn validate(&self) -> Result<(), TriError> {
        if self.vertex_count() > u32::MAX as usize {
            return Err(TriError::TooManyVertices);
        }
        validate_loop(self.outer, None)?;
        for (index, hole) in self.holes.iter().enumerate() {
            validate_loop(hole, Some(index))?;
        }
        Ok(())
    }
}

fn validate_loop(points: &[[f64; 2]], hole: Option<usize>) -> Result<(), TriError> {
    if points.len() < 3 {
        return Err(TriError::DegenerateLoop { hole });
    }
    if points
        .iter()
        .any(|p| !p[0].is_finite() || !p[1].is_finite())
    {
        return Err(TriError::NonFiniteCoordinate { hole });
    }
    if points
        .iter()
        .any(|p| p[0].abs() > MAX_COORDINATE || p[1].abs() > MAX_COORDINATE)
    {
        return Err(TriError::CoordinateOutOfRange { hole });
    }
    Ok(())
}

/// Triangulates one polygon.
///
/// The outer loop must be a simple counter-clockwise polygon; holes must be
/// simple, clockwise, strictly inside the outer loop, and mutually disjoint.
/// Output triangles wind counter-clockwise and index the virtual
/// concatenation `outer ++ holes[0] ++ holes[1] ++ …`.
/// A successful result also preserves the simplified input rings exactly:
/// every boundary edge has one incident triangle and every bridge or interior
/// edge has two. This rules out zero-width bridge corridors, which can have
/// the correct summed area while silently joining two aligned holes.
///
/// Deterministic: identical input bits and parameters produce identical
/// output on every platform. Never panics; every failure is a typed
/// [`TriError`].
///
/// # Errors
///
/// Structural violations are reported by [`PolygonInput::validate`];
/// geometric violations surface as [`TriError::WrongWinding`],
/// [`TriError::HoleOutsideOuter`], [`TriError::UnbridgeableHole`], and
/// [`TriError::NonSimple`].
pub fn triangulate(
    input: &PolygonInput<'_>,
    params: &TriParams,
) -> Result<Triangulation, TriError> {
    triangulate_with_stats(input, params).map(|evaluation| evaluation.triangulation)
}

/// Triangulates one polygon and reports deterministic strategy work.
///
/// This is the diagnostic form of [`triangulate`]. It produces the same
/// triangles and additionally reports work such as constrained-Delaunay edge
/// flips without using global counters.
///
/// # Errors
///
/// Returns the same typed input and geometry errors as [`triangulate`].
pub fn triangulate_with_stats(
    input: &PolygonInput<'_>,
    params: &TriParams,
) -> Result<TriangulationEvaluation, TriError> {
    input.validate()?;

    // Materialize the virtual concatenation the output indices address.
    let mut points: Vec<[f64; 2]> = Vec::with_capacity(input.vertex_count());
    points.extend_from_slice(input.outer);
    let mut hole_ranges: Vec<(u32, u32)> = Vec::with_capacity(input.holes.len());
    for hole in input.holes {
        hole_ranges.push((len_u32(points.len()), len_u32(hole.len())));
        points.extend_from_slice(hole);
    }

    // The u32 budget is validated above; enumerate ring positions directly.
    let ring: Vec<u32> = (0..).take(input.outer.len()).collect();
    if twice_signed_area(&points, &ring) <= 0.0 {
        return Err(TriError::WrongWinding { hole: None });
    }
    for (index, &(base, len)) in hole_ranges.iter().enumerate() {
        let hole_ring: Vec<u32> = (base..base + len).collect();
        if twice_signed_area(&points, &hole_ring) >= 0.0 {
            return Err(TriError::WrongWinding { hole: Some(index) });
        }
    }

    // Preserve the historical hole and candidate ordering first. Aligned
    // holes can make that traversal weakly simple in area yet topologically
    // wrong; bounded fallbacks reverse the exact-x hole tie and prefer short
    // visible bridges. The incidence check below, not successful ear clipping
    // alone, decides whether an attempt represents these exact input rings.
    let mut last_error = TriError::NonSimple;
    for (descending_y_ties, nearest_candidates) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let composite = match bridge_holes(
            &points,
            ring.clone(),
            &hole_ranges,
            descending_y_ties,
            nearest_candidates,
        ) {
            Ok(composite) => composite,
            Err(error) => {
                last_error = error;
                continue;
            }
        };
        let mut triangles = Vec::with_capacity(points.len().saturating_sub(2));
        if let Err(error) = earclip_ring(&points, &composite, &mut triangles) {
            last_error = error;
            continue;
        }
        if !triangulation_preserves_boundaries(
            &points,
            len_u32(input.outer.len()),
            &hole_ranges,
            &triangles,
        ) {
            last_error = TriError::NonSimple;
            continue;
        }
        // Legalize only an accepted cover: boundary incidence is already
        // verified, so every edge with two incident triangles is a bridge or
        // interior edge that flipping may legitimately replace.
        let edge_flips = match params.strategy {
            TriStrategy::EarClip => 0,
            TriStrategy::ConstrainedDelaunay => legalize_edges(&points, &mut triangles),
        };
        return Ok(TriangulationEvaluation {
            triangulation: Triangulation { triangles },
            stats: TriangulationStats { edge_flips },
        });
    }
    Err(last_error)
}

/// Checks that triangle edge incidence reproduces exactly the simplified
/// input rings: boundary edges occur once and every bridge/interior edge
/// occurs twice. Area agreement alone cannot detect a zero-width bridge
/// corridor that silently joins two aligned holes.
fn triangulation_preserves_boundaries(
    points: &[[f64; 2]],
    outer_len: u32,
    holes: &[(u32, u32)],
    triangles: &[[u32; 3]],
) -> bool {
    let edge = |a: u32, b: u32| (a.min(b), a.max(b));
    let mut expected = Vec::<(u32, u32)>::new();
    let mut rings = Vec::<Vec<u32>>::with_capacity(holes.len() + 1);
    rings.push((0..outer_len).collect());
    rings.extend(
        holes
            .iter()
            .map(|&(base, len)| (base..base + len).collect()),
    );
    for mut ring in rings {
        prune_collinear_between(points, &mut ring);
        if ring.len() < 3 {
            return false;
        }
        for index in 0..ring.len() {
            expected.push(edge(ring[index], ring[(index + 1) % ring.len()]));
        }
    }
    expected.sort_unstable();

    let mut incidence = BTreeMap::<(u32, u32), u8>::new();
    for triangle in triangles {
        for corner in 0..3 {
            let count = incidence
                .entry(edge(triangle[corner], triangle[(corner + 1) % 3]))
                .or_default();
            *count = count.saturating_add(1);
            if *count > 2 {
                return false;
            }
        }
    }
    let actual: Vec<_> = incidence
        .into_iter()
        .filter_map(|(edge, count)| (count == 1).then_some(edge))
        .collect();
    actual == expected
}

/// Triangulation parameters.
///
/// The default configuration is the supported v1 behavior: deterministic ear
/// clipping with hole bridging. [`TriStrategy`] also offers opt-in exact edge
/// legalization; each strategy is independently deterministic.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TriParams {
    /// Triangulation strategy to apply.
    pub strategy: TriStrategy,
}

/// Selects the triangulation algorithm.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TriStrategy {
    /// Deterministic ear clipping with hole bridging.
    ///
    /// Ambiguous ear choices resolve by lowest stable input index. Suitable
    /// for the small, well-conditioned loops produced by profile flattening;
    /// O(n²) in the vertex count.
    #[default]
    EarClip,
    /// Ear clipping followed by exact constrained-Delaunay edge legalization.
    ///
    /// Polygon and hole boundaries remain fixed. Unconstrained interior edges
    /// are flipped in stable canonical order; exact cocircular ties choose the
    /// lexicographically smaller diagonal. The result uses only input vertices.
    ConstrainedDelaunay,
}

/// Structured triangulation failure.
///
/// Errors distinguish invalid input classes from internal invariant
/// violations so callers can report actionable diagnostics. The triangulator
/// never panics on any input.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TriError {
    /// A loop has fewer than three vertices. `hole` is the hole index, or
    /// `None` for the outer loop.
    DegenerateLoop {
        /// Index of the offending hole, or `None` for the outer loop.
        hole: Option<usize>,
    },
    /// A coordinate is NaN or infinite. `hole` is the hole index, or `None`
    /// for the outer loop.
    NonFiniteCoordinate {
        /// Index of the offending hole, or `None` for the outer loop.
        hole: Option<usize>,
    },
    /// A coordinate magnitude exceeds [`MAX_COORDINATE`], the bound that
    /// keeps the exact predicates overflow-free. `hole` is the hole index,
    /// or `None` for the outer loop.
    CoordinateOutOfRange {
        /// Index of the offending hole, or `None` for the outer loop.
        hole: Option<usize>,
    },
    /// A loop winds the wrong way: the outer loop must be counter-clockwise
    /// and holes clockwise. `hole` is the hole index, or `None` for the
    /// outer loop.
    WrongWinding {
        /// Index of the offending hole, or `None` for the outer loop.
        hole: Option<usize>,
    },
    /// The input is not a simple polygon with positive area:
    /// self-intersecting, zero-width, or degenerate. Detected honestly — a
    /// simple polygon always has an ear under exact predicates, so running
    /// out of ears proves non-simplicity.
    NonSimple,
    /// A hole's anchor vertex is not strictly inside the outer loop.
    HoleOutsideOuter {
        /// Index of the offending hole.
        hole: usize,
    },
    /// No valid bridge exists between this hole and the rest of the polygon:
    /// every candidate segment crosses or touches an edge.
    UnbridgeableHole {
        /// Index of the offending hole.
        hole: usize,
    },
    /// Total vertex count exceeds the `u32` index budget.
    TooManyVertices,
}

impl core::fmt::Display for TriError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DegenerateLoop { hole: None } => {
                write!(f, "outer loop has fewer than three vertices")
            }
            Self::DegenerateLoop { hole: Some(i) } => {
                write!(f, "hole {i} has fewer than three vertices")
            }
            Self::NonFiniteCoordinate { hole: None } => {
                write!(f, "outer loop contains a non-finite coordinate")
            }
            Self::NonFiniteCoordinate { hole: Some(i) } => {
                write!(f, "hole {i} contains a non-finite coordinate")
            }
            Self::CoordinateOutOfRange { hole: None } => {
                write!(
                    f,
                    "outer loop coordinate magnitude exceeds the supported range"
                )
            }
            Self::CoordinateOutOfRange { hole: Some(i) } => {
                write!(
                    f,
                    "hole {i} coordinate magnitude exceeds the supported range"
                )
            }
            Self::WrongWinding { hole: None } => {
                write!(f, "outer loop must wind counter-clockwise")
            }
            Self::WrongWinding { hole: Some(i) } => {
                write!(f, "hole {i} must wind clockwise")
            }
            Self::NonSimple => {
                write!(f, "input is not a simple polygon with positive area")
            }
            Self::HoleOutsideOuter { hole } => {
                write!(f, "hole {hole} is not strictly inside the outer loop")
            }
            Self::UnbridgeableHole { hole } => {
                write!(f, "hole {hole} cannot be bridged without crossing an edge")
            }
            Self::TooManyVertices => write!(f, "total vertex count exceeds u32 indices"),
        }
    }
}

impl core::error::Error for TriError {}

/// Triangulation output: triangles as indices into the input concatenation.
///
/// Indices address `outer ++ holes[0] ++ holes[1] ++ …` in input order. Every
/// index refers to an input vertex; triangulation never introduces points.
/// Triangle order and vertex order within each triangle are deterministic for
/// fixed input and parameters, and every triangle winds counter-clockwise.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Triangulation {
    /// Triangles in deterministic emission order.
    pub triangles: Vec<[u32; 3]>,
}

impl Triangulation {
    /// Number of triangles.
    #[must_use]
    pub fn len(&self) -> usize {
        self.triangles.len()
    }

    /// Returns true when no triangles were produced.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }
}

/// Triangulation result together with deterministic strategy work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TriangulationEvaluation {
    /// Resulting input-indexed triangles.
    pub triangulation: Triangulation,
    /// Work performed after the initial ear-clipped cover was built.
    pub stats: TriangulationStats,
}

/// Deterministic work counters for one triangulation.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TriangulationStats {
    /// Number of unconstrained interior edge flips.
    ///
    /// This is zero for [`TriStrategy::EarClip`].
    pub edge_flips: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    #[test]
    fn validate_accepts_simple_square() {
        let input = PolygonInput {
            outer: &SQUARE,
            holes: &[],
        };
        assert_eq!(input.validate(), Ok(()));
        assert_eq!(input.vertex_count(), 4);
    }

    #[test]
    fn validate_rejects_short_outer_loop() {
        let input = PolygonInput {
            outer: &SQUARE[..2],
            holes: &[],
        };
        assert_eq!(
            input.validate(),
            Err(TriError::DegenerateLoop { hole: None })
        );
    }

    #[test]
    fn validate_rejects_short_hole_by_index() {
        let hole: [[f64; 2]; 2] = [[0.25, 0.25], [0.5, 0.25]];
        let input = PolygonInput {
            outer: &SQUARE,
            holes: &[&hole],
        };
        assert_eq!(
            input.validate(),
            Err(TriError::DegenerateLoop { hole: Some(0) })
        );
    }

    #[test]
    fn validate_rejects_non_finite_coordinates() {
        let bad: [[f64; 2]; 3] = [[0.0, 0.0], [1.0, f64::NAN], [1.0, 1.0]];
        let input = PolygonInput {
            outer: &bad,
            holes: &[],
        };
        assert_eq!(
            input.validate(),
            Err(TriError::NonFiniteCoordinate { hole: None })
        );
    }

    #[test]
    fn params_default_is_ear_clip() {
        assert_eq!(TriParams::default().strategy, TriStrategy::EarClip);
    }

    fn constrained_delaunay_params() -> TriParams {
        TriParams {
            strategy: TriStrategy::ConstrainedDelaunay,
        }
    }

    /// Twice the signed area of the triangle `(a, b, c)` over `points`.
    fn tri_area2(points: &[[f64; 2]], t: [u32; 3]) -> f64 {
        let a = points[t[0] as usize];
        let b = points[t[1] as usize];
        let c = points[t[2] as usize];
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }

    /// Twice the signed shoelace area of one test ring.
    fn ring_area2(points: &[[f64; 2]]) -> f64 {
        points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let next = points[(index + 1) % points.len()];
                point[0] * next[1] - next[0] * point[1]
            })
            .sum()
    }

    /// Asserts the invariants every successful triangulation must satisfy:
    /// n-2 triangle count bound, positive (CCW) triangle areas, area sum
    /// matching the polygon, and double-run determinism.
    fn assert_triangulation(outer: &[[f64; 2]], expected_area2: f64) -> Triangulation {
        let input = PolygonInput { outer, holes: &[] };
        let params = TriParams::default();
        let result = triangulate(&input, &params).expect("triangulation must succeed");
        let again = triangulate(&input, &params).expect("second run must succeed");
        assert_eq!(result, again, "triangulation must be deterministic");

        let mut area2 = 0.0;
        for &t in &result.triangles {
            let a2 = tri_area2(outer, t);
            assert!(a2 > 0.0, "triangle {t:?} must wind counter-clockwise");
            area2 += a2;
        }
        let scale = expected_area2.abs().max(1.0);
        assert!(
            (area2 - expected_area2).abs() <= scale * 1e-12,
            "area sum {area2} must match polygon area {expected_area2}"
        );
        result
    }

    #[test]
    fn square_clips_to_two_triangles() {
        let result = assert_triangulation(&SQUARE, 2.0);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn constrained_delaunay_flips_the_choice_driven_quad() {
        let outer = [[4.9, 4.9], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[],
        };
        let evaluation = triangulate_with_stats(&input, &constrained_delaunay_params())
            .expect("choice-driven quad triangulates");
        assert_eq!(evaluation.stats.edge_flips, 1);
        assert_eq!(evaluation.triangulation.triangles, [[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn constrained_delaunay_cocircular_tie_is_canonical() {
        let outer = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[],
        };
        let first = triangulate_with_stats(&input, &constrained_delaunay_params())
            .expect("cocircular quad triangulates");
        let second = triangulate_with_stats(&input, &constrained_delaunay_params())
            .expect("repeat triangulation succeeds");
        assert_eq!(first, second);
        assert_eq!(first.triangulation.triangles, [[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn concave_l_profile_clips_correctly() {
        // L-shape: 1x1 square with the top-right 0.5 x 0.5 quadrant removed.
        let l: [[f64; 2]; 6] = [
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.5],
            [0.5, 0.5],
            [0.5, 1.0],
            [0.0, 1.0],
        ];
        let result = assert_triangulation(&l, 2.0 * 0.75);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn spiky_concave_polygon_clips_correctly() {
        // A comb-like profile with deep concavities.
        let comb: [[f64; 2]; 8] = [
            [0.0, 0.0],
            [6.0, 0.0],
            [6.0, 3.0],
            [5.0, 1.0],
            [4.0, 3.0],
            [2.0, 1.0],
            [1.0, 3.0],
            [0.0, 3.0],
        ];
        let area2 = twice_signed_area(&comb, &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(area2 > 0.0, "fixture must be counter-clockwise");
        let result = assert_triangulation(&comb, area2);
        assert_eq!(result.len(), comb.len() - 2);
    }

    #[test]
    fn collinear_vertices_are_pruned() {
        // Square with redundant collinear vertices along the bottom edge.
        let redundant: [[f64; 2]; 6] = [
            [0.0, 0.0],
            [0.5, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.5, 1.0],
            [0.0, 1.0],
        ];
        let result = assert_triangulation(&redundant, 2.0);
        // Collinear vertices may be dropped or kept as degenerate-free ears;
        // either way the area invariant above holds and nothing panics.
        assert!(result.len() >= 2, "at least the square itself is covered");
    }

    #[test]
    fn duplicate_consecutive_points_are_pruned() {
        let doubled: [[f64; 2]; 5] = [[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let result = assert_triangulation(&doubled, 2.0);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn clockwise_input_is_rejected() {
        let cw: [[f64; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let input = PolygonInput {
            outer: &cw,
            holes: &[],
        };
        assert_eq!(
            triangulate(&input, &TriParams::default()),
            Err(TriError::WrongWinding { hole: None })
        );
    }

    #[test]
    fn self_intersecting_bowtie_is_rejected() {
        let bowtie: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]];
        let input = PolygonInput {
            outer: &bowtie,
            holes: &[],
        };
        let result = triangulate(&input, &TriParams::default());
        assert!(
            matches!(
                result,
                Err(TriError::NonSimple | TriError::WrongWinding { .. })
            ),
            "bowtie must be rejected, got {result:?}"
        );
    }

    /// Like [`assert_triangulation`] but for inputs with holes.
    fn assert_holed_triangulation(
        outer: &[[f64; 2]],
        holes: &[&[[f64; 2]]],
        expected_area2: f64,
    ) -> Triangulation {
        let input = PolygonInput { outer, holes };
        let params = TriParams::default();
        let result = triangulate(&input, &params).expect("triangulation must succeed");
        let again = triangulate(&input, &params).expect("second run must succeed");
        assert_eq!(result, again, "triangulation must be deterministic");

        let mut points: Vec<[f64; 2]> = outer.to_vec();
        for h in holes {
            points.extend_from_slice(h);
        }
        let mut area2 = 0.0;
        for &t in &result.triangles {
            let a2 = tri_area2(&points, t);
            assert!(a2 > 0.0, "triangle {t:?} must wind counter-clockwise");
            area2 += a2;
        }
        let scale = expected_area2.abs().max(1.0);
        assert!(
            (area2 - expected_area2).abs() <= scale * 1e-12,
            "area sum {area2} must match polygon-minus-holes area {expected_area2}"
        );
        result
    }

    // A 0.2 x 0.2 square hole centered in the unit square, clockwise.
    const SQUARE_HOLE: [[f64; 2]; 4] = [[0.4, 0.4], [0.4, 0.6], [0.6, 0.6], [0.6, 0.4]];

    #[test]
    fn square_with_square_hole() {
        let result = assert_holed_triangulation(&SQUARE, &[&SQUARE_HOLE], 2.0 * (1.0 - 0.04));
        // n + 2h - 2 triangles for n total vertices and h bridged holes.
        assert_eq!(result.len(), 8 + 2 - 2);
    }

    #[test]
    fn constrained_delaunay_preserves_outer_and_hole_boundaries() {
        let holes = [&SQUARE_HOLE[..]];
        let input = PolygonInput {
            outer: &SQUARE,
            holes: &holes,
        };
        let first = triangulate_with_stats(&input, &constrained_delaunay_params())
            .expect("holed square triangulates");
        let second = triangulate_with_stats(&input, &constrained_delaunay_params())
            .expect("repeat triangulation succeeds");
        assert_eq!(first, second);

        let mut edges: Vec<[u32; 2]> = first
            .triangulation
            .triangles
            .iter()
            .flat_map(|&[a, b, c]| {
                [
                    [a.min(b), a.max(b)],
                    [b.min(c), b.max(c)],
                    [c.min(a), c.max(a)],
                ]
            })
            .collect();
        edges.sort_unstable();
        for boundary in [
            [0, 1],
            [1, 2],
            [2, 3],
            [0, 3],
            [4, 5],
            [5, 6],
            [6, 7],
            [4, 7],
        ] {
            assert!(edges.contains(&boundary), "missing boundary {boundary:?}");
        }

        let points: Vec<[f64; 2]> = SQUARE.into_iter().chain(SQUARE_HOLE).collect();
        let area2 = first
            .triangulation
            .triangles
            .iter()
            .map(|&triangle| tri_area2(&points, triangle))
            .sum::<f64>();
        assert!((area2 - 1.92).abs() <= 1e-12);
    }

    #[test]
    fn square_with_two_holes() {
        // Two clockwise triangular holes, left and right of center.
        let left: [[f64; 2]; 3] = [[0.1, 0.1], [0.1, 0.3], [0.3, 0.2]];
        let right: [[f64; 2]; 3] = [[0.7, 0.1], [0.7, 0.3], [0.9, 0.2]];
        let hole_area2 = 2.0 * (0.5 * 0.2 * 0.2);
        assert_holed_triangulation(&SQUARE, &[&left, &right], 2.0 - 2.0 * hole_area2);
    }

    #[test]
    fn rectangle_with_four_aligned_rectangular_holes() {
        // Pins the four-cutter cap that used to exhaust ear clipping when
        // collinear samples made several equal-x hole bridges compete.
        let outer = [[0.0, 0.0], [4.0, 0.0], [4.0, 6.0], [0.0, 6.0]];
        let left = [
            [2.3750000430736695, 0.8999999761581421],
            [2.5, 0.8999999761581421],
            [2.5, 0.8250000064726907],
            [2.5, 0.30000001192092896],
            [1.6249999569263305, 0.30000001192092896],
            [1.5, 0.30000001192092896],
            [1.5, 0.3749999816063804],
            [1.5, 0.8999999761581421],
        ];
        let middle_left = [
            [2.3750000430736695, 2.0999999046325684],
            [2.5, 2.0999999046325684],
            [2.5, 2.024999942397695],
            [2.5, 1.5],
            [1.6249999569263305, 1.5],
            [1.5, 1.5],
            [1.5, 1.5749999622348734],
            [1.5, 2.0999999046325684],
        ];
        let middle_right = [
            [2.3750000430736695, 2.700000047683716],
            [1.800000031789144, 2.700000047683716],
            [1.5, 2.700000047683716],
            [1.5, 3.224999990081411],
            [1.5, 3.299999952316284],
            [1.6249999569263305, 3.299999952316284],
            [2.1999999682108564, 3.299999952316284],
            [2.5, 3.299999952316284],
            [2.5, 2.775000009918589],
            [2.5, 2.700000047683716],
        ];
        let right = [
            [1.5, 4.5],
            [2.3750000430736695, 4.5],
            [2.5, 4.5],
            [2.5, 4.425000037765127],
            [2.5, 3.9000000953674316],
            [1.6249999569263305, 3.9000000953674316],
            [1.5, 3.9000000953674316],
            [1.5, 3.975000057602305],
        ];
        let expected = ring_area2(&outer)
            + ring_area2(&left)
            + ring_area2(&middle_left)
            + ring_area2(&middle_right)
            + ring_area2(&right);
        assert_holed_triangulation(
            &outer,
            &[&left, &middle_left, &middle_right, &right],
            expected,
        );
    }

    #[test]
    fn rotated_rectangle_with_four_aligned_rectangular_holes() {
        // Pins the opposite cap from the same boolean: its rotated sampling
        // once returned correct area with a zero-width corridor between two
        // holes, so success must preserve all five boundary components.
        let outer = [[6.0, 0.0], [6.0, 4.0], [0.0, 4.0], [0.0, 0.0]];
        let first = [
            [3.299999952316284, 2.5],
            [3.299999952316284, 2.3749999965075403],
            [3.299999952316284, 1.800000031789144],
            [3.299999952316284, 1.5],
            [2.775000037858262, 1.5],
            [2.700000047683716, 1.5],
            [2.700000047683716, 1.6250000034924597],
            [2.700000047683716, 2.1999999682108564],
            [2.700000047683716, 2.5],
            [3.224999962141738, 2.5],
        ];
        let second = [
            [3.9000000953674316, 2.3749999965075403],
            [3.9000000953674316, 2.5],
            [3.975000085541978, 2.5],
            [4.5, 2.5],
            [4.5, 1.6250000034924597],
            [4.5, 1.5],
            [4.425000009825453, 1.5],
            [3.9000000953674316, 1.5],
        ];
        let third = [
            [0.30000001192092896, 1.5],
            [0.30000001192092896, 2.3749999965075403],
            [0.30000001192092896, 2.5],
            [0.3750000095460563, 2.5],
            [0.8999999761581421, 2.5],
            [0.8999999761581421, 1.6250000034924597],
            [0.8999999761581421, 1.5],
            [0.8249999785330148, 1.5],
        ];
        let fourth = [
            [1.5, 1.5],
            [1.5, 2.3749999965075403],
            [1.5, 2.5],
            [1.5749999901745464, 2.5],
            [2.0999999046325684, 2.5],
            [2.0999999046325684, 1.6250000034924597],
            [2.0999999046325684, 1.5],
            [2.024999914458022, 1.5],
        ];
        let expected = ring_area2(&outer)
            + ring_area2(&first)
            + ring_area2(&second)
            + ring_area2(&third)
            + ring_area2(&fourth);
        assert_holed_triangulation(&outer, &[&first, &second, &third, &fourth], expected);
    }

    #[test]
    fn hole_in_concave_l_profile() {
        // The et-uwtn checkpoint fixture: L-shape with a hole in its foot.
        let l: [[f64; 2]; 6] = [
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.5],
            [0.5, 0.5],
            [0.5, 1.0],
            [0.0, 1.0],
        ];
        let hole: [[f64; 2]; 4] = [[0.1, 0.1], [0.1, 0.4], [0.4, 0.4], [0.4, 0.1]];
        assert_holed_triangulation(&l, &[&hole], 2.0 * 0.75 - 2.0 * 0.09);
    }

    #[test]
    fn counter_clockwise_hole_is_rejected() {
        let ccw_hole: [[f64; 2]; 4] = [[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]];
        let input = PolygonInput {
            outer: &SQUARE,
            holes: &[&ccw_hole],
        };
        assert_eq!(
            triangulate(&input, &TriParams::default()),
            Err(TriError::WrongWinding { hole: Some(0) })
        );
    }

    #[test]
    fn hole_outside_outer_is_rejected() {
        let outside: [[f64; 2]; 4] = [[2.0, 2.0], [2.0, 2.5], [2.5, 2.5], [2.5, 2.0]];
        let input = PolygonInput {
            outer: &SQUARE,
            holes: &[&outside],
        };
        assert_eq!(
            triangulate(&input, &TriParams::default()),
            Err(TriError::HoleOutsideOuter { hole: 0 })
        );
    }

    #[test]
    fn out_of_range_coordinates_are_rejected() {
        let huge: [[f64; 2]; 3] = [[0.0, 0.0], [1.5e100, 0.0], [0.0, 1.0]];
        let input = PolygonInput {
            outer: &huge,
            holes: &[],
        };
        assert_eq!(
            input.validate(),
            Err(TriError::CoordinateOutOfRange { hole: None })
        );
    }

    #[test]
    fn lowest_index_ear_is_clipped_first() {
        // Regular convex hexagon: every vertex is a valid ear, so the
        // tie-break rule fully determines emission order. The first clipped
        // ear must be vertex 0, emitting triangle (5, 0, 1).
        let hex: [[f64; 2]; 6] = [
            [2.0, 0.0],
            [1.0, 2.0],
            [-1.0, 2.0],
            [-2.0, 0.0],
            [-1.0, -2.0],
            [1.0, -2.0],
        ];
        let input = PolygonInput {
            outer: &hex,
            holes: &[],
        };
        let result = triangulate(&input, &TriParams::default()).expect("hexagon triangulates");
        assert_eq!(result.triangles[0], [5, 0, 1]);
        // After clipping 0, the lowest live valid ear is 1, and so on.
        assert_eq!(result.triangles[1], [5, 1, 2]);
    }
}
