// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact, explainable setting-out propagation.
//!
//! **Setting-out** is the work of turning design intent and controlling
//! measurements into the datums, points, dimensions, and alignments from which
//! physical or virtual construction is located. A drawing may say that a roof
//! has a span and pitch; setting-out determines the wall seats, ridge, rafter
//! endpoints, and every dependent line without independently typing those
//! coordinates again.
//!
//! This crate owns that deterministic propagation and its structural provenance.
//! It explicitly does not own construction knowledge, historical-reconstruction
//! policy, mesh realization, or generative topology. `setout_joiner` translates
//! resolved quantities into construction extents; `joiner` validates how elements
//! fit; Exedra's constructive crates realize geometry.
//!
//! # Evaluation model
//!
//! ```text
//! immutable NetworkDef + RootClaimSet + EvaluationScenario
//!                         │
//!                         ▼
//!                compile_plan()
//!                         │
//!                         ▼
//!                    evaluate()
//!                         │
//!        exact values + conflicts + provenance + delta
//! ```
//!
//! Relations are multi-way. `a + b = total` can derive any one term from the
//! other two, and plan direction follows the roots available in a scenario.
//! Independent disagreement is retained as a contested quantity; challengers do
//! not silently become upstream inputs. Durable decisions name semantic producers
//! and expected structural claim keys, never container positions.
//!
//! [`Length`] is a strictly positive size and [`Offset`] is a signed
//! displacement; both use joto iotas (one ninth of a nanometer). [`Point3`]
//! stores offsets because coordinates may be zero or negative. Rational
//! arithmetic and integer square roots remain exact until an explicitly
//! recorded selection is required. Floating-point lowering happens once in an
//! adapter.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod evaluation;
mod fingerprint;
mod identity;
mod key;
mod network;
mod value;

pub use evaluation::{
    AccessError, ClaimView, Diagnostic, Evaluation, EvaluationDelta, EvaluationError,
    EvaluationScenario, EvaluationScenarioBuilder, IncrementalEvaluator, PlanError, PlanStep,
    PropagationPlan, ProvenanceView, QuantityState, RootBuildError, RootClaimSet,
    RootClaimSetBuilder, ScenarioBuildError, WorkReport, compile_plan, evaluate,
};
pub use fingerprint::{CanonicalEncoder, FINGERPRINT_SCHEMA_VERSION, Fingerprint};
pub use identity::{
    Candidate, CandidateKey, ClaimFingerprint, ClaimId, ClaimKey, ClaimOrigin, ClaimProducer,
    ClaimSelection, Decision, DecisionAction, Knowledge, RootSource, SupportAtom, SupportKey,
    SupportRef,
};
pub use key::{
    ChoiceDomainKey, ChoiceOptionKey, DecisionKey, KeyError, MethodId, QuantityKey, RelationKey,
    RootClaimKey, ScenarioKey,
};
pub use network::{
    AdjustLength, AnyQuantity, BuildError, ComposePoint, Equal, NetworkBuilder, NetworkDef,
    OffsetByLength, OffsetDirection, Pitch, Pythagorean, Quantity, QuantityPolicy, QuantitySlot,
    RelationSpec, ScaleLength, Sum, TranslateOffset,
};
#[doc(hidden)]
pub use network::{RelationDef, relation_private};
#[doc(hidden)]
pub use value::private;
pub use value::{
    ArithmeticError, ChoiceValue, Count, Domain, DomainTag, ExactnessTrace, Flag, Length, Offset,
    ParseMeasurementError, Point3, Rational, RootQuantization, RootRounding, Round, parse_length,
    parse_offset, quantize_length_meters, quantize_offset_meters,
};

#[cfg(test)]
mod tests;
