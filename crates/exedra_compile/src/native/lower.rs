// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native construction of constructive recipes.

use super::NativeSource;
use exedra_analytic::{
    extrusion::{Extrusion, ExtrusionError},
    loft::{Loft, LoftError, LoftPolicy},
    revolve::{Revolution, RevolutionError},
    sweep::{Sweep, SweepError},
};
use exedra_constructive::{
    ir::{CapMode, LoftPolicy as Interpolation, NodeId, NodeKind, Placement3, ProfileId, Recipe},
    profile::ToProfileError,
    section_law::SectionLaw,
};
use exedra_curves::profile::{Profile, ProfilePolicy};

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
        /// `"open_caps"`, a sweep's varying `"section_law"` or a loft's
        /// `"smooth_interpolation"`.
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
        }
    }
}

impl core::error::Error for NativeLoweringError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Unsupported { .. } | Self::Unkept { .. } => None,
            Self::Profile { error, .. } => Some(error),
            Self::Extrusion { error, .. } => Some(error),
            Self::Revolution { error, .. } => Some(error),
            Self::Sweep { error, .. } => Some(error),
            Self::Loft { error, .. } => Some(error),
        }
    }
}

impl NativeSource {
    /// The recipe's root built natively, so that its measures, selections
    /// and exchange are exact: an extrusion or revolution of a profile, a
    /// constant-section sweep or a ruled loft, under any rigid transforms. Profiles are validated under `policy`;
    /// segment tags become the solid's face tags.
    ///
    /// Lowering is the caller's explicit choice. A caller that requires
    /// exact geometry propagates the refusal; one that prefers it falls back
    /// to [`crate::PartSource::Recipe`], whose mesh evaluation covers every
    /// node. The node that produced the body, as mesh evaluation attributes
    /// it, lends it its source label, by which a snapshot resolves it.
    ///
    /// ```
    /// use exedra_compile::NativeSource;
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
    /// let native = NativeSource::from_recipe(&builder.finish(root)?, &ProfilePolicy::new(1e-9))?;
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
        policy: &ProfilePolicy,
    ) -> Result<Self, NativeLoweringError> {
        let mut id = recipe.root();
        let mut outer = Placement3::IDENTITY;
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
                    .to_profile(policy)
                    .map_err(|error| NativeLoweringError::Profile { node: id, error })
            };
            match &node.kind {
                NodeKind::Transform { child, xf } => {
                    outer = outer * *xf;
                    id = *child;
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
                    return Extrusion::new(profile(*p)?, outer * *placement, *height)
                        .map(|solid| produced(Self::from(solid), recipe, id))
                        .map_err(|error| NativeLoweringError::Extrusion { node: id, error });
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
                        .map(|solid| produced(Self::from(solid), recipe, id))
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
                        .map(|solid| produced(Self::from(solid), recipe, id))
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
                    return Loft::from_profiles(sections, outer, &LoftPolicy::new(*policy))
                        .map(|solid| produced(Self::from(solid), recipe, id))
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
}

/// `source`, labelled as the body `node` produced.
fn produced(source: NativeSource, recipe: &Recipe, node: NodeId) -> NativeSource {
    match recipe.source_of(node) {
        Some(label) => source.labelled(label),
        None => source,
    }
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
        let lowered = NativeSource::from_recipe(&recipe, &POLICY).unwrap();
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
        let native = NativeSource::from_recipe(&recipe, &POLICY).unwrap();
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
        let lowered = NativeSource::from_recipe(&builder.finish(root).unwrap(), &POLICY).unwrap();
        // Pappus: area 8 about a centroid at radius 5.
        let exact = 8.0 * core::f64::consts::TAU * 5.0;
        assert!((lowered.volume().unwrap() - exact).abs() < 1e-9 * exact);
        assert_eq!(lowered.family(), crate::NativeFamily::Revolution);
    }

    /// The recipe compiled by mesh evaluation, and natively: their volumes
    /// and boxes, which agree for one solid to the meshes' chords.
    fn both_ways(recipe: Recipe) -> [(f64, [[f64; 3]; 2]); 2] {
        let native = PartSource::Native(NativeSource::from_recipe(&recipe, &POLICY).unwrap());
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
            NativeSource::from_recipe(&builder.finish(root).unwrap(), &POLICY).err()
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
