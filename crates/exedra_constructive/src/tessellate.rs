// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Body tessellation: constructive bodies into Exedra meshes.
//!
//! Tessellation is deterministic (identical inputs and policy produce
//! bit-identical meshes on every platform) and provenance-carrying: every
//! produced face records the feature that generated it, down to profile
//! segment granularity.
//!
//! The f64 construction domain narrows to f32 exactly once, here, at vertex
//! emission (`as f32`, round-to-nearest-even).

use alloc::vec::Vec;

use exedra::{FaceBuildAttrs, MeshBuilder};
use exedra_triangulate::{PolygonInput, TriParams, triangulate};

use crate::discretize::{DiscretizePolicy, DiscretizedProfile, discretize_profile};
use crate::ir::{CapMode, Placement3};
use crate::len_u32;
use crate::profile::Profile2;

/// Evaluation policy shared by body tessellation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct EvalPolicy {
    /// Curve discretization policy.
    pub discretize: DiscretizePolicy,
    /// Threshold on `|sin(turn angle)|` above which a profile corner
    /// authors a sharp lateral edge. Tangent-continuous junctions (arcs
    /// meeting lines smoothly) fall below any sensible threshold and stay
    /// smooth; square corners exceed it and crease.
    pub sharp_sin_threshold: f64,
}

impl Default for EvalPolicy {
    fn default() -> Self {
        Self {
            discretize: DiscretizePolicy::default(),
            sharp_sin_threshold: 0.1,
        }
    }
}

/// The feature of a body that produced a mesh element.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// The start cap (profile plane, facing local -Z for extrusions).
    CapStart,
    /// The end cap.
    CapEnd,
    /// A side-wall face. `loop_index` 0 is the outer loop, `1 + i` is hole
    /// `i`; `seg` is the source segment index within that loop.
    Wall {
        /// Which profile loop: 0 = outer, `1 + i` = hole `i`.
        loop_index: u16,
        /// Source segment index within the loop.
        seg: u32,
    },
}

/// A tessellated body: the mesh plus its element provenance.
#[derive(Debug)]
pub struct TessellatedBody {
    /// The tessellated mesh.
    pub mesh: exedra::Mesh,
    /// Element provenance, pinned to the mesh's revision.
    pub source_map: crate::source_map::SourceMap,
}

/// Region values written into [`exedra::attr::FACE_REGION`].
///
/// Stable, documented mapping: `0` = start cap, `1` = end cap, `2 + k` =
/// side wall of global segment `k` (outer loop segments first, then each
/// hole's segments in order).
pub const REGION_CAP_START: u32 = 0;
/// End-cap region value.
pub const REGION_CAP_END: u32 = 1;
/// First side-wall region value; segment `k` maps to `REGION_WALL_BASE + k`.
pub const REGION_WALL_BASE: u32 = 2;

/// Typed tessellation failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TessellateError {
    /// Discretization failed (invalid policy).
    Discretize(crate::discretize::DiscretizeError),
    /// Cap triangulation failed; the profile was not simple after
    /// discretization.
    Triangulate(exedra_triangulate::TriError),
    /// Mesh construction failed (an internal invariant violation).
    Build(exedra::BuildError),
    /// A revolved profile touches or crosses the revolution axis; axis
    /// contact is out of scope for v1 (use primitives for solid cylinders
    /// and spheres).
    AxisContact {
        /// The smallest radius found in the discretized profile.
        min_radius: f64,
    },
}

impl core::fmt::Display for TessellateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Discretize(e) => write!(f, "discretization failed: {e}"),
            Self::Triangulate(e) => write!(f, "cap triangulation failed: {e}"),
            Self::Build(e) => write!(f, "mesh construction failed: {e:?}"),
            Self::AxisContact { min_radius } => write!(
                f,
                "revolved profile touches the axis (min radius {min_radius})"
            ),
        }
    }
}

impl core::error::Error for TessellateError {}

impl From<crate::discretize::DiscretizeError> for TessellateError {
    fn from(e: crate::discretize::DiscretizeError) -> Self {
        Self::Discretize(e)
    }
}

impl From<exedra_triangulate::TriError> for TessellateError {
    fn from(e: exedra_triangulate::TriError) -> Self {
        Self::Triangulate(e)
    }
}

impl From<exedra::BuildError> for TessellateError {
    fn from(e: exedra::BuildError) -> Self {
        Self::Build(e)
    }
}

fn apply_placement(p: &Placement3, v: [f64; 3]) -> [f64; 3] {
    let r = &p.rows;
    [
        r[0][0] * v[0] + r[0][1] * v[1] + r[0][2] * v[2] + r[0][3],
        r[1][0] * v[0] + r[1][1] * v[1] + r[1][2] * v[2] + r[1][3],
        r[2][0] * v[0] + r[2][1] * v[1] + r[2][2] * v[2] + r[2][3],
    ]
}

/// The single documented f64 -> f32 narrowing point.
fn narrow(v: [f64; 3]) -> [f32; 3] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the deliberate f64->f32 narrowing at mesh emission (ADR-0001)"
    )]
    {
        [v[0] as f32, v[1] as f32, v[2] as f32]
    }
}

/// Tessellates an extrusion: the profile's local XY plane extruded along
/// local +Z by `height`, placed by `placement`.
///
/// Caps are pre-triangulated for concave or holed profiles (single ngons
/// for convex hole-free ones); side walls are one quad per discretized
/// edge, with `FACE_REGION` naming the source segment. Lateral edges at
/// profile corners sharper than the policy threshold are creased, as are
/// cap rims; arc interiors stay smooth.
///
/// # Errors
///
/// Returns a typed [`TessellateError`]; never panics.
pub fn tessellate_extrude(
    profile: &Profile2,
    placement: &Placement3,
    height: f64,
    caps: CapMode,
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    let d = discretize_profile(profile, &policy.discretize)?;

    // Ring layout: outer points first, then each hole's points, matching
    // the triangulator's index convention.
    let ring_starts = ring_starts(&d);
    let total: usize = d.points_len();
    let mut builder = MeshBuilder::new();

    // Bottom ring vertices (z = 0), then top ring vertices (z = height).
    for ring in d.rings() {
        for p in &ring.points {
            builder.push_vertex(narrow(apply_placement(placement, [p[0], p[1], 0.0])));
        }
    }
    for ring in d.rings() {
        for p in &ring.points {
            builder.push_vertex(narrow(apply_placement(placement, [p[0], p[1], height])));
        }
    }
    let top_offset = len_u32(total);

    let mut face_origins: Vec<Feature> = Vec::new();

    // Global segment index offsets per loop for region numbering.
    let seg_offsets = seg_offsets(profile);

    // Corner sharpness per loop, from exact source tangents.
    let corner_sharp: Vec<Vec<bool>> = core::iter::once(profile.outer())
        .chain(profile.holes().iter())
        .map(|source| loop_corner_sharpness(source, policy))
        .collect();

    // Side walls: one quad per discretized edge, every ring.
    let bottom_cap = matches!(caps, CapMode::Both | CapMode::Start);
    let top_cap = matches!(caps, CapMode::Both | CapMode::End);
    for (ring_index, ring) in d.rings().enumerate() {
        let base = ring_starts[ring_index];
        let n = len_u32(ring.points.len());
        let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
        for i in 0..n {
            let j = (i + 1) % n;
            let b_i = base + i;
            let b_j = base + j;
            let t_i = top_offset + base + i;
            let t_j = top_offset + base + j;
            let seg = ring.edge_seg[i as usize];

            // Lateral sharpness at ring points i and j: only original
            // endpoints (segment boundaries) are candidates — a point where
            // edge ownership changes is the start endpoint of its segment —
            // and the verdict comes from exact source tangents, so arc
            // interiors and tangent junctions stay smooth at any
            // discretization density.
            let sharp_at = |point: u32| {
                ring.is_endpoint(point)
                    && corner_sharp[ring_index][ring.edge_seg[point as usize] as usize]
            };
            let sharp_i = sharp_at(i);
            let sharp_j = sharp_at(j);

            // Quad loop [b_i, b_j, t_j, t_i]: edges are bottom rim,
            // lateral j, top rim, lateral i.
            let sharp = [
                if bottom_cap { 1.0 } else { 0.0 },
                if sharp_j { 1.0 } else { 0.0 },
                if top_cap { 1.0 } else { 0.0 },
                if sharp_i { 1.0 } else { 0.0 },
            ];
            builder.add_face_with_attrs(
                &[b_i, b_j, t_j, t_i],
                &FaceBuildAttrs {
                    region: Some(REGION_WALL_BASE + seg_offsets[ring_index] + seg),
                    edge_seams: None,
                    edge_sharpness: Some(&sharp),
                },
            )?;
            face_origins.push(Feature::Wall { loop_index, seg });
        }
    }

    // Caps.
    let convex_simple = d.holes.is_empty() && is_convex_ring(&d.outer);
    if bottom_cap || top_cap {
        if convex_simple {
            let n = len_u32(d.outer.points.len());
            if bottom_cap {
                // Bottom cap faces -Z: reverse the CCW ring.
                let ring: Vec<u32> = (0..n).rev().collect();
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(REGION_CAP_START),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(Feature::CapStart);
            }
            if top_cap {
                let ring: Vec<u32> = (0..n).map(|i| top_offset + i).collect();
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(REGION_CAP_END),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(Feature::CapEnd);
            }
        } else {
            let holes: Vec<&[[f64; 2]]> = d.holes.iter().map(|h| h.points.as_slice()).collect();
            let input = PolygonInput {
                outer: &d.outer.points,
                holes: &holes,
            };
            let tri = triangulate(&input, &TriParams::default())?;
            for t in &tri.triangles {
                if bottom_cap {
                    builder.add_face_with_attrs(
                        &[t[2], t[1], t[0]],
                        &FaceBuildAttrs {
                            region: Some(REGION_CAP_START),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(Feature::CapStart);
                }
            }
            for t in &tri.triangles {
                if top_cap {
                    builder.add_face_with_attrs(
                        &[top_offset + t[0], top_offset + t[1], top_offset + t[2]],
                        &FaceBuildAttrs {
                            region: Some(REGION_CAP_END),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(Feature::CapEnd);
                }
            }
        }
    }

    let result = builder.build()?;
    let vertex_features = profile_vertex_features(&d, 2);
    let source_map = crate::source_map::SourceMap::new(&result.mesh, face_origins, vertex_features);
    Ok(TessellatedBody {
        mesh: result.mesh,
        source_map,
    })
}

/// Vertex features: each ring point's wall feature, repeated for each of
/// the body's vertex rings (2 for extrusions, one per angular step for
/// revolutions).
fn profile_vertex_features(d: &DiscretizedProfile, vertex_rings: u32) -> Vec<Feature> {
    let mut per_ring: Vec<Feature> = Vec::with_capacity(d.points_len());
    for (ring_index, ring) in d.rings().enumerate() {
        let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
        for &seg in &ring.edge_seg {
            per_ring.push(Feature::Wall { loop_index, seg });
        }
    }
    let mut out = Vec::with_capacity(per_ring.len() * vertex_rings as usize);
    for _ in 0..vertex_rings {
        out.extend_from_slice(&per_ring);
    }
    out
}

impl DiscretizedProfile {
    fn rings(&self) -> impl Iterator<Item = &crate::discretize::DiscretizedLoop> {
        core::iter::once(&self.outer).chain(self.holes.iter())
    }

    fn points_len(&self) -> usize {
        self.outer.points.len() + self.holes.iter().map(|h| h.points.len()).sum::<usize>()
    }
}

impl crate::discretize::DiscretizedLoop {
    /// True when ring point `i` is an exact source endpoint (edge ownership
    /// changes there).
    fn is_endpoint(&self, i: u32) -> bool {
        let n = self.edge_seg.len();
        let prev = self.edge_seg[(i as usize + n - 1) % n];
        self.edge_seg[i as usize] != prev
    }
}

fn ring_starts(d: &DiscretizedProfile) -> Vec<u32> {
    let mut starts = Vec::with_capacity(1 + d.holes.len());
    let mut acc = 0_u32;
    starts.push(acc);
    acc += len_u32(d.outer.points.len());
    for hole in &d.holes {
        starts.push(acc);
        acc += len_u32(hole.points.len());
    }
    starts
}

fn seg_offsets(profile: &Profile2) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(1 + profile.holes().len());
    let mut acc = 0_u32;
    offsets.push(acc);
    acc += len_u32(profile.outer().segs().len());
    for hole in profile.holes() {
        offsets.push(acc);
        acc += len_u32(hole.segs().len());
    }
    offsets
}

/// Start and end tangent directions of a segment (unnormalized).
///
/// Arc tangents come from the tangent-chord angle: the tangent deviates
/// from the chord by half the sweep, and `cos(sweep/2)`, `sin(sweep/2)`
/// derive from the bulge by half-angle identities — pure arithmetic, no
/// trig, bit-deterministic.
fn seg_tangents(start: kurbo::Point, seg: &crate::profile::Seg2) -> ([f64; 2], [f64; 2]) {
    use crate::profile::SegKind;
    let chord = [seg.to.x - start.x, seg.to.y - start.y];
    match seg.kind {
        SegKind::Line => (chord, chord),
        SegKind::Arc { bulge } => {
            // cos(sweep/2) = (1 - b^2) / (1 + b^2); sin = 2b / (1 + b^2).
            let denom = 1.0 + bulge * bulge;
            let c = (1.0 - bulge * bulge) / denom;
            let s = 2.0 * bulge / denom;
            // Start tangent: chord rotated by -sweep/2; end: by +sweep/2.
            let start_t = [chord[0] * c + chord[1] * s, -chord[0] * s + chord[1] * c];
            let end_t = [chord[0] * c - chord[1] * s, chord[0] * s + chord[1] * c];
            (start_t, end_t)
        }
        SegKind::Cubic { c1, c2 } => {
            let s = [c1.x - start.x, c1.y - start.y];
            let e = [seg.to.x - c2.x, seg.to.y - c2.y];
            let s = if s == [0.0, 0.0] { chord } else { s };
            let e = if e == [0.0, 0.0] { chord } else { e };
            (s, e)
        }
    }
}

/// Per-segment corner sharpness for one source loop: entry `i` is true when
/// the original endpoint *starting* segment `i` (the junction between
/// segment `i - 1` and segment `i`) turns more than the policy threshold,
/// measured on exact source-curve tangents.
fn loop_corner_sharpness(source: &crate::profile::Loop2, policy: &EvalPolicy) -> Vec<bool> {
    let tangents: Vec<([f64; 2], [f64; 2])> = source
        .iter_with_starts()
        .map(|(start, seg)| seg_tangents(start, seg))
        .collect();
    let n = tangents.len();
    (0..n)
        .map(|i| {
            let incoming = tangents[(i + n - 1) % n].1;
            let outgoing = tangents[i].0;
            let cross = incoming[0] * outgoing[1] - incoming[1] * outgoing[0];
            let la = libm::sqrt(incoming[0] * incoming[0] + incoming[1] * incoming[1]);
            let lb = libm::sqrt(outgoing[0] * outgoing[0] + outgoing[1] * outgoing[1]);
            libm::fabs(cross) > policy.sharp_sin_threshold * la * lb
        })
        .collect()
}

/// True when every turn of the (CCW) ring is non-reflex.
fn is_convex_ring(ring: &crate::discretize::DiscretizedLoop) -> bool {
    let n = ring.points.len();
    for i in 0..n {
        let a = ring.points[i];
        let b = ring.points[(i + 1) % n];
        let c = ring.points[(i + 2) % n];
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross < 0.0 {
            return false;
        }
    }
    true
}

/// Tessellates a revolution: the profile's local `(x, y)` plane revolved
/// about the local Y axis, `x` as radius and `y` as height, swept through
/// `sweep` radians (counter-clockwise viewed from +Y), placed by
/// `placement`.
///
/// A full sweep (`sweep == tau`, compared exactly) closes on itself with a
/// seam meridian tagged via edge seams; partial sweeps close their
/// boundary planes according to `caps` (start cap at angle zero). The
/// profile must lie at strictly positive radius — bodies touching the axis
/// are rejected rather than emitted with degenerate walls.
///
/// # Errors
///
/// Returns a typed [`TessellateError`]; never panics.
pub fn tessellate_revolve(
    profile: &Profile2,
    placement: &Placement3,
    sweep: f64,
    caps: CapMode,
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    let d = discretize_profile(profile, &policy.discretize)?;
    let full = sweep == core::f64::consts::TAU;

    // Strictly positive radius: no axis contact (documented v1 scope).
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    for ring in d.rings() {
        for p in &ring.points {
            min_x = min_x.min(p[0]);
            max_x = max_x.max(p[0]);
        }
    }
    if min_x <= 0.0 || min_x.is_nan() {
        return Err(TessellateError::AxisContact { min_radius: min_x });
    }

    // Angular step count from the chord tolerance at the outermost radius.
    let steps = {
        let tol = policy.discretize.chord_tolerance;
        let per_edge = if tol >= 2.0 * max_x {
            sweep
        } else {
            2.0 * libm::acos(1.0 - tol / max_x)
        };
        let needed = libm::ceil(sweep / per_edge);
        let capped = if needed >= f64::from(policy.discretize.max_segment_edges) {
            policy.discretize.max_segment_edges
        } else {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "needed is a ceil of a finite positive value below the u32 cap"
            )]
            {
                needed as u32
            }
        };
        capped.clamp(
            policy.discretize.min_arc_edges.max(3),
            policy.discretize.max_segment_edges,
        )
    };
    // Vertex rings: `steps` for a full sweep (wrapping), `steps + 1`
    // otherwise.
    let vertex_rings = if full { steps } else { steps + 1 };

    let ring_starts = ring_starts(&d);
    let total = len_u32(d.points_len());
    let mut builder = MeshBuilder::new();

    // Vertices: angular-step major, profile point minor. Angles evaluated
    // independently per step (no accumulation drift), libm trig only.
    let step_angle = sweep / f64::from(steps);
    for k in 0..vertex_rings {
        let angle = step_angle * f64::from(k);
        let (s, c) = (libm::sin(angle), libm::cos(angle));
        for ring in d.rings() {
            for p in &ring.points {
                let v = [p[0] * c, p[1], p[0] * s];
                builder.push_vertex(narrow(apply_placement(placement, v)));
            }
        }
    }
    let vertex_at = |k: u32, flat: u32| (k % vertex_rings) * total + flat;

    let mut face_origins: Vec<Feature> = Vec::new();
    let seg_offsets = seg_offsets(profile);
    let corner_sharp: Vec<Vec<bool>> = core::iter::once(profile.outer())
        .chain(profile.holes().iter())
        .map(|source| loop_corner_sharpness(source, policy))
        .collect();

    let start_cap = !full && matches!(caps, CapMode::Both | CapMode::Start);
    let end_cap = !full && matches!(caps, CapMode::Both | CapMode::End);

    // Walls: one quad per profile edge per angular step.
    for (ring_index, ring) in d.rings().enumerate() {
        let base = ring_starts[ring_index];
        let n = len_u32(ring.points.len());
        let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
        for i in 0..n {
            let j = (i + 1) % n;
            let seg = ring.edge_seg[i as usize];
            let sharp_at = |point: u32| {
                ring.is_endpoint(point)
                    && corner_sharp[ring_index][ring.edge_seg[point as usize] as usize]
            };
            let ring_sharp_i = sharp_at(i);
            let ring_sharp_j = sharp_at(j);
            for k in 0..steps {
                let a = vertex_at(k, base + i);
                let d_v = vertex_at(k, base + j);
                let c_v = vertex_at(k + 1, base + j);
                let b = vertex_at(k + 1, base + i);
                // Quad loop [a, d, c, b]: edges are the theta_k meridian
                // (a->d), the ring at profile point j (d->c), the
                // theta_{k+1} meridian (c->b), and the ring at point i
                // (b->a).
                let meridian_start = k == 0 && (start_cap || full);
                let meridian_end = (k + 1 == steps) && end_cap;
                let sharp = [
                    if k == 0 && start_cap { 1.0 } else { 0.0 },
                    if ring_sharp_j { 1.0 } else { 0.0 },
                    if meridian_end { 1.0 } else { 0.0 },
                    if ring_sharp_i { 1.0 } else { 0.0 },
                ];
                // Seams only on the seam-bearing face: writing `false`
                // entries from the wrapping neighbor would clobber the
                // shared canonical edge's `true`.
                let seams = [true, false, false, false];
                let edge_seams = (meridian_start && full).then_some(&seams[..]);
                builder.add_face_with_attrs(
                    &[a, d_v, c_v, b],
                    &FaceBuildAttrs {
                        region: Some(REGION_WALL_BASE + seg_offsets[ring_index] + seg),
                        edge_seams,
                        edge_sharpness: Some(&sharp),
                    },
                )?;
                face_origins.push(Feature::Wall { loop_index, seg });
            }
        }
    }

    // Caps for partial sweeps: the profile placed at the boundary planes.
    if start_cap || end_cap {
        let convex_simple = d.holes.is_empty() && is_convex_ring(&d.outer);
        let end_ring = vertex_rings - 1;
        if convex_simple {
            let n = len_u32(d.outer.points.len());
            if start_cap {
                // Start cap faces the negative tangential direction:
                // reverse the CCW profile ring at angle zero.
                let ring: Vec<u32> = (0..n).rev().map(|i| vertex_at(0, i)).collect();
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(REGION_CAP_START),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(Feature::CapStart);
            }
            if end_cap {
                let ring: Vec<u32> = (0..n).map(|i| vertex_at(end_ring, i)).collect();
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(REGION_CAP_END),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(Feature::CapEnd);
            }
        } else {
            let holes: Vec<&[[f64; 2]]> = d.holes.iter().map(|h| h.points.as_slice()).collect();
            let input = PolygonInput {
                outer: &d.outer.points,
                holes: &holes,
            };
            let tri = triangulate(&input, &TriParams::default())?;
            for t in &tri.triangles {
                if start_cap {
                    builder.add_face_with_attrs(
                        &[vertex_at(0, t[2]), vertex_at(0, t[1]), vertex_at(0, t[0])],
                        &FaceBuildAttrs {
                            region: Some(REGION_CAP_START),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(Feature::CapStart);
                }
            }
            for t in &tri.triangles {
                if end_cap {
                    builder.add_face_with_attrs(
                        &[
                            vertex_at(end_ring, t[0]),
                            vertex_at(end_ring, t[1]),
                            vertex_at(end_ring, t[2]),
                        ],
                        &FaceBuildAttrs {
                            region: Some(REGION_CAP_END),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(Feature::CapEnd);
                }
            }
        }
    }

    let result = builder.build()?;
    let vertex_features = profile_vertex_features(&d, vertex_rings);
    let source_map = crate::source_map::SourceMap::new(&result.mesh, face_origins, vertex_features);
    Ok(TessellatedBody {
        mesh: result.mesh,
        source_map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;

    /// Signed volume via the divergence theorem, fanning each face loop.
    /// Valid for planar convex faces (all faces this tessellator emits).
    fn mesh_volume(mesh: &exedra::Mesh) -> f64 {
        let mut vol = 0.0;
        for face in mesh.faces() {
            let verts: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                .collect();
            for i in 1..verts.len().saturating_sub(1) {
                let (a, b, c) = (verts[0], verts[i], verts[i + 1]);
                vol += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]);
            }
        }
        vol / 6.0
    }

    fn assert_clean(body: &TessellatedBody) {
        let errors = body.mesh.validate_deep();
        assert!(errors.is_empty(), "validate_deep: {errors:?}");
        assert_eq!(
            body.source_map.face_count(),
            body.mesh.faces().count(),
            "one origin per face"
        );
    }

    #[test]
    fn rect_extrude_is_a_box() {
        let profile = builders::rect(2.0, 1.0).expect("rect");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            3.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        assert_eq!(body.mesh.faces().count(), 6, "4 walls + 2 ngon caps");
        assert!((mesh_volume(&body.mesh) - 6.0).abs() < 1e-4);
    }

    #[test]
    fn l_profile_extrude_has_triangulated_caps() {
        let profile = builders::l_profile(1.0, 1.0, 0.5, 0.5).expect("L");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            2.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        assert!((mesh_volume(&body.mesh) - 1.5).abs() < 1e-4);
        let caps = body
            .source_map
            .face_features()
            .iter()
            .filter(|f| matches!(f, Feature::CapStart | Feature::CapEnd))
            .count();
        assert_eq!(caps, 8, "4 triangles per concave cap");
    }

    #[test]
    fn holed_profile_extrude() {
        let profile = builders::ring(2.0, 1.0).expect("ring");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        let expected = core::f64::consts::PI * 3.0;
        let vol = mesh_volume(&body.mesh);
        // Discretized circles under-approximate the true area slightly.
        assert!(
            (vol - expected).abs() < 0.05,
            "ring volume {vol} vs {expected}"
        );
        // Both wall families exist: outer loop and hole loop.
        assert!(
            body.source_map
                .face_features()
                .iter()
                .any(|f| matches!(f, Feature::Wall { loop_index: 0, .. }))
        );
        assert!(
            body.source_map
                .face_features()
                .iter()
                .any(|f| matches!(f, Feature::Wall { loop_index: 1, .. }))
        );
    }

    #[test]
    fn rounded_profile_walls_are_smooth_at_tangent_junctions() {
        let profile = builders::rounded_rect(4.0, 2.0, 0.5).expect("rounded rect");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        // Tangent-continuous junctions: no lateral edge may be sharp. Cap
        // rims are sharp. Count sharp edges: expect exactly the two rims.
        let mesh = &body.mesh;
        let mut sharp_lateral = 0;
        let mut sharp_rim = 0;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(edge) = mesh.canonical_edge(he)
                    && mesh.edge_sharpness(edge).unwrap_or(0.0) > 0.5
                {
                    // Classify by geometry: rim edges are horizontal
                    // (endpoints share z), laterals vertical.
                    let a = mesh.to_vertex(he).and_then(|v| mesh.vertex_position(v));
                    let b = mesh
                        .to_vertex(mesh.twin(he).expect("twin"))
                        .and_then(|v| mesh.vertex_position(v));
                    if let (Some(a), Some(b)) = (a, b) {
                        if (a[2] - b[2]).abs() < 1e-6 {
                            sharp_rim += 1;
                        } else {
                            sharp_lateral += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(sharp_lateral, 0, "tangent junctions must stay smooth");
        assert!(sharp_rim > 0, "cap rims must crease");
    }

    #[test]
    fn square_corners_crease_laterals() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        let mesh = &body.mesh;
        let mut sharp_lateral = 0;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(edge) = mesh.canonical_edge(he)
                    && mesh.edge_sharpness(edge).unwrap_or(0.0) > 0.5
                {
                    let a = mesh.to_vertex(he).and_then(|v| mesh.vertex_position(v));
                    let b = mesh
                        .to_vertex(mesh.twin(he).expect("twin"))
                        .and_then(|v| mesh.vertex_position(v));
                    if let (Some(a), Some(b)) = (a, b)
                        && (a[2] - b[2]).abs() > 1e-6
                    {
                        sharp_lateral += 1;
                    }
                }
            }
        }
        // 4 lateral edges, each visited from two adjacent faces.
        assert_eq!(sharp_lateral, 8, "square corners crease all laterals");
    }

    #[test]
    fn open_shell_has_boundaries() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::None,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        assert_eq!(body.mesh.faces().count(), 4, "walls only");
    }

    #[test]
    fn placement_moves_the_body() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let placed = Placement3::translate(10.0, 0.0, 5.0);
        let body = tessellate_extrude(
            &profile,
            &placed,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        let any = body
            .mesh
            .faces()
            .next()
            .and_then(|f| body.mesh.face_loop(f).next())
            .and_then(|he| body.mesh.to_vertex(he))
            .and_then(|v| body.mesh.vertex_position(v))
            .copied()
            .expect("has a vertex");
        assert!(any[0] >= 10.0 && any[2] >= 5.0);
    }

    #[test]
    fn tessellation_is_deterministic() {
        let profile = builders::rounded_rect(4.0, 2.0, 0.5).expect("rounded rect");
        let policy = EvalPolicy::default();
        let sig = |body: &TessellatedBody| {
            let (tri, _) = body.mesh.to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&tri)
        };
        let a = tessellate_extrude(&profile, &Placement3::IDENTITY, 1.0, CapMode::Both, &policy)
            .expect("first");
        let b = tessellate_extrude(&profile, &Placement3::IDENTITY, 1.0, CapMode::Both, &policy)
            .expect("second");
        assert_eq!(sig(&a), sig(&b), "double-run signature equality");
    }

    /// An off-axis square cross-section: `a x a` at radial center `r0`.
    fn annulus_square(r0: f64, a: f64) -> Profile2 {
        use crate::profile::{Loop2, Seg2};
        let (x0, x1) = (r0 - a / 2.0, r0 + a / 2.0);
        let outer = Loop2::new(alloc::vec![
            Seg2::line((x1, 0.0)),
            Seg2::line((x1, a)),
            Seg2::line((x0, a)),
            Seg2::line((x0, 0.0)),
        ])
        .expect("valid square section");
        Profile2::simple(outer).expect("valid profile")
    }

    #[test]
    fn full_revolve_square_torus_volume_matches_pappus() {
        // Fine tolerance so the discretized ring area is close to ideal.
        let policy = EvalPolicy {
            discretize: DiscretizePolicy {
                chord_tolerance: 1e-3,
                ..Default::default()
            },
            ..Default::default()
        };
        let profile = annulus_square(3.0, 1.0);
        let body = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::TAU,
            CapMode::Both,
            &policy,
        )
        .expect("tessellates");
        assert_clean(&body);
        // Pappus: V = 2 pi R A. The polygonal ring slightly under-sweeps;
        // fine steps keep it within a fraction of a percent.
        let expected = core::f64::consts::TAU * 3.0 * 1.0;
        let vol = mesh_volume(&body.mesh);
        assert!(
            (vol - expected).abs() / expected < 0.005,
            "torus volume {vol} vs {expected}"
        );
        // Full sweeps have no caps.
        assert!(
            body.source_map
                .face_features()
                .iter()
                .all(|f| matches!(f, Feature::Wall { .. })),
            "full sweep emits walls only"
        );
    }

    #[test]
    fn full_revolve_tags_a_seam() {
        let profile = annulus_square(2.0, 0.5);
        let body = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::TAU,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        let mesh = &body.mesh;
        let mut seam_edges = 0;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(edge) = mesh.canonical_edge(he)
                    && mesh.edge_seam(edge) == Some(true)
                {
                    seam_edges += 1;
                }
            }
        }
        assert!(seam_edges > 0, "full sweep tags its closure meridian");
    }

    #[test]
    fn quarter_revolve_with_caps() {
        let policy = EvalPolicy {
            discretize: DiscretizePolicy {
                chord_tolerance: 1e-3,
                ..Default::default()
            },
            ..Default::default()
        };
        let profile = annulus_square(3.0, 1.0);
        let body = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::FRAC_PI_2,
            CapMode::Both,
            &policy,
        )
        .expect("tessellates");
        assert_clean(&body);
        let expected = core::f64::consts::TAU * 3.0 / 4.0;
        let vol = mesh_volume(&body.mesh);
        assert!(
            (vol - expected).abs() / expected < 0.005,
            "quarter torus volume {vol} vs {expected}"
        );
        let caps = body
            .source_map
            .face_features()
            .iter()
            .filter(|f| matches!(f, Feature::CapStart | Feature::CapEnd))
            .count();
        assert_eq!(caps, 2, "one convex ngon cap per boundary plane");
    }

    #[test]
    fn axis_contact_is_rejected() {
        let profile = builders::rect(1.0, 1.0).expect("rect touches x = 0");
        let result = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::TAU,
            CapMode::Both,
            &EvalPolicy::default(),
        );
        assert!(
            matches!(result, Err(TessellateError::AxisContact { .. })),
            "axis-touching profiles are rejected, got {result:?}"
        );
    }

    #[test]
    fn revolve_is_deterministic() {
        let profile = annulus_square(2.0, 0.5);
        let policy = EvalPolicy::default();
        let sig = |body: &TessellatedBody| {
            let (tri, _) = body.mesh.to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&tri)
        };
        let a = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::FRAC_PI_2,
            CapMode::Both,
            &policy,
        )
        .expect("first");
        let b = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::FRAC_PI_2,
            CapMode::Both,
            &policy,
        )
        .expect("second");
        assert_eq!(sig(&a), sig(&b), "double-run signature equality");
    }
}
