// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Layered validation over a whole construction.
//!
//! Three layers, deliberately separate, and one line about what none of them
//! are:
//!
//! 1. **Schema and coherence.** Finite geometry, positive extents,
//!    orthonormal frames, evidence links that exist and agree, centrelines
//!    that stay inside their own solids, relations incident to everything
//!    they name.
//! 2. **Contact.** Anchors that lie inside the extents they claim, coincide
//!    in the complete contact frame, have an orthonormal frame, a signed gap
//!    within [`CONTACT_TOLERANCE`], and finite, sufficient overlap along both
//!    tangents. A [`crate::ContactMeaning::ClearanceOnly`] patch is the
//!    inverse assertion — it must *not* interpenetrate — and is measured that
//!    way.
//! 3. **Load path.** Unique required direct-support multiplicity, acyclic
//!    transfers, and a named route to ground from every present element. A
//!    transfer counts only when a witness of its own kind is present and
//!    measures up, so a load path can never be repaired by asserting one.
//!
//! This is not statics, finite-element analysis, capacity checking, code
//! compliance, or engineering certification. Joint stiffness, grading,
//! buckling, wind, seismic loads, and connection capacity are all outside it.
//!
//! Diagnostics are deterministic: they come out in category order and, within
//! a category, in registration order, and every one is keyed by a stable key.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};

use hashbrown::HashSet;

use exedra_math::{dot, finite, is_orthogonal_frame, is_unit, norm, sub};

use crate::construction::{Construction, ElementId};
use crate::element::Element;
use crate::evidence::Evidence;
use crate::geometry::{FRAME_TOLERANCE, Vec3, interval_overlap};
use crate::relation::RelationKind;
use crate::rule::{ContactMeaning, ContactPatch, TransferEdge, TransferKind, TransferTarget};

/// The documented contact tolerance, in metres.
///
/// Signed gaps and penetrations within this distance are treated as
/// coincident. It is a published number, not an implementation detail:
/// contact claims mean nothing without the tolerance they were made at.
pub const CONTACT_TOLERANCE: f64 = 1.0e-9;

/// One validation finding, keyed by a stable key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// The key this finding is about.
    pub key: String,
    /// Human-readable detail, including any measurement that decided it.
    pub message: String,
}

/// Everything validation found, in deterministic order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidationReport {
    /// The findings.
    pub issues: Vec<Diagnostic>,
}

impl ValidationReport {
    /// Whether nothing was found.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    /// Whether `code` was reported against `key`.
    #[must_use]
    pub fn has(&self, code: &str, key: &str) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.code == code && issue.key == key)
    }

    /// Whether `code` was reported at all.
    #[must_use]
    pub fn has_code(&self, code: &str) -> bool {
        self.issues.iter().any(|issue| issue.code == code)
    }

    fn push(&mut self, code: &'static str, key: &str, message: String) {
        self.issues.push(Diagnostic {
            code,
            key: key.to_string(),
            message,
        });
    }
}

impl core::fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_clean() {
            return f.write_str("construction validation: clean");
        }
        writeln!(f, "construction validation: {} issue(s)", self.issues.len())?;
        for issue in &self.issues {
            writeln!(f, "{} {}: {}", issue.code, issue.key, issue.message)?;
        }
        Ok(())
    }
}

/// What a contact patch actually measures, in world space.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ContactMeasurement {
    /// World position of the carried anchor.
    pub carried_point: Vec3,
    /// World position of the carrier anchor.
    pub carrier_point: Vec3,
    /// Signed separation along the contact normal: positive floats, negative
    /// penetrates.
    pub gap: f64,
    /// Anchor offsets along each tangent; both must vanish for the anchors to
    /// be the same point.
    pub tangent_offsets: [f64; 2],
    /// Measured overlap of the two extents along each tangent.
    pub overlap: [f64; 2],
}

impl ContactMeasurement {
    /// Whether the two anchors coincide in the complete frame within
    /// [`CONTACT_TOLERANCE`].
    #[must_use]
    pub fn anchors_coincide(&self) -> bool {
        self.gap.abs() <= CONTACT_TOLERANCE
            && self
                .tangent_offsets
                .iter()
                .all(|offset| offset.abs() <= CONTACT_TOLERANCE)
    }
}

/// Measures a contact patch against the two extents it names.
///
/// Returns `None` when either anchor element is not registered.
#[must_use]
pub fn measure_contact(
    construction: &Construction,
    contact: &ContactPatch,
) -> Option<ContactMeasurement> {
    let carried = construction.element(&contact.carried.element)?;
    let carrier = construction.element(&contact.carrier.element)?;
    let carried_point = carried.extent.anchor(contact.carried.local);
    let carrier_point = carrier.extent.anchor(contact.carrier.local);
    let delta = sub(carried_point, carrier_point);
    Some(ContactMeasurement {
        carried_point,
        carrier_point,
        gap: dot(delta, contact.normal),
        tangent_offsets: contact.tangents.map(|tangent| dot(delta, tangent)),
        overlap: contact.tangents.map(|tangent| {
            interval_overlap(
                carried.extent.projection_interval(tangent),
                carrier.extent.projection_interval(tangent),
            )
        }),
    })
}

/// Whether a load-path edge has a witness of its own kind that measures up.
///
/// This is the whole of the load-path claim: an edge with no witness carries
/// nothing, however confidently it was authored.
#[must_use]
pub fn is_witnessed(construction: &Construction, transfer: &TransferEdge) -> bool {
    let Some(from) = construction.element(&transfer.from) else {
        return false;
    };
    if !from.present {
        return false;
    }
    match (transfer.kind, &transfer.to) {
        (TransferKind::Contact, TransferTarget::Element(to)) => {
            construction.contacts().iter().any(|contact| {
                contact.meaning.carries_load()
                    && contact.carried.element == transfer.from
                    && contact.carrier.element == *to
                    && contact_is_valid_witness(construction, contact)
            })
        }
        (TransferKind::Joint, TransferTarget::Element(to)) => {
            joint_witness(construction, &transfer.from, to)
        }
        (TransferKind::Ground, TransferTarget::Support(support)) => {
            construction.support(support).is_some_and(|support| {
                support.element == transfer.from
                    && !support.ground.is_empty()
                    && support.restraints.translation.iter().any(|value| *value)
            })
        }
        _ => false,
    }
}

fn joint_witness(construction: &Construction, from: &str, to: &str) -> bool {
    let (Some(from_element), Some(to_element)) =
        (construction.element(from), construction.element(to))
    else {
        return false;
    };
    if !from_element.present || !to_element.present {
        return false;
    }
    construction.relations().iter().any(|relation| {
        let RelationKind::MemberMember { node, members } = &relation.kind else {
            return false;
        };
        let Some(node) = construction.node(node) else {
            return false;
        };
        let incident =
            |element: &Element| element.extent.contains_point(node.point, CONTACT_TOLERANCE);
        if !incident(from_element) || !incident(to_element) {
            return false;
        }
        let owns = |element: &str| {
            members.iter().any(|member| {
                construction
                    .member(member)
                    .is_some_and(|member| member.element == element)
            })
        };
        owns(from) && owns(to)
    })
}

fn contact_is_valid_witness(construction: &Construction, contact: &ContactPatch) -> bool {
    let (Some(carried), Some(carrier)) = (
        construction.element(&contact.carried.element),
        construction.element(&contact.carrier.element),
    ) else {
        return false;
    };
    if !carried.present
        || !carrier.present
        || contact.carried.element == contact.carrier.element
        || !valid_contact_frame(contact.normal, contact.tangents)
        || !carried
            .extent
            .contains_local(contact.carried.local, CONTACT_TOLERANCE)
        || !carrier
            .extent
            .contains_local(contact.carrier.local, CONTACT_TOLERANCE)
        || contact
            .minimum_overlap
            .iter()
            .any(|minimum| !minimum.is_finite() || *minimum < 0.0)
    {
        return false;
    }
    let Some(measurement) = measure_contact(construction, contact) else {
        return false;
    };
    measurement.anchors_coincide()
        && measurement
            .overlap
            .iter()
            .zip(contact.minimum_overlap.iter())
            .all(|(overlap, minimum)| overlap + CONTACT_TOLERANCE >= *minimum)
}

fn valid_contact_frame(normal: Vec3, tangents: [Vec3; 2]) -> bool {
    is_unit(normal, FRAME_TOLERANCE)
        && tangents
            .iter()
            .all(|tangent| is_unit(*tangent, FRAME_TOLERANCE))
        && is_orthogonal_frame([normal, tangents[0], tangents[1]], FRAME_TOLERANCE)
}

/// Traces a named route from `element` to ground, or `None` when no witnessed
/// route reaches one.
///
/// The route is a list of keys: the elements load passes through, then the
/// support it leaves by, then that support's ground name. Search order
/// follows transfer registration order, so the route is deterministic.
#[must_use]
pub fn trace_to_ground(construction: &Construction, element: &str) -> Option<Vec<String>> {
    let start = construction.element_id(element)?;
    if !construction.element_by_id(start).is_some_and(|e| e.present) {
        return None;
    }
    let transfers = construction.transfers();
    let mut path = vec![start];
    let mut on_path: HashSet<ElementId> = HashSet::new();
    on_path.insert(start);
    let mut cursor = vec![0_usize];

    while let (Some(&current), Some(&resume)) = (path.last(), cursor.last()) {
        let current_key = construction
            .element_by_id(current)
            .map(|element| element.key.as_str())
            .unwrap_or_default();
        let mut descended = false;
        for (index, transfer) in transfers.iter().enumerate().skip(resume) {
            if transfer.from != current_key || !is_witnessed(construction, transfer) {
                continue;
            }
            match &transfer.to {
                TransferTarget::Support(key) => {
                    let Some(support) = construction.support(key) else {
                        continue;
                    };
                    let mut route: Vec<String> = path
                        .iter()
                        .filter_map(|id| construction.element_by_id(*id))
                        .map(|element| element.key.clone())
                        .collect();
                    route.push(support.key.clone());
                    route.push(support.ground.clone());
                    return Some(route);
                }
                TransferTarget::Element(key) => {
                    let Some(next) = construction.element_id(key) else {
                        continue;
                    };
                    if on_path.contains(&next)
                        || !construction
                            .element_by_id(next)
                            .is_some_and(|element| element.present)
                    {
                        continue;
                    }
                    if let Some(slot) = cursor.last_mut() {
                        *slot = index + 1;
                    }
                    path.push(next);
                    on_path.insert(next);
                    cursor.push(0);
                    descended = true;
                    break;
                }
            }
        }
        if !descended {
            on_path.remove(&current);
            path.pop();
            cursor.pop();
        }
    }
    None
}

/// Runs schema, contact, and load-path validation over a whole construction.
#[must_use]
pub fn validate(construction: &Construction) -> ValidationReport {
    let mut report = ValidationReport::default();
    validate_evidence(construction, &mut report);
    validate_nodes(construction, &mut report);
    validate_elements(construction, &mut report);
    validate_members(construction, &mut report);
    validate_relations(construction, &mut report);
    validate_contacts(construction, &mut report);
    validate_supports(construction, &mut report);
    validate_transfers(construction, &mut report);
    validate_load_paths(construction, &mut report);
    validate_acyclic(construction, &mut report);
    report
}

fn validate_evidence(construction: &Construction, report: &mut ValidationReport) {
    for source in construction.evidence_sources() {
        if source.url.is_empty() || source.note.is_empty() {
            report.push(
                "invalid-evidence",
                &source.key,
                format!("{} evidence requires a URL and a note", source.class),
            );
        }
    }
    for application in construction.applications() {
        check_evidence(
            construction,
            report,
            &application.key,
            &application.evidence,
        );
    }
}

fn check_evidence(
    construction: &Construction,
    report: &mut ValidationReport,
    key: &str,
    evidence: &Evidence,
) {
    match construction.evidence_source(&evidence.source) {
        Some(source) if source.class == evidence.class => {}
        Some(source) => report.push(
            "evidence-class-mismatch",
            key,
            format!(
                "record class {} does not match source {} class {}",
                evidence.class, source.key, source.class
            ),
        ),
        None => report.push(
            "missing-evidence-source",
            key,
            format!("evidence source {:?} does not exist", evidence.source),
        ),
    }
}

fn validate_nodes(construction: &Construction, report: &mut ValidationReport) {
    for node in construction.nodes() {
        if !finite(node.point) {
            report.push(
                "non-finite-node",
                &node.key,
                "node position must be finite".to_string(),
            );
        }
        let used_by_member = construction
            .members()
            .iter()
            .any(|member| member.from == node.key || member.to == node.key);
        let used_by_relation = construction.relations().iter().any(|relation| {
            matches!(&relation.kind, RelationKind::MemberMember { node: at, .. } if *at == node.key)
        });
        if !used_by_member && !used_by_relation {
            report.push(
                "orphan-node",
                &node.key,
                "node is referenced by no member and no relation".to_string(),
            );
        }
    }
}

fn validate_elements(construction: &Construction, report: &mut ValidationReport) {
    for element in construction.elements() {
        if !element.extent.is_well_formed() {
            report.push(
                "invalid-element-geometry",
                &element.key,
                format!(
                    "{} extent must be finite, positively sized, and orthonormal ({})",
                    element.role, element.evidence.class
                ),
            );
        }
        check_evidence(construction, report, &element.key, &element.evidence);
        let members = construction.members_of(&element.key).count();
        if element.present && element.requires_member && members == 0 {
            report.push(
                "orphan-element",
                &element.key,
                "element declares a centreline but owns no member".to_string(),
            );
        }
        if members > 1 {
            report.push(
                "duplicate-element-member",
                &element.key,
                format!("element is owned by {members} members"),
            );
        }
        let edits = construction.part_edits_for(&element.key).count();
        if edits > 0 && element.part.is_none() {
            report.push(
                "part-edit-without-part",
                &element.key,
                format!("{edits} part edit(s) target an element that bears no geometry"),
            );
        }
        if edits > 0 && !element.present {
            report.push(
                "part-edit-on-omitted-element",
                &element.key,
                format!("{edits} part edit(s) target an omitted element"),
            );
        }
    }
    for edit in construction.part_edits() {
        check_evidence(
            construction,
            report,
            edit.op.tool().key.as_str(),
            &edit.evidence,
        );
    }
}

fn validate_members(construction: &Construction, report: &mut ValidationReport) {
    for member in construction.members() {
        check_evidence(construction, report, &member.key, &member.evidence);
        let (Some(element), Some(from), Some(to)) = (
            construction.element(&member.element),
            construction.node(&member.from),
            construction.node(&member.to),
        ) else {
            continue;
        };
        if !element.present {
            report.push(
                "member-on-omitted-element",
                &member.key,
                format!("member belongs to omitted element {}", element.key),
            );
        }
        if member.from == member.to || norm(sub(from.point, to.point)) <= CONTACT_TOLERANCE {
            report.push(
                "coincident-member-endpoints",
                &member.key,
                "member endpoints must be distinct".to_string(),
            );
        }
        for (label, node) in [("from", from), ("to", to)] {
            if !element.extent.contains_point(node.point, CONTACT_TOLERANCE) {
                report.push(
                    "member-endpoint-outside-extent",
                    &member.key,
                    format!(
                        "{label} node {} lies outside the extent of {}",
                        node.key, element.key
                    ),
                );
            }
        }
    }
}

fn validate_relations(construction: &Construction, report: &mut ValidationReport) {
    for relation in construction.relations() {
        check_evidence(construction, report, &relation.key, &relation.evidence);
        let fitted = construction
            .applications()
            .iter()
            .any(|application| application.relation == relation.key);
        match &relation.kind {
            RelationKind::HostFill { host, fills } => {
                report_omitted(construction, report, &relation.key, core::iter::once(host));
                report_omitted(construction, report, &relation.key, fills.iter());
                if fills.is_empty() && !fitted {
                    report.push(
                        "unfitted-relation",
                        &relation.key,
                        "host/fill relation has no fills and no rule application".to_string(),
                    );
                }
            }
            RelationKind::ElementUnits { whole, units } => {
                report_omitted(construction, report, &relation.key, core::iter::once(whole));
                report_omitted(construction, report, &relation.key, units.iter());
                if units.is_empty() && !fitted {
                    report.push(
                        "unfitted-relation",
                        &relation.key,
                        "element/units relation has no units and no rule application".to_string(),
                    );
                }
            }
            RelationKind::MemberMember { node, members } => {
                let Some(node) = construction.node(node) else {
                    continue;
                };
                for member in members {
                    let Some(member) = construction.member(member) else {
                        continue;
                    };
                    let Some(element) = construction.element(&member.element) else {
                        continue;
                    };
                    if !element.present {
                        report.push(
                            "omitted-relation-participant",
                            &relation.key,
                            format!("member {} belongs to omitted {}", member.key, element.key),
                        );
                        continue;
                    }
                    if !element.extent.contains_point(node.point, CONTACT_TOLERANCE) {
                        report.push(
                            "relation-not-incident-to-member",
                            &relation.key,
                            format!(
                                "node {} lies outside the extent of member {}",
                                node.key, member.key
                            ),
                        );
                    }
                }
            }
        }
    }
}

fn report_omitted<'a>(
    construction: &Construction,
    report: &mut ValidationReport,
    relation: &str,
    keys: impl Iterator<Item = &'a String>,
) {
    for key in keys {
        if construction
            .element(key)
            .is_some_and(|element| !element.present)
        {
            report.push(
                "omitted-relation-participant",
                relation,
                format!("element {key} is omitted"),
            );
        }
    }
}

fn validate_contacts(construction: &Construction, report: &mut ValidationReport) {
    for contact in construction.contacts() {
        check_evidence(construction, report, &contact.key, &contact.evidence);
        let (Some(carried), Some(carrier)) = (
            construction.element(&contact.carried.element),
            construction.element(&contact.carrier.element),
        ) else {
            continue;
        };
        if contact.carried.element == contact.carrier.element {
            report.push(
                "self-contact",
                &contact.key,
                "carried and carrier elements must differ".to_string(),
            );
        }
        if !valid_contact_frame(contact.normal, contact.tangents) {
            report.push(
                "invalid-contact-frame",
                &contact.key,
                "normal and tangents must form a finite orthonormal frame".to_string(),
            );
        }
        if !carried
            .extent
            .contains_local(contact.carried.local, CONTACT_TOLERANCE)
            || !carrier
                .extent
                .contains_local(contact.carrier.local, CONTACT_TOLERANCE)
        {
            report.push(
                "contact-anchor-out-of-bounds",
                &contact.key,
                "anchors must lie inside the local bounds of their own extent".to_string(),
            );
        }
        if contact
            .minimum_overlap
            .iter()
            .any(|minimum| !minimum.is_finite() || *minimum < 0.0)
        {
            report.push(
                "invalid-contact-minimum",
                &contact.key,
                "minimum overlaps must be finite and nonnegative".to_string(),
            );
        }
        if !carried.present || !carrier.present {
            report.push(
                "missing-contact-element",
                &contact.key,
                format!(
                    "{} contact references omitted carried or carrier geometry ({})",
                    contact.meaning.label(),
                    contact.evidence.class
                ),
            );
            continue;
        }
        let Some(measurement) = measure_contact(construction, contact) else {
            continue;
        };
        if contact.meaning == ContactMeaning::ClearanceOnly {
            // The inverse assertion: a clearance is proven by *not* touching.
            if measurement.gap < -CONTACT_TOLERANCE {
                report.push(
                    "embedded-contact",
                    &contact.key,
                    format!(
                        "clearance-only contact penetrates by {:.12} m",
                        -measurement.gap
                    ),
                );
            }
            continue;
        }
        if !measurement.anchors_coincide() {
            report.push(
                "misaligned-contact-anchors",
                &contact.key,
                format!(
                    "anchor frame offsets are normal={:.12} m, tangent-0={:.12} m, tangent-1={:.12} m",
                    measurement.gap,
                    measurement.tangent_offsets[0],
                    measurement.tangent_offsets[1]
                ),
            );
        }
        if measurement.gap > CONTACT_TOLERANCE {
            report.push(
                "floating-contact",
                &contact.key,
                format!("signed gap is {:.12} m", measurement.gap),
            );
        } else if measurement.gap < -CONTACT_TOLERANCE {
            report.push(
                "embedded-contact",
                &contact.key,
                format!("signed penetration is {:.12} m", -measurement.gap),
            );
        }
        for (axis, overlap) in measurement.overlap.into_iter().enumerate() {
            if overlap + CONTACT_TOLERANCE < contact.minimum_overlap[axis] {
                report.push(
                    "insufficient-contact-overlap",
                    &contact.key,
                    format!(
                        "axis {axis} overlap {overlap:.6} m is below {:.6} m",
                        contact.minimum_overlap[axis]
                    ),
                );
            }
        }
    }
}

fn validate_supports(construction: &Construction, report: &mut ValidationReport) {
    for support in construction.supports() {
        if construction
            .element(&support.element)
            .is_none_or(|element| !element.present)
            || support.ground.is_empty()
        {
            report.push(
                "invalid-support",
                &support.key,
                "support must reference a present element and a named ground".to_string(),
            );
        }
        if !support.restraints.translation.iter().any(|value| *value) {
            report.push(
                "unrestrained-support",
                &support.key,
                "at least one translational restraint is required".to_string(),
            );
        }
        if !construction.transfers().iter().any(|transfer| {
            transfer.kind == TransferKind::Ground
                && transfer.to == TransferTarget::Support(support.key.clone())
        }) {
            report.push(
                "orphan-support",
                &support.key,
                "support is not targeted by a ground transfer".to_string(),
            );
        }
    }
}

fn validate_transfers(construction: &Construction, report: &mut ValidationReport) {
    let mut routes: HashSet<(&str, &TransferTarget, TransferKind)> = HashSet::new();
    for transfer in construction.transfers() {
        let from_present = construction
            .element(&transfer.from)
            .is_some_and(|element| element.present);
        let to_present = match &transfer.to {
            TransferTarget::Element(key) => construction
                .element(key)
                .is_some_and(|element| element.present),
            TransferTarget::Support(key) => construction.support(key).is_some(),
        };
        if !from_present || !to_present {
            report.push(
                "missing-transfer-element",
                &transfer.key,
                format!(
                    "{} transfer references omitted graph state",
                    transfer.kind.label()
                ),
            );
        }
        if !routes.insert((transfer.from.as_str(), &transfer.to, transfer.kind)) {
            report.push(
                "duplicate-transfer-route",
                &transfer.key,
                "duplicate transfers do not add support multiplicity".to_string(),
            );
        }
        if from_present && to_present && !is_witnessed(construction, transfer) {
            report.push(
                transfer.kind.unwitnessed_code(),
                &transfer.key,
                format!("{} transfer has no matching witness", transfer.kind.label()),
            );
        }
    }
}

fn validate_load_paths(construction: &Construction, report: &mut ValidationReport) {
    for element in construction.elements() {
        if !element.present {
            continue;
        }
        let targets: HashSet<&TransferTarget> = construction
            .transfers()
            .iter()
            .filter(|transfer| transfer.from == element.key)
            .filter(|transfer| is_witnessed(construction, transfer))
            .map(|transfer| &transfer.to)
            .collect();
        if targets.len() < element.required_supports {
            report.push(
                "insufficient-direct-supports",
                &element.key,
                format!(
                    "requires {} direct supports but has {}",
                    element.required_supports,
                    targets.len()
                ),
            );
        }
        if trace_to_ground(construction, &element.key).is_none() {
            report.push(
                "no-ground-path",
                &element.key,
                "no witnessed load-transfer route reaches ground".to_string(),
            );
        }
    }
}

/// Iterative three-colour depth-first search: hostile or deeply chained
/// constructions report a cycle instead of exhausting the stack.
fn validate_acyclic(construction: &Construction, report: &mut ValidationReport) {
    let count = construction.elements().len();
    let mut permanent = vec![false; count];
    let mut temporary = vec![false; count];
    let transfers = construction.transfers();

    let key_of = |index: usize| -> Option<&str> {
        u32::try_from(index)
            .ok()
            .and_then(|index| construction.element_by_id(ElementId(index)))
            .map(|element| element.key.as_str())
    };

    for start in 0..count {
        if permanent[start] {
            continue;
        }
        let mut stack = vec![(start, 0_usize)];
        temporary[start] = true;
        while let Some((current, resume)) = stack.last().copied() {
            let Some(current_key) = key_of(current) else {
                stack.pop();
                continue;
            };
            let mut descended = None;
            let mut next_resume = transfers.len();
            for (index, transfer) in transfers.iter().enumerate().skip(resume) {
                if transfer.from != current_key {
                    continue;
                }
                let TransferTarget::Element(key) = &transfer.to else {
                    continue;
                };
                let Some(next_id) = construction.element_id(key) else {
                    continue;
                };
                if !construction
                    .element_by_id(next_id)
                    .is_some_and(|element| element.present)
                {
                    continue;
                }
                let next = next_id.0 as usize;
                if temporary[next] {
                    report.push(
                        "load-transfer-cycle",
                        key,
                        "directed load transfers must be acyclic".to_string(),
                    );
                    return;
                }
                if !permanent[next] {
                    next_resume = index + 1;
                    descended = Some(next);
                    break;
                }
            }
            match descended {
                Some(next) => {
                    if let Some(slot) = stack.last_mut() {
                        slot.1 = next_resume;
                    }
                    temporary[next] = true;
                    stack.push((next, 0));
                }
                None => {
                    temporary[current] = false;
                    permanent[current] = true;
                    stack.pop();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Evidence, EvidenceClass, EvidenceSource, Member, Node, OrientedBox, Relation,
        TransferTarget,
    };

    fn evidence() -> Evidence {
        Evidence::new("fixture", EvidenceClass::ModernEngineeringInference)
    }

    #[test]
    fn member_relation_incidence_uses_extents_not_centrelines() {
        let mut construction = Construction::new();
        construction
            .add_evidence_source(EvidenceSource::new(
                "fixture",
                EvidenceClass::ModernEngineeringInference,
                "https://example.invalid",
                "test fixture",
            ))
            .expect("evidence");
        for (key, point) in [
            ("a0", [0.0, 0.0, 0.5]),
            ("a1", [1.0, 0.0, 0.5]),
            ("b0", [0.0, 1.0, 0.5]),
            ("b1", [1.0, 1.0, 0.5]),
            ("off", [0.5, 0.5, 0.5]),
        ] {
            construction.add_node(Node::new(key, point)).expect("node");
        }
        for key in ["a", "b"] {
            construction
                .add_element(
                    Element::new(
                        key,
                        "member",
                        "wood",
                        OrientedBox::axis_aligned([0.0; 3], [1.0; 3]),
                        evidence(),
                    )
                    .with_member(),
                )
                .expect("element");
        }
        construction
            .add_member(Member::new("ma", "a", "a0", "a1", evidence()))
            .expect("member");
        construction
            .add_member(Member::new("mb", "b", "b0", "b1", evidence()))
            .expect("member");
        construction
            .add_relation(Relation::new(
                "joint",
                RelationKind::member_member("off", &["ma", "mb"]),
                "review-joint",
                evidence(),
            ))
            .expect("relation");
        let transfer = TransferEdge::new(
            "load-a-to-b",
            "a",
            TransferTarget::element("b"),
            TransferKind::Joint,
        );
        construction
            .add_transfer(transfer.clone())
            .expect("transfer");

        let report = validate(&construction);
        assert!(
            !report.has("relation-not-incident-to-member", "joint"),
            "solid incidence does not require centreline intersection"
        );
        assert!(is_witnessed(&construction, &transfer));
    }
}
