// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Read-only differential measurements of the stored, piecewise affine UV map.
//!
//! Density is measured in texels per input position unit. No model-unit scale
//! is inferred. Anisotropy measures directional stretch, not distortion against
//! an unknown authored rest surface. Mirrored charts are evidence, not errors,
//! unless the caller disallows them. This does not detect global UV overlap or
//! certify the chosen polygon triangulation. Use the same triangulation as the
//! consumer whose output is being inspected.

use alloc::vec::Vec;
use exedra_math::{cross, dot, sub};
use exedra_mesh::{CornerId, FaceId, FaceTriangulation, Mesh, MeshRevision, attr};

use super::{UvError, UvScope, UvSet, validate_set};
use crate::selection::canonicalize_face_set;

/// Optional acceptance bounds, in texels per position unit and dimensionless ratios.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UvQualityLimits {
    /// Largest permitted principal stretch ratio, at least one.
    pub max_anisotropy: Option<f64>,
    /// Smallest permitted area-equivalent linear density, strictly positive.
    pub min_density: Option<f64>,
    /// Largest permitted area-equivalent linear density, strictly positive.
    pub max_density: Option<f64>,
    /// Treat a negative UV orientation as a violation. Reflections are otherwise permitted.
    pub forbid_mirrored: bool,
}

/// Inspection scope, image resolution, triangulation and bounded evidence storage.
#[derive(Clone, Debug, PartialEq)]
pub struct UvQualityParams {
    /// Face selection. Whole-mesh scope streams without allocating a face list.
    pub scope: UvScope,
    /// UV channel to measure.
    pub set: UvSet,
    /// Image width and height in texels; `[1, 1]` measures repeats per position unit.
    pub texture_size: [u32; 2],
    /// Must match render extraction when comparing triangle-local results.
    pub triangulation: FaceTriangulation,
    /// Optional quality thresholds. Absence does not impose an implicit quality target.
    pub limits: UvQualityLimits,
    /// Maximum stored issues; all issues are counted even when evidence is truncated.
    pub witness_limit: usize,
}
impl Default for UvQualityParams {
    fn default() -> Self {
        Self {
            scope: UvScope::WholeMesh,
            set: UvSet::Primary,
            texture_size: [1; 2],
            triangulation: FaceTriangulation::Fan,
            limits: UvQualityLimits::default(),
            witness_limit: 16,
        }
    }
}

/// Why one triangle could not be measured, or did not meet caller-supplied limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UvQualityIssue {
    /// At least one corner has no UV value.
    MissingUv,
    /// At least one authored UV component is nonfinite.
    NonfiniteUv,
    /// At least one position component is nonfinite.
    NonfiniteGeometry,
    /// The triangle has zero geometric area in the f64 metric calculation.
    DegenerateGeometry,
    /// The triangle has zero UV area in the f64 metric calculation.
    DegenerateUv,
    /// Differential arithmetic cannot produce finite positive measurements.
    NumericLimit,
    /// The metrics violate one or more explicit acceptance bounds.
    LimitExceeded,
    /// Robust triangulation refused the polygon; its fallback fan was not measured.
    TriangulationFallback,
}

/// Independent acceptance failures on an otherwise measurable triangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UvQualityViolations {
    /// Principal stretch ratio exceeds the requested maximum.
    pub anisotropy: bool,
    /// Density is below the requested minimum.
    pub density_below: bool,
    /// Density is above the requested maximum.
    pub density_above: bool,
    /// UV winding is negative and the caller forbids mirrored charts.
    pub mirrored: bool,
}
impl UvQualityViolations {
    /// Whether any caller-supplied bound was violated.
    #[must_use]
    pub const fn any(self) -> bool {
        self.anisotropy || self.density_below || self.density_above || self.mirrored
    }
}

/// Differential measurements of a single stored UV triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTriangleMetrics {
    /// Triangle area in squared input position units.
    pub surface_area: f64,
    /// Absolute UV area after applying image resolution, in squared texels.
    pub texture_area: f64,
    /// Smallest and largest singular values of the position-to-texel Jacobian.
    pub principal_density: [f64; 2],
    /// Largest principal density divided by the smallest; one is isotropic.
    pub anisotropy: f64,
    /// Square root of texture area / surface area, in texels per position unit.
    pub density: f64,
    /// UV orientation is negative relative to the triangle corner order.
    pub mirrored: bool,
}

/// A measured triangle, retaining exact mesh corner identities for drill-down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTriangleQuality {
    /// Face owning the triangle.
    pub face: FaceId,
    /// Triangle corners, in the requested enumeration order.
    pub corners: [CornerId; 3],
    /// Metrics, or the first refusal. Threshold violations do not remove metrics.
    pub metrics: Result<UvTriangleMetrics, UvQualityIssue>,
    /// Threshold violations; empty for unmeasurable triangles.
    pub violations: UvQualityViolations,
}

/// One bounded issue witness. Face-level triangulation refusals have no triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvQualityWitness {
    /// Offending face.
    pub face: FaceId,
    /// Classification for recovery or filtering.
    pub issue: UvQualityIssue,
    /// Triangle-local evidence when enumeration succeeded.
    pub triangle: Option<UvTriangleQuality>,
}

/// An extreme metric and the triangle that produced it. Equal values keep the first witness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvQualityExtremum {
    /// Metric value.
    pub value: f64,
    /// Source face.
    pub face: FaceId,
    /// Source triangle corners.
    pub corners: [CornerId; 3],
}

/// Triangle refusal counts. Each refused triangle contributes to its first
/// failing classification only; these counts sum to `triangles_refused`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UvQualityRefusals {
    /// Triangles with missing corner UVs.
    pub missing_uv: u64,
    /// Triangles with nonfinite authored UVs.
    pub nonfinite_uv: u64,
    /// Triangles with nonfinite positions.
    pub nonfinite_geometry: u64,
    /// Triangles with zero geometric area in the metric calculation.
    pub degenerate_geometry: u64,
    /// Triangles with zero UV area in the metric calculation.
    pub degenerate_uv: u64,
    /// Triangles whose differential arithmetic failed.
    pub numeric_limit: u64,
}
impl UvQualityRefusals {
    fn record(&mut self, issue: UvQualityIssue) {
        match issue {
            UvQualityIssue::MissingUv => self.missing_uv += 1,
            UvQualityIssue::NonfiniteUv => self.nonfinite_uv += 1,
            UvQualityIssue::NonfiniteGeometry => self.nonfinite_geometry += 1,
            UvQualityIssue::DegenerateGeometry => self.degenerate_geometry += 1,
            UvQualityIssue::DegenerateUv => self.degenerate_uv += 1,
            UvQualityIssue::NumericLimit => self.numeric_limit += 1,
            UvQualityIssue::LimitExceeded | UvQualityIssue::TriangulationFallback => {
                unreachable!("not a triangle metric refusal")
            }
        }
    }
}

/// Bounded overview. Triangle failures do not discard the rest of the survey.
///
/// Corner IDs belong to the supplied mesh at `revision`. A revision is not a
/// cross-mesh identity or a cache key; retain the originating mesh identity in
/// the caller. No spatial overlap or smooth-surface error bound is inferred.
#[derive(Clone, Debug, PartialEq)]
pub struct UvQualityReport {
    /// Source revision observed during inspection.
    pub revision: MeshRevision,
    /// Channel measured by this report.
    pub set: UvSet,
    /// Image dimensions used by the density calculation, in texels.
    pub texture_size: [u32; 2],
    /// Triangle enumeration used by the survey.
    pub triangulation: FaceTriangulation,
    /// Acceptance bounds used to classify measured triangles.
    pub limits: UvQualityLimits,
    /// Selected faces visited, including triangulation refusals.
    pub faces_examined: u64,
    /// Enumerated triangles visited.
    pub triangles_examined: u64,
    /// Triangles with finite, nondegenerate metrics.
    pub triangles_measured: u64,
    /// Triangles refused because their input or metric was invalid.
    pub triangles_refused: u64,
    /// Refused triangles grouped by the first failed invariant.
    pub refusals: UvQualityRefusals,
    /// Measured triangles exceeding at least one explicit limit.
    pub triangles_outside_limits: u64,
    /// Sum of finite measurable triangle areas, in squared position units.
    /// Refused triangles are excluded. This is an f64 sum, not an exact area certificate.
    pub measured_surface_area: f64,
    /// Measurable area on triangles exceeding one or more acceptance bounds.
    /// Small slivers can have extreme derivatives while covering negligible area.
    pub outside_limits_surface_area: f64,
    /// Measured triangles with negative UV winding, including allowed reflections.
    pub mirrored_triangles: u64,
    /// Faces whose robust triangulation fell back and was excluded.
    pub triangulation_fallback_faces: u64,
    /// Maximum measured anisotropy.
    pub max_anisotropy: Option<UvQualityExtremum>,
    /// Minimum measured area-equivalent density.
    pub min_density: Option<UvQualityExtremum>,
    /// Maximum measured area-equivalent density.
    pub max_density: Option<UvQualityExtremum>,
    /// First issues in deterministic face/triangle order, limited by `witness_limit`.
    pub witnesses: Vec<UvQualityWitness>,
    /// Additional issues not retained in `witnesses`.
    pub witnesses_omitted: u64,
    /// Whether an explicit face selection needed sorting or deduplication.
    pub selections_canonicalized: bool,
    /// Reusable operation capacity bytes, excluding output and kernel internals.
    pub scratch_bytes: usize,
}

/// Reusable selection and triangle buffers. Robust polygon triangulation may
/// allocate inside the kernel. Triangular faces bypass that path entirely.
#[derive(Debug, Default)]
pub struct UvQualityScratch {
    faces: Vec<FaceId>,
    triangles: Vec<[CornerId; 3]>,
}
impl UvQualityScratch {
    /// Retained capacity bytes, excluding allocator overhead.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.faces.capacity() * size_of::<FaceId>()
            + self.triangles.capacity() * size_of::<[CornerId; 3]>()
    }
    /// Releases retained operation storage.
    pub fn release(&mut self) {
        *self = Self::default();
    }
}

/// A parameter or structural mesh error. Per-triangle quality refusals are reported as evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UvQualityError {
    /// Invalid quality parameter; the field is a stable Rust field name.
    InvalidParameter {
        /// Name of the rejected field, including its containing field when nested.
        field: &'static str,
    },
    /// A channel or mesh selection was structurally invalid.
    Mesh(UvError),
}
impl core::fmt::Display for UvQualityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidParameter { field } => write!(f, "invalid UV quality parameter: {field}"),
            Self::Mesh(error) => write!(f, "UV quality: {error}"),
        }
    }
}
impl core::error::Error for UvQualityError {}

fn validate(params: &UvQualityParams) -> Result<(), UvQualityError> {
    let bad = if params.texture_size.contains(&0) {
        Some("texture_size")
    } else if params
        .limits
        .max_anisotropy
        .is_some_and(|v| !v.is_finite() || v < 1.0)
    {
        Some("limits.max_anisotropy")
    } else if params
        .limits
        .min_density
        .is_some_and(|v| !v.is_finite() || v <= 0.0)
    {
        Some("limits.min_density")
    } else if params
        .limits
        .max_density
        .is_some_and(|v| !v.is_finite() || v <= 0.0)
    {
        Some("limits.max_density")
    } else if params
        .limits
        .min_density
        .zip(params.limits.max_density)
        .is_some_and(|(a, b)| a > b)
    {
        Some("limits.min_density")
    } else {
        None
    };
    bad.map_or(Ok(()), |field| {
        Err(UvQualityError::InvalidParameter { field })
    })
}

/// Surveys stored UV quality without mutation. Retain scratch between calls;
/// request a face set for a local update. No timing or allocation count is inferred.
pub fn inspect_uv_quality(
    mesh: &Mesh,
    params: &UvQualityParams,
    scratch: &mut UvQualityScratch,
) -> Result<UvQualityReport, UvQualityError> {
    visit_uv_quality(mesh, params, scratch, |_| {})
}

/// Streams triangle measurements and returns the same bounded overview as
/// `inspect_uv_quality`. Use this for heatmaps or selective detail capture without
/// allocating a whole-mesh triangle report. Callback delivery is incremental;
/// a later structural error can follow earlier callbacks. Fallback polygons
/// produce face-level report witnesses and no untrusted triangle callbacks.
pub fn visit_uv_quality(
    mesh: &Mesh,
    params: &UvQualityParams,
    scratch: &mut UvQualityScratch,
    mut visit: impl FnMut(UvTriangleQuality),
) -> Result<UvQualityReport, UvQualityError> {
    validate(params)?;
    validate_set(mesh, params.set).map_err(UvQualityError::Mesh)?;
    // Resolve typed storage once. Per-triangle attribute-name lookup dominated
    // the initial survey benchmark, especially on already triangulated meshes.
    let source_positions = mesh
        .attrs()
        .dense(attr::VERTEX_POSITION)
        .expect("required position layer");
    let sparse_uvs = mesh.attrs().sparse(params.set.key());
    let dense_uvs = mesh.attrs().dense(params.set.key());
    scratch.faces.clear();
    let canonicalized = if let UvScope::FaceSet(faces) = &params.scope {
        scratch.faces.extend_from_slice(faces);
        canonicalize_face_set(&mut scratch.faces)
    } else {
        false
    };
    let mut report = UvQualityReport {
        revision: mesh.revision(),
        set: params.set,
        texture_size: params.texture_size,
        triangulation: params.triangulation,
        limits: params.limits,
        faces_examined: 0,
        triangles_examined: 0,
        triangles_measured: 0,
        triangles_refused: 0,
        refusals: UvQualityRefusals::default(),
        triangles_outside_limits: 0,
        measured_surface_area: 0.0,
        outside_limits_surface_area: 0.0,
        mirrored_triangles: 0,
        triangulation_fallback_faces: 0,
        max_anisotropy: None,
        min_density: None,
        max_density: None,
        witnesses: Vec::new(),
        witnesses_omitted: 0,
        selections_canonicalized: canonicalized,
        scratch_bytes: 0,
    };
    let whole = matches!(params.scope, UvScope::WholeMesh);
    let faces = mesh
        .faces()
        .take(if whole { usize::MAX } else { 0 })
        .chain(scratch.faces.iter().copied());
    for face in faces {
        if face == FaceId::OUTSIDE || mesh.face_edge(face).is_none() {
            return Err(UvQualityError::Mesh(UvError::InvalidFace { face }));
        }
        report.faces_examined += 1;
        let mut corners = mesh.face_loop(face);
        let triple = match (
            corners.next(),
            corners.next(),
            corners.next(),
            corners.next(),
        ) {
            (Some(a), Some(b), Some(c), None) => Some([a, b, c]),
            _ => None,
        };
        scratch.triangles.clear();
        let fallback = if let Some(triangle) = triple {
            scratch.triangles.push(triangle);
            false
        } else {
            mesh.face_triangles_into(face, params.triangulation, &mut scratch.triangles)
        };
        if fallback {
            report.triangulation_fallback_faces += 1;
            report.issue(
                UvQualityWitness {
                    face,
                    issue: UvQualityIssue::TriangulationFallback,
                    triangle: None,
                },
                params.witness_limit,
            );
            continue;
        }
        for &corners in &scratch.triangles {
            let mut positions = [[0.0; 3]; 3];
            let mut uvs = [[0.0; 2]; 3];
            let mut missing = false;
            for (i, &corner) in corners.iter().enumerate() {
                positions[i] = mesh
                    .to_vertex(corner)
                    .and_then(|vertex| source_positions.get(vertex.into()))
                    .copied()
                    .ok_or(UvQualityError::Mesh(UvError::InvalidCorner { corner }))?;
                match sparse_uvs
                    .and_then(|layer| layer.get(corner.into()))
                    .or_else(|| dense_uvs.and_then(|layer| layer.get(corner.into())))
                    .copied()
                {
                    Some(uv) => uvs[i] = uv,
                    None => missing = true,
                }
            }
            let metrics = if missing {
                Err(UvQualityIssue::MissingUv)
            } else {
                measure(positions, uvs, params.texture_size)
            };
            let violations = metrics
                .map(|metrics| params.limits.check(metrics))
                .unwrap_or_default();
            let triangle = UvTriangleQuality {
                face,
                corners,
                metrics,
                violations,
            };
            report.triangles_examined += 1;
            let issue = match metrics {
                Err(issue) => {
                    report.triangles_refused += 1;
                    report.refusals.record(issue);
                    Some(issue)
                }
                Ok(metrics) => {
                    report.triangles_measured += 1;
                    report.measured_surface_area += metrics.surface_area;
                    report.mirrored_triangles += u64::from(metrics.mirrored);
                    update_extreme(
                        &mut report.max_anisotropy,
                        triangle,
                        metrics.anisotropy,
                        true,
                    );
                    update_extreme(&mut report.min_density, triangle, metrics.density, false);
                    update_extreme(&mut report.max_density, triangle, metrics.density, true);
                    if violations.any() {
                        report.triangles_outside_limits += 1;
                        report.outside_limits_surface_area += metrics.surface_area;
                        Some(UvQualityIssue::LimitExceeded)
                    } else {
                        None
                    }
                }
            };
            if let Some(issue) = issue {
                report.issue(
                    UvQualityWitness {
                        face,
                        issue,
                        triangle: Some(triangle),
                    },
                    params.witness_limit,
                );
            }
            visit(triangle);
        }
    }
    report.scratch_bytes = scratch.retained_bytes();
    Ok(report)
}

impl UvQualityReport {
    /// Every examined face was triangulated and every triangle was measurable
    /// and within the requested bounds. Empty selections satisfy this predicate.
    /// This does not assert absence of global overlap or smooth-surface distortion.
    #[must_use]
    pub const fn is_acceptable(&self) -> bool {
        self.triangles_refused == 0
            && self.triangles_outside_limits == 0
            && self.triangulation_fallback_faces == 0
    }

    /// Fraction of measured area exceeding the requested bounds, or `None`
    /// when no triangles were measurable. Refused triangles remain separate
    /// counts and must not be treated as acceptable area. This lets callers
    /// distinguish a tiny ill-conditioned sliver from a large distorted patch.
    #[must_use]
    pub fn outside_limits_area_fraction(&self) -> Option<f64> {
        (self.measured_surface_area > 0.0).then(|| {
            (self.outside_limits_surface_area / self.measured_surface_area).clamp(0.0, 1.0)
        })
    }

    fn issue(&mut self, witness: UvQualityWitness, limit: usize) {
        if self.witnesses.len() < limit {
            self.witnesses.push(witness);
        } else {
            self.witnesses_omitted += 1;
        }
    }
}
impl UvQualityLimits {
    fn check(self, metrics: UvTriangleMetrics) -> UvQualityViolations {
        UvQualityViolations {
            anisotropy: self.max_anisotropy.is_some_and(|v| metrics.anisotropy > v),
            density_below: self.min_density.is_some_and(|v| metrics.density < v),
            density_above: self.max_density.is_some_and(|v| metrics.density > v),
            mirrored: self.forbid_mirrored && metrics.mirrored,
        }
    }
}
fn update_extreme(
    target: &mut Option<UvQualityExtremum>,
    triangle: UvTriangleQuality,
    value: f64,
    max: bool,
) {
    if target.is_none_or(|old| {
        if max {
            value > old.value
        } else {
            value < old.value
        }
    }) {
        *target = Some(UvQualityExtremum {
            value,
            face: triangle.face,
            corners: triangle.corners,
        });
    }
}

fn measure(
    positions: [[f32; 3]; 3],
    uvs: [[f32; 2]; 3],
    texture_size: [u32; 2],
) -> Result<UvTriangleMetrics, UvQualityIssue> {
    if !positions.iter().flatten().all(|v| v.is_finite()) {
        return Err(UvQualityIssue::NonfiniteGeometry);
    }
    if !uvs.iter().flatten().all(|v| v.is_finite()) {
        return Err(UvQualityIssue::NonfiniteUv);
    }
    let p = positions.map(|p| p.map(f64::from));
    let a = sub(p[1], p[0]);
    let b = sub(p[2], p[0]);
    let length = libm::sqrt(dot(a, a));
    let normal = cross(a, b);
    let area2 = libm::sqrt(dot(normal, normal));
    if length == 0.0 || area2 == 0.0 {
        return Err(UvQualityIssue::DegenerateGeometry);
    }
    let uv = uvs.map(|uv| uv.map(f64::from));
    let d = core::array::from_fn::<_, 2, _>(|i| (uv[1][i] - uv[0][i]) * f64::from(texture_size[i]));
    let e = core::array::from_fn::<_, 2, _>(|i| (uv[2][i] - uv[0][i]) * f64::from(texture_size[i]));
    let det_uv = d[0] * e[1] - d[1] * e[0];
    if det_uv == 0.0 {
        return Err(UvQualityIssue::DegenerateUv);
    }
    // Orthonormal tangent coordinates avoid subtracting near-equal Gram products
    // for thin triangles. The determinant ratio gives the small singular value
    // without cancellation in the eigenvalue subtraction.
    let x = dot(a, b) / length;
    let y = area2 / length;
    let j00 = d[0] / length;
    let j10 = d[1] / length;
    let j01 = (e[0] - j00 * x) / y;
    let j11 = (e[1] - j10 * x) / y;
    let max = (libm::hypot(j00 + j11, j10 - j01) + libm::hypot(j00 - j11, j10 + j01)) * 0.5;
    let determinant = det_uv.abs() / area2;
    let min = (determinant / max).min(max);
    let density = libm::sqrt(determinant);
    let anisotropy = max / min;
    if ![area2, det_uv.abs(), min, max, density, anisotropy]
        .into_iter()
        .all(|v| v.is_finite() && v > 0.0)
    {
        return Err(UvQualityIssue::NumericLimit);
    }
    Ok(UvTriangleMetrics {
        surface_area: area2 * 0.5,
        texture_area: det_uv.abs() * 0.5,
        principal_density: [min, max],
        density,
        anisotropy,
        mirrored: det_uv < 0.0,
    })
}

#[cfg(test)]
#[path = "quality_tests.rs"]
mod tests;
