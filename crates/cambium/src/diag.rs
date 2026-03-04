// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Structured diagnostics with bounded, deterministic retention.

use alloc::string::String;
use alloc::vec::Vec;

use exedra::{CornerId, FaceId, HalfEdgeId, VertexId};

/// Diagnostic severity level.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DiagLevel {
    /// Informational note.
    Note,
    /// Warning that does not prevent execution.
    Warn,
    /// Error that indicates a failed operation or invalid state.
    Error,
}

impl DiagLevel {
    const fn priority(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
            Self::Note => 2,
        }
    }
}

/// Programmatic diagnostic code.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DiagCode {
    /// A required precondition failed before work started.
    PreconditionFailed,
    /// Input topology is non-manifold.
    NonManifoldInput,
    /// A required attribute layer/value was missing.
    MissingRequiredAttribute,
    /// Numeric tolerance/configuration produced an issue.
    NumericToleranceIssue,
    /// Internal invariant was violated.
    InternalInvariantViolation,
    /// Operation was cancelled.
    Cancelled,
    /// Resource/time budget was exceeded.
    BudgetExceeded,
}

/// Topology span associated with a diagnostic.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DiagSpan {
    /// Vertex-local span.
    Vertex(VertexId),
    /// Half-edge-local span.
    HalfEdge(HalfEdgeId),
    /// Face-local span.
    Face(FaceId),
    /// Corner-local span.
    Corner(CornerId),
}

/// Structured diagnostic record.
///
/// Emitted through [`DiagnosticsSink`] and often attached to
/// [`OpError`](crate::OpError) and [`OpReport`](crate::OpReport).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// Severity.
    pub level: DiagLevel,
    /// Programmatic code.
    pub code: DiagCode,
    /// Human-readable description.
    pub message: String,
    /// Topology spans related to this diagnostic.
    pub spans: Vec<DiagSpan>,
}

impl Diagnostic {
    /// Creates a diagnostic with no spans.
    #[must_use]
    pub fn new(level: DiagLevel, code: DiagCode, message: impl Into<String>) -> Self {
        Self {
            level,
            code,
            message: message.into(),
            spans: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Entry {
    seq: u64,
    diagnostic: Diagnostic,
}

impl Entry {
    const fn rank_key(&self) -> (u8, u64) {
        (self.diagnostic.level.priority(), self.seq)
    }
}

/// Default retained diagnostics bound for [`DiagnosticsSink`].
pub const DEFAULT_MAX_DIAGNOSTICS: usize = 64;

/// Bounded sink that retains diagnostics deterministically.
///
/// Overflow retention policy:
/// - keep all `Error` first, then `Warn`, then `Note`
/// - within each level keep earliest insertion order
/// - truncate at `max_diagnostics`
///
/// Stored in [`OpContext::diagnostics`](crate::OpContext::diagnostics).
#[derive(Clone, Debug)]
pub struct DiagnosticsSink {
    max_diagnostics: usize,
    next_seq: u64,
    entries: Vec<Entry>,
}

impl Default for DiagnosticsSink {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_DIAGNOSTICS)
    }
}

impl DiagnosticsSink {
    /// Creates a sink with a maximum number of retained diagnostics.
    #[must_use]
    pub fn new(max_diagnostics: usize) -> Self {
        Self {
            max_diagnostics,
            next_seq: 0,
            entries: Vec::new(),
        }
    }

    /// Returns the capacity bound.
    #[must_use]
    pub const fn max_diagnostics(&self) -> usize {
        self.max_diagnostics
    }

    /// Returns the number of retained diagnostics.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when the sink is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns retained diagnostics in deterministic sink order.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.entries.iter().map(|entry| &entry.diagnostic)
    }

    /// Pushes one diagnostic and applies overflow retention policy.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        if self.max_diagnostics == 0 {
            return;
        }

        let seq = self.next_seq;
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .expect("diagnostic sequence overflowed u64");
        self.entries.push(Entry { seq, diagnostic });
        self.evict_worst_if_needed();
    }

    fn evict_worst_if_needed(&mut self) {
        if self.entries.len() <= self.max_diagnostics {
            return;
        }

        let mut worst_index = 0_usize;
        let mut worst_key = self.entries[0].rank_key();
        for (index, entry) in self.entries.iter().enumerate().skip(1) {
            let key = entry.rank_key();
            if key > worst_key {
                worst_index = index;
                worst_key = key;
            }
        }
        self.entries.remove(worst_index);
        self.entries.sort_unstable_by_key(Entry::rank_key);
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{DEFAULT_MAX_DIAGNOSTICS, DiagCode, DiagLevel, Diagnostic, DiagnosticsSink};

    fn diag(level: DiagLevel, code: DiagCode, message: &str) -> Diagnostic {
        Diagnostic::new(level, code, message)
    }

    #[test]
    fn sink_retains_mixed_severity_with_priority_policy() {
        let mut sink = DiagnosticsSink::new(4);
        sink.push(diag(DiagLevel::Note, DiagCode::Cancelled, "n1"));
        sink.push(diag(DiagLevel::Warn, DiagCode::BudgetExceeded, "w1"));
        sink.push(diag(DiagLevel::Error, DiagCode::PreconditionFailed, "e1"));
        sink.push(diag(DiagLevel::Warn, DiagCode::NonManifoldInput, "w2"));
        sink.push(diag(DiagLevel::Note, DiagCode::Cancelled, "n2"));
        sink.push(diag(
            DiagLevel::Error,
            DiagCode::InternalInvariantViolation,
            "e2",
        ));

        let kept = sink.iter().map(|d| d.message.as_str()).collect::<Vec<_>>();
        assert_eq!(kept, vec!["e1", "e2", "w1", "w2"]);
    }

    #[test]
    fn sink_keeps_earliest_within_same_level() {
        let mut sink = DiagnosticsSink::new(2);
        sink.push(diag(DiagLevel::Warn, DiagCode::BudgetExceeded, "w1"));
        sink.push(diag(DiagLevel::Warn, DiagCode::BudgetExceeded, "w2"));
        sink.push(diag(DiagLevel::Warn, DiagCode::BudgetExceeded, "w3"));

        let kept = sink.iter().map(|d| d.message.as_str()).collect::<Vec<_>>();
        assert_eq!(kept, vec!["w1", "w2"]);
    }

    #[test]
    fn sink_zero_capacity_is_always_empty() {
        let mut sink = DiagnosticsSink::new(0);
        sink.push(diag(DiagLevel::Error, DiagCode::PreconditionFailed, "e1"));
        assert_eq!(sink.len(), 0);
        assert!(sink.is_empty());
        assert!(sink.iter().next().is_none());
    }

    #[test]
    fn sink_default_uses_non_zero_capacity() {
        let sink = DiagnosticsSink::default();
        assert_eq!(sink.max_diagnostics(), DEFAULT_MAX_DIAGNOSTICS);
    }

    #[test]
    fn sink_is_deterministic_across_identical_sequences() {
        let input = [
            diag(DiagLevel::Warn, DiagCode::BudgetExceeded, "w1"),
            diag(DiagLevel::Note, DiagCode::Cancelled, "n1"),
            diag(DiagLevel::Error, DiagCode::PreconditionFailed, "e1"),
            diag(DiagLevel::Note, DiagCode::Cancelled, "n2"),
            diag(DiagLevel::Warn, DiagCode::BudgetExceeded, "w2"),
        ];

        let mut a = DiagnosticsSink::new(3);
        let mut b = DiagnosticsSink::new(3);
        for item in input.clone() {
            a.push(item);
        }
        for item in input {
            b.push(item);
        }

        let a_codes = a.iter().map(|d| d.code).collect::<Vec<_>>();
        let b_codes = b.iter().map(|d| d.code).collect::<Vec<_>>();
        let a_messages = a.iter().map(|d| d.message.clone()).collect::<Vec<_>>();
        let b_messages = b.iter().map(|d| d.message.clone()).collect::<Vec<_>>();
        assert_eq!(a_codes, b_codes);
        assert_eq!(a_messages, b_messages);
    }
}
