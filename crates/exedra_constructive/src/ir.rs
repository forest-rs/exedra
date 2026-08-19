// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The constructive recipe IR: immutable, content-addressed, evaluated.
//!
//! A [`Recipe`] is a frozen DAG of constructive nodes over a table of
//! validated profiles. Recipes are built through [`RecipeBuilder`], which
//! enforces children-before-parent ordering (so cycles cannot exist) and
//! validates every node's parameters at insertion; [`RecipeBuilder::finish`]
//! freezes the recipe and computes content fingerprints bottom-up.
//!
//! ## Identity
//!
//! Every node has two identities with different jobs:
//!
//! - A **fingerprint** ([`Recipe::fingerprint`]): a 128-bit FNV-1a hash over
//!   the node's canonical bytes, its children's fingerprints, and
//!   [`crate::EVAL_SCHEMA_VERSION`]. Equal fingerprints mean equal evaluated
//!   geometry; fingerprints key caches.
//! - An optional **source reference** ([`SourceId`]): an opaque
//!   frontend-assigned string, interned per recipe. Source references give
//!   provenance continuity across recipe edits and are never parsed here.

use alloc::string::String;
use alloc::vec::Vec;

use crate::len_u32;
use crate::profile::{CanonBytes, Profile2};

/// Index of a profile in its recipe.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfileId(pub u32);

/// Index of a node in its recipe. Children always have smaller indices
/// than their parents.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

/// Interned opaque source reference supplied by a frontend.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceId(pub u32);

/// Interned material-slot name.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SlotId(pub u32);

/// A 128-bit content fingerprint.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub u128);

/// A rigid-or-affine placement as a 3x4 row-major matrix (rotation/scale
/// columns plus translation).
///
/// Constructors cover the common cases; matrices are validated finite at
/// node insertion. Mirroring belongs in [`NodeKind::Mirror`], scaling in
/// [`NodeKind::Transform`] — placements attached to bodies are expected to
/// be rigid, which evaluation checks.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Placement3 {
    /// Rows of the 3x4 matrix: `[r0x, r0y, r0z, tx]`, etc.
    pub rows: [[f64; 4]; 3],
}

impl Placement3 {
    /// The identity placement.
    pub const IDENTITY: Self = Self {
        rows: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };

    /// A pure translation.
    #[must_use]
    pub fn translate(x: f64, y: f64, z: f64) -> Self {
        Self {
            rows: [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z]],
        }
    }

    /// Rotation about +Z by `radians` (libm trig), then translation.
    #[must_use]
    pub fn rotate_z_then_translate(radians: f64, x: f64, y: f64, z: f64) -> Self {
        let (s, c) = (libm::sin(radians), libm::cos(radians));
        Self {
            rows: [[c, -s, 0.0, x], [s, c, 0.0, y], [0.0, 0.0, 1.0, z]],
        }
    }

    /// Euler rotation applied in x, then y', then z'' order (the catalog
    /// convention), followed by translation.
    #[must_use]
    pub fn euler_xyz_then_translate(rx: f64, ry: f64, rz: f64, t: [f64; 3]) -> Self {
        let (sx, cx) = (libm::sin(rx), libm::cos(rx));
        let (sy, cy) = (libm::sin(ry), libm::cos(ry));
        let (sz, cz) = (libm::sin(rz), libm::cos(rz));
        // R = Rz * Ry * Rx (intrinsic x, y', z'' == extrinsic z, y, x).
        let rows = [
            [
                cz * cy,
                cz * sy * sx - sz * cx,
                cz * sy * cx + sz * sx,
                t[0],
            ],
            [
                sz * cy,
                sz * sy * sx + cz * cx,
                sz * sy * cx - cz * sx,
                t[1],
            ],
            [-sy, cy * sx, cy * cx, t[2]],
        ];
        Self { rows }
    }

    fn is_finite(&self) -> bool {
        self.rows
            .iter()
            .all(|row| row.iter().all(|v| v.is_finite()))
    }
}

/// A plane in 3D: unit-ish normal plus signed distance from the origin.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Plane3 {
    /// Plane normal (must be nonzero and finite; not required to be unit).
    pub normal: [f64; 3],
    /// Signed distance term: the plane is `dot(normal, p) = distance`.
    pub distance: f64,
}

/// Which caps an extrusion or revolution closes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CapMode {
    /// Close both ends (a solid).
    #[default]
    Both,
    /// Close only the start cap.
    Start,
    /// Close only the end cap.
    End,
    /// Leave both ends open (a shell).
    None,
}

/// Loft section-correspondence policy.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoftPolicy {
    /// Ruled surface between equal-segment-count sections.
    #[default]
    Ruled,
}

/// Frame policy for polyline sweeps.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FramePolicy {
    /// Rotation-minimizing frame seeded from the first segment.
    #[default]
    RotationMinimizing,
}

/// A 3D sweep path.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Path3 {
    /// A polyline path with a deterministic frame policy.
    Polyline {
        /// Path points, at least two, consecutive points distinct.
        points: Vec<[f64; 3]>,
        /// How profile frames orient along the path.
        frame: FramePolicy,
    },
}

/// Parametric primitive specifications, evaluated via `exedra_primitives`.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PrimitiveSpec {
    /// An axis-aligned box with the given extents, minimum corner at the
    /// origin.
    Box {
        /// Extents along x, y, z; all positive.
        size: [f64; 3],
    },
    /// A capped cylinder along +Z, base center at the origin.
    Cylinder {
        /// Radius; positive.
        radius: f64,
        /// Height along +Z; positive.
        height: f64,
        /// Number of side segments; at least 3.
        segments: u32,
    },
}

/// CSG operation over two or more operands.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CsgOp {
    /// Union of all operands.
    Union,
    /// First operand minus the union of the rest (the catalog difference
    /// convention).
    Difference,
    /// Intersection of all operands.
    Intersection,
}

/// One constructive node.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NodeKind {
    /// Extrude a profile from its local XY plane along local +Z.
    Extrude {
        /// The profile to extrude.
        profile: ProfileId,
        /// Placement of the profile's local frame in 3D.
        placement: Placement3,
        /// Extrusion distance along local +Z; positive and finite.
        height: f64,
        /// Which caps to close.
        caps: CapMode,
    },
    /// Revolve a profile about its local Y axis.
    ///
    /// The profile lives in its local XZ half-plane (profile x maps to
    /// radius, profile y to height along the axis).
    Revolve {
        /// The profile to revolve; must lie at nonnegative radius.
        profile: ProfileId,
        /// Placement of the revolution frame in 3D.
        placement: Placement3,
        /// Sweep angle in radians, in `(0, 2*pi]`.
        sweep: f64,
        /// Which end caps to close for partial sweeps.
        caps: CapMode,
    },
    /// Loft between placed sections.
    Loft {
        /// Sections in order; at least two.
        sections: Vec<(Placement3, ProfileId)>,
        /// Correspondence policy.
        policy: LoftPolicy,
        /// Which end caps to close.
        caps: CapMode,
    },
    /// Sweep a profile along a path.
    Sweep {
        /// The swept profile.
        profile: ProfileId,
        /// The path to sweep along.
        path: Path3,
        /// Which end caps to close.
        caps: CapMode,
    },
    /// A single-sided planar face (no thickness).
    PlanarFace {
        /// The face profile.
        profile: ProfileId,
        /// Placement of the profile plane in 3D.
        placement: Placement3,
    },
    /// A parametric primitive.
    Primitive {
        /// Primitive parameters.
        spec: PrimitiveSpec,
        /// Placement of the primitive's local frame.
        placement: Placement3,
    },
    /// Boolean combination of two or more child bodies.
    Csg {
        /// The operation.
        op: CsgOp,
        /// Operand nodes, in order; at least two.
        operands: Vec<NodeId>,
    },
    /// An affine transform of a child (rigid, scaled, or sheared;
    /// evaluation reports non-rigid transforms explicitly).
    Transform {
        /// The transformed child.
        child: NodeId,
        /// The transform matrix.
        xf: Placement3,
    },
    /// Mirror a child across a plane (winding is corrected at evaluation).
    Mirror {
        /// The mirrored child.
        child: NodeId,
        /// The mirror plane.
        plane: Plane3,
    },
    /// Instance of an earlier node's geometry under a placement.
    ///
    /// Instances never duplicate tessellation: evaluation reuses the
    /// definition's body and records the placement.
    Instance {
        /// The instanced definition.
        of: NodeId,
        /// Instance placement.
        placement: Placement3,
    },
    /// A grouping of child nodes with no geometry of its own.
    Group {
        /// The grouped children.
        children: Vec<NodeId>,
    },
    /// Reserved: plane-split stretch (cut by plane, translate one half,
    /// stitch). Present in the schema from day one so frontends can encode
    /// intent; evaluation reports it unimplemented until the boolean
    /// pipeline's split/stitch stages land.
    Stretch {
        /// The stretched child.
        child: NodeId,
        /// The cutting plane.
        plane: Plane3,
        /// Signed stretch distance along the plane normal.
        length: f64,
    },
}

/// One node: kind plus optional source reference and material slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What this node constructs.
    pub kind: NodeKind,
    /// Opaque frontend-assigned source reference.
    pub source: Option<SourceId>,
    /// Material slot binding, inherited by descendants that lack one.
    pub material: Option<SlotId>,
}

/// A frozen constructive recipe.
///
/// Immutable after [`RecipeBuilder::finish`]; all lookups are index-based
/// and deterministic.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    profiles: Vec<Profile2>,
    nodes: Vec<Node>,
    root: NodeId,
    sources: Vec<String>,
    slots: Vec<String>,
    fingerprints: Vec<Fingerprint>,
}

impl Recipe {
    /// The root node.
    #[must_use]
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// All nodes, children before parents.
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Looks up a node.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    /// Looks up a profile.
    #[must_use]
    pub fn profile(&self, id: ProfileId) -> &Profile2 {
        &self.profiles[id.0 as usize]
    }

    /// All profiles.
    #[must_use]
    pub fn profiles(&self) -> &[Profile2] {
        &self.profiles
    }

    /// The interned source string behind a [`SourceId`].
    #[must_use]
    pub fn source(&self, id: SourceId) -> &str {
        &self.sources[id.0 as usize]
    }

    /// All interned source strings, in intern order.
    #[must_use]
    pub fn sources(&self) -> &[String] {
        &self.sources
    }

    /// All interned slot names, in intern order.
    #[must_use]
    pub fn slots(&self) -> &[String] {
        &self.slots
    }

    /// The interned slot name behind a [`SlotId`].
    #[must_use]
    pub fn slot(&self, id: SlotId) -> &str {
        &self.slots[id.0 as usize]
    }

    /// A node's content fingerprint (children and schema version included).
    #[must_use]
    pub fn fingerprint(&self, id: NodeId) -> Fingerprint {
        self.fingerprints[id.0 as usize]
    }

    /// The whole recipe's fingerprint: the root's.
    #[must_use]
    pub fn recipe_fingerprint(&self) -> Fingerprint {
        self.fingerprint(self.root)
    }
}

/// Typed recipe-construction failure.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecipeError {
    /// A referenced profile does not exist.
    UnknownProfile {
        /// The offending reference.
        profile: u32,
    },
    /// A referenced child node does not exist (children must be added
    /// before their parents).
    UnknownNode {
        /// The offending reference.
        node: u32,
    },
    /// A numeric parameter is non-finite or out of range.
    InvalidParameter {
        /// Which parameter, as a stable diagnostic name.
        what: &'static str,
    },
    /// A node needs more operands/children/sections than provided.
    TooFewOperands {
        /// Which node kind, as a stable diagnostic name.
        what: &'static str,
        /// How many were provided.
        count: usize,
    },
    /// The recipe has no nodes.
    Empty,
}

impl core::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownProfile { profile } => write!(f, "unknown profile id {profile}"),
            Self::UnknownNode { node } => write!(f, "unknown node id {node}"),
            Self::InvalidParameter { what } => write!(f, "invalid parameter: {what}"),
            Self::TooFewOperands { what, count } => {
                write!(f, "{what} needs more operands, got {count}")
            }
            Self::Empty => write!(f, "recipe has no nodes"),
        }
    }
}

impl core::error::Error for RecipeError {}

/// Append-only builder producing frozen [`Recipe`]s.
#[derive(Debug, Default)]
pub struct RecipeBuilder {
    profiles: Vec<Profile2>,
    nodes: Vec<Node>,
    sources: Vec<String>,
    slots: Vec<String>,
    pending_source: Option<SourceId>,
    pending_material: Option<SlotId>,
}

impl RecipeBuilder {
    /// Creates an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a validated profile.
    pub fn add_profile(&mut self, profile: Profile2) -> ProfileId {
        self.profiles.push(profile);
        ProfileId(len_u32(self.profiles.len()) - 1)
    }

    /// Interns a source reference string.
    pub fn source_ref(&mut self, source: &str) -> SourceId {
        intern(&mut self.sources, source, SourceId)
    }

    /// Interns a material-slot name.
    pub fn material_slot(&mut self, name: &str) -> SlotId {
        intern(&mut self.slots, name, SlotId)
    }

    /// Attaches a source reference to the next node added.
    pub fn with_source(&mut self, source: SourceId) -> &mut Self {
        self.pending_source = Some(source);
        self
    }

    /// Attaches a material slot to the next node added.
    pub fn with_material(&mut self, slot: SlotId) -> &mut Self {
        self.pending_material = Some(slot);
        self
    }

    /// Adds a node after validating its parameters and references.
    ///
    /// # Errors
    ///
    /// Returns a typed [`RecipeError`]; the builder is unchanged on error.
    pub fn add(&mut self, kind: NodeKind) -> Result<NodeId, RecipeError> {
        self.validate_kind(&kind)?;
        let node = Node {
            kind,
            source: self.pending_source.take(),
            material: self.pending_material.take(),
        };
        self.nodes.push(node);
        Ok(NodeId(len_u32(self.nodes.len()) - 1))
    }

    /// Freezes the recipe with `root` as its result.
    ///
    /// # Errors
    ///
    /// Fails when the recipe is empty or `root` is out of range.
    pub fn finish(self, root: NodeId) -> Result<Recipe, RecipeError> {
        if self.nodes.is_empty() {
            return Err(RecipeError::Empty);
        }
        if root.0 as usize >= self.nodes.len() {
            return Err(RecipeError::UnknownNode { node: root.0 });
        }
        let fingerprints = compute_fingerprints(&self.profiles, &self.nodes, &self.sources);
        Ok(Recipe {
            profiles: self.profiles,
            nodes: self.nodes,
            root,
            sources: self.sources,
            slots: self.slots,
            fingerprints,
        })
    }

    fn check_profile(&self, id: ProfileId) -> Result<(), RecipeError> {
        if (id.0 as usize) < self.profiles.len() {
            Ok(())
        } else {
            Err(RecipeError::UnknownProfile { profile: id.0 })
        }
    }

    fn check_node(&self, id: NodeId) -> Result<(), RecipeError> {
        if (id.0 as usize) < self.nodes.len() {
            Ok(())
        } else {
            Err(RecipeError::UnknownNode { node: id.0 })
        }
    }

    fn check_placement(&self, p: &Placement3) -> Result<(), RecipeError> {
        if p.is_finite() {
            Ok(())
        } else {
            Err(RecipeError::InvalidParameter { what: "placement" })
        }
    }

    fn check_plane(&self, plane: &Plane3) -> Result<(), RecipeError> {
        let n = plane.normal;
        let finite = n.iter().all(|v| v.is_finite()) && plane.distance.is_finite();
        let nonzero = n[0] != 0.0 || n[1] != 0.0 || n[2] != 0.0;
        if finite && nonzero {
            Ok(())
        } else {
            Err(RecipeError::InvalidParameter { what: "plane" })
        }
    }

    fn validate_kind(&self, kind: &NodeKind) -> Result<(), RecipeError> {
        match kind {
            NodeKind::Extrude {
                profile,
                placement,
                height,
                caps: _,
            } => {
                self.check_profile(*profile)?;
                self.check_placement(placement)?;
                if !(height.is_finite() && *height > 0.0) {
                    return Err(RecipeError::InvalidParameter { what: "height" });
                }
                Ok(())
            }
            NodeKind::Revolve {
                profile,
                placement,
                sweep,
                caps: _,
            } => {
                self.check_profile(*profile)?;
                self.check_placement(placement)?;
                if !(sweep.is_finite() && *sweep > 0.0 && *sweep <= core::f64::consts::TAU) {
                    return Err(RecipeError::InvalidParameter { what: "sweep" });
                }
                Ok(())
            }
            NodeKind::Loft {
                sections,
                policy: _,
                caps: _,
            } => {
                if sections.len() < 2 {
                    return Err(RecipeError::TooFewOperands {
                        what: "loft sections",
                        count: sections.len(),
                    });
                }
                for (placement, profile) in sections {
                    self.check_placement(placement)?;
                    self.check_profile(*profile)?;
                }
                Ok(())
            }
            NodeKind::Sweep {
                profile,
                path,
                caps: _,
            } => {
                self.check_profile(*profile)?;
                match path {
                    Path3::Polyline { points, frame: _ } => {
                        if points.len() < 2 {
                            return Err(RecipeError::TooFewOperands {
                                what: "sweep path points",
                                count: points.len(),
                            });
                        }
                        if points.iter().any(|p| p.iter().any(|v| !v.is_finite())) {
                            return Err(RecipeError::InvalidParameter { what: "sweep path" });
                        }
                        if points.windows(2).any(|w| w[0] == w[1]) {
                            return Err(RecipeError::InvalidParameter {
                                what: "sweep path duplicate point",
                            });
                        }
                        Ok(())
                    }
                }
            }
            NodeKind::PlanarFace { profile, placement } => {
                self.check_profile(*profile)?;
                self.check_placement(placement)
            }
            NodeKind::Primitive { spec, placement } => {
                self.check_placement(placement)?;
                match spec {
                    PrimitiveSpec::Box { size } => {
                        if size.iter().all(|v| v.is_finite() && *v > 0.0) {
                            Ok(())
                        } else {
                            Err(RecipeError::InvalidParameter { what: "box size" })
                        }
                    }
                    PrimitiveSpec::Cylinder {
                        radius,
                        height,
                        segments,
                    } => {
                        let ok = radius.is_finite()
                            && *radius > 0.0
                            && height.is_finite()
                            && *height > 0.0
                            && *segments >= 3;
                        if ok {
                            Ok(())
                        } else {
                            Err(RecipeError::InvalidParameter { what: "cylinder" })
                        }
                    }
                }
            }
            NodeKind::Csg { op: _, operands } => {
                if operands.len() < 2 {
                    return Err(RecipeError::TooFewOperands {
                        what: "csg operands",
                        count: operands.len(),
                    });
                }
                for operand in operands {
                    self.check_node(*operand)?;
                }
                Ok(())
            }
            NodeKind::Transform { child, xf } => {
                self.check_node(*child)?;
                self.check_placement(xf)
            }
            NodeKind::Mirror { child, plane } => {
                self.check_node(*child)?;
                self.check_plane(plane)
            }
            NodeKind::Instance { of, placement } => {
                self.check_node(*of)?;
                self.check_placement(placement)
            }
            NodeKind::Group { children } => {
                if children.is_empty() {
                    return Err(RecipeError::TooFewOperands {
                        what: "group children",
                        count: 0,
                    });
                }
                for child in children {
                    self.check_node(*child)?;
                }
                Ok(())
            }
            NodeKind::Stretch {
                child,
                plane,
                length,
            } => {
                self.check_node(*child)?;
                self.check_plane(plane)?;
                if length.is_finite() && *length != 0.0 {
                    Ok(())
                } else {
                    Err(RecipeError::InvalidParameter {
                        what: "stretch length",
                    })
                }
            }
        }
    }
}

fn intern<T>(table: &mut Vec<String>, value: &str, wrap: impl Fn(u32) -> T) -> T {
    if let Some(index) = table.iter().position(|s| s == value) {
        wrap(len_u32(index + 1) - 1)
    } else {
        table.push(String::from(value));
        wrap(len_u32(table.len()) - 1)
    }
}

// --- Fingerprints -----------------------------------------------------------

const FNV128_OFFSET: u128 = 0x6C62_272E_07BB_0142_62B8_2175_6295_C58D;
const FNV128_PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013B;

fn fnv128(bytes: &[u8], seed: u128) -> u128 {
    let mut hash = seed;
    for &b in bytes {
        hash ^= u128::from(b);
        hash = hash.wrapping_mul(FNV128_PRIME);
    }
    hash
}

fn compute_fingerprints(
    profiles: &[Profile2],
    nodes: &[Node],
    sources: &[String],
) -> Vec<Fingerprint> {
    // Profile hashes first (content-addressed, schema-stamped).
    let profile_hashes: Vec<u128> = profiles
        .iter()
        .map(|p| {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&crate::EVAL_SCHEMA_VERSION.to_le_bytes());
            p.canon_bytes(&mut bytes);
            fnv128(&bytes, FNV128_OFFSET)
        })
        .collect();

    let mut out: Vec<Fingerprint> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&crate::EVAL_SCHEMA_VERSION.to_le_bytes());
        node_canon_bytes(node, &profile_hashes, &out, sources, &mut bytes);
        out.push(Fingerprint(fnv128(&bytes, FNV128_OFFSET)));
    }
    out
}

fn put_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_bits().to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u128(out: &mut Vec<u8>, v: u128) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_placement(out: &mut Vec<u8>, p: &Placement3) {
    for row in &p.rows {
        for &v in row {
            put_f64(out, v);
        }
    }
}

fn put_plane(out: &mut Vec<u8>, plane: &Plane3) {
    for &v in &plane.normal {
        put_f64(out, v);
    }
    put_f64(out, plane.distance);
}

fn put_caps(out: &mut Vec<u8>, caps: CapMode) {
    out.push(match caps {
        CapMode::Both => 0,
        CapMode::Start => 1,
        CapMode::End => 2,
        CapMode::None => 3,
    });
}

/// Canonical bytes of one node, referencing children by fingerprint (so the
/// hash is a Merkle hash) and profiles by content hash.
fn node_canon_bytes(
    node: &Node,
    profile_hashes: &[u128],
    node_fingerprints: &[Fingerprint],
    sources: &[String],
    out: &mut Vec<u8>,
) {
    let child = |out: &mut Vec<u8>, id: NodeId| {
        put_u128(out, node_fingerprints[id.0 as usize].0);
    };
    match &node.kind {
        NodeKind::Extrude {
            profile,
            placement,
            height,
            caps,
        } => {
            out.push(0);
            put_u128(out, profile_hashes[profile.0 as usize]);
            put_placement(out, placement);
            put_f64(out, *height);
            put_caps(out, *caps);
        }
        NodeKind::Revolve {
            profile,
            placement,
            sweep,
            caps,
        } => {
            out.push(1);
            put_u128(out, profile_hashes[profile.0 as usize]);
            put_placement(out, placement);
            put_f64(out, *sweep);
            put_caps(out, *caps);
        }
        NodeKind::Loft {
            sections,
            policy,
            caps,
        } => {
            out.push(2);
            put_u32(out, len_u32(sections.len()));
            for (placement, profile) in sections {
                put_placement(out, placement);
                put_u128(out, profile_hashes[profile.0 as usize]);
            }
            out.push(match policy {
                LoftPolicy::Ruled => 0,
            });
            put_caps(out, *caps);
        }
        NodeKind::Sweep {
            profile,
            path,
            caps,
        } => {
            out.push(3);
            put_u128(out, profile_hashes[profile.0 as usize]);
            match path {
                Path3::Polyline { points, frame } => {
                    out.push(0);
                    put_u32(out, len_u32(points.len()));
                    for p in points {
                        for &v in p {
                            put_f64(out, v);
                        }
                    }
                    out.push(match frame {
                        FramePolicy::RotationMinimizing => 0,
                    });
                }
            }
            put_caps(out, *caps);
        }
        NodeKind::PlanarFace { profile, placement } => {
            out.push(4);
            put_u128(out, profile_hashes[profile.0 as usize]);
            put_placement(out, placement);
        }
        NodeKind::Primitive { spec, placement } => {
            out.push(5);
            match spec {
                PrimitiveSpec::Box { size } => {
                    out.push(0);
                    for &v in size {
                        put_f64(out, v);
                    }
                }
                PrimitiveSpec::Cylinder {
                    radius,
                    height,
                    segments,
                } => {
                    out.push(1);
                    put_f64(out, *radius);
                    put_f64(out, *height);
                    put_u32(out, *segments);
                }
            }
            put_placement(out, placement);
        }
        NodeKind::Csg { op, operands } => {
            out.push(6);
            out.push(match op {
                CsgOp::Union => 0,
                CsgOp::Difference => 1,
                CsgOp::Intersection => 2,
            });
            put_u32(out, len_u32(operands.len()));
            for operand in operands {
                child(out, *operand);
            }
        }
        NodeKind::Transform { child: c, xf } => {
            out.push(7);
            child(out, *c);
            put_placement(out, xf);
        }
        NodeKind::Mirror { child: c, plane } => {
            out.push(8);
            child(out, *c);
            put_plane(out, plane);
        }
        NodeKind::Instance { of, placement } => {
            out.push(9);
            child(out, *of);
            put_placement(out, placement);
        }
        NodeKind::Group { children } => {
            out.push(10);
            put_u32(out, len_u32(children.len()));
            for c in children {
                child(out, *c);
            }
        }
        NodeKind::Stretch {
            child: c,
            plane,
            length,
        } => {
            out.push(11);
            child(out, *c);
            put_plane(out, plane);
            put_f64(out, *length);
        }
    }
    // Source references participate in identity: two structurally equal
    // nodes with different source labels are different provenance-wise but
    // evaluate identically, so sources are hashed into a separate trailing
    // section rather than mixed into geometry bytes. (Kept in the same
    // fingerprint: caches key on geometry + provenance to keep source maps
    // reusable verbatim.)
    match node.source {
        None => out.push(0),
        Some(id) => {
            out.push(1);
            let s = sources[id.0 as usize].as_bytes();
            put_u32(out, len_u32(s.len()));
            out.extend_from_slice(s);
        }
    }
    match node.material {
        None => out.push(0),
        Some(SlotId(slot)) => {
            out.push(1);
            put_u32(out, slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use alloc::vec;

    fn simple_recipe(height: f64) -> Recipe {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
        let src = b.source_ref("test:panel");
        let node = b
            .with_source(src)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height,
                caps: CapMode::Both,
            })
            .expect("valid extrude");
        b.finish(node).expect("valid recipe")
    }

    #[test]
    fn builder_validates_references_and_parameters() {
        let mut b = RecipeBuilder::new();
        assert_eq!(
            b.add(NodeKind::Extrude {
                profile: ProfileId(0),
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            }),
            Err(RecipeError::UnknownProfile { profile: 0 })
        );
        let profile = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        assert_eq!(
            b.add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: -1.0,
                caps: CapMode::Both,
            }),
            Err(RecipeError::InvalidParameter { what: "height" })
        );
        assert_eq!(
            b.add(NodeKind::Csg {
                op: CsgOp::Union,
                operands: vec![NodeId(5), NodeId(6)],
            }),
            Err(RecipeError::UnknownNode { node: 5 })
        );
        // Forward references are impossible: children must already exist.
        let node = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        assert_eq!(
            b.add(NodeKind::Instance {
                of: NodeId(node.0 + 1),
                placement: Placement3::IDENTITY,
            }),
            Err(RecipeError::UnknownNode { node: 1 })
        );
    }

    #[test]
    fn fingerprints_are_stable_and_content_sensitive() {
        let a = simple_recipe(3.0);
        let b = simple_recipe(3.0);
        assert_eq!(a.recipe_fingerprint(), b.recipe_fingerprint());
        let c = simple_recipe(4.0);
        assert_ne!(a.recipe_fingerprint(), c.recipe_fingerprint());
    }

    #[test]
    fn fingerprints_are_merkle() {
        // Wrapping the same child in the same transform twice gives equal
        // hashes; changing the child changes the parent hash.
        let build = |height: f64| {
            let mut b = RecipeBuilder::new();
            let profile = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
            let child = b
                .add(NodeKind::Extrude {
                    profile,
                    placement: Placement3::IDENTITY,
                    height,
                    caps: CapMode::Both,
                })
                .expect("valid");
            let parent = b
                .add(NodeKind::Transform {
                    child,
                    xf: Placement3::translate(1.0, 0.0, 0.0),
                })
                .expect("valid");
            b.finish(parent).expect("valid")
        };
        assert_eq!(
            build(2.0).recipe_fingerprint(),
            build(2.0).recipe_fingerprint()
        );
        assert_ne!(
            build(2.0).recipe_fingerprint(),
            build(2.5).recipe_fingerprint()
        );
    }

    #[test]
    fn source_and_material_participate_in_identity() {
        let bare = {
            let mut b = RecipeBuilder::new();
            let profile = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
            let n = b
                .add(NodeKind::Extrude {
                    profile,
                    placement: Placement3::IDENTITY,
                    height: 3.0,
                    caps: CapMode::Both,
                })
                .expect("valid");
            b.finish(n).expect("valid")
        };
        let sourced = simple_recipe(3.0);
        assert_ne!(bare.recipe_fingerprint(), sourced.recipe_fingerprint());
    }

    #[test]
    fn interning_dedups() {
        let mut b = RecipeBuilder::new();
        let s1 = b.source_ref("x:one");
        let s2 = b.source_ref("x:one");
        let s3 = b.source_ref("x:two");
        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
        let m1 = b.material_slot("front");
        let m2 = b.material_slot("front");
        assert_eq!(m1, m2);
    }

    #[test]
    fn stretch_is_reserved_but_buildable() {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
        let body = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let stretch = b
            .add(NodeKind::Stretch {
                child: body,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 1.0,
                },
                length: 0.5,
            })
            .expect("stretch encodes");
        assert!(b.finish(stretch).is_ok());
    }

    #[test]
    fn placement_constructors() {
        let p = Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, 1.0, 2.0, 3.0);
        // Rotating +X by 90 degrees about Z gives +Y.
        let x = [p.rows[0][0], p.rows[1][0], p.rows[2][0]];
        assert!(x[0].abs() < 1e-12 && (x[1] - 1.0).abs() < 1e-12);
        let e = Placement3::euler_xyz_then_translate(
            0.0,
            0.0,
            core::f64::consts::FRAC_PI_2,
            [1.0, 2.0, 3.0],
        );
        assert!((e.rows[1][0] - 1.0).abs() < 1e-12);

        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        assert_eq!(
            b.add(NodeKind::Extrude {
                profile,
                placement: Placement3 {
                    rows: [[f64::NAN; 4]; 3]
                },
                height: 1.0,
                caps: CapMode::Both,
            }),
            Err(RecipeError::InvalidParameter { what: "placement" })
        );
    }

    #[test]
    fn golden_recipe_fingerprint() {
        // Compatibility pin for canonical encoding + FNV-128. A change here
        // is a schema change: bump EVAL_SCHEMA_VERSION deliberately.
        let r = simple_recipe(3.0);
        assert_eq!(
            r.recipe_fingerprint().0,
            0x6A9F_0848_3026_C621_47C5_0EC0_D654_7A3B,
            "canonical encoding changed; bump EVAL_SCHEMA_VERSION"
        );
    }
}
