// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Safe participant resolution shared by concrete timber rules.

use joiner::{Element, Node, Rejection, RejectionReason, RelationKind, RuleContext};

/// The role-selected pair a two-member timber rule operates on.
pub(crate) struct ParticipantPair<'a> {
    pub(crate) node: &'a Node,
    pub(crate) carried: &'a Element,
    pub(crate) carrier: &'a Element,
}

/// Which physical end of a carried member contains the relation node.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum MemberEnd {
    Start,
    End,
}

/// A role-selected pair whose carried member may meet at either end.
pub(crate) struct EndpointPair<'a> {
    pub(crate) node: &'a Node,
    pub(crate) carried: &'a Element,
    pub(crate) carrier: &'a Element,
    pub(crate) carried_end: MemberEnd,
}

/// Resolves a two-member relation by element role, never by participant order.
///
/// Relation order is authoring order and is not a semantic contract. Keeping
/// role selection here prevents the heel and king-post rules from drifting
/// into subtly different lookup and refusal behavior.
pub(crate) fn resolve_pair<'a>(
    ctx: &'a RuleContext<'_>,
    carried_role: &'static str,
    carrier_role: &'static str,
) -> Result<ParticipantPair<'a>, Rejection> {
    let RelationKind::MemberMember { node, members } = &ctx.relation().kind else {
        return Err(Rejection::new(
            &ctx.relation().key,
            RejectionReason::WrongRelationKind {
                expected: "member/member",
                found: ctx.relation().kind.label(),
            },
        ));
    };
    if members.len() != 2 {
        return Err(Rejection::new(
            &ctx.relation().key,
            RejectionReason::ParticipantCount {
                what: "members",
                found: members.len(),
                minimum: 2,
                maximum: Some(2),
            },
        ));
    }
    let node_record = ctx.node(node).ok_or_else(|| {
        Rejection::new(
            node,
            RejectionReason::UnknownParticipant { what: "joint node" },
        )
    })?;

    let mut carried = None;
    let mut carrier = None;
    for member_key in members {
        let member = ctx.member(member_key).ok_or_else(|| {
            Rejection::new(
                member_key,
                RejectionReason::UnknownParticipant { what: "member" },
            )
        })?;
        let element = ctx.element(&member.element).ok_or_else(|| {
            Rejection::new(
                &member.element,
                RejectionReason::UnknownParticipant {
                    what: "member element",
                },
            )
        })?;
        if !element.present {
            return Err(Rejection::new(
                &element.key,
                RejectionReason::OmittedParticipant { what: "element" },
            ));
        }
        if element.role == carried_role {
            if carried.replace((member, element)).is_some() {
                return Err(Rejection::new(
                    &ctx.relation().key,
                    RejectionReason::Unsupported {
                        what: "more than one carried-role member",
                    },
                ));
            }
        } else if element.role == carrier_role && carrier.replace((member, element)).is_some() {
            return Err(Rejection::new(
                &ctx.relation().key,
                RejectionReason::Unsupported {
                    what: "more than one carrier-role member",
                },
            ));
        }
    }
    let (_, carried_element) = carried.ok_or_else(|| {
        Rejection::new(
            &ctx.relation().key,
            RejectionReason::MissingParticipant { what: carried_role },
        )
    })?;
    let (_, carrier_element) = carrier.ok_or_else(|| {
        Rejection::new(
            &ctx.relation().key,
            RejectionReason::MissingParticipant { what: carrier_role },
        )
    })?;
    if !carried_element.extent.is_well_formed() || !carrier_element.extent.is_well_formed() {
        return Err(Rejection::new(
            &ctx.relation().key,
            RejectionReason::Unsupported {
                what: "malformed member extent",
            },
        ));
    }

    Ok(ParticipantPair {
        node: node_record,
        carried: carried_element,
        carrier: carrier_element,
    })
}

/// Resolves two roles and records which carried-member endpoint is fitted.
///
/// A carrier may be crossed at an interior topology node, but the carried
/// piece must actually terminate at the relation. This is the distinction a
/// two-ended strut rule needs to orient its tenon without participant-order
/// conventions or reflected cutter recipes.
pub(crate) fn resolve_endpoint_pair<'a>(
    ctx: &'a RuleContext<'_>,
    carried_role: &'static str,
    carrier_role: &'static str,
) -> Result<EndpointPair<'a>, Rejection> {
    let RelationKind::MemberMember { node, members } = &ctx.relation().kind else {
        return Err(Rejection::new(
            &ctx.relation().key,
            RejectionReason::WrongRelationKind {
                expected: "member/member",
                found: ctx.relation().kind.label(),
            },
        ));
    };
    if members.len() != 2 {
        return Err(Rejection::new(
            &ctx.relation().key,
            RejectionReason::ParticipantCount {
                what: "members",
                found: members.len(),
                minimum: 2,
                maximum: Some(2),
            },
        ));
    }
    let node_record = ctx.node(node).ok_or_else(|| {
        Rejection::new(
            node,
            RejectionReason::UnknownParticipant { what: "joint node" },
        )
    })?;

    let mut carried = None;
    let mut carrier = None;
    for member_key in members {
        let member = ctx.member(member_key).ok_or_else(|| {
            Rejection::new(
                member_key,
                RejectionReason::UnknownParticipant { what: "member" },
            )
        })?;
        let element = ctx.element(&member.element).ok_or_else(|| {
            Rejection::new(
                &member.element,
                RejectionReason::UnknownParticipant {
                    what: "member element",
                },
            )
        })?;
        if !element.present {
            return Err(Rejection::new(
                &element.key,
                RejectionReason::OmittedParticipant { what: "element" },
            ));
        }
        if element.role == carried_role {
            if carried.replace((member, element)).is_some() {
                return Err(Rejection::new(
                    &ctx.relation().key,
                    RejectionReason::Unsupported {
                        what: "more than one carried-role member",
                    },
                ));
            }
        } else if element.role == carrier_role && carrier.replace(element).is_some() {
            return Err(Rejection::new(
                &ctx.relation().key,
                RejectionReason::Unsupported {
                    what: "more than one carrier-role member",
                },
            ));
        }
    }
    let (carried_member, carried_element) = carried.ok_or_else(|| {
        Rejection::new(
            &ctx.relation().key,
            RejectionReason::MissingParticipant { what: carried_role },
        )
    })?;
    let carrier_element = carrier.ok_or_else(|| {
        Rejection::new(
            &ctx.relation().key,
            RejectionReason::MissingParticipant { what: carrier_role },
        )
    })?;
    let carried_end = if carried_member.from == node_record.key {
        MemberEnd::Start
    } else if carried_member.to == node_record.key {
        MemberEnd::End
    } else {
        return Err(Rejection::new(
            &carried_member.key,
            RejectionReason::Unsupported {
                what: "carried member does not end at the relation node",
            },
        ));
    };
    if !carried_element.extent.is_well_formed() || !carrier_element.extent.is_well_formed() {
        return Err(Rejection::new(
            &ctx.relation().key,
            RejectionReason::Unsupported {
                what: "malformed member extent",
            },
        ));
    }

    Ok(EndpointPair {
        node: node_record,
        carried: carried_element,
        carrier: carrier_element,
        carried_end,
    })
}
