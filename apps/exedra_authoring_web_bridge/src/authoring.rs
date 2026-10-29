// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounded acyclic authoring requests lowered once into native constructors.

use crate::{CornerLocation, Error};
use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec::Vec,
};
use exedra_analytic::{
    bore::{Bore, BoreError, BorePolicy, BoredPlate},
    extrusion::{End, Extrusion, Pocket, Workplane},
    loft::{Loft, LoftPolicy},
    revolve::{Revolution, SphericalCapError},
    sweep::Sweep,
};
use exedra_compile::{NativeGeometryRef, NativeSource};
use exedra_constructive::{
    builders,
    path::PathCursor3,
    profile::{Loop2, Profile2, Seg2},
};
use exedra_math::Placement3;
use exedra_profile::{
    CornerError, CornerSelectionError, CornerTreatment, LoopRef, Profile as SharedProfile,
    ProfilePolicy, SegTag, SegmentRef,
};
use serde::Deserialize;

#[path = "solid_operations.rs"]
mod solid_operations;

const UNNAMED: SegTag = SegTag(u32::MAX);
const MAX_PROFILES: usize = 256;
const MAX_NODES: usize = 256;
const MAX_ROOTS: usize = 64;
const MAX_DEPTH: u16 = 64;

#[derive(Copy, Clone, Debug, Deserialize, serde::Serialize)]
pub(crate) enum Units {
    #[serde(rename = "mm")]
    Millimetre,
    #[serde(rename = "m")]
    Metre,
}
#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum InputEnd {
    Start,
    End,
}
impl From<InputEnd> for End {
    fn from(end: InputEnd) -> Self {
        match end {
            InputEnd::Start => Self::Start,
            InputEnd::End => Self::End,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ProfileNode {
    Corners {
        profile: u32,
        at: Vec<[f64; 2]>,
        tolerance: Option<f64>,
        treatment: FinishKind,
        size: f64,
        feature: Option<String>,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
    Loop {
        segments: Vec<ProfileSegment>,
    },
    Rectangle {
        width: f64,
        height: f64,
    },
    RoundedRectangle {
        width: f64,
        height: f64,
        radius: f64,
    },
    Circle {
        radius: f64,
    },
    Slot {
        length: f64,
        radius: f64,
    },
    Tagged {
        profile: u32,
        name: String,
    },
    Translated {
        profile: u32,
        offset: [f64; 2],
    },
    Cutouts {
        profile: u32,
        template: u32,
        positions: Vec<[f64; 2]>,
    },
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ProfileSegment {
    Line {
        to: [f64; 2],
        feature: Option<String>,
    },
    Arc {
        to: [f64; 2],
        center: [f64; 2],
        clockwise: bool,
        feature: Option<String>,
    },
}
impl ProfileSegment {
    fn feature(&self) -> Option<&str> {
        match self {
            Self::Line { feature, .. } | Self::Arc { feature, .. } => feature.as_deref(),
        }
    }
    fn lower(&self, labels: &BTreeMap<String, SegTag>) -> exedra_profile::Segment {
        let tag = self
            .feature()
            .and_then(|name| labels.get(name))
            .copied()
            .unwrap_or(UNNAMED);
        match self {
            Self::Line { to, .. } => exedra_profile::Segment::line(*to),
            Self::Arc {
                to,
                center,
                clockwise,
                ..
            } => exedra_profile::Segment::arc_centered(*to, *center, *clockwise),
        }
        .tagged(tag)
    }
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum FaceSelector {
    Floor { feature: String },
    Cap { end: InputEnd },
}
#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InputPlacement {
    #[serde(default)]
    translation: [f64; 3],
    #[serde(default, rename = "rotationXYZ")]
    rotation_xyz: [f64; 3],
}
impl InputPlacement {
    pub(crate) fn frame(self) -> Placement3 {
        let [x, y, z] = self.rotation_xyz;
        Placement3::euler_extrinsic_xyz_then_translate(x, y, z, self.translation)
    }
}
#[derive(Debug)]
enum Model {
    Extrusion(Extrusion),
    Revolution(Revolution),
    Sweep(Sweep),
    Loft(Loft),
    BoredPlate(Box<BoredPlate>),
    Body {
        body: exedra_analytic::body::Body<Vec<SegTag>>,
        frame: Placement3,
        machining: Vec<AuthoredMachining>,
        operations: Vec<AuthoredSolidOperation>,
        integrals: Option<[f64; 2]>,
    },
}
impl Model {
    fn integral_values(&self) -> Option<[f64; 2]> {
        match self {
            Self::Extrusion(v) => Some([v.volume(), v.surface_area()]),
            Self::Revolution(v) => Some([v.volume(), v.surface_area()]),
            Self::Sweep(v) => Some([v.volume(), v.surface_area()]),
            Self::BoredPlate(v) => Some([v.volume(), v.surface_area()]),
            Self::Loft(_) => None,
            Self::Body { integrals, .. } => *integrals,
        }
        .filter(|values| values.iter().all(|v| v.is_finite() && *v > 0.0))
    }
    fn geometry(&self) -> NativeGeometryRef<'_> {
        match self {
            Self::Extrusion(v) => NativeGeometryRef::Extrusion(v),
            Self::Revolution(v) => NativeGeometryRef::Revolution(v),
            Self::Sweep(v) => NativeGeometryRef::Sweep(v),
            Self::Loft(v) => NativeGeometryRef::Loft(v),
            Self::BoredPlate(v) => NativeGeometryRef::BoredPlate(v),
            Self::Body { body, frame, .. } => NativeGeometryRef::Body {
                body,
                frame: *frame,
            },
        }
    }
    fn extrusion(&self) -> Result<&Extrusion, Error> {
        if let Self::Extrusion(v) = self {
            Ok(v)
        } else {
            Err(Error::new(
                "operation.unsupportedFamily",
                "solid.extrusionFeature",
                "This operation requires an extrusion source",
            ))
        }
    }
    fn install(self) -> NativeSource {
        match self {
            Self::Extrusion(v) => NativeSource::from(v),
            Self::Revolution(v) => NativeSource::from(v),
            Self::Sweep(v) => NativeSource::from(v),
            Self::Loft(v) => NativeSource::from(v),
            Self::BoredPlate(v) => NativeSource::from(*v),
            Self::Body { body, frame, .. } => {
                NativeSource::from_body(body, frame).expect("authoring checked body placement")
            }
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum PathStep {
    LineTo {
        to: [f64; 3],
    },
    LineBy {
        offset: [f64; 3],
    },
    TangentArc {
        radius: f64,
        angle: f64,
        normal: [f64; 3],
    },
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputPath {
    start: [f64; 3],
    tangent: [f64; 3],
    section_x: [f64; 3],
    steps: Vec<PathStep>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoftSection {
    profile: u32,
    placement: InputPlacement,
}
#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FinishKind {
    Fillet,
    Chamfer,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Node {
    Union {
        base: u32,
        other: u32,
    },
    Intersect {
        base: u32,
        other: u32,
    },
    Subtract {
        base: u32,
        other: u32,
    },
    IntersectHalfSpace {
        base: u32,
        point: [f64; 3],
        normal: [f64; 3],
        feature: Option<String>,
    },
    Body {
        body: crate::body::InputBody,
        #[serde(default)]
        placement: InputPlacement,
    },
    SphericalCap {
        radius: f64,
        height: f64,
        #[serde(default)]
        placement: InputPlacement,
    },
    SubtractThroughCylinder {
        base: u32,
        point: [f64; 3],
        direction: [f64; 3],
        radius: f64,
        separation: f64,
        feature: Option<String>,
    },

    ThroughBore {
        base: u32,
        point: [f64; 3],
        direction: [f64; 3],
        radius: f64,
        separation: f64,
        feature: Option<String>,
    },
    Sweep {
        profile: u32,
        path: u32,
        #[serde(default)]
        placement: InputPlacement,
    },
    Loft {
        sections: Vec<LoftSection>,
        #[serde(default)]
        placement: InputPlacement,
    },
    AxialFinish {
        base: u32,
        feature: String,
        treatment: FinishKind,
        size: f64,
        transition: String,
    },
    Extrude {
        profile: u32,
        height: f64,
        #[serde(default)]
        placement: InputPlacement,
    },
    Revolve {
        profile: u32,
        angle: f64,
        #[serde(default)]
        placement: InputPlacement,
    },
    Pocket {
        base: u32,
        profile: u32,
        end: InputEnd,
        depth: f64,
        at: [f64; 2],
    },
    OnFace {
        host: u32,
        face: FaceSelector,
        profile: u32,
        height: f64,
        offset: f64,
        at: [f64; 2],
    },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Root {
    name: String,
    node: u32,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    units: Units,
    separation: f64,
    profiles: Vec<ProfileNode>,
    #[serde(default)]
    paths: Vec<InputPath>,
    nodes: Vec<Node>,
    roots: Vec<Root>,
}
#[derive(Clone, Debug)]
pub(crate) struct AuthoredMachining {
    pub report: exedra_analytic::machining::MachiningReport,
    pub point: [f64; 3],
    pub direction: [f64; 3],
    pub radius: f64,
    pub feature: Option<String>,
}
#[derive(Clone, Debug)]
pub(crate) enum AuthoredSolidOperation {
    Rectilinear {
        node: usize,
        left: u32,
        right: u32,
        report: exedra_analytic::rectilinear::RectilinearReport,
    },
    HalfSpace {
        node: usize,
        base: u32,
        point: [f64; 3],
        normal: [f64; 3],
        feature: Option<String>,
        report: exedra_analytic::halfspace::ClipReport,
    },
}
impl AuthoredSolidOperation {
    fn node(&self) -> usize {
        match self {
            Self::Rectilinear { node, .. } | Self::HalfSpace { node, .. } => *node,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct MachiningEvidence {
    pub reports: Vec<AuthoredMachining>,
    pub operations: Vec<AuthoredSolidOperation>,
    pub integrals: Option<[f64; 2]>,
}
#[derive(Debug)]
pub(crate) struct Built {
    pub units: Units,
    pub separation: f64,
    pub roots: Vec<(String, NativeSource)>,
    pub names: BTreeMap<u32, String>,
    pub machining: BTreeMap<String, MachiningEvidence>,
}
fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128
}
fn corner_location(profile: &SharedProfile, reference: SegmentRef) -> Option<CornerLocation> {
    let (boundary, hole, loop_) = match reference.loop_ref {
        LoopRef::Outer => ("outer", None, profile.outer()),
        LoopRef::Hole(index) => ("hole", Some(index), profile.holes().get(index)?),
    };
    Some(CornerLocation {
        boundary,
        hole,
        segment: reference.segment,
        point: loop_.segments().get(reference.segment)?.to,
    })
}
fn corner_selection_error(
    profile: &SharedProfile,
    error: CornerSelectionError,
    index: usize,
) -> Error {
    let code = match &error {
        CornerSelectionError::Input { .. } => "profile.corner.selectionInput",
        CornerSelectionError::Missing { .. } => "profile.corner.missing",
        CornerSelectionError::Ambiguous { .. } => "profile.corner.ambiguous",
        _ => "profile.corner.selection",
    };
    let mut diagnostic = Error::new(code, "profile.corners", error.to_string()).profile(index);
    match error {
        CornerSelectionError::Input {
            point,
            max_distance,
        }
        | CornerSelectionError::Missing {
            point,
            max_distance,
        }
        | CornerSelectionError::Ambiguous {
            point,
            max_distance,
            ..
        } => {
            let context = diagnostic.context_mut();
            context.corner_point = point.iter().all(|x| x.is_finite()).then_some(point);
            context.corner_tolerance = max_distance.is_finite().then_some(max_distance);
        }
        _ => {}
    }
    if let CornerSelectionError::Ambiguous { matches, .. } = error {
        diagnostic.context_mut().corner_candidates = Some(
            matches
                .into_iter()
                .filter_map(|r| corner_location(profile, r))
                .collect(),
        );
    }
    diagnostic
}
fn corner_treatment_error(
    profile: &SharedProfile,
    error: CornerError,
    index: usize,
    size: f64,
    feature: Option<String>,
) -> Error {
    let code = match &error {
        CornerError::InvalidSize { .. } => "profile.corner.size",
        CornerError::UnknownCorner { .. } => "profile.corner.missing",
        CornerError::RepeatedCorner { .. } => "profile.corner.repeated",
        CornerError::SmoothCorner { .. } => "profile.corner.smooth",
        CornerError::UnsupportedCorner { .. } => "profile.corner.unsupportedJunction",
        CornerError::OverlappingTrims { requested, .. } if !requested.is_finite() => {
            "profile.corner.numericLimit"
        }
        CornerError::OverlappingTrims { .. } => "profile.corner.overlap",
        CornerError::Loop(_) => "profile.corner.numericLimit",
        CornerError::Input(_) => "profile.corner.input",
        CornerError::Output(_) => "profile.corner.output",
        _ => "profile.corner.invalid",
    };
    let mut diagnostic = Error::new(code, "profile.corners", error.to_string()).profile(index);
    let context = diagnostic.context_mut();
    context.feature = feature;
    context.requested_size = size.is_finite().then_some(size);
    match error {
        CornerError::UnknownCorner { corner }
        | CornerError::RepeatedCorner { corner }
        | CornerError::SmoothCorner { corner }
        | CornerError::UnsupportedCorner { corner, .. } => {
            context.corner = corner_location(profile, corner);
        }
        CornerError::OverlappingTrims {
            segment,
            length,
            requested,
            min_separation,
        } => {
            context.corner = corner_location(profile, segment);
            context.segment_length = length.is_finite().then_some(length);
            context.requested_trim = requested.is_finite().then_some(requested);
            context.minimum_separation = min_separation.is_finite().then_some(min_separation);
        }
        _ => {}
    }
    diagnostic
}

fn spherical_cap_error(error: SphericalCapError, index: usize, radius: f64, height: f64) -> Error {
    let code = match &error {
        SphericalCapError::Radius { .. } => "revolve.cap.radius",
        SphericalCapError::Height { .. } => "revolve.cap.height",
        SphericalCapError::HeightResolution { .. } => "revolve.cap.heightResolution",
        SphericalCapError::RimResolution { .. } => "revolve.cap.rimResolution",
        SphericalCapError::Loop(_) | SphericalCapError::Profile(_) => "revolve.cap.profile",
        SphericalCapError::Revolution(_) => "revolve.cap.invalid",
    };
    let mut diagnostic = Error::new(code, "solid.sphericalCap", error.to_string()).node(index);
    let context = diagnostic.context_mut();
    context.radius = radius.is_finite().then_some(radius);
    context.height = height.is_finite().then_some(height);
    match error {
        SphericalCapError::HeightResolution { axis_offset, .. } => {
            context.axis_offset = axis_offset.is_finite().then_some(axis_offset);
        }
        SphericalCapError::RimResolution { rim_radius, .. } => {
            context.rim_radius = rim_radius.is_finite().then_some(rim_radius);
        }
        _ => {}
    }
    diagnostic
}

impl Request {
    fn validate(&self) -> Result<BTreeMap<String, SegTag>, Error> {
        if !self.separation.is_finite() || self.separation <= 0.0 {
            return Err(Error::new(
                "policy.separation",
                "document.validate",
                "Profile separation must be positive and finite",
            ));
        }
        if self.profiles.len() > MAX_PROFILES
            || self.nodes.len() > MAX_NODES
            || self.roots.is_empty()
            || self.roots.len() > MAX_ROOTS
        {
            return Err(Error::new(
                "budget.graph",
                "document.validate",
                "A document supports 256 profiles, 256 operations and 1–64 roots",
            ));
        }
        if self.paths.len() > 256
            || self
                .paths
                .iter()
                .any(|p| p.steps.is_empty() || p.steps.len() > 128)
        {
            return Err(Error::new(
                "budget.path",
                "document.validate",
                "A document supports 256 paths of 1–128 steps",
            ));
        }
        let mut labels = BTreeSet::new();
        let mut depths = Vec::with_capacity(self.profiles.len());
        let mut counts: Vec<usize> = Vec::with_capacity(self.profiles.len());
        let mut profile_work = 0_usize;
        for (i, p) in self.profiles.iter().enumerate() {
            let mut depth = 1;
            let mut dependency = |id: u32| -> Result<(), Error> {
                if id as usize >= i {
                    return Err(Error::new(
                        "graph.profileReference",
                        "document.validate",
                        "Profile references must point to earlier profiles",
                    )
                    .profile(i));
                }
                depth = depth.max(depths[id as usize] + 1);
                Ok(())
            };
            match p {
                ProfileNode::Tagged { profile, name } => {
                    dependency(*profile)?;
                    if !label(name) {
                        return Err(Error::new(
                            "input.name",
                            "document.validate",
                            "Feature names must contain text and fit in 128 UTF-8 bytes",
                        )
                        .profile(i));
                    }
                    labels.insert(name.clone());
                }
                ProfileNode::Translated { profile, .. } => dependency(*profile)?,
                ProfileNode::Loop { segments } => {
                    for segment in segments {
                        if let Some(feature) = segment.feature() {
                            if !label(feature) {
                                return Err(Error::new("input.name","document.validate","Segment feature names must contain text and fit in 128 UTF-8 bytes").profile(i));
                            }
                            labels.insert(feature.to_string());
                        }
                    }
                }
                ProfileNode::Corners {
                    profile,
                    at,
                    feature,
                    ..
                } => {
                    dependency(*profile)?;
                    if at.is_empty() || at.len() > 4096 {
                        return Err(Error::new(
                            "budget.corners",
                            "document.validate",
                            "Select 1–4096 profile corners",
                        )
                        .profile(i));
                    }
                    if let Some(feature) = feature {
                        if !label(feature) {
                            return Err(Error::new(
                                "input.name",
                                "document.validate",
                                "Corner feature names must contain text and fit in 128 UTF-8 bytes",
                            )
                            .profile(i));
                        }
                        labels.insert(feature.clone());
                    }
                }
                ProfileNode::Cutouts {
                    profile,
                    template,
                    positions,
                } => {
                    dependency(*profile)?;
                    dependency(*template)?;
                    if positions.len() > 1024 {
                        return Err(Error::new(
                            "budget.cutouts",
                            "document.validate",
                            "At most 1024 cutout placements are allowed",
                        )
                        .profile(i));
                    }
                }
                _ => {}
            }
            if depth > MAX_DEPTH {
                return Err(Error::new(
                    "budget.depth",
                    "document.validate",
                    "Profile depth exceeds 64 operations",
                )
                .profile(i));
            }
            let count = match p {
                ProfileNode::Rectangle { .. } | ProfileNode::Slot { .. } => 4,
                ProfileNode::RoundedRectangle { .. } => 8,
                ProfileNode::Circle { .. } => 2,
                ProfileNode::Polygon { points } => points.len(),
                ProfileNode::Loop { segments } => segments.len(),
                ProfileNode::Tagged { profile, .. } | ProfileNode::Translated { profile, .. } => {
                    counts[*profile as usize]
                }
                ProfileNode::Corners { profile, at, .. } => {
                    counts[*profile as usize].saturating_add(at.len())
                }
                ProfileNode::Cutouts {
                    profile,
                    template,
                    positions,
                } => counts[*profile as usize]
                    .saturating_add(counts[*template as usize].saturating_mul(positions.len())),
            };
            profile_work = profile_work.saturating_add(count);
            if count > 4096 || profile_work > 16384 {
                return Err(Error::new(
                    "budget.profileSegments",
                    "document.validate",
                    "Profile descriptions exceed the segment work budget",
                )
                .profile(i));
            }
            counts.push(count);
            depths.push(depth);
        }
        let mut body_counts: Vec<usize> = Vec::with_capacity(self.nodes.len());
        let mut native_work = 0_usize;
        let mut depths: Vec<u16> = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.iter().enumerate() {
            let (profile_ids, parent, multiplier): (Vec<u32>, Option<u32>, usize) = match node {
                Node::Body { body, .. } => {
                    for feature in body.feature_names() {
                        if !label(feature) {
                            return Err(Error::new(
                                "input.name",
                                "document.validate",
                                "Body feature names must contain text and fit in 128 UTF-8 bytes",
                            )
                            .node(i));
                        }
                        labels.insert(feature.to_string());
                    }
                    (Vec::new(), None, 1)
                }
                Node::SphericalCap { .. } => (Vec::new(), None, 1),
                Node::Union { base, other }
                | Node::Intersect { base, other }
                | Node::Subtract { base, other } => {
                    if *other as usize >= i {
                        return Err(Error::new(
                            "graph.nodeReference",
                            "document.validate",
                            "Boolean operands must reference earlier nodes",
                        )
                        .node(i));
                    }
                    (Vec::new(), Some(*base), 1)
                }
                Node::IntersectHalfSpace { base, feature, .. } => {
                    if let Some(feature) = feature {
                        if !label(feature) {
                            return Err(Error::new("input.name","document.validate","Halfspace feature names must contain text and fit in 128 UTF-8 bytes").node(i));
                        }
                        labels.insert(feature.clone());
                    }
                    (Vec::new(), Some(*base), 1)
                }
                Node::Extrude { profile, .. } | Node::Revolve { profile, .. } => {
                    (alloc::vec![*profile], None, 1)
                }
                Node::Sweep { profile, path, .. } => {
                    let path = self.paths.get(*path as usize).ok_or_else(|| {
                        Error::new(
                            "graph.pathReference",
                            "document.validate",
                            "Sweep path is outside the path table",
                        )
                        .node(i)
                    })?;
                    (alloc::vec![*profile], None, path.steps.len())
                }
                Node::Loft { sections, .. } => {
                    if !(2..=64).contains(&sections.len()) {
                        return Err(Error::new(
                            "budget.sections",
                            "document.validate",
                            "A loft needs 2–64 corresponding sections",
                        )
                        .node(i));
                    }
                    (sections.iter().map(|s| s.profile).collect(), None, 1)
                }
                Node::ThroughBore { base, feature, .. }
                | Node::SubtractThroughCylinder { base, feature, .. } => {
                    if let Some(feature) = feature {
                        if !label(feature) {
                            return Err(Error::new(
                                "input.name",
                                "document.validate",
                                "Bore feature names must contain text and fit in 128 UTF-8 bytes",
                            )
                            .node(i));
                        }
                        labels.insert(feature.clone());
                    }
                    (Vec::new(), Some(*base), 1)
                }
                Node::AxialFinish {
                    base,
                    feature,
                    transition,
                    ..
                } => {
                    if !label(feature) || !label(transition) {
                        return Err(Error::new(
                            "input.name",
                            "document.validate",
                            "Finish feature names must contain text and fit in 128 UTF-8 bytes",
                        )
                        .node(i));
                    }
                    labels.insert(transition.clone());
                    (Vec::new(), Some(*base), 1)
                }
                Node::Pocket { profile, base, .. } => (alloc::vec![*profile], Some(*base), 1),
                Node::OnFace { profile, host, .. } => (alloc::vec![*profile], Some(*host), 1),
            };
            if profile_ids
                .iter()
                .any(|id| *id as usize >= self.profiles.len())
            {
                return Err(Error::new(
                    "graph.profileReference",
                    "document.validate",
                    "Operation profile is outside the profile table",
                )
                .node(i));
            }
            let depth = if let Some(parent) = parent {
                if parent as usize >= i {
                    return Err(Error::new(
                        "graph.nodeReference",
                        "document.validate",
                        "Operation references must point to earlier nodes",
                    )
                    .node(i));
                }
                (if let Node::Union { other, .. }
                | Node::Intersect { other, .. }
                | Node::Subtract { other, .. } = node
                {
                    depths[parent as usize].max(depths[*other as usize])
                } else {
                    depths[parent as usize]
                }) + 1
            } else {
                1
            };
            if depth > MAX_DEPTH {
                return Err(Error::new(
                    "budget.depth",
                    "document.validate",
                    "Construction depth exceeds 64 operations",
                )
                .node(i));
            }
            let count = profile_ids
                .iter()
                .fold(0_usize, |sum, id| sum.saturating_add(counts[*id as usize]))
                .saturating_mul(multiplier)
                .saturating_add(if let Node::Body { body, .. } = node {
                    body.work()
                } else {
                    0
                })
                .saturating_add(if matches!(node, Node::SphericalCap { .. }) {
                    3
                } else {
                    0
                })
                .saturating_add(
                    if matches!(
                        node,
                        Node::Pocket { .. }
                            | Node::Union { .. }
                            | Node::Intersect { .. }
                            | Node::Subtract { .. }
                            | Node::IntersectHalfSpace { .. }
                            | Node::AxialFinish { .. }
                            | Node::ThroughBore { .. }
                            | Node::SubtractThroughCylinder { .. }
                    ) {
                        body_counts[parent.expect("feature parent") as usize]
                    } else {
                        0
                    },
                );
            let count = count.saturating_add(
                if let Node::Union { other, .. }
                | Node::Intersect { other, .. }
                | Node::Subtract { other, .. } = node
                {
                    body_counts[*other as usize]
                } else {
                    0
                },
            );
            native_work = native_work.saturating_add(count);
            if native_work > 8192 {
                return Err(Error::new(
                    "budget.nativeSegments",
                    "document.validate",
                    "Construction exceeds the 8192 transient native-segment work budget",
                )
                .node(i));
            }
            body_counts.push(count);
            depths.push(depth);
        }
        let mut roots = BTreeSet::new();
        for root in &self.roots {
            if !label(&root.name)
                || !roots.insert(&root.name)
                || root.node as usize >= self.nodes.len()
            {
                return Err(Error::new(
                    "graph.root",
                    "document.validate",
                    "Root names must be unique and reference existing operations",
                ));
            }
        }
        Ok(labels
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name, SegTag(u32::try_from(i + 1).expect("bounded labels"))))
            .collect())
    }
    pub(crate) fn build(self) -> Result<Built, Error> {
        let labels = self.validate()?;
        let policy = ProfilePolicy::new(self.separation);
        struct AuthoredProfile {
            geometry: SharedProfile,
            feature: Option<SegTag>,
        }
        let mut profiles: Vec<AuthoredProfile> = Vec::with_capacity(self.profiles.len());
        for (i, node) in self.profiles.into_iter().enumerate() {
            let failure = |message: String| {
                Error::new("profile.invalid", "profile.build", message).profile(i)
            };
            let mut feature = None;
            let canonical = |profile: Profile2| {
                profile
                    .to_line_arc(&policy)
                    .map(|p| p.tagged(UNNAMED))
                    .map_err(|e| failure(e.to_string()))
            };
            let geometry = match node {
                ProfileNode::Polygon { points } => canonical(
                    Profile2::new(
                        Loop2::new(
                            points
                                .into_iter()
                                .map(|p| Seg2::line((p[0], p[1])))
                                .collect(),
                        )
                        .map_err(|e| failure(e.to_string()))?,
                        Vec::new(),
                    )
                    .map_err(|e| failure(e.to_string()))?,
                )?,
                ProfileNode::Loop { segments } => {
                    let lowered = segments
                        .iter()
                        .map(|segment| segment.lower(&labels))
                        .collect();
                    let boundary = exedra_profile::Loop::new(lowered).map_err(|error| {
                        if let exedra_profile::LoopError::InvalidArc { segment } = error {
                            let current = segments[segment].lower(&labels);
                            let from = segments[(segment + segments.len() - 1) % segments.len()]
                                .lower(&labels)
                                .to;
                            let error =
                                exedra_profile::Arc::resolve(from, current.to, current.curve)
                                    .expect_err("invalid arc");
                            let code = match error {
                                exedra_profile::ArcError::EndpointRadii { .. } => {
                                    "profile.arc.endpointRadii"
                                }
                                exedra_profile::ArcError::NumericLimit => {
                                    "profile.arc.numericLimit"
                                }
                                _ => "profile.arc.input",
                            };
                            let mut diagnostic =
                                Error::new(code, "profile.loop", error.to_string()).profile(i);
                            diagnostic.context_mut().segment = Some(segment);
                            if let exedra_profile::ArcError::EndpointRadii {
                                start_radius,
                                end_radius,
                                roundoff_bound,
                            } = error
                            {
                                diagnostic.context_mut().start_radius = Some(start_radius);
                                diagnostic.context_mut().end_radius = Some(end_radius);
                                diagnostic.context_mut().radius_roundoff_bound =
                                    Some(roundoff_bound);
                            }
                            if let Some(tag) = current.tag {
                                diagnostic.context_mut().feature = labels
                                    .iter()
                                    .find(|(_, v)| **v == tag)
                                    .map(|(name, _)| name.clone());
                            }
                            diagnostic
                        } else {
                            failure(error.to_string())
                        }
                    })?;
                    SharedProfile::new(boundary, Vec::new(), &policy)
                        .map_err(|error| failure(error.to_string()))?
                }
                ProfileNode::Rectangle { width, height } => canonical(
                    builders::rect_centered(width, height).map_err(|e| failure(e.to_string()))?,
                )?,
                ProfileNode::RoundedRectangle {
                    width,
                    height,
                    radius,
                } => canonical(
                    builders::rounded_rect_centered(width, height, radius)
                        .map_err(|e| failure(e.to_string()))?,
                )?,
                ProfileNode::Circle { radius } => {
                    canonical(builders::circle(radius).map_err(|e| failure(e.to_string()))?)?
                }
                ProfileNode::Slot { length, radius } => canonical(
                    builders::slot_centered(length, radius).map_err(|e| failure(e.to_string()))?,
                )?,
                ProfileNode::Tagged { profile, name } => {
                    feature = Some(labels[&name]);
                    profiles[profile as usize]
                        .geometry
                        .clone()
                        .tagged(labels[&name])
                }
                ProfileNode::Translated { profile, offset } => {
                    feature = profiles[profile as usize].feature;
                    profiles[profile as usize]
                        .geometry
                        .translated(offset, &policy)
                        .map_err(|e| failure(e.to_string()))?
                }
                ProfileNode::Corners {
                    profile,
                    at,
                    tolerance,
                    treatment,
                    size,
                    feature: transition,
                } => {
                    let source = &profiles[profile as usize].geometry;
                    feature = profiles[profile as usize].feature;
                    let tolerance = tolerance.unwrap_or(self.separation);
                    let mut selected = Vec::with_capacity(at.len());
                    for point in at {
                        selected.push(
                            source
                                .corner_at(point, tolerance)
                                .map_err(|e| corner_selection_error(source, e, i))?
                                .segment,
                        );
                    }
                    let mut treatment = match treatment {
                        FinishKind::Fillet => CornerTreatment::fillet(size),
                        FinishKind::Chamfer => CornerTreatment::chamfer(size),
                    };
                    if let Some(name) = &transition {
                        treatment = treatment.tagged(labels[name]);
                    }
                    source
                        .with_corners(selected, treatment, &policy)
                        .map_err(|e| corner_treatment_error(source, e, i, size, transition))?
                        .profile
                }
                ProfileNode::Cutouts {
                    profile,
                    template,
                    positions,
                } => {
                    let parent = &profiles[profile as usize].geometry;
                    let template = &profiles[template as usize].geometry;
                    if !template.holes().is_empty() {
                        return Err(failure("Cutout templates must have no holes".into()));
                    }
                    let mut holes = parent.holes().to_vec();
                    for offset in positions {
                        holes.push(
                            template
                                .outer()
                                .translated(offset)
                                .map_err(|e| failure(e.to_string()))?
                                .reversed(),
                        );
                    }
                    SharedProfile::new(parent.outer().clone(), holes, &policy)
                        .map_err(|e| failure(e.to_string()))?
                }
            };
            profiles.push(AuthoredProfile { geometry, feature });
        }
        let paths = self
            .paths
            .into_iter()
            .enumerate()
            .map(|(i, path)| {
                let mut cursor = PathCursor3::new(path.start, path.tangent);
                for step in path.steps {
                    cursor = match step {
                        PathStep::LineTo { to } => cursor.line_to(to),
                        PathStep::LineBy { offset } => cursor.line_by(offset),
                        PathStep::TangentArc {
                            radius,
                            angle,
                            normal,
                        } => cursor.tangent_arc(radius, angle, normal),
                    };
                }
                cursor
                    .finish(path.section_x)
                    .map_err(|e| Error::new("sweep.path", "path.build", format!("Path {i}: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut models: Vec<Model> = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.into_iter().enumerate() {
            let failure = |code: &'static str, operation: &'static str, message: String| {
                Error::new(code, operation, message).node(i)
            };
            let solid = match node {
                Node::Body { body, placement } => {
                    let frame = placement.frame();
                    exedra_analytic::geometry::Surface::plane(frame)
                        .map_err(|e| failure("body.placement", "solid.body", e.to_string()))?;
                    Model::Body {
                        body: body
                            .lower(&labels, self.separation)
                            .map_err(|e| failure(e.code, e.operation, e.message))?,
                        frame,
                        machining: Vec::new(),
                        operations: Vec::new(),
                        integrals: None,
                    }
                }

                Node::Union { base, other } => solid_operations::boolean(
                    &models,
                    base,
                    other,
                    exedra_analytic::rectilinear::BooleanOperation::Union,
                    self.separation,
                    i,
                )?,
                Node::Intersect { base, other } => solid_operations::boolean(
                    &models,
                    base,
                    other,
                    exedra_analytic::rectilinear::BooleanOperation::Intersection,
                    self.separation,
                    i,
                )?,
                Node::Subtract { base, other } => solid_operations::boolean(
                    &models,
                    base,
                    other,
                    exedra_analytic::rectilinear::BooleanOperation::Difference,
                    self.separation,
                    i,
                )?,
                Node::IntersectHalfSpace {
                    base,
                    point,
                    normal,
                    feature,
                } => solid_operations::halfspace(
                    &models[base as usize],
                    base,
                    point,
                    normal,
                    feature,
                    &labels,
                    self.separation,
                    i,
                )?,
                Node::ThroughBore {
                    base,
                    point,
                    direction,
                    radius,
                    separation,
                    feature,
                } => {
                    let refusal = |error: BoreError| {
                        let code = match &error {
                            BoreError::Input => "bore.input",
                            BoreError::InvalidPolicy => "bore.policy",
                            BoreError::UnsupportedPlate => "bore.unsupportedPlate",
                            BoreError::Sidewall { .. } => "bore.clearance",
                            BoreError::NumericLimit => "bore.numericLimit",
                            BoreError::Section(
                                exedra_analytic::geometry::intersection::SectionError::NearParallel {
                                    ..
                                },
                            ) => "bore.nearParallel",
                            _ => "bore.invalid",
                        };
                        {
                            let mut diagnostic =
                                failure(code, "solid.throughBore", error.to_string());
                            let context = diagnostic.context_mut();
                            context.feature = feature.clone();
                            match &error {
                                BoreError::Sidewall { end, clearance, required } => {
                                    context.end = Some(match end {
                                        End::Start => "start",
                                        End::End => "end",
                                    });
                                    context.clearance = Some(*clearance);
                                    context.required_clearance = Some(*required);
                                }
                                BoreError::Section(
                                    exedra_analytic::geometry::intersection::SectionError::NearParallel {
                                        axis_dot, minimum,
                                    },
                                ) => {
                                    context.axis_dot = Some(*axis_dot);
                                    context.minimum_axis_dot = Some(*minimum);
                                }
                                _ => {}
                            }
                            diagnostic
                        }
                    };
                    let mut bore = Bore::new(point, direction, radius).map_err(refusal)?;
                    if let Some(feature) = &feature {
                        bore = bore.tagged(labels[feature]);
                    }
                    let source = models[base as usize]
                        .extrusion()
                        .map_err(|e| failure(e.code, "solid.throughBore", e.message))?;
                    Model::BoredPlate(Box::new(
                        source
                            .with_through_bore(
                                bore,
                                &BorePolicy {
                                    separation,
                                    geometry_gap: self.separation,
                                    min_axis_dot: 0.05,
                                },
                            )
                            .map_err(refusal)?,
                    ))
                }

                Node::SubtractThroughCylinder {
                    base,
                    point,
                    direction,
                    radius,
                    separation,
                    feature,
                } => {
                    let source = &models[base as usize];
                    use exedra_analytic::machining::{
                        CylinderTool, MachiningError, MachiningPolicy,
                        subtract_through_cylinder_with_roles,
                    };
                    let refusal = |error: MachiningError| {
                        let code=match &error {
                                MachiningError::InvalidPolicy=>"machining.policy",
                                MachiningError::OpenBoundary=>"machining.openBoundary",
                                MachiningError::UnsupportedFace{..}=>"machining.unsupportedFace",
                                MachiningError::Clearance{..}=>"machining.clearance",
                                MachiningError::ExitCount{..}=>"machining.exitCount",
                                MachiningError::NumericLimit=>"machining.numericLimit",
                                MachiningError::Section(exedra_analytic::geometry::intersection::SectionError::NearParallel{..})=>"machining.nearParallel",
                                _=>"machining.invalid",
                            };
                        let mut error_context =
                            failure(code, "solid.subtractThroughCylinder", error.to_string());
                        error_context.context_mut().feature = feature.clone();
                        match error {
                                MachiningError::UnsupportedFace{face}=>error_context.context_mut().face=Some(face.0),
                                MachiningError::Clearance{face,bound,required}=>{let context=error_context.context_mut();context.face=Some(face.0);context.clearance=Some(bound);context.required_clearance=Some(required);},
                                MachiningError::Section(exedra_analytic::geometry::intersection::SectionError::NearParallel{axis_dot,minimum})=>{let context=error_context.context_mut();context.axis_dot=Some(axis_dot);context.minimum_axis_dot=Some(minimum);},
                                _=>{},
                            }
                        error_context
                    };
                    let tool = CylinderTool::new(point, direction, radius).map_err(|error| {
                        failure(
                            "machining.tool",
                            "solid.subtractThroughCylinder",
                            error.to_string(),
                        )
                    })?;
                    let role = feature
                        .as_ref()
                        .map(|name| labels[name])
                        .into_iter()
                        .collect();
                    let result = subtract_through_cylinder_with_roles(
                        &source.geometry().to_body(),
                        &tool,
                        &MachiningPolicy {
                            separation,
                            geometry_gap: self.separation,
                            min_axis_dot: 0.05,
                        },
                        role,
                        (),
                    )
                    .map_err(refusal)?;
                    let integrals = source
                        .integral_values()
                        .map(|[volume, area]| {
                            [
                                volume - result.report.removed_volume,
                                area + result.report.surface_area_delta,
                            ]
                        })
                        .filter(|values| values.iter().all(|v| v.is_finite() && *v > 0.0));
                    let mut machining = if let Model::Body { machining, .. } = source {
                        machining.clone()
                    } else {
                        Vec::new()
                    };
                    machining.push(AuthoredMachining {
                        report: result.report,
                        point,
                        direction: tool.surface().frame().rows.map(|row| row[2]),
                        radius,
                        feature,
                    });
                    Model::Body {
                        body: result.body,
                        frame: source.geometry().frame(),
                        machining,
                        operations: if let Model::Body { operations, .. } = source {
                            operations.clone()
                        } else {
                            Vec::new()
                        },
                        integrals,
                    }
                }

                Node::Sweep {
                    profile,
                    path,
                    placement,
                } => Model::Sweep(
                    Sweep::from_profile(
                        &profiles[profile as usize].geometry,
                        &paths[path as usize],
                        placement.frame(),
                        &policy,
                    )
                    .map_err(|e| failure("sweep.invalid", "solid.sweep", e.to_string()))?,
                ),
                Node::Loft {
                    sections,
                    placement,
                } => Model::Loft(
                    Loft::from_profiles(
                        sections
                            .iter()
                            .map(|s| (s.placement.frame(), &profiles[s.profile as usize].geometry)),
                        placement.frame(),
                        &LoftPolicy::new(policy),
                    )
                    .map_err(|e| failure("loft.invalid", "solid.loft", e.to_string()))?,
                ),
                Node::AxialFinish {
                    base,
                    feature,
                    treatment,
                    size,
                    transition,
                } => {
                    let tag = labels.get(&feature).ok_or_else(|| {
                        failure(
                            "selection.missing",
                            "solid.axialFinish",
                            format!("No feature named '{feature}' was authored"),
                        )
                    })?;
                    let source = models[base as usize]
                        .extrusion()
                        .map_err(|e| failure(e.code, "solid.axialFinish", e.message))?;
                    let edges: Vec<_> = source
                        .profile_axial_edges_tagged(*tag)
                        .map(|(id, _)| id)
                        .collect();
                    if edges.is_empty() {
                        return Err(failure(
                            "selection.missing",
                            "solid.axialFinish",
                            format!("No profile axial edges named '{feature}' exist"),
                        ));
                    }
                    let treatment = match treatment {
                        FinishKind::Fillet => CornerTreatment::fillet(size),
                        FinishKind::Chamfer => CornerTreatment::chamfer(size),
                    }
                    .tagged(labels[&transition]);
                    Model::Extrusion(
                        source
                            .with_axial_corners(edges, treatment, &policy)
                            .map_err(|e| {
                                failure("finish.invalid", "solid.axialFinish", e.to_string())
                            })?
                            .solid,
                    )
                }
                Node::Extrude {
                    profile,
                    height,
                    placement,
                } => Model::Extrusion(
                    Extrusion::from_profile(
                        &profiles[profile as usize].geometry,
                        placement.frame(),
                        height,
                        &policy,
                    )
                    .map_err(|e| failure("extrude.invalid", "solid.extrude", e.to_string()))?,
                ),
                Node::SphericalCap {
                    radius,
                    height,
                    placement,
                } => Model::Revolution(
                    Revolution::spherical_cap(radius, height, placement.frame(), &policy)
                        .map_err(|error| spherical_cap_error(error, i, radius, height))?,
                ),
                Node::Revolve {
                    profile,
                    angle,
                    placement,
                } => Model::Revolution(
                    Revolution::new(
                        profiles[profile as usize].geometry.clone(),
                        placement.frame(),
                        angle,
                    )
                    .map_err(|e| failure("revolve.invalid", "solid.revolve", e.to_string()))?,
                ),
                Node::Pocket {
                    base,
                    profile,
                    end,
                    depth,
                    at,
                } => {
                    let feature = profiles[profile as usize].feature;
                    let profile = profiles[profile as usize]
                        .geometry
                        .translated(at, &policy)
                        .map_err(|e| failure("pocket.profile", "solid.pocket", e.to_string()))?;
                    let mut pocket = Pocket::from_profile(&profile, end.into(), depth)
                        .map_err(|e| failure("pocket.profile", "solid.pocket", e.to_string()))?;
                    if let Some(feature) = feature {
                        pocket = pocket.tagged(feature);
                    }
                    Model::Extrusion(
                        models[base as usize]
                            .extrusion()
                            .map_err(|e| failure(e.code, "solid.pocket", e.message))?
                            .clone()
                            .with_pocket(pocket, &policy)
                            .map_err(|e| {
                                failure("pocket.invalid", "solid.pocket", e.to_string())
                            })?,
                    )
                }
                Node::OnFace {
                    host,
                    face,
                    profile,
                    height,
                    offset,
                    at,
                } => {
                    let geometry = models[host as usize].geometry();
                    let face = match face {
                        FaceSelector::Cap { end } => geometry.cap(end.into()).ok_or_else(|| {
                            failure(
                                "selection.missingCap",
                                "workplane.cap",
                                "This construction end has no cap".into(),
                            )
                        })?,
                        FaceSelector::Floor { feature } => {
                            let Some(tag) = labels.get(&feature) else {
                                return Err(failure(
                                    "selection.missing",
                                    "workplane.floor",
                                    format!("No feature named '{feature}' was authored"),
                                ));
                            };
                            models[host as usize]
                                .extrusion()
                                .map_err(|e| failure(e.code, "workplane.floor", e.message))?
                                .floor_tagged(*tag)
                                .map_err(|e| {
                                    failure(
                                        "selection.floor",
                                        "workplane.floor",
                                        format!("Feature '{feature}': {e}"),
                                    )
                                })?
                        }
                    };
                    let frame = geometry
                        .planar_frame(face)
                        .map_err(|e| failure("workplane.invalid", "workplane", e.to_string()))?;
                    let plane = Workplane::new(frame)
                        .map_err(|e| failure("workplane.invalid", "workplane", e.to_string()))?
                        .offset(offset)
                        .map_err(|e| {
                            failure("workplane.invalid", "workplane.offset", e.to_string())
                        })?
                        .translated(at)
                        .map_err(|e| {
                            failure("workplane.invalid", "workplane.translated", e.to_string())
                        })?;
                    Model::Extrusion(
                        plane
                            .extrude_profile(&profiles[profile as usize].geometry, height, &policy)
                            .map_err(|e| {
                                failure("extrude.invalid", "workplane.extrude", e.to_string())
                            })?,
                    )
                }
            };
            models.push(solid);
        }
        let machining_by_node: BTreeMap<_, _> = models
            .iter()
            .enumerate()
            .filter_map(|(id, model)| {
                if let Model::Body {
                    machining,
                    operations,
                    integrals,
                    ..
                } = model
                    && (!machining.is_empty() || !operations.is_empty())
                {
                    return Some((
                        id,
                        MachiningEvidence {
                            reports: machining.clone(),
                            operations: operations.clone(),
                            integrals: *integrals,
                        },
                    ));
                }
                None
            })
            .collect();
        let machining = self
            .roots
            .iter()
            .filter_map(|root| {
                machining_by_node
                    .get(&(root.node as usize))
                    .map(|evidence| (root.name.clone(), evidence.clone()))
            })
            .collect();
        let wanted: BTreeSet<_> = self.roots.iter().map(|r| r.node as usize).collect();
        let retained: BTreeMap<_, _> = models
            .into_iter()
            .enumerate()
            .filter(|(i, _)| wanted.contains(i))
            .map(|(i, solid)| (i, solid.install()))
            .collect();
        let roots = self
            .roots
            .into_iter()
            .map(|root| (root.name, retained[&(root.node as usize)].clone()))
            .collect();
        Ok(Built {
            units: self.units,
            separation: self.separation,
            roots,
            machining,
            names: labels
                .into_iter()
                .map(|(name, tag)| (tag.0, name))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set_operation_request(kind: &str, translation: [f64; 3]) -> serde_json::Value {
        serde_json::json!({"units":"mm","separation":1e-9,
            "profiles":[{"kind":"rectangle","width":10.,"height":10.},{"kind":"tagged","profile":0,"name":"left"},{"kind":"tagged","profile":0,"name":"right"}],
            "nodes":[{"kind":"extrude","profile":1,"height":10.},{"kind":"extrude","profile":2,"height":10.,"placement":{"translation":translation}},{"kind":kind,"base":0,"other":1}],
            "roots":[{"name":"result","node":2}]})
    }
    #[test]
    fn rectilinear_graph_reports_exact_integrals_tags_and_local_history() {
        for (kind, volume, area) in [
            ("union", 1500., 800.),
            ("intersect", 500., 400.),
            ("subtract", 500., 400.),
        ] {
            let request: Request =
                serde_json::from_value(set_operation_request(kind, [5., 0., 0.])).unwrap();
            let built = request.build().unwrap();
            let evidence = &built.machining["result"];
            assert_eq!(evidence.integrals, Some([volume, area]));
            let AuthoredSolidOperation::Rectilinear {
                node,
                left,
                right,
                report,
            } = &evidence.operations[0]
            else {
                panic!("Boolean report")
            };
            assert_eq!((*node, *left, *right), (2, 0, 1));
            assert_eq!(report.material_components, 1);
            assert_eq!(report.cavities, 0);
            let body = built.roots[0].1.geometry().to_body();
            for (face, sources) in body.faces().iter().zip(&report.face_sources) {
                let expected: BTreeSet<_> = sources
                    .iter()
                    .filter(|source| source.face.index() >= 2)
                    .map(|source| match source.operand {
                        exedra_analytic::rectilinear::Operand::Left => SegTag(1),
                        exedra_analytic::rectilinear::Operand::Right => SegTag(2),
                    })
                    .collect();
                assert_eq!(face.role.iter().copied().collect::<BTreeSet<_>>(), expected);
            }
        }
    }
    #[test]
    fn shared_rotated_placement_is_applied_once_after_local_boolean() {
        let mut value = set_operation_request("union", [0., 0., 0.]);
        value["nodes"][0]["placement"] =
            serde_json::json!({"translation":[20.,30.,40.],"rotationXYZ":[0.,0.,0.7]});
        value["nodes"][1]["placement"] =
            serde_json::json!({"translation":[20.,30.,45.],"rotationXYZ":[0.,0.,0.7]});
        let built = serde_json::from_value::<Request>(value)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(built.machining["result"].integrals, Some([1500., 800.]));
        let frame = built.roots[0].1.geometry().frame();
        assert_eq!(frame.rows.map(|row| row[3]), [20., 30., 40.]);
        let body = built.roots[0].1.geometry().to_body();
        assert_eq!(
            body.vertices()
                .iter()
                .map(|point| point[2])
                .fold(f64::NEG_INFINITY, f64::max),
            15.
        );
    }
    #[test]
    fn boolean_frames_empty_results_and_curve_refusals_are_explicit() {
        let mut value = set_operation_request("union", [5., 0., 0.]);
        value["nodes"][1]["placement"]["rotationXYZ"] =
            serde_json::json!([0., 0., core::f64::consts::FRAC_PI_2]);
        let error = serde_json::from_value::<Request>(value)
            .unwrap()
            .build()
            .unwrap_err();
        assert_eq!(error.code, "boolean.frameMismatch");
        assert_eq!(error.context.unwrap().operand, Some("right"));
        let error =
            serde_json::from_value::<Request>(set_operation_request("intersect", [20., 0., 0.]))
                .unwrap()
                .build()
                .unwrap_err();
        assert_eq!(error.code, "boolean.empty");
        assert_eq!(error.operation, "solid.intersect");
        let mut value = set_operation_request("union", [0., 0., 0.]);
        value["profiles"][0] = serde_json::json!({"kind":"circle","radius":5.});
        let error = serde_json::from_value::<Request>(value)
            .unwrap()
            .build()
            .unwrap_err();
        assert_eq!(error.code, "boolean.unsupportedFace");
        assert!(error.context.unwrap().face.is_some());
    }
    #[test]
    fn halfspace_graph_preserves_source_tags_and_labels_the_new_cap() {
        let mut value = set_operation_request("union", [5., 0., 0.]);
        value["nodes"][2] = serde_json::json!({"kind":"intersectHalfSpace","base":0,"point":[0.,0.,5.],"normal":[0.,0.,1.],"feature":"cut"});
        let built = serde_json::from_value::<Request>(value.clone())
            .unwrap()
            .build()
            .unwrap();
        let evidence = &built.machining["result"];
        assert_eq!(evidence.integrals, None);
        let AuthoredSolidOperation::HalfSpace { report, .. } = &evidence.operations[0] else {
            panic!("halfspace report")
        };
        assert_eq!(
            report.outcome,
            exedra_analytic::halfspace::ClipOutcome::Clipped
        );
        let body = built.roots[0].1.geometry().to_body();
        let cap = report.cap.unwrap();
        assert_eq!(body.faces()[cap.index()].role, alloc::vec![SegTag(1)]);
        for face in &report.faces {
            if let Some(result) = face.result {
                assert_eq!(
                    body.faces()[result.index()].role,
                    if face.source.index() >= 2 {
                        alloc::vec![SegTag(2)]
                    } else {
                        Vec::new()
                    }
                );
            }
        }
        value["nodes"][2]["point"] = serde_json::json!([0., 0., -1.]);
        assert_eq!(
            serde_json::from_value::<Request>(value)
                .unwrap()
                .build()
                .unwrap_err()
                .code,
            "halfspace.empty"
        );
    }
    #[test]
    fn spherical_caps_preserve_dimensions_and_remain_native_revolutions() {
        let input = |radius, height| serde_json::json!({"units":"mm","separation":1e-9,"profiles":[],"nodes":[{"kind":"sphericalCap","radius":radius,"height":height}],"roots":[{"name":"dome","node":0}]});
        let mut failures = Vec::new();
        let cases = [(25.0, 0.25), (25.0, 12.0), (25.0, 40.0)]
            .into_iter()
            .chain([2.0_f64, 25.0, 60.0].into_iter().flat_map(|radius| {
                [0.5, radius / 2.0, radius, 1.5 * radius, 2.0 * radius]
                    .into_iter()
                    .map(move |height| (radius, height))
            }));
        for (radius, height) in cases {
            let request: Request = serde_json::from_value(input(radius, height)).unwrap();
            let built = match request.build() {
                Ok(built) => built,
                Err(error) => {
                    failures.push((radius, height, error));
                    continue;
                }
            };
            let NativeGeometryRef::Revolution(cap) = built.roots[0].1.geometry() else {
                panic!("cap is an ordinary native revolution")
            };
            assert!(cap.body().is_closed());
            let volume = core::f64::consts::PI * height * height * (radius - height / 3.0);
            assert!((cap.volume() - volume).abs() <= volume * 2e-12);
            assert_eq!(cap.profile().bounds()[0][1], 0.0);
            assert!((cap.profile().bounds()[1][1] - height).abs() < 1e-12);
        }
        assert!(
            failures.is_empty(),
            "valid native cap refusals: {failures:?}"
        );
        for (height, code, offset) in [
            (51.0, "revolve.cap.height", None),
            (1e-20, "revolve.cap.heightResolution", Some(25.0)),
        ] {
            let request: Request = serde_json::from_value(input(25.0, height)).unwrap();
            let error = request.build().unwrap_err();
            assert_eq!(error.code, code);
            let context = error.context.unwrap();
            assert_eq!(context.node, Some(0));
            assert_eq!(context.radius, Some(25.0));
            assert_eq!(context.height, Some(height));
            assert_eq!(context.axis_offset, offset);
        }
    }
    fn request(nodes: serde_json::Value) -> Request {
        serde_json::from_value(serde_json::json!({"units":"mm","separation":1e-6,"profiles":[{"kind":"rectangle","width":40,"height":30}],"nodes":nodes,"roots":[{"name":"panel","node":0}]})).unwrap()
    }
    #[test]
    fn centered_curve_profile_lowers_once_with_native_tags_and_typed_radial_refusal() {
        let input = |end_y| {
            serde_json::json!({
                "units":"mm","separation":1e-6,
                "profiles":[{"kind":"loop","segments":[
                    {"kind":"line","to":[3,0]}, {"kind":"line","to":[4,0]},
                    {"kind":"arc","to":[3,end_y],"center":[3,0],"clockwise":false,"feature":"rim"}
                ]}],
                "nodes":[{"kind":"extrude","profile":0,"height":2},{"kind":"revolve","profile":0,"angle":1.7}],
                "roots":[{"name":"plate","node":0},{"name":"turned","node":1}]
            })
        };
        let request: Request = serde_json::from_value(input(1.0)).unwrap();
        let built = request.build().unwrap();
        assert_eq!(built.roots.len(), 2);
        let tag = SegTag(
            *built
                .names
                .iter()
                .find(|(_, name)| name.as_str() == "rim")
                .unwrap()
                .0,
        );
        for (_, source) in &built.roots {
            assert_eq!(
                source
                    .face_tags(match source.geometry() {
                        NativeGeometryRef::Extrusion(v) => v.faces_tagged(tag).next().unwrap().0,
                        NativeGeometryRef::Revolution(v) => v.faces_tagged(tag).next().unwrap(),
                        _ => panic!("expected authored family"),
                    })
                    .unwrap()
                    .collect::<Vec<_>>(),
                alloc::vec![tag]
            );
        }
        let bad: Request = serde_json::from_value(input(2.0)).unwrap();
        let error = bad.build().unwrap_err();
        assert_eq!(error.code, "profile.arc.endpointRadii");
        let context = error.context.unwrap();
        assert_eq!(context.profile, Some(0));
        assert_eq!(context.segment, Some(2));
        assert_eq!(context.feature.as_deref(), Some("rim"));
        assert_eq!(context.start_radius, Some(1.0));
        assert_eq!(context.end_radius, Some(2.0));
        assert!(context.radius_roundoff_bound.unwrap() < 1e-12);
    }
    #[test]
    fn graph_validation_precedes_geometry_work_and_refuses_cycles() {
        let value = request(
            serde_json::json!([{"kind":"pocket","base":0,"profile":0,"depth":1,"end":"end","at":[0,0]}]),
        );
        assert_eq!(value.validate().unwrap_err().code, "graph.nodeReference");
        let value = request(serde_json::json!([{"kind":"extrude","profile":9,"height":3}]));
        assert_eq!(value.validate().unwrap_err().code, "graph.profileReference");
    }
    #[test]
    fn native_build_preserves_exact_measures() {
        let built = request(serde_json::json!([{"kind":"extrude","profile":0,"height":3}]))
            .build()
            .unwrap();
        assert_eq!(built.roots[0].1.as_extrusion().unwrap().volume(), 3600.0);
    }
    #[test]
    fn sweep_loft_and_named_finishes_lower_to_existing_native_constructors() {
        let value: Request=serde_json::from_value(serde_json::json!({
            "units":"mm","separation":1e-6,
            "profiles":[{"kind":"circle","radius":3},{"kind":"rectangle","width":20,"height":16},{"kind":"tagged","profile":1,"name":"body"}],
            "paths":[{"start":[0,0,0],"tangent":[0,1,0],"sectionX":[1,0,0],"steps":[{"kind":"lineBy","offset":[0,30,0]},{"kind":"tangentArc","radius":20,"angle":core::f64::consts::FRAC_PI_2,"normal":[0,0,1]}]}],
            "nodes":[
                {"kind":"sweep","profile":0,"path":0},
                {"kind":"loft","sections":[{"profile":2,"placement":{}},{"profile":2,"placement":{"translation":[3,0,30],"rotationXYZ":[0,0,0.15]}}]},
                {"kind":"extrude","profile":2,"height":12},
                {"kind":"axialFinish","base":2,"feature":"body","treatment":"fillet","size":2,"transition":"round"}
            ],
            "roots":[{"name":"rail","node":0},{"name":"loft","node":1},{"name":"finished","node":3}]
        })).unwrap();
        let built = value.build().unwrap();
        assert!(matches!(
            built.roots[0].1.geometry(),
            NativeGeometryRef::Sweep(_)
        ));
        assert!(matches!(
            built.roots[1].1.geometry(),
            NativeGeometryRef::Loft(_)
        ));
        assert!(matches!(
            built.roots[1].1.volume(),
            Err(exedra_compile::NativeMeasureError::Unavailable { .. })
        ));
        let solid = built.roots[2].1.as_extrusion().unwrap();
        let tag = built
            .names
            .iter()
            .find(|(_, name)| name.as_str() == "round")
            .unwrap()
            .0;
        assert_eq!(solid.faces_tagged(SegTag(*tag)).count(), 4);
    }
    #[test]
    fn path_and_section_budgets_precede_native_construction() {
        let mut value = request(serde_json::json!([{"kind":"sweep","profile":0,"path":9}]));
        assert_eq!(value.validate().unwrap_err().code, "graph.pathReference");
        value.nodes = alloc::vec![Node::Loft {
            sections: Vec::new(),
            placement: InputPlacement::default()
        }];
        assert_eq!(value.validate().unwrap_err().code, "budget.sections");
    }
    #[test]
    fn generic_cap_attachments_use_borrowed_native_frames_without_installing_transient_sources() {
        for (kind, origin) in [("sweep", [0.0, 20.5, 0.0]), ("loft", [0.0, 0.0, 20.5])] {
            let host = if kind == "sweep" {
                serde_json::json!({"kind":"sweep","profile":0,"path":0})
            } else {
                serde_json::json!({"kind":"loft","sections":[{"profile":0,"placement":{}},{"profile":0,"placement":{"translation":[0,0,20]}}]})
            };
            let value:Request=serde_json::from_value(serde_json::json!({"units":"mm","separation":1e-6,
                "profiles":[{"kind":"circle","radius":3},{"kind":"circle","radius":1}],
                "paths":[{"start":[0,0,0],"tangent":[0,1,0],"sectionX":[1,0,0],"steps":[{"kind":"lineBy","offset":[0,20,0]}]}],
                "nodes":[host,{"kind":"onFace","host":0,"face":{"kind":"cap","end":"end"},"profile":1,"height":5,"offset":0.5,"at":[0,0]}],
                "roots":[{"name":"attached","node":1}]
            })).unwrap();
            let built = value.build().unwrap();
            assert_eq!(built.roots.len(), 1);
            let part = built.roots[0].1.as_extrusion().unwrap();
            assert_eq!(part.frame().transform_point([0.0; 3]), origin);
            assert!((part.volume() - 5.0 * core::f64::consts::PI).abs() < 1e-12);
        }
        let value:Request=serde_json::from_value(serde_json::json!({"units":"mm","separation":1e-6,
            "profiles":[{"kind":"rectangle","width":4,"height":4},{"kind":"translated","profile":0,"offset":[10,2]}],
            "nodes":[{"kind":"revolve","profile":1,"angle":core::f64::consts::TAU},{"kind":"onFace","host":0,"face":{"kind":"cap","end":"end"},"profile":0,"height":2,"offset":0,"at":[0,0]}],
            "roots":[{"name":"attached","node":1}]
        })).unwrap();
        let error = value.build().unwrap_err();
        assert_eq!(error.code, "selection.missingCap");
        assert_eq!(error.context.as_ref().and_then(|c| c.node), Some(1));
    }
    fn corner_request(treatment: &str, size: f64, point: [f64; 2]) -> serde_json::Value {
        serde_json::json!({"units":"mm","separation":1e-6,
            "profiles":[
                {"kind":"polygon","points":[[7,0],[22,0],[22,5],[18,5],[18,30],[7,30]]},
                {"kind":"tagged","profile":0,"name":"body"},
                {"kind":"corners","profile":1,"at":[point],"treatment":treatment,"size":size,"feature":"shoulder"},
                {"kind":"translated","profile":2,"offset":[2,1]},
                {"kind":"rectangle","width":100,"height":100},
                {"kind":"cutouts","profile":4,"template":3,"positions":[[-20,-10],[5,-10]]}
            ],
            "paths":[{"start":[0,0,0],"tangent":[0,1,0],"sectionX":[1,0,0],"steps":[{"kind":"lineBy","offset":[0,20,0]}]}],
            "nodes":[
                {"kind":"extrude","profile":3,"height":5},
                {"kind":"revolve","profile":3,"angle":core::f64::consts::TAU},
                {"kind":"sweep","profile":3,"path":0},
                {"kind":"loft","sections":[{"profile":3,"placement":{}},{"profile":3,"placement":{"translation":[0,0,20]}}]},
                {"kind":"extrude","profile":5,"height":3}
            ],"roots":[{"name":"plate","node":0},{"name":"turn","node":1},{"name":"rail","node":2},{"name":"loft","node":3},{"name":"cutoutPlate","node":4}]
        })
    }
    #[test]
    fn one_treated_shared_profile_drives_all_native_families_and_placed_cutouts() {
        for treatment in ["fillet", "chamfer"] {
            let request: Request =
                serde_json::from_value(corner_request(treatment, 2.0, [18.0, 5.0])).unwrap();
            let built = request.build().unwrap();
            assert_eq!(built.roots.len(), 5);
            let area = 350.0
                + if treatment == "fillet" {
                    4.0 - core::f64::consts::PI
                } else {
                    2.0
                };
            assert!((built.roots[0].1.volume().unwrap() - 5.0 * area).abs() < 1e-8);
            assert!((built.roots[2].1.volume().unwrap() - 20.0 * area).abs() < 1e-8);
            assert!(
                (built.roots[4].1.volume().unwrap() - 3.0 * (10000.0 - 2.0 * area)).abs() < 1e-8
            );
            for ((_, native), surface) in built.roots.iter().take(4).zip(if treatment == "fillet" {
                ["cylinder", "torus", "cylinder", "ruled"]
            } else {
                ["plane", "cone", "plane", "ruled"]
            }) {
                let metadata = crate::inspect::metadata(native, &built.names).unwrap();
                let generated: Vec<_> = metadata["faces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|face| {
                        face["features"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|name| name == "shoulder")
                    })
                    .collect();
                assert_eq!(
                    generated.len(),
                    1,
                    "one selected corner yields one labelled transition"
                );
                assert_eq!(generated[0]["surface"], surface);
            }
            if treatment == "fillet" {
                let NativeGeometryRef::Revolution(turn) = built.roots[1].1.geometry() else {
                    panic!("native revolution")
                };
                let torus = turn
                    .body()
                    .faces()
                    .iter()
                    .find_map(|face| {
                        if let exedra_analytic::geometry::Surface::Torus(t) = &face.surface {
                            Some(t)
                        } else {
                            None
                        }
                    })
                    .unwrap();
                assert!((torus.major_radius() - 22.0).abs() < 1e-12);
                assert!((torus.minor_radius() - 2.0).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn corner_refusals_keep_query_and_limiting_edge_evidence_without_nonfinite_json() {
        for (point, size, expected) in [
            ([18.0, 5.0], 5.0, "profile.corner.overlap"),
            ([99.0, 99.0], 1.0, "profile.corner.missing"),
            ([18.0, 5.0], 0.0, "profile.corner.size"),
            ([18.0, 5.0], 1e308, "profile.corner.overlap"),
        ] {
            let value: Request =
                serde_json::from_value(corner_request("fillet", size, point)).unwrap();
            let error = value.build().unwrap_err();
            let json = serde_json::to_value(error).unwrap();
            assert_eq!(json["code"], expected);
            assert_eq!(json["profile"], 2);
            assert_eq!(json["operation"], "profile.corners");
            if expected == "profile.corner.overlap" {
                assert!(json["segmentLength"].as_f64().unwrap().is_finite());
                assert!(json["requestedTrim"].as_f64().unwrap().is_finite());
                assert_eq!(json["feature"], "shoulder");
            }
        }
        let mut request = corner_request("fillet", 1.0, [18.0, 5.0]);
        request["profiles"][2]["tolerance"] = serde_json::json!(100);
        let request: Request = serde_json::from_value(request).unwrap();
        let json = serde_json::to_value(request.build().unwrap_err()).unwrap();
        assert_eq!(json["code"], "profile.corner.ambiguous");
        assert_eq!(json["cornerCandidates"].as_array().unwrap().len(), 2);
        let mut request = corner_request("fillet", 1e308, [22.0, 0.0]);
        request["profiles"][0]["points"] =
            serde_json::json!([[7, 0], [22, 0], [7.01, 0.001], [7, 30]]);
        let request: Request = serde_json::from_value(request).unwrap();
        let json = serde_json::to_value(request.build().unwrap_err()).unwrap();
        assert_eq!(json["code"], "profile.corner.numericLimit");
        assert!(
            json.get("requestedTrim").is_none(),
            "non-finite numeric evidence is absent, never number-typed null"
        );
        let mut request = corner_request("fillet", 1.0, [10.0, 0.0]);
        request["profiles"][0] = serde_json::json!({"kind":"circle","radius":10});
        let request: Request = serde_json::from_value(request).unwrap();
        assert_eq!(request.build().unwrap_err().code, "profile.corner.smooth");
        let mut request = corner_request("fillet", 1.0, [18.0, 5.0]);
        request["profiles"][2]["at"] = serde_json::json!([[18, 5], [18, 5]]);
        let request: Request = serde_json::from_value(request).unwrap();
        assert_eq!(request.build().unwrap_err().code, "profile.corner.repeated");
    }
}
