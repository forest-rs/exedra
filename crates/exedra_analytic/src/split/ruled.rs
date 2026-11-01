// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::body::ruled::{RuledRectangle, RuledSide};
use crate::body::{BodyParts, BoundaryLoop, Coedge, EdgeGeometry, Face, Sense, TrimCurve};
use crate::geometry::{Curve, CurveSpan, Surface};
use alloc::{vec, vec::Vec};
use exedra_math::{norm, sub};

struct RimPlan {
    report: RuledRimSplit,
    point: [f64; 3],
    reverse: bool,
}

pub(super) fn split<F: Clone, E: Clone>(
    body: &Body<F, E>,
    face_id: FaceId,
    u: f64,
    ruling_role: E,
    policy: &RuledSplitPolicy,
) -> Result<RuledSplitResult<F, E>, RuledSplitError> {
    use RuledSplitError as Error;
    if !policy.parameter_clearance.is_finite()
        || policy.parameter_clearance <= 0.0
        || !policy.vertex_clearance.is_finite()
        || policy.vertex_clearance <= policy.validation.max_gap
    {
        return Err(Error::InvalidPolicy);
    }
    let face = body
        .face(face_id)
        .ok_or(Error::MissingFace { face: face_id })?;
    let [a, b] = face.domain.u.bounds();
    if !u.is_finite() || u - a <= policy.parameter_clearance || b - u <= policy.parameter_clearance
    {
        return Err(Error::Parameter {
            u,
            domain: face.domain.u,
            clearance: policy.parameter_clearance,
        });
    }
    // Count detached loop occurrences too: they are validated and cannot hide
    // unbudgeted inspection or silently keep references to a replaced rim.
    let input_coedges = body
        .loops()
        .iter()
        .map(|l| l.coedges.len())
        .try_fold(0_usize, usize::checked_add)
        .ok_or(Error::Budget {
            resource: "input coedges",
            required: usize::MAX,
            maximum: policy.validation.max_coedges,
        })?;
    budget(
        "input coedges",
        input_coedges,
        policy.validation.max_coedges,
    )?;
    let mut source_budget =
        crate::geometry::SourceBudget::new(policy.max_source_controls, policy.max_source_work);
    let input_validation = body
        .validate_boundary_with_source_budget(&policy.validation, &mut source_budget)
        .map_err(|error| source_refusal(&source_budget).unwrap_or(Error::InputBoundary(error)))?;
    drop(input_validation);
    if let Some(error) = source_refusal(&source_budget) {
        return Err(error);
    }
    let rectangle = body
        .ruled_rectangle_with_source_budget(face_id, &mut source_budget)
        .ok_or_else(|| {
            let unsupported = Error::UnsupportedFace {
                face: face_id,
                reason: "requires one checked four-edge original ruled rectangle, with full V and U inside [0,1]",
            };
            source_refusal(&source_budget).unwrap_or(unsupported)
        })?;
    for (i, side) in rectangle.sides.iter().enumerate() {
        if rectangle.sides[..i].iter().any(|s| s.edge == side.edge) {
            return Err(Error::UnsupportedFace {
                face: face_id,
                reason: "periodic or repeated boundary edges are unsupported",
            });
        }
    }
    let target_loop = face.loops[0];
    if body.trims().iter().any(|t| t.loop_id == target_loop) {
        return Err(Error::UnsupportedFace {
            face: face_id,
            reason: "explicit ruled-face trim records are outside this split subset",
        });
    }
    preflight_output(body, input_coedges, policy)?;
    let Surface::Ruled(support) = &face.surface else {
        unreachable!("rectangle classifier");
    };
    let plans = [
        plan_rim(
            body,
            face_id,
            u,
            0,
            rectangle.sides[0],
            &support.curves()[0],
            policy,
        )?,
        plan_rim(
            body,
            face_id,
            u,
            1,
            rectangle.sides[2],
            &support.curves()[1],
            policy,
        )?,
    ];
    if plans[0].report.neighbor_loop == plans[1].report.neighbor_loop {
        return Err(Error::UnsupportedFace {
            face: face_id,
            reason: "both rims in one neighboring loop are unsupported",
        });
    }
    let pair_work = body
        .faces()
        .iter()
        .enumerate()
        .try_fold(0_usize, |sum, (index, face)| {
            let pairs = if index == face_id.index() {
                Some(32)
            } else {
                face.loops
                    .iter()
                    .try_fold(0_usize, |n, &id| {
                        let extra = plans
                            .iter()
                            .filter(|p| p.report.neighbor_loop == id)
                            .count();
                        n.checked_add(
                            body.loops()[id.index()]
                                .coedges
                                .len()
                                .checked_add(extra)?
                                .max(4),
                        )
                    })
                    .and_then(|n| n.checked_mul(n))
            }?;
            sum.checked_add(pairs)
        })
        .unwrap_or(usize::MAX);
    budget(
        "output trim pair work",
        pair_work,
        policy.validation.max_pair_tests,
    )?;
    let ruling_length = norm(sub(plans[1].point, plans[0].point));
    if !ruling_length.is_finite() || ruling_length <= policy.vertex_clearance {
        return Err(Error::NearVertex {
            edge: plans[0].report.source,
            distance: ruling_length,
            clearance: policy.vertex_clearance,
        });
    }
    let ruling = EdgeId(crate::usize_to_u32(body.edges().len() + 2));
    let domains = [
        ParameterDomain {
            u: ParameterRange::new(a, u).map_err(Error::Geometry)?,
            v: face.domain.v,
        },
        ParameterDomain {
            u: ParameterRange::new(u, b).map_err(Error::Geometry)?,
            v: face.domain.v,
        },
    ];
    let output_domains = body.faces().iter().enumerate().flat_map(|(index, face)| {
        let domains = if index == face_id.index() {
            [Some(domains[0]), Some(domains[1])]
        } else {
            [Some(face.domain), None]
        };
        domains
            .into_iter()
            .flatten()
            .filter_map(move |domain| match &face.surface {
                Surface::Ruled(ruled) => Some((ruled, domain)),
                _ => None,
            })
    });
    if !source_budget.preflight(output_domains) {
        return Err(
            source_refusal(&source_budget).unwrap_or(Error::UnsupportedFace {
                face: face_id,
                reason: "original source restriction proof cannot be prepared",
            }),
        );
    }
    let upper_face = FaceId(crate::usize_to_u32(body.faces().len()));
    let upper_loop = LoopId(crate::usize_to_u32(body.loops().len()));
    // Reserve final table sizes before cloning; an exact-length Body clone
    // followed by growth would allocate each immutable output table twice.
    fn copied<T: Clone>(source: &[T], extra: usize) -> Vec<T> {
        let mut result = Vec::with_capacity(source.len() + extra);
        result.extend_from_slice(source);
        result
    }
    let mut parts = BodyParts {
        vertices: copied(body.vertices(), 2),
        edges: copied(body.edges(), 3),
        faces: copied(body.faces(), 1),
        loops: copied(body.loops(), 1),
        trims: copied(body.trims(), 2),
    };
    for plan in &plans {
        parts.vertices.push(plan.point);
        let original = &body.edges()[plan.report.source.index()];
        let mut upper = original.clone();
        set_child(&mut upper, plan, 1, original.start, original.end);
        parts.edges.push(upper);
        set_child(
            &mut parts.edges[plan.report.source.index()],
            plan,
            0,
            original.start,
            original.end,
        );
        rewrite_neighbor(&mut parts, body, plan)?;
    }
    parts.edges.push(crate::body::Edge::new(
        Curve::line(plans[0].point, sub(plans[1].point, plans[0].point))
            .map_err(Error::Geometry)?,
        ParameterRange::UNIT,
        plans[0].report.vertex,
        plans[1].report.vertex,
        ruling_role,
    ));
    let loops = [
        child_loop(rectangle, &plans, ruling, false, face.sense),
        child_loop(rectangle, &plans, ruling, true, face.sense),
    ];
    let [lower, upper] = loops;
    parts.loops[target_loop.index()] = lower;
    parts.loops.push(upper);
    parts.faces[face_id.index()].domain = domains[0];
    parts.faces.push(Face {
        surface: face.surface.clone(),
        domain: domains[1],
        sense: face.sense,
        loops: vec![upper_loop],
        role: face.role.clone(),
    });
    // Temporary tables stay private until complete geometry/topology admission.
    let output = Body { parts };
    let validation = output
        .validate_boundary_with_source_budget(&policy.validation, &mut source_budget)
        .map_err(|error| source_refusal(&source_budget).unwrap_or(Error::OutputBoundary(error)))?;
    if let Some(error) = source_refusal(&source_budget) {
        return Err(error);
    }
    Ok(RuledSplitResult {
        body: output,
        report: RuledSplitReport {
            source: face_id,
            source_domain: face.domain,
            u,
            faces: [
                RuledFaceFragment {
                    face: face_id,
                    domain: domains[0],
                },
                RuledFaceFragment {
                    face: upper_face,
                    domain: domains[1],
                },
            ],
            rims: plans.map(|p| p.report),
            ruling,
            work: RuledSplitWork {
                input_coedges,
                source_controls: source_budget.controls,
                source_metadata_work: source_budget.metadata_work,
                source_preparation_work: source_budget.preparation_work,
                source_enclosure_work: source_budget.enclosure_work,
            },
            validation,
        },
    })
}

fn source_refusal(budget: &crate::geometry::SourceBudget) -> Option<RuledSplitError> {
    budget.refusal.map(|r| RuledSplitError::SourceBudget {
        resource: r.resource,
        required: r.required,
        maximum: r.maximum,
    })
}

fn budget(resource: &'static str, required: usize, maximum: usize) -> Result<(), RuledSplitError> {
    if required > maximum {
        Err(RuledSplitError::Budget {
            resource,
            required,
            maximum,
        })
    } else {
        Ok(())
    }
}
fn preflight_output<F, E>(
    body: &Body<F, E>,
    coedges: usize,
    policy: &RuledSplitPolicy,
) -> Result<(), RuledSplitError> {
    budget(
        "output coedges",
        coedges.saturating_add(6),
        policy.validation.max_coedges,
    )?;
    let scratch = body
        .edges()
        .len()
        .saturating_mul(2)
        .saturating_add(body.vertices().len())
        .saturating_add(body.loops().len())
        .saturating_add(body.faces().len().saturating_mul(2))
        .saturating_add(11);
    budget(
        "output topology scratch",
        scratch,
        policy.validation.max_coedges.saturating_mul(4),
    )?;
    for (name, count, extra) in [
        ("vertices", body.vertices().len(), 2),
        ("edges", body.edges().len(), 3),
        ("faces", body.faces().len(), 1),
        ("loops", body.loops().len(), 1),
    ] {
        budget(name, count.saturating_add(extra), u32::MAX as usize)?;
    }
    Ok(())
}
fn plan_rim<F, E>(
    body: &Body<F, E>,
    face_id: FaceId,
    u: f64,
    index: usize,
    side: RuledSide,
    source: &CurveSpan,
    policy: &RuledSplitPolicy,
) -> Result<RimPlan, RuledSplitError> {
    use RuledSplitError as Error;
    let unsupported = |reason| Error::UnsupportedRim {
        edge: side.edge,
        reason,
    };
    let edge = &body.edges()[side.edge.index()];
    let (curve, range) = edge
        .regular()
        .ok_or_else(|| unsupported("collapsed boundary"))?;
    if !matches!(curve, Curve::Line(_) | Curve::Nurbs(_)) || edge.start == edge.end {
        return Err(unsupported("requires nonclosed line or NURBS rim"));
    }
    let endpoint_distance = norm(sub(
        body.vertices()[edge.start.index()],
        body.vertices()[edge.end.index()],
    ));
    if !endpoint_distance.is_finite() || endpoint_distance <= 2.0 * policy.vertex_clearance {
        return Err(unsupported(
            "closed or unresolved rim endpoints are unsupported",
        ));
    }
    if source.curve() != curve || source.reversed() != side.reverse {
        return Err(unsupported(
            "rim must retain its original generating curve and traversal",
        ));
    }
    let parameter = source
        .range()
        .at(if source.reversed() { 1.0 - u } else { u })
        .ok_or(Error::Numeric {
            edge: Some(side.edge),
        })?;
    let [lo, hi] = range.bounds();
    if !(lo < parameter && parameter < hi) {
        return Err(Error::Numeric {
            edge: Some(side.edge),
        });
    }
    let low = ParameterRange::new(lo, parameter).map_err(Error::Geometry)?;
    let high = ParameterRange::new(parameter, hi).map_err(Error::Geometry)?;
    let ranges = if side.reverse {
        [high, low]
    } else {
        [low, high]
    };
    let point = curve
        .evaluate(parameter)
        .ok_or(Error::Numeric {
            edge: Some(side.edge),
        })?
        .point;
    let distance = norm(sub(point, body.vertices()[edge.start.index()]))
        .min(norm(sub(point, body.vertices()[edge.end.index()])));
    if !distance.is_finite() || distance <= policy.vertex_clearance {
        return Err(Error::NearVertex {
            edge: side.edge,
            distance,
            clearance: policy.vertex_clearance,
        });
    }
    let mut neighbor = None;
    for (fi, face) in body.faces().iter().enumerate() {
        let id = FaceId(crate::usize_to_u32(fi));
        if id == face_id {
            continue;
        }
        for &loop_id in &face.loops {
            for co in &body.loops()[loop_id.index()].coedges {
                if co.edge != side.edge {
                    continue;
                }
                if neighbor.is_some() || !matches!(face.surface, Surface::Plane(_)) {
                    return Err(unsupported("rim needs exactly one planar neighboring face"));
                }
                neighbor = Some((id, loop_id));
            }
        }
    }
    let (neighbor, neighbor_loop) =
        neighbor.ok_or_else(|| unsupported("rim has no planar neighbor"))?;
    let uses = body
        .loops()
        .iter()
        .flat_map(|l| &l.coedges)
        .filter(|co| co.edge == side.edge)
        .count();
    if uses != 2 {
        return Err(unsupported(
            "rim is also referenced by detached or repeated loop occurrences",
        ));
    }
    if let Some(trims) = body.trims().iter().find(|t| t.loop_id == neighbor_loop) {
        for (co, trim) in body.loops()[neighbor_loop.index()]
            .coedges
            .iter()
            .zip(&trims.curves)
        {
            if co.edge == side.edge
                && !matches!(
                    (curve, trim),
                    (Curve::Line(_), TrimCurve::Line { .. })
                        | (Curve::Nurbs(_), TrimCurve::Nurbs { .. })
                )
            {
                return Err(Error::UnsupportedTrim {
                    face: neighbor,
                    edge: side.edge,
                });
            }
        }
    }
    Ok(RimPlan {
        point,
        reverse: side.reverse,
        report: RuledRimSplit {
            source: side.edge,
            source_range: range,
            children: [
                side.edge,
                EdgeId(crate::usize_to_u32(body.edges().len() + index)),
            ],
            ranges,
            parameter,
            vertex: VertexId(crate::usize_to_u32(body.vertices().len() + index)),
            neighbor,
            neighbor_loop,
        },
    })
}
fn set_child<E>(
    edge: &mut crate::body::Edge<E>,
    plan: &RimPlan,
    child: usize,
    start: VertexId,
    end: VertexId,
) {
    let first_canonical = (child == 0) != plan.reverse;
    let EdgeGeometry::Curve { range, .. } = &mut edge.geometry else {
        unreachable!("regular rim");
    };
    *range = plan.report.ranges[child];
    edge.start = if first_canonical {
        start
    } else {
        plan.report.vertex
    };
    edge.end = if first_canonical {
        plan.report.vertex
    } else {
        end
    };
}
fn rewrite_neighbor<F, E>(
    parts: &mut BodyParts<F, E>,
    source: &Body<F, E>,
    plan: &RimPlan,
) -> Result<(), RuledSplitError> {
    use RuledSplitError as Error;
    let id = plan.report.neighbor_loop;
    let old = &source.loops()[id.index()].coedges;
    let mut coedges = Vec::with_capacity(old.len() + 1);
    let trim_index = parts.trims.iter().position(|t| t.loop_id == id);
    let old_trims = trim_index.map(|i| &parts.trims[i].curves);
    let mut curves = old_trims.map(|curves| Vec::with_capacity(curves.len() + 1));
    let children = plan.report.children;
    for (i, &co) in old.iter().enumerate() {
        if co.edge != plan.report.source {
            coedges.push(co);
            if let (Some(old), Some(new)) = (old_trims, curves.as_mut()) {
                new.push(old[i].clone());
            }
            continue;
        }
        let first_canonical = if plan.reverse { [1, 0] } else { [0, 1] };
        let order = if co.reversed {
            [first_canonical[1], first_canonical[0]]
        } else {
            first_canonical
        };
        for child in order {
            coedges.push(Coedge {
                edge: children[child],
                reversed: co.reversed,
            });
            if let (Some(old), Some(new)) = (old_trims, curves.as_mut()) {
                let trim = match &old[i] {
                    TrimCurve::Nurbs { curve, .. } => TrimCurve::Nurbs {
                        curve: curve.clone(),
                        range: plan.report.ranges[child],
                    },
                    TrimCurve::Line { start, end } => {
                        let cut = source.faces()[plan.report.neighbor.index()]
                            .surface
                            .parameters(plan.point)
                            .ok_or(Error::Numeric {
                                edge: Some(plan.report.source),
                            })?;
                        if (child == 0) != plan.reverse {
                            TrimCurve::Line {
                                start: *start,
                                end: cut,
                            }
                        } else {
                            TrimCurve::Line {
                                start: cut,
                                end: *end,
                            }
                        }
                    }
                    _ => {
                        return Err(Error::UnsupportedTrim {
                            face: plan.report.neighbor,
                            edge: plan.report.source,
                        });
                    }
                };
                new.push(trim);
            }
        }
    }
    parts.loops[id.index()].coedges = coedges;
    if let (Some(index), Some(curves)) = (trim_index, curves) {
        parts.trims[index].curves = curves;
    } else {
        crate::operation_trims::append_planar_loop(parts, plan.report.neighbor, id)
            .map_err(Error::Geometry)?;
    }
    Ok(())
}
fn child_loop(
    rectangle: RuledRectangle,
    plans: &[RimPlan; 2],
    ruling: EdgeId,
    upper: bool,
    sense: Sense,
) -> BoundaryLoop {
    let child = usize::from(upper);
    let mut sides = rectangle.sides;
    sides[0].edge = plans[0].report.children[child];
    sides[2].edge = plans[1].report.children[child];
    sides[if upper { 3 } else { 1 }] = RuledSide {
        edge: ruling,
        reverse: false,
    };
    let mut coedges: Vec<_> = sides
        .into_iter()
        .enumerate()
        .map(|(i, side)| Coedge {
            edge: side.edge,
            reversed: side.reverse ^ (i >= 2),
        })
        .collect();
    if sense == Sense::Reverse {
        coedges.reverse();
        for co in &mut coedges {
            co.reversed = !co.reversed;
        }
    }
    BoundaryLoop { coedges }
}
