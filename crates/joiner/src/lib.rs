// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction layer: building elements, how they are related, and the rules
//! that turn a relation into coordinated geometry.
//!
//! This crate owns *how building elements fit together*. Below it,
//! [`exedra_constructive`] compiles one part's recipe into meshes and
//! [`exedra_assembly`] arranges parts as placed instances. Neither knows what
//! a rafter or a window is. `joiner` does, and it knows nothing about meshes.
//!
//! ## The shape of the layer
//!
//! ```text
//! elements + relations           (the construction: the source of truth)
//!     -> rule.assess / rule.instantiate
//!     -> RuleOutput              (part edits, generated parts, contacts, transfers)
//!     -> Construction::apply     (merged into the same tables authored facts live in)
//!     -> validate                (schema/coherence, contact, load path)
//!     -> lower                   (one Assembly instance per geometry-bearing element)
//! ```
//!
//! Three relation kinds are first-class siblings in one IR —
//! [`RelationKind::HostFill`], [`RelationKind::MemberMember`],
//! [`RelationKind::ElementUnits`] — and none is expressed in terms of
//! another. Every rule, of every kind, returns the same four things, so
//! validation and lowering consume rule output without knowing which rule
//! produced it, or whether a rule produced it at all.
//!
//! ## Mechanism, not knowledge
//!
//! This crate contains no knowledge of any particular joint, bond, or
//! profile. It has the element graph, the relation kinds, the rule seam, the
//! uniform output, validation, and lowering. Construction *knowledge* lives in
//! separate rule-library crates (`joiner_timber`, `joiner_masonry`, …) so a
//! consumer that needs four timber joints does not inherit a dependency on
//! thirty, nor on stone.
//!
//! It owns none of: geometry math (that is [`exedra_constructive`] and
//! `exedra`); site, massing, and plan layout; statics, finite-element
//! analysis, capacity, or code compliance; rendering and export; or an
//! erased, document-shaped parameter boundary. See
//! `docs/adr-0001-construction-layer-scope.md`.
//!
//! ## Identity
//!
//! Element keys are the identity contract. They are frontend-supplied, stable
//! across re-evaluations, and the seed of the
//! [`exedra_assembly::InstancePath`] each element lowers to. Handles such as
//! [`ElementId`] are per-construction indices; they are never identity, and
//! rule output never uses them — a rule names the parts it is about to
//! generate, which have no handle yet.
//!
//! ## Evidence
//!
//! Every element, relation, contact, part edit, and rule application carries
//! an [`Evidence`] label: a source key plus an [`EvidenceClass`]. Validation
//! checks that the source exists and that the classes agree, so a modern
//! inference cannot quietly present itself as observed fabric.
//!
//! ## Invalidation
//!
//! The element is the dirty-tracking unit, through the `invalidation` crate,
//! on three channels: [`channel::GEOMETRY`], [`channel::CONTACT`],
//! [`channel::LOAD_PATH`]. Moving one window marks one wall and nothing else.
//!
//! ## Example
//!
//! A pier carrying a lintel, hand-authored as a rule output, validated, and
//! lowered:
//!
//! ```
//! use joiner::{
//!     Anchor, ContactMeaning, ContactPatch, Construction, Element, Evidence,
//!     EvidenceClass, EvidenceSource, OrientedBox, Relation, RelationKind,
//!     RuleApplication, RuleOutput, Support, TransferEdge, TransferKind,
//!     TransferTarget, lower, validate,
//! };
//!
//! let evidence = Evidence::new("worked-example", EvidenceClass::ModernEngineeringInference);
//! let mut construction = Construction::new();
//! construction.add_evidence_source(EvidenceSource::new(
//!     "worked-example",
//!     EvidenceClass::ModernEngineeringInference,
//!     "https://example.invalid/worked-example",
//!     "A two-element illustration, not a historical claim",
//! ))?;
//!
//! let pier = OrientedBox::axis_aligned([0.0, 0.0, 0.0], [0.6, 0.6, 3.0]);
//! let seat = pier.anchor([0.3, 0.3, 3.0]);
//! construction.add_element(
//!     Element::new("pier", "pier", "stone", pier, evidence.clone()).with_required_supports(1),
//! )?;
//! construction.add_support(Support::fixed("support-pier", "pier", "ground"))?;
//! construction.add_relation(Relation::new(
//!     "opening-north",
//!     RelationKind::host_fill("pier"),
//!     "trabeated-opening",
//!     evidence.clone(),
//! ))?;
//!
//! // The lintel's origin *is* the seat point, so the two anchors coincide
//! // exactly rather than nearly.
//! let lintel = OrientedBox {
//!     origin: [seat[0] - 0.3, seat[1] - 0.3, seat[2]],
//!     axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
//!     size: [2.4, 0.6, 0.4],
//! };
//! let mut output = RuleOutput::new();
//! output
//!     .generate(
//!         Element::new("lintel-north", "lintel", "stone", lintel, evidence.clone())
//!             .with_required_supports(1),
//!     )
//!     .contact(
//!         ContactPatch::new(
//!             "contact-lintel-on-pier",
//!             Anchor::new("lintel-north", [0.3, 0.3, 0.0]),
//!             Anchor::new("pier", [0.3, 0.3, 3.0]),
//!             [0.0, 0.0, 1.0],
//!             [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
//!             ContactMeaning::Bearing,
//!             evidence.clone(),
//!         )
//!         .with_minimum_overlap([0.5, 0.5]),
//!     )
//!     .transfer(TransferEdge::new(
//!         "load-lintel-to-pier",
//!         "lintel-north",
//!         TransferTarget::element("pier"),
//!         TransferKind::Contact,
//!     ))
//!     .transfer(TransferEdge::new(
//!         "load-pier-to-ground",
//!         "pier",
//!         TransferTarget::support("support-pier"),
//!         TransferKind::Ground,
//!     ));
//! construction.apply(RuleApplication::new(
//!     "fit-opening-north",
//!     "example:trabeated-opening",
//!     "opening-north",
//!     evidence,
//!     output,
//! ))?;
//!
//! let report = validate(&construction);
//! assert!(report.is_clean(), "{report}");
//!
//! let assembly = lower(&construction)?;
//! let paths: Vec<String> = assembly
//!     .instances()
//!     .iter()
//!     .map(|instance| instance.key().to_string())
//!     .collect();
//! assert_eq!(paths, ["pier", "lintel-north"]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod construction;
pub mod element;
pub mod evidence;
pub mod geometry;
pub mod lower;
pub mod relation;
pub mod rule;
pub mod validate;

#[cfg(test)]
mod seeds;

pub use construction::{Construction, ConstructionError, ElementId, channel};
pub use element::{DEFAULT_SLOT, Element, ElementOrigin, Member, Node, Part, Restraints, Support};
pub use evidence::{Evidence, EvidenceClass, EvidenceSource};
pub use geometry::{OrientedBox, Vec3};
pub use lower::{LowerError, compose, instance_path, lower, lower_selected, part_key};
pub use relation::{Relation, RelationKind};
pub use rule::{
    Anchor, Applicability, AppliedRule, ContactMeaning, ContactPatch, Observation, PartEdit,
    PartEditOp, Rejection, RejectionReason, Rule, RuleApplication, RuleContext, RuleError,
    RuleOutput, ToolSolid, TransferEdge, TransferKind, TransferTarget,
};
pub use validate::{
    CONTACT_TOLERANCE, ContactMeasurement, Diagnostic, ValidationReport, is_witnessed,
    measure_contact, trace_to_ground, validate,
};
