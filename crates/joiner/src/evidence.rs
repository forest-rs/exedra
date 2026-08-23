// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Evidence labelling: what a construction claim is based on.
//!
//! Every element, relation, contact, and rule application carries an
//! [`Evidence`] label: a named source key plus the [`EvidenceClass`] of the
//! claim. The class travels from day one because it is cheap to carry and
//! expensive to retrofit — without it a modern inference is indistinguishable
//! from observed fabric once it has been drawn.
//!
//! `joiner` validates only that a label names a registered source and that
//! the label's class matches that source's class. What the sources *are* is
//! the frontend's business.

use alloc::string::{String, ToString};

/// How well founded a construction claim is.
///
/// The four classes are ordered by strength of evidence, strongest first.
/// They are a vocabulary, not a confidence score: nothing in this crate
/// arithmetically combines them.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum EvidenceClass {
    /// Directly observed in surviving fabric.
    Observed,
    /// Reconstructed from documentary records of something now lost.
    DocumentedReconstruction,
    /// Taken from comparable construction in the same region or tradition.
    RegionalAnalogy,
    /// Inferred from modern engineering practice, not from historical
    /// evidence.
    ModernEngineeringInference,
}

impl EvidenceClass {
    /// The stable lowercase label used in diagnostics and metadata.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::DocumentedReconstruction => "documented-reconstruction",
            Self::RegionalAnalogy => "regional-analogy",
            Self::ModernEngineeringInference => "modern-engineering-inference",
        }
    }
}

impl core::fmt::Display for EvidenceClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

/// A named, citable basis for construction claims.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSource {
    /// Stable frontend-supplied key; unique within a construction.
    pub key: String,
    /// The strength of everything cited to this source.
    pub class: EvidenceClass,
    /// Where the source can be read. Must be non-empty.
    pub url: String,
    /// What the source actually establishes, and what it does not. Must be
    /// non-empty: an unqualified citation is how an analogy becomes a claim.
    pub note: String,
}

impl EvidenceSource {
    /// Registers a source under `key`.
    #[must_use]
    pub fn new(key: &str, class: EvidenceClass, url: &str, note: &str) -> Self {
        Self {
            key: key.to_string(),
            class,
            url: url.to_string(),
            note: note.to_string(),
        }
    }
}

/// A claim's evidence label: which source, and at what class.
///
/// Validation rejects a label whose class disagrees with its source's, so a
/// record cannot quietly upgrade an analogy into an observation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Evidence {
    /// The [`EvidenceSource::key`] this claim cites.
    pub source: String,
    /// The class claimed here; must equal the source's class.
    pub class: EvidenceClass,
}

impl Evidence {
    /// Labels a claim with `source` at `class`.
    #[must_use]
    pub fn new(source: &str, class: EvidenceClass) -> Self {
        Self {
            source: source.to_string(),
            class,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::*;

    #[test]
    fn classes_are_ordered_strongest_first_and_labelled_stably() {
        assert!(EvidenceClass::Observed < EvidenceClass::ModernEngineeringInference);
        assert_eq!(
            format!("{}", EvidenceClass::RegionalAnalogy),
            "regional-analogy"
        );
        let source = EvidenceSource::new(
            "example-survey",
            EvidenceClass::RegionalAnalogy,
            "https://example.invalid/survey",
            "Comparable construction, not this building",
        );
        assert_eq!(
            Evidence::new("example-survey", EvidenceClass::RegionalAnalogy).class,
            source.class
        );
    }
}
