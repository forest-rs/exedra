// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The construction: the element graph and everything rules have added to it.
//!
//! A [`Construction`] is the source of truth. An `exedra_assembly::Assembly`
//! is a compiled artifact of it, exactly as a mesh is a compiled artifact of a
//! recipe — see [`crate::lower()`].
//!
//! Structure is append-mostly and validated at every mutation, in the same
//! spirit as `exedra_assembly::Assembly`: keys are non-empty, free of `/`, and
//! unique within their category, and every reference resolves at the moment
//! it is made. Dangling references and duplicate keys are therefore not
//! representable, and [`crate::validate()`] is left to the questions that
//! genuinely need the whole graph — geometry coherence, contact measurement,
//! transfer witnesses, and load paths.
//!
//! Rule output and authored fact live in the same tables. A contact patch a
//! rule produced and one a frontend declared are indistinguishable downstream,
//! which is what lets validation consume rule output without knowing any rule
//! exists. Provenance is not lost: [`Construction::applications`] records
//! which rule fitted which relation, and generated parts carry
//! [`crate::ElementOrigin::Generated`].
//!
//! ## Invalidation
//!
//! The **element** is the dirty-tracking unit, through the `invalidation`
//! crate. Three channels say what went stale, so a consumer can re-do only
//! that work:
//!
//! - [`channel::GEOMETRY`] — the element's recipe or extent changed, so its
//!   part must be recompiled.
//! - [`channel::CONTACT`] — a contact patch touching the element changed, so
//!   its contact measurements must be re-taken.
//! - [`channel::LOAD_PATH`] — a transfer touching the element changed, so
//!   routes through it must be re-traced.
//!
//! Moving one window marks one wall on all three and nothing else.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use hashbrown::{HashMap, HashSet};
use invalidation::{Channel, InvalidationSet};

use crate::element::{Element, ElementOrigin, Member, Node, Support};
use crate::evidence::EvidenceSource;
use crate::geometry::OrientedBox;
use crate::relation::{Relation, RelationKind};
use crate::rule::{
    AppliedRule, ContactPatch, PartEdit, RuleApplication, TransferEdge, TransferTarget,
};

/// The invalidation channels a construction marks on.
pub mod channel {
    use invalidation::Channel;

    /// The element's own geometry changed: recompile its part.
    pub const GEOMETRY: Channel = Channel::new(0);
    /// A contact touching the element changed: re-measure its contacts.
    pub const CONTACT: Channel = Channel::new(1);
    /// A transfer touching the element changed: re-trace its load path.
    pub const LOAD_PATH: Channel = Channel::new(2);

    /// All three, in channel order.
    pub const ALL: [Channel; 3] = [GEOMETRY, CONTACT, LOAD_PATH];
}

/// Index of an element within a [`Construction`].
///
/// A handle, never an identity: it is meaningful only for the construction
/// that produced it and only until that construction changes. Element *keys*
/// are identity. The handle exists because dirty tracking needs a dense,
/// `Copy` key.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ElementId(pub u32);

/// Typed construction mutation failure.
///
/// Every variant means the mutation did not happen: a construction is never
/// left half-edited, and [`Construction::apply`] in particular is all or
/// nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConstructionError {
    /// A key is empty or contains the instance-path separator `/`.
    InvalidKey {
        /// Which namespace, as a stable diagnostic name.
        category: &'static str,
        /// The offending key.
        key: String,
    },
    /// A key is already registered in its namespace.
    DuplicateKey {
        /// Which namespace, as a stable diagnostic name.
        category: &'static str,
        /// The offending key.
        key: String,
    },
    /// A reference names something that is not registered.
    UnknownReference {
        /// Which namespace was consulted, as a stable diagnostic name.
        category: &'static str,
        /// The unresolved key.
        key: String,
    },
    /// A relation named too few participants to be that kind of relation.
    TooFewParticipants {
        /// The relation key.
        key: String,
        /// What was counted, as a stable diagnostic name.
        what: &'static str,
        /// How many were named.
        found: usize,
        /// The smallest meaningful count.
        minimum: usize,
    },
    /// A relation named the same participant twice.
    DuplicateParticipant {
        /// The relation key.
        key: String,
        /// The repeated participant key.
        participant: String,
    },
}

impl core::fmt::Display for ConstructionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidKey { category, key } => {
                write!(f, "{category} key {key:?} is empty or contains {:?}", '/')
            }
            Self::DuplicateKey { category, key } => {
                write!(f, "{category} key {key:?} is already registered")
            }
            Self::UnknownReference { category, key } => {
                write!(f, "no {category} is registered under {key:?}")
            }
            Self::TooFewParticipants {
                key,
                what,
                found,
                minimum,
            } => write!(
                f,
                "relation {key:?} names {found} {what} but needs at least {minimum}"
            ),
            Self::DuplicateParticipant { key, participant } => {
                write!(f, "relation {key:?} names {participant:?} twice")
            }
        }
    }
}

impl core::error::Error for ConstructionError {}

/// The element graph plus every fact a rule has added to it.
#[derive(Debug, Default)]
pub struct Construction {
    evidence_sources: Vec<EvidenceSource>,
    nodes: Vec<Node>,
    elements: Vec<Element>,
    members: Vec<Member>,
    relations: Vec<Relation>,
    supports: Vec<Support>,
    contacts: Vec<ContactPatch>,
    transfers: Vec<TransferEdge>,
    part_edits: Vec<PartEdit>,
    applications: Vec<AppliedRule>,
    evidence_keys: HashSet<String>,
    node_keys: HashMap<String, u32>,
    element_keys: HashMap<String, ElementId>,
    member_keys: HashMap<String, u32>,
    relation_keys: HashMap<String, u32>,
    support_keys: HashSet<String>,
    contact_keys: HashSet<String>,
    transfer_keys: HashSet<String>,
    application_keys: HashSet<String>,
    dirty: InvalidationSet<ElementId>,
}

impl Clone for Construction {
    /// Copies the graph and marks every element dirty on every channel.
    ///
    /// A clone has never been lowered, so nothing about it is known to be
    /// current; carrying the original's invalidation state across would claim
    /// otherwise.
    fn clone(&self) -> Self {
        let mut clone = Self {
            evidence_sources: self.evidence_sources.clone(),
            nodes: self.nodes.clone(),
            elements: self.elements.clone(),
            members: self.members.clone(),
            relations: self.relations.clone(),
            supports: self.supports.clone(),
            contacts: self.contacts.clone(),
            transfers: self.transfers.clone(),
            part_edits: self.part_edits.clone(),
            applications: self.applications.clone(),
            evidence_keys: self.evidence_keys.clone(),
            node_keys: self.node_keys.clone(),
            element_keys: self.element_keys.clone(),
            member_keys: self.member_keys.clone(),
            relation_keys: self.relation_keys.clone(),
            support_keys: self.support_keys.clone(),
            contact_keys: self.contact_keys.clone(),
            transfer_keys: self.transfer_keys.clone(),
            application_keys: self.application_keys.clone(),
            dirty: InvalidationSet::new(),
        };
        for index in 0..u32::try_from(clone.elements.len()).unwrap_or(u32::MAX) {
            clone.mark_all(ElementId(index));
        }
        clone
    }
}

impl Construction {
    /// An empty construction.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an evidence source.
    ///
    /// # Errors
    ///
    /// Fails on an invalid or duplicate key.
    pub fn add_evidence_source(&mut self, source: EvidenceSource) -> Result<(), ConstructionError> {
        check_key("evidence", &source.key)?;
        if !self.evidence_keys.insert(source.key.clone()) {
            return Err(ConstructionError::DuplicateKey {
                category: "evidence",
                key: source.key,
            });
        }
        self.evidence_sources.push(source);
        Ok(())
    }

    /// Registers a centreline node.
    ///
    /// # Errors
    ///
    /// Fails on an invalid or duplicate key.
    pub fn add_node(&mut self, node: Node) -> Result<(), ConstructionError> {
        check_key("node", &node.key)?;
        if self.node_keys.contains_key(&node.key) {
            return Err(ConstructionError::DuplicateKey {
                category: "node",
                key: node.key,
            });
        }
        let index = len_u32(self.nodes.len());
        self.node_keys.insert(node.key.clone(), index);
        self.nodes.push(node);
        Ok(())
    }

    /// Registers an element and returns its handle.
    ///
    /// The new element is marked dirty on every channel.
    ///
    /// # Errors
    ///
    /// Fails on an invalid or duplicate key.
    pub fn add_element(&mut self, element: Element) -> Result<ElementId, ConstructionError> {
        check_key("element", &element.key)?;
        if self.element_keys.contains_key(&element.key) {
            return Err(ConstructionError::DuplicateKey {
                category: "element",
                key: element.key,
            });
        }
        let id = ElementId(len_u32(self.elements.len()));
        self.element_keys.insert(element.key.clone(), id);
        self.elements.push(element);
        self.mark_all(id);
        Ok(id)
    }

    /// Registers a centreline member.
    ///
    /// # Errors
    ///
    /// Fails on an invalid or duplicate key, or when the element or either
    /// node is not registered.
    pub fn add_member(&mut self, member: Member) -> Result<(), ConstructionError> {
        check_key("member", &member.key)?;
        self.require_element(&member.element)?;
        self.require_node(&member.from)?;
        self.require_node(&member.to)?;
        if self.member_keys.contains_key(&member.key) {
            return Err(ConstructionError::DuplicateKey {
                category: "member",
                key: member.key,
            });
        }
        let index = len_u32(self.members.len());
        self.member_keys.insert(member.key.clone(), index);
        self.members.push(member);
        Ok(())
    }

    /// Registers a relation between elements or members.
    ///
    /// # Errors
    ///
    /// Fails on an invalid or duplicate key, an unregistered participant, a
    /// member/member relation with fewer than two members, or a repeated
    /// participant.
    pub fn add_relation(&mut self, relation: Relation) -> Result<(), ConstructionError> {
        check_key("relation", &relation.key)?;
        self.check_relation_participants(&relation)?;
        if self.relation_keys.contains_key(&relation.key) {
            return Err(ConstructionError::DuplicateKey {
                category: "relation",
                key: relation.key,
            });
        }
        let index = len_u32(self.relations.len());
        self.relation_keys.insert(relation.key.clone(), index);
        self.relations.push(relation);
        Ok(())
    }

    /// Registers a ground support.
    ///
    /// # Errors
    ///
    /// Fails on an invalid or duplicate key, or an unregistered element.
    pub fn add_support(&mut self, support: Support) -> Result<(), ConstructionError> {
        check_key("support", &support.key)?;
        self.require_element(&support.element)?;
        if !self.support_keys.insert(support.key.clone()) {
            return Err(ConstructionError::DuplicateKey {
                category: "support",
                key: support.key,
            });
        }
        let id = self.element_keys[&support.element];
        self.supports.push(support);
        self.dirty.mark(id, channel::LOAD_PATH);
        Ok(())
    }

    /// Registers a contact patch, marking both elements' contact channel.
    ///
    /// # Errors
    ///
    /// Fails on an invalid or duplicate key, or an unregistered anchor
    /// element.
    pub fn add_contact(&mut self, contact: ContactPatch) -> Result<(), ConstructionError> {
        check_key("contact", &contact.key)?;
        self.require_element(&contact.carried.element)?;
        self.require_element(&contact.carrier.element)?;
        if !self.contact_keys.insert(contact.key.clone()) {
            return Err(ConstructionError::DuplicateKey {
                category: "contact",
                key: contact.key,
            });
        }
        for id in [
            self.element_keys[&contact.carried.element],
            self.element_keys[&contact.carrier.element],
        ] {
            self.dirty.mark(id, channel::CONTACT);
            self.dirty.mark(id, channel::LOAD_PATH);
        }
        self.contacts.push(contact);
        Ok(())
    }

    /// Registers a load-path edge, marking both ends' load-path channel.
    ///
    /// # Errors
    ///
    /// Fails on an invalid or duplicate key, or an unregistered source
    /// element or target.
    pub fn add_transfer(&mut self, transfer: TransferEdge) -> Result<(), ConstructionError> {
        check_key("transfer", &transfer.key)?;
        self.require_element(&transfer.from)?;
        let target = match &transfer.to {
            TransferTarget::Element(key) => Some(self.require_element(key)?),
            TransferTarget::Support(key) => {
                if !self.support_keys.contains(key) {
                    return Err(ConstructionError::UnknownReference {
                        category: "support",
                        key: key.clone(),
                    });
                }
                None
            }
        };
        if self.transfer_keys.contains(&transfer.key) {
            return Err(ConstructionError::DuplicateKey {
                category: "transfer",
                key: transfer.key,
            });
        }
        self.transfer_keys.insert(transfer.key.clone());
        if let Some(target) = target {
            self.dirty.mark(target, channel::LOAD_PATH);
        }
        let from = self.element_keys[&transfer.from];
        self.dirty.mark(from, channel::LOAD_PATH);
        self.transfers.push(transfer);
        Ok(())
    }

    /// Merges a rule application into the construction.
    ///
    /// Generated parts enter as elements stamped with
    /// [`ElementOrigin::Generated`], part edits are queued for lowering,
    /// contacts and transfers join the same tables authored facts live in,
    /// and host/fill and element/units relations gain the generated keys as
    /// participants. Every touched element is marked dirty on the channels
    /// its change affects.
    ///
    /// # Errors
    ///
    /// Fails on an unregistered relation, a duplicate application, element,
    /// contact or transfer key, or a part edit or anchor naming something
    /// neither registered nor generated by this same output. Nothing is
    /// merged when it fails.
    pub fn apply(&mut self, application: RuleApplication) -> Result<(), ConstructionError> {
        check_key("application", &application.key)?;
        if self.application_keys.contains(&application.key) {
            return Err(ConstructionError::DuplicateKey {
                category: "application",
                key: application.key,
            });
        }
        let Some(&relation_index) = self.relation_keys.get(&application.relation) else {
            return Err(ConstructionError::UnknownReference {
                category: "relation",
                key: application.relation,
            });
        };
        self.preflight(&application)?;

        let RuleApplication {
            key,
            rule,
            relation,
            evidence,
            output,
        } = application;
        let mut generated_keys = Vec::with_capacity(output.generated.len());
        for mut element in output.generated {
            element.origin = ElementOrigin::Generated(key.clone());
            generated_keys.push(element.key.clone());
            self.add_element(element)?;
        }
        for edit in output.part_edits {
            let id = self.element_keys[&edit.target];
            self.dirty.mark(id, channel::GEOMETRY);
            self.part_edits.push(edit);
        }
        for contact in output.contacts {
            self.add_contact(contact)?;
        }
        for transfer in output.transfers {
            self.add_transfer(transfer)?;
        }
        match &mut self.relations[relation_index as usize].kind {
            RelationKind::HostFill { fills, .. } => fills.extend(generated_keys),
            RelationKind::ElementUnits { units, .. } => units.extend(generated_keys),
            RelationKind::MemberMember { .. } => {}
        }
        self.application_keys.insert(key.clone());
        self.applications.push(AppliedRule {
            key,
            rule,
            relation,
            evidence,
        });
        Ok(())
    }

    /// Includes or omits an element from the current hypothesis.
    ///
    /// Omitting keeps the key, the relations, and the transfers while
    /// removing the element from geometry, contact witnesses, and load paths.
    ///
    /// # Errors
    ///
    /// Fails when the element is not registered.
    pub fn set_element_present(
        &mut self,
        key: &str,
        present: bool,
    ) -> Result<(), ConstructionError> {
        let id = self.require_element(key)?;
        self.elements[id.0 as usize].present = present;
        self.mark_all(id);
        Ok(())
    }

    /// Replaces an element's declared extent.
    ///
    /// The extent is the frame its part is placed by and the space its
    /// contacts are measured in, so this marks all three channels.
    ///
    /// # Errors
    ///
    /// Fails when the element is not registered.
    pub fn set_element_extent(
        &mut self,
        key: &str,
        extent: OrientedBox,
    ) -> Result<(), ConstructionError> {
        let id = self.require_element(key)?;
        self.elements[id.0 as usize].extent = extent;
        self.mark_all(id);
        Ok(())
    }

    /// All registered evidence sources, in registration order.
    #[must_use]
    pub fn evidence_sources(&self) -> &[EvidenceSource] {
        &self.evidence_sources
    }

    /// Looks up an evidence source by key.
    #[must_use]
    pub fn evidence_source(&self, key: &str) -> Option<&EvidenceSource> {
        self.evidence_sources.iter().find(|s| s.key == key)
    }

    /// All nodes, in registration order.
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Looks up a node by key.
    #[must_use]
    pub fn node(&self, key: &str) -> Option<&Node> {
        self.node_keys
            .get(key)
            .and_then(|index| self.nodes.get(*index as usize))
    }

    /// All elements, in registration order.
    #[must_use]
    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    /// Looks up an element by key.
    #[must_use]
    pub fn element(&self, key: &str) -> Option<&Element> {
        self.element_id(key).and_then(|id| self.element_by_id(id))
    }

    /// Resolves an element key to its handle.
    #[must_use]
    pub fn element_id(&self, key: &str) -> Option<ElementId> {
        self.element_keys.get(key).copied()
    }

    /// Looks up an element by handle.
    #[must_use]
    pub fn element_by_id(&self, id: ElementId) -> Option<&Element> {
        self.elements.get(id.0 as usize)
    }

    /// All members, in registration order.
    #[must_use]
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Looks up a member by key.
    #[must_use]
    pub fn member(&self, key: &str) -> Option<&Member> {
        self.member_keys
            .get(key)
            .and_then(|index| self.members.get(*index as usize))
    }

    /// The members of one element, in registration order.
    pub fn members_of<'a>(&'a self, element: &'a str) -> impl Iterator<Item = &'a Member> + 'a {
        self.members.iter().filter(move |m| m.element == element)
    }

    /// All relations, in registration order.
    #[must_use]
    pub fn relations(&self) -> &[Relation] {
        &self.relations
    }

    /// Looks up a relation by key.
    #[must_use]
    pub fn relation(&self, key: &str) -> Option<&Relation> {
        self.relation_keys
            .get(key)
            .and_then(|index| self.relations.get(*index as usize))
    }

    /// All supports, in registration order.
    #[must_use]
    pub fn supports(&self) -> &[Support] {
        &self.supports
    }

    /// Looks up a support by key.
    #[must_use]
    pub fn support(&self, key: &str) -> Option<&Support> {
        self.supports.iter().find(|s| s.key == key)
    }

    /// All contact patches, authored and rule-produced alike, in
    /// registration order.
    #[must_use]
    pub fn contacts(&self) -> &[ContactPatch] {
        &self.contacts
    }

    /// Looks up a contact patch by key.
    #[must_use]
    pub fn contact(&self, key: &str) -> Option<&ContactPatch> {
        self.contacts.iter().find(|c| c.key == key)
    }

    /// All load-path edges, in registration order.
    #[must_use]
    pub fn transfers(&self) -> &[TransferEdge] {
        &self.transfers
    }

    /// Looks up a load-path edge by key.
    #[must_use]
    pub fn transfer(&self, key: &str) -> Option<&TransferEdge> {
        self.transfers.iter().find(|t| t.key == key)
    }

    /// All queued part edits, in application order.
    #[must_use]
    pub fn part_edits(&self) -> &[PartEdit] {
        &self.part_edits
    }

    /// The part edits targeting one element, in application order. Lowering
    /// composes them onto the element's recipe in exactly this order.
    pub fn part_edits_for<'a>(
        &'a self,
        element: &'a str,
    ) -> impl Iterator<Item = &'a PartEdit> + 'a {
        self.part_edits.iter().filter(move |e| e.target == element)
    }

    /// Provenance for every rule application merged so far, in order.
    #[must_use]
    pub fn applications(&self) -> &[AppliedRule] {
        &self.applications
    }

    /// Whether an element is marked dirty on `channel`.
    #[must_use]
    pub fn is_dirty(&self, key: &str, channel: Channel) -> bool {
        self.element_id(key)
            .is_some_and(|id| self.dirty.is_invalidated(id, channel))
    }

    /// How many elements are marked dirty on `channel`.
    #[must_use]
    pub fn dirty_count(&self, channel: Channel) -> usize {
        self.dirty.len(channel)
    }

    /// Drains `channel`, returning the dirty elements' keys in registration
    /// order.
    ///
    /// Deterministic by construction: the handles are sorted before they are
    /// turned back into keys, so two runs of the same edits drain the same
    /// list.
    pub fn take_dirty(&mut self, channel: Channel) -> Vec<String> {
        let mut ids: Vec<ElementId> = self.dirty.drain(channel).collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| self.elements.get(id.0 as usize))
            .map(|element| element.key.clone())
            .collect()
    }

    /// Marks every element clean on every channel, declaring the whole
    /// construction current.
    pub fn clear_dirty(&mut self) {
        self.dirty.clear_all();
    }

    fn mark_all(&mut self, id: ElementId) {
        for channel in channel::ALL {
            self.dirty.mark(id, channel);
        }
    }

    fn require_element(&self, key: &str) -> Result<ElementId, ConstructionError> {
        self.element_keys
            .get(key)
            .copied()
            .ok_or_else(|| ConstructionError::UnknownReference {
                category: "element",
                key: key.to_string(),
            })
    }

    fn require_node(&self, key: &str) -> Result<(), ConstructionError> {
        if self.node_keys.contains_key(key) {
            return Ok(());
        }
        Err(ConstructionError::UnknownReference {
            category: "node",
            key: key.to_string(),
        })
    }

    fn require_member(&self, key: &str) -> Result<(), ConstructionError> {
        if self.member_keys.contains_key(key) {
            return Ok(());
        }
        Err(ConstructionError::UnknownReference {
            category: "member",
            key: key.to_string(),
        })
    }

    fn check_relation_participants(&self, relation: &Relation) -> Result<(), ConstructionError> {
        let mut seen: HashSet<&str> = HashSet::new();
        match &relation.kind {
            RelationKind::HostFill { host, fills } => {
                for element in core::iter::once(host).chain(fills) {
                    self.require_element(element)?;
                    check_unique(&mut seen, &relation.key, element)?;
                }
            }
            RelationKind::MemberMember { node, members } => {
                self.require_node(node)?;
                if members.len() < 2 {
                    return Err(ConstructionError::TooFewParticipants {
                        key: relation.key.clone(),
                        what: "members",
                        found: members.len(),
                        minimum: 2,
                    });
                }
                for member in members {
                    self.require_member(member)?;
                    check_unique(&mut seen, &relation.key, member)?;
                }
            }
            RelationKind::ElementUnits { whole, units } => {
                for element in core::iter::once(whole).chain(units) {
                    self.require_element(element)?;
                    check_unique(&mut seen, &relation.key, element)?;
                }
            }
        }
        Ok(())
    }

    /// Checks every key and reference in an application before any of it is
    /// merged, so a rejected application leaves nothing behind.
    fn preflight(&self, application: &RuleApplication) -> Result<(), ConstructionError> {
        let output = &application.output;
        let mut generated: HashSet<&str> = HashSet::new();
        for element in &output.generated {
            check_key("element", &element.key)?;
            if self.element_keys.contains_key(element.key.as_str())
                || !generated.insert(element.key.as_str())
            {
                return Err(ConstructionError::DuplicateKey {
                    category: "element",
                    key: element.key.clone(),
                });
            }
        }
        let known_element =
            |key: &str| self.element_keys.contains_key(key) || generated.contains(key);
        let require = |category: &'static str, key: &str, ok: bool| {
            if ok {
                Ok(())
            } else {
                Err(ConstructionError::UnknownReference {
                    category,
                    key: key.to_string(),
                })
            }
        };
        for edit in &output.part_edits {
            require("element", &edit.target, known_element(&edit.target))?;
        }
        let mut contacts: HashSet<&str> = HashSet::new();
        for contact in &output.contacts {
            check_key("contact", &contact.key)?;
            if self.contact_keys.contains(contact.key.as_str())
                || !contacts.insert(contact.key.as_str())
            {
                return Err(ConstructionError::DuplicateKey {
                    category: "contact",
                    key: contact.key.clone(),
                });
            }
            for anchor in [&contact.carried, &contact.carrier] {
                require("element", &anchor.element, known_element(&anchor.element))?;
            }
        }
        let mut transfers: HashSet<&str> = HashSet::new();
        for transfer in &output.transfers {
            check_key("transfer", &transfer.key)?;
            if self.transfer_keys.contains(transfer.key.as_str())
                || !transfers.insert(transfer.key.as_str())
            {
                return Err(ConstructionError::DuplicateKey {
                    category: "transfer",
                    key: transfer.key.clone(),
                });
            }
            require("element", &transfer.from, known_element(&transfer.from))?;
            match &transfer.to {
                TransferTarget::Element(key) => require("element", key, known_element(key))?,
                TransferTarget::Support(key) => {
                    require("support", key, self.support_keys.contains(key.as_str()))?;
                }
            }
        }
        Ok(())
    }
}

fn check_unique<'a>(
    seen: &mut HashSet<&'a str>,
    relation: &str,
    participant: &'a str,
) -> Result<(), ConstructionError> {
    if seen.insert(participant) {
        return Ok(());
    }
    Err(ConstructionError::DuplicateParticipant {
        key: relation.to_string(),
        participant: participant.to_string(),
    })
}

fn check_key(category: &'static str, key: &str) -> Result<(), ConstructionError> {
    if key.is_empty() || key.contains('/') {
        return Err(ConstructionError::InvalidKey {
            category,
            key: key.to_string(),
        });
    }
    Ok(())
}

fn len_u32(n: usize) -> u32 {
    debug_assert!(u32::try_from(n).is_ok(), "count budget validated upstream");
    #[expect(
        clippy::cast_possible_truncation,
        reason = "construction sizes are validated against the u32 budget at insertion"
    )]
    {
        n as u32
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::element::Restraints;
    use crate::evidence::{Evidence, EvidenceClass};
    use crate::rule::{
        Anchor, ContactMeaning, RuleApplication, RuleOutput, TransferEdge, TransferKind,
    };

    fn evidence() -> Evidence {
        Evidence::new("fixture", EvidenceClass::ModernEngineeringInference)
    }

    fn box_element(key: &str) -> Element {
        Element::new(
            key,
            "test",
            "test-material",
            OrientedBox::axis_aligned([0.0; 3], [1.0, 1.0, 1.0]),
            evidence(),
        )
    }

    fn seeded() -> Construction {
        let mut construction = Construction::new();
        construction
            .add_evidence_source(EvidenceSource::new(
                "fixture",
                EvidenceClass::ModernEngineeringInference,
                "https://example.invalid",
                "test fixture",
            ))
            .expect("fresh source");
        construction.add_element(box_element("wall")).expect("wall");
        construction
            .add_element(box_element("lintel"))
            .expect("lintel");
        construction
            .add_relation(Relation::new(
                "window",
                RelationKind::host_fill("wall"),
                "clerestory",
                evidence(),
            ))
            .expect("relation");
        construction
    }

    #[test]
    fn keys_are_identity_and_handles_are_not() {
        let construction = seeded();
        let wall = construction.element_id("wall").expect("wall exists");
        assert_eq!(
            construction.element_by_id(wall).map(|e| e.key.as_str()),
            Some("wall")
        );
        assert!(construction.element("no-such-wall").is_none());
    }

    #[test]
    fn invalid_and_duplicate_keys_are_typed_errors() {
        let mut construction = seeded();
        assert_eq!(
            construction.add_element(box_element("wall")),
            Err(ConstructionError::DuplicateKey {
                category: "element",
                key: "wall".into()
            })
        );
        assert!(
            construction.element("wall").is_some(),
            "rejecting a duplicate must preserve the original lookup"
        );
        assert_eq!(
            construction.add_element(box_element("a/b")),
            Err(ConstructionError::InvalidKey {
                category: "element",
                key: "a/b".into()
            })
        );
    }

    #[test]
    fn duplicate_rejection_is_atomic_in_every_map_backed_namespace() {
        let mut construction = seeded();
        construction
            .add_node(Node::new("n0", [0.0, 0.0, 0.0]))
            .expect("node");
        construction
            .add_node(Node::new("n1", [1.0, 0.0, 0.0]))
            .expect("node");
        construction
            .add_member(Member::new("member", "wall", "n0", "n1", evidence()))
            .expect("member");

        assert!(construction.add_node(Node::new("n0", [0.5; 3])).is_err());
        assert_eq!(
            construction.node("n0").map(|node| node.point),
            Some([0.0; 3])
        );

        assert!(
            construction
                .add_member(Member::new("member", "lintel", "n0", "n1", evidence()))
                .is_err()
        );
        assert_eq!(
            construction
                .member("member")
                .map(|member| member.element.as_str()),
            Some("wall")
        );

        assert!(
            construction
                .add_relation(Relation::new(
                    "window",
                    RelationKind::host_fill("lintel"),
                    "duplicate",
                    evidence(),
                ))
                .is_err()
        );
        assert_eq!(
            construction
                .relation("window")
                .map(|relation| &relation.kind),
            Some(&RelationKind::host_fill("wall"))
        );
    }

    #[test]
    fn rejected_transfer_does_not_dirty_its_target() {
        let mut construction = seeded();
        construction
            .add_transfer(TransferEdge::new(
                "load-wall-to-lintel",
                "wall",
                TransferTarget::element("lintel"),
                TransferKind::Contact,
            ))
            .expect("transfer");
        construction.clear_dirty();

        assert!(
            construction
                .add_transfer(TransferEdge::new(
                    "load-wall-to-lintel",
                    "wall",
                    TransferTarget::element("lintel"),
                    TransferKind::Contact,
                ))
                .is_err()
        );
        assert_eq!(construction.dirty_count(channel::LOAD_PATH), 0);
    }

    #[test]
    fn references_must_resolve_at_the_moment_they_are_made() {
        let mut construction = seeded();
        assert_eq!(
            construction.add_support(Support::fixed("s", "missing", "ground")),
            Err(ConstructionError::UnknownReference {
                category: "element",
                key: "missing".into()
            })
        );
        assert_eq!(
            construction.add_member(Member::new("m", "wall", "n0", "n1", evidence())),
            Err(ConstructionError::UnknownReference {
                category: "node",
                key: "n0".into()
            })
        );
        assert_eq!(
            construction.add_relation(Relation::new(
                "fit",
                RelationKind::member_member("n0", &["m0", "m1"]),
                "",
                evidence()
            )),
            Err(ConstructionError::UnknownReference {
                category: "node",
                key: "n0".into()
            })
        );
    }

    #[test]
    fn a_member_member_relation_needs_two_distinct_members() {
        let mut construction = seeded();
        construction
            .add_node(Node::new("n0", [0.5, 0.5, 0.5]))
            .expect("node");
        construction
            .add_node(Node::new("n1", [0.5, 0.5, 0.9]))
            .expect("node");
        construction
            .add_member(Member::new("m0", "wall", "n0", "n1", evidence()))
            .expect("member");
        assert_eq!(
            construction.add_relation(Relation::new(
                "fit",
                RelationKind::member_member("n0", &["m0"]),
                "",
                evidence()
            )),
            Err(ConstructionError::TooFewParticipants {
                key: "fit".into(),
                what: "members",
                found: 1,
                minimum: 2
            })
        );
        assert_eq!(
            construction.add_relation(Relation::new(
                "fit",
                RelationKind::member_member("n0", &["m0", "m0"]),
                "",
                evidence()
            )),
            Err(ConstructionError::DuplicateParticipant {
                key: "fit".into(),
                participant: "m0".into()
            })
        );
    }

    #[test]
    fn a_rejected_application_merges_nothing() {
        let mut construction = seeded();
        let elements = construction.elements().len();
        let mut output = RuleOutput::new();
        output
            .generate(box_element("sill"))
            .transfer(TransferEdge::new(
                "load-sill-to-nowhere",
                "sill",
                TransferTarget::element("missing"),
                TransferKind::Contact,
            ));
        let error = construction
            .apply(RuleApplication::new(
                "apply-window",
                "test:window",
                "window",
                evidence(),
                output,
            ))
            .expect_err("target does not exist");
        assert_eq!(
            error,
            ConstructionError::UnknownReference {
                category: "element",
                key: "missing".into()
            }
        );
        assert_eq!(construction.elements().len(), elements, "no element leaked");
        assert!(construction.applications().is_empty());
    }

    #[test]
    fn applying_a_fit_generates_parts_and_extends_the_relation() {
        let mut construction = seeded();
        construction
            .add_support(Support::fixed("support-wall", "wall", "ground"))
            .expect("support");
        let mut output = RuleOutput::new();
        output
            .generate(box_element("sill"))
            .contact(
                ContactPatch::new(
                    "contact-sill-on-wall",
                    Anchor::new("sill", [0.5, 0.5, 0.0]),
                    Anchor::new("wall", [0.5, 0.5, 1.0]),
                    [0.0, 0.0, 1.0],
                    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    ContactMeaning::Bearing,
                    evidence(),
                )
                .with_detail("wall-head"),
            )
            .transfer(TransferEdge::new(
                "load-sill-to-wall",
                "sill",
                TransferTarget::element("wall"),
                TransferKind::Contact,
            ));
        construction
            .apply(RuleApplication::new(
                "apply-window",
                "test:window",
                "window",
                evidence(),
                output,
            ))
            .expect("application merges");

        let sill = construction.element("sill").expect("generated element");
        assert_eq!(
            sill.origin,
            ElementOrigin::Generated("apply-window".to_string())
        );
        assert_eq!(
            construction.relation("window").map(|r| r.kind.clone()),
            Some(RelationKind::HostFill {
                host: "wall".into(),
                fills: vec!["sill".into()],
            })
        );
        assert_eq!(construction.applications().len(), 1);
        assert_eq!(construction.contacts().len(), 1);
        assert!(construction.is_dirty("wall", channel::CONTACT));
    }

    #[test]
    fn dirty_drains_deterministically_by_registration_order() {
        let mut construction = seeded();
        assert_eq!(
            construction.take_dirty(channel::GEOMETRY),
            ["wall", "lintel"]
        );
        assert_eq!(construction.dirty_count(channel::GEOMETRY), 0);
        construction
            .set_element_extent("lintel", OrientedBox::axis_aligned([0.0; 3], [2.0; 3]))
            .expect("known element");
        assert_eq!(construction.take_dirty(channel::GEOMETRY), ["lintel"]);
        construction.clear_dirty();
        assert_eq!(construction.dirty_count(channel::CONTACT), 0);
    }

    #[test]
    fn a_clone_is_entirely_dirty() {
        let mut construction = seeded();
        construction.clear_dirty();
        let clone = construction.clone();
        assert_eq!(clone.dirty_count(channel::GEOMETRY), clone.elements().len());
        assert_eq!(construction.dirty_count(channel::GEOMETRY), 0);
    }

    #[test]
    fn restraints_default_to_fully_fixed() {
        assert_eq!(Restraints::FIXED.translation, [true, true, true]);
    }
}
