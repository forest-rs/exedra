// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native construction of constructive recipes.

use super::NativeSource;
use alloc::{collections::BTreeSet, vec::Vec};
use exedra_analytic::{
    End,
    body::{Body, EdgeGeometry, FaceId, ValidationPolicy},
    extrusion::{Extrusion, ExtrusionError, FaceRole, Pocket},
    geometry::{Curve, GeometryError, Surface},
    loft::{Loft, LoftError, LoftPolicy},
    machining::{
        CylinderTool, MachiningError, MachiningPolicy, subtract_through_cylinder_with_roles,
    },
    rectilinear::{
        BooleanOperation, Operand, RectilinearError, RectilinearPolicy, boolean_rectilinear,
    },
    revolve::{Revolution, RevolutionError},
    sweep::{Sweep, SweepError},
};
use exedra_constructive::{
    ir::{
        CapMode, CsgOp, ExtrusionEnd, LoftPolicy as Interpolation, NodeId, NodeKind, Placement3,
        ProfileId, Recipe,
    },
    profile::ToProfileError,
    section_law::SectionLaw,
    workplane::{SurfaceSelector, WorkplaneAttachment, WorkplaneError, WorkplanePolicy},
};
use exedra_curves::profile::{Profile, ProfilePolicy, SegTag};
use exedra_math::{add, cross, dot, norm, normalize, scale, sub};

/// What lowering holds a recipe's native constructions to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeLoweringPolicy {
    /// Validation of every profile.
    pub profile: ProfilePolicy,
    /// How nearly an attachment's projection may lie in its face's plane,
    /// and its X along the face's normal, as mesh evaluation's policy says.
    pub workplane: WorkplanePolicy,
    /// Limits of exact Booleans of axis-aligned bodies.
    pub boolean: RectilinearPolicy,
    /// Clearances of exact through cuts.
    pub machining: MachiningPolicy,
    /// How deeply Boolean operands and attachment supports may nest, so
    /// that lowering a hostile recipe ends in a refusal, not an exhausted
    /// stack.
    pub max_depth: u32,
}

impl NativeLoweringPolicy {
    /// Profiles validated under `profile`, the default workplane, Boolean
    /// and machining policies, and nesting 64 deep.
    #[must_use]
    pub fn new(profile: ProfilePolicy) -> Self {
        Self {
            profile,
            workplane: WorkplanePolicy::default(),
            boolean: RectilinearPolicy::default(),
            machining: MachiningPolicy::default(),
            max_depth: 64,
        }
    }
}

/// Why a recipe has no native construction.
///
/// Lowering never drops what a recipe asks for: a node it cannot build
/// exactly, or a request native construction would not honor, is refused by
/// name rather than approximated or left out.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativeLoweringError {
    /// The node's operation has no native construction.
    Unsupported {
        /// The node.
        node: NodeId,
        /// The operation, as its node kind is named.
        operation: &'static str,
    },
    /// The node asks for something its native construction would not keep.
    Unkept {
        /// The node.
        node: NodeId,
        /// What would be lost: `"material"`, `"surface_chart"`, `"issue"`,
        /// `"open_caps"`, a sweep's varying `"section_law"`, a loft's
        /// `"smooth_interpolation"` or an attachment's `"region_selection"`.
        request: &'static str,
    },
    /// The node's profile has no shared form under the policy.
    Profile {
        /// The node.
        node: NodeId,
        /// Why.
        error: ToProfileError,
    },
    /// The extrusion refused its parameters.
    Extrusion {
        /// The node.
        node: NodeId,
        /// Why.
        error: ExtrusionError,
    },
    /// The revolution refused its parameters.
    Revolution {
        /// The node.
        node: NodeId,
        /// Why.
        error: RevolutionError,
    },
    /// The sweep refused its parameters.
    Sweep {
        /// The node.
        node: NodeId,
        /// Why.
        error: SweepError,
    },
    /// The loft refused its sections, including a section's profile.
    Loft {
        /// The node.
        node: NodeId,
        /// Why.
        error: LoftError<ToProfileError>,
    },
    /// A Boolean's operand could not be combined natively.
    Boolean {
        /// The Boolean node.
        node: NodeId,
        /// The operand that could not be combined with those before it.
        operand: usize,
        /// Why.
        error: NativeBooleanError,
    },
    /// Boolean operands or attachment supports nest more deeply than the
    /// policy allows.
    Depth {
        /// The node one level too deep.
        node: NodeId,
        /// The policy's limit.
        maximum: u32,
    },
    /// The attachment's surface could not be resolved on its native
    /// support, for the reason mesh evaluation would give.
    Attachment {
        /// The attaching node.
        node: NodeId,
        /// Why.
        error: WorkplaneError,
    },
}

impl core::fmt::Display for NativeLoweringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported { node, operation } => {
                write!(f, "node {}: {operation} has no native construction", node.0)
            }
            Self::Unkept { node, request } => write!(
                f,
                "node {}: native construction would not keep its {request}",
                node.0
            ),
            Self::Profile { node, error } => write!(f, "node {}: {error}", node.0),
            Self::Extrusion { node, error } => write!(f, "node {}: {error}", node.0),
            Self::Revolution { node, error } => write!(f, "node {}: {error}", node.0),
            Self::Sweep { node, error } => write!(f, "node {}: {error}", node.0),
            Self::Loft { node, error } => write!(f, "node {}: {error}", node.0),
            Self::Attachment { node, error } => write!(f, "node {}: {error}", node.0),
            Self::Depth { node, maximum } => {
                write!(f, "node {}: nested more than {maximum} deep", node.0)
            }
            Self::Boolean {
                node,
                operand,
                error,
            } => write!(f, "node {}: operand {operand}: {error}", node.0),
        }
    }
}

impl core::error::Error for NativeLoweringError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Unsupported { .. } | Self::Unkept { .. } | Self::Depth { .. } => None,
            Self::Profile { error, .. } => Some(error),
            Self::Extrusion { error, .. } => Some(error),
            Self::Revolution { error, .. } => Some(error),
            Self::Sweep { error, .. } => Some(error),
            Self::Loft { error, .. } => Some(error),
            Self::Attachment { error, .. } => Some(error),
            Self::Boolean { error, .. } => Some(error),
        }
    }
}

/// Why a Boolean has no exact native result.
///
/// Natively, a Boolean is exact on axis-aligned bodies, and a difference
/// also where the cutter is a circular extrusion passing through the base.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativeBooleanError {
    /// The operand is oriented differently from those before it, which an
    /// exact axis-aligned arrangement cannot combine.
    Orientation,
    /// The axis-aligned arrangement refused the operands.
    Rectilinear(RectilinearError),
    /// The through cut refused the cutter.
    Machining(MachiningError),
    /// Moving the operand into the others' frame refused its geometry.
    Geometry(GeometryError),
    /// The exact result has no material.
    Empty,
}

impl core::fmt::Display for NativeBooleanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Orientation => write!(f, "operands are oriented differently"),
            Self::Rectilinear(error) => error.fmt(f),
            Self::Machining(error) => error.fmt(f),
            Self::Geometry(error) => error.fmt(f),
            Self::Empty => write!(f, "the exact result is empty"),
        }
    }
}

impl core::error::Error for NativeBooleanError {}

impl NativeSource {
    /// The recipe's root built natively, so that its measures, selections
    /// and exchange are exact: an extrusion of a profile with any pockets
    /// cut into it, a revolution, a constant-section sweep or a ruled loft,
    /// under any rigid transforms, placed on a workplane attached to another
    /// such solid's cap or pocket floor, which is not emitted, and combined
    /// by Booleans that are exact on axis-aligned bodies or cut circular
    /// extrusions through them. Profiles are validated under `policy`;
    /// segment tags become the solid's face tags.
    ///
    /// Lowering is the caller's explicit choice. A caller that requires
    /// exact geometry propagates the refusal; one that prefers it falls back
    /// to [`crate::PartSource::Recipe`], whose mesh evaluation covers every
    /// node. The node that produced the body, as mesh evaluation attributes
    /// it, lends it its source label, by which a snapshot resolves it.
    ///
    /// ```
    /// use exedra_compile::{NativeLoweringPolicy, NativeSource};
    /// use exedra_constructive::{builders, ir::{CapMode, NodeKind, Placement3, RecipeBuilder}};
    /// use exedra_curves::profile::ProfilePolicy;
    /// let mut builder = RecipeBuilder::new();
    /// let disk = builder.add_profile(builders::circle(5.0)?);
    /// let root = builder.add_node(NodeKind::Extrude {
    ///     profile: disk,
    ///     placement: Placement3::IDENTITY,
    ///     height: 3.0,
    ///     caps: CapMode::Both,
    /// })?;
    /// let policy = NativeLoweringPolicy::new(ProfilePolicy::new(1e-9));
    /// let native = NativeSource::from_recipe(&builder.finish(root)?, &policy)?;
    /// assert!(native.as_extrusion().is_some());
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first node, from the root down, that has no native
    /// construction, asks for something it would not keep, or whose
    /// construction refuses its parameters.
    pub fn from_recipe(
        recipe: &Recipe,
        policy: &NativeLoweringPolicy,
    ) -> Result<Self, NativeLoweringError> {
        let lowered = lower(recipe, recipe.root(), policy, 0)?;
        Ok(match recipe.source_of(lowered.producer) {
            Some(label) => lowered.source.labelled(label),
            None => lowered.source,
        })
    }
}

/// A surface an attachment can name by its node's label.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Named {
    StartCap,
    EndCap,
    Floor,
}

/// A natively built subtree, with the node that produced its body, as
/// mesh evaluation attributes the body, and the faces its labelled nodes
/// made.
struct Lowered<'r> {
    source: NativeSource,
    producer: NodeId,
    named: Vec<(&'r str, Named, FaceId)>,
}

fn lower<'r>(
    recipe: &'r Recipe,
    mut id: NodeId,
    policy: &NativeLoweringPolicy,
    depth: u32,
) -> Result<Lowered<'r>, NativeLoweringError> {
    if depth > policy.max_depth {
        return Err(NativeLoweringError::Depth {
            node: id,
            maximum: policy.max_depth,
        });
    }
    let mut outer = Placement3::IDENTITY;
    // Pockets met on the way down, outermost first.
    let mut pockets = Vec::new();
    loop {
        let node = recipe.node(id).expect("a recipe's nodes resolve");
        let unkept = |request| Err(NativeLoweringError::Unkept { node: id, request });
        if node.material.is_some() {
            return unkept("material");
        }
        if node.surface_chart.is_some() {
            return unkept("surface_chart");
        }
        if node.issue.is_some() {
            return unkept("issue");
        }
        let profile = |profile: ProfileId| -> Result<Profile, NativeLoweringError> {
            recipe
                .profile(profile)
                .expect("a recipe's profiles resolve")
                .to_profile(&policy.profile)
                .map_err(|error| NativeLoweringError::Profile { node: id, error })
        };
        // A leaf's caps, under its label.
        let leaf = |source: NativeSource| {
            let mut named = Vec::new();
            if let Some(label) = recipe.source_of(id) {
                for (end, which) in [(End::Start, Named::StartCap), (End::End, Named::EndCap)] {
                    if let Some(face) = source.cap(end) {
                        named.push((label, which, face));
                    }
                }
            }
            Lowered {
                source,
                producer: id,
                named,
            }
        };
        match &node.kind {
            NodeKind::Transform { child, xf } => {
                outer = outer * *xf;
                id = *child;
            }
            NodeKind::OnWorkplane {
                support,
                child,
                attachment,
            } => {
                if matches!(
                    attachment.surface,
                    SurfaceSelector::Region(_) | SurfaceSelector::OperandRegion(_)
                ) {
                    // Regions are mesh attributes, which native faces lack.
                    return unkept("region_selection");
                }
                // The support stands in this node's coordinates, unemitted.
                let support = lower(recipe, *support, policy, depth + 1)?;
                outer = outer
                    * attach(recipe, &support, attachment, &policy.workplane)
                        .map_err(|error| NativeLoweringError::Attachment { node: id, error })?;
                id = *child;
            }
            NodeKind::Pocket {
                base,
                profile: p,
                end,
                depth,
            } => {
                let end = match end {
                    ExtrusionEnd::Start => End::Start,
                    ExtrusionEnd::End => End::End,
                };
                pockets.push((id, profile(*p)?, end, *depth));
                id = *base;
            }
            NodeKind::Extrude {
                profile: p,
                placement,
                height,
                caps,
            } => {
                if *caps != CapMode::Both {
                    return unkept("open_caps");
                }
                let mut solid = Extrusion::new(profile(*p)?, outer * *placement, *height)
                    .map_err(|error| NativeLoweringError::Extrusion { node: id, error })?;
                let mut floors = Vec::new();
                let outermost = pockets.first().map(|pocket| pocket.0);
                for (index, (node, region, end, depth)) in pockets.into_iter().rev().enumerate() {
                    let refused = |error| NativeLoweringError::Extrusion { node, error };
                    let pocket = Pocket::from_profile(&region, end, depth).map_err(|error| {
                        refused(ExtrusionError::Pocket {
                            pocket: index,
                            error,
                        })
                    })?;
                    solid = solid
                        .with_pocket(pocket, &policy.profile)
                        .map_err(refused)?;
                    if let Some(label) = recipe.source_of(node) {
                        floors.push((label, index));
                    }
                }
                let floor = |pocket| {
                    let at = solid
                        .body()
                        .faces()
                        .iter()
                        .position(|face| face.role == FaceRole::Floor { pocket })
                        .expect("every pocket has a floor");
                    FaceId(u32::try_from(at).expect("native face ids fit u32"))
                };
                let floors: Vec<_> = floors
                    .into_iter()
                    .map(|(label, pocket)| (label, Named::Floor, floor(pocket)))
                    .collect();
                let mut lowered = leaf(NativeSource::from(solid));
                lowered.named.extend(floors);
                // The outermost pocket produced the body.
                lowered.producer = outermost.unwrap_or(id);
                return Ok(lowered);
            }
            NodeKind::Revolve {
                profile: p,
                placement,
                sweep,
                caps,
            } => {
                if *caps != CapMode::Both {
                    return unkept("open_caps");
                }
                return Revolution::new(profile(*p)?, outer * *placement, *sweep)
                    .map(|solid| leaf(NativeSource::from(solid)))
                    .map_err(|error| NativeLoweringError::Revolution { node: id, error });
            }
            NodeKind::Sweep {
                profile: p,
                path,
                section,
                caps,
            } => {
                if *caps != CapMode::Both {
                    return unkept("open_caps");
                }
                if *section != SectionLaw::IDENTITY {
                    return unkept("section_law");
                }
                return Sweep::new(profile(*p)?, path, outer)
                    .map(|solid| leaf(NativeSource::from(solid)))
                    .map_err(|error| NativeLoweringError::Sweep { node: id, error });
            }
            NodeKind::Loft {
                sections,
                policy: interpolation,
                caps,
            } => {
                if *caps != CapMode::Both {
                    return unkept("open_caps");
                }
                if *interpolation != Interpolation::Ruled {
                    return unkept("smooth_interpolation");
                }
                let sections = sections.iter().map(|section| {
                    let profile = recipe
                        .profile(section.profile)
                        .expect("a recipe's profiles resolve");
                    (section.placement, profile)
                });
                return Loft::from_profiles(sections, outer, &LoftPolicy::new(policy.profile))
                    .map(|solid| leaf(NativeSource::from(solid)))
                    .map_err(|error| NativeLoweringError::Loft { node: id, error });
            }
            NodeKind::Csg { op, operands } => {
                let refused = |operand, error| NativeLoweringError::Boolean {
                    node: id,
                    operand,
                    error,
                };
                let mut combined =
                    Combined::from(&lower(recipe, operands[0], policy, depth + 1)?.source);
                for (index, &operand) in operands.iter().enumerate().skip(1) {
                    let other = lower(recipe, operand, policy, depth + 1)?.source;
                    combined = combined
                        .with(*op, &other, policy)
                        .map_err(|error| refused(index, error))?;
                }
                let source = NativeSource::from_measured_body(
                    combined.body,
                    outer * combined.frame,
                    combined.integrals,
                )
                .map_err(|_| refused(0, NativeBooleanError::Geometry(GeometryError::NonFinite)))?;
                return Ok(Lowered {
                    source,
                    producer: id,
                    named: Vec::new(),
                });
            }
            kind => {
                return Err(NativeLoweringError::Unsupported {
                    node: id,
                    operation: operation(kind),
                });
            }
        }
    }
}

/// The frame `attachment` resolves to on its native support: on the
/// selected face's plane, where the authored line meets it, with +Z the
/// face's outward normal and X the authored direction projected into the
/// plane, as mesh evaluation resolves it.
fn attach(
    recipe: &Recipe,
    support: &Lowered<'_>,
    attachment: &WorkplaneAttachment,
    policy: &WorkplanePolicy,
) -> Result<Placement3, WorkplaneError> {
    if !policy.is_valid() {
        return Err(WorkplaneError::InvalidInput);
    }
    let named = |label: &str, wanted| {
        if recipe.source_is_ambiguous(label) {
            return Err(WorkplaneError::AmbiguousSelection);
        }
        Ok(support
            .named
            .iter()
            .find(|(l, n, _)| *l == label && *n == wanted)
            .map(|&(.., face)| face))
    };
    let face = match &attachment.surface {
        SurfaceSelector::StartCap => support.source.cap(End::Start),
        SurfaceSelector::EndCap => support.source.cap(End::End),
        SurfaceSelector::SourceStartCap(label) => named(label, Named::StartCap)?,
        SurfaceSelector::SourceEndCap(label) => named(label, Named::EndCap)?,
        SurfaceSelector::SourceFloor(label) => named(label, Named::Floor)?,
        SurfaceSelector::Region(_) | SurfaceSelector::OperandRegion(_) => None,
    }
    .ok_or(WorkplaneError::EmptySelection)?;
    let frame = support
        .source
        .planar_frame(face)
        .map_err(|_| WorkplaneError::NonPlanar)?;
    let column = |k: usize| core::array::from_fn(|i| frame.rows[i][k]);
    let (normal, point) = (column(2), column(3));
    let projection = normalize(attachment.projection).ok_or(WorkplaneError::InvalidInput)?;
    let x = normalize(attachment.x_direction).ok_or(WorkplaneError::InvalidInput)?;
    let across = dot(normal, projection);
    if across.abs() < policy.min_projection_cos {
        return Err(WorkplaneError::ParallelProjection);
    }
    let origin = add(
        attachment.anchor,
        scale(
            projection,
            dot(normal, sub(point, attachment.anchor)) / across,
        ),
    );
    let x = sub(x, scale(normal, dot(x, normal)));
    if norm(x) < policy.min_axis_sine {
        return Err(WorkplaneError::ParallelAxis);
    }
    let x = normalize(x).ok_or(WorkplaneError::ParallelAxis)?;
    Ok(Placement3::from_axes(x, cross(normal, x), normal, origin))
}

/// Operands combined so far: a body in its own frame, and its exact
/// volume and area where every step carried them.
struct Combined {
    body: Body<Vec<SegTag>>,
    frame: Placement3,
    integrals: Option<[f64; 2]>,
}

impl Combined {
    fn from(source: &NativeSource) -> Self {
        Self {
            body: source.to_body(),
            frame: source.frame(),
            integrals: source
                .volume()
                .ok()
                .zip(source.surface_area().ok())
                .map(Into::into),
        }
    }

    /// These operands combined with `other` by `op`: cut through by it
    /// where a difference's cutter is a circular extrusion passing
    /// through, otherwise arranged exactly with it.
    fn with(
        self,
        op: CsgOp,
        other: &NativeSource,
        policy: &NativeLoweringPolicy,
    ) -> Result<Self, NativeBooleanError> {
        if op == CsgOp::Difference
            && let Some((tool, tags)) = self.through_cylinder(other)?
        {
            let result = subtract_through_cylinder_with_roles(
                &self.body,
                &tool,
                &policy.machining,
                tags,
                (),
            )
            .map_err(NativeBooleanError::Machining)?;
            let integrals = self.integrals.map(|[volume, area]| {
                [
                    volume - result.report.removed_volume,
                    area + result.report.surface_area_delta,
                ]
            });
            return Ok(Self {
                body: result.body,
                frame: self.frame,
                integrals,
            });
        }
        let right = into_frame(
            other.to_body(),
            other.frame(),
            self.frame,
            policy.boolean.geometry_gap,
        )?;
        let operation = match op {
            CsgOp::Union => BooleanOperation::Union,
            CsgOp::Intersection => BooleanOperation::Intersection,
            CsgOp::Difference => BooleanOperation::Difference,
        };
        let result = boolean_rectilinear(&self.body, &right, operation, &policy.boolean)
            .map_err(NativeBooleanError::Rectilinear)?;
        if result.report.material_cells == 0 {
            return Err(NativeBooleanError::Empty);
        }
        let mut sources = result.report.face_sources.iter();
        let body = result.body.map_roles(
            |()| {
                let mut tags = BTreeSet::new();
                for source in sources.next().expect("one correspondence per output face") {
                    let body = match source.operand {
                        Operand::Left => &self.body,
                        Operand::Right => &right,
                    };
                    tags.extend(body.faces()[source.face.index()].role.iter().copied());
                }
                tags.into_iter().collect()
            },
            |()| (),
        );
        Ok(Self {
            body,
            frame: self.frame,
            integrals: Some([result.report.volume, result.report.surface_area]),
        })
    }

    /// The cylinder `cutter` sweeps, in these operands' coordinates, and its
    /// wall's tags, when it is an extrusion of arcs of exactly one circle,
    /// without holes or pockets, whose ends lie beyond these operands'
    /// conservative bounds,
    /// so that it cuts all the way through them as the infinite cylinder
    /// does.
    fn through_cylinder(
        &self,
        cutter: &NativeSource,
    ) -> Result<Option<(CylinderTool, Vec<SegTag>)>, NativeBooleanError> {
        let Some(extrusion) = cutter.as_extrusion() else {
            return Ok(None);
        };
        let profile = extrusion.profile();
        if !profile.holes().is_empty() || !extrusion.pockets().is_empty() {
            return Ok(None);
        }
        let outline = profile.outer();
        let Some(first) = outline.arc(0) else {
            return Ok(None);
        };
        // One circle exactly: every arc resolves to the support its native
        // wall would have. Any arc on another, however near, is not this
        // cylinder.
        let circular = (0..outline.segments().len()).all(|k| {
            outline
                .arc(k)
                .is_some_and(|arc| arc.center == first.center && arc.radius == first.radius)
        });
        if !circular {
            return Ok(None);
        }
        // The axis, from the cutter's frame into these operands'.
        let place = |frame: Placement3, p: [f64; 3], w: f64| -> [f64; 3] {
            core::array::from_fn(|i| {
                (0..3).map(|j| frame.rows[i][j] * p[j]).sum::<f64>() + w * frame.rows[i][3]
            })
        };
        let unplace = |p: [f64; 3], w: f64| -> [f64; 3] {
            let d: [f64; 3] = core::array::from_fn(|i| p[i] - w * self.frame.rows[i][3]);
            core::array::from_fn(|i| (0..3).map(|j| self.frame.rows[j][i] * d[j]).sum())
        };
        let frame = extrusion.frame();
        let point = unplace(
            place(frame, [first.center[0], first.center[1], 0.0], 1.0),
            1.0,
        );
        let direction = unplace(place(frame, [0.0, 0.0, 1.0], 0.0), 0.0);
        let [lo, hi] = self
            .body
            .conservative_bounds_in(Placement3::IDENTITY)
            .map_err(|_| NativeBooleanError::Geometry(GeometryError::NonFinite))?;
        let through = (0..8).all(|corner| {
            let p = core::array::from_fn(|i| if corner >> i & 1 == 0 { lo[i] } else { hi[i] });
            let along = dot(sub(p, point), direction);
            0.0 < along && along < extrusion.length()
        });
        if !through {
            return Ok(None);
        }
        let tool = CylinderTool::new(point, direction, first.radius)
            .map_err(NativeBooleanError::Geometry)?;
        let tags = outline
            .segments()
            .iter()
            .filter_map(|segment| segment.tag)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(Some((tool, tags)))
    }
}

/// `body`, placed by `from`, in the coordinates of `to`, which must be
/// equally oriented: its planes and lines translated, which is exact for
/// the axis-aligned bodies an exact arrangement takes, and revalidated to
/// `gap`.
fn into_frame(
    body: Body<Vec<SegTag>>,
    from: Placement3,
    to: Placement3,
    gap: f64,
) -> Result<Body<Vec<SegTag>>, NativeBooleanError> {
    if from == to {
        return Ok(body);
    }
    if (0..3).any(|row| (0..3).any(|column| from.rows[row][column] != to.rows[row][column])) {
        return Err(NativeBooleanError::Orientation);
    }
    // Rigid frames of one orientation differ by R^T (t_from - t_to).
    let delta: [f64; 3] = core::array::from_fn(|i| from.rows[i][3] - to.rows[i][3]);
    let offset: [f64; 3] = core::array::from_fn(|i| (0..3).map(|j| to.rows[j][i] * delta[j]).sum());
    let translate = |p: [f64; 3]| add(p, offset);
    let mut parts = body.into_parts();
    for point in &mut parts.vertices {
        *point = translate(*point);
    }
    for edge in &mut parts.edges {
        if let EdgeGeometry::Curve {
            curve: Curve::Line(line),
            range,
        } = &edge.geometry
        {
            let range = *range;
            edge.geometry = EdgeGeometry::Curve {
                curve: Curve::line(translate(line.origin()), line.direction())
                    .map_err(NativeBooleanError::Geometry)?,
                range,
            };
        }
    }
    for (id, face) in parts.faces.iter_mut().enumerate() {
        let lines = face.loops.iter().all(|id| {
            parts.loops[id.index()].coedges.iter().all(|coedge| {
                matches!(
                    parts.edges[coedge.edge.index()].regular(),
                    Some((Curve::Line(_), _))
                )
            })
        });
        if !matches!(face.surface, Surface::Plane(_)) || !lines {
            return Err(NativeBooleanError::Rectilinear(
                RectilinearError::UnsupportedFace {
                    operand: Operand::Right,
                    face: FaceId(u32::try_from(id).expect("native face ids fit u32")),
                },
            ));
        }
        let mut frame = face.surface.frame();
        let origin = translate(frame.rows.map(|row| row[3]));
        for (row, value) in frame.rows.iter_mut().zip(origin) {
            row[3] = value;
        }
        face.surface = Surface::plane(frame).map_err(NativeBooleanError::Geometry)?;
    }
    Body::try_from_parts(parts, &ValidationPolicy { max_gap: gap })
        .map_err(|_| NativeBooleanError::Geometry(GeometryError::NonFinite))
}

/// The node kind's name, as recipe interchange spells it.
fn operation(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::UvMap { .. } => "uv_map",
        NodeKind::OnWorkplane { .. } => "on_workplane",
        NodeKind::PlaneCut { .. } => "plane_cut",
        NodeKind::ExtrudeToPlane { .. } => "extrude_to_plane",
        NodeKind::EdgeFinish { .. } => "edge_finish",
        NodeKind::Extrude { .. } => "extrude",
        NodeKind::Pocket { .. } => "pocket",
        NodeKind::Revolve { .. } => "revolve",
        NodeKind::Loft { .. } => "loft",
        NodeKind::Sweep { .. } => "sweep",
        NodeKind::PlanarFace { .. } => "planar_face",
        NodeKind::Primitive { .. } => "primitive",
        NodeKind::Csg { .. } => "csg",
        NodeKind::Transform { .. } => "transform",
        NodeKind::Mirror { .. } => "mirror",
        NodeKind::Instance { .. } => "instance",
        NodeKind::Group { .. } => "group",
        NodeKind::MeshImport { .. } => "mesh_import",
        NodeKind::Stretch { .. } => "stretch",
        NodeKind::StretchVertices { .. } => "stretch_vertices",
        NodeKind::GridSurface { .. } => "grid_surface",
        NodeKind::Implicit { .. } => "implicit",
        _ => "operation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompilePolicy, PartCompiler, PartInput, PartSource};
    use exedra_constructive::{
        builders,
        ir::{CsgOp, Node, RecipeBuilder},
        profile::{Loop2, Profile2, Seg2, SegKind},
    };
    use exedra_curves::profile::SegTag;

    const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);

    fn lowering() -> NativeLoweringPolicy {
        NativeLoweringPolicy::new(POLICY)
    }

    fn plate() -> Profile2 {
        builders::rect_centered(40.0, 30.0)
            .unwrap()
            .with_cutouts([builders::circle(3.0).unwrap().tagged(SegTag(7))])
            .unwrap()
    }

    fn extrude(profile: ProfileId) -> NodeKind {
        NodeKind::Extrude {
            profile,
            placement: Placement3::translate(1.0, 2.0, 3.0),
            height: 12.0,
            caps: CapMode::Both,
        }
    }

    #[test]
    fn extrusions_lower_to_the_solid_built_directly() {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(plate());
        let kind = extrude(profile);
        let leaf = builder.add_node(kind).unwrap();
        let root = builder
            .add_node(NodeKind::Transform {
                child: leaf,
                xf: Placement3::translate(10.0, 0.0, 0.0),
            })
            .unwrap();
        let recipe = builder.finish(root).unwrap();
        let lowered = NativeSource::from_recipe(&recipe, &lowering()).unwrap();
        let direct = NativeSource::from(
            Extrusion::new(
                plate().to_profile(&POLICY).unwrap(),
                Placement3::translate(11.0, 2.0, 3.0),
                12.0,
            )
            .unwrap(),
        );
        assert_eq!(lowered.fingerprint(), direct.fingerprint());
        let exact = (1200.0 - core::f64::consts::PI * 9.0) * 12.0;
        assert!((lowered.volume().unwrap() - exact).abs() < 1e-9);
        let solid = lowered.as_extrusion().unwrap();
        assert_eq!(
            solid.faces_tagged(SegTag(7)).count(),
            2,
            "the bore's two arcs"
        );

        // Mesh evaluation of the same recipe encloses the same volume, to
        // its chords.
        let source = PartSource::Recipe(recipe);
        let parts = PartCompiler::new()
            .compile_parts(
                &[PartInput::new("plate", &source)],
                &CompilePolicy::default(),
            )
            .unwrap();
        let meshed = parts.part(crate::PartId(0)).unwrap().bodies[0]
            .signed_volume()
            .unwrap()
            .value;
        assert!((meshed - exact).abs() < 1e-3 * exact, "{meshed} {exact}");
    }

    #[test]
    fn lowered_bodies_keep_their_producing_label() {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(plate());
        let label = builder.source_ref("plate");
        let leaf = builder
            .add_node(Node::new(extrude(profile)).with_source(label))
            .unwrap();
        let root = builder
            .add_node(NodeKind::Transform {
                child: leaf,
                xf: Placement3::translate(1.0, 0.0, 0.0),
            })
            .unwrap();
        let recipe = builder.finish(root).unwrap();
        let native = NativeSource::from_recipe(&recipe, &lowering()).unwrap();
        assert_eq!(native.label(), Some("plate"));
        let unlabelled = NativeSource::from(native.as_extrusion().unwrap().clone());
        assert_ne!(native.fingerprint(), unlabelled.fingerprint());
        for source in [PartSource::Recipe(recipe), PartSource::Native(native)] {
            let snapshot = PartCompiler::new()
                .compile_snapshot(
                    &[PartInput::new("plate", &source)],
                    &CompilePolicy::default(),
                )
                .unwrap();
            assert!(snapshot.body_by_source(crate::PartId(0), "plate").is_ok());
        }
    }

    #[test]
    fn revolutions_lower_with_their_placement() {
        let mut builder = RecipeBuilder::new();
        let ring = builder.add_profile(
            builders::rect_centered(2.0, 4.0)
                .unwrap()
                .translated((5.0, 0.0))
                .unwrap(),
        );
        let root = builder
            .add_node(NodeKind::Revolve {
                profile: ring,
                placement: Placement3::translate(0.0, 0.0, 1.0),
                sweep: core::f64::consts::TAU,
                caps: CapMode::Both,
            })
            .unwrap();
        let lowered =
            NativeSource::from_recipe(&builder.finish(root).unwrap(), &lowering()).unwrap();
        // Pappus: area 8 about a centroid at radius 5.
        let exact = 8.0 * core::f64::consts::TAU * 5.0;
        assert!((lowered.volume().unwrap() - exact).abs() < 1e-9 * exact);
        assert_eq!(lowered.family(), crate::NativeFamily::Revolution);
    }

    /// The recipe compiled by mesh evaluation, and natively: their volumes
    /// and boxes, which agree for one solid to the meshes' chords.
    fn both_ways(recipe: Recipe) -> [(f64, [[f64; 3]; 2]); 2] {
        let native = PartSource::Native(NativeSource::from_recipe(&recipe, &lowering()).unwrap());
        let recipe = PartSource::Recipe(recipe);
        let parts = PartCompiler::new()
            .compile_parts(
                &[
                    PartInput::new("meshed", &recipe),
                    PartInput::new("native", &native),
                ],
                &CompilePolicy::default(),
            )
            .unwrap();
        [0, 1].map(|id| {
            let part = parts.part(crate::PartId(id)).unwrap();
            let volume: f64 = part
                .bodies
                .iter()
                .map(|body| body.signed_volume().unwrap().value)
                .sum();
            let bounds = part.bounds().unwrap();
            (volume, [bounds.min, bounds.max])
        })
    }

    fn agree(both: [(f64, [[f64; 3]; 2]); 2], reach: f64) {
        let [(meshed, a), (native, b)] = both;
        assert!(
            (meshed - native).abs() < 2e-3 * native.abs(),
            "{meshed} {native}"
        );
        for k in 0..2 {
            for axis in 0..3 {
                assert!((a[k][axis] - b[k][axis]).abs() < reach, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn pocketed_extrusions_lower_with_every_pocket() {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(plate());
        let leaf = builder.add_node(extrude(profile)).unwrap();
        let recess = builder.add_profile(
            builders::rect_centered(10.0, 6.0)
                .unwrap()
                .translated((-10.0, 5.0))
                .unwrap()
                .tagged(SegTag(9)),
        );
        let first = builder
            .add_node(NodeKind::Pocket {
                base: leaf,
                profile: recess,
                end: ExtrusionEnd::End,
                depth: 4.0,
            })
            .unwrap();
        let slot = builder.add_profile(
            builders::rect_centered(8.0, 4.0)
                .unwrap()
                .translated((10.0, -5.0))
                .unwrap(),
        );
        let root = builder
            .add_node(NodeKind::Pocket {
                base: first,
                profile: slot,
                end: ExtrusionEnd::Start,
                depth: 3.0,
            })
            .unwrap();
        let recipe = builder.finish(root).unwrap();
        let lowered = NativeSource::from_recipe(&recipe, &lowering()).unwrap();
        let solid = lowered.as_extrusion().unwrap();
        assert_eq!(solid.pockets().len(), 2);
        assert_eq!(solid.faces_tagged(SegTag(9)).count(), 4, "the recess walls");
        let exact = (1200.0 - core::f64::consts::PI * 9.0) * 12.0 - 60.0 * 4.0 - 32.0 * 3.0;
        assert!((lowered.volume().unwrap() - exact).abs() < 1e-9 * exact);
        agree(both_ways(recipe), 1e-4);
    }

    /// A peg attached by `surface` to a rotated block named `block`, with
    /// a pocket named `recess` cut into its end cap.
    fn peg(surface: SurfaceSelector, label: &str) -> Recipe {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::rect_centered(4.0, 3.0).unwrap());
        let block = builder.source_ref("block");
        let base = builder
            .add_node(
                Node::new(NodeKind::Extrude {
                    profile,
                    placement: Placement3::rotate_z_then_translate(0.4, 5.0, -2.0, 1.0),
                    height: 2.0,
                    caps: CapMode::Both,
                })
                .with_source(block),
            )
            .unwrap();
        let recess = builder.add_profile(
            builders::rect_centered(2.0, 1.0)
                .unwrap()
                .translated((-0.5, 0.5))
                .unwrap(),
        );
        let recess_label = builder.source_ref(label);
        let support = builder
            .add_node(
                Node::new(NodeKind::Pocket {
                    base,
                    profile: recess,
                    end: ExtrusionEnd::End,
                    depth: 0.75,
                })
                .with_source(recess_label),
            )
            .unwrap();
        let pin = builder.add_profile(builders::circle(0.1).unwrap());
        let child = builder
            .add_node(NodeKind::Extrude {
                profile: pin,
                placement: Placement3::IDENTITY,
                height: 0.3,
                caps: CapMode::Both,
            })
            .unwrap();
        let attached = builder
            .add_node(NodeKind::OnWorkplane {
                support,
                child,
                attachment: WorkplaneAttachment {
                    surface,
                    anchor: [4.6, -1.5, 0.0],
                    projection: [0.1, -0.2, 1.0],
                    x_direction: [1.0, 1.0, 0.3],
                },
            })
            .unwrap();
        let root = builder
            .add_node(NodeKind::Transform {
                child: attached,
                xf: Placement3::translate(0.0, 0.0, 3.0),
            })
            .unwrap();
        builder.finish(root).unwrap()
    }

    #[test]
    fn attachments_resolve_on_native_caps_and_pocket_floors() {
        for surface in [
            SurfaceSelector::EndCap,
            SurfaceSelector::SourceEndCap("block".into()),
            SurfaceSelector::SourceStartCap("block".into()),
            SurfaceSelector::SourceFloor("recess".into()),
        ] {
            agree(both_ways(peg(surface, "recess")), 1e-4);
        }
    }

    #[test]
    fn attachments_refuse_as_mesh_evaluation_does() {
        let refusal =
            |surface, label| NativeSource::from_recipe(&peg(surface, label), &lowering()).err();
        assert!(matches!(
            refusal(SurfaceSelector::SourceFloor("block".into()), "recess"),
            Some(NativeLoweringError::Attachment {
                error: WorkplaneError::EmptySelection,
                ..
            })
        ));
        assert!(matches!(
            refusal(SurfaceSelector::SourceEndCap("block".into()), "block"),
            Some(NativeLoweringError::Attachment {
                error: WorkplaneError::AmbiguousSelection,
                ..
            })
        ));
        assert!(matches!(
            refusal(SurfaceSelector::Region(1), "recess"),
            Some(NativeLoweringError::Unkept {
                request: "region_selection",
                ..
            })
        ));
        // A policy mesh evaluation would refuse, refused alike.
        let mut policy = lowering();
        policy.workplane.min_projection_cos = f64::NAN;
        assert!(matches!(
            NativeSource::from_recipe(&peg(SurfaceSelector::EndCap, "recess"), &policy),
            Err(NativeLoweringError::Attachment {
                error: WorkplaneError::InvalidInput,
                ..
            })
        ));
    }

    /// A box of `size` at `at`, tagged.
    fn cuboid(builder: &mut RecipeBuilder, size: [f64; 3], at: [f64; 3], tag: u32) -> NodeId {
        let profile = builder.add_profile(
            builders::rect_centered(size[0], size[1])
                .unwrap()
                .tagged(SegTag(tag)),
        );
        builder
            .add_node(NodeKind::Extrude {
                profile,
                placement: Placement3::translate(at[0], at[1], at[2]),
                height: size[2],
                caps: CapMode::Both,
            })
            .unwrap()
    }

    fn csg(builder: &mut RecipeBuilder, op: CsgOp, operands: Vec<NodeId>) -> NodeId {
        builder.add_node(NodeKind::Csg { op, operands }).unwrap()
    }

    #[test]
    fn attachments_nested_past_the_policy_are_refused() {
        // Each peg stands on the end cap of the one before.
        let mut builder = RecipeBuilder::new();
        let square = builder.add_profile(builders::rect_centered(1.0, 1.0).unwrap());
        let mut root = builder.add_node(extrude(square)).unwrap();
        for _ in 0..100 {
            let child = builder.add_node(extrude(square)).unwrap();
            root = builder
                .add_node(NodeKind::OnWorkplane {
                    support: root,
                    child,
                    attachment: WorkplaneAttachment {
                        surface: SurfaceSelector::EndCap,
                        anchor: [0.0; 3],
                        projection: [0.0, 0.0, 1.0],
                        x_direction: [1.0, 0.0, 0.0],
                    },
                })
                .unwrap();
        }
        let recipe = builder.finish(root).unwrap();
        assert!(matches!(
            NativeSource::from_recipe(&recipe, &lowering()),
            Err(NativeLoweringError::Depth { maximum: 64, .. })
        ));
    }

    #[test]
    fn axis_aligned_booleans_lower_exactly() {
        // 4 x 4 x 2 and 2 x 2 x 4, overlapping in a 2 x 2 x 1 block.
        for (op, volume) in [
            (CsgOp::Union, 32.0 + 16.0 - 4.0),
            (CsgOp::Difference, 32.0 - 4.0),
            (CsgOp::Intersection, 4.0),
        ] {
            let mut builder = RecipeBuilder::new();
            let slab = cuboid(&mut builder, [4.0, 4.0, 2.0], [1.0, 1.0, 0.0], 3);
            let post = cuboid(&mut builder, [2.0, 2.0, 4.0], [2.0, 2.0, 1.0], 4);
            let root = csg(&mut builder, op, alloc::vec![slab, post]);
            let recipe = builder.finish(root).unwrap();
            let lowered = NativeSource::from_recipe(&recipe, &lowering()).unwrap();
            assert_eq!(lowered.family(), crate::NativeFamily::Body);
            assert!((lowered.volume().unwrap() - volume).abs() < 1e-12, "{op:?}");
            agree(both_ways(recipe), 1e-6);
        }
    }

    #[test]
    fn circular_cutters_through_a_body_lower_to_bores() {
        let mut builder = RecipeBuilder::new();
        let slab = cuboid(&mut builder, [6.0, 4.0, 2.0], [0.0, 0.0, 0.0], 3);
        let bore = builder.add_profile(builders::circle(1.0).unwrap().tagged(SegTag(8)));
        let cutter = builder
            .add_node(NodeKind::Extrude {
                profile: bore,
                placement: Placement3::translate(1.0, 0.5, -1.0),
                height: 4.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let root = csg(&mut builder, CsgOp::Difference, alloc::vec![slab, cutter]);
        let recipe = builder.finish(root).unwrap();
        let lowered = NativeSource::from_recipe(&recipe, &lowering()).unwrap();
        let exact = 48.0 - core::f64::consts::PI * 2.0;
        assert!((lowered.volume().unwrap() - exact).abs() < 1e-9 * exact);
        let body = lowered.to_body();
        assert!(
            body.faces().iter().any(|face| face.role == [SegTag(8)]),
            "the bore's wall carries the cutter's tag"
        );
        agree(both_ways(recipe), 2e-3);
    }

    #[test]
    fn booleans_refuse_what_they_cannot_make_exactly() {
        let refusal = |build: &dyn Fn(&mut RecipeBuilder) -> NodeId| {
            let mut builder = RecipeBuilder::new();
            let root = build(&mut builder);
            NativeSource::from_recipe(&builder.finish(root).unwrap(), &lowering()).err()
        };
        // A circular cutter stopping inside the body is no through cut.
        assert!(matches!(
            refusal(&|builder| {
                let slab = cuboid(builder, [6.0, 4.0, 2.0], [0.0; 3], 3);
                let bore = builder.add_profile(builders::circle(1.0).unwrap());
                let cutter = builder
                    .add_node(NodeKind::Extrude {
                        profile: bore,
                        placement: Placement3::translate(1.0, 0.5, 1.0),
                        height: 4.0,
                        caps: CapMode::Both,
                    })
                    .unwrap();
                csg(builder, CsgOp::Difference, alloc::vec![slab, cutter])
            }),
            Some(NativeLoweringError::Boolean {
                operand: 1,
                error: NativeBooleanError::Rectilinear(RectilinearError::UnsupportedFace { .. }),
                ..
            })
        ));
        // Two arcs of nearly one circle are not that circle's cylinder.
        assert!(matches!(
            refusal(&|builder| {
                let slab = cuboid(builder, [10.0, 10.0, 2.0], [0.0; 3], 3);
                let lens = builder.add_profile(
                    Profile2::simple(
                        Loop2::new(alloc::vec![
                            Seg2::arc((1.0, 0.0), 1.0),
                            Seg2::arc((-1.0, 0.0), 0.95),
                        ])
                        .unwrap(),
                    )
                    .unwrap(),
                );
                let cutter = builder
                    .add_node(NodeKind::Extrude {
                        profile: lens,
                        placement: Placement3::translate(0.0, 0.0, -1.0),
                        height: 4.0,
                        caps: CapMode::Both,
                    })
                    .unwrap();
                csg(builder, CsgOp::Difference, alloc::vec![slab, cutter])
            }),
            Some(NativeLoweringError::Boolean {
                operand: 1,
                error: NativeBooleanError::Rectilinear(RectilinearError::UnsupportedFace { .. }),
                ..
            })
        ));
        // Turned operands have no common axis-aligned arrangement.
        assert!(matches!(
            refusal(&|builder| {
                let slab = cuboid(builder, [4.0, 4.0, 2.0], [0.0; 3], 3);
                let post = cuboid(builder, [1.0, 1.0, 4.0], [0.0; 3], 4);
                let turned = builder
                    .add_node(NodeKind::Transform {
                        child: post,
                        xf: Placement3::rotate_z_then_translate(0.3, 0.0, 0.0, 0.0),
                    })
                    .unwrap();
                csg(builder, CsgOp::Union, alloc::vec![slab, turned])
            }),
            Some(NativeLoweringError::Boolean {
                error: NativeBooleanError::Orientation,
                ..
            })
        ));
        // Disjoint operands have no material in common.
        assert!(matches!(
            refusal(&|builder| {
                let a = cuboid(builder, [1.0, 1.0, 1.0], [0.0; 3], 3);
                let b = cuboid(builder, [1.0, 1.0, 1.0], [5.0, 0.0, 0.0], 4);
                csg(builder, CsgOp::Intersection, alloc::vec![a, b])
            }),
            Some(NativeLoweringError::Boolean {
                error: NativeBooleanError::Empty,
                ..
            })
        ));
    }

    #[test]
    fn nesting_past_the_policy_is_refused() {
        // Each union's first operand is the union before it.
        let mut builder = RecipeBuilder::new();
        let unit = cuboid(&mut builder, [1.0, 1.0, 1.0], [0.0; 3], 3);
        let mut root = unit;
        for _ in 0..100 {
            root = csg(&mut builder, CsgOp::Union, alloc::vec![root, unit]);
        }
        let recipe = builder.finish(root).unwrap();
        assert!(matches!(
            NativeSource::from_recipe(&recipe, &lowering()),
            Err(NativeLoweringError::Depth { maximum: 64, .. })
        ));
    }

    #[test]
    fn sweeps_lower_with_constructive_framing() {
        // An off-center section around a bend: a mirrored or rolled frame
        // would move it to the other side, changing the volume and box.
        let mut builder = RecipeBuilder::new();
        let section = builder.add_profile(
            builders::rect_centered(4.0, 2.0)
                .unwrap()
                .translated((3.0, 1.0))
                .unwrap(),
        );
        let path = exedra_constructive::path::PathCursor3::new([0.0; 3], [0.0, 1.0, 0.0])
            .line_by([0.0, 10.0, 0.0])
            .tangent_arc(20.0, core::f64::consts::FRAC_PI_2, [0.0, 0.0, 1.0])
            .line_by([-10.0, 0.0, 0.0])
            .finish([1.0, 0.0, 0.0])
            .unwrap();
        let leaf = builder
            .add_node(NodeKind::Sweep {
                profile: section,
                path,
                section: SectionLaw::IDENTITY,
                caps: CapMode::Both,
            })
            .unwrap();
        let root = builder
            .add_node(NodeKind::Transform {
                child: leaf,
                xf: Placement3::translate(0.0, 0.0, 5.0),
            })
            .unwrap();
        agree(both_ways(builder.finish(root).unwrap()), 0.05);
    }

    #[test]
    fn ruled_lofts_lower_through_their_placed_sections() {
        let mut builder = RecipeBuilder::new();
        let base = builder.add_profile(builders::rect_centered(4.0, 4.0).unwrap());
        let top = builder.add_profile(builders::rect_centered(2.0, 6.0).unwrap());
        let root = builder
            .add_node(NodeKind::Loft {
                sections: alloc::vec![
                    exedra_constructive::ir::LoftSection::new(Placement3::IDENTITY, base),
                    exedra_constructive::ir::LoftSection::new(
                        Placement3::translate(1.0, 0.0, 5.0),
                        top
                    ),
                ],
                policy: Interpolation::Ruled,
                caps: CapMode::Both,
            })
            .unwrap();
        agree(both_ways(builder.finish(root).unwrap()), 1e-4);
    }

    #[test]
    fn what_native_construction_would_not_keep_is_refused_by_node() {
        let lower = |builder: RecipeBuilder, root| {
            NativeSource::from_recipe(&builder.finish(root).unwrap(), &lowering()).err()
        };
        // An operation with no native construction.
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(plate());
        let a = builder.add_node(extrude(profile)).unwrap();
        let root = builder
            .add_node(NodeKind::Mirror {
                child: a,
                plane: exedra_constructive::ir::Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 0.0,
                },
            })
            .unwrap();
        assert_eq!(
            lower(builder, root),
            Some(NativeLoweringError::Unsupported {
                node: root,
                operation: "mirror"
            })
        );
        // A material binding, and open caps.
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(plate());
        let slot = builder.material_slot("steel");
        let kind = extrude(profile);
        let root = builder
            .add_node(Node {
                material: Some(slot),
                ..Node::new(kind)
            })
            .unwrap();
        assert_eq!(
            lower(builder, root),
            Some(NativeLoweringError::Unkept {
                node: root,
                request: "material"
            })
        );
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(plate());
        let root = builder
            .add_node(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Start,
            })
            .unwrap();
        assert_eq!(
            lower(builder, root),
            Some(NativeLoweringError::Unkept {
                node: root,
                request: "open_caps"
            })
        );
        // A policy-defined segment, and a placement that is not rigid.
        let mut builder = RecipeBuilder::new();
        let policy_id = builder.curve_policy("spec.transition@1");
        let profile = builder.add_profile(
            Profile2::simple(
                Loop2::new(alloc::vec![
                    Seg2::line((2.0, 0.0)),
                    Seg2::policy((0.0, 0.0), policy_id, SegKind::Arc { bulge: 0.5 }),
                ])
                .unwrap(),
            )
            .unwrap(),
        );
        let kind = extrude(profile);
        let root = builder.add_node(kind).unwrap();
        assert!(matches!(
            lower(builder, root),
            Some(NativeLoweringError::Profile {
                error: ToProfileError::PolicyDefined { hole: None, seg: 1 },
                ..
            })
        ));
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(plate());
        let kind = extrude(profile);
        let leaf = builder.add_node(kind).unwrap();
        let root = builder
            .add_node(NodeKind::Transform {
                child: leaf,
                xf: Placement3::from_axes(
                    [2.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [0.0; 3],
                ),
            })
            .unwrap();
        assert!(matches!(
            lower(builder, root),
            Some(NativeLoweringError::Extrusion {
                node,
                error: ExtrusionError::SingularFrame(_),
            }) if node == leaf
        ));
    }
}
