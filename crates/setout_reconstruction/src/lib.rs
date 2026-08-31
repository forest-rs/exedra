// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Historical-reconstruction assessment over structural setting-out provenance.
//!
//! This crate owns source catalogues, method warrants, and claim assessment. It
//! explicitly does not mutate [`setout::Evaluation`], choose operative claims,
//! rewrite Joiner evidence, or participate in numeric propagation. A proposal can
//! be presented to a human, but only a new explicit setout decision changes a
//! scenario.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::fmt;

use setout::{
    CanonicalEncoder, ClaimKey, ClaimOrigin, DecisionKey, Fingerprint, MethodId, ProvenanceView,
    RelationKey, RootClaimKey,
};

/// Historical basis of an authored premise.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SourceBasis {
    /// Surviving fabric or direct measurement.
    Observed,
    /// Contemporary or otherwise documentary evidence.
    Documented,
    /// Comparable regional or typological fabric.
    RegionalAnalogy,
    /// Explicit modern engineering inference.
    ModernInference,
}

impl SourceBasis {
    const fn code(self) -> u8 {
        match self {
            Self::Observed => 0,
            Self::Documented => 1,
            Self::RegionalAnalogy => 2,
            Self::ModernInference => 3,
        }
    }
}

/// Stable source annotation for one root claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRef {
    /// Stable source identity shared with external catalogues where practical.
    pub key: Box<str>,
    /// Historical basis represented by the source.
    pub basis: SourceBasis,
    /// Human-readable citation or source locator.
    pub citation: Box<str>,
    /// Scope limitation that prevents the label from claiming too much.
    pub limitation: Box<str>,
}

impl SourceRef {
    /// Creates a source annotation.
    #[must_use]
    pub fn new(
        key: impl Into<Box<str>>,
        basis: SourceBasis,
        citation: impl Into<Box<str>>,
        limitation: impl Into<Box<str>>,
    ) -> Self {
        Self {
            key: key.into(),
            basis,
            citation: citation.into(),
            limitation: limitation.into(),
        }
    }
}

/// Character of a relation method in reconstruction reasoning.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DerivationCharacter {
    /// Transparent exact arithmetic with no additional historical premise.
    Transparent,
    /// Conventional construction interpretation.
    ConstructionConvention,
    /// Modern analytical inference.
    ModernInference,
}

impl DerivationCharacter {
    const fn code(self) -> u8 {
        match self {
            Self::Transparent => 0,
            Self::ConstructionConvention => 1,
            Self::ModernInference => 2,
        }
    }
}

/// Warrant attached to one directed setout method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MethodWarrant {
    /// Stable calculation or convention identity.
    pub key: Box<str>,
    /// Interpretation of the derivation.
    pub character: DerivationCharacter,
    /// Concise scope note.
    pub note: Box<str>,
}

impl MethodWarrant {
    /// Creates a method warrant.
    #[must_use]
    pub fn new(
        key: impl Into<Box<str>>,
        character: DerivationCharacter,
        note: impl Into<Box<str>>,
    ) -> Self {
        Self {
            key: key.into(),
            character,
            note: note.into(),
        }
    }

    /// Creates a transparent exact-arithmetic warrant.
    #[must_use]
    pub fn transparent(key: impl Into<Box<str>>) -> Self {
        Self::new(key, DerivationCharacter::Transparent, "exact arithmetic")
    }
}

/// Stable identity of a directed relation method in the catalogue.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RelationMethodKey {
    /// Relation identity.
    pub relation: RelationKey,
    /// Directed method identity.
    pub method: MethodId,
}

/// Immutable reconstruction annotations keyed only by stable core identity.
#[derive(Clone, Debug)]
pub struct ReconstructionCatalogue {
    roots: BTreeMap<RootClaimKey, SourceRef>,
    methods: BTreeMap<RelationMethodKey, MethodWarrant>,
    decisions: BTreeMap<DecisionKey, Box<str>>,
    fingerprint: Fingerprint,
}

impl ReconstructionCatalogue {
    /// Returns the sidecar-only catalogue fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Finds a source by stable root key.
    #[must_use]
    pub fn root(&self, key: &RootClaimKey) -> Option<&SourceRef> {
        self.roots.get(key)
    }

    /// Finds a warrant by stable relation and method identity.
    #[must_use]
    pub fn method(&self, relation: &RelationKey, method: &MethodId) -> Option<&MethodWarrant> {
        self.methods.get(&RelationMethodKey {
            relation: relation.clone(),
            method: method.clone(),
        })
    }
}

/// Builder for a [`ReconstructionCatalogue`].
#[derive(Debug, Default)]
pub struct ReconstructionCatalogueBuilder {
    roots: BTreeMap<RootClaimKey, SourceRef>,
    methods: BTreeMap<RelationMethodKey, MethodWarrant>,
    decisions: BTreeMap<DecisionKey, Box<str>>,
}

impl ReconstructionCatalogueBuilder {
    /// Starts an empty catalogue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Annotates one root claim.
    pub fn root(mut self, key: RootClaimKey, source: SourceRef) -> Result<Self, CatalogueError> {
        if self.roots.insert(key, source).is_some() {
            return Err(CatalogueError::DuplicateEntry);
        }
        Ok(self)
    }

    /// Annotates one directed relation method.
    pub fn method(
        mut self,
        relation: RelationKey,
        method: MethodId,
        warrant: MethodWarrant,
    ) -> Result<Self, CatalogueError> {
        if self
            .methods
            .insert(RelationMethodKey { relation, method }, warrant)
            .is_some()
        {
            return Err(CatalogueError::DuplicateEntry);
        }
        Ok(self)
    }

    /// Adds an application-owned justification for an explicit decision.
    pub fn decision(
        mut self,
        decision: DecisionKey,
        justification: impl Into<Box<str>>,
    ) -> Result<Self, CatalogueError> {
        if self
            .decisions
            .insert(decision, justification.into())
            .is_some()
        {
            return Err(CatalogueError::DuplicateEntry);
        }
        Ok(self)
    }

    /// Canonicalizes and freezes the catalogue.
    #[must_use]
    pub fn finish(self) -> ReconstructionCatalogue {
        let mut encoder = CanonicalEncoder::new("setout-reconstruction/catalogue");
        for (root, source) in &self.roots {
            encoder.u8(0);
            encoder.str(root.as_str());
            encode_source(source, &mut encoder);
        }
        for (method, warrant) in &self.methods {
            encoder.u8(1);
            encoder.str(method.relation.as_str());
            encoder.str(method.method.as_str());
            encode_warrant(warrant, &mut encoder);
        }
        for (decision, justification) in &self.decisions {
            encoder.u8(2);
            encoder.str(decision.as_str());
            encoder.str(justification);
        }
        ReconstructionCatalogue {
            roots: self.roots,
            methods: self.methods,
            decisions: self.decisions,
            fingerprint: encoder.finish(),
        }
    }
}

/// Catalogue construction failure.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CatalogueError {
    /// Stable identity already has an annotation.
    DuplicateEntry,
}

impl fmt::Display for CatalogueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("catalogue already contains this stable identity")
    }
}

impl core::error::Error for CatalogueError {}

/// Reconstruction interpretation of one core claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimAssessment {
    /// Assessed core claim.
    pub claim: ClaimKey,
    /// Weakest source basis in its structural support, if annotated.
    pub limiting_basis: Option<SourceBasis>,
    /// Root premises responsible for the limiting basis.
    pub limiting_roots: Box<[RootClaimKey]>,
    /// Most interpretive derivation encountered in its ancestry.
    pub derivation: DerivationCharacter,
    /// Whether every root and relation in the ancestry was annotated.
    pub complete: bool,
}

/// Finding local to reconstruction analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReconstructionFinding {
    /// Core root has no source annotation.
    UnannotatedRoot(RootClaimKey),
    /// Core relation method has no warrant.
    UnannotatedMethod(RelationMethodKey),
    /// A selected decision has no application-owned justification.
    UnjustifiedDecision(DecisionKey),
}

/// Immutable sidecar assessment over one evaluation.
#[derive(Clone, Debug)]
pub struct ReconstructionAssessment {
    /// Assessments by stable core claim key.
    pub claims: BTreeMap<ClaimKey, ClaimAssessment>,
    /// Sorted, deduplicated analysis findings.
    pub findings: Box<[ReconstructionFinding]>,
    /// Fingerprint that changes with evaluation content, catalogue, or policy.
    pub fingerprint: Fingerprint,
}

/// Assesses all claims without changing core evaluation state.
#[must_use]
pub fn assess(
    provenance: ProvenanceView<'_>,
    evaluation_fingerprint: Fingerprint,
    catalogue: &ReconstructionCatalogue,
) -> ReconstructionAssessment {
    let mut claims = BTreeMap::new();
    let mut findings = Vec::new();
    for claim in provenance.claims() {
        let mut visited = BTreeSet::new();
        let assessment = assess_claim(
            provenance,
            claim.key(),
            catalogue,
            &mut visited,
            &mut findings,
        );
        claims.insert(claim.key(), assessment);
    }
    findings.sort_by_key(format_finding);
    findings.dedup();

    let mut encoder = CanonicalEncoder::new("setout-reconstruction/assessment");
    encoder.fingerprint(evaluation_fingerprint);
    encoder.fingerprint(catalogue.fingerprint());
    for assessment in claims.values() {
        encoder.fingerprint(assessment.claim.fingerprint());
        encoder.u8(assessment.limiting_basis.map_or(u8::MAX, SourceBasis::code));
        encoder.u8(assessment.derivation.code());
        encoder.bool(assessment.complete);
        for root in &assessment.limiting_roots {
            encoder.str(root.as_str());
        }
    }
    for finding in &findings {
        encoder.str(&format_finding(finding));
    }
    ReconstructionAssessment {
        claims,
        findings: findings.into_boxed_slice(),
        fingerprint: encoder.finish(),
    }
}

fn assess_claim(
    provenance: ProvenanceView<'_>,
    key: ClaimKey,
    catalogue: &ReconstructionCatalogue,
    visited: &mut BTreeSet<ClaimKey>,
    findings: &mut Vec<ReconstructionFinding>,
) -> ClaimAssessment {
    if !visited.insert(key) {
        return ClaimAssessment {
            claim: key,
            limiting_basis: None,
            limiting_roots: Box::new([]),
            derivation: DerivationCharacter::ModernInference,
            complete: false,
        };
    }
    let assessment = if let Some(claim) = provenance.claim(key) {
        match claim.origin() {
            ClaimOrigin::Root { key: root, .. } => match catalogue.root(root) {
                Some(source) => ClaimAssessment {
                    claim: key,
                    limiting_basis: Some(source.basis),
                    limiting_roots: Box::new([root.clone()]),
                    derivation: DerivationCharacter::Transparent,
                    complete: true,
                },
                None => {
                    findings.push(ReconstructionFinding::UnannotatedRoot(root.clone()));
                    ClaimAssessment {
                        claim: key,
                        limiting_basis: None,
                        limiting_roots: Box::new([root.clone()]),
                        derivation: DerivationCharacter::Transparent,
                        complete: false,
                    }
                }
            },
            ClaimOrigin::Relation {
                relation,
                method,
                inputs,
            } => {
                let warrant = catalogue.method(relation, method);
                if warrant.is_none() {
                    findings.push(ReconstructionFinding::UnannotatedMethod(
                        RelationMethodKey {
                            relation: relation.clone(),
                            method: method.clone(),
                        },
                    ));
                }
                let mut input_assessments = Vec::new();
                for input in inputs {
                    input_assessments.push(assess_claim(
                        provenance, *input, catalogue, visited, findings,
                    ));
                }
                let limiting_basis = input_assessments
                    .iter()
                    .filter_map(|assessment| assessment.limiting_basis)
                    .max();
                let mut limiting_roots: Vec<_> = input_assessments
                    .iter()
                    .filter(|assessment| assessment.limiting_basis == limiting_basis)
                    .flat_map(|assessment| assessment.limiting_roots.iter().cloned())
                    .collect();
                limiting_roots.sort();
                limiting_roots.dedup();
                let derivation = input_assessments
                    .iter()
                    .map(|assessment| assessment.derivation)
                    .chain(warrant.map(|value| value.character))
                    .max()
                    .unwrap_or(DerivationCharacter::ModernInference);
                ClaimAssessment {
                    claim: key,
                    limiting_basis,
                    limiting_roots: limiting_roots.into_boxed_slice(),
                    derivation,
                    complete: warrant.is_some()
                        && input_assessments
                            .iter()
                            .all(|assessment| assessment.complete),
                }
            }
            ClaimOrigin::Resolution { decision, selected } => {
                if !catalogue.decisions.contains_key(decision) {
                    findings.push(ReconstructionFinding::UnjustifiedDecision(decision.clone()));
                }
                let selected = assess_claim(provenance, *selected, catalogue, visited, findings);
                ClaimAssessment {
                    claim: key,
                    complete: selected.complete && catalogue.decisions.contains_key(decision),
                    ..selected
                }
            }
            _ => ClaimAssessment {
                claim: key,
                limiting_basis: None,
                limiting_roots: Box::new([]),
                derivation: DerivationCharacter::ModernInference,
                complete: false,
            },
        }
    } else {
        ClaimAssessment {
            claim: key,
            limiting_basis: None,
            limiting_roots: Box::new([]),
            derivation: DerivationCharacter::ModernInference,
            complete: false,
        }
    };
    // `visited` is an ancestry stack, not a global seen set. Shared support in
    // a provenance DAG is valid and must be assessed along both branches;
    // only a key already on the current recursive path indicates a cycle.
    visited.remove(&key);
    assessment
}

/// Sidecar-only changes between two assessments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisDelta {
    /// Core claims whose assessment changed.
    pub claims_reassessed: Box<[ClaimKey]>,
    /// Whether the finding set changed.
    pub findings_changed: bool,
}

impl ReconstructionAssessment {
    /// Compares two assessments without implying a core evaluation delta.
    #[must_use]
    pub fn delta_from(&self, previous: &Self) -> AnalysisDelta {
        let claims_reassessed = self
            .claims
            .keys()
            .chain(previous.claims.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|key| self.claims.get(key) != previous.claims.get(key))
            .collect();
        AnalysisDelta {
            claims_reassessed,
            findings_changed: self.findings != previous.findings,
        }
    }
}

fn encode_source(source: &SourceRef, encoder: &mut CanonicalEncoder) {
    encoder.str(&source.key);
    encoder.u8(source.basis.code());
    encoder.str(&source.citation);
    encoder.str(&source.limitation);
}

fn encode_warrant(warrant: &MethodWarrant, encoder: &mut CanonicalEncoder) {
    encoder.str(&warrant.key);
    encoder.u8(warrant.character.code());
    encoder.str(&warrant.note);
}

fn format_finding(finding: &ReconstructionFinding) -> alloc::string::String {
    match finding {
        ReconstructionFinding::UnannotatedRoot(root) => alloc::format!("root:{root}"),
        ReconstructionFinding::UnannotatedMethod(method) => {
            alloc::format!("method:{}:{}", method.relation, method.method)
        }
        ReconstructionFinding::UnjustifiedDecision(decision) => {
            alloc::format!("decision:{decision}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use setout::{
        AdjustLength, EvaluationScenarioBuilder, Knowledge, Length, NetworkBuilder, Offset,
        QuantityPolicy, RootClaimKey, RootClaimSetBuilder, Sum, compile_plan, evaluate,
    };

    #[test]
    fn shared_provenance_is_a_complete_dag_not_a_false_cycle() {
        let mut network = NetworkBuilder::new();
        let base = network
            .declare::<Length>("roof/base", QuantityPolicy::positive())
            .unwrap();
        let left = network
            .declare::<Length>("roof/left", QuantityPolicy::positive())
            .unwrap();
        let right = network
            .declare::<Length>("roof/right", QuantityPolicy::positive())
            .unwrap();
        let total = network
            .declare::<Length>("roof/total", QuantityPolicy::positive())
            .unwrap();
        network
            .relate(
                AdjustLength::new(
                    "roof/a-left",
                    base.clone(),
                    left.clone(),
                    Offset::millimeters(10).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        network
            .relate(
                AdjustLength::new(
                    "roof/b-right",
                    base.clone(),
                    right.clone(),
                    Offset::millimeters(20).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        network
            .relate(Sum::new("roof/c-total", left, right, total.clone()).unwrap())
            .unwrap();
        let definition = network.finish().unwrap();
        let mut roots = RootClaimSetBuilder::new(&definition);
        roots
            .author(
                "root/base",
                &base,
                Knowledge::exact(Length::millimeters(100).unwrap()),
            )
            .unwrap();
        let roots = roots.finish().unwrap();
        let scenario = EvaluationScenarioBuilder::new("roof/assessment")
            .unwrap()
            .activate_all(&roots)
            .finish(&roots)
            .unwrap();
        let plan = compile_plan(&definition, &roots, &scenario).unwrap();
        let evaluation = evaluate(&definition, &roots, &scenario, &plan).unwrap();

        let mut catalogue = ReconstructionCatalogueBuilder::new()
            .root(
                RootClaimKey::new("root/base").unwrap(),
                SourceRef::new(
                    "survey/base",
                    SourceBasis::Observed,
                    "survey record",
                    "controls this fixture only",
                ),
            )
            .unwrap();
        for step in plan.steps() {
            catalogue = catalogue
                .method(
                    step.relation.clone(),
                    step.method.clone(),
                    MethodWarrant::transparent("exact fixture arithmetic"),
                )
                .unwrap();
        }
        let assessment = assess(
            evaluation.provenance(),
            evaluation.fingerprint(),
            &catalogue.finish(),
        );
        let total_claim = evaluation
            .provenance()
            .operative(total.key())
            .expect("total is derived")
            .key();
        let total = assessment
            .claims
            .get(&total_claim)
            .expect("derived total is assessed");

        // The observed base supports both branches of the diamond. Seeing it
        // twice is shared ancestry, not recursion, so completeness remains true.
        assert!(total.complete);
        assert_eq!(total.derivation, DerivationCharacter::Transparent);
        assert_eq!(total.limiting_basis, Some(SourceBasis::Observed));
        assert_eq!(
            total.limiting_roots.as_ref(),
            [RootClaimKey::new("root/base").unwrap()]
        );
        assert!(assessment.findings.is_empty());
    }
}
