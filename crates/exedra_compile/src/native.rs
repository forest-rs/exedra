// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Immutable native source installation and explicit realization correspondence.

use crate::PartFingerprint;
mod chart;
mod inspect;
use alloc::{boxed::Box, collections::BTreeSet, sync::Arc, vec::Vec};
pub use chart::NativeChartError;
use core::mem::{size_of, size_of_val};
use exedra_analytic::{
    body::{Body, FaceId},
    bore::BoredPlate,
    extrusion::Extrusion,
    loft::Loft,
    realize::{
        EdgeChains, RealizationError, RealizationPolicy, RealizationReport, RealizedBody,
        VertexSource,
    },
    revolve::Revolution,
    sweep::Sweep,
};
use exedra_curves::path::{PathClosure, PathJoin, PathSegment3};
use exedra_curves::profile::{Profile, SegTag};
use exedra_math::Placement3;

/// Supported retained construction family, independent of display tolerance.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NativeFamily {
    /// Line/arc extrusion, including supported blind pockets and axial finishes.
    Extrusion,
    /// Line/arc profile revolution.
    Revolution,
    /// Constant-section smooth line/arc sweep.
    Sweep,
    /// Ruled loft through corresponding line/arc profiles.
    Loft,
    /// A supported rectangular plate with one fully contained cylindrical through bore.
    BoredPlate,
    /// A validated boundary representation independent of construction history.
    Body,
}

#[derive(Debug)]
enum Geometry {
    Extrusion(Extrusion),
    Revolution(Revolution),
    Sweep(Sweep),
    Loft(Loft),
    BoredPlate(Box<BoredPlate>),
    Body {
        body: Body<Vec<SegTag>>,
        frame: Placement3,
    },
}

/// Original constructor-specific geometry, preserving typed face and edge roles.
#[derive(Copy, Clone, Debug)]
pub enum NativeGeometryRef<'a> {
    /// Original extrusion and pocket identities.
    Extrusion(&'a Extrusion),
    /// Original generating profile and revolution parameters.
    Revolution(&'a Revolution),
    /// Original section and borrowed mathematical path.
    Sweep(&'a Sweep),
    /// Original corresponding placed sections.
    Loft(&'a Loft),
    /// Original plate source, authored bore and inherited/generated construction roles.
    BoredPlate(&'a BoredPlate),
    /// Generic native boundary with retained per-face authored feature tags.
    Body {
        /// Geometry and tags in body-local coordinates.
        body: &'a Body<Vec<SegTag>>,
        /// Rigid occurrence placement.
        frame: Placement3,
    },
}

/// Immutable native model installation with content identity captured once.
///
/// Clones share geometry and native selection scope. Installing a separately
/// constructed model creates a fresh scope even when content is equal. Compilers
/// may share equal-content realization buffers without sharing those scopes.
/// No scene identity, material binding, mutable construction or display policy
/// is owned here. Geometry and its typed roles remain in the analytic head.
///
/// ```
/// use exedra_analytic::extrusion::Extrusion;
/// use exedra_compile::{NativeSource, PartSource, PreparedSource, PartInput, PartCompiler, CompilePolicy, PartId};
/// use exedra_constructive::builders;
/// use exedra_math::Placement3;
/// use exedra_curves::profile::ProfilePolicy;
/// let profile = builders::circle(5.0)?.to_line_arc(&ProfilePolicy::new(1e-9))?;
/// let native = NativeSource::from(Extrusion::new(profile, Placement3::IDENTITY, 3.0)?);
/// let prepared = PreparedSource::new(PartSource::Native(native.clone()));
/// let snapshot = PartCompiler::new().compile_snapshot(
///     &[PartInput::prepared("disk", &prepared)], &CompilePolicy::default(),
/// )?;
/// assert!(snapshot.body(PartId(0), 0).unwrap().native().unwrap().same_installation(&native));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Clone, Debug)]
pub struct NativeSource {
    geometry: Arc<Geometry>,
    fingerprint: PartFingerprint,
}

/// Native integral being queried, independently of display realization.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NativeMeasure {
    /// Oriented volume in cubed model units.
    Volume,
    /// Boundary area in squared model units.
    SurfaceArea,
}

/// A native measure cannot be returned as a positive finite number.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NativeMeasureError {
    /// This construction family has no implementation for the requested measure.
    Unavailable {
        /// Construction family.
        family: NativeFamily,
        /// Requested measure.
        measure: NativeMeasure,
    },
    /// The native calculation cannot produce a positive finite result, including
    /// overflow, underflow or cancellation.
    /// This does not prevent other measures or queries on the same source.
    NumericLimit {
        /// Construction family.
        family: NativeFamily,
        /// Requested measure.
        measure: NativeMeasure,
    },
}
impl core::fmt::Display for NativeMeasureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable { family, measure } => {
                write!(f, "native {measure:?} is unavailable for {family:?}")
            }
            Self::NumericLimit { family, measure } => write!(
                f,
                "native {measure:?} for {family:?} cannot produce a positive finite result"
            ),
        }
    }
}
impl core::error::Error for NativeMeasureError {}

/// Native boundary table payload, independently of display buffers.
///
/// Byte counts use live table lengths and include inline metadata and nested
/// coedge/loop-ID arrays. They exclude spare capacity, allocator overhead,
/// constructor/source-history records such as profiles, paths or the original
/// plate, and heap storage owned by custom
/// metadata. This is boundary payload evidence, not total process residency.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct NativeBoundaryStorage {
    /// Authoritative f64 vertices.
    pub vertices: usize,
    /// Shared native edges, including collapsed pole boundaries.
    pub edges: usize,
    /// Oriented boundary loops.
    pub loops: usize,
    /// Individual edge occurrences.
    pub coedges: usize,
    /// Trimmed supporting surfaces.
    pub faces: usize,
    /// Live boundary table payload bytes, with the exclusions above.
    pub payload_bytes: usize,
}

/// Accuracy and source correspondence for one compiled native realization.
///
/// Native IDs refer to [`crate::SnapshotBody::native`]'s installed source.
/// The source remains immutable while each display policy produces its own
/// realization. Mesh IDs and triangle ordinals are local to this particular
/// realization; retain a [`NativeFaceSelection`] to cross display-policy changes.
/// Mapping through this record never reconstructs source geometry.
#[derive(Clone, Debug)]
pub struct NativeRealization {
    /// Native source of each mesh face, in mesh-face order.
    pub mesh_faces: Vec<(exedra_mesh::FaceId, FaceId)>,
    /// Native source of each emitted topology vertex.
    pub mesh_vertices: Vec<(exedra_mesh::VertexId, VertexSource)>,
    /// Shared sampled edge chains.
    pub edge_chains: EdgeChains,
    /// Sparse forward face parameters at welded pole corners.
    pub pole_corners: exedra_analytic::realize::PoleCorners,
    /// Original realization accuracy and sampling evidence, including f32 emission.
    pub report: RealizationReport,
    render_faces: Vec<FaceId>,
    chart: Option<Box<exedra_constructive::chart::SurfaceChart>>,
}
impl NativeRealization {
    /// Applied native construction-chart policy, identical to the validated request.
    ///
    /// This evidence shares the cached realization's lifetime. Borrowing it
    /// never clones or reconstructs the chart or native geometry. `None` means
    /// no native construction chart was applied; extraction may still project UVs.
    #[must_use]
    pub fn chart(&self) -> Option<&exedra_constructive::chart::SurfaceChart> {
        self.chart.as_deref()
    }
    /// Native face for one triangle in the compiler's final grouped index buffer.
    /// The triangle index is an ordinal, not an index-buffer offset.
    #[must_use]
    pub fn render_face(&self, triangle: usize) -> Option<FaceId> {
        self.render_faces.get(triangle).copied()
    }
    /// Native face per final render triangle; independent of region decoding.
    #[must_use]
    pub fn render_faces(&self) -> &[FaceId] {
        &self.render_faces
    }
    pub(crate) fn from_realized(
        realized: RealizedBody,
        render_faces: Vec<FaceId>,
        chart: Option<exedra_constructive::chart::SurfaceChart>,
    ) -> (exedra_mesh::Mesh, Self) {
        let RealizedBody {
            mesh,
            mesh_faces,
            mesh_vertices,
            edge_chains,
            pole_corners,
            report,
        } = realized;
        (
            mesh,
            Self {
                mesh_faces,
                mesh_vertices,
                edge_chains,
                pole_corners,
                report,
                render_faces,
                chart: chart.map(Box::new),
            },
        )
    }
}

/// A native face selection scoped to its immutable installed source.
/// It survives display-quality changes and compiler eviction while held.
#[derive(Clone, Debug)]
pub struct NativeFaceSelection {
    source: NativeSource,
    face: FaceId,
}
impl NativeFaceSelection {
    /// Native ID in the selected source's own body tables.
    #[must_use]
    pub const fn native_id(&self) -> FaceId {
        self.face
    }
    /// Installed source retained by the selection.
    #[must_use]
    pub fn source(&self) -> &NativeSource {
        &self.source
    }
}
/// A native face selection could not be resolved in an installed source.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NativeSelectionError {
    /// A face ID does not exist in this source.
    UnknownFace {
        /// Requested local native ID.
        face: FaceId,
    },
    /// The selected supporting surface is curved and has no planar workplane.
    NotPlanar {
        /// Requested native face.
        face: FaceId,
    },
    /// A supporting plane frame cannot be represented finitely.
    PlaneNumericLimit {
        /// Requested native face.
        face: FaceId,
    },
    /// Selection belongs to another source installation, including a new
    /// installation whose content happens to equal this model.
    SourceMismatch,
}
impl core::fmt::Display for NativeSelectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownFace { face } => write!(f, "native face {face:?} does not exist"),
            Self::NotPlanar { face } => {
                write!(f, "native face {face:?} has no planar supporting frame")
            }
            Self::PlaneNumericLimit { face } => write!(
                f,
                "native face {face:?} supporting frame exceeds numeric limits"
            ),
            Self::SourceMismatch => {
                write!(f, "native selection belongs to another source installation")
            }
        }
    }
}
impl core::error::Error for NativeSelectionError {}

enum FaceTags<'a> {
    Constructor(core::iter::Flatten<core::array::IntoIter<Option<SegTag>, 2>>),
    Body(core::iter::Copied<core::slice::Iter<'a, SegTag>>),
}
impl Iterator for FaceTags<'_> {
    type Item = SegTag;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Constructor(tags) => tags.next(),
            Self::Body(tags) => tags.next(),
        }
    }
}

impl NativeSource {
    /// Installs a validated body with caller-owned per-face feature tags.
    ///
    /// Native IDs, geometry and tags survive installation without reconstructing a
    /// constructor. The body retains its validation contract; installation does
    /// not certify global self-intersection, shell containment or solid regularity.
    /// Repeated face tags are removed once, preserving their first authored order.
    /// Construction charts and analytic integrals are unavailable for this path.
    ///
    /// # Errors
    /// Returns the invalid occurrence-frame diagnosis before installing geometry.
    pub fn from_body(
        body: Body<Vec<SegTag>>,
        frame: Placement3,
    ) -> Result<Self, exedra_math::FrameError> {
        let column = |i| frame.rows.map(|r| r[i]);
        Placement3::try_from_orthonormal_axes(
            column(0),
            column(1),
            column(2),
            column(3),
            exedra_analytic::geometry::FRAME_TOLERANCE,
        )?;
        let body = body.map_roles(
            |mut tags| {
                if tags.len() > 1 {
                    let mut seen = BTreeSet::new();
                    tags.retain(|tag| seen.insert(*tag));
                }
                tags
            },
            |()| (),
        );
        Ok(Self::new(Geometry::Body { body, frame }))
    }
    fn new(geometry: Geometry) -> Self {
        let fingerprint = source_fingerprint(&geometry);
        Self {
            geometry: Arc::new(geometry),
            fingerprint,
        }
    }
    /// Captured content identity; no geometry walk or hashing occurs on access.
    #[must_use]
    pub const fn fingerprint(&self) -> PartFingerprint {
        self.fingerprint
    }
    pub(crate) fn installation_key(&self) -> usize {
        Arc::as_ptr(&self.geometry).addr()
    }
    /// Whether two values share an installed model and its native selection scope.
    #[must_use]
    pub fn same_installation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.geometry, &other.geometry)
    }
    /// Original constructor-specific geometry without metadata erasure or copying.
    #[must_use]
    pub fn geometry(&self) -> NativeGeometryRef<'_> {
        match self.geometry.as_ref() {
            Geometry::Extrusion(v) => NativeGeometryRef::Extrusion(v),
            Geometry::Revolution(v) => NativeGeometryRef::Revolution(v),
            Geometry::Sweep(v) => NativeGeometryRef::Sweep(v),
            Geometry::Loft(v) => NativeGeometryRef::Loft(v),
            Geometry::BoredPlate(v) => NativeGeometryRef::BoredPlate(v),
            Geometry::Body { body, frame } => NativeGeometryRef::Body {
                body,
                frame: *frame,
            },
        }
    }
    /// Original extrusion, or `None` for another construction family.
    #[must_use]
    pub fn as_extrusion(&self) -> Option<&Extrusion> {
        if let Geometry::Extrusion(v) = self.geometry.as_ref() {
            Some(v)
        } else {
            None
        }
    }
    /// Supported construction family.
    #[must_use]
    pub fn family(&self) -> NativeFamily {
        self.geometry().family()
    }
    /// Complete rigid occurrence placement, separate from body-local geometry.
    #[must_use]
    pub fn frame(&self) -> Placement3 {
        self.geometry().frame()
    }
    /// Structural native chart support, without metric checks or realization.
    ///
    /// Use [`Self::check_chart`] to check a requested configuration. Coordinate
    /// representability is checked during realization. `None` is an explicit
    /// absence of a charted path for this family, not an assessment of geometry validity.
    #[must_use]
    pub fn chart_kind(&self) -> Option<exedra_constructive::chart::SurfaceChartKind> {
        self.geometry().chart_kind()
    }
    /// Current construction end cap, if present, in this source's native tables.
    /// Use [`Self::select_face`] when retaining the result as a scoped selection.
    #[must_use]
    pub fn cap(&self, end: exedra_analytic::End) -> Option<FaceId> {
        self.geometry().cap(end)
    }

    /// Distinct authored feature tags on one native face, without allocating.
    ///
    /// A loft wall can carry both source-section tags. Cap faces have none.
    /// Tags are geometry provenance; callers own names and material bindings.
    /// # Errors
    /// Returns [`NativeSelectionError::UnknownFace`] for an absent source-local ID.
    pub fn face_tags(
        &self,
        face: FaceId,
    ) -> Result<impl Iterator<Item = SegTag>, NativeSelectionError> {
        self.geometry().face_tags(face)
    }
    /// Copies retained geometry into construction-independent tagged body tables.
    ///
    /// Native table IDs and all face feature tags are preserved. Constructor roles
    /// and edge metadata are intentionally replaced by tags and unit metadata;
    /// occurrence placement remains available separately through [`Self::frame`].
    #[must_use]
    pub fn to_body(&self) -> Body<Vec<SegTag>> {
        self.geometry().to_body()
    }
    /// Finite conservative native support bounds in occurrence coordinates.
    ///
    /// This enclosure is independent of display quality and may be loose around
    /// trims, partial angular surfaces and rotated source-space boxes.
    /// # Errors
    /// Returns the analytic bounds diagnosis or a numeric limit during placement.
    pub fn conservative_bounds(
        &self,
    ) -> Result<[[f64; 3]; 2], exedra_analytic::bounds::BoundsError> {
        match self.geometry() {
            NativeGeometryRef::Extrusion(v) => v.body().conservative_bounds_in(v.frame()),
            NativeGeometryRef::Revolution(v) => v.body().conservative_bounds_in(v.frame()),
            NativeGeometryRef::Sweep(v) => v.body().conservative_bounds_in(v.frame()),
            NativeGeometryRef::Loft(v) => v.body().conservative_bounds_in(v.frame()),
            NativeGeometryRef::BoredPlate(v) => v.body().conservative_bounds_in(v.frame()),
            NativeGeometryRef::Body { body, frame } => body.conservative_bounds_in(frame),
        }
    }

    /// Exact oriented supporting-plane frame in occurrence coordinates.
    ///
    /// The datum is parameter zero of the plane; trims do not reposition it.
    /// Curved faces are refused explicitly rather than approximated by mesh normals.
    /// # Errors
    /// Returns a typed unknown-face, curved-support or numeric-limit diagnosis.
    pub fn planar_frame(&self, face: FaceId) -> Result<Placement3, NativeSelectionError> {
        self.geometry().planar_frame(face)
    }
    /// Positive finite native boundary area in squared model units, without meshing.
    ///
    /// # Errors
    /// Returns [`NativeMeasureError::Unavailable`] for ruled lofts, or
    /// [`NativeMeasureError::NumericLimit`] when the calculation exceeds numeric limits.
    pub fn surface_area(&self) -> Result<f64, NativeMeasureError> {
        self.measure(NativeMeasure::SurfaceArea)
    }
    /// Positive finite native volume in cubed model units, without meshing.
    /// Supporting constructions do not subtract distant self-overlap; inspect
    /// their contracts.
    ///
    /// # Errors
    /// Returns [`NativeMeasureError::Unavailable`] for ruled lofts, or
    /// [`NativeMeasureError::NumericLimit`] when the calculation exceeds numeric limits.
    pub fn volume(&self) -> Result<f64, NativeMeasureError> {
        self.measure(NativeMeasure::Volume)
    }
    fn measure(&self, measure: NativeMeasure) -> Result<f64, NativeMeasureError> {
        let family = self.family();
        let value = match (measure, self.geometry()) {
            (NativeMeasure::SurfaceArea, NativeGeometryRef::Extrusion(v)) => v.surface_area(),
            (NativeMeasure::SurfaceArea, NativeGeometryRef::Revolution(v)) => v.surface_area(),
            (NativeMeasure::SurfaceArea, NativeGeometryRef::Sweep(v)) => v.surface_area(),
            (NativeMeasure::Volume, NativeGeometryRef::Extrusion(v)) => v.volume(),
            (NativeMeasure::Volume, NativeGeometryRef::Revolution(v)) => v.volume(),
            (NativeMeasure::Volume, NativeGeometryRef::Sweep(v)) => v.volume(),
            (NativeMeasure::SurfaceArea, NativeGeometryRef::BoredPlate(v)) => v.surface_area(),
            (NativeMeasure::Volume, NativeGeometryRef::BoredPlate(v)) => v.volume(),
            (_, NativeGeometryRef::Loft(_) | NativeGeometryRef::Body { .. }) => {
                return Err(NativeMeasureError::Unavailable { family, measure });
            }
        };
        if value.is_finite() && value > 0.0 {
            Ok(value)
        } else {
            Err(NativeMeasureError::NumericLimit { family, measure })
        }
    }
    /// Live boundary table payload and work counts, separate from realizations.
    #[must_use]
    pub fn boundary_storage(&self) -> NativeBoundaryStorage {
        match self.geometry() {
            NativeGeometryRef::Extrusion(v) => boundary_storage(v.body()),
            NativeGeometryRef::Revolution(v) => boundary_storage(v.body()),
            NativeGeometryRef::Sweep(v) => boundary_storage(v.body()),
            NativeGeometryRef::Loft(v) => boundary_storage(v.body()),
            NativeGeometryRef::BoredPlate(v) => boundary_storage(v.body()),
            NativeGeometryRef::Body { body, .. } => boundary_storage(body),
        }
    }
    /// Selects a native face in this installation.
    ///
    /// # Errors
    /// Returns the unknown native face ID.
    pub fn select_face(&self, face: FaceId) -> Result<NativeFaceSelection, NativeSelectionError> {
        let faces = match self.geometry() {
            NativeGeometryRef::Extrusion(v) => v.body().faces().len(),
            NativeGeometryRef::Revolution(v) => v.body().faces().len(),
            NativeGeometryRef::Sweep(v) => v.body().faces().len(),
            NativeGeometryRef::Loft(v) => v.body().faces().len(),
            NativeGeometryRef::BoredPlate(v) => v.body().faces().len(),
            NativeGeometryRef::Body { body, .. } => body.faces().len(),
        };
        if face.index() >= faces {
            return Err(NativeSelectionError::UnknownFace { face });
        }
        Ok(NativeFaceSelection {
            source: self.clone(),
            face,
        })
    }
    /// Resolves a selection only within the same source installation.
    ///
    /// # Errors
    /// Rejects another model/revision even when its content is identical.
    pub fn resolve_face(
        &self,
        selection: &NativeFaceSelection,
    ) -> Result<FaceId, NativeSelectionError> {
        if !self.same_installation(&selection.source) {
            return Err(NativeSelectionError::SourceMismatch);
        }
        Ok(selection.face)
    }
    pub(crate) fn realize(
        &self,
        policy: &RealizationPolicy,
    ) -> Result<RealizedBody, RealizationError> {
        match self.geometry() {
            NativeGeometryRef::Extrusion(v) => {
                v.body().realize_with_regions(v.frame(), policy, |_, _| 0)
            }
            NativeGeometryRef::Revolution(v) => v.realize_with_regions(policy, |_, _| 0),
            NativeGeometryRef::Sweep(v) => v.realize_with_regions(policy, |_, _| 0),
            NativeGeometryRef::Loft(v) => v.realize_with_regions(policy, |_, _| 0),
            NativeGeometryRef::BoredPlate(v) => {
                v.body().realize_with_regions(v.frame(), policy, |_, _| 0)
            }
            NativeGeometryRef::Body { body, frame } => {
                body.realize_with_regions(frame, policy, |_, _| 0)
            }
        }
    }
}
impl<'a> NativeGeometryRef<'a> {
    /// Authored feature tags for a native face, borrowed without allocation.
    /// # Errors
    /// Returns the absent source-local face identity.
    pub fn face_tags(
        self,
        face: FaceId,
    ) -> Result<impl Iterator<Item = SegTag> + 'a, NativeSelectionError> {
        fn check<F, E>(body: &Body<F, E>, face: FaceId) -> Result<(), NativeSelectionError> {
            body.face(face)
                .map(|_| ())
                .ok_or(NativeSelectionError::UnknownFace { face })
        }
        if let NativeGeometryRef::Body { body, .. } = self {
            let tags = &body
                .face(face)
                .ok_or(NativeSelectionError::UnknownFace { face })?
                .role;
            return Ok(FaceTags::Body(tags.iter().copied()));
        }
        let mut tags = match self {
            NativeGeometryRef::Extrusion(v) => {
                check(v.body(), face)?;
                [v.face_tag(face), None]
            }
            NativeGeometryRef::Revolution(v) => {
                check(v.body(), face)?;
                [v.face_tag(face), None]
            }
            NativeGeometryRef::Sweep(v) => {
                check(v.body(), face)?;
                [v.face_tag(face), None]
            }
            NativeGeometryRef::BoredPlate(v) => {
                check(v.body(), face)?;
                [v.face_tag(face), None]
            }
            NativeGeometryRef::Body { .. } => unreachable!("handled generic face tags"),
            NativeGeometryRef::Loft(v) => match v
                .body()
                .face(face)
                .ok_or(NativeSelectionError::UnknownFace { face })?
                .role
            {
                exedra_analytic::loft::FaceRole::Wall { tags, .. } => tags,
                exedra_analytic::loft::FaceRole::Cap { .. } => [None; 2],
            },
        };
        if tags[0] == tags[1] {
            tags[1] = None;
        }
        Ok(FaceTags::Constructor(tags.into_iter().flatten()))
    }
    /// Copies this boundary into generic tagged tables, retaining native IDs.
    /// Constructor roles become feature tags; edge metadata becomes unit metadata.
    /// The occurrence frame is available separately through [`Self::frame`].
    #[must_use]
    pub fn to_body(self) -> Body<Vec<SegTag>> {
        fn copy<F: Clone, E: Clone>(
            body: &Body<F, E>,
            source: NativeGeometryRef<'_>,
        ) -> Body<Vec<SegTag>> {
            let mut index = 0_u32;
            body.clone().map_roles(
                |_| {
                    let tags = source
                        .face_tags(FaceId(index))
                        .expect("iterated native face")
                        .collect();
                    index += 1;
                    tags
                },
                |_| (),
            )
        }
        match self {
            Self::Extrusion(v) => copy(v.body(), self),
            Self::Revolution(v) => copy(v.body(), self),
            Self::Sweep(v) => copy(v.body(), self),
            Self::Loft(v) => copy(v.body(), self),
            Self::BoredPlate(v) => copy(v.body(), self),
            Self::Body { body, .. } => body.clone(),
        }
    }
}
macro_rules! install {
    ($ty:ty,$variant:ident) => {
        impl From<$ty> for NativeSource {
            fn from(value: $ty) -> Self {
                Self::new(Geometry::$variant(value))
            }
        }
    };
}
install!(Extrusion, Extrusion);
install!(Revolution, Revolution);
install!(Sweep, Sweep);
install!(Loft, Loft);
/// Installs a checked spline extrusion through the construction-independent body
/// path. Exact geometry, occurrence placement and wall feature tags are retained.
/// This source reports `NativeFamily::Body`; constructor UV mapping remains on
/// `PlanarExtrusion`, and exact integral measurements are unavailable.
impl From<exedra_analytic::planar_extrusion::PlanarExtrusion> for NativeSource {
    fn from(solid: exedra_analytic::planar_extrusion::PlanarExtrusion) -> Self {
        let frame = solid.frame();
        let body = solid.into_body().map_roles(
            |role| match role {
                exedra_analytic::extrusion::FaceRole::Wall { tag: Some(tag), .. } => {
                    alloc::vec![tag]
                }
                _ => Vec::new(),
            },
            |_| (),
        );
        Self::from_body(body, frame).expect("constructor checked the occurrence frame")
    }
}

impl From<BoredPlate> for NativeSource {
    fn from(value: BoredPlate) -> Self {
        Self::new(Geometry::BoredPlate(Box::new(value)))
    }
}

fn boundary_storage<F, E>(body: &Body<F, E>) -> NativeBoundaryStorage {
    let coedges = body.loops().iter().map(|l| l.coedges.len()).sum::<usize>();
    let face_loops = body.faces().iter().map(|f| f.loops.len()).sum::<usize>();
    NativeBoundaryStorage {
        vertices: body.vertices().len(),
        edges: body.edges().len(),
        loops: body.loops().len(),
        coedges,
        faces: body.faces().len(),
        payload_bytes: size_of_val(body.vertices())
            + size_of_val(body.edges())
            + size_of_val(body.loops())
            + size_of_val(body.faces())
            + coedges * size_of::<exedra_analytic::body::Coedge>()
            + face_loops * size_of::<exedra_analytic::body::LoopId>()
            + size_of_val(body.trims())
            + body
                .trims()
                .iter()
                .map(|trims| size_of_val(trims.curves.as_slice()))
                .sum::<usize>(),
    }
}

// Canonical defining inputs, encoded explicitly once when a source is installed.
// Float words retain signed zero and all authored feature labels. This stream
// does not depend on Debug output, pointers, mesh tolerance or display buffers.
struct Identity(u128);
impl Identity {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u128::from(b);
            self.0 = self.0.wrapping_mul(0x0000000001000000000000000000013b);
        }
    }
    fn word(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }
    fn tag(&mut self, tag: Option<SegTag>) {
        self.word(u64::from(tag.is_some()));
        if let Some(t) = tag {
            self.word(u64::from(t.0));
        }
    }
    fn values<const N: usize>(&mut self, values: [f64; N]) {
        for v in values {
            self.word(v.to_bits());
        }
    }
    fn frame(&mut self, frame: Placement3) {
        for row in frame.rows {
            self.values(row);
        }
    }
    fn curve(&mut self, curve: &exedra_analytic::geometry::Curve) {
        use exedra_analytic::geometry::Curve;
        match curve {
            Curve::Line(v) => {
                self.word(0);
                self.values(v.origin());
                self.values(v.direction());
            }
            Curve::Circle(v) => {
                self.word(1);
                self.frame(v.frame());
                self.values([v.radius()]);
            }
            Curve::Ellipse(v) => {
                self.word(2);
                self.frame(v.frame());
                self.values([v.semi_major(), v.semi_minor()]);
            }
            Curve::Nurbs(v) => {
                self.word(3);
                self.nurbs(v.curve());
            }
        }
    }
    fn nurbs<const N: usize>(&mut self, curve: &exedra_analytic::geometry::NurbsCurve<N>) {
        self.word(u64::from(curve.order()));
        self.word(curve.knots().len() as u64);
        for &knot in curve.knots() {
            self.values([knot]);
        }
        self.word(curve.points().len() as u64);
        for &point in curve.points() {
            self.values(point);
        }
        match curve.weights() {
            None => self.word(0),
            Some(weights) => {
                self.word(1);
                for &weight in weights {
                    self.values([weight]);
                }
            }
        }
    }
    fn surface(&mut self, surface: &exedra_analytic::geometry::Surface) {
        use exedra_analytic::geometry::Surface;
        match surface {
            Surface::Plane(v) => {
                self.word(0);
                self.frame(v.frame());
            }
            Surface::Cylinder(v) => {
                self.word(1);
                self.frame(v.frame());
                self.values([v.radius()]);
            }
            Surface::Cone(v) => {
                self.word(2);
                self.frame(v.frame());
                self.values([v.radius(), v.slope()]);
            }
            Surface::Sphere(v) => {
                self.word(3);
                self.frame(v.frame());
                self.values([v.radius()]);
            }
            Surface::Torus(v) => {
                self.word(4);
                self.frame(v.frame());
                self.values([v.major_radius(), v.minor_radius()]);
            }
            Surface::Ruled(v) => {
                self.word(5);
                for span in v.curves() {
                    self.curve(span.curve());
                    self.values(span.range().bounds());
                    self.word(u64::from(span.reversed()));
                }
            }
        }
    }
    fn body(&mut self, body: &Body<Vec<SegTag>>) {
        self.word(body.vertices().len() as u64);
        for &v in body.vertices() {
            self.values(v);
        }
        self.word(body.edges().len() as u64);
        for edge in body.edges() {
            self.word(u64::from(edge.start.0));
            self.word(u64::from(edge.end.0));
            if let Some((curve, range)) = edge.regular() {
                self.word(1);
                self.curve(curve);
                self.values(range.bounds());
            } else {
                self.word(0);
            }
        }
        self.word(body.loops().len() as u64);
        for ring in body.loops() {
            self.word(ring.coedges.len() as u64);
            for c in &ring.coedges {
                self.word(u64::from(c.edge.0));
                self.word(u64::from(c.reversed));
            }
        }
        self.word(body.trims().len() as u64);
        for trims in body.trims() {
            self.word(u64::from(trims.loop_id.0));
            self.word(trims.curves.len() as u64);
            for curve in &trims.curves {
                match curve {
                    exedra_analytic::body::TrimCurve::Line { start, end } => {
                        self.word(0);
                        self.values(*start);
                        self.values(*end);
                    }
                    exedra_analytic::body::TrimCurve::Harmonic {
                        origin,
                        linear,
                        cosine,
                        sine,
                        range,
                    } => {
                        self.word(1);
                        self.values(*origin);
                        self.values(*linear);
                        self.values(*cosine);
                        self.values(*sine);
                        self.values(range.bounds());
                    }
                    exedra_analytic::body::TrimCurve::Nurbs { curve, range } => {
                        self.word(2);
                        self.nurbs(curve);
                        self.values(range.bounds());
                    }
                }
            }
        }
        self.word(body.faces().len() as u64);
        for face in body.faces() {
            self.surface(&face.surface);
            self.values(face.domain.u.bounds());
            self.values(face.domain.v.bounds());
            self.word(u64::from(
                face.sense == exedra_analytic::body::Sense::Reverse,
            ));
            self.word(face.loops.len() as u64);
            for ring in &face.loops {
                self.word(u64::from(ring.0));
            }
            self.word(face.role.len() as u64);
            for tag in &face.role {
                self.word(u64::from(tag.0));
            }
        }
    }
    fn profile(&mut self, profile: &Profile) {
        self.word(profile.holes().len() as u64);
        for ring in core::iter::once(profile.outer()).chain(profile.holes()) {
            self.word(ring.segments().len() as u64);
            for segment in ring.segments() {
                self.segment(segment);
            }
        }
    }
    fn segment(&mut self, segment: &exedra_curves::profile::Segment) {
        use exedra_curves::profile::Curve;
        self.values(segment.to);
        self.tag(segment.tag);
        match &segment.curve {
            Curve::Line => self.word(0),
            Curve::Arc { bulge } => {
                self.word(1);
                self.values([*bulge]);
            }
            Curve::ArcCenter { center, clockwise } => {
                self.word(2);
                self.values(*center);
                self.word(u64::from(*clockwise));
            }
            Curve::Bezier(bezier) => {
                self.word(3);
                self.word(bezier.interior().len() as u64);
                for &control in bezier.interior() {
                    self.values(control);
                }
                match bezier.weights() {
                    None => self.word(0),
                    Some(weights) => {
                        self.word(1);
                        for &weight in weights {
                            self.values([weight]);
                        }
                    }
                }
            }
        }
    }
}
fn source_fingerprint(geometry: &Geometry) -> PartFingerprint {
    let mut id = Identity(0x6c62272e07bb014262b821756295c58d);
    id.bytes(b"exedra-native-source-v1");
    match geometry {
        Geometry::Body { body, frame } => {
            id.word(5);
            id.frame(*frame);
            id.body(body);
        }
        Geometry::Extrusion(v) => {
            id.word(0);
            id.frame(v.frame());
            id.profile(v.profile());
            id.values([v.length()]);
            id.word(v.pockets().len() as u64);
            for p in v.pockets() {
                id.word(u64::from(p.end == exedra_analytic::End::End));
                id.values([p.depth]);
                id.tag(p.tag);
                id.word(p.outline.segments().len() as u64);
                for s in p.outline.segments() {
                    id.segment(s);
                }
            }
        }
        Geometry::BoredPlate(v) => {
            id.word(4);
            id.frame(v.frame());
            id.profile(v.source().profile());
            id.values([v.source().length()]);
            id.values(v.bore().point());
            id.values(v.bore().direction());
            id.values([v.bore().radius()]);
            id.tag(v.bore().tag());
            // The finite enclosure is retained native content too, including its
            // explicitly padded axial bounds. Capturing it is constant work.
            let domain = v.body().face(v.report().wall).expect("bore wall").domain;
            id.values(domain.u.bounds());
            id.values(domain.v.bounds());
        }
        Geometry::Revolution(v) => {
            id.word(1);
            id.frame(v.frame());
            id.profile(v.profile());
            id.values([v.sweep()]);
        }
        Geometry::Sweep(v) => {
            id.word(2);
            id.frame(v.frame());
            id.profile(v.profile());
            let p = v.path();
            id.values(p.start);
            id.values(p.section_x);
            id.values(p.section_origin);
            match p.closure {
                PathClosure::Open => id.word(0),
                PathClosure::ClosedPlanar { normal } => {
                    id.word(1);
                    id.values(normal);
                }
            }
            match p.joins {
                PathJoin::Smooth => id.word(0),
                PathJoin::Miter { limit } => {
                    id.word(1);
                    id.values([limit]);
                }
            }
            id.word(p.segments.len() as u64);
            for s in p.segments {
                match s {
                    PathSegment3::Line { to } => {
                        id.word(0);
                        id.values(*to);
                    }
                    PathSegment3::Arc {
                        axis_origin,
                        axis,
                        sweep,
                    } => {
                        id.word(1);
                        id.values(*axis_origin);
                        id.values(*axis);
                        id.values([*sweep]);
                    }
                    PathSegment3::Cubic {
                        control1,
                        control2,
                        to,
                    } => {
                        id.word(2);
                        id.values(*control1);
                        id.values(*control2);
                        id.values(*to);
                    }
                }
            }
        }
        Geometry::Loft(v) => {
            id.word(3);
            id.frame(v.frame());
            id.word(v.sections().len() as u64);
            for (f, p) in v.sections() {
                id.frame(*f);
                id.profile(p);
            }
        }
    }
    PartFingerprint(id.0)
}

#[cfg(test)]
mod centered_identity_tests {
    use super::*;
    use alloc::vec;
    use exedra_curves::profile::{Loop, ProfilePolicy, Segment};
    fn circle(center: [f64; 2], centered: bool) -> Profile {
        let segment = |to| {
            if centered {
                Segment::arc_centered(to, center, false)
            } else {
                Segment::arc(to, 1.0)
            }
        };
        Profile::new(
            Loop::new(vec![segment([-1.0, 0.0]), segment([1.0, 0.0])]).unwrap(),
            vec![],
            &ProfilePolicy::new(1e-9),
        )
        .unwrap()
    }
    fn hash(profile: &Profile) -> u128 {
        let mut id = Identity(0x6c62272e07bb014262b821756295c58d);
        id.profile(profile);
        id.0
    }
    #[test]
    fn source_identity_keeps_curve_representation_and_authoritative_center_bits() {
        let bulge = circle([0.0; 2], false);
        let center = circle([0.0; 2], true);
        let signed_center = circle([-0.0, 0.0], true);
        assert_eq!(bulge.area(), center.area());
        assert_eq!(center.area(), signed_center.area());
        assert_ne!(hash(&bulge), hash(&center));
        assert_ne!(hash(&center), hash(&signed_center));
        assert_eq!(hash(&center), hash(&center.clone()));
    }
}
