// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Root claims, deterministic plans, evaluation, provenance, and deltas.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::fingerprint::{CanonicalEncoder, Fingerprint};
use crate::identity::{
    Candidate, CandidateKey, ClaimFingerprint, ClaimId, ClaimKey, ClaimOrigin, ClaimProducer,
    ClaimSelection, Decision, DecisionAction, Knowledge, RootSource, SupportAtom, SupportKey,
    SupportRef,
};
use crate::key::{
    DecisionKey, KeyError, MethodId, QuantityKey, RelationKey, RootClaimKey, ScenarioKey,
};
use crate::network::{AnyQuantity, MethodDef, NetworkDef, Quantity, QuantitySlot, SolveResult};
use crate::value::{ArithmeticError, Domain, DomainTag, ExactnessTrace, RootQuantization, Value};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ErasedCandidate {
    key: CandidateKey,
    value: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ErasedKnowledge {
    Exact(Value),
    Discrete(Box<[ErasedCandidate]>),
}

impl ErasedKnowledge {
    fn encode(&self, encoder: &mut CanonicalEncoder) {
        match self {
            Self::Exact(value) => {
                encoder.u8(0);
                value.encode(encoder);
            }
            Self::Discrete(candidates) => {
                encoder.u8(1);
                encoder.u32(u32::try_from(candidates.len()).expect("candidate count is bounded"));
                for candidate in candidates {
                    encoder.fingerprint(candidate.key.fingerprint());
                    candidate.value.encode(encoder);
                }
            }
        }
    }

    fn exact(&self) -> Option<&Value> {
        match self {
            Self::Exact(value) => Some(value),
            Self::Discrete(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
struct RootClaimDef {
    key: RootClaimKey,
    quantity: AnyQuantity,
    knowledge: ErasedKnowledge,
    source: RootSource,
    exactness: ExactnessTrace,
    claim_key: ClaimKey,
    fingerprint: ClaimFingerprint,
}

/// Immutable collection of root claims for a network definition.
#[derive(Clone, Debug)]
pub struct RootClaimSet {
    definition: Fingerprint,
    claims: BTreeMap<RootClaimKey, RootClaimDef>,
    fingerprint: Fingerprint,
}

impl RootClaimSet {
    /// Returns the definition this root set was checked against.
    #[must_use]
    pub const fn definition(&self) -> Fingerprint {
        self.definition
    }

    /// Returns the canonical root-content fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Iterates stable root identities in canonical order.
    pub fn keys(&self) -> impl Iterator<Item = &RootClaimKey> {
        self.claims.keys()
    }

    /// Returns the structural claim identity produced by a root.
    #[must_use]
    pub fn claim_key(&self, root: &RootClaimKey) -> Option<ClaimKey> {
        self.claims.get(root).map(|claim| claim.claim_key)
    }
}

/// Builder for root claims checked against one immutable definition.
#[derive(Debug)]
pub struct RootClaimSetBuilder<'definition> {
    definition: &'definition NetworkDef,
    claims: BTreeMap<RootClaimKey, RootClaimDef>,
}

impl<'definition> RootClaimSetBuilder<'definition> {
    /// Starts a root set for `definition`.
    #[must_use]
    pub fn new(definition: &'definition NetworkDef) -> Self {
        Self {
            definition,
            claims: BTreeMap::new(),
        }
    }

    /// Adds an authored root with exact structural provenance.
    pub fn author<T: Domain>(
        &mut self,
        key: &str,
        quantity: &Quantity<T>,
        knowledge: Knowledge<T>,
    ) -> Result<&mut Self, RootBuildError> {
        self.insert(
            key,
            quantity,
            knowledge,
            RootSource::Authored,
            ExactnessTrace::Exact,
        )
    }

    /// Adds a legacy floating root together with its one-time import trace.
    pub fn author_quantized<T: Domain>(
        &mut self,
        key: &str,
        quantity: &Quantity<T>,
        knowledge: Knowledge<T>,
        quantization: RootQuantization,
    ) -> Result<&mut Self, RootBuildError> {
        self.insert(
            key,
            quantity,
            knowledge,
            RootSource::Authored,
            ExactnessTrace::ImportedFloat(quantization),
        )
    }

    /// Adds an imported root from a stable external item.
    pub fn import<T: Domain>(
        &mut self,
        key: &str,
        quantity: &Quantity<T>,
        knowledge: Knowledge<T>,
        importer: impl Into<Box<str>>,
        item: impl Into<Box<str>>,
    ) -> Result<&mut Self, RootBuildError> {
        self.insert(
            key,
            quantity,
            knowledge,
            RootSource::Imported {
                importer: importer.into(),
                item: item.into(),
            },
            ExactnessTrace::Exact,
        )
    }

    fn insert<T: Domain>(
        &mut self,
        key: &str,
        quantity: &Quantity<T>,
        knowledge: Knowledge<T>,
        source: RootSource,
        exactness: ExactnessTrace,
    ) -> Result<&mut Self, RootBuildError> {
        let root_key = RootClaimKey::new(key)?;
        if self.claims.contains_key(&root_key) {
            return Err(RootBuildError::DuplicateRoot(root_key));
        }
        let Some(definition_quantity) = self.definition.quantity(quantity.key()) else {
            return Err(RootBuildError::ForeignQuantity);
        };
        if definition_quantity.slot() != quantity.slot() || definition_quantity.domain() != T::TAG {
            return Err(RootBuildError::ForeignQuantity);
        }

        let knowledge = erase_knowledge(knowledge)?;
        for value in knowledge_values(&knowledge) {
            if !self
                .definition
                .quantity_def(quantity.slot())
                .policy
                .accepts(value)
            {
                return Err(RootBuildError::InadmissibleValue(quantity.key().clone()));
            }
        }
        let claim_key = root_claim_key(quantity.key(), T::TAG, &root_key);
        let fingerprint = root_claim_fingerprint(claim_key, &knowledge, &source, &exactness);
        self.claims.insert(
            root_key.clone(),
            RootClaimDef {
                key: root_key,
                quantity: quantity.erase(),
                knowledge,
                source,
                exactness,
                claim_key,
                fingerprint,
            },
        );
        Ok(self)
    }

    /// Canonicalizes and freezes all roots.
    pub fn finish(self) -> Result<RootClaimSet, RootBuildError> {
        let mut encoder = CanonicalEncoder::new("setout/root-claim-set");
        encoder.fingerprint(self.definition.fingerprint());
        encoder.u32(u32::try_from(self.claims.len()).map_err(|_| RootBuildError::TooManyRoots)?);
        for root in self.claims.values() {
            encoder.str(root.key.as_str());
            encoder.fingerprint(root.fingerprint.fingerprint());
        }
        Ok(RootClaimSet {
            definition: self.definition.fingerprint(),
            claims: self.claims,
            fingerprint: encoder.finish(),
        })
    }
}

fn erase_knowledge<T: Domain>(knowledge: Knowledge<T>) -> Result<ErasedKnowledge, RootBuildError> {
    match knowledge {
        Knowledge::Exact(value) => Ok(ErasedKnowledge::Exact(Value::from_domain(value))),
        Knowledge::Discrete { candidates } => {
            if candidates.is_empty() {
                return Err(RootBuildError::EmptyCandidates);
            }
            let mut candidates: Vec<_> = candidates
                .into_vec()
                .into_iter()
                .map(|Candidate { key, value }| ErasedCandidate {
                    key,
                    value: Value::from_domain(value),
                })
                .collect();
            candidates.sort_by_key(|candidate| candidate.key);
            if candidates.windows(2).any(|pair| pair[0].key == pair[1].key) {
                return Err(RootBuildError::DuplicateCandidate);
            }
            Ok(ErasedKnowledge::Discrete(candidates.into_boxed_slice()))
        }
    }
}

fn knowledge_values(knowledge: &ErasedKnowledge) -> Box<dyn Iterator<Item = &Value> + '_> {
    match knowledge {
        ErasedKnowledge::Exact(value) => Box::new(core::iter::once(value)),
        ErasedKnowledge::Discrete(candidates) => {
            Box::new(candidates.iter().map(|candidate| &candidate.value))
        }
    }
}

/// Error that prevents construction of an immutable root set.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RootBuildError {
    /// A root key is invalid.
    InvalidKey(KeyError),
    /// A stable root key appears more than once.
    DuplicateRoot(RootClaimKey),
    /// The typed handle belongs to another definition.
    ForeignQuantity,
    /// A value violates the quantity's admissibility policy.
    InadmissibleValue(QuantityKey),
    /// A discrete claim has no candidates.
    EmptyCandidates,
    /// Two candidates share stable identity.
    DuplicateCandidate,
    /// The root set exceeds a stable `u32` storage boundary.
    TooManyRoots,
}

impl From<KeyError> for RootBuildError {
    fn from(error: KeyError) -> Self {
        Self::InvalidKey(error)
    }
}

impl fmt::Display for RootBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey(error) => write!(formatter, "invalid root key: {error}"),
            Self::DuplicateRoot(key) => write!(formatter, "duplicate root `{key}`"),
            Self::ForeignQuantity => formatter.write_str("root references a foreign quantity"),
            Self::InadmissibleValue(key) => {
                write!(formatter, "root value is inadmissible for `{key}`")
            }
            Self::EmptyCandidates => formatter.write_str("discrete root has no candidates"),
            Self::DuplicateCandidate => {
                formatter.write_str("discrete root repeats a candidate key")
            }
            Self::TooManyRoots => formatter.write_str("root set exceeds u32 item capacity"),
        }
    }
}

impl core::error::Error for RootBuildError {}

/// Immutable choice of active roots and explicit decisions.
#[derive(Clone, Debug)]
pub struct EvaluationScenario {
    /// Stable scenario identity.
    pub key: ScenarioKey,
    active_external_roots: Box<[RootClaimKey]>,
    decisions: Box<[Decision]>,
    fingerprint: Fingerprint,
}

impl EvaluationScenario {
    /// Returns active root keys in canonical order.
    #[must_use]
    pub fn active_external_roots(&self) -> &[RootClaimKey] {
        &self.active_external_roots
    }

    /// Returns decisions in canonical key order.
    #[must_use]
    pub fn decisions(&self) -> &[Decision] {
        &self.decisions
    }

    /// Returns the scenario-content fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

/// Builder for an [`EvaluationScenario`].
#[derive(Debug)]
pub struct EvaluationScenarioBuilder {
    key: ScenarioKey,
    active: BTreeSet<RootClaimKey>,
    decisions: BTreeMap<DecisionKey, Decision>,
}

impl EvaluationScenarioBuilder {
    /// Starts a named evaluation scenario.
    pub fn new(key: &str) -> Result<Self, ScenarioBuildError> {
        Ok(Self {
            key: ScenarioKey::new(key)?,
            active: BTreeSet::new(),
            decisions: BTreeMap::new(),
        })
    }

    /// Activates every root in the supplied root set.
    #[must_use]
    pub fn activate_all(mut self, roots: &RootClaimSet) -> Self {
        self.active.extend(roots.keys().cloned());
        self
    }

    /// Activates one known external root.
    pub fn activate(mut self, key: &str) -> Result<Self, ScenarioBuildError> {
        self.active.insert(RootClaimKey::new(key)?);
        Ok(self)
    }

    /// Adds one explicit decision.
    pub fn decide(mut self, decision: Decision) -> Result<Self, ScenarioBuildError> {
        if self
            .decisions
            .insert(decision.key.clone(), decision)
            .is_some()
        {
            return Err(ScenarioBuildError::DuplicateDecision);
        }
        Ok(self)
    }

    /// Validates active roots and freezes the scenario.
    pub fn finish(self, roots: &RootClaimSet) -> Result<EvaluationScenario, ScenarioBuildError> {
        if self
            .active
            .iter()
            .any(|key| !roots.claims.contains_key(key))
        {
            return Err(ScenarioBuildError::UnknownRoot);
        }
        let active_external_roots: Box<[_]> = self.active.into_iter().collect();
        let decisions: Box<[_]> = self.decisions.into_values().collect();
        let mut encoder = CanonicalEncoder::new("setout/evaluation-scenario");
        encoder.str(self.key.as_str());
        encoder.u32(
            u32::try_from(active_external_roots.len())
                .map_err(|_| ScenarioBuildError::TooManyItems)?,
        );
        for root in &active_external_roots {
            encoder.str(root.as_str());
        }
        encoder.u32(u32::try_from(decisions.len()).map_err(|_| ScenarioBuildError::TooManyItems)?);
        for decision in &decisions {
            encode_decision(decision, &mut encoder);
        }
        Ok(EvaluationScenario {
            key: self.key,
            active_external_roots,
            decisions,
            fingerprint: encoder.finish(),
        })
    }
}

/// Error that prevents construction of a scenario.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ScenarioBuildError {
    /// A scenario or root key is invalid.
    InvalidKey(KeyError),
    /// An active root is absent from the supplied root set.
    UnknownRoot,
    /// A decision key appears more than once.
    DuplicateDecision,
    /// The scenario exceeds a stable `u32` storage boundary.
    TooManyItems,
}

impl From<KeyError> for ScenarioBuildError {
    fn from(error: KeyError) -> Self {
        Self::InvalidKey(error)
    }
}

impl fmt::Display for ScenarioBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey(error) => write!(formatter, "invalid scenario key: {error}"),
            Self::UnknownRoot => formatter.write_str("scenario activates an unknown root"),
            Self::DuplicateDecision => formatter.write_str("scenario repeats a decision key"),
            Self::TooManyItems => formatter.write_str("scenario exceeds u32 item capacity"),
        }
    }
}

impl core::error::Error for ScenarioBuildError {}

/// One deterministic directed propagation or agreement-check step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanStep {
    /// Stable relation identity.
    pub relation: RelationKey,
    /// Stable directed method identity.
    pub method: MethodId,
    /// Quantity this method produces.
    pub target: QuantityKey,
    /// Ordered input quantities.
    pub inputs: Box<[QuantityKey]>,
    /// Whether the target already had an operative claim when planning.
    pub check: bool,
    /// Decision promoted this producer to the operative path, if any.
    pub resolution: Option<DecisionKey>,
    relation_index: u32,
    method_index: u32,
}

/// Immutable deterministic propagation plan.
#[derive(Clone, Debug)]
pub struct PropagationPlan {
    definition: Fingerprint,
    scenario_shape: Fingerprint,
    steps: Box<[PlanStep]>,
    unknowns: Box<[QuantityKey]>,
    fingerprint: Fingerprint,
}

impl PropagationPlan {
    /// Returns directed steps in execution order.
    #[must_use]
    pub fn steps(&self) -> &[PlanStep] {
        &self.steps
    }

    /// Returns quantities that remain structurally under-determined.
    #[must_use]
    pub fn unknowns(&self) -> &[QuantityKey] {
        &self.unknowns
    }

    /// Returns the canonical plan fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

/// Compiles a deterministic plan from root availability and explicit producer decisions.
pub fn compile_plan(
    definition: &NetworkDef,
    roots: &RootClaimSet,
    scenario: &EvaluationScenario,
) -> Result<PropagationPlan, PlanError> {
    if roots.definition != definition.fingerprint() {
        return Err(PlanError::DefinitionMismatch);
    }
    let mut ready = vec![false; definition.quantity_count()];
    for root_key in scenario.active_external_roots() {
        let root = roots.claims.get(root_key).ok_or(PlanError::UnknownRoot)?;
        match &root.knowledge {
            ErasedKnowledge::Exact(_) => ready[root.quantity.slot().index()] = true,
            ErasedKnowledge::Discrete(_) => {
                if scenario
                    .decisions()
                    .iter()
                    .any(|decision| candidate_decision_matches_root(decision, root))
                {
                    ready[root.quantity.slot().index()] = true;
                }
            }
        }
    }

    let forced: BTreeMap<_, _> = scenario
        .decisions()
        .iter()
        .filter_map(|decision| match &decision.action {
            DecisionAction::SelectClaim {
                quantity,
                selection:
                    ClaimSelection {
                        producer: ClaimProducer::Relation { relation, method },
                        ..
                    },
            } => Some((
                (relation.clone(), method.clone()),
                (decision.key.clone(), quantity.clone()),
            )),
            _ => None,
        })
        .collect();

    let mut consumed = vec![false; definition.relations.len()];
    let mut steps = Vec::new();
    loop {
        let mut progress = false;
        for (relation_index, relation) in definition.relations.iter().enumerate() {
            if consumed[relation_index] {
                continue;
            }
            let forced_method = relation
                .methods
                .iter()
                .enumerate()
                .find_map(|(index, method)| {
                    forced
                        .get(&(relation.key.clone(), method.id.clone()))
                        .filter(|(_, quantity)| {
                            definition.quantity_def(method.target).quantity.key() == quantity
                        })
                        .map(|(decision, _)| (index, method, decision.clone()))
                });
            if let Some((method_index, method, decision)) = forced_method {
                if method.inputs.iter().all(|slot| ready[slot.index()]) {
                    let check = ready[method.target.index()];
                    steps.push(public_plan_step(
                        definition,
                        relation_index,
                        method_index,
                        method,
                        check,
                        Some(decision),
                    )?);
                    ready[method.target.index()] = true;
                    consumed[relation_index] = true;
                    progress = true;
                }
                continue;
            }

            // One multi-way relation may discover several independent targets.
            // In particular, a rooted Point3 must decompose into x, y, and z;
            // consuming the relation after the first component silently left
            // the other two unknown. Collect before mutating readiness so each
            // target is judged against the same input frontier.
            let mut discovered_targets = BTreeSet::new();
            let discoveries: Vec<_> = relation
                .methods
                .iter()
                .enumerate()
                .filter(|(_, method)| {
                    !ready[method.target.index()]
                        && method.inputs.iter().all(|slot| ready[slot.index()])
                        && discovered_targets.insert(method.target)
                })
                .map(|(index, _)| index)
                .collect();
            if !discoveries.is_empty() {
                for method_index in discoveries {
                    let method = &relation.methods[method_index];
                    steps.push(public_plan_step(
                        definition,
                        relation_index,
                        method_index,
                        method,
                        false,
                        None,
                    )?);
                    ready[method.target.index()] = true;
                }
                consumed[relation_index] = true;
                progress = true;
                continue;
            }

            // A fully-known relation still contributes one deterministic
            // agreement check, never a redundant check for every inverse form.
            let check = relation
                .methods
                .iter()
                .enumerate()
                .filter(|(_, method)| method.inputs.iter().all(|slot| ready[slot.index()]))
                .min_by_key(|(_, method)| method.id.clone());
            if let Some((method_index, method)) = check {
                steps.push(public_plan_step(
                    definition,
                    relation_index,
                    method_index,
                    method,
                    true,
                    None,
                )?);
                consumed[relation_index] = true;
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }

    let unknowns: Box<[_]> = definition
        .quantities
        .iter()
        .filter(|quantity| !ready[quantity.quantity.slot().index()])
        .map(|quantity| quantity.quantity.key().clone())
        .collect();
    let scenario_shape = scenario_shape_fingerprint(definition, roots, scenario);
    let mut encoder = CanonicalEncoder::new("setout/propagation-plan");
    encoder.fingerprint(definition.fingerprint());
    encoder.fingerprint(scenario_shape);
    encoder.u32(u32::try_from(steps.len()).map_err(|_| PlanError::TooManyItems)?);
    for step in &steps {
        encode_plan_step(step, &mut encoder);
    }
    encoder.u32(u32::try_from(unknowns.len()).map_err(|_| PlanError::TooManyItems)?);
    for unknown in &unknowns {
        encoder.str(unknown.as_str());
    }
    Ok(PropagationPlan {
        definition: definition.fingerprint(),
        scenario_shape,
        steps: steps.into_boxed_slice(),
        unknowns,
        fingerprint: encoder.finish(),
    })
}

fn public_plan_step(
    definition: &NetworkDef,
    relation_index: usize,
    method_index: usize,
    method: &MethodDef,
    check: bool,
    resolution: Option<DecisionKey>,
) -> Result<PlanStep, PlanError> {
    let relation = &definition.relations[relation_index];
    Ok(PlanStep {
        relation: relation.key.clone(),
        method: method.id.clone(),
        target: definition
            .quantity_def(method.target)
            .quantity
            .key()
            .clone(),
        inputs: method
            .inputs
            .iter()
            .map(|slot| definition.quantity_def(*slot).quantity.key().clone())
            .collect(),
        check,
        resolution,
        relation_index: u32::try_from(relation_index).map_err(|_| PlanError::TooManyItems)?,
        method_index: u32::try_from(method_index).map_err(|_| PlanError::TooManyItems)?,
    })
}

/// Failure to compile a valid propagation plan.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PlanError {
    /// Roots were built for another definition.
    DefinitionMismatch,
    /// A scenario refers to a root absent from the supplied set.
    UnknownRoot,
    /// A stable `u32` storage boundary was exceeded.
    TooManyItems,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefinitionMismatch => formatter.write_str("root set and definition do not match"),
            Self::UnknownRoot => formatter.write_str("plan scenario contains an unknown root"),
            Self::TooManyItems => formatter.write_str("plan exceeds u32 item capacity"),
        }
    }
}

impl core::error::Error for PlanError {}

/// Public state of a quantity after evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QuantityState {
    /// No method or active root determines the quantity.
    Unknown,
    /// One operative claim, possibly with equivalent independent witnesses.
    Unique {
        /// Operative claim.
        operative: ClaimKey,
        /// Numerically identical independent claims.
        equivalent: Box<[ClaimKey]>,
    },
    /// Independent claims disagree; the provisional claim alone propagated.
    Contested {
        /// Deterministic provisional claim.
        provisional: ClaimKey,
        /// Disagreeing alternatives retained for inspection.
        alternatives: Box<[ClaimKey]>,
    },
    /// An explicit decision made one claim operative.
    Selected {
        /// Selected operative claim.
        operative: ClaimKey,
        /// Retained alternatives.
        alternatives: Box<[ClaimKey]>,
        /// Decision responsible for the selection.
        decision: DecisionKey,
    },
    /// A durable decision no longer matches its structural target.
    OrphanedSelection {
        /// Provisional claim that would otherwise be available.
        provisional: Option<ClaimKey>,
        /// Retained alternatives.
        alternatives: Box<[ClaimKey]>,
        /// Orphaned decision.
        decision: DecisionKey,
    },
    /// A discrete claim needs a stable candidate selection.
    Ambiguous {
        /// Producing claim.
        claim: ClaimKey,
        /// Number of candidates.
        candidates: u32,
    },
}

#[derive(Clone, Debug)]
struct QuantityResult {
    claims: Vec<ClaimId>,
    operative: Option<ClaimId>,
    selected: Option<DecisionKey>,
    orphaned: Option<DecisionKey>,
    ambiguous: Option<(ClaimId, u32)>,
}

impl QuantityResult {
    fn new() -> Self {
        Self {
            claims: Vec::new(),
            operative: None,
            selected: None,
            orphaned: None,
            ambiguous: None,
        }
    }
}

#[derive(Clone, Debug)]
struct ClaimRecord {
    id: ClaimId,
    key: ClaimKey,
    fingerprint: ClaimFingerprint,
    quantity: AnyQuantity,
    knowledge: ErasedKnowledge,
    origin: ClaimOrigin,
    support: SupportRef,
    exactness: ExactnessTrace,
}

#[derive(Clone, Debug)]
struct ClaimArena {
    records: Vec<Option<ClaimRecord>>,
    generations: Vec<u32>,
    by_key: BTreeMap<ClaimKey, ClaimId>,
}

impl ClaimArena {
    fn empty() -> Self {
        Self {
            records: Vec::new(),
            generations: Vec::new(),
            by_key: BTreeMap::new(),
        }
    }

    fn seeded(previous: &Self) -> Self {
        Self {
            records: vec![None; previous.records.len()],
            generations: previous.generations.clone(),
            by_key: BTreeMap::new(),
        }
    }

    fn insert(&mut self, mut record: ClaimRecord, previous: Option<&Self>) -> (ClaimId, bool) {
        if let Some(previous) = previous
            && let Some(previous_id) = previous.by_key.get(&record.key).copied()
        {
            let previous_record = previous.get(previous_id).expect("previous key resolves");
            let unchanged = previous_record.fingerprint == record.fingerprint;
            let generation = if unchanged {
                previous_id.generation()
            } else {
                previous_id.generation().wrapping_add(1)
            };
            let index = u32::try_from(previous_id.index()).expect("claim id index remains u32");
            let id = ClaimId::new(index, generation);
            record.id = id;
            self.generations[previous_id.index()] = generation;
            self.records[previous_id.index()] = Some(record);
            self.by_key.insert(previous_record.key, id);
            return (id, unchanged);
        }
        let index = self.records.len();
        let id = ClaimId::new(u32::try_from(index).expect("claim count is bounded"), 0);
        record.id = id;
        self.records.push(Some(record));
        self.generations.push(0);
        self.by_key
            .insert(self.records[index].as_ref().expect("inserted").key, id);
        (id, false)
    }

    fn get(&self, id: ClaimId) -> Option<&ClaimRecord> {
        (self.generations.get(id.index()).copied() == Some(id.generation()))
            .then(|| self.records.get(id.index()).and_then(Option::as_ref))
            .flatten()
    }

    fn by_key(&self, key: ClaimKey) -> Option<&ClaimRecord> {
        self.by_key.get(&key).and_then(|id| self.get(*id))
    }
}

#[derive(Clone, Debug)]
struct SupportArena {
    sets: Vec<Box<[SupportAtom]>>,
    by_atoms: BTreeMap<Box<[SupportAtom]>, SupportRef>,
}

impl SupportArena {
    fn new() -> Self {
        Self {
            sets: Vec::new(),
            by_atoms: BTreeMap::new(),
        }
    }

    fn intern(&mut self, atoms: impl IntoIterator<Item = SupportAtom>) -> SupportRef {
        let mut atoms: Vec<_> = atoms.into_iter().collect();
        atoms.sort();
        atoms.dedup();
        let atoms = atoms.into_boxed_slice();
        if let Some(reference) = self.by_atoms.get(&atoms) {
            return *reference;
        }
        let mut encoder = CanonicalEncoder::new("setout/support");
        encoder.u32(u32::try_from(atoms.len()).expect("support atoms are bounded"));
        for atom in &atoms {
            match atom {
                SupportAtom::Root(key) => {
                    encoder.u8(0);
                    encoder.str(key.as_str());
                }
                SupportAtom::Decision(key) => {
                    encoder.u8(1);
                    encoder.str(key.as_str());
                }
            }
        }
        let reference = SupportRef {
            key: SupportKey::from_fingerprint(encoder.finish()),
            slot: u32::try_from(self.sets.len()).expect("support set count is bounded"),
        };
        self.sets.push(atoms.clone());
        self.by_atoms.insert(atoms, reference);
        reference
    }

    fn atoms(&self, reference: SupportRef) -> &[SupportAtom] {
        &self.sets[reference.slot as usize]
    }
}

/// A typed, immutable result of evaluating one scenario.
#[derive(Clone, Debug)]
pub struct Evaluation {
    definition: Fingerprint,
    roots: Fingerprint,
    scenario: Fingerprint,
    plan: Fingerprint,
    quantity_by_key: BTreeMap<QuantityKey, QuantitySlot>,
    quantities: Vec<QuantityResult>,
    claims: ClaimArena,
    support: SupportArena,
    diagnostics: Box<[Diagnostic]>,
    fingerprint: Fingerprint,
    work: WorkReport,
}

impl Evaluation {
    /// Returns the fingerprint of the root set consumed by this evaluation.
    #[must_use]
    pub const fn roots_fingerprint(&self) -> Fingerprint {
        self.roots
    }

    /// Returns the fingerprint of the scenario consumed by this evaluation.
    #[must_use]
    pub const fn scenario_fingerprint(&self) -> Fingerprint {
        self.scenario
    }

    /// Strictly resolves one typed quantity.
    ///
    /// Contested, ambiguous, unknown, and orphaned quantities never silently
    /// yield their provisional value at a construction boundary.
    pub fn exact<T: Domain>(&self, quantity: &Quantity<T>) -> Result<T, AccessError> {
        let result = self
            .quantities
            .get(quantity.slot().index())
            .ok_or(AccessError::ForeignQuantity)?;
        match self.state(quantity.key()) {
            Some(QuantityState::Unique { .. } | QuantityState::Selected { .. }) => {
                let operative = result.operative.ok_or(AccessError::Unknown)?;
                let value = self
                    .claims
                    .get(operative)
                    .and_then(|claim| claim.knowledge.exact())
                    .and_then(Value::downcast::<T>)
                    .ok_or(AccessError::DomainMismatch)?;
                Ok(value.clone())
            }
            Some(QuantityState::Contested { .. }) => Err(AccessError::Contested),
            Some(QuantityState::Ambiguous { .. }) => Err(AccessError::Ambiguous),
            Some(QuantityState::OrphanedSelection { .. }) => Err(AccessError::OrphanedSelection),
            Some(QuantityState::Unknown) | None => Err(AccessError::Unknown),
        }
    }

    /// Returns the public state of a quantity by stable key.
    #[must_use]
    pub fn state(&self, quantity: &QuantityKey) -> Option<QuantityState> {
        self.quantity_by_key
            .get(quantity)
            .copied()
            .map(|slot| self.state_at(slot))
    }

    fn state_at(&self, slot: QuantitySlot) -> QuantityState {
        let result = &self.quantities[slot.index()];
        if let Some(decision) = &result.orphaned {
            return QuantityState::OrphanedSelection {
                provisional: result
                    .operative
                    .and_then(|id| self.claims.get(id))
                    .map(|claim| claim.key),
                alternatives: result
                    .claims
                    .iter()
                    .filter_map(|id| self.claims.get(*id).map(|claim| claim.key))
                    .collect(),
                decision: decision.clone(),
            };
        }
        if let Some((claim, candidates)) = result.ambiguous {
            return QuantityState::Ambiguous {
                claim: self.claims.get(claim).expect("ambiguous claim exists").key,
                candidates,
            };
        }
        let Some(operative_id) = result.operative else {
            return QuantityState::Unknown;
        };
        let operative = self
            .claims
            .get(operative_id)
            .expect("operative claim exists");
        let mut equivalent = Vec::new();
        let mut alternatives = Vec::new();
        for id in &result.claims {
            if *id == operative_id {
                continue;
            }
            let claim = self.claims.get(*id).expect("quantity claim exists");
            if claim.knowledge == operative.knowledge {
                equivalent.push(claim.key);
            } else {
                alternatives.push(claim.key);
            }
        }
        if let Some(decision) = &result.selected {
            return QuantityState::Selected {
                operative: operative.key,
                alternatives: alternatives.into_boxed_slice(),
                decision: decision.clone(),
            };
        }
        if alternatives.is_empty() {
            QuantityState::Unique {
                operative: operative.key,
                equivalent: equivalent.into_boxed_slice(),
            }
        } else {
            QuantityState::Contested {
                provisional: operative.key,
                alternatives: alternatives.into_boxed_slice(),
            }
        }
    }

    /// Returns all evaluation diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Returns the canonical evaluation fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Reports how much work was reused or evaluated.
    #[must_use]
    pub const fn work_report(&self) -> WorkReport {
        self.work
    }

    /// Opens an immutable structural provenance view.
    #[must_use]
    pub fn provenance(&self) -> ProvenanceView<'_> {
        ProvenanceView { evaluation: self }
    }

    /// Computes stable public changes from an earlier evaluation.
    #[must_use]
    pub fn delta_from(&self, previous: &Self) -> EvaluationDelta {
        let mut quantities_changed = Vec::new();
        let slots = self.quantities.len().max(previous.quantities.len());
        for index in 0..slots {
            // Structural ClaimKey deliberately survives a value-only edit, so
            // delta comparison must include ClaimFingerprint content as well as
            // the public state variant. Otherwise an exact root edit would look
            // clean even though every dependent value changed.
            let current = self.quantity_content_fingerprint(index);
            let old = previous.quantity_content_fingerprint(index);
            if current != old {
                let key =
                    self.quantity_by_key
                        .iter()
                        .find_map(|(key, slot)| (slot.index() == index).then(|| key.clone()))
                        .or_else(|| {
                            previous.quantity_by_key.iter().find_map(|(key, slot)| {
                                (slot.index() == index).then(|| key.clone())
                            })
                        });
                if let Some(key) = key {
                    quantities_changed.push(key);
                }
            }
        }
        let current_claims: BTreeMap<_, _> = self
            .claims
            .records
            .iter()
            .flatten()
            .map(|claim| (claim.key, claim.fingerprint))
            .collect();
        let previous_claims: BTreeMap<_, _> = previous
            .claims
            .records
            .iter()
            .flatten()
            .map(|claim| (claim.key, claim.fingerprint))
            .collect();
        let claims_changed = current_claims
            .keys()
            .chain(previous_claims.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|key| current_claims.get(key) != previous_claims.get(key))
            .collect();
        EvaluationDelta {
            quantities_changed: quantities_changed.into_boxed_slice(),
            claims_changed,
        }
    }

    fn quantity_content_fingerprint(&self, index: usize) -> Option<Fingerprint> {
        let result = self.quantities.get(index)?;
        let mut encoder = CanonicalEncoder::new("setout/quantity-result");
        match result.operative.and_then(|id| self.claims.get(id)) {
            Some(claim) => {
                encoder.u8(1);
                encoder.fingerprint(claim.fingerprint.fingerprint());
            }
            None => encoder.u8(0),
        }
        let mut claims: Vec<_> = result
            .claims
            .iter()
            .filter_map(|id| self.claims.get(*id).map(|claim| claim.fingerprint))
            .collect();
        claims.sort();
        for claim in claims {
            encoder.fingerprint(claim.fingerprint());
        }
        for decision in [&result.selected, &result.orphaned].into_iter().flatten() {
            encoder.str(decision.as_str());
        }
        if let Some((_, candidates)) = result.ambiguous {
            encoder.u32(candidates);
        }
        Some(encoder.finish())
    }
}

/// Counts fresh versus reused evaluation work.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkReport {
    /// Root claims whose content matched the predecessor.
    pub roots_reused: u32,
    /// Directed steps whose result was reused without solving again.
    pub steps_reused: u32,
    /// Directed steps executed in this evaluation.
    pub steps_evaluated: u32,
}

/// Stable public changes between evaluations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationDelta {
    /// Quantities whose public state or operative claim changed.
    pub quantities_changed: Box<[QuantityKey]>,
    /// Structural claims added, removed, or changed in content.
    pub claims_changed: Box<[ClaimKey]>,
}

/// A finding that does not prevent inspection of the rest of an evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Diagnostic {
    /// A relation could not produce exact knowledge.
    Arithmetic {
        /// Relation being evaluated.
        relation: RelationKey,
        /// Directed method being evaluated.
        method: MethodId,
        /// Checked-arithmetic failure.
        error: ArithmeticError,
    },
    /// A derived value violates the target quantity's policy.
    Inadmissible {
        /// Target quantity.
        quantity: QuantityKey,
        /// Producing relation.
        relation: RelationKey,
    },
    /// A durable selection did not reproduce its expected structural target.
    OrphanedDecision {
        /// Orphaned decision.
        decision: DecisionKey,
        /// Expected structural identity.
        expected: ClaimKey,
        /// Actual structural identity, if the producer still exists.
        actual: Option<ClaimKey>,
    },
    /// A stable candidate no longer exists in its discrete claim.
    MissingCandidate {
        /// Decision that named the candidate.
        decision: DecisionKey,
        /// Missing stable candidate identity.
        candidate: CandidateKey,
    },
}

/// Strict access failure at a consumer boundary.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AccessError {
    /// Quantity is unknown.
    Unknown,
    /// Independent claims disagree.
    Contested,
    /// A discrete claim lacks candidate selection.
    Ambiguous,
    /// A decision no longer matches its target.
    OrphanedSelection,
    /// Typed handle belongs to another definition.
    ForeignQuantity,
    /// Stored domain does not match the typed handle.
    DomainMismatch,
}

impl fmt::Display for AccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => formatter.write_str("quantity is unknown"),
            Self::Contested => formatter.write_str("quantity is contested"),
            Self::Ambiguous => formatter.write_str("quantity has unresolved candidates"),
            Self::OrphanedSelection => formatter.write_str("quantity has an orphaned selection"),
            Self::ForeignQuantity => formatter.write_str("quantity belongs to another definition"),
            Self::DomainMismatch => formatter.write_str("quantity domain mismatch"),
        }
    }
}

impl core::error::Error for AccessError {}

/// Evaluates a plan from scratch, serving as the reference oracle.
pub fn evaluate(
    definition: &NetworkDef,
    roots: &RootClaimSet,
    scenario: &EvaluationScenario,
    plan: &PropagationPlan,
) -> Result<Evaluation, EvaluationError> {
    evaluate_internal(definition, roots, scenario, plan, None)
}

/// Incremental evaluator that reuses structurally identical claims and steps.
#[derive(Copy, Clone, Debug, Default)]
pub struct IncrementalEvaluator;

impl IncrementalEvaluator {
    /// Creates an incremental evaluator.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Produces a successor while retaining safe claim handles and skipping
    /// directed methods whose exact inputs are unchanged.
    pub fn successor(
        &self,
        definition: &NetworkDef,
        roots: &RootClaimSet,
        scenario: &EvaluationScenario,
        plan: &PropagationPlan,
        previous: &Evaluation,
    ) -> Result<Evaluation, EvaluationError> {
        evaluate_internal(definition, roots, scenario, plan, Some(previous))
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the evaluation loop is intentionally linear and auditable as one state transition"
)]
fn evaluate_internal(
    definition: &NetworkDef,
    roots: &RootClaimSet,
    scenario: &EvaluationScenario,
    plan: &PropagationPlan,
    previous: Option<&Evaluation>,
) -> Result<Evaluation, EvaluationError> {
    if roots.definition != definition.fingerprint() || plan.definition != definition.fingerprint() {
        return Err(EvaluationError::DefinitionMismatch);
    }
    let expected_shape = scenario_shape_fingerprint(definition, roots, scenario);
    if plan.scenario_shape != expected_shape {
        return Err(EvaluationError::PlanShapeMismatch);
    }
    if let Some(previous) = previous
        && (previous.definition != definition.fingerprint() || previous.plan != plan.fingerprint())
    {
        return Err(EvaluationError::IncrementalLineageMismatch);
    }

    let previous_arena = previous.map(|value| &value.claims);
    let mut claims = previous_arena.map_or_else(ClaimArena::empty, ClaimArena::seeded);
    let mut support = SupportArena::new();
    let mut quantities: Vec<_> = (0..definition.quantity_count())
        .map(|_| QuantityResult::new())
        .collect();
    let mut diagnostics = Vec::new();
    let mut work = WorkReport::default();

    // Roots are inserted by stable key, never authoring order. This makes the
    // provisional choice in an over-determined quantity deterministic while
    // retaining every independent claim for conflict inspection.
    for root_key in scenario.active_external_roots() {
        let root = roots
            .claims
            .get(root_key)
            .ok_or(EvaluationError::UnknownRoot)?;
        let support_ref = support.intern([SupportAtom::Root(root.key.clone())]);
        let record = ClaimRecord {
            id: ClaimId::new(0, 0),
            key: root.claim_key,
            fingerprint: root.fingerprint,
            quantity: root.quantity.clone(),
            knowledge: root.knowledge.clone(),
            origin: ClaimOrigin::Root {
                key: root.key.clone(),
                source: root.source.clone(),
            },
            support: support_ref,
            exactness: root.exactness.clone(),
        };
        let (id, reused) = claims.insert(record, previous_arena);
        work.roots_reused += u32::from(reused);
        let result = &mut quantities[root.quantity.slot().index()];
        result.claims.push(id);
        match &root.knowledge {
            ErasedKnowledge::Exact(_) => {
                if result.operative.is_none() {
                    result.operative = Some(id);
                }
            }
            ErasedKnowledge::Discrete(candidates) => {
                result.ambiguous = Some((
                    id,
                    u32::try_from(candidates.len()).expect("candidate count is bounded"),
                ));
            }
        }
    }

    apply_root_decisions(
        definition,
        scenario,
        roots,
        &mut claims,
        previous_arena,
        &mut support,
        &mut quantities,
        &mut diagnostics,
    );

    for step in plan.steps() {
        let relation = &definition.relations[step.relation_index as usize];
        let method = &relation.methods[step.method_index as usize];
        let input_ids: Option<Vec<_>> = method
            .inputs
            .iter()
            .map(|slot| quantities[slot.index()].operative)
            .collect();
        let Some(input_ids) = input_ids else {
            // A producer decision can become orphaned after root-shape change.
            // The plan remains inspectable; no method runs on invented input.
            continue;
        };
        let input_records: Vec<_> = input_ids
            .iter()
            .map(|id| claims.get(*id).expect("operative input claim exists"))
            .collect();
        let input_values: Option<Vec<_>> = input_records
            .iter()
            .map(|claim| claim.knowledge.exact().cloned())
            .collect();
        let Some(input_values) = input_values else {
            continue;
        };
        let target = &definition.quantity_def(method.target).quantity;
        let claim_key = relation_claim_key(relation.key.as_str(), method, target, &input_records);

        // Reuse is decided from structural identity plus exact input content.
        // A root value edit preserves ClaimKey but changes ClaimFingerprint, so
        // descendants cannot be incorrectly retained merely because topology
        // stayed the same.
        let reusable = previous.and_then(|previous| {
            let old = previous.claims.by_key(claim_key)?;
            let old_inputs = match &old.origin {
                ClaimOrigin::Relation { inputs, .. } => inputs,
                _ => return None,
            };
            (old_inputs.len() == input_records.len()
                && old_inputs.iter().zip(&input_records).all(|(key, current)| {
                    previous
                        .claims
                        .by_key(*key)
                        .is_some_and(|prior| prior.fingerprint == current.fingerprint)
                }))
            .then_some(old)
        });

        let solved = if let Some(old) = reusable {
            work.steps_reused += 1;
            SolveResult {
                value: old
                    .knowledge
                    .exact()
                    .expect("derived relation claim is exact")
                    .clone(),
                exactness: old.exactness.clone(),
            }
        } else {
            work.steps_evaluated += 1;
            match method.operation.solve(&input_values) {
                Ok(solved) => solved,
                Err(error) => {
                    diagnostics.push(Diagnostic::Arithmetic {
                        relation: relation.key.clone(),
                        method: method.id.clone(),
                        error,
                    });
                    continue;
                }
            }
        };
        if !definition
            .quantity_def(method.target)
            .policy
            .accepts(&solved.value)
        {
            diagnostics.push(Diagnostic::Inadmissible {
                quantity: definition
                    .quantity_def(method.target)
                    .quantity
                    .key()
                    .clone(),
                relation: relation.key.clone(),
            });
            continue;
        }
        let decision = step.resolution.as_ref();
        let input_claim_keys: Box<[_]> = input_records.iter().map(|claim| claim.key).collect();
        let support_ref = {
            let mut atoms = Vec::new();
            for claim in &input_records {
                atoms.extend(support.atoms(claim.support).iter().cloned());
            }
            if let Some(decision) = decision {
                atoms.push(SupportAtom::Decision(decision.clone()));
            }
            support.intern(atoms)
        };
        let knowledge = ErasedKnowledge::Exact(solved.value);
        let fingerprint =
            relation_claim_fingerprint(claim_key, &knowledge, support_ref.key, &solved.exactness);
        let record = ClaimRecord {
            id: ClaimId::new(0, 0),
            key: claim_key,
            fingerprint,
            quantity: definition.quantity_def(method.target).quantity.clone(),
            knowledge,
            origin: ClaimOrigin::Relation {
                relation: relation.key.clone(),
                method: method.id.clone(),
                inputs: input_claim_keys,
            },
            support: support_ref,
            exactness: solved.exactness,
        };
        let (id, _) = claims.insert(record, previous_arena);
        let result = &mut quantities[method.target.index()];
        if !result.claims.contains(&id) {
            result.claims.push(id);
        }
        if let Some(decision_key) = decision {
            let decision = scenario
                .decisions()
                .iter()
                .find(|decision| &decision.key == decision_key)
                .expect("planned resolution names a scenario decision");
            let expected =
                decision_expected(decision).expect("relation selection has expected key");
            if expected == claim_key {
                result.operative = Some(id);
                result.selected = Some(decision_key.clone());
                result.orphaned = None;
            } else {
                result.orphaned = Some(decision_key.clone());
                diagnostics.push(Diagnostic::OrphanedDecision {
                    decision: decision_key.clone(),
                    expected,
                    actual: Some(claim_key),
                });
            }
        } else if result.operative.is_none() {
            result.operative = Some(id);
        }
    }

    // A relation selection whose producer never became runnable is retained as
    // an orphan instead of being dropped or rebound to an unrelated claim.
    for decision in scenario.decisions() {
        let relation_selection = match &decision.action {
            DecisionAction::SelectClaim {
                quantity,
                selection:
                    ClaimSelection {
                        producer: ClaimProducer::Relation { .. },
                        expected,
                    },
            }
            | DecisionAction::SelectCandidate {
                quantity,
                claim:
                    ClaimSelection {
                        producer: ClaimProducer::Relation { .. },
                        expected,
                    },
                ..
            } => Some((quantity, *expected)),
            _ => None,
        };
        if let Some((quantity, expected)) = relation_selection {
            let matched = diagnostics.iter().any(|diagnostic| {
                matches!(diagnostic, Diagnostic::OrphanedDecision { decision: key, .. } if key == &decision.key)
            }) || quantities.iter().any(|result| result.selected.as_ref() == Some(&decision.key));
            if !matched {
                mark_quantity_orphaned(definition, quantity, &decision.key, &mut quantities);
                diagnostics.push(Diagnostic::OrphanedDecision {
                    decision: decision.key.clone(),
                    expected,
                    actual: None,
                });
            }
        }
    }

    let fingerprint = evaluation_fingerprint(
        definition,
        roots,
        scenario,
        plan,
        &quantities,
        &claims,
        &diagnostics,
    );
    Ok(Evaluation {
        definition: definition.fingerprint(),
        roots: roots.fingerprint(),
        scenario: scenario.fingerprint(),
        plan: plan.fingerprint(),
        quantity_by_key: definition.quantity_by_key.clone(),
        quantities,
        claims,
        support,
        diagnostics: diagnostics.into_boxed_slice(),
        fingerprint,
        work,
    })
}

fn mark_quantity_orphaned(
    definition: &NetworkDef,
    quantity: &QuantityKey,
    decision: &DecisionKey,
    quantities: &mut [QuantityResult],
) {
    if let Some(slot) = definition.quantity_by_key.get(quantity).copied() {
        quantities[slot.index()].orphaned = Some(decision.clone());
    }
}

fn apply_root_decisions(
    definition: &NetworkDef,
    scenario: &EvaluationScenario,
    roots: &RootClaimSet,
    claims: &mut ClaimArena,
    previous_arena: Option<&ClaimArena>,
    support: &mut SupportArena,
    quantities: &mut [QuantityResult],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for decision in scenario.decisions() {
        let (quantity_key, selection, candidate) = match &decision.action {
            DecisionAction::SelectClaim {
                quantity,
                selection,
            } => (quantity, selection, None),
            DecisionAction::SelectCandidate {
                quantity,
                claim,
                candidate,
            } => (quantity, claim, Some(*candidate)),
        };
        let ClaimProducer::ExternalRoot(root_key) = &selection.producer else {
            continue;
        };
        let Some(root) = roots.claims.get(root_key) else {
            mark_quantity_orphaned(definition, quantity_key, &decision.key, quantities);
            diagnostics.push(Diagnostic::OrphanedDecision {
                decision: decision.key.clone(),
                expected: selection.expected,
                actual: None,
            });
            continue;
        };
        if root.quantity.key() != quantity_key || root.claim_key != selection.expected {
            // The decision belongs to its requested quantity, even when the
            // named producer now targets another one. Marking the producer's
            // slot would corrupt an unrelated public state.
            mark_quantity_orphaned(definition, quantity_key, &decision.key, quantities);
            diagnostics.push(Diagnostic::OrphanedDecision {
                decision: decision.key.clone(),
                expected: selection.expected,
                actual: Some(root.claim_key),
            });
            continue;
        }
        let Some(root_id) = claims.by_key.get(&root.claim_key).copied() else {
            // Scenarios may retain a durable selection while deactivating its
            // root. That is an inspectable orphan, not malformed input and
            // never grounds for an evaluation panic.
            mark_quantity_orphaned(definition, quantity_key, &decision.key, quantities);
            diagnostics.push(Diagnostic::OrphanedDecision {
                decision: decision.key.clone(),
                expected: selection.expected,
                actual: None,
            });
            continue;
        };
        let result = &mut quantities[root.quantity.slot().index()];
        if let Some(candidate_key) = candidate {
            let ErasedKnowledge::Discrete(candidates) = &root.knowledge else {
                result.orphaned = Some(decision.key.clone());
                diagnostics.push(Diagnostic::MissingCandidate {
                    decision: decision.key.clone(),
                    candidate: candidate_key,
                });
                continue;
            };
            let Some(candidate) = candidates
                .iter()
                .find(|candidate| candidate.key == candidate_key)
            else {
                result.orphaned = Some(decision.key.clone());
                diagnostics.push(Diagnostic::MissingCandidate {
                    decision: decision.key.clone(),
                    candidate: candidate_key,
                });
                continue;
            };
            let support_ref = support.intern([
                SupportAtom::Root(root.key.clone()),
                SupportAtom::Decision(decision.key.clone()),
            ]);
            let claim_key = resolution_claim_key(root.claim_key, &decision.key, candidate_key);
            let knowledge = ErasedKnowledge::Exact(candidate.value.clone());
            let fingerprint = relation_claim_fingerprint(
                claim_key,
                &knowledge,
                support_ref.key,
                &ExactnessTrace::Exact,
            );
            let record = ClaimRecord {
                id: ClaimId::new(0, 0),
                key: claim_key,
                fingerprint,
                quantity: root.quantity.clone(),
                knowledge,
                origin: ClaimOrigin::Resolution {
                    decision: decision.key.clone(),
                    selected: root.claim_key,
                },
                support: support_ref,
                exactness: ExactnessTrace::Exact,
            };
            let (id, _) = claims.insert(record, previous_arena);
            result.claims.push(id);
            result.operative = Some(id);
            result.selected = Some(decision.key.clone());
            result.ambiguous = None;
            result.orphaned = None;
        } else if root.knowledge.exact().is_some() {
            result.operative = Some(root_id);
            result.selected = Some(decision.key.clone());
            result.orphaned = None;
        }
    }
}

/// Failure that invalidates the evaluation request as a whole.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EvaluationError {
    /// Definition, roots, or plan do not belong together.
    DefinitionMismatch,
    /// Plan readiness shape does not match the scenario.
    PlanShapeMismatch,
    /// Incremental predecessor belongs to a different definition or plan.
    IncrementalLineageMismatch,
    /// Scenario activates a root absent from the supplied set.
    UnknownRoot,
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefinitionMismatch => {
                formatter.write_str("definition, roots, and plan do not match")
            }
            Self::PlanShapeMismatch => {
                formatter.write_str("plan does not match scenario readiness")
            }
            Self::IncrementalLineageMismatch => {
                formatter.write_str("incremental predecessor belongs to another lineage")
            }
            Self::UnknownRoot => {
                formatter.write_str("evaluation scenario contains an unknown root")
            }
        }
    }
}

impl core::error::Error for EvaluationError {}

/// Immutable erased view of one claim.
#[derive(Copy, Clone, Debug)]
pub struct ClaimView<'evaluation> {
    record: &'evaluation ClaimRecord,
}

impl<'evaluation> ClaimView<'evaluation> {
    /// Evaluation-local handle.
    #[must_use]
    pub const fn id(self) -> ClaimId {
        self.record.id
    }

    /// Stable structural identity.
    #[must_use]
    pub const fn key(self) -> ClaimKey {
        self.record.key
    }

    /// Content fingerprint, including value and exactness.
    #[must_use]
    pub const fn fingerprint(self) -> ClaimFingerprint {
        self.record.fingerprint
    }

    /// Quantity asserted by this claim.
    #[must_use]
    pub fn quantity(self) -> &'evaluation AnyQuantity {
        &self.record.quantity
    }

    /// Universal structural origin.
    #[must_use]
    pub fn origin(self) -> &'evaluation ClaimOrigin {
        &self.record.origin
    }

    /// Constant-size structural-support reference.
    #[must_use]
    pub const fn support(self) -> SupportRef {
        self.record.support
    }

    /// Exactness or explicit quantization certificate.
    #[must_use]
    pub fn exactness(self) -> &'evaluation ExactnessTrace {
        &self.record.exactness
    }
}

/// Structural provenance inspection detached from reconstruction policy.
#[derive(Copy, Clone, Debug)]
pub struct ProvenanceView<'evaluation> {
    evaluation: &'evaluation Evaluation,
}

impl<'evaluation> ProvenanceView<'evaluation> {
    /// Finds a claim by stable structural identity.
    #[must_use]
    pub fn claim(self, key: ClaimKey) -> Option<ClaimView<'evaluation>> {
        self.evaluation
            .claims
            .by_key(key)
            .map(|record| ClaimView { record })
    }

    /// Returns the operative claim for a quantity, including a provisional
    /// contested claim for exploratory inspection.
    #[must_use]
    pub fn operative(self, quantity: &QuantityKey) -> Option<ClaimView<'evaluation>> {
        let record = self
            .evaluation
            .claims
            .records
            .iter()
            .flatten()
            .find(|claim| claim.quantity.key() == quantity)?;
        let id = self.evaluation.quantities[record.quantity.slot().index()].operative?;
        self.evaluation
            .claims
            .get(id)
            .map(|record| ClaimView { record })
    }

    /// Iterates every live claim in arena order.
    pub fn claims(self) -> impl Iterator<Item = ClaimView<'evaluation>> {
        self.evaluation
            .claims
            .records
            .iter()
            .flatten()
            .map(|record| ClaimView { record })
    }

    /// Returns ordered immediate inputs of a relation claim.
    #[must_use]
    pub fn inputs(self, key: ClaimKey) -> Box<[ClaimKey]> {
        self.claim(key)
            .and_then(|claim| match claim.origin() {
                ClaimOrigin::Relation { inputs, .. } => Some(inputs.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Returns sorted, deduplicated structural support atoms.
    #[must_use]
    pub fn support(self, key: ClaimKey) -> Box<[SupportAtom]> {
        self.claim(key)
            .map(|claim| self.evaluation.support.atoms(claim.support()).into())
            .unwrap_or_default()
    }

    /// Renders a compact ancestry tree suitable for diagnostics and commit artifacts.
    #[must_use]
    pub fn explain(self, quantity: &QuantityKey) -> Option<String> {
        let claim = self.operative(quantity)?;
        let mut output = format!("{} = {}\n", quantity, claim.key());
        self.render_claim(claim.key(), 1, &mut output);
        Some(output)
    }

    fn render_claim(self, key: ClaimKey, depth: usize, output: &mut String) {
        let Some(claim) = self.claim(key) else {
            return;
        };
        for _ in 0..depth {
            output.push_str("  ");
        }
        match claim.origin() {
            ClaimOrigin::Root { key, .. } => {
                output.push_str("root ");
                output.push_str(key.as_str());
                output.push('\n');
            }
            ClaimOrigin::Relation {
                relation,
                method,
                inputs,
            } => {
                output.push_str("relation ");
                output.push_str(relation.as_str());
                output.push_str(" [");
                output.push_str(method.as_str());
                output.push_str("]\n");
                for input in inputs {
                    self.render_claim(*input, depth + 1, output);
                }
            }
            ClaimOrigin::Resolution { decision, selected } => {
                output.push_str("decision ");
                output.push_str(decision.as_str());
                output.push('\n');
                self.render_claim(*selected, depth + 1, output);
            }
        }
    }
}

fn root_claim_key(quantity: &QuantityKey, domain: DomainTag, root: &RootClaimKey) -> ClaimKey {
    let mut encoder = CanonicalEncoder::new("setout/claim-key/root");
    encoder.str(quantity.as_str());
    encoder.u8(domain.code());
    encoder.str(root.as_str());
    ClaimKey::from_fingerprint(encoder.finish())
}

fn root_claim_fingerprint(
    key: ClaimKey,
    knowledge: &ErasedKnowledge,
    source: &RootSource,
    exactness: &ExactnessTrace,
) -> ClaimFingerprint {
    let mut encoder = CanonicalEncoder::new("setout/claim-fingerprint/root");
    encoder.fingerprint(key.fingerprint());
    knowledge.encode(&mut encoder);
    encode_root_source(source, &mut encoder);
    encode_exactness(exactness, &mut encoder);
    ClaimFingerprint::from_fingerprint(encoder.finish())
}

fn relation_claim_key(
    relation: &str,
    method: &MethodDef,
    target: &AnyQuantity,
    inputs: &[&ClaimRecord],
) -> ClaimKey {
    let mut encoder = CanonicalEncoder::new("setout/claim-key/relation");
    encoder.str(relation);
    encoder.str(method.id.as_str());
    // Persisted selections must not silently rebind if a readable relation and
    // method key survive a schema edit that changes the produced quantity.
    encoder.str(target.key().as_str());
    encoder.u8(target.domain().code());
    // Operation parameters are structural. Changing a scale factor or rounding
    // policy must orphan a persisted selection even if the readable method key
    // was accidentally left unchanged.
    method.operation.encode(&mut encoder);
    encoder.u32(u32::try_from(inputs.len()).expect("method inputs are bounded"));
    for input in inputs {
        encoder.fingerprint(input.key.fingerprint());
    }
    ClaimKey::from_fingerprint(encoder.finish())
}

fn relation_claim_fingerprint(
    key: ClaimKey,
    knowledge: &ErasedKnowledge,
    support: SupportKey,
    exactness: &ExactnessTrace,
) -> ClaimFingerprint {
    let mut encoder = CanonicalEncoder::new("setout/claim-fingerprint/relation");
    encoder.fingerprint(key.fingerprint());
    knowledge.encode(&mut encoder);
    encoder.fingerprint(support.fingerprint());
    encode_exactness(exactness, &mut encoder);
    ClaimFingerprint::from_fingerprint(encoder.finish())
}

fn resolution_claim_key(
    root: ClaimKey,
    decision: &DecisionKey,
    candidate: CandidateKey,
) -> ClaimKey {
    let mut encoder = CanonicalEncoder::new("setout/claim-key/resolution");
    encoder.fingerprint(root.fingerprint());
    encoder.str(decision.as_str());
    encoder.fingerprint(candidate.fingerprint());
    ClaimKey::from_fingerprint(encoder.finish())
}

fn scenario_shape_fingerprint(
    definition: &NetworkDef,
    roots: &RootClaimSet,
    scenario: &EvaluationScenario,
) -> Fingerprint {
    let mut encoder = CanonicalEncoder::new("setout/scenario-shape");
    encoder.fingerprint(definition.fingerprint());
    for root_key in scenario.active_external_roots() {
        if let Some(root) = roots.claims.get(root_key) {
            encoder.str(root.key.as_str());
            encoder.str(root.quantity.key().as_str());
            encoder.u8(match root.knowledge {
                ErasedKnowledge::Exact(_) => 0,
                ErasedKnowledge::Discrete(_) => 1,
            });
        }
    }
    for decision in scenario.decisions() {
        encode_decision(decision, &mut encoder);
    }
    encoder.finish()
}

fn evaluation_fingerprint(
    definition: &NetworkDef,
    roots: &RootClaimSet,
    scenario: &EvaluationScenario,
    plan: &PropagationPlan,
    quantities: &[QuantityResult],
    claims: &ClaimArena,
    diagnostics: &[Diagnostic],
) -> Fingerprint {
    let mut encoder = CanonicalEncoder::new("setout/evaluation");
    encoder.fingerprint(definition.fingerprint());
    encoder.fingerprint(roots.fingerprint());
    encoder.fingerprint(scenario.fingerprint());
    encoder.fingerprint(plan.fingerprint());
    encoder.u32(u32::try_from(quantities.len()).expect("quantities are bounded"));
    for (index, quantity) in quantities.iter().enumerate() {
        encoder.str(definition.quantities[index].quantity.key().as_str());
        if let Some(operative) = quantity.operative.and_then(|id| claims.get(id)) {
            encoder.u8(1);
            encoder.fingerprint(operative.fingerprint.fingerprint());
        } else {
            encoder.u8(0);
        }
        let mut claim_fingerprints: Vec<_> = quantity
            .claims
            .iter()
            .filter_map(|id| claims.get(*id).map(|claim| claim.fingerprint))
            .collect();
        claim_fingerprints.sort();
        encoder.u32(u32::try_from(claim_fingerprints.len()).expect("claims are bounded"));
        for fingerprint in claim_fingerprints {
            encoder.fingerprint(fingerprint.fingerprint());
        }
    }
    encoder.u32(u32::try_from(diagnostics.len()).expect("diagnostics are bounded"));
    for diagnostic in diagnostics {
        encode_diagnostic(diagnostic, &mut encoder);
    }
    encoder.finish()
}

fn encode_root_source(source: &RootSource, encoder: &mut CanonicalEncoder) {
    match source {
        RootSource::Authored => encoder.u8(0),
        RootSource::Imported { importer, item } => {
            encoder.u8(1);
            encoder.str(importer);
            encoder.str(item);
        }
    }
}

fn encode_exactness(exactness: &ExactnessTrace, encoder: &mut CanonicalEncoder) {
    match exactness {
        ExactnessTrace::Exact => encoder.u8(0),
        ExactnessTrace::RationalQuantization {
            exact,
            selected,
            policy,
        } => {
            encoder.u8(1);
            exact.encode(encoder);
            encoder.i128(*selected);
            encoder.u8(policy.code());
        }
        ExactnessTrace::RootQuantization(rounding) => {
            encoder.u8(2);
            encoder.u128(rounding.radicand);
            encoder.u128(rounding.floor_root);
            encoder.u128(rounding.remainder);
            encoder.u128(rounding.selected_root);
            encoder.u8(rounding.policy.code());
        }
        ExactnessTrace::ImportedFloat(quantization) => {
            encoder.u8(3);
            encoder.u64(quantization.source_bits);
            encoder.i128(quantization.selected_iota);
            encoder.u64(quantization.error_iota_bits);
        }
    }
}

fn encode_decision(decision: &Decision, encoder: &mut CanonicalEncoder) {
    encoder.str(decision.key.as_str());
    match &decision.action {
        DecisionAction::SelectClaim {
            quantity,
            selection,
        } => {
            encoder.u8(0);
            encoder.str(quantity.as_str());
            encode_selection(selection, encoder);
        }
        DecisionAction::SelectCandidate {
            quantity,
            claim,
            candidate,
        } => {
            encoder.u8(1);
            encoder.str(quantity.as_str());
            encode_selection(claim, encoder);
            encoder.fingerprint(candidate.fingerprint());
        }
    }
}

fn encode_selection(selection: &ClaimSelection, encoder: &mut CanonicalEncoder) {
    match &selection.producer {
        ClaimProducer::ExternalRoot(root) => {
            encoder.u8(0);
            encoder.str(root.as_str());
        }
        ClaimProducer::Relation { relation, method } => {
            encoder.u8(1);
            encoder.str(relation.as_str());
            encoder.str(method.as_str());
        }
    }
    encoder.fingerprint(selection.expected.fingerprint());
}

fn encode_plan_step(step: &PlanStep, encoder: &mut CanonicalEncoder) {
    encoder.str(step.relation.as_str());
    encoder.str(step.method.as_str());
    encoder.str(step.target.as_str());
    encoder.u32(u32::try_from(step.inputs.len()).expect("plan inputs are bounded"));
    for input in &step.inputs {
        encoder.str(input.as_str());
    }
    encoder.bool(step.check);
    match &step.resolution {
        Some(decision) => {
            encoder.u8(1);
            encoder.str(decision.as_str());
        }
        None => encoder.u8(0),
    }
}

fn encode_diagnostic(diagnostic: &Diagnostic, encoder: &mut CanonicalEncoder) {
    match diagnostic {
        Diagnostic::Arithmetic {
            relation,
            method,
            error,
        } => {
            encoder.u8(0);
            encoder.str(relation.as_str());
            encoder.str(method.as_str());
            encoder.str(&error.to_string());
        }
        Diagnostic::Inadmissible { quantity, relation } => {
            encoder.u8(1);
            encoder.str(quantity.as_str());
            encoder.str(relation.as_str());
        }
        Diagnostic::OrphanedDecision {
            decision,
            expected,
            actual,
        } => {
            encoder.u8(2);
            encoder.str(decision.as_str());
            encoder.fingerprint(expected.fingerprint());
            match actual {
                Some(actual) => {
                    encoder.u8(1);
                    encoder.fingerprint(actual.fingerprint());
                }
                None => encoder.u8(0),
            }
        }
        Diagnostic::MissingCandidate {
            decision,
            candidate,
        } => {
            encoder.u8(3);
            encoder.str(decision.as_str());
            encoder.fingerprint(candidate.fingerprint());
        }
    }
}

fn candidate_decision_matches_root(decision: &Decision, root: &RootClaimDef) -> bool {
    matches!(
        &decision.action,
        DecisionAction::SelectCandidate {
            quantity,
            claim: ClaimSelection {
                producer: ClaimProducer::ExternalRoot(root_key),
                expected,
            },
            candidate,
        } if quantity == root.quantity.key()
            && root_key == &root.key
            && expected == &root.claim_key
            && matches!(&root.knowledge, ErasedKnowledge::Discrete(candidates) if candidates.iter().any(|item| item.key == *candidate))
    )
}

fn decision_expected(decision: &Decision) -> Option<ClaimKey> {
    match &decision.action {
        DecisionAction::SelectClaim { selection, .. } => Some(selection.expected),
        DecisionAction::SelectCandidate { claim, .. } => Some(claim.expected),
    }
}
