// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Boolean failure taxonomy and diagnostic accumulation.
//!
//! The boolean pipeline never panics on geometric trouble: every stage
//! classifies what it saw and pushes typed [`BooleanDiagnostic`] values
//! into a caller-owned, bounded [`BooleanDiagnostics`] accumulator.
//! Diagnostics are *data* — deterministically ordered (push order, and
//! stages push in candidate order), bounded (an explicit cap plus a
//! dropped counter, so pathological inputs cannot balloon memory), and
//! inspectable after the fact.

use alloc::vec::Vec;

use super::BooleanTriangleRef;

/// What kind of trouble a boolean stage classified.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum BooleanFailureKind {
    /// Input topology is non-manifold where the pipeline requires
    /// manifoldness.
    NonManifoldInput,
    /// A single input mesh intersects itself.
    SelfIntersectionDetected,
    /// An exactly coplanar configuration outside the handled face-on-face
    /// contact envelope (non-planar faces hiding coplanar triangles,
    /// degenerate orientation probes, contact regions not cleanly carved);
    /// reported instead of guessed.
    CoplanarAmbiguity,
    /// A quantity fell inside the tolerance band where classification is
    /// unreliable.
    ToleranceExceeded,
    /// Floating-point construction produced an unusable value (for
    /// example a zero plane-intersection direction for triangles the
    /// exact predicates say must cross).
    NumericalInstability,
    /// An internal invariant was violated; always a bug, never an input
    /// problem.
    InternalInvariantViolation,
    /// A splitting configuration outside the current envelope (for example
    /// mixed open chains and loops, dangling cuts, or ambiguous sub-loop
    /// assignment); the face is left unsplit and reported instead of guessed.
    SplitDeferred,
    /// An input triangle is degenerate (zero area / collinear corners).
    DegenerateTriangle,
}

impl core::fmt::Display for BooleanFailureKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = match self {
            Self::NonManifoldInput => "non-manifold input",
            Self::SelfIntersectionDetected => "self-intersection detected",
            Self::CoplanarAmbiguity => "coplanar ambiguity",
            Self::ToleranceExceeded => "tolerance exceeded",
            Self::NumericalInstability => "numerical instability",
            Self::InternalInvariantViolation => "internal invariant violation",
            Self::SplitDeferred => "splitting deferred for this configuration",
            Self::DegenerateTriangle => "degenerate triangle",
        };
        f.write_str(name)
    }
}

/// One classified diagnostic, attributed to the triangles that produced it.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BooleanDiagnostic {
    /// Failure classification.
    pub kind: BooleanFailureKind,
    /// Triangle from mesh A this concerns, when attributable.
    pub a: Option<BooleanTriangleRef>,
    /// Triangle from mesh B this concerns, when attributable.
    pub b: Option<BooleanTriangleRef>,
    /// Stable, static human-readable detail (no allocation; diagnostics
    /// stay bounded and streamable).
    pub detail: &'static str,
}

impl core::fmt::Display for BooleanDiagnostic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.kind, self.detail)?;
        if let Some(a) = self.a {
            write!(f, " [a: face {} tri {}]", a.face.index(), a.triangle_index)?;
        }
        if let Some(b) = self.b {
            write!(f, " [b: face {} tri {}]", b.face.index(), b.triangle_index)?;
        }
        Ok(())
    }
}

/// Bounded, deterministic diagnostic accumulator.
///
/// Stages push in candidate order, so entries are deterministically
/// ordered for fixed input. Once `limit` entries are held, further pushes
/// only increment [`BooleanDiagnostics::dropped`] — large pathological
/// inputs degrade to counters, never to unbounded memory.
#[derive(Clone, Debug)]
pub struct BooleanDiagnostics {
    entries: Vec<BooleanDiagnostic>,
    limit: usize,
    dropped: u64,
}

impl Default for BooleanDiagnostics {
    fn default() -> Self {
        Self::new(Self::DEFAULT_LIMIT)
    }
}

impl BooleanDiagnostics {
    /// Default retained-entry cap.
    pub const DEFAULT_LIMIT: usize = 1024;

    /// Creates an accumulator retaining at most `limit` entries.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit,
            dropped: 0,
        }
    }

    /// Pushes one diagnostic, dropping (and counting) beyond the cap.
    pub fn push(&mut self, diagnostic: BooleanDiagnostic) {
        if self.entries.len() < self.limit {
            self.entries.push(diagnostic);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Retained diagnostics in push order.
    #[must_use]
    pub fn entries(&self) -> &[BooleanDiagnostic] {
        &self.entries
    }

    /// Number of diagnostics dropped beyond the cap.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// True when nothing was recorded (or dropped).
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty() && self.dropped == 0
    }

    /// Number of diagnostics of `kind` retained.
    #[must_use]
    pub fn count_of(&self, kind: BooleanFailureKind) -> usize {
        self.entries.iter().filter(|d| d.kind == kind).count()
    }

    /// Clears entries and the dropped counter, retaining capacity.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.dropped = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FaceId;
    use core::num::NonZeroU32;

    fn touch_ref() -> BooleanTriangleRef {
        BooleanTriangleRef {
            face: FaceId::new(3, NonZeroU32::MIN),
            triangle_index: 1,
        }
    }

    #[test]
    fn accumulator_bounds_and_counts() {
        let mut diags = BooleanDiagnostics::new(2);
        for _ in 0..5 {
            diags.push(BooleanDiagnostic {
                kind: BooleanFailureKind::CoplanarAmbiguity,
                a: Some(touch_ref()),
                b: None,
                detail: "test entry",
            });
        }
        assert_eq!(diags.entries().len(), 2);
        assert_eq!(diags.dropped(), 3);
        assert!(!diags.is_clean());
        assert_eq!(diags.count_of(BooleanFailureKind::CoplanarAmbiguity), 2);
        diags.clear();
        assert!(diags.is_clean());
    }

    #[test]
    fn display_is_informative() {
        let diagnostic = BooleanDiagnostic {
            kind: BooleanFailureKind::DegenerateTriangle,
            a: Some(touch_ref()),
            b: None,
            detail: "zero-area corner fan",
        };
        let rendered = alloc::format!("{diagnostic}");
        assert!(rendered.contains("degenerate triangle"));
        assert!(rendered.contains("face 3 tri 1"));
    }
}
