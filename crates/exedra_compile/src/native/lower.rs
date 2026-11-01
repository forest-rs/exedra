// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native construction of constructive recipes.

use super::NativeSource;
use alloc::vec::Vec;
use exedra_analytic::{
    End,
    body::FaceId,
    extrusion::{Extrusion, ExtrusionError, FaceRole, Pocket},
    loft::{Loft, LoftError, LoftPolicy},
    revolve::{Revolution, RevolutionError},
    sweep::{Sweep, SweepError},
};
use exedra_constructive::{
    ir::{
        CapMode, ExtrusionEnd, LoftPolicy as Interpolation, NodeId, NodeKind, Placement3,
        ProfileId, Recipe,
    },
    profile::ToProfileError,
    section_law::SectionLaw,
    workplane::{SurfaceSelector, WorkplaneAttachment, WorkplaneError, WorkplanePolicy},
};
use exedra_curves::profile::{Profile, ProfilePolicy};
use exedra_math::{add, cross, dot, norm, normalize, scale, sub};

/// What lowering holds a recipe's native constructions to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeLoweringPolicy {
    /// Validation of every profile.
    pub profile: ProfilePolicy,
    /// How nearly an attachment's projection may lie in its face's plane,
    /// and its X along the face's normal, as mesh evaluation's policy says.
    pub workplane: WorkplanePolicy,
    /// How deeply attachment supports may nest, so that lowering a hostile
    /// recipe ends in a refusal, not an exhausted stack.
    pub max_depth: u32,
}

impl NativeLoweringPolicy {
    /// Profiles validated under `profile`, the default workplane policy,
    /// and nesting 64 deep.
    #[must_use]
    pub fn new(profile: ProfilePolicy) -> Self {
        Self {
            profile,
            workplane: WorkplanePolicy::default(),
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
    /// Attachment supports nest more deeply than the policy allows.
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
        }
    }
}

impl NativeSource {
    /// The recipe's root built natively, so that its measures, selections
    /// and exchange are exact: an extrusion of a profile with any pockets
    /// cut into it, a revolution, a constant-section sweep or a ruled loft,
    /// under any rigid transforms, and placed on a workplane attached to
    /// another such solid's cap or pocket floor, which is not emitted. Profiles are validated under `policy`;
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
        let kind = extrude(profile);
        let a = builder.add_node(kind.clone()).unwrap();
        let b = builder.add_node(kind).unwrap();
        let root = builder
            .add_node(NodeKind::Csg {
                op: CsgOp::Union,
                operands: alloc::vec![a, b],
            })
            .unwrap();
        assert_eq!(
            lower(builder, root),
            Some(NativeLoweringError::Unsupported {
                node: root,
                operation: "csg"
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
