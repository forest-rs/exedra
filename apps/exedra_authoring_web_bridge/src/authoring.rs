// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounded acyclic authoring requests lowered once into native constructors.

use crate::Error;
use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec::Vec,
};
use exedra_analytic::extrusion::{End, Extrusion, Pocket};
use exedra_compile::NativeSource;
use exedra_constructive::{builders, profile::Profile2};
use exedra_math::Placement3;
use exedra_profile::{ProfilePolicy, SegTag};
use serde::Deserialize;

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
enum FaceSelector {
    Floor { feature: String },
    Cap { end: InputEnd },
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Node {
    Extrude {
        profile: u32,
        height: f64,
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
    nodes: Vec<Node>,
    roots: Vec<Root>,
}
#[derive(Debug)]
pub(crate) struct Built {
    pub units: Units,
    pub separation: f64,
    pub roots: Vec<(String, NativeSource)>,
    pub names: BTreeMap<u32, String>,
}
fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128
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
                ProfileNode::Tagged { profile, .. } | ProfileNode::Translated { profile, .. } => {
                    counts[*profile as usize]
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
            let (profile, parent) = match node {
                Node::Extrude { profile, .. } => (*profile, None),
                Node::Pocket { profile, base, .. } => (*profile, Some(*base)),
                Node::OnFace { profile, host, .. } => (*profile, Some(*host)),
            };
            if profile as usize >= self.profiles.len() {
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
                depths[parent as usize] + 1
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
            let count =
                counts[profile as usize].saturating_add(if matches!(node, Node::Pocket { .. }) {
                    body_counts[parent.expect("pocket parent") as usize]
                } else {
                    0
                });
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
            geometry: Profile2,
            feature: Option<SegTag>,
        }
        let mut profiles: Vec<AuthoredProfile> = Vec::with_capacity(self.profiles.len());
        for (i, node) in self.profiles.into_iter().enumerate() {
            let failure = |message: String| {
                Error::new("profile.invalid", "profile.build", message).profile(i)
            };
            let mut feature = None;
            let geometry = match node {
                ProfileNode::Rectangle { width, height } => builders::rect_centered(width, height)
                    .map(|p| p.tagged(UNNAMED))
                    .map_err(|e| failure(e.to_string()))?,
                ProfileNode::RoundedRectangle {
                    width,
                    height,
                    radius,
                } => builders::rounded_rect_centered(width, height, radius)
                    .map(|p| p.tagged(UNNAMED))
                    .map_err(|e| failure(e.to_string()))?,
                ProfileNode::Circle { radius } => builders::circle(radius)
                    .map(|p| p.tagged(UNNAMED))
                    .map_err(|e| failure(e.to_string()))?,
                ProfileNode::Slot { length, radius } => builders::slot_centered(length, radius)
                    .map(|p| p.tagged(UNNAMED))
                    .map_err(|e| failure(e.to_string()))?,
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
                        .translated((offset[0], offset[1]))
                        .map_err(|e| failure(e.to_string()))?
                }
                ProfileNode::Cutouts {
                    profile,
                    template,
                    positions,
                } => profiles[profile as usize]
                    .geometry
                    .clone()
                    .with_cutouts_at(
                        &profiles[template as usize].geometry,
                        positions.into_iter().map(|p| (p[0], p[1])),
                    )
                    .map_err(|e| failure(e.to_string()))?,
            };
            profiles.push(AuthoredProfile { geometry, feature });
        }
        let mut models: Vec<Extrusion> = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.into_iter().enumerate() {
            let failure = |code: &'static str, operation: &'static str, message: String| {
                Error::new(code, operation, message).node(i)
            };
            let solid = match node {
                Node::Extrude { profile, height } => Extrusion::from_profile(
                    &profiles[profile as usize].geometry,
                    Placement3::IDENTITY,
                    height,
                    &policy,
                )
                .map_err(|e| failure("extrude.invalid", "solid.extrude", e.to_string()))?,
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
                        .translated((at[0], at[1]))
                        .map_err(|e| failure("pocket.profile", "solid.pocket", e.to_string()))?;
                    let native = profile
                        .to_line_arc(&policy)
                        .map_err(|e| failure("pocket.profile", "solid.pocket", e.to_string()))?;
                    let mut pocket = Pocket::from_profile(&native, end.into(), depth)
                        .map_err(|e| failure("pocket.profile", "solid.pocket", e.to_string()))?;
                    if let Some(feature) = feature {
                        pocket = pocket.tagged(feature);
                    }
                    models[base as usize]
                        .clone()
                        .with_pocket(pocket, &policy)
                        .map_err(|e| failure("pocket.invalid", "solid.pocket", e.to_string()))?
                }
                Node::OnFace {
                    host,
                    face,
                    profile,
                    height,
                    offset,
                    at,
                } => {
                    let host = &models[host as usize];
                    let face = match face {
                        FaceSelector::Cap { end } => host.cap(end.into()),
                        FaceSelector::Floor { feature } => {
                            let Some(tag) = labels.get(&feature) else {
                                return Err(failure(
                                    "selection.missing",
                                    "workplane.floor",
                                    format!("No feature named '{feature}' was authored"),
                                ));
                            };
                            host.floor_tagged(*tag).map_err(|e| {
                                failure(
                                    "selection.floor",
                                    "workplane.floor",
                                    format!("Feature '{feature}': {e}"),
                                )
                            })?
                        }
                    };
                    let plane = host
                        .workplane(face)
                        .map_err(|e| failure("workplane.invalid", "workplane", e.to_string()))?
                        .offset(offset)
                        .map_err(|e| {
                            failure("workplane.invalid", "workplane.offset", e.to_string())
                        })?
                        .translated(at)
                        .map_err(|e| {
                            failure("workplane.invalid", "workplane.translated", e.to_string())
                        })?;
                    plane
                        .extrude_profile(&profiles[profile as usize].geometry, height, &policy)
                        .map_err(|e| {
                            failure("extrude.invalid", "workplane.extrude", e.to_string())
                        })?
                }
            };
            models.push(solid);
        }
        let wanted: BTreeSet<_> = self.roots.iter().map(|r| r.node as usize).collect();
        let retained: BTreeMap<_, _> = models
            .into_iter()
            .enumerate()
            .filter(|(i, _)| wanted.contains(i))
            .map(|(i, solid)| (i, NativeSource::from(solid)))
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
    fn request(nodes: serde_json::Value) -> Request {
        serde_json::from_value(serde_json::json!({"units":"mm","separation":1e-6,"profiles":[{"kind":"rectangle","width":40,"height":30}],"nodes":nodes,"roots":[{"name":"panel","node":0}]})).unwrap()
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
}
