// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `exedra-recipe-v1`: the JSON interchange schema for external frontends.
//!
//! The schema is defined by dedicated DTO types in this module — never by
//! deriving on the internal IR — so internal refactors cannot silently
//! change the wire format. Deserialization rebuilds through
//! [`RecipeBuilder`], re-running every validation; round-trip fingerprint
//! equality is the correctness oracle.
//!
//! Stability policy (ADR-0003):
//! - The header is `{"format": "exedra-recipe", "version": 1}`; readers
//!   reject other formats/versions with a typed error.
//! - Evolution within version 1 is additive-only: new optional fields and
//!   new node kinds may appear, existing fields never change meaning.
//!   Unknown *fields* are ignored (additive tolerance); unknown node
//!   *kinds* are hard errors — a recipe is executable content, and
//!   skipping an unknown operation would silently change geometry.
//! - Floats are plain JSON numbers; serde's shortest-round-trip rendering
//!   re-parses to the identical `f64` bits, so content fingerprints
//!   survive the wire exactly.
//! - Curve segments serialize as explicit records, insulating the format
//!   from kurbo's own types and versions.

#![cfg(feature = "serde")]

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::ir::{
    CapMode, CsgOp, FramePolicy, LoftPolicy, NodeId, NodeKind, Path3, Placement3, Plane3,
    PrimitiveSpec, ProfileId, Recipe, RecipeBuilder, RecipeError, SlotId, SourceId,
};
use crate::profile::{Loop2, Profile2, ProfileError, Seg2, SegKind, SegTag};

/// Format name in the header.
pub const FORMAT: &str = "exedra-recipe";
/// Format version this module reads and writes.
pub const VERSION: u32 = 1;

/// Top-level interchange document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipeDto {
    /// Format name; always [`FORMAT`].
    pub format: String,
    /// Format version; always [`VERSION`].
    pub version: u32,
    /// Interned source-reference strings, in id order.
    pub sources: Vec<String>,
    /// Interned material-slot names, in id order.
    pub slots: Vec<String>,
    /// Interned curve-policy references, in id order.
    #[serde(default)]
    pub policies: Vec<String>,
    /// Imported meshes, in id order.
    #[serde(default)]
    pub imports: Vec<MeshDto>,
    /// Profiles, in id order.
    pub profiles: Vec<ProfileDto>,
    /// Nodes, children before parents.
    pub nodes: Vec<NodeDto>,
    /// Root node index.
    pub root: u32,
}

/// One profile: outer loop plus holes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileDto {
    /// Outer loop (counter-clockwise).
    pub outer: Vec<SegDto>,
    /// Hole loops (clockwise).
    #[serde(default)]
    pub holes: Vec<Vec<SegDto>>,
}

/// One profile segment as an explicit record.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SegDto {
    /// Straight chord.
    Line {
        /// Endpoint.
        to: [f64; 2],
        /// Optional provenance tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u32>,
    },
    /// Circular arc (bulge parameterization, `tan(sweep/4)`).
    Arc {
        /// Endpoint.
        to: [f64; 2],
        /// Bulge value.
        bulge: f64,
        /// Optional provenance tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u32>,
    },
    /// Cubic Bézier.
    Cubic {
        /// Endpoint.
        to: [f64; 2],
        /// First control point.
        c1: [f64; 2],
        /// Second control point.
        c2: [f64; 2],
        /// Optional provenance tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u32>,
    },
    /// Policy-defined segment: an opaque policy reference plus its
    /// concrete realization.
    Policy {
        /// Endpoint.
        to: [f64; 2],
        /// Index into `policies`.
        policy: u32,
        /// The realization.
        realized: RealizedDto,
        /// Optional provenance tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u32>,
    },
}

/// Concrete realization of a policy segment (endpoint lives on the parent).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RealizedDto {
    /// Straight chord.
    Line,
    /// Circular arc by bulge.
    Arc {
        /// Bulge value.
        bulge: f64,
    },
    /// Cubic Bézier.
    Cubic {
        /// First control point.
        c1: [f64; 2],
        /// Second control point.
        c2: [f64; 2],
    },
}

/// An imported mesh: f32 vertex positions plus polygon face loops.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeshDto {
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Face loops as vertex indices.
    pub faces: Vec<Vec<u32>>,
}

/// One node record: kind payload plus optional source/material bindings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeDto {
    /// The node kind payload.
    #[serde(flatten)]
    pub kind: NodeKindDto,
    /// Optional index into `sources`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<u32>,
    /// Optional index into `slots`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<u32>,
    /// Optional index into `sources`: a spec-issue citation marking the
    /// node's specification as contradictory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<u32>,
}

/// 3x4 row-major placement.
pub type PlacementDto = [[f64; 4]; 3];

/// Plane as `[nx, ny, nz, d]` (`dot(n, p) = d`).
pub type PlaneDto = [f64; 4];

/// Node kind payloads. Unknown kinds are deserialization errors by design.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NodeKindDto {
    /// Extrusion along local +Z.
    Extrude {
        /// Profile index.
        profile: u32,
        /// Placement.
        placement: PlacementDto,
        /// Extrusion height.
        height: f64,
        /// Cap mode: "both" | "start" | "end" | "none".
        caps: String,
    },
    /// Revolution about local Y.
    Revolve {
        /// Profile index.
        profile: u32,
        /// Placement.
        placement: PlacementDto,
        /// Sweep angle in radians.
        sweep: f64,
        /// Cap mode.
        caps: String,
    },
    /// Ruled loft between placed sections.
    Loft {
        /// Sections as `(placement, profile)` pairs.
        sections: Vec<(PlacementDto, u32)>,
        /// Cap mode.
        caps: String,
    },
    /// Sweep along a polyline path.
    Sweep {
        /// Profile index.
        profile: u32,
        /// Path points.
        points: Vec<[f64; 3]>,
        /// Cap mode.
        caps: String,
    },
    /// Single-sided planar face.
    PlanarFace {
        /// Profile index.
        profile: u32,
        /// Placement.
        placement: PlacementDto,
    },
    /// Axis-aligned box primitive.
    Box {
        /// Extents.
        size: [f64; 3],
        /// Placement.
        placement: PlacementDto,
    },
    /// Capped cylinder primitive.
    Cylinder {
        /// Radius.
        radius: f64,
        /// Height.
        height: f64,
        /// Side segments.
        segments: u32,
        /// Placement.
        placement: PlacementDto,
    },
    /// Boolean combination.
    Csg {
        /// Operation: "union" | "difference" | "intersection".
        csg: String,
        /// Operand node indices.
        operands: Vec<u32>,
    },
    /// Affine transform of a child.
    Transform {
        /// Child node index.
        child: u32,
        /// Placement.
        placement: PlacementDto,
    },
    /// Mirror across a plane.
    Mirror {
        /// Child node index.
        child: u32,
        /// Mirror plane.
        plane: PlaneDto,
    },
    /// Instance of an earlier node.
    Instance {
        /// Definition node index.
        of: u32,
        /// Instance placement.
        placement: PlacementDto,
    },
    /// Grouping node.
    Group {
        /// Child node indices.
        children: Vec<u32>,
    },
    /// Opaque imported mesh leaf.
    MeshImport {
        /// Index into `imports`.
        import: u32,
        /// Placement.
        placement: PlacementDto,
    },
    /// Reserved plane-split stretch.
    Stretch {
        /// Child node index.
        child: u32,
        /// Cutting plane.
        plane: PlaneDto,
        /// Signed stretch length.
        length: f64,
    },
}

/// Typed interchange failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum InterchangeError {
    /// Wrong format name or unsupported version.
    UnsupportedFormat,
    /// An enum-like string field held an unknown value.
    UnknownValue {
        /// The field, as a stable name.
        field: &'static str,
    },
    /// Profile validation failed on rebuild.
    Profile(ProfileError),
    /// Recipe validation failed on rebuild.
    Recipe(RecipeError),
    /// An imported mesh failed to rebuild.
    InvalidImport,
}

impl core::fmt::Display for InterchangeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedFormat => write!(f, "unsupported format or version"),
            Self::UnknownValue { field } => write!(f, "unknown value in field {field}"),
            Self::Profile(e) => write!(f, "profile validation failed: {e}"),
            Self::Recipe(e) => write!(f, "recipe validation failed: {e}"),
            Self::InvalidImport => write!(f, "imported mesh failed to rebuild"),
        }
    }
}

impl core::error::Error for InterchangeError {}

fn caps_name(caps: CapMode) -> String {
    String::from(match caps {
        CapMode::Both => "both",
        CapMode::Start => "start",
        CapMode::End => "end",
        CapMode::None => "none",
    })
}

fn caps_value(name: &str) -> Result<CapMode, InterchangeError> {
    match name {
        "both" => Ok(CapMode::Both),
        "start" => Ok(CapMode::Start),
        "end" => Ok(CapMode::End),
        "none" => Ok(CapMode::None),
        _ => Err(InterchangeError::UnknownValue { field: "caps" }),
    }
}

fn placement_dto(p: &Placement3) -> PlacementDto {
    p.rows
}

fn placement_value(rows: PlacementDto) -> Placement3 {
    Placement3 { rows }
}

fn plane_dto(p: &Plane3) -> PlaneDto {
    [p.normal[0], p.normal[1], p.normal[2], p.distance]
}

fn plane_value(v: PlaneDto) -> Plane3 {
    Plane3 {
        normal: [v[0], v[1], v[2]],
        distance: v[3],
    }
}

fn loop_dto(source: &Loop2) -> Vec<SegDto> {
    source
        .segs()
        .iter()
        .map(|seg| {
            let to = [seg.to.x, seg.to.y];
            let tag = seg.tag.map(|SegTag(t)| t);
            match &seg.kind {
                SegKind::Line => SegDto::Line { to, tag },
                SegKind::Arc { bulge } => SegDto::Arc {
                    to,
                    bulge: *bulge,
                    tag,
                },
                SegKind::Cubic { c1, c2 } => SegDto::Cubic {
                    to,
                    c1: [c1.x, c1.y],
                    c2: [c2.x, c2.y],
                    tag,
                },
                SegKind::PolicyTo { policy, realized } => SegDto::Policy {
                    to,
                    policy: policy.0,
                    realized: match realized.as_ref() {
                        SegKind::Line => RealizedDto::Line,
                        SegKind::Arc { bulge } => RealizedDto::Arc { bulge: *bulge },
                        SegKind::Cubic { c1, c2 } => RealizedDto::Cubic {
                            c1: [c1.x, c1.y],
                            c2: [c2.x, c2.y],
                        },
                        SegKind::PolicyTo { .. } => {
                            unreachable!("nested policies are rejected at validation")
                        }
                    },
                    tag,
                },
            }
        })
        .collect()
}

fn loop_value(segs: &[SegDto]) -> Result<Loop2, InterchangeError> {
    let segs = segs
        .iter()
        .map(|dto| {
            let (seg, tag) = match dto {
                SegDto::Line { to, tag } => (Seg2::line((to[0], to[1])), tag),
                SegDto::Arc { to, bulge, tag } => (Seg2::arc((to[0], to[1]), *bulge), tag),
                SegDto::Cubic { to, c1, c2, tag } => (
                    Seg2::cubic((to[0], to[1]), (c1[0], c1[1]), (c2[0], c2[1])),
                    tag,
                ),
                SegDto::Policy {
                    to,
                    policy,
                    realized,
                    tag,
                } => {
                    let inner = match realized {
                        RealizedDto::Line => SegKind::Line,
                        RealizedDto::Arc { bulge } => SegKind::Arc { bulge: *bulge },
                        RealizedDto::Cubic { c1, c2 } => SegKind::Cubic {
                            c1: kurbo::Point::new(c1[0], c1[1]),
                            c2: kurbo::Point::new(c2[0], c2[1]),
                        },
                    };
                    (
                        Seg2::policy((to[0], to[1]), crate::ir::PolicyId(*policy), inner),
                        tag,
                    )
                }
            };
            match tag {
                Some(t) => seg.tagged(SegTag(*t)),
                None => seg,
            }
        })
        .collect();
    Loop2::new(segs).map_err(InterchangeError::Profile)
}

/// Converts a recipe into its interchange document.
#[must_use]
pub fn to_dto(recipe: &Recipe) -> RecipeDto {
    let profiles = recipe
        .profiles()
        .iter()
        .map(|profile| ProfileDto {
            outer: loop_dto(profile.outer()),
            holes: profile.holes().iter().map(loop_dto).collect(),
        })
        .collect();
    let nodes = recipe
        .nodes()
        .iter()
        .map(|node| NodeDto {
            kind: kind_dto(&node.kind),
            source: node.source.map(|SourceId(s)| s),
            material: node.material.map(|SlotId(m)| m),
            issue: node.issue.map(|SourceId(s)| s),
        })
        .collect();
    RecipeDto {
        format: String::from(FORMAT),
        version: VERSION,
        sources: recipe.sources().to_vec(),
        slots: recipe.slots().to_vec(),
        policies: recipe.policies().to_vec(),
        imports: recipe
            .imports()
            .iter()
            .map(|mesh| MeshDto {
                positions: mesh
                    .vertices()
                    .filter_map(|v| mesh.vertex_position(v))
                    .copied()
                    .collect(),
                faces: mesh
                    .faces()
                    .map(|face| crate::ir::canonical_face_loop_pub(mesh, face))
                    .collect(),
            })
            .collect(),
        profiles,
        nodes,
        root: recipe.root().0,
    }
}

fn kind_dto(kind: &NodeKind) -> NodeKindDto {
    match kind {
        NodeKind::Extrude {
            profile,
            placement,
            height,
            caps,
        } => NodeKindDto::Extrude {
            profile: profile.0,
            placement: placement_dto(placement),
            height: *height,
            caps: caps_name(*caps),
        },
        NodeKind::Revolve {
            profile,
            placement,
            sweep,
            caps,
        } => NodeKindDto::Revolve {
            profile: profile.0,
            placement: placement_dto(placement),
            sweep: *sweep,
            caps: caps_name(*caps),
        },
        NodeKind::Loft {
            sections,
            policy,
            caps,
        } => {
            let LoftPolicy::Ruled = policy;
            NodeKindDto::Loft {
                sections: sections
                    .iter()
                    .map(|(placement, profile)| (placement_dto(placement), profile.0))
                    .collect(),
                caps: caps_name(*caps),
            }
        }
        NodeKind::Sweep {
            profile,
            path,
            caps,
        } => {
            let Path3::Polyline { points, frame } = path;
            let FramePolicy::RotationMinimizing = frame;
            NodeKindDto::Sweep {
                profile: profile.0,
                points: points.clone(),
                caps: caps_name(*caps),
            }
        }
        NodeKind::PlanarFace { profile, placement } => NodeKindDto::PlanarFace {
            profile: profile.0,
            placement: placement_dto(placement),
        },
        NodeKind::Primitive { spec, placement } => match spec {
            PrimitiveSpec::Box { size } => NodeKindDto::Box {
                size: *size,
                placement: placement_dto(placement),
            },
            PrimitiveSpec::Cylinder {
                radius,
                height,
                segments,
            } => NodeKindDto::Cylinder {
                radius: *radius,
                height: *height,
                segments: *segments,
                placement: placement_dto(placement),
            },
        },
        NodeKind::Csg { op, operands } => NodeKindDto::Csg {
            csg: String::from(match op {
                CsgOp::Union => "union",
                CsgOp::Difference => "difference",
                CsgOp::Intersection => "intersection",
            }),
            operands: operands.iter().map(|n| n.0).collect(),
        },
        NodeKind::Transform { child, xf } => NodeKindDto::Transform {
            child: child.0,
            placement: placement_dto(xf),
        },
        NodeKind::Mirror { child, plane } => NodeKindDto::Mirror {
            child: child.0,
            plane: plane_dto(plane),
        },
        NodeKind::Instance { of, placement } => NodeKindDto::Instance {
            of: of.0,
            placement: placement_dto(placement),
        },
        NodeKind::Group { children } => NodeKindDto::Group {
            children: children.iter().map(|n| n.0).collect(),
        },
        NodeKind::MeshImport { import, placement } => NodeKindDto::MeshImport {
            import: import.0,
            placement: placement_dto(placement),
        },
        NodeKind::Stretch {
            child,
            plane,
            length,
        } => NodeKindDto::Stretch {
            child: child.0,
            plane: plane_dto(plane),
            length: *length,
        },
    }
}

/// Rebuilds a validated recipe from its interchange document.
///
/// # Errors
///
/// Returns a typed [`InterchangeError`]; the rebuild runs full builder
/// validation, so malformed geometry fails exactly like direct
/// construction.
pub fn from_dto(dto: &RecipeDto) -> Result<Recipe, InterchangeError> {
    if dto.format != FORMAT || dto.version != VERSION {
        return Err(InterchangeError::UnsupportedFormat);
    }
    let mut builder = RecipeBuilder::new();
    for source in &dto.sources {
        builder.source_ref(source);
    }
    for slot in &dto.slots {
        builder.material_slot(slot);
    }
    for policy in &dto.policies {
        builder.curve_policy(policy);
    }
    for import in &dto.imports {
        let mut mesh_builder = exedra::MeshBuilder::new();
        for position in &import.positions {
            mesh_builder.push_vertex(*position);
        }
        for face in &import.faces {
            mesh_builder
                .add_face(face)
                .map_err(|_| InterchangeError::InvalidImport)?;
        }
        let built = mesh_builder
            .build()
            .map_err(|_| InterchangeError::InvalidImport)?;
        builder
            .add_import(built.mesh)
            .map_err(InterchangeError::Recipe)?;
    }
    for profile in &dto.profiles {
        let outer = loop_value(&profile.outer)?;
        let holes = profile
            .holes
            .iter()
            .map(|hole| loop_value(hole))
            .collect::<Result<Vec<_>, _>>()?;
        builder.add_profile(Profile2::new(outer, holes).map_err(InterchangeError::Profile)?);
    }
    for node in &dto.nodes {
        if let Some(source) = node.source {
            builder.with_source(SourceId(source));
        }
        if let Some(material) = node.material {
            builder.with_material(SlotId(material));
        }
        if let Some(issue) = node.issue {
            builder.with_issue(SourceId(issue));
        }
        let kind = kind_value(&node.kind)?;
        builder.add(kind).map_err(InterchangeError::Recipe)?;
    }
    builder
        .finish(NodeId(dto.root))
        .map_err(InterchangeError::Recipe)
}

fn kind_value(dto: &NodeKindDto) -> Result<NodeKind, InterchangeError> {
    Ok(match dto {
        NodeKindDto::Extrude {
            profile,
            placement,
            height,
            caps,
        } => NodeKind::Extrude {
            profile: ProfileId(*profile),
            placement: placement_value(*placement),
            height: *height,
            caps: caps_value(caps)?,
        },
        NodeKindDto::Revolve {
            profile,
            placement,
            sweep,
            caps,
        } => NodeKind::Revolve {
            profile: ProfileId(*profile),
            placement: placement_value(*placement),
            sweep: *sweep,
            caps: caps_value(caps)?,
        },
        NodeKindDto::Loft { sections, caps } => NodeKind::Loft {
            sections: sections
                .iter()
                .map(|(placement, profile)| (placement_value(*placement), ProfileId(*profile)))
                .collect(),
            policy: LoftPolicy::Ruled,
            caps: caps_value(caps)?,
        },
        NodeKindDto::Sweep {
            profile,
            points,
            caps,
        } => NodeKind::Sweep {
            profile: ProfileId(*profile),
            path: Path3::Polyline {
                points: points.clone(),
                frame: FramePolicy::RotationMinimizing,
            },
            caps: caps_value(caps)?,
        },
        NodeKindDto::PlanarFace { profile, placement } => NodeKind::PlanarFace {
            profile: ProfileId(*profile),
            placement: placement_value(*placement),
        },
        NodeKindDto::Box { size, placement } => NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: *size },
            placement: placement_value(*placement),
        },
        NodeKindDto::Cylinder {
            radius,
            height,
            segments,
            placement,
        } => NodeKind::Primitive {
            spec: PrimitiveSpec::Cylinder {
                radius: *radius,
                height: *height,
                segments: *segments,
            },
            placement: placement_value(*placement),
        },
        NodeKindDto::Csg { csg, operands } => NodeKind::Csg {
            op: match csg.as_str() {
                "union" => CsgOp::Union,
                "difference" => CsgOp::Difference,
                "intersection" => CsgOp::Intersection,
                _ => return Err(InterchangeError::UnknownValue { field: "csg" }),
            },
            operands: operands.iter().map(|n| NodeId(*n)).collect(),
        },
        NodeKindDto::Transform { child, placement } => NodeKind::Transform {
            child: NodeId(*child),
            xf: placement_value(*placement),
        },
        NodeKindDto::Mirror { child, plane } => NodeKind::Mirror {
            child: NodeId(*child),
            plane: plane_value(*plane),
        },
        NodeKindDto::Instance { of, placement } => NodeKind::Instance {
            of: NodeId(*of),
            placement: placement_value(*placement),
        },
        NodeKindDto::Group { children } => NodeKind::Group {
            children: children.iter().map(|n| NodeId(*n)).collect(),
        },
        NodeKindDto::MeshImport { import, placement } => NodeKind::MeshImport {
            import: crate::ir::ImportId(*import),
            placement: placement_value(*placement),
        },
        NodeKindDto::Stretch {
            child,
            plane,
            length,
        } => NodeKind::Stretch {
            child: NodeId(*child),
            plane: plane_value(*plane),
            length: *length,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn json_round_trip_preserves_fingerprints() {
        let recipe = crate::text::tests_support::full_coverage_recipe();
        let dto = to_dto(&recipe);
        let json = serde_json::to_string_pretty(&dto).expect("serializes");
        let parsed: RecipeDto = serde_json::from_str(&json).expect("parses");
        let rebuilt = from_dto(&parsed).expect("rebuilds");
        assert_eq!(
            recipe.recipe_fingerprint(),
            rebuilt.recipe_fingerprint(),
            "the wire must preserve content identity exactly"
        );
    }

    #[test]
    fn frozen_schema_corpus_still_parses() {
        // The checked-in corpus pins v1 field names and shapes; if this
        // fails, the schema changed — additive evolution only, and this
        // file never regenerates silently.
        let json = include_str!("../goldens/recipe_v1.frozen.json");
        let dto: RecipeDto = serde_json::from_str(json).expect("frozen corpus parses");
        let recipe = from_dto(&dto).expect("frozen corpus rebuilds");
        assert_eq!(
            format!("{:032x}", recipe.recipe_fingerprint().0),
            include_str!("../goldens/recipe_v1.frozen.fingerprint").trim(),
            "frozen corpus fingerprint changed; the schema or encoding drifted"
        );
    }

    #[test]
    fn unknown_node_kinds_are_hard_errors() {
        let json = r#"{"op": "teleport", "warp": 9}"#;
        let parsed: Result<NodeKindDto, _> = serde_json::from_str(json);
        assert!(parsed.is_err(), "unknown ops must not be skippable");
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let json = r#"{
            "op": "extrude", "profile": 0, "height": 1.0, "caps": "both",
            "placement": [[1,0,0,0],[0,1,0,0],[0,0,1,0]],
            "future_field": {"anything": true}
        }"#;
        let parsed: Result<NodeDto, _> = serde_json::from_str(json);
        assert!(parsed.is_ok(), "additive evolution: unknown fields ignored");
    }

    #[test]
    fn wrong_version_is_rejected() {
        let recipe = crate::text::tests_support::full_coverage_recipe();
        let mut dto = to_dto(&recipe);
        dto.version = 2;
        assert!(matches!(
            from_dto(&dto),
            Err(InterchangeError::UnsupportedFormat)
        ));
    }

    /// Regenerates the frozen corpus. Run deliberately after a reviewed,
    /// additive schema change:
    /// `cargo test -p exedra_constructive --all-features -- --ignored bless_frozen_schema`
    #[test]
    #[ignore = "regenerates the frozen schema corpus; run deliberately"]
    fn bless_frozen_schema() {
        let recipe = crate::text::tests_support::full_coverage_recipe();
        let dto = to_dto(&recipe);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("goldens");
        std::fs::write(
            dir.join("recipe_v1.frozen.json"),
            serde_json::to_string_pretty(&dto).expect("serializes"),
        )
        .expect("write corpus");
        let mut fingerprint = format!("{:032x}", recipe.recipe_fingerprint().0);
        fingerprint.push('\n');
        std::fs::write(dir.join("recipe_v1.frozen.fingerprint"), fingerprint)
            .expect("write fingerprint");
    }
}
