// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Retained constant-section sweeps with shared native section boundaries.
//!
//! Line/arc profiles follow smooth open line/circular-arc paths. Section
//! orientation is transported analytically, without sampling the path first.
//! Local bend-axis contact and authored sharp corners are refused. Boundary
//! validation does not certify absence of distant self-intersections.

use crate::End;
use crate::body::{Body, BodyError, EdgeId, FaceId};
use crate::geometry::GeometryError;
use crate::realize::{RealizationError, RealizationPolicy, RealizedBody};
use alloc::vec::Vec;
use exedra_curves::profile::{IntoProfile, Profile, ProfilePolicy, SegTag, SegmentRef};
use exedra_math::Placement3;
use exedra_math::path::{CurvePathSource, CurvePathView, PathClosure, PathJoin, PathSegment3};

mod build;
mod chart;
pub use chart::SweepUvError;

/// Native origin of a swept face.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FaceRole {
    /// A source profile segment carried along one authored path span.
    Wall {
        /// Authored path segment, independent of tessellation density.
        path_segment: usize,
        /// Source profile segment.
        segment: SegmentRef,
        /// Caller feature label.
        tag: Option<SegTag>,
    },
    /// A planar start or end cap.
    Cap {
        /// Construction end.
        end: End,
    },
}

/// Native origin of a swept edge.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum EdgeRole {
    /// One shared section edge at an authored path station.
    Section {
        /// Station zero is the start; station N follows path segment N-1.
        station: usize,
        /// Source profile segment.
        segment: SegmentRef,
    },
    /// Trajectory of one source profile vertex through a path span.
    Rail {
        /// Authored path segment.
        path_segment: usize,
        /// Segment starting at this profile vertex.
        vertex: SegmentRef,
    },
}

/// A constant-section native solid over a smooth open line-and-arc path.
#[derive(Clone, Debug, PartialEq)]
pub struct Sweep {
    profile: Profile,
    frame: Placement3,
    body: Body<FaceRole, EdgeRole>,
    path_start: [f64; 3],
    path_segments: Vec<PathSegment3>,
    section_x: [f64; 3],
    joins: PathJoin,
    path_distances: Vec<f64>,
    section_origin: [f64; 2],
    area: f64,
    volume: f64,
}

/// A construction refusal retaining authored path/profile context.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SweepError {
    /// Placement, supporting geometry, or a derived frame is invalid.
    Geometry(GeometryError),
    /// The path is empty, closed, or not represented by explicit curve spans.
    UnsupportedPath,
    /// The authored miter stretch limit is non-finite or smaller than one.
    Join,
    /// A span is cubic, malformed, stationary, or makes a complete turn.
    PathSegment {
        /// Authored span.
        segment: usize,
    },
    /// The section-X direction cannot define an initial perpendicular frame.
    SectionDirection,
    /// The profile-space section datum is non-finite.
    SectionOrigin,
    /// A curved wall would require a nonregular torus.
    NonRegularArc {
        /// Authored path span.
        path_segment: usize,
        /// Unsupported profile arc.
        segment: SegmentRef,
        /// Generating circle's distance from the bend axis.
        major_radius: f64,
        /// Generating circle radius.
        minor_radius: f64,
    },
    /// An authored tangent discontinuity needs a different corner construction.
    Corner {
        /// Span beginning at the corner.
        segment: usize,
    },
    /// A swept profile piece touches or passes a bend axis.
    AxisContact {
        /// Authored path span.
        path_segment: usize,
        /// Offending source segment, including its interior arc extrema.
        segment: SegmentRef,
        /// Minimum signed radius in the bend's meridian plane.
        radius: f64,
    },
    /// Construction exceeds finite arithmetic or local table capacity.
    NumericLimit,
    /// Generated native boundary agreement failed.
    Boundary(BodyError),
}
impl core::fmt::Display for SweepError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "native sweep: {self:?}")
    }
}
impl core::error::Error for SweepError {}

/// Profile conversion and native sweep construction remain separate diagnoses.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileSweepError<E> {
    /// Original authoring/conversion failure.
    Profile(E),
    /// Native path or boundary construction failure.
    Sweep(SweepError),
}
impl<E: core::fmt::Display> core::fmt::Display for ProfileSweepError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Profile(e) => write!(f, "sweep profile: {e}"),
            Self::Sweep(e) => e.fmt(f),
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for ProfileSweepError<E> {}

impl Sweep {
    /// Constructs a native boundary from a checked profile and borrowed path.
    ///
    /// The path's section-X and datum have the same meaning as in constructive
    /// sweeping. Joins must be tangent-continuous; there is no implicit corner
    /// smoothing, endpoint snapping or mesh Boolean stitching.
    ///
    /// # Errors
    /// Returns authored span/segment failures, local bend-axis contact, invalid
    /// frames, finite-work limits or native boundary validation failures.
    pub fn new(
        profile: Profile,
        path: &impl CurvePathSource,
        frame: Placement3,
    ) -> Result<Self, SweepError> {
        build::construct(profile, path, frame)
    }

    /// Converts an authored profile under `policy`, then constructs a native sweep.
    ///
    /// # Errors
    /// Preserves the original conversion error or the native [`SweepError`].
    pub fn from_profile<P: IntoProfile>(
        profile: P,
        path: &impl CurvePathSource,
        frame: Placement3,
        policy: &ProfilePolicy,
    ) -> Result<Self, ProfileSweepError<P::Error>> {
        Self::new(
            profile
                .into_profile(policy)
                .map_err(ProfileSweepError::Profile)?,
            path,
            frame,
        )
        .map_err(ProfileSweepError::Sweep)
    }

    /// Independent native boundary geometry in path-local coordinates.
    #[must_use]
    pub fn body(&self) -> &Body<FaceRole, EdgeRole> {
        &self.body
    }
    /// Original checked section profile.
    #[must_use]
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
    /// Rigid occurrence placement.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
    /// Sum of authored centerline span lengths in model units.
    #[must_use]
    pub fn path_length(&self) -> f64 {
        *self.path_distances.last().expect("nonempty path")
    }

    /// Borrowed source spans and moving-section intent in path-local coordinates.
    #[must_use]
    pub fn path(&self) -> CurvePathView<'_> {
        CurvePathView {
            start: self.path_start,
            segments: &self.path_segments,
            section_x: self.section_x,
            section_origin: self.section_origin,
            closure: PathClosure::Open,
            joins: self.joins,
        }
    }

    /// Oriented enclosed volume of the constructed boundary.
    /// Does not subtract overlaps of distant spans that self-intersect.
    #[must_use]
    pub const fn volume(&self) -> f64 {
        self.volume
    }
    /// Total native wall and cap area.
    #[must_use]
    pub const fn surface_area(&self) -> f64 {
        self.area
    }
    /// Planar boundary at the requested construction end, independent of display quality.
    #[must_use]
    pub fn cap(&self, end: End) -> Option<FaceId> {
        self.body
            .faces()
            .iter()
            .position(|f| f.role == FaceRole::Cap { end })
            .map(|i| FaceId(crate::usize_to_u32(i)))
    }
    /// Original feature label on a wall; caps have none.
    #[must_use]
    pub fn face_tag(&self, face: FaceId) -> Option<SegTag> {
        match self.body.face(face)?.role {
            FaceRole::Wall { tag, .. } => tag,
            FaceRole::Cap { .. } => None,
        }
    }
    /// Every native wall carrying the requested feature label.
    pub fn faces_tagged(&self, tag: SegTag) -> impl Iterator<Item = FaceId> + '_ {
        self.body
            .faces()
            .iter()
            .enumerate()
            .filter_map(move |(i, _)| {
                let face = FaceId(crate::usize_to_u32(i));
                (self.face_tag(face) == Some(tag)).then_some(face)
            })
    }
    /// Native source of a bounded edge.
    #[must_use]
    pub fn edge_role(&self, edge: EdgeId) -> Option<EdgeRole> {
        self.body.edge(edge).map(|e| e.role)
    }
    /// Realizes the independent boundary, retaining native correspondence.
    ///
    /// # Errors
    /// Returns the shared realizer's typed accuracy, topology and storage refusals.
    pub fn realize(&self, policy: &RealizationPolicy) -> Result<RealizedBody, RealizationError> {
        self.body.realize(self.frame, policy)
    }
    /// Realizes with one caller region per native face.
    ///
    /// # Errors
    /// Returns the same failures as [`Self::realize`].
    pub fn realize_with_regions(
        &self,
        policy: &RealizationPolicy,
        region: impl Fn(FaceId, &crate::body::Face<FaceRole>) -> u32,
    ) -> Result<RealizedBody, RealizationError> {
        self.body.realize_with_regions(self.frame, policy, region)
    }
}

impl CurvePathSource for Sweep {
    fn curve_path(&self) -> Option<CurvePathView<'_>> {
        Some(self.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use exedra_curves::profile::{Loop, Segment};
    use exedra_math::path::{CurvePathView, PathClosure, PathJoin, PathSegment3};

    fn square(half: f64) -> Loop {
        Loop::new(vec![
            Segment::line([half, -half]),
            Segment::line([half, half]),
            Segment::line([-half, half]),
            Segment::line([-half, -half]),
        ])
        .unwrap()
    }
    pub(super) fn profile() -> Profile {
        Profile::new(square(10.0), vec![], &ProfilePolicy::new(1e-9)).unwrap()
    }
    pub(super) fn view(segments: &[PathSegment3]) -> CurvePathView<'_> {
        CurvePathView {
            start: [0.0; 3],
            segments,
            section_x: [1.0, 0.0, 0.0],
            section_origin: [0.0; 2],
            closure: PathClosure::Open,
            joins: PathJoin::Smooth,
        }
    }
    pub(super) fn rail() -> [PathSegment3; 3] {
        [
            PathSegment3::Arc {
                axis_origin: [-40.0, 0.0, 0.0],
                axis: [0.0, 0.0, 1.0],
                sweep: core::f64::consts::PI,
            },
            PathSegment3::Arc {
                axis_origin: [-120.0, 0.0, 0.0],
                axis: [0.0, 0.0, 1.0],
                sweep: -core::f64::consts::FRAC_PI_2,
            },
            PathSegment3::Line {
                to: [-160.0, -40.0, 0.0],
            },
        ]
    }
    fn assert_realizes(sweep: &Sweep) {
        assert!(sweep.body().is_closed());
        let output = sweep
            .realize(&RealizationPolicy {
                tolerance: 0.02,
                ..RealizationPolicy::default()
            })
            .unwrap();
        assert!(output.report.within_tolerance(), "{:?}", output.report);
        assert!(output.mesh.validate_deep().is_empty());
        assert!(output.mesh.boundary_loops().unwrap().is_empty());
        let mesh_volume: f64 = output
            .mesh
            .faces()
            .map(|face| {
                let points: Vec<_> = output
                    .mesh
                    .face_loop(face)
                    .map(|corner| {
                        output
                            .mesh
                            .vertex_position(output.mesh.from_vertex(corner).unwrap())
                            .unwrap()
                            .map(f64::from)
                    })
                    .collect();
                assert_eq!(points.len(), 3);
                exedra_math::dot(points[0], exedra_math::cross(points[1], points[2])) / 6.0
            })
            .sum();
        // Independent integration of emitted oriented triangles catches an
        // inside-out or missing wall even when edge incidence remains closed.
        assert!(
            (mesh_volume - sweep.volume()).abs() < sweep.volume() * 0.005,
            "emitted {mesh_volume}, native {}",
            sweep.volume()
        );
        let detached = sweep.body().clone();
        let repeat = detached
            .realize(
                sweep.frame(),
                &RealizationPolicy {
                    tolerance: 0.02,
                    ..RealizationPolicy::default()
                },
            )
            .unwrap();
        assert_eq!(output.mesh.faces().count(), repeat.mesh.faces().count());
    }
    #[test]
    fn straight_sweep_matches_native_extrusion_measures_and_topology() {
        let segments = [PathSegment3::Line {
            to: [0.0, 20.0, 0.0],
        }];
        let sweep = Sweep::new(profile(), &view(&segments), Placement3::IDENTITY).unwrap();
        assert_eq!(sweep.body().vertices().len(), 8);
        assert_eq!(sweep.body().edges().len(), 12);
        assert_eq!(sweep.body().faces().len(), 6);
        for (end, expected) in [(End::Start, -1.0), (End::End, 1.0)] {
            let face = sweep.cap(end).unwrap();
            let plane = sweep.body().planar_frame(face, [0.0; 2]).unwrap();
            assert_eq!(plane.rows[1][2], expected);
        }
        assert_eq!(sweep.volume(), 8000.0);
        assert_eq!(sweep.surface_area(), 2400.0);
        assert_realizes(&sweep);
    }
    #[test]
    fn native_rail_shares_section_edges_and_keeps_exact_measures() {
        let rail = rail();
        let sweep = Sweep::new(profile(), &view(&rail), Placement3::IDENTITY).unwrap();
        assert_eq!(sweep.body().vertices().len(), 16);
        assert_eq!(sweep.body().edges().len(), 28);
        assert_eq!(sweep.body().faces().len(), 14);
        let length = 60.0 * core::f64::consts::PI + 40.0;
        assert!((sweep.volume() - 400.0 * length).abs() < 1e-9);
        assert!((sweep.surface_area() - (800.0 + 80.0 * length)).abs() < 1e-9);
        assert!(
            sweep
                .body()
                .faces()
                .iter()
                .any(|f| matches!(f.surface, crate::geometry::Surface::Cylinder(_)))
        );
        assert_realizes(&sweep);
    }
    #[test]
    fn holes_and_non_axial_section_edges_use_shared_native_geometry() {
        let profile = Profile::new(
            square(10.0),
            vec![square(3.0).reversed()],
            &ProfilePolicy::new(1e-9),
        )
        .unwrap();
        let rail = rail();
        let sweep = Sweep::new(profile, &view(&rail), Placement3::IDENTITY).unwrap();
        assert!((sweep.volume() - 364.0 * sweep.path_length()).abs() < 1e-9);
        assert_realizes(&sweep);
        let triangle = Profile::new(
            Loop::new(vec![
                Segment::line([5.0, -5.0]),
                Segment::line([0.0, 5.0]),
                Segment::line([-5.0, -5.0]),
            ])
            .unwrap(),
            vec![],
            &ProfilePolicy::new(1e-9),
        )
        .unwrap();
        let sweep = Sweep::new(triangle, &view(&rail[..1]), Placement3::IDENTITY).unwrap();
        assert!(
            sweep
                .body()
                .faces()
                .iter()
                .any(|f| matches!(f.surface, crate::geometry::Surface::Cone(_)))
        );
        assert_realizes(&sweep);
    }
    #[test]
    fn unsupported_corners_and_axis_contact_name_authored_inputs() {
        let corner = [
            PathSegment3::Line {
                to: [0.0, 20.0, 0.0],
            },
            PathSegment3::Line {
                to: [20.0, 20.0, 0.0],
            },
        ];
        assert!(matches!(
            Sweep::new(profile(), &view(&corner), Placement3::IDENTITY),
            Err(SweepError::Corner { segment: 1 })
        ));
        let tight = [PathSegment3::Arc {
            axis_origin: [-5.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            sweep: 1.0,
        }];
        assert!(matches!(
            Sweep::new(profile(), &view(&tight), Placement3::IDENTITY),
            Err(SweepError::AxisContact {
                path_segment: 0,
                ..
            })
        ));
    }

    #[test]
    fn offset_section_datum_preserves_volume_tags_and_reusable_path() {
        let tag = SegTag(23);
        let profile = Profile::new(
            Loop::new(vec![
                Segment::line([4.0, 0.0]).tagged(tag),
                Segment::line([4.0, 6.0]).tagged(tag),
                Segment::line([0.0, 6.0]).tagged(tag),
                Segment::line([0.0, 0.0]).tagged(tag),
            ])
            .unwrap(),
            vec![],
            &ProfilePolicy::new(1e-9),
        )
        .unwrap();
        let spans = rail();
        let mut path = view(&spans[..1]);
        path.section_origin = [1.0, 2.0];
        let sweep = Sweep::new(profile.clone(), &path, Placement3::IDENTITY).unwrap();
        let expected = 24.0 * 41.0 * core::f64::consts::PI;
        assert!((sweep.volume() - expected).abs() < 1e-10);
        assert_eq!(sweep.faces_tagged(tag).count(), 4);
        let reused = Sweep::new(profile, &sweep, Placement3::IDENTITY).unwrap();
        assert_eq!(sweep, reused);
        assert_realizes(&sweep);
    }

    #[test]
    fn spatial_bends_transport_the_section_without_a_world_plane() {
        let quarter = core::f64::consts::FRAC_PI_2;
        let spans = [
            PathSegment3::Arc {
                axis_origin: [-40.0, 0.0, 0.0],
                axis: [0.0, 0.0, 2.0],
                sweep: quarter,
            },
            PathSegment3::Arc {
                axis_origin: [-40.0, 40.0, 40.0],
                axis: [0.0, 3.0, 0.0],
                sweep: quarter,
            },
        ];
        let path = view(&spans);
        let sweep = Sweep::new(profile(), &path, Placement3::IDENTITY).unwrap();
        assert!((sweep.volume() - 400.0 * 80.0 * quarter).abs() < 1e-8);
        assert_realizes(&sweep);
        let budget = RealizationPolicy {
            max_vertices: 5,
            ..RealizationPolicy::default()
        };
        assert!(matches!(
            sweep.realize(&budget),
            Err(RealizationError::Budget {
                storage: "vertices",
                ..
            })
        ));
        let invalid = CurvePathView {
            joins: PathJoin::Miter { limit: f64::NAN },
            ..path
        };
        assert!(matches!(
            Sweep::new(profile(), &invalid, Placement3::IDENTITY),
            Err(SweepError::Join)
        ));
    }

    #[test]
    fn rounded_sections_share_toroidal_bends_cylindrical_runs_and_hole_boundaries() {
        let disk = |r: f64| {
            Loop::new(vec![
                Segment::arc([-r, 0.0], 1.0),
                Segment::arc([r, 0.0], 1.0),
            ])
            .unwrap()
        };
        let path = rail();
        for holes in [vec![], vec![disk(3.0).reversed()]] {
            let profile = Profile::new(disk(10.0), holes, &ProfilePolicy::new(1e-9)).unwrap();
            let area = profile.area();
            let sweep = Sweep::new(profile, &view(&path), Placement3::IDENTITY).unwrap();
            assert!((sweep.volume() - area * sweep.path_length()).abs() < 1e-8);
            assert!(
                sweep
                    .body()
                    .faces()
                    .iter()
                    .any(|f| matches!(f.surface, crate::geometry::Surface::Torus(_)))
            );
            assert_realizes(&sweep);
            let mapped = sweep
                .realize_with_uvs(
                    &RealizationPolicy {
                        tolerance: 0.05,
                        ..RealizationPolicy::default()
                    },
                    |_, _| 0,
                    |_, p| -> Result<_, core::convert::Infallible> {
                        Ok(p.map(|p| {
                            let n = exedra_math::narrow([p[0], p[1], 0.0]);
                            [n[0], n[1]]
                        }))
                    },
                )
                .unwrap();
            assert!(mapped.report.within_tolerance());
        }
        let p = Profile::new(
            Loop::new(vec![
                Segment::arc([0.0, -10.0], 1.0),
                Segment::arc([0.0, 10.0], 1.0),
            ])
            .unwrap(),
            vec![],
            &ProfilePolicy::new(1e-9),
        )
        .unwrap();
        let tight = [PathSegment3::Arc {
            axis_origin: [-5.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            sweep: 1.0,
        }];
        assert!(
            matches!(Sweep::new(p,&view(&tight),Placement3::IDENTITY),Err(SweepError::AxisContact {radius,..}) if radius < 0.0)
        );
    }
}
