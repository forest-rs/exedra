// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One component, two construction routes.
//!
//! A mounting plate with a bolt hole, an adjustment slot and a central boss is
//! built two ways:
//!
//! - [`build_plate`] is handwritten: it draws the cutouts with a
//!   [`LoopCursor`], states extent and axis with [`ops`] records, and builds
//!   into a caller's [`RecipeBuilder`].
//! - [`plate_parts`] is shaped like a spec compiler's output: it emits complete
//!   [`RecipeParts`] tables in one pass for [`Recipe::try_from_parts`].
//!
//! Both routes produce the same tables in the same order, so their recipes
//! have the same fingerprint. Profile segments carry [`SLOT`] and
//! [`BOLT_HOLE`] tags, which evaluation carries to the wall faces they generate.
//!
//! [`ops`]: exedra::constructive::ops

use core::f64::consts::PI;
use core::fmt;

use exedra::constructive::builders;
use exedra::constructive::ir::{
    Node, NodeId, NodeKind, Placement3, Recipe, RecipeBuilder, RecipeError, RecipeParts,
    RecipePartsError,
};
use exedra::constructive::ops::{Axis, Extrude, Revolve};
use exedra::constructive::profile::{LoopCursor, Profile2, ProfileError, SegTag};

/// Tag on every segment of the adjustment slot.
pub const SLOT: SegTag = SegTag(100);
/// Tag on every segment of the bolt hole.
pub const BOLT_HOLE: SegTag = SegTag(101);

/// Opaque source references, in table order.
const SOURCES: [&str; 2] = ["plate", "plate/boss"];

/// Plate dimensions in millimeters. The plate is centered on the origin with
/// its midplane at `z = 0`; the slot runs along X.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlateSpec {
    /// Length along X.
    pub width: f64,
    /// Length along Y.
    pub depth: f64,
    /// Thickness along Z.
    pub thickness: f64,
    /// Outline corner radius.
    pub corner_radius: f64,
    /// Bolt hole radius.
    pub hole_radius: f64,
    /// Distance from each end of the plate to the hole and slot centers.
    pub end_inset: f64,
    /// Center-to-center length of the slot's round ends.
    pub slot_length: f64,
    /// Slot width.
    pub slot_width: f64,
    /// Boss radius.
    pub boss_radius: f64,
    /// Boss height above the plate's top face.
    pub boss_height: f64,
}

impl Default for PlateSpec {
    fn default() -> Self {
        Self {
            width: 120.0,
            depth: 50.0,
            thickness: 6.0,
            corner_radius: 8.0,
            hole_radius: 4.5,
            end_inset: 15.0,
            slot_length: 12.0,
            slot_width: 9.0,
            boss_radius: 12.0,
            boss_height: 10.0,
        }
    }
}

/// Nodes of a plate built into a recipe.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PlateHandles {
    /// The extruded plate, bound to source `plate`.
    pub plate: NodeId,
    /// The revolved boss, bound to source `plate/boss`.
    pub boss: NodeId,
    /// The group of both.
    pub root: NodeId,
}

/// Why a plate could not be built.
#[derive(Clone, Debug, PartialEq)]
pub enum PlateError {
    /// A profile is invalid, such as a cutout that escapes the outline.
    Profile(ProfileError),
    /// A node is invalid, such as a nonpositive thickness.
    Recipe(RecipeError),
    /// The complete tables were refused.
    Parts(RecipePartsError),
}

impl fmt::Display for PlateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(error) => write!(f, "plate profile: {error}"),
            Self::Recipe(error) => write!(f, "plate node: {error}"),
            Self::Parts(error) => write!(f, "plate tables: {error}"),
        }
    }
}

impl core::error::Error for PlateError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Profile(error) => Some(error),
            Self::Recipe(error) => Some(error),
            Self::Parts(error) => Some(error),
        }
    }
}

impl From<ProfileError> for PlateError {
    fn from(error: ProfileError) -> Self {
        Self::Profile(error)
    }
}

impl From<RecipeError> for PlateError {
    fn from(error: RecipeError) -> Self {
        Self::Recipe(error)
    }
}

/// The plate outline with its bolt hole (hole 0) and slot (hole 1).
///
/// # Errors
///
/// Returns the profile error for dimensions that do not fit, for example
/// [`ProfileError::HoleOutsideOuter`] naming the cutout that escapes.
pub fn plate_profile(spec: &PlateSpec) -> Result<Profile2, ProfileError> {
    let end = 0.5 * spec.width - spec.end_inset;
    let (r, half, w) = (
        spec.hole_radius,
        0.5 * spec.slot_length,
        0.5 * spec.slot_width,
    );
    let hole = LoopCursor::new((-end + r, 0.0))
        .arc_through((-end, r), (-end - r, 0.0))
        .tagged(BOLT_HOLE)
        .arc_through((-end, -r), (-end + r, 0.0))
        .tagged(BOLT_HOLE)
        .close()?;
    let slot = LoopCursor::new((end - half, -w))
        .line_to((end + half, -w))
        .tagged(SLOT)
        .arc_about((end + half, 0.0), PI)
        .tagged(SLOT)
        .line_to((end - half, w))
        .tagged(SLOT)
        .arc_through((end - half - w, 0.0), (end - half, -w))
        .tagged(SLOT)
        .close()?;
    builders::rounded_rect_centered(spec.width, spec.depth, spec.corner_radius)?
        .with_cutouts([Profile2::simple(hole)?, Profile2::simple(slot)?])
}

/// The boss's radius-height profile, revolved about Z.
fn boss_profile(spec: &PlateSpec) -> Result<Profile2, ProfileError> {
    builders::rect_from_corner(spec.boss_radius, spec.boss_height)
}

/// The plate extrusion and the boss revolution, lowered to native nodes.
fn plate_operations(spec: &PlateSpec, profiles: [u32; 2]) -> [NodeKind; 2] {
    use exedra::constructive::ir::ProfileId;
    let top = Placement3::translate(0.0, 0.0, 0.5 * spec.thickness);
    [
        Extrude::symmetric(ProfileId(profiles[0]), spec.thickness).into(),
        Revolve::full(ProfileId(profiles[1]))
            .about(Axis::Z)
            .placed(top)
            .into(),
    ]
}

/// Builds the plate into `builder` and returns its nodes.
///
/// Profiles, sources and nodes join the builder's tables. Building is
/// all-or-abandon: an error after the first insertion leaves earlier tables in
/// the builder, so the caller discards the builder and propagates the error
/// rather than continuing with it. Each single insertion is still checked and
/// atomic.
///
/// # Errors
///
/// Returns [`PlateError::Profile`] or [`PlateError::Recipe`] for dimensions
/// that do not construct.
pub fn build_plate(
    builder: &mut RecipeBuilder,
    spec: &PlateSpec,
) -> Result<PlateHandles, PlateError> {
    let outline = builder.add_profile(plate_profile(spec)?);
    let boss = builder.add_profile(boss_profile(spec)?);
    let [plate_kind, boss_kind] = plate_operations(spec, [outline.0, boss.0]);
    let [plate_source, boss_source] = SOURCES.map(|source| builder.source_ref(source));
    let plate = builder.add_node(Node::new(plate_kind).with_source(plate_source))?;
    let boss = builder.add_node(Node::new(boss_kind).with_source(boss_source))?;
    let root = builder.add_node(NodeKind::Group {
        children: vec![plate, boss],
    })?;
    Ok(PlateHandles { plate, boss, root })
}

/// Emits the plate as complete tables, as a compiler that holds the whole
/// component at once would.
///
/// # Errors
///
/// Returns [`PlateError::Profile`] when a profile does not construct. Node
/// validation happens in [`Recipe::try_from_parts`].
pub fn plate_parts(spec: &PlateSpec) -> Result<RecipeParts, PlateError> {
    let profiles = vec![plate_profile(spec)?, boss_profile(spec)?];
    let [plate, boss] = plate_operations(spec, [0, 1]);
    let source = |index: u32| exedra::constructive::ir::SourceId(index);
    Ok(RecipeParts {
        profiles,
        sources: SOURCES.map(String::from).into(),
        slots: Vec::new(),
        policies: Vec::new(),
        imports: Vec::new(),
        nodes: vec![
            Node::new(plate).with_source(source(0)),
            Node::new(boss).with_source(source(1)),
            Node::new(NodeKind::Group {
                children: vec![NodeId(0), NodeId(1)],
            }),
        ],
        root: NodeId(2),
    })
}

/// Freezes [`plate_parts`] into a recipe.
///
/// # Errors
///
/// Returns the profile or table error for dimensions that do not construct.
pub fn compile_plate(spec: &PlateSpec) -> Result<Recipe, PlateError> {
    Recipe::try_from_parts(plate_parts(spec)?).map_err(PlateError::Parts)
}

/// Builds the plate by hand into a fresh builder and freezes it.
///
/// # Errors
///
/// Returns the [`build_plate`] error; the builder is dropped with it.
pub fn handwritten_plate(spec: &PlateSpec) -> Result<Recipe, PlateError> {
    let mut builder = RecipeBuilder::new();
    let handles = build_plate(&mut builder, spec)?;
    Ok(builder.finish(handles.root)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra::constructive::evaluate::{Fidelity, evaluate};
    use exedra::constructive::ir::RecipeTable;
    use exedra::constructive::tessellate::{EvalPolicy, Feature};

    #[test]
    fn both_routes_freeze_to_the_same_recipe() {
        let spec = PlateSpec::default();
        let handwritten = handwritten_plate(&spec).unwrap();
        let compiled = compile_plate(&spec).unwrap();
        assert_eq!(
            handwritten.recipe_fingerprint(),
            compiled.recipe_fingerprint()
        );
        let wider = PlateSpec {
            width: 140.0,
            ..spec
        };
        assert_ne!(
            compile_plate(&wider).unwrap().recipe_fingerprint(),
            compiled.recipe_fingerprint()
        );
    }

    #[test]
    fn tagged_cutouts_select_their_wall_faces() {
        let recipe = handwritten_plate(&PlateSpec::default()).unwrap();
        let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
        for body in &result.bodies {
            assert_eq!(result.report.fidelity_of(body.node), Some(Fidelity::Exact));
            assert!(body.body.mesh.validate_deep().is_empty());
        }
        // The boss stands on the plate's top face.
        let boss = &result.bodies[1].body.mesh;
        let heights: Vec<f32> = boss
            .vertices()
            .map(|v| boss.vertex_position(v).unwrap()[2])
            .collect();
        let low = heights.iter().copied().fold(f32::INFINITY, f32::min);
        let high = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert_eq!((low, high), (3.0, 13.0));
        let plate = &result.bodies[0];
        assert_eq!(recipe.source_of(plate.node), Some("plate"));
        let profile = recipe
            .profile(exedra::constructive::ir::ProfileId(0))
            .unwrap();
        for (loop_index, tag, segments) in [(1, BOLT_HOLE, 2), (2, SLOT, 4)] {
            let loop_ = &profile.holes()[usize::from(loop_index) - 1];
            assert!(loop_.segs().iter().all(|seg| seg.tag == Some(tag)));
            for seg in 0..segments {
                let feature = Feature::Wall { loop_index, seg };
                assert!(!plate.body.source_map.faces_for(feature).is_empty());
            }
        }
    }

    #[test]
    fn failures_name_their_cause_on_both_routes() {
        let escaping = PlateSpec {
            end_inset: 2.0,
            ..PlateSpec::default()
        };
        for result in [handwritten_plate(&escaping), compile_plate(&escaping)] {
            assert_eq!(
                result.unwrap_err(),
                PlateError::Profile(ProfileError::HoleOutsideOuter { hole: 0 })
            );
        }
        let flat = PlateSpec {
            thickness: 0.0,
            ..PlateSpec::default()
        };
        assert!(matches!(
            handwritten_plate(&flat),
            Err(PlateError::Recipe(RecipeError::InvalidParameter { .. }))
        ));
        let Err(PlateError::Parts(refused)) = compile_plate(&flat) else {
            panic!("a flat plate must be refused");
        };
        assert_eq!((refused.table, refused.index), (RecipeTable::Nodes, 0));
    }
}
