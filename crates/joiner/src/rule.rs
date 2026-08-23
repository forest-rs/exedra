// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The rule seam: how a declared relation becomes coordinated geometry.
//!
//! A rule is asked two questions, in this order:
//!
//! 1. [`Rule::assess`] — *can you fit this?* The answer is an
//!    [`Applicability`], never a panic and never a quietly degenerate cut. A
//!    rule that rejects itself says why, in typed [`Rejection`]s a caller can
//!    show, count, or match on.
//! 2. [`Rule::instantiate`] — *then fit it.* The answer is a [`RuleOutput`],
//!    which is the same four things for every rule of every relation kind:
//!    part edits, generated parts, contact patches, load-path edges. That
//!    uniformity is the point: validation and lowering consume rule output
//!    without knowing which rule produced it, or whether a rule produced it
//!    at all.
//!
//! Parameters are strongly typed per rule, through [`Rule::Params`]. There is
//! no erased, document-shaped, or agent-facing parameter boundary here and no
//! registry; if one is ever earned it is an additive adapter above this seam,
//! not a change to it.
//!
//! Everything in a [`RuleOutput`] names its participants by **key**. A rule
//! output is authored before the parts it generates exist, so handles would
//! be meaningless; keys are identity and stay meaningful.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use exedra_constructive::ir::{Placement3, Recipe, RecipeError};

use crate::construction::Construction;
use crate::element::{Element, Member, Node};
use crate::evidence::{Evidence, EvidenceClass};
use crate::geometry::Vec3;
use crate::relation::Relation;

/// What a rule is allowed to see when assessing or instantiating.
///
/// The context is read-only: a rule never mutates the construction. It
/// proposes, [`Construction::apply`] disposes.
#[derive(Debug)]
pub struct RuleContext<'a> {
    construction: &'a Construction,
    relation: &'a Relation,
}

impl<'a> RuleContext<'a> {
    /// Binds a context to one relation of `construction`.
    ///
    /// Returns `None` when no relation is registered under `relation`.
    #[must_use]
    pub fn new(construction: &'a Construction, relation: &str) -> Option<Self> {
        Some(Self {
            construction,
            relation: construction.relation(relation)?,
        })
    }

    /// The whole construction, for rules that need context beyond their own
    /// participants.
    #[must_use]
    pub fn construction(&self) -> &'a Construction {
        self.construction
    }

    /// The relation being fitted.
    #[must_use]
    pub fn relation(&self) -> &'a Relation {
        self.relation
    }

    /// Looks up a participating element by key.
    #[must_use]
    pub fn element(&self, key: &str) -> Option<&'a Element> {
        self.construction.element(key)
    }

    /// Looks up a participating member by key.
    #[must_use]
    pub fn member(&self, key: &str) -> Option<&'a Member> {
        self.construction.member(key)
    }

    /// Looks up a node by key.
    #[must_use]
    pub fn node(&self, key: &str) -> Option<&'a Node> {
        self.construction.node(key)
    }
}

/// Something a rule noticed while assessing a relation it *can* fit.
///
/// Observations are not warnings and not diagnostics: they are the rule's
/// reading of the situation, kept so a caller choosing between rules can see
/// what each one found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// The key this observation is about, or empty for the relation itself.
    pub subject: String,
    /// Human-readable detail.
    pub detail: String,
}

impl Observation {
    /// An observation about `subject`.
    #[must_use]
    pub fn new(code: &'static str, subject: &str, detail: &str) -> Self {
        Self {
            code,
            subject: subject.to_string(),
            detail: detail.to_string(),
        }
    }
}

/// Why a rule refuses a relation.
///
/// Every variant is a thing a rule library genuinely needs to say. The enum
/// is `#[non_exhaustive]` because the list will grow as rule libraries land;
/// callers must keep a fallback arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RejectionReason {
    /// The rule fits a different relation kind.
    WrongRelationKind {
        /// The kind this rule fits, as [`crate::RelationKind::label`].
        expected: &'static str,
        /// The kind it was offered.
        found: &'static str,
    },
    /// A participant the rule needs was not named.
    MissingParticipant {
        /// What was missing, as a stable diagnostic name.
        what: &'static str,
    },
    /// A named participant is not registered in the construction.
    UnknownParticipant {
        /// What the rule was looking for.
        what: &'static str,
    },
    /// A named participant is omitted from the current hypothesis.
    OmittedParticipant {
        /// What was omitted.
        what: &'static str,
    },
    /// The rule needs a different number of participants.
    ParticipantCount {
        /// What was counted, as a stable diagnostic name.
        what: &'static str,
        /// How many were named.
        found: usize,
        /// The smallest workable count.
        minimum: usize,
        /// The largest workable count, when bounded.
        maximum: Option<usize>,
    },
    /// A measurement is below what the rule can cut safely.
    TooSmall {
        /// What was measured, as a stable diagnostic name.
        what: &'static str,
        /// The measurement, in metres or radians.
        measured: f64,
        /// The smallest workable value.
        minimum: f64,
    },
    /// A measurement is above what the rule can cut safely.
    TooLarge {
        /// What was measured, as a stable diagnostic name.
        what: &'static str,
        /// The measurement, in metres or radians.
        measured: f64,
        /// The largest workable value.
        maximum: f64,
    },
    /// The participants' materials cannot be fitted this way.
    IncompatibleMaterial {
        /// The material key found.
        found: String,
        /// What the rule needs, as a stable diagnostic name.
        expected: &'static str,
    },
    /// The construction lacks something the rule requires (a centreline, a
    /// declared grain direction, a bond pattern).
    MissingCapability {
        /// What is required, as a stable diagnostic name.
        what: &'static str,
    },
    /// The relation's evidence is weaker than the rule is willing to assert.
    EvidenceTooWeak {
        /// The weakest class the rule accepts.
        required: EvidenceClass,
        /// The class found on the relation.
        found: EvidenceClass,
    },
    /// The geometry is a case this rule does not cover.
    Unsupported {
        /// What is unsupported, as a stable diagnostic name.
        what: &'static str,
    },
}

impl core::fmt::Display for RejectionReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongRelationKind { expected, found } => {
                write!(f, "rule fits {expected} relations, not {found}")
            }
            Self::MissingParticipant { what } => write!(f, "no {what} was named"),
            Self::UnknownParticipant { what } => write!(f, "{what} is not registered"),
            Self::OmittedParticipant { what } => write!(f, "{what} is omitted"),
            Self::ParticipantCount {
                what,
                found,
                minimum,
                maximum,
            } => match maximum {
                Some(maximum) => write!(f, "{found} {what} outside {minimum}..={maximum}"),
                None => write!(f, "{found} {what} is below the minimum of {minimum}"),
            },
            Self::TooSmall {
                what,
                measured,
                minimum,
            } => write!(f, "{what} {measured} is below {minimum}"),
            Self::TooLarge {
                what,
                measured,
                maximum,
            } => write!(f, "{what} {measured} is above {maximum}"),
            Self::IncompatibleMaterial { found, expected } => {
                write!(f, "material {found:?} is not {expected}")
            }
            Self::MissingCapability { what } => write!(f, "{what} is required but absent"),
            Self::EvidenceTooWeak { required, found } => {
                write!(f, "evidence {found} is weaker than {required}")
            }
            Self::Unsupported { what } => write!(f, "{what} is not covered by this rule"),
        }
    }
}

/// One typed refusal, attached to the key it is about.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejection {
    /// The key this refusal is about, or empty for the relation itself.
    pub subject: String,
    /// Why the rule refuses.
    pub reason: RejectionReason,
}

impl Rejection {
    /// A refusal about `subject`.
    #[must_use]
    pub fn new(subject: &str, reason: RejectionReason) -> Self {
        Self {
            subject: subject.to_string(),
            reason,
        }
    }
}

impl core::fmt::Display for Rejection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.subject.is_empty() {
            write!(f, "{}", self.reason)
        } else {
            write!(f, "{}: {}", self.subject, self.reason)
        }
    }
}

/// Whether a rule will fit a relation, and what it noticed either way.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Applicability {
    /// The rule will fit this relation.
    Suitable(Vec<Observation>),
    /// The rule refuses, for at least one typed reason.
    Unsuitable(Vec<Rejection>),
}

impl Applicability {
    /// Suitable, with nothing to remark on.
    #[must_use]
    pub fn suitable() -> Self {
        Self::Suitable(Vec::new())
    }

    /// Unsuitable, for one reason.
    #[must_use]
    pub fn unsuitable(subject: &str, reason: RejectionReason) -> Self {
        Self::Unsuitable(alloc::vec![Rejection::new(subject, reason)])
    }

    /// Whether the rule will fit the relation.
    #[must_use]
    pub fn is_suitable(&self) -> bool {
        matches!(self, Self::Suitable(_))
    }

    /// The observations, empty when unsuitable.
    #[must_use]
    pub fn observations(&self) -> &[Observation] {
        match self {
            Self::Suitable(observations) => observations,
            Self::Unsuitable(_) => &[],
        }
    }

    /// The refusals, empty when suitable.
    #[must_use]
    pub fn rejections(&self) -> &[Rejection] {
        match self {
            Self::Suitable(_) => &[],
            Self::Unsuitable(rejections) => rejections,
        }
    }
}

/// Why instantiation failed after a rule agreed to try.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RuleError {
    /// The rule was instantiated without being assessed, and refuses.
    NotApplicable(Vec<Rejection>),
    /// A typed parameter is out of range for this relation.
    InvalidParameter {
        /// Which parameter, as a stable diagnostic name.
        what: &'static str,
    },
    /// The fit would produce degenerate geometry, so the rule produced
    /// nothing instead.
    Degenerate {
        /// What would have degenerated, as a stable diagnostic name.
        what: &'static str,
    },
    /// A tool or generated part could not be built.
    Recipe(RecipeError),
}

impl core::fmt::Display for RuleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotApplicable(rejections) => {
                write!(f, "rule is not applicable ({} reason(s))", rejections.len())
            }
            Self::InvalidParameter { what } => write!(f, "invalid parameter: {what}"),
            Self::Degenerate { what } => write!(f, "fit would degenerate: {what}"),
            Self::Recipe(error) => write!(f, "tool geometry: {error}"),
        }
    }
}

impl core::error::Error for RuleError {}

impl From<RecipeError> for RuleError {
    fn from(error: RecipeError) -> Self {
        Self::Recipe(error)
    }
}

/// A way of fitting one kind of relation.
///
/// Implementations live in rule-library crates (`joiner_timber`,
/// `joiner_masonry`, …), never here: this crate owns the mechanism, not the
/// knowledge.
pub trait Rule {
    /// The rule's strongly typed parameters.
    type Params;

    /// Stable identity of the rule, recorded on every application.
    fn key(&self) -> &str;

    /// Whether this rule will fit `ctx`'s relation, and why not if it will
    /// not. Must not panic on any construction, however malformed.
    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability;

    /// Produces the coordinated geometry for `ctx`'s relation.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] when the parameters or the geometry do not
    /// permit the fit. Callers are expected to [`Rule::assess`] first;
    /// implementations must still refuse rather than emit a degenerate cut.
    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError>;
}

/// A solid used to edit another part, expressed in that part's local frame.
///
/// Tools are ordinary recipes. Expressing them in the *target's* frame is
/// what lets both sides of a fit derive from one nominal expression without
/// this crate ever inverting a matrix.
#[derive(Clone, Debug)]
pub struct ToolSolid {
    /// Stable frontend-supplied name, used in provenance and diagnostics.
    pub key: String,
    /// The tool's geometry.
    pub recipe: Recipe,
    /// Where the tool sits in the target part's local frame.
    pub placement: Placement3,
}

impl ToolSolid {
    /// A tool at `placement` in the target's local frame.
    #[must_use]
    pub fn new(key: &str, recipe: Recipe, placement: Placement3) -> Self {
        Self {
            key: key.to_string(),
            recipe,
            placement,
        }
    }
}

/// What a part edit does to its target.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PartEditOp {
    /// Subtract the tool: a seat, a housing, an opening void. Lowers to a
    /// constructive difference.
    RemoveSolid(ToolSolid),
    /// Keep only what the tool contains: a trim to a bounding form. Lowers
    /// to a constructive intersection.
    RetainSolid(ToolSolid),
}

impl PartEditOp {
    /// The tool this edit uses.
    #[must_use]
    pub fn tool(&self) -> &ToolSolid {
        match self {
            Self::RemoveSolid(tool) | Self::RetainSolid(tool) => tool,
        }
    }

    /// The stable label used in diagnostics.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::RemoveSolid(_) => "remove",
            Self::RetainSolid(_) => "retain",
        }
    }
}

/// A constructive edit appended to one participant's recipe.
///
/// Edits are applied by composing recipes *before* the part is registered
/// with `exedra_assembly`, never by a boolean between two placed instances —
/// see `exedra_assembly` ADR-0001.
#[derive(Clone, Debug)]
pub struct PartEdit {
    /// [`crate::Element::key`] whose recipe is edited. May name a part this
    /// same output generates.
    pub target: String,
    /// What the edit does.
    pub op: PartEditOp,
    /// What this cut is based on.
    pub evidence: Evidence,
}

impl PartEdit {
    /// Subtracts `tool` from `target`.
    #[must_use]
    pub fn remove(target: &str, tool: ToolSolid, evidence: Evidence) -> Self {
        Self {
            target: target.to_string(),
            op: PartEditOp::RemoveSolid(tool),
            evidence,
        }
    }

    /// Trims `target` to `tool`.
    #[must_use]
    pub fn retain(target: &str, tool: ToolSolid, evidence: Evidence) -> Self {
        Self {
            target: target.to_string(),
            op: PartEditOp::RetainSolid(tool),
            evidence,
        }
    }
}

/// A point on an element, in that element's local extent coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    /// [`crate::Element::key`] the point belongs to.
    pub element: String,
    /// Local coordinates within [`crate::Element::extent`].
    pub local: Vec3,
}

impl Anchor {
    /// A point at `local` on `element`.
    #[must_use]
    pub fn new(element: &str, local: Vec3) -> Self {
        Self {
            element: element.to_string(),
            local,
        }
    }
}

/// What a contact patch means structurally.
///
/// The meaning is not decoration: it decides whether the patch can witness a
/// load transfer. A clearance gap and a side fit hold a piece in place; they
/// do not carry it.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ContactMeaning {
    /// A face that carries load in compression across its normal.
    Bearing,
    /// Faces that locate a piece laterally without carrying it.
    SideFit,
    /// A shoulder that transfers thrust along a member.
    Shoulder,
    /// A mortar bed: bearing through a jointing material.
    MortarBed,
    /// A deliberate gap. Carries nothing; recorded so that "these do not
    /// touch" is an assertion rather than an omission.
    ClearanceOnly,
}

impl ContactMeaning {
    /// The stable label used in diagnostics.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Bearing => "bearing",
            Self::SideFit => "side-fit",
            Self::Shoulder => "shoulder",
            Self::MortarBed => "mortar-bed",
            Self::ClearanceOnly => "clearance-only",
        }
    }

    /// Whether a patch of this meaning can witness a
    /// [`TransferKind::Contact`] load transfer.
    #[must_use]
    pub fn carries_load(self) -> bool {
        match self {
            Self::Bearing | Self::Shoulder | Self::MortarBed => true,
            Self::SideFit | Self::ClearanceOnly => false,
        }
    }
}

/// Two faces declared to meet, and the frame they meet in.
///
/// The patch is an assertion with measurable content: the two anchors must
/// coincide in the complete frame within [`crate::CONTACT_TOLERANCE`], and
/// the elements must overlap across both tangents by at least
/// [`ContactPatch::minimum_overlap`]. A patch that does not measure up is
/// reported and stops witnessing transfers.
#[derive(Clone, Debug)]
pub struct ContactPatch {
    /// Stable frontend-supplied identity, unique among contacts.
    pub key: String,
    /// The anchor on the element being carried.
    pub carried: Anchor,
    /// The anchor on the element carrying it.
    pub carrier: Anchor,
    /// Unit contact normal, pointing from carrier into carried.
    pub normal: Vec3,
    /// Two unit tangents completing an orthonormal frame with `normal`.
    pub tangents: [Vec3; 2],
    /// The least acceptable overlap along each tangent, in metres.
    pub minimum_overlap: [f64; 2],
    /// What the contact means structurally.
    pub meaning: ContactMeaning,
    /// Opaque frontend label for the specific fit (`"crossed-seat"`,
    /// `"wall-head"`). `joiner` never interprets it.
    pub detail: String,
    /// What this contact is based on.
    pub evidence: Evidence,
}

impl ContactPatch {
    /// A patch between `carried` and `carrier` in the frame
    /// `normal`/`tangents`, with no overlap minimum yet.
    #[must_use]
    pub fn new(
        key: &str,
        carried: Anchor,
        carrier: Anchor,
        normal: Vec3,
        tangents: [Vec3; 2],
        meaning: ContactMeaning,
        evidence: Evidence,
    ) -> Self {
        Self {
            key: key.to_string(),
            carried,
            carrier,
            normal,
            tangents,
            minimum_overlap: [0.0, 0.0],
            meaning,
            detail: String::new(),
            evidence,
        }
    }

    /// Requires at least `minimum` metres of overlap along each tangent.
    #[must_use]
    pub fn with_minimum_overlap(mut self, minimum: [f64; 2]) -> Self {
        self.minimum_overlap = minimum;
        self
    }

    /// Attaches an opaque frontend label for the specific fit.
    #[must_use]
    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = detail.to_string();
        self
    }
}

/// Where a load transfer goes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum TransferTarget {
    /// Onto another element, by [`crate::Element::key`].
    Element(String),
    /// Out of the model, by [`crate::Support::key`].
    Support(String),
}

impl TransferTarget {
    /// Onto an element.
    #[must_use]
    pub fn element(key: &str) -> Self {
        Self::Element(key.to_string())
    }

    /// Out through a support.
    #[must_use]
    pub fn support(key: &str) -> Self {
        Self::Support(key.to_string())
    }
}

/// How a load transfer is carried, and therefore what must witness it.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum TransferKind {
    /// Through a load-carrying [`ContactPatch`] from source to target.
    Contact,
    /// Through a [`crate::RelationKind::MemberMember`] relation incident to
    /// members of both elements. The relation *is* the joint.
    Joint,
    /// Out of the model through the source element's own [`crate::Support`].
    Ground,
}

impl TransferKind {
    /// The stable label used in diagnostics.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Joint => "joint",
            Self::Ground => "ground",
        }
    }

    /// The diagnostic code reported when no witness matches.
    #[must_use]
    pub fn unwitnessed_code(self) -> &'static str {
        match self {
            Self::Contact => "unwitnessed-contact-transfer",
            Self::Joint => "unwitnessed-joint-transfer",
            Self::Ground => "unwitnessed-ground-transfer",
        }
    }
}

/// One directed edge of the load path.
///
/// An edge is a claim that load leaves `from` by `kind`. It counts only when
/// a witness of the matching kind is present and measures up; an unwitnessed
/// edge is reported and carries nothing, so a load path cannot be repaired
/// by asserting one.
#[derive(Clone, Debug)]
pub struct TransferEdge {
    /// Stable frontend-supplied identity, unique among transfers.
    pub key: String,
    /// The [`crate::Element::key`] shedding load.
    pub from: String,
    /// Where it goes.
    pub to: TransferTarget,
    /// How it is carried.
    pub kind: TransferKind,
}

impl TransferEdge {
    /// An edge from `from` to `to`, carried by `kind`.
    #[must_use]
    pub fn new(key: &str, from: &str, to: TransferTarget, kind: TransferKind) -> Self {
        Self {
            key: key.to_string(),
            from: from.to_string(),
            to,
            kind,
        }
    }
}

/// The uniform result of fitting any relation with any rule.
///
/// Four lists, always the same four, whatever the relation kind. A truss
/// heel fills the first, third, and fourth; a window opening fills all four;
/// a bond fills the second and third heavily. Nothing downstream branches on
/// which.
#[derive(Clone, Debug, Default)]
pub struct RuleOutput {
    /// Constructive edits to existing or generated parts.
    pub part_edits: Vec<PartEdit>,
    /// New elements this fit brings into being: sills, lintels, pegs,
    /// voussoirs. They enter the construction as ordinary elements.
    pub generated: Vec<Element>,
    /// The faces that meet, and what their meeting means.
    pub contacts: Vec<ContactPatch>,
    /// The load these faces and fits carry.
    pub transfers: Vec<TransferEdge>,
}

impl RuleOutput {
    /// An empty output.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a part edit.
    pub fn edit(&mut self, edit: PartEdit) -> &mut Self {
        self.part_edits.push(edit);
        self
    }

    /// Appends a generated part.
    pub fn generate(&mut self, element: Element) -> &mut Self {
        self.generated.push(element);
        self
    }

    /// Appends a contact patch.
    pub fn contact(&mut self, contact: ContactPatch) -> &mut Self {
        self.contacts.push(contact);
        self
    }

    /// Appends a load-path edge.
    pub fn transfer(&mut self, transfer: TransferEdge) -> &mut Self {
        self.transfers.push(transfer);
        self
    }

    /// Whether the fit produced nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.part_edits.is_empty()
            && self.generated.is_empty()
            && self.contacts.is_empty()
            && self.transfers.is_empty()
    }
}

/// One rule, applied to one relation, with its result.
///
/// The application is the provenance record: which rule fitted which
/// relation, on what evidence. [`Construction::apply`] keeps it after
/// merging the output, so a cut in the assembly can always be traced back.
#[derive(Clone, Debug)]
pub struct RuleApplication {
    /// Stable frontend-supplied identity, unique among applications. It also
    /// stamps [`crate::ElementOrigin::Generated`] on the parts this fit
    /// creates.
    pub key: String,
    /// [`Rule::key`] of the rule that produced the output.
    pub rule: String,
    /// [`Relation::key`] that was fitted.
    pub relation: String,
    /// What applying this rule here is based on.
    pub evidence: Evidence,
    /// The coordinated geometry.
    pub output: RuleOutput,
}

impl RuleApplication {
    /// Records `output` as `rule` applied to `relation`.
    #[must_use]
    pub fn new(
        key: &str,
        rule: &str,
        relation: &str,
        evidence: Evidence,
        output: RuleOutput,
    ) -> Self {
        Self {
            key: key.to_string(),
            rule: rule.to_string(),
            relation: relation.to_string(),
            evidence,
            output,
        }
    }
}

/// A rule application after its output has been merged, kept as provenance.
#[derive(Clone, Debug)]
pub struct AppliedRule {
    /// The application key.
    pub key: String,
    /// The rule that produced it.
    pub rule: String,
    /// The relation it fitted.
    pub relation: String,
    /// What it was based on.
    pub evidence: Evidence,
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::*;

    #[test]
    fn applicability_never_mixes_observations_with_refusals() {
        let suitable = Applicability::Suitable(alloc::vec![Observation::new(
            "shallow-seat",
            "rafter",
            "seat depth is at the lower bound",
        )]);
        assert!(suitable.is_suitable());
        assert_eq!(suitable.observations().len(), 1);
        assert!(suitable.rejections().is_empty());

        let unsuitable = Applicability::unsuitable(
            "rafter",
            RejectionReason::TooSmall {
                what: "member depth",
                measured: 0.04,
                minimum: 0.10,
            },
        );
        assert!(!unsuitable.is_suitable());
        assert!(unsuitable.observations().is_empty());
        assert_eq!(
            format!("{}", unsuitable.rejections()[0]),
            "rafter: member depth 0.04 is below 0.1"
        );
    }

    #[test]
    fn only_load_carrying_meanings_can_witness_a_transfer() {
        assert!(ContactMeaning::Bearing.carries_load());
        assert!(ContactMeaning::Shoulder.carries_load());
        assert!(ContactMeaning::MortarBed.carries_load());
        assert!(!ContactMeaning::SideFit.carries_load());
        assert!(!ContactMeaning::ClearanceOnly.carries_load());
        assert_eq!(
            TransferKind::Contact.unwitnessed_code(),
            "unwitnessed-contact-transfer"
        );
    }

    #[test]
    fn an_empty_output_is_the_default() {
        let mut output = RuleOutput::new();
        assert!(output.is_empty());
        output.transfer(TransferEdge::new(
            "load-a-to-b",
            "a",
            TransferTarget::element("b"),
            TransferKind::Contact,
        ));
        assert!(!output.is_empty());
    }
}
