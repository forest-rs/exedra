// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Resolution-independent structural rewrites for stretch.

use alloc::vec::Vec;

use kurbo::Point;

use crate::ir::{CapMode, NodeId, NodeKind, Placement3, Plane3, PrimitiveSpec, Recipe};
use crate::profile::{Loop2, Profile2, Seg2, SegKind};
use crate::tessellate::{
    EvalPolicy, ExtrudeWallSource, REGION_WALL_BASE, TessellateError, TessellatedBody,
    tessellate_extrude_with_wall_sources, tessellate_primitive,
};

use super::StretchRefusal;

/// A resolution-independent constructive rewrite ready for tessellation.
pub(crate) enum ExactStretchPlan {
    Primitive {
        spec: PrimitiveSpec,
        placement: Placement3,
        stretch_nodes: u32,
    },
    Extrude {
        profile: Profile2,
        wall_sources: Vec<Vec<ExtrudeWallSource>>,
        placement: Placement3,
        height: f64,
        caps: CapMode,
        stretch_nodes: u32,
    },
}

impl ExactStretchPlan {
    pub(crate) fn tessellate(
        &self,
        policy: &EvalPolicy,
    ) -> Result<TessellatedBody, TessellateError> {
        match self {
            Self::Primitive {
                spec, placement, ..
            } => tessellate_primitive(*spec, placement, policy),
            Self::Extrude {
                profile,
                wall_sources,
                placement,
                height,
                caps,
                ..
            } => tessellate_extrude_with_wall_sources(
                profile,
                placement,
                *height,
                *caps,
                policy,
                Some(wall_sources),
            ),
        }
    }

    pub(crate) const fn stretch_nodes(&self) -> u32 {
        match self {
            Self::Primitive { stretch_nodes, .. } | Self::Extrude { stretch_nodes, .. } => {
                *stretch_nodes
            }
        }
    }

    fn under(mut self, outer: &Placement3) -> Self {
        let placement = match &mut self {
            Self::Primitive { placement, .. } | Self::Extrude { placement, .. } => placement,
        };
        *placement = compose(outer, placement);
        self
    }
}

/// Recognizes exact stretches without evaluating the child mesh first.
///
/// The plane is expressed in the stretch input frame. Primitive/extrusion
/// placements are proper coordinate maps into that frame, so the plane is
/// pulled back into the child's local coordinates before choosing an axis or
/// editing a profile. Ancestor `world` placement is composed only after the
/// rewrite; this preserves the IR's node-local composition rule even under an
/// affine ancestor.
pub(crate) fn exact_plan(
    recipe: &Recipe,
    child: NodeId,
    plane: &Plane3,
    length: f64,
    world: &Placement3,
) -> Result<Option<ExactStretchPlan>, StretchRefusal> {
    let Some(plan) = exact_child_plan(recipe, child) else {
        return Ok(None);
    };
    Ok(stretch_exact_plan(plan, plane, length)?.map(|plan| plan.under(world)))
}

/// Carries constructive structure through nodes that do not inherently
/// require tessellation. A nested refusal is left to the ordinary evaluator
/// so its diagnostic remains attached to the inner node; only successful
/// nested rewrites are folded into the outer plan.
fn exact_child_plan(recipe: &Recipe, node: NodeId) -> Option<ExactStretchPlan> {
    let node = recipe.node(node).expect("stretch child is validated");
    match &node.kind {
        NodeKind::Transform { child, xf } => {
            exact_child_plan(recipe, *child).map(|plan| plan.under(xf))
        }
        NodeKind::Stretch {
            child,
            plane,
            length,
        } => {
            let plan = exact_child_plan(recipe, *child)?;
            stretch_exact_plan(plan, plane, *length).ok().flatten()
        }
        NodeKind::Primitive {
            spec: spec @ PrimitiveSpec::Box { .. },
            placement,
        } => Some(ExactStretchPlan::Primitive {
            spec: *spec,
            placement: *placement,
            stretch_nodes: 0,
        }),
        NodeKind::Extrude {
            profile,
            placement,
            height,
            caps,
        } => {
            let profile = recipe.profile(*profile).expect("profile id is validated");
            Some(ExactStretchPlan::Extrude {
                wall_sources: initial_wall_sources(profile),
                profile: profile.clone(),
                placement: *placement,
                height: *height,
                caps: *caps,
                stretch_nodes: 0,
            })
        }
        _ => None,
    }
}

fn initial_wall_sources(profile: &Profile2) -> Vec<Vec<ExtrudeWallSource>> {
    let mut offset = 0_u32;
    core::iter::once(profile.outer())
        .chain(profile.holes().iter())
        .map(|loop_| {
            let sources = (0..loop_.segs().len())
                .map(|segment| {
                    let segment = u32::try_from(segment).expect("validated profile fits u32");
                    ExtrudeWallSource {
                        region: REGION_WALL_BASE + offset + segment,
                        segment,
                    }
                })
                .collect::<Vec<_>>();
            offset += u32::try_from(loop_.segs().len()).expect("validated profile fits u32");
            sources
        })
        .collect()
}

fn stretch_exact_plan(
    plan: ExactStretchPlan,
    plane: &Plane3,
    length: f64,
) -> Result<Option<ExactStretchPlan>, StretchRefusal> {
    match plan {
        ExactStretchPlan::Primitive {
            spec: PrimitiveSpec::Box { size },
            placement,
            stretch_nodes,
        } => {
            let Some((local_normal, local_distance)) = plane_in_rigid_local(plane, &placement)
            else {
                return Ok(None);
            };
            let Some((axis, sign)) = aligned_axis(local_normal) else {
                return Ok(None);
            };
            let interval = stretch_interval(size[axis], sign, local_distance, length)?;
            let mut stretched_size = size;
            stretched_size[axis] = interval.extent;
            let local_shift = axis_vector(axis, interval.origin_shift);
            let placement = compose(
                &placement,
                &Placement3::translate(local_shift[0], local_shift[1], local_shift[2]),
            );
            Ok(Some(ExactStretchPlan::Primitive {
                spec: PrimitiveSpec::Box {
                    size: stretched_size,
                },
                placement,
                stretch_nodes: stretch_nodes + 1,
            }))
        }
        ExactStretchPlan::Extrude {
            profile,
            wall_sources,
            placement,
            height,
            caps,
            stretch_nodes,
        } => {
            let Some((local_normal, local_distance)) = plane_in_rigid_local(plane, &placement)
            else {
                return Ok(None);
            };
            if let Some((axis, sign)) = aligned_axis(local_normal)
                && axis == 2
            {
                let interval = stretch_interval(height, sign, local_distance, length)?;
                let placement = compose(
                    &placement,
                    &Placement3::translate(0.0, 0.0, interval.origin_shift),
                );
                return Ok(Some(ExactStretchPlan::Extrude {
                    profile,
                    wall_sources,
                    placement,
                    height: interval.extent,
                    caps,
                    stretch_nodes: stretch_nodes + 1,
                }));
            }
            if local_normal[2].abs() > AXIS_EPSILON {
                return Ok(None);
            }
            let normal_2d = [local_normal[0], local_normal[1]];
            let Some(rewrite) =
                stretch_profile(&profile, &wall_sources, normal_2d, local_distance, length)?
            else {
                return Ok(None);
            };
            let (profile, wall_sources, local_shift) = match rewrite {
                ProfileRewrite::Profile {
                    profile,
                    wall_sources,
                } => (profile, wall_sources, [0.0, 0.0]),
                ProfileRewrite::Unchanged => (profile, wall_sources, [0.0, 0.0]),
                ProfileRewrite::Translated => (
                    profile,
                    wall_sources,
                    [length * normal_2d[0], length * normal_2d[1]],
                ),
            };
            let placement = compose(
                &placement,
                &Placement3::translate(local_shift[0], local_shift[1], 0.0),
            );
            Ok(Some(ExactStretchPlan::Extrude {
                profile,
                wall_sources,
                placement,
                height,
                caps,
                stretch_nodes: stretch_nodes + 1,
            }))
        }
        _ => Ok(None),
    }
}

const AXIS_EPSILON: f64 = 1.0e-10;

#[derive(Copy, Clone)]
struct IntervalRewrite {
    extent: f64,
    origin_shift: f64,
}

/// Rewrites `[0, extent]` under the one-dimensional form of stretch.
fn stretch_interval(
    extent: f64,
    sign: f64,
    distance: f64,
    length: f64,
) -> Result<IntervalRewrite, StretchRefusal> {
    let signed_at_zero = -distance;
    let signed_at_end = sign * extent - distance;
    let min_signed = signed_at_zero.min(signed_at_end);
    let max_signed = signed_at_zero.max(signed_at_end);

    // A plane coincident with an extent boundary has no unambiguous cut-side
    // ownership. Keep exact primitives aligned with the general mesh contract
    // instead of silently turning the same shape into a no-op or translation.
    if min_signed == 0.0
        || max_signed == 0.0
        || (length < 0.0 && (min_signed == -length || max_signed == -length))
    {
        return Err(StretchRefusal::AmbiguousContact);
    }

    if length > 0.0 {
        if max_signed <= 0.0 {
            return Ok(IntervalRewrite {
                extent,
                origin_shift: 0.0,
            });
        }
        if min_signed >= 0.0 {
            return Ok(IntervalRewrite {
                extent,
                origin_shift: sign * length,
            });
        }
        return Ok(IntervalRewrite {
            extent: extent + length,
            origin_shift: if sign < 0.0 { -length } else { 0.0 },
        });
    }

    let removed = -length;
    if max_signed <= 0.0 {
        return Ok(IntervalRewrite {
            extent,
            origin_shift: 0.0,
        });
    }
    if min_signed >= removed {
        return Ok(IntervalRewrite {
            extent,
            origin_shift: sign * length,
        });
    }
    if min_signed < 0.0 && max_signed > removed {
        let contracted = extent - removed;
        if contracted <= 0.0 {
            return Err(StretchRefusal::ContractionCollapsesExtent);
        }
        return Ok(IntervalRewrite {
            extent: contracted,
            origin_shift: if sign < 0.0 { removed } else { 0.0 },
        });
    }
    Err(StretchRefusal::ContractionConsumesHalf)
}

fn axis_vector(axis: usize, value: f64) -> [f64; 3] {
    let mut vector = [0.0; 3];
    vector[axis] = value;
    vector
}

fn aligned_axis(normal: [f64; 3]) -> Option<(usize, f64)> {
    let axis = normal
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))?
        .0;
    if normal[axis].abs() < 1.0 - AXIS_EPSILON
        || normal
            .iter()
            .enumerate()
            .any(|(index, value)| index != axis && value.abs() > AXIS_EPSILON)
    {
        return None;
    }
    Some((axis, normal[axis].signum()))
}

/// Pulls a parent-space normalized plane through a rigid child placement.
fn plane_in_rigid_local(plane: &Plane3, placement: &Placement3) -> Option<([f64; 3], f64)> {
    let (normal, distance) = plane.normalized()?;
    let linear = [
        [
            placement.rows[0][0],
            placement.rows[0][1],
            placement.rows[0][2],
        ],
        [
            placement.rows[1][0],
            placement.rows[1][1],
            placement.rows[1][2],
        ],
        [
            placement.rows[2][0],
            placement.rows[2][1],
            placement.rows[2][2],
        ],
    ];
    for column in 0..3 {
        let norm = linear[0][column] * linear[0][column]
            + linear[1][column] * linear[1][column]
            + linear[2][column] * linear[2][column];
        if (norm - 1.0).abs() > AXIS_EPSILON {
            return None;
        }
        for other in column + 1..3 {
            let dot = linear[0][column] * linear[0][other]
                + linear[1][column] * linear[1][other]
                + linear[2][column] * linear[2][other];
            if dot.abs() > AXIS_EPSILON {
                return None;
            }
        }
    }
    let local_normal = [
        linear[0][0] * normal[0] + linear[1][0] * normal[1] + linear[2][0] * normal[2],
        linear[0][1] * normal[0] + linear[1][1] * normal[1] + linear[2][1] * normal[2],
        linear[0][2] * normal[0] + linear[1][2] * normal[1] + linear[2][2] * normal[2],
    ];
    let translation = [
        placement.rows[0][3],
        placement.rows[1][3],
        placement.rows[2][3],
    ];
    let local_distance = distance
        - normal[0] * translation[0]
        - normal[1] * translation[1]
        - normal[2] * translation[2];
    Some((local_normal, local_distance))
}

enum ProfileRewrite {
    Profile {
        profile: Profile2,
        wall_sources: Vec<Vec<ExtrudeWallSource>>,
    },
    Unchanged,
    Translated,
}

fn stretch_profile(
    profile: &Profile2,
    wall_sources: &[Vec<ExtrudeWallSource>],
    normal: [f64; 2],
    distance: f64,
    length: f64,
) -> Result<Option<ProfileRewrite>, StretchRefusal> {
    if length > 0.0 {
        Ok(stretch_profile_expansion(
            profile,
            wall_sources,
            normal,
            distance,
            length,
        ))
    } else {
        stretch_profile_contraction(profile, wall_sources, normal, distance, -length)
    }
}

/// Inserts material into a profile without flattening its untouched curves.
///
/// Curves whose conservative control/circle bounds overlap the cut are left
/// to the general mesh path. A line crossing is split exactly; its two pieces
/// and the inserted connector retain the original `SegTag`.
fn stretch_profile_expansion(
    profile: &Profile2,
    wall_sources: &[Vec<ExtrudeWallSource>],
    normal: [f64; 2],
    distance: f64,
    length: f64,
) -> Option<ProfileRewrite> {
    let mut any_negative = false;
    let mut any_positive = false;
    for loop_ in core::iter::once(profile.outer()).chain(profile.holes().iter()) {
        for (start, segment) in loop_.iter_with_starts() {
            let (minimum, maximum) = curve_signed_bounds(start, segment, normal, distance)?;
            if minimum <= 0.0 && maximum >= 0.0 {
                let start_signed = signed_2d(start, normal, distance);
                let end_signed = signed_2d(segment.to, normal, distance);
                if start_signed.signum() == end_signed.signum()
                    && !matches!(segment.kind, SegKind::Line)
                {
                    return None;
                }
            }
            let signed = signed_2d(segment.to, normal, distance);
            if signed == 0.0 {
                return None;
            }
            any_negative |= signed < 0.0;
            any_positive |= signed > 0.0;
        }
    }
    if !any_positive {
        return Some(ProfileRewrite::Unchanged);
    }
    if !any_negative {
        return Some(ProfileRewrite::Translated);
    }

    let (outer, outer_sources) =
        stretch_loop_expansion(profile.outer(), &wall_sources[0], normal, distance, length)?;
    let rewritten_holes = profile
        .holes()
        .iter()
        .zip(&wall_sources[1..])
        .map(|(loop_, sources)| stretch_loop_expansion(loop_, sources, normal, distance, length))
        .collect::<Option<Vec<_>>>()?;
    let (holes, hole_sources): (Vec<_>, Vec<_>) = rewritten_holes.into_iter().unzip();
    Profile2::new(outer, holes).ok().map(|profile| {
        let mut wall_sources = Vec::with_capacity(1 + hole_sources.len());
        wall_sources.push(outer_sources);
        wall_sources.extend(hole_sources);
        ProfileRewrite::Profile {
            profile,
            wall_sources,
        }
    })
}

fn stretch_profile_contraction(
    profile: &Profile2,
    wall_sources: &[Vec<ExtrudeWallSource>],
    normal: [f64; 2],
    distance: f64,
    removed: f64,
) -> Result<Option<ProfileRewrite>, StretchRefusal> {
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for loop_ in core::iter::once(profile.outer()).chain(profile.holes().iter()) {
        for (start, segment) in loop_.iter_with_starts() {
            let Some((segment_min, segment_max)) =
                curve_signed_bounds(start, segment, normal, distance)
            else {
                return Ok(None);
            };
            minimum = minimum.min(segment_min);
            maximum = maximum.max(segment_max);
        }
    }
    if maximum <= 0.0 {
        return Ok(Some(ProfileRewrite::Unchanged));
    }
    if minimum >= removed {
        return Ok(Some(ProfileRewrite::Translated));
    }
    if minimum >= 0.0 || maximum <= removed {
        return Err(StretchRefusal::ContractionConsumesHalf);
    }

    let displacement = [-removed * normal[0], -removed * normal[1]];
    let Some((outer, outer_sources)) = contract_loop(
        profile.outer(),
        &wall_sources[0],
        normal,
        distance,
        removed,
        displacement,
    ) else {
        return Ok(None);
    };
    let mut holes = Vec::with_capacity(profile.holes().len());
    let mut hole_sources = Vec::with_capacity(profile.holes().len());
    for (hole, sources) in profile.holes().iter().zip(&wall_sources[1..]) {
        let Some((contracted, contracted_sources)) =
            contract_loop(hole, sources, normal, distance, removed, displacement)
        else {
            return Ok(None);
        };
        holes.push(contracted);
        hole_sources.push(contracted_sources);
    }
    Ok(Profile2::new(outer, holes).ok().map(|profile| {
        let mut wall_sources = Vec::with_capacity(1 + hole_sources.len());
        wall_sources.push(outer_sources);
        wall_sources.extend(hole_sources);
        ProfileRewrite::Profile {
            profile,
            wall_sources,
        }
    }))
}

#[derive(Clone)]
struct ProfilePiece {
    start: Point,
    segment: Seg2,
    source: ExtrudeWallSource,
}

fn contract_loop(
    loop_: &Loop2,
    wall_sources: &[ExtrudeWallSource],
    normal: [f64; 2],
    distance: f64,
    removed: f64,
    displacement: [f64; 2],
) -> Option<(Loop2, Vec<ExtrudeWallSource>)> {
    let mut pieces = Vec::<ProfilePiece>::new();
    for ((start, segment), source) in loop_.iter_with_starts().zip(wall_sources.iter().copied()) {
        let (minimum, maximum) = curve_signed_bounds(start, segment, normal, distance)?;
        if minimum > 0.0 && maximum < removed {
            // Removing a whole curved/detail segment changes section
            // topology; let the mesh path prove compatibility instead.
            if !matches!(segment.kind, SegKind::Line) {
                return None;
            }
        }
        if !matches!(realized_kind(&segment.kind), SegKind::Line) {
            let zone = if maximum <= 0.0 {
                -1
            } else if minimum >= removed {
                1
            } else {
                return None;
            };
            if zone < 0 {
                pieces.push(ProfilePiece {
                    start,
                    segment: segment.clone(),
                    source,
                });
            } else {
                pieces.push(ProfilePiece {
                    start: translate_point(start, displacement),
                    segment: transform_segment(segment, displacement, true),
                    source,
                });
            }
            continue;
        }

        let start_signed = signed_2d(start, normal, distance);
        let end_signed = signed_2d(segment.to, normal, distance);
        if start_signed == 0.0
            || end_signed == 0.0
            || start_signed == removed
            || end_signed == removed
        {
            return None;
        }
        let mut parameters = alloc::vec![0.0, 1.0];
        for boundary in [0.0, removed] {
            let denominator = end_signed - start_signed;
            if denominator != 0.0 {
                let parameter = (boundary - start_signed) / denominator;
                if parameter > 0.0 && parameter < 1.0 {
                    parameters.push(parameter);
                }
            }
        }
        parameters.sort_by(f64::total_cmp);
        parameters.dedup_by(|a, b| a.to_bits() == b.to_bits());
        for window in parameters.windows(2) {
            let from = lerp_point(start, segment.to, window[0]);
            let to = lerp_point(start, segment.to, window[1]);
            let middle = (window[0] + window[1]) * 0.5;
            let middle_signed = start_signed + middle * (end_signed - start_signed);
            if middle_signed > 0.0 && middle_signed < removed {
                continue;
            }
            let moved = middle_signed >= removed;
            pieces.push(ProfilePiece {
                start: if moved {
                    translate_point(from, displacement)
                } else {
                    from
                },
                segment: Seg2 {
                    to: if moved {
                        translate_point(to, displacement)
                    } else {
                        to
                    },
                    kind: SegKind::Line,
                    tag: segment.tag,
                },
                source,
            });
        }
    }
    if pieces.len() < 2 {
        return None;
    }
    for index in 0..pieces.len() {
        let previous = (index + pieces.len() - 1) % pieces.len();
        if pieces[previous].segment.to != pieces[index].start {
            return None;
        }
    }
    let sources = pieces.iter().map(|piece| piece.source).collect();
    Loop2::new(pieces.into_iter().map(|piece| piece.segment).collect())
        .ok()
        .map(|loop_| (loop_, sources))
}

fn lerp_point(a: Point, b: Point, parameter: f64) -> Point {
    Point::new(a.x + parameter * (b.x - a.x), a.y + parameter * (b.y - a.y))
}

fn stretch_loop_expansion(
    loop_: &Loop2,
    wall_sources: &[ExtrudeWallSource],
    normal: [f64; 2],
    distance: f64,
    length: f64,
) -> Option<(Loop2, Vec<ExtrudeWallSource>)> {
    let displacement = [normal[0] * length, normal[1] * length];
    let mut output = Vec::new();
    let mut output_sources = Vec::new();
    for ((start, segment), source) in loop_.iter_with_starts().zip(wall_sources.iter().copied()) {
        let start_signed = signed_2d(start, normal, distance);
        let end_signed = signed_2d(segment.to, normal, distance);
        if start_signed == 0.0 || end_signed == 0.0 {
            return None;
        }
        if start_signed.signum() == end_signed.signum() {
            if !curve_stays_on_side(start, segment, normal, distance, start_signed.signum()) {
                return None;
            }
            let moved = start_signed > 0.0;
            output.push(transform_segment(segment, displacement, moved));
            output_sources.push(source);
            continue;
        }
        if !matches!(segment.kind, SegKind::Line) {
            return None;
        }
        let denominator = start_signed - end_signed;
        let t = start_signed / denominator;
        let cut = Point::new(
            start.x + t * (segment.to.x - start.x),
            start.y + t * (segment.to.y - start.y),
        );
        let moved_cut = translate_point(cut, displacement);
        if start_signed < 0.0 {
            output.push(Seg2 {
                to: cut,
                kind: SegKind::Line,
                tag: segment.tag,
            });
            output_sources.push(source);
            output.push(Seg2 {
                to: moved_cut,
                kind: SegKind::Line,
                tag: segment.tag,
            });
            output_sources.push(source);
            output.push(Seg2 {
                to: translate_point(segment.to, displacement),
                kind: SegKind::Line,
                tag: segment.tag,
            });
            output_sources.push(source);
        } else {
            output.push(Seg2 {
                to: moved_cut,
                kind: SegKind::Line,
                tag: segment.tag,
            });
            output_sources.push(source);
            output.push(Seg2 {
                to: cut,
                kind: SegKind::Line,
                tag: segment.tag,
            });
            output_sources.push(source);
            output.push(Seg2 {
                to: segment.to,
                kind: SegKind::Line,
                tag: segment.tag,
            });
            output_sources.push(source);
        }
    }
    Loop2::new(output).ok().map(|loop_| (loop_, output_sources))
}

fn signed_2d(point: Point, normal: [f64; 2], distance: f64) -> f64 {
    normal[0] * point.x + normal[1] * point.y - distance
}

fn translate_point(point: Point, displacement: [f64; 2]) -> Point {
    Point::new(point.x + displacement[0], point.y + displacement[1])
}

fn transform_segment(segment: &Seg2, displacement: [f64; 2], moved: bool) -> Seg2 {
    if !moved {
        return segment.clone();
    }
    Seg2 {
        to: translate_point(segment.to, displacement),
        kind: translate_kind(&segment.kind, displacement),
        tag: segment.tag,
    }
}

fn translate_kind(kind: &SegKind, displacement: [f64; 2]) -> SegKind {
    match kind {
        SegKind::Line => SegKind::Line,
        SegKind::Arc { bulge } => SegKind::Arc { bulge: *bulge },
        SegKind::Cubic { c1, c2 } => SegKind::Cubic {
            c1: translate_point(*c1, displacement),
            c2: translate_point(*c2, displacement),
        },
        SegKind::PolicyTo { policy, realized } => SegKind::PolicyTo {
            policy: *policy,
            realized: alloc::boxed::Box::new(translate_kind(realized, displacement)),
        },
    }
}

fn curve_stays_on_side(
    start: Point,
    segment: &Seg2,
    normal: [f64; 2],
    distance: f64,
    side: f64,
) -> bool {
    match realized_kind(&segment.kind) {
        SegKind::Line => true,
        SegKind::Cubic { c1, c2 } => [*c1, *c2]
            .into_iter()
            .all(|point| signed_2d(point, normal, distance) * side > 0.0),
        SegKind::Arc { bulge } => {
            let dx = segment.to.x - start.x;
            let dy = segment.to.y - start.y;
            let chord = libm::sqrt(dx * dx + dy * dy);
            let center_scale = (1.0 - bulge * bulge) / (4.0 * bulge);
            let center = Point::new(
                (start.x + segment.to.x) * 0.5 - dy * center_scale,
                (start.y + segment.to.y) * 0.5 + dx * center_scale,
            );
            let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
            let center_signed = signed_2d(center, normal, distance);
            if side > 0.0 {
                center_signed - radius > 0.0
            } else {
                center_signed + radius < 0.0
            }
        }
        SegKind::PolicyTo { .. } => unreachable!("realized_kind unwraps policy segments"),
    }
}

fn curve_signed_bounds(
    start: Point,
    segment: &Seg2,
    normal: [f64; 2],
    distance: f64,
) -> Option<(f64, f64)> {
    let start_signed = signed_2d(start, normal, distance);
    let end_signed = signed_2d(segment.to, normal, distance);
    let bounds = match realized_kind(&segment.kind) {
        SegKind::Line => (start_signed.min(end_signed), start_signed.max(end_signed)),
        SegKind::Cubic { c1, c2 } => {
            let values = [
                start_signed,
                signed_2d(*c1, normal, distance),
                signed_2d(*c2, normal, distance),
                end_signed,
            ];
            values.into_iter().fold(
                (f64::INFINITY, f64::NEG_INFINITY),
                |(minimum, maximum), value| (minimum.min(value), maximum.max(value)),
            )
        }
        SegKind::Arc { bulge } => {
            let dx = segment.to.x - start.x;
            let dy = segment.to.y - start.y;
            let chord = libm::sqrt(dx * dx + dy * dy);
            let center_scale = (1.0 - bulge * bulge) / (4.0 * bulge);
            let center = Point::new(
                (start.x + segment.to.x) * 0.5 - dy * center_scale,
                (start.y + segment.to.y) * 0.5 + dx * center_scale,
            );
            let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
            let center_signed = signed_2d(center, normal, distance);
            (center_signed - radius, center_signed + radius)
        }
        SegKind::PolicyTo { .. } => return None,
    };
    (bounds.0.is_finite() && bounds.1.is_finite()).then_some(bounds)
}

fn realized_kind(mut kind: &SegKind) -> &SegKind {
    while let SegKind::PolicyTo { realized, .. } = kind {
        kind = realized;
    }
    kind
}

fn compose(outer: &Placement3, inner: &Placement3) -> Placement3 {
    let a = &outer.rows;
    let b = &inner.rows;
    let mut rows = [[0.0; 4]; 3];
    for (i, row) in rows.iter_mut().enumerate() {
        for j in 0..3 {
            row[j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
        row[3] = a[i][0] * b[0][3] + a[i][1] * b[1][3] + a[i][2] * b[2][3] + a[i][3];
    }
    Placement3 { rows }
}
