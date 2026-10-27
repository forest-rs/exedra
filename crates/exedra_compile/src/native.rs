// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Immutable native source installation and explicit realization correspondence.

use crate::PartFingerprint;
mod chart;
use alloc::{boxed::Box, sync::Arc, vec::Vec};
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
use exedra_math::{
    Placement3,
    path::{PathClosure, PathJoin, PathSegment3},
};
use exedra_profile::{Profile, SegTag};

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
}

#[derive(Debug)]
enum Geometry {
    Extrusion(Extrusion),
    Revolution(Revolution),
    Sweep(Sweep),
    Loft(Loft),
    BoredPlate(Box<BoredPlate>),
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
/// use exedra_profile::ProfilePolicy;
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
    /// Bounded shared curves.
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
            report,
        } = realized;
        (
            mesh,
            Self {
                mesh_faces,
                mesh_vertices,
                edge_chains,
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

impl NativeSource {
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
        match self.geometry() {
            NativeGeometryRef::Extrusion(_) => NativeFamily::Extrusion,
            NativeGeometryRef::Revolution(_) => NativeFamily::Revolution,
            NativeGeometryRef::Sweep(_) => NativeFamily::Sweep,
            NativeGeometryRef::Loft(_) => NativeFamily::Loft,
            NativeGeometryRef::BoredPlate(_) => NativeFamily::BoredPlate,
        }
    }
    /// Complete rigid occurrence placement, separate from body-local geometry.
    #[must_use]
    pub fn frame(&self) -> Placement3 {
        match self.geometry() {
            NativeGeometryRef::Extrusion(v) => v.frame(),
            NativeGeometryRef::Revolution(v) => v.frame(),
            NativeGeometryRef::Sweep(v) => v.frame(),
            NativeGeometryRef::Loft(v) => v.frame(),
            NativeGeometryRef::BoredPlate(v) => v.frame(),
        }
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
        fn check<F, E>(body: &Body<F, E>, face: FaceId) -> Result<(), NativeSelectionError> {
            body.face(face)
                .map(|_| ())
                .ok_or(NativeSelectionError::UnknownFace { face })
        }
        let mut tags = match self.geometry() {
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
        Ok(tags.into_iter().flatten())
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
        }
    }

    /// Exact oriented supporting-plane frame in occurrence coordinates.
    ///
    /// The datum is parameter zero of the plane; trims do not reposition it.
    /// Curved faces are refused explicitly rather than approximated by mesh normals.
    /// # Errors
    /// Returns a typed unknown-face, curved-support or numeric-limit diagnosis.
    pub fn planar_frame(&self, face: FaceId) -> Result<Placement3, NativeSelectionError> {
        fn local<F, E>(
            body: &Body<F, E>,
            face: FaceId,
        ) -> Result<Placement3, NativeSelectionError> {
            if !matches!(
                body.face(face)
                    .ok_or(NativeSelectionError::UnknownFace { face })?
                    .surface,
                exedra_analytic::geometry::Surface::Plane(_)
            ) {
                return Err(NativeSelectionError::NotPlanar { face });
            }
            body.planar_frame(face, [0.0; 2])
                .ok_or(NativeSelectionError::PlaneNumericLimit { face })
        }
        let plane = match self.geometry() {
            NativeGeometryRef::Extrusion(v) => local(v.body(), face),
            NativeGeometryRef::Revolution(v) => local(v.body(), face),
            NativeGeometryRef::Sweep(v) => local(v.body(), face),
            NativeGeometryRef::Loft(v) => local(v.body(), face),
            NativeGeometryRef::BoredPlate(v) => local(v.body(), face),
        }?;
        let frame = self.frame() * plane;
        if frame.rows.iter().flatten().any(|v| !v.is_finite()) {
            return Err(NativeSelectionError::PlaneNumericLimit { face });
        }
        Ok(frame)
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
            (_, NativeGeometryRef::Loft(_)) => {
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
            + face_loops * size_of::<exedra_analytic::body::LoopId>(),
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
    fn profile(&mut self, profile: &Profile) {
        self.word(profile.holes().len() as u64);
        for ring in core::iter::once(profile.outer()).chain(profile.holes()) {
            self.word(ring.segments().len() as u64);
            for segment in ring.segments() {
                self.values(segment.to);
                self.tag(segment.tag);
                match segment.curve {
                    exedra_profile::Curve::Line => self.word(0),
                    exedra_profile::Curve::Arc { bulge } => {
                        self.word(1);
                        self.values([bulge]);
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
                    id.values(s.to);
                    id.tag(s.tag);
                    match s.curve {
                        exedra_profile::Curve::Line => id.word(0),
                        exedra_profile::Curve::Arc { bulge } => {
                            id.word(1);
                            id.values([bulge]);
                        }
                    }
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
