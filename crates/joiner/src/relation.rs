// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The three ways building elements are related.
//!
//! [`RelationKind::HostFill`], [`RelationKind::MemberMember`], and
//! [`RelationKind::ElementUnits`] are first-class siblings in one IR. None is
//! expressed in terms of another, and no rule sees a relation kind it did not
//! ask for. A window is not a degenerate joint; a bond is not a stack of
//! two-member fits.
//!
//! A [`Relation`] states the *intent*: these participants belong together,
//! on this evidence. It says nothing about geometry. Geometry arrives when a
//! rule is applied to the relation and returns a [`crate::RuleOutput`] — cuts
//! on the participants, parts to fill the gap, the faces that touch, and the
//! load those faces carry.
//!
//! A member/member relation is also the load-path witness for
//! [`crate::TransferKind::Joint`]: the relation *is* the joint. There is no
//! separate joint record to keep consistent with it.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::evidence::Evidence;

/// Which of the three relation kinds a [`Relation`] is, and who takes part.
///
/// Participants are named by key, never by handle, so a relation authored
/// before its rule runs still means the same thing afterwards.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RelationKind {
    /// A host element is voided and filled: a window in a wall, a niche, a
    /// doorway.
    ///
    /// The host exists up front. The fills usually do not — they are what
    /// the rule generates — so `fills` starts empty and
    /// [`crate::Construction::apply`] extends it with the keys of the parts
    /// the rule generated.
    HostFill {
        /// [`crate::Element::key`] of the element being voided.
        host: String,
        /// [`crate::Element::key`]s of the filling elements.
        fills: Vec<String>,
    },
    /// Two or more members are fitted to each other where their structural
    /// extents meet: a rafter heel on a tie beam, a king-post foot, a brace
    /// end.
    ///
    /// Every participant exists up front — a fit needs both sides — and the
    /// node is where they meet.
    MemberMember {
        /// [`crate::Node::key`] of the meeting point. It must lie inside the
        /// analytic extent of every named member's element; a surface joint
        /// need not lie on every member's centreline.
        node: String,
        /// [`crate::Member::key`]s taking part; at least two, all distinct.
        members: Vec<String>,
    },
    /// A whole element decomposes into units under a bond or coursing: a
    /// wall into courses and stones, an arch into voussoirs.
    ///
    /// Like host/fill, `units` starts empty and is extended by the rule
    /// application that generates them.
    ElementUnits {
        /// [`crate::Element::key`] of the element being decomposed.
        whole: String,
        /// [`crate::Element::key`]s of the units.
        units: Vec<String>,
    },
}

impl RelationKind {
    /// The stable label used in diagnostics.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::HostFill { .. } => "host/fill",
            Self::MemberMember { .. } => "member/member",
            Self::ElementUnits { .. } => "element/units",
        }
    }

    /// A host/fill relation with no fills yet.
    #[must_use]
    pub fn host_fill(host: &str) -> Self {
        Self::HostFill {
            host: host.to_string(),
            fills: Vec::new(),
        }
    }

    /// A member/member relation at `node` between `members`.
    #[must_use]
    pub fn member_member(node: &str, members: &[&str]) -> Self {
        Self::MemberMember {
            node: node.to_string(),
            members: members.iter().map(|key| (*key).to_string()).collect(),
        }
    }

    /// An element/units relation with no units yet.
    #[must_use]
    pub fn element_units(whole: &str) -> Self {
        Self::ElementUnits {
            whole: whole.to_string(),
            units: Vec::new(),
        }
    }
}

/// One declared relation between building elements.
#[derive(Clone, Debug)]
pub struct Relation {
    /// Stable frontend-supplied identity, unique among relations.
    pub key: String,
    /// The kind and its participants.
    pub kind: RelationKind,
    /// Opaque frontend label for the specific fit being claimed
    /// (`"housed-mortise-tenon"`, `"flemish-bond"`). `joiner` never
    /// interprets it; rule libraries and diagnostics do.
    pub detail: String,
    /// What this relation is based on.
    pub evidence: Evidence,
}

impl Relation {
    /// A relation of `kind`, labelled `detail`.
    #[must_use]
    pub fn new(key: &str, kind: RelationKind, detail: &str, evidence: Evidence) -> Self {
        Self {
            key: key.to_string(),
            kind,
            detail: detail.to_string(),
            evidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::EvidenceClass;

    #[test]
    fn the_three_kinds_are_distinguishable_and_labelled() {
        let evidence = Evidence::new("fixture", EvidenceClass::ModernEngineeringInference);
        let heel = Relation::new(
            "heel",
            RelationKind::member_member("node-heel", &["rafter", "tie"]),
            "birdsmouth",
            evidence.clone(),
        );
        let window = Relation::new(
            "window",
            RelationKind::host_fill("wall"),
            "clerestory",
            evidence.clone(),
        );
        let bond = Relation::new(
            "bond",
            RelationKind::element_units("wall"),
            "coursed-rubble",
            evidence,
        );
        assert_eq!(heel.kind.label(), "member/member");
        assert_eq!(window.kind.label(), "host/fill");
        assert_eq!(bond.kind.label(), "element/units");
        assert_ne!(window.kind, bond.kind, "host/fill is not element/units");
    }
}
