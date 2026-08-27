// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lowering a construction to an `exedra_assembly::Assembly`.
//!
//! The construction is the source of truth; the assembly is a compiled
//! artifact of it, exactly as a mesh is a compiled artifact of a recipe.
//! Lowering is a pure function of the construction, so the same graph lowers
//! to the same assembly every time.
//!
//! Three rules govern the result:
//!
//! - **Identity is derived.** Every geometry-bearing element becomes one root
//!   instance keyed by the element key, so its
//!   [`exedra_assembly::InstanceAddress`] is the element key beneath `/` and
//!   nothing else.
//!   Generated parts are elements, so they lower the same way.
//! - **Part edits are composed before registration.** A cut is a constructive
//!   [`NodeKind::Csg`] on the participant's own recipe, appended in
//!   application order and frozen before the part is handed to the assembly.
//!   There is never a boolean between two placed instances —
//!   `exedra_assembly` ADR-0001 puts those out of scope, and this is how
//!   `joiner` stays inside it.
//! - **Hierarchy is not connectivity.** Every instance is a root. Structural
//!   connectivity lives in relations, contacts, and transfers, never in
//!   parent-child placement.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use exedra_assembly::{Assembly, AssemblyError, InstanceAddress};
use exedra_constructive::ir::{
    CsgOp, ImportId, NodeId, NodeKind, Placement3, ProfileId, Recipe, RecipeBuilder, RecipeError,
    SlotId, SourceId,
};

use crate::construction::Construction;
use crate::element::{Element, ElementOrigin};
use crate::rule::PartEditOp;

/// Typed lowering failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LowerError {
    /// The assembly rejected a part, instance, slot, or placement.
    Assembly(AssemblyError),
    /// A composed recipe failed to build.
    Recipe(RecipeError),
    /// A tool recipe uses a node kind this crate cannot splice.
    ///
    /// `exedra_constructive::ir::NodeKind` is `#[non_exhaustive]`, so a node
    /// added upstream is reported here rather than silently dropped.
    UnsupportedToolNode {
        /// The element whose part was being composed.
        element: String,
        /// The tool that could not be spliced.
        tool: String,
    },
    /// A spliced recipe carries curve policies that cannot keep their
    /// interned identities in the composed recipe.
    ///
    /// Profile segments reference policies by index, and this crate does not
    /// rebuild profiles, so a tool may only bring policies that intern to the
    /// same indices they already had.
    PolicyRemapUnsupported {
        /// The element whose part was being composed.
        element: String,
        /// The tool that could not be spliced.
        tool: String,
    },
}

impl core::fmt::Display for LowerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Assembly(error) => write!(f, "assembly rejected the lowering: {error}"),
            Self::Recipe(error) => write!(f, "composed recipe is invalid: {error}"),
            Self::UnsupportedToolNode { element, tool } => write!(
                f,
                "tool {tool:?} on element {element:?} uses an unsupported node kind"
            ),
            Self::PolicyRemapUnsupported { element, tool } => write!(
                f,
                "tool {tool:?} on element {element:?} carries curve policies that cannot be re-interned"
            ),
        }
    }
}

impl core::error::Error for LowerError {}

impl From<AssemblyError> for LowerError {
    fn from(error: AssemblyError) -> Self {
        Self::Assembly(error)
    }
}

impl From<RecipeError> for LowerError {
    fn from(error: RecipeError) -> Self {
        Self::Recipe(error)
    }
}

/// The instance address an element lowers to.
///
/// Derived, deterministic, and flat: the element key is the only segment.
/// Returns `None` when `element` is not a valid Addressable name segment; the
/// same key would be rejected when the construction is lowered.
#[must_use]
pub fn instance_address(element: &str) -> Option<InstanceAddress> {
    InstanceAddress::parse(&format!("/{element}")).ok()
}

/// The assembly part key an element lowers to.
#[must_use]
pub fn part_key(element: &str) -> String {
    format!("part-{element}")
}

/// Lowers every present, geometry-bearing element.
///
/// # Errors
///
/// Returns [`LowerError`] when a part edit cannot be composed or the assembly
/// rejects a registration.
pub fn lower(construction: &Construction) -> Result<Assembly, LowerError> {
    lower_selected(construction, |_| true)
}

/// Lowers the present, geometry-bearing elements `include` accepts.
///
/// Selection is how a consumer builds a diagnostic layer — framing only,
/// skin only — without pretending the omitted elements are absent from the
/// hypothesis. Use [`Construction::set_element_present`] for that.
///
/// # Errors
///
/// Returns [`LowerError`] when a part edit cannot be composed or the assembly
/// rejects a registration.
pub fn lower_selected(
    construction: &Construction,
    include: impl Fn(&Element) -> bool,
) -> Result<Assembly, LowerError> {
    let mut assembly = Assembly::new();
    for element in construction.elements() {
        if !element.present || !include(element) {
            continue;
        }
        let Some(part) = element.part.as_ref() else {
            continue;
        };
        let recipe = compose(construction, element)?;
        let key = part_key(&element.key);
        let id = assembly.add_recipe_part(&key, recipe)?;
        assembly.set_default_slot(id, &part.slot)?;
        assembly.set_part_material(id, &part.slot, &element.material)?;
        let instance = assembly.add_instance(None, &element.key, id, element.extent.placement())?;
        assembly.set_metadata(instance, "structural_role", &element.role)?;
        assembly.set_metadata(instance, "evidence_class", element.evidence.class.label())?;
        assembly.set_metadata(instance, "evidence_source", &element.evidence.source)?;
        if let ElementOrigin::Generated(application) = &element.origin {
            assembly.set_metadata(instance, "generated_by", application)?;
        }
    }
    Ok(assembly)
}

/// Composes an element's part edits onto its recipe, in application order.
///
/// The result is what lowering registers: a single frozen recipe in which
/// every seat, housing, and void is an ordinary constructive operation.
///
/// # Errors
///
/// Returns [`LowerError`] when a tool cannot be spliced or the composed
/// recipe is invalid.
pub fn compose(construction: &Construction, element: &Element) -> Result<Recipe, LowerError> {
    let Some(part) = element.part.as_ref() else {
        return Err(LowerError::Recipe(RecipeError::Empty));
    };
    let edits: Vec<_> = construction.part_edits_for(&element.key).collect();
    if edits.is_empty() {
        return Ok(part.recipe.clone());
    }
    let mut builder = RecipeBuilder::new();
    let mut root = splice(&mut builder, &part.recipe)
        .map_err(|error| error.into_lower(&element.key, &format!("base:{}", element.key)))?;
    // Consecutive edits of the same kind fold into one n-ary node, because
    // that is what the operation already means: a difference is "the first
    // operand minus the union of the rest", an intersection is the common
    // part of all of them. Folding is not only fewer nodes — it is the only
    // form in which cutters that touch or overlap each other are unioned
    // inside a single boolean evaluation rather than being applied to a body
    // the previous cutter already opened.
    let mut index = 0;
    while index < edits.len() {
        let op = match edits[index].op {
            PartEditOp::RemoveSolid(_) => CsgOp::Difference,
            PartEditOp::RetainSolid(_) => CsgOp::Intersection,
        };
        let mut operands = alloc::vec![root];
        while index < edits.len() && edit_op(&edits[index].op) == op {
            let tool = edits[index].op.tool();
            let mut node = splice(&mut builder, &tool.recipe)
                .map_err(|error| error.into_lower(&element.key, &tool.key))?;
            if tool.placement != Placement3::IDENTITY {
                node = builder.add(NodeKind::Transform {
                    child: node,
                    xf: tool.placement,
                })?;
            }
            operands.push(node);
            index += 1;
        }
        root = builder.add(NodeKind::Csg { op, operands })?;
    }
    Ok(builder.finish(root)?)
}

/// Why a recipe could not be grafted into another recipe's builder.
enum SpliceError {
    UnsupportedNode,
    PolicyRemap,
    Recipe(RecipeError),
}

impl SpliceError {
    fn into_lower(self, element: &str, tool: &str) -> LowerError {
        match self {
            Self::UnsupportedNode => LowerError::UnsupportedToolNode {
                element: element.to_string(),
                tool: tool.to_string(),
            },
            Self::PolicyRemap => LowerError::PolicyRemapUnsupported {
                element: element.to_string(),
                tool: tool.to_string(),
            },
            Self::Recipe(error) => LowerError::Recipe(error),
        }
    }
}

impl From<RecipeError> for SpliceError {
    fn from(error: RecipeError) -> Self {
        Self::Recipe(error)
    }
}

/// Grafts a frozen recipe into `builder`, returning the grafted root.
///
/// Every interned table is re-interned and every id remapped, so the graft
/// keeps its meaning inside a recipe it did not start in. Curve policies are
/// the one table that cannot be remapped: profile segments reference them by
/// index and this crate never rebuilds a profile.
fn splice(builder: &mut RecipeBuilder, source: &Recipe) -> Result<NodeId, SpliceError> {
    for (index, policy) in source.policies().iter().enumerate() {
        let id = builder.curve_policy(policy);
        if usize::try_from(id.0) != Ok(index) {
            return Err(SpliceError::PolicyRemap);
        }
    }
    let profiles: Vec<ProfileId> = source
        .profiles()
        .iter()
        .map(|profile| builder.add_profile(profile.clone()))
        .collect();
    let sources: Vec<SourceId> = source
        .sources()
        .iter()
        .map(|name| builder.source_ref(name))
        .collect();
    let slots: Vec<SlotId> = source
        .slots()
        .iter()
        .map(|name| builder.material_slot(name))
        .collect();
    let mut imports: Vec<ImportId> = Vec::with_capacity(source.imports().len());
    for mesh in source.imports() {
        imports.push(builder.add_import(mesh.clone())?);
    }

    let mut nodes: Vec<NodeId> = Vec::with_capacity(source.nodes().len());
    for node in source.nodes() {
        let kind =
            remap(&node.kind, &profiles, &nodes, &imports).ok_or(SpliceError::UnsupportedNode)?;
        if let Some(id) = node.source.and_then(|id| sources.get(id.0 as usize)) {
            builder.with_source(*id);
        }
        if let Some(id) = node.material.and_then(|id| slots.get(id.0 as usize)) {
            builder.with_material(*id);
        }
        if let Some(id) = node.issue.and_then(|id| sources.get(id.0 as usize)) {
            builder.with_issue(*id);
        }
        nodes.push(builder.add(kind)?);
    }
    nodes
        .get(source.root().0 as usize)
        .copied()
        .ok_or(SpliceError::Recipe(RecipeError::Empty))
}

/// Rewrites one node's references into the destination recipe's id space.
///
/// Returns `None` for a node kind this crate does not know how to remap;
/// `NodeKind` is `#[non_exhaustive]`, so that case is reported rather than
/// guessed at.
fn remap(
    kind: &NodeKind,
    profiles: &[ProfileId],
    nodes: &[NodeId],
    imports: &[ImportId],
) -> Option<NodeKind> {
    let profile = |id: ProfileId| profiles.get(id.0 as usize).copied();
    let node = |id: NodeId| nodes.get(id.0 as usize).copied();
    Some(match kind {
        NodeKind::Extrude {
            profile: id,
            placement,
            height,
            caps,
        } => NodeKind::Extrude {
            profile: profile(*id)?,
            placement: *placement,
            height: *height,
            caps: *caps,
        },
        NodeKind::Revolve {
            profile: id,
            placement,
            sweep,
            caps,
        } => NodeKind::Revolve {
            profile: profile(*id)?,
            placement: *placement,
            sweep: *sweep,
            caps: *caps,
        },
        NodeKind::Loft {
            sections,
            policy,
            caps,
        } => NodeKind::Loft {
            sections: sections
                .iter()
                .map(|(placement, id)| profile(*id).map(|id| (*placement, id)))
                .collect::<Option<Vec<_>>>()?,
            policy: *policy,
            caps: *caps,
        },
        NodeKind::Sweep {
            profile: id,
            path,
            caps,
        } => NodeKind::Sweep {
            profile: profile(*id)?,
            path: path.clone(),
            caps: *caps,
        },
        NodeKind::PlanarFace {
            profile: id,
            placement,
        } => NodeKind::PlanarFace {
            profile: profile(*id)?,
            placement: *placement,
        },
        NodeKind::Primitive { spec, placement } => NodeKind::Primitive {
            spec: *spec,
            placement: *placement,
        },
        NodeKind::Csg { op, operands } => NodeKind::Csg {
            op: *op,
            operands: operands
                .iter()
                .map(|id| node(*id))
                .collect::<Option<Vec<_>>>()?,
        },
        NodeKind::Transform { child, xf } => NodeKind::Transform {
            child: node(*child)?,
            xf: *xf,
        },
        NodeKind::Mirror { child, plane } => NodeKind::Mirror {
            child: node(*child)?,
            plane: *plane,
        },
        NodeKind::Instance { of, placement } => NodeKind::Instance {
            of: node(*of)?,
            placement: *placement,
        },
        NodeKind::Group { children } => NodeKind::Group {
            children: children
                .iter()
                .map(|id| node(*id))
                .collect::<Option<Vec<_>>>()?,
        },
        NodeKind::MeshImport { import, placement } => NodeKind::MeshImport {
            import: imports.get(import.0 as usize).copied()?,
            placement: *placement,
        },
        NodeKind::Stretch {
            child,
            plane,
            length,
        } => NodeKind::Stretch {
            child: node(*child)?,
            plane: *plane,
            length: *length,
        },
        NodeKind::GridSurface {
            points,
            rows,
            cols,
            close_u,
            close_w,
            thickness,
            placement,
        } => NodeKind::GridSurface {
            points: points.clone(),
            rows: *rows,
            cols: *cols,
            close_u: *close_u,
            close_w: *close_w,
            thickness: *thickness,
            placement: *placement,
        },
        _ => return None,
    })
}

/// The constructive operation one part edit lowers to.
fn edit_op(op: &PartEditOp) -> CsgOp {
    match op {
        PartEditOp::RemoveSolid(_) => CsgOp::Difference,
        PartEditOp::RetainSolid(_) => CsgOp::Intersection,
    }
}
