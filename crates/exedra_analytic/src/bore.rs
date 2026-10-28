// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact-form through bores in bounded rectangular plates.
//! This operation adds native ellipse trims and a cylindrical wall; it performs
//! no mesh Boolean, healing, general solid intersection or sidewall clipping.

use crate::{
    End,
    body::{
        Body, BodyError, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, FaceId, LoopId,
        Sense, ValidationPolicy, VertexId,
    },
    extrusion::{EdgeRole as SourceEdgeRole, Extrusion, FaceRole as SourceFaceRole},
    geometry::{
        Curve, GeometryError, ParameterDomain, ParameterRange, Surface,
        intersection::{SectionError, SectionPolicy, plane_cylinder},
    },
};
use alloc::vec::Vec;
use exedra_math::{Placement3, sub};
use exedra_profile::SegTag;

/// An unoriented cylinder axis and positive radius in plate-local coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Bore {
    point: [f64; 3],
    direction: [f64; 3],
    radius: f64,
    tag: Option<SegTag>,
}
impl Bore {
    /// Checks finite inputs and normalizes the nonzero direction without overflow.
    /// Direction is canonicalized toward nonnegative local Z; an axis is unoriented.
    /// # Errors
    /// Returns [`BoreError::Input`] for invalid point, direction or radius.
    pub fn new(point: [f64; 3], direction: [f64; 3], radius: f64) -> Result<Self, BoreError> {
        if !point
            .into_iter()
            .chain(direction)
            .chain([radius])
            .all(f64::is_finite)
            || radius <= 0.0
        {
            return Err(BoreError::Input);
        }
        let magnitude = direction.into_iter().map(f64::abs).fold(0.0, f64::max);
        if magnitude == 0.0 {
            return Err(BoreError::Input);
        }
        let d = direction.map(|v| v / magnitude);
        let length = libm::hypot(libm::hypot(d[0], d[1]), d[2]);
        let mut direction = d.map(|v| v / length);
        if direction[2] < 0.0 {
            direction = direction.map(|v| -v);
        }
        Ok(Self {
            point,
            direction,
            radius,
            tag: None,
        })
    }
    /// Labels both native rims, the seam and the generated cylinder wall.
    #[must_use]
    pub const fn tagged(mut self, tag: SegTag) -> Self {
        self.tag = Some(tag);
        self
    }
    /// Original point on the authored axis, in plate-local coordinates.
    #[must_use]
    pub const fn point(&self) -> [f64; 3] {
        self.point
    }
    /// Canonical normalized axis direction, in plate-local coordinates.
    #[must_use]
    pub const fn direction(&self) -> [f64; 3] {
        self.direction
    }
    /// Positive radius in model units.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
    /// Caller feature tag, if supplied.
    #[must_use]
    pub const fn tag(&self) -> Option<SegTag> {
        self.tag
    }
}

/// Independent manufacturing separation and native numeric agreement policy.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BorePolicy {
    /// Minimum material distance from either complete rim to a plate sidewall.
    pub separation: f64,
    /// Maximum native section/support/trim disagreement, in model units.
    pub geometry_gap: f64,
    /// Minimum absolute cosine between cylinder axis and plate normal.
    pub min_axis_dot: f64,
}

/// Native source/provenance of a resulting face.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FaceRole {
    /// Original plate support and role; source IDs belong to [`BoredPlate::source`].
    Plate {
        /// Face in the original extrusion.
        source: FaceId,
        /// Original typed construction role.
        role: SourceFaceRole,
        /// The cap's material domain acquired an ellipse inner loop.
        trimmed: bool,
    },
    /// Generated inner cylinder wall.
    BoreWall {
        /// Authored feature tag.
        tag: Option<SegTag>,
    },
}
/// Native source/provenance of a resulting edge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EdgeRole {
    /// Untouched original bounded edge.
    Plate {
        /// Edge in the original extrusion.
        source: EdgeId,
        /// Original typed construction role.
        role: SourceEdgeRole,
    },
    /// Exact plane/cylinder section at one plate end.
    Rim {
        /// Plate end.
        end: End,
        /// Authored feature tag.
        tag: Option<SegTag>,
    },
    /// Shared periodic cylinder seam; not an extra material boundary.
    Seam {
        /// Authored feature tag.
        tag: Option<SegTag>,
    },
}

/// Operation evidence in the resulting body's local ID scope.
#[derive(Clone, Debug, PartialEq)]
pub struct BoreReport {
    /// Original start/end cap faces, now with inner ellipse loops.
    pub trimmed_faces: [FaceId; 2],
    /// Start/end section edges.
    pub rims: [EdgeId; 2],
    /// Generated cylinder wall.
    pub wall: FaceId,
    /// Shared periodic seam.
    pub seam: EdgeId,
    /// Conservative minimum sidewall clearance after numeric allowance.
    pub min_clearance: f64,
    /// Maximum section, support or full-band agreement evidence.
    pub max_gap: f64,
    /// Absolute cosine between bore axis and plate normal.
    pub axis_dot: f64,
}

/// Independent plate result retaining its original source and exact native trims.
/// Newly installed results always have a new selection scope in the compiler.
#[derive(Clone, Debug, PartialEq)]
pub struct BoredPlate {
    source: Extrusion,
    bore: Bore,
    body: Body<FaceRole, EdgeRole>,
    report: BoreReport,
    volume: f64,
    area: f64,
}
impl BoredPlate {
    /// Original immutable plate; IDs in inherited roles refer to this source.
    #[must_use]
    pub const fn source(&self) -> &Extrusion {
        &self.source
    }
    /// Authored local cylinder axis/radius/tag.
    #[must_use]
    pub const fn bore(&self) -> &Bore {
        &self.bore
    }
    /// Native result boundary, with typed inherited/generated roles.
    #[must_use]
    pub const fn body(&self) -> &Body<FaceRole, EdgeRole> {
        &self.body
    }
    /// Complete source occurrence placement.
    #[must_use]
    pub fn frame(&self) -> Placement3 {
        self.source.frame()
    }
    /// Native operation and agreement evidence.
    #[must_use]
    pub const fn report(&self) -> &BoreReport {
        &self.report
    }
    /// Finite positive native enclosed volume, independent of tessellation.
    #[must_use]
    pub const fn volume(&self) -> f64 {
        self.volume
    }
    /// Finite positive native boundary area, independent of tessellation.
    #[must_use]
    pub const fn surface_area(&self) -> f64 {
        self.area
    }
    /// Source tag or generated bore tag on a checked result face.
    #[must_use]
    pub fn face_tag(&self, id: FaceId) -> Option<SegTag> {
        match self.body.face(id)?.role {
            FaceRole::Plate { source, .. } => self.source.face_tag(source),
            FaceRole::BoreWall { tag } => tag,
        }
    }
    /// Result faces carrying `tag`, without conflating IDs with the source plate.
    pub fn faces_tagged(
        &self,
        tag: SegTag,
    ) -> impl Iterator<Item = (FaceId, &Face<FaceRole>)> + '_ {
        self.body
            .faces()
            .iter()
            .enumerate()
            .filter_map(move |(i, face)| {
                let id = FaceId(crate::usize_to_u32(i));
                (self.face_tag(id) == Some(tag)).then_some((id, face))
            })
    }
}

/// An unsupported/invalid bore is refused before a result replaces its source.
#[derive(Clone, Debug, PartialEq)]
pub enum BoreError {
    /// Axis point/direction/radius must be finite, direction nonzero, radius positive.
    Input,
    /// Separation positive finite, gap nonnegative finite, cosine threshold in `(0,1]`.
    InvalidPolicy,
    /// Only plain axis-aligned local rectangular profiles without holes/pockets qualify.
    UnsupportedPlate,
    /// Complete native section could not be established.
    Section(SectionError),
    /// Derived geometry violates a checked primitive invariant.
    Geometry(GeometryError),
    /// Complete rim contacts or approaches a sidewall closer than separation.
    Sidewall {
        /// Plate end whose full ellipse fails containment.
        end: End,
        /// Conservative clearance after numeric allowance.
        clearance: f64,
        /// Required remaining material in model units.
        required: f64,
    },
    /// A required native coordinate, domain or positive measure is not representable.
    NumericLimit,
    /// Native boundary support/incidence failed validation.
    Boundary(BodyError),
    /// Complete cylindrical trim-band relationship could not be certified.
    Trim(crate::body::CylindricalBandError),
}
impl core::fmt::Display for BoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "through bore: {self:?}")
    }
}
impl core::error::Error for BoreError {}

impl Extrusion {
    /// Cuts one fully contained exact circular cylinder through a plain rectangular plate.
    ///
    /// Axis inputs are plate-local; the source occurrence placement is retained.
    /// Both complete ellipse rims must leave the requested separation from all
    /// sidewalls. Convexity then establishes containment throughout the slab.
    /// No sidewall clipping, pre-existing openings/pockets or general Boolean is attempted.
    /// The borrowed source is unchanged on success and failure.
    ///
    /// ```
    /// use exedra_analytic::{bore::{Bore,BorePolicy},extrusion::Extrusion};
    /// use exedra_math::Placement3;
    /// use exedra_profile::{Loop,Profile,ProfilePolicy,Segment};
    /// let outline=Loop::new(vec![Segment::line([20.0,-15.0]),Segment::line([20.0,15.0]),
    ///     Segment::line([-20.0,15.0]),Segment::line([-20.0,-15.0])])?;
    /// let profile=Profile::new(outline,vec![],&ProfilePolicy::new(1e-9))?;
    /// let plate=Extrusion::new(profile,Placement3::IDENTITY,10.0)?;
    /// let bored=plate.with_through_bore(Bore::new([0.0;3],[0.5,0.0,1.0],2.0)?,
    ///     &BorePolicy{separation:1e-6,geometry_gap:1e-9,min_axis_dot:0.05})?;
    /// assert!(bored.body().is_closed());
    /// assert_eq!(bored.body().faces().len(),7);
    /// # Ok::<(),Box<dyn core::error::Error>>(())
    /// ```
    /// # Errors
    /// Returns precise input, unsupported-base, near-parallel, sidewall, numeric,
    /// native-boundary or trim-agreement refusals.
    pub fn with_through_bore(
        &self,
        bore: Bore,
        policy: &BorePolicy,
    ) -> Result<BoredPlate, BoreError> {
        if !policy.separation.is_finite()
            || policy.separation <= 0.0
            || !policy.geometry_gap.is_finite()
            || policy.geometry_gap < 0.0
            || !policy.min_axis_dot.is_finite()
            || policy.min_axis_dot <= 0.0
            || policy.min_axis_dot > 1.0
        {
            return Err(BoreError::InvalidPolicy);
        }
        let bounds = self.profile().bounds();
        let rectangle = self.profile().outer().segments();
        if !self.profile().holes().is_empty()
            || !self.pockets().is_empty()
            || rectangle.len() != 4
            || rectangle.iter().any(|s| {
                !matches!(s.curve, exedra_profile::Curve::Line)
                    || !(s.to[0] == bounds[0][0] || s.to[0] == bounds[1][0])
                    || !(s.to[1] == bounds[0][1] || s.to[1] == bounds[1][1])
            })
        {
            return Err(BoreError::UnsupportedPlate);
        }
        let direction = bore.direction;
        if direction[2] < policy.min_axis_dot {
            return Err(BoreError::Section(SectionError::NearParallel {
                axis_dot: direction[2],
                minimum: policy.min_axis_dot,
            }));
        }
        let radial = libm::hypot(direction[0], direction[1]);
        let x = if radial == 0.0 {
            [1.0, 0.0, 0.0]
        } else {
            [
                direction[2] * direction[0] / radial,
                direction[2] * direction[1] / radial,
                -radial,
            ]
        };
        let y = if radial == 0.0 {
            [0.0, 1.0, 0.0]
        } else {
            [-direction[1] / radial, direction[0] / radial, 0.0]
        };
        let surface = Surface::cylinder(
            Placement3::from_axes(x, y, direction, bore.point),
            bore.radius,
        )
        .map_err(BoreError::Geometry)?;
        let Surface::Cylinder(cylinder) = &surface else {
            unreachable!()
        };
        let section_policy = SectionPolicy {
            min_axis_dot: policy.min_axis_dot,
            max_gap: policy.geometry_gap,
        };
        let mut sections = Vec::with_capacity(2);
        let mut clearance = f64::INFINITY;
        let mut max_gap = 0.0_f64;
        for (end, z) in [(End::Start, 0.0), (End::End, self.length())] {
            let Surface::Plane(plane) =
                Surface::plane(Placement3::translate(0.0, 0.0, z)).map_err(BoreError::Geometry)?
            else {
                unreachable!()
            };
            let section =
                plane_cylinder(&plane, cylinder, &section_policy).map_err(BoreError::Section)?;
            let ellipse = section.ellipse();
            let frame = ellipse.frame();
            let mut minimum = f64::INFINITY;
            for (axis, (&min, &max)) in bounds[0].iter().zip(&bounds[1]).enumerate() {
                let extent = libm::hypot(
                    ellipse.semi_major() * frame.rows[axis][0],
                    ellipse.semi_minor() * frame.rows[axis][1],
                )
                .next_up();
                minimum = minimum
                    .min((frame.rows[axis][3] - extent) - min)
                    .min(max - (frame.rows[axis][3] + extent));
            }
            minimum = (minimum - section.max_gap()).next_down();
            if !minimum.is_finite() || minimum < policy.separation {
                return Err(BoreError::Sidewall {
                    end,
                    clearance: minimum,
                    required: policy.separation,
                });
            }
            clearance = clearance.min(minimum);
            max_gap = max_gap.max(section.max_gap());
            sections.push(section);
        }
        let source = self.body().clone().into_parts();
        let mut parts = BodyParts {
            trims: Vec::new(),
            vertices: source.vertices,
            loops: source.loops,
            edges: source
                .edges
                .into_iter()
                .enumerate()
                .map(|(i, e)| Edge {
                    geometry: e.geometry,
                    start: e.start,
                    end: e.end,
                    role: EdgeRole::Plate {
                        source: EdgeId(crate::usize_to_u32(i)),
                        role: e.role,
                    },
                })
                .collect(),
            faces: source
                .faces
                .into_iter()
                .enumerate()
                .map(|(i, f)| Face {
                    surface: f.surface,
                    domain: f.domain,
                    sense: f.sense,
                    loops: f.loops,
                    role: FaceRole::Plate {
                        source: FaceId(crate::usize_to_u32(i)),
                        trimmed: matches!(f.role, SourceFaceRole::Cap(_)),
                        role: f.role,
                    },
                })
                .collect(),
        };
        let range =
            ParameterRange::new(0.0, core::f64::consts::TAU).map_err(BoreError::Geometry)?;
        let rims = core::array::from_fn(|i| EdgeId(crate::usize_to_u32(parts.edges.len() + i)));
        let vertices = core::array::from_fn::<_, 2, _>(|i| {
            VertexId(crate::usize_to_u32(parts.vertices.len() + i))
        });
        for (i, end) in [End::Start, End::End].into_iter().enumerate() {
            let curve = sections[i].curve().clone();
            let point = curve.evaluate(0.0).ok_or(BoreError::NumericLimit)?.point;
            parts.vertices.push(point);
            parts.edges.push(Edge {
                geometry: crate::body::EdgeGeometry::Curve { curve, range },
                start: vertices[i],
                end: vertices[i],
                role: EdgeRole::Rim { end, tag: bore.tag },
            });
            let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
            parts.loops.push(BoundaryLoop {
                coedges: alloc::vec![Coedge {
                    edge: rims[i],
                    reversed: end == End::End
                }],
            });
            parts.faces[self.cap(end).index()].loops.push(loop_id);
        }
        let seam = EdgeId(crate::usize_to_u32(parts.edges.len()));
        let a = parts.vertices[vertices[0].index()];
        let b = parts.vertices[vertices[1].index()];
        parts.edges.push(Edge {
            geometry: crate::body::EdgeGeometry::Curve {
                curve: Curve::line(a, sub(b, a)).map_err(BoreError::Geometry)?,
                range: ParameterRange::UNIT,
            },
            start: vertices[0],
            end: vertices[1],
            role: EdgeRole::Seam { tag: bore.tag },
        });
        let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
        parts.loops.push(BoundaryLoop {
            coedges: alloc::vec![
                Coedge {
                    edge: rims[0],
                    reversed: true
                },
                Coedge {
                    edge: seam,
                    reversed: false
                },
                Coedge {
                    edge: rims[1],
                    reversed: false
                },
                Coedge {
                    edge: seam,
                    reversed: true
                }
            ],
        });
        let padding = 4.0 * max_gap;
        let domain = ParameterDomain {
            u: range,
            v: ParameterRange::new(
                (sections[0].axial().bounds()[0] - padding).next_down(),
                (sections[1].axial().bounds()[1] + padding).next_up(),
            )
            .map_err(|_| BoreError::NumericLimit)?,
        };
        let wall = FaceId(crate::usize_to_u32(parts.faces.len()));
        parts.faces.push(Face {
            surface,
            domain,
            sense: Sense::Reverse,
            loops: alloc::vec![loop_id],
            role: FaceRole::BoreWall { tag: bore.tag },
        });
        let validation = ValidationPolicy {
            max_gap: policy.geometry_gap,
        };
        let body = Body { parts };
        let checked = body.validate(&validation).map_err(BoreError::Boundary)?;
        let band = body
            .cylindrical_band(wall, &validation)
            .map_err(BoreError::Trim)?;
        max_gap = max_gap.max(band.max_gap()).max(checked.max_gap);
        let section_area = core::f64::consts::PI * bore.radius * bore.radius / direction[2];
        let volume = self.volume() - section_area * self.length();
        let area = self.surface_area() - 2.0 * section_area
            + core::f64::consts::TAU * bore.radius * self.length() / direction[2];
        if !volume.is_finite() || volume <= 0.0 || !area.is_finite() || area <= 0.0 {
            return Err(BoreError::NumericLimit);
        }
        let report = BoreReport {
            trimmed_faces: [self.cap(End::Start), self.cap(End::End)],
            rims,
            wall,
            seam,
            min_clearance: clearance,
            max_gap,
            axis_dot: direction[2],
        };
        Ok(BoredPlate {
            source: self.clone(),
            bore,
            body,
            report,
            volume,
            area,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
    fn plate(center: f64, frame: Placement3) -> Extrusion {
        let outline = Loop::new(alloc::vec![
            Segment::line([center + 20.0, -15.0]),
            Segment::line([center + 20.0, 15.0]),
            Segment::line([center - 20.0, 15.0]),
            Segment::line([center - 20.0, -15.0])
        ])
        .unwrap();
        let profile = Profile::new(outline, alloc::vec![], &ProfilePolicy::new(1e-9)).unwrap();
        Extrusion::new(profile, frame, 10.0).unwrap()
    }
    fn policy() -> BorePolicy {
        BorePolicy {
            separation: 1e-6,
            geometry_gap: 1e-9,
            min_axis_dot: 0.05,
        }
    }
    #[test]
    fn exact_bore_keeps_source_and_reusable_native_geometry() {
        let source = plate(0.0, Placement3::IDENTITY);
        let original = source.clone();
        let result = source
            .with_through_bore(
                Bore::new([0.0; 3], [0.5, 0.25, 1.0], 2.0)
                    .unwrap()
                    .tagged(SegTag(9)),
                &policy(),
            )
            .unwrap();
        assert_eq!(source, original);
        assert!(result.body().is_closed());
        assert_eq!(result.body().faces().len(), 7);
        assert_eq!(result.body().edges().len(), 15);
        assert_eq!(result.body().vertices().len(), 10);
        let cosine = result.bore().direction()[2];
        let removed = core::f64::consts::PI * 4.0 * 10.0 / cosine;
        assert!((result.volume() - (12000.0 - removed)).abs() < 1e-10);
        assert!(
            (result.surface_area()
                - (3800.0 - 2.0 * core::f64::consts::PI * 4.0 / cosine
                    + core::f64::consts::TAU * 2.0 * 10.0 / cosine))
                .abs()
                < 1e-10
        );
        for edge in result.report().rims {
            assert!(matches!(
                result.body().edge(edge).unwrap().regular().unwrap().0,
                Curve::Ellipse(_)
            ));
        }
        assert_eq!(result.faces_tagged(SegTag(9)).count(), 1);
        let band = result
            .body()
            .cylindrical_band(
                result.report().wall,
                &ValidationPolicy {
                    max_gap: policy().geometry_gap,
                },
            )
            .unwrap();
        assert!(band.contains_parameter([
            0.0,
            (band.boundaries()[0].coefficients()[0] + band.boundaries()[1].coefficients()[0]) * 0.5
        ]));
        let display = crate::realize::RealizationPolicy {
            tolerance: 0.02,
            geometry_gap: 1e-9,
            ..Default::default()
        };
        let realized = result.body().realize(result.frame(), &display).unwrap();
        assert!(realized.mesh.validate_deep().is_empty());
        assert!(realized.mesh.boundary_loops().unwrap().is_empty());
        assert!(realized.report.within_tolerance(), "{:?}", realized.report);
    }
    #[test]
    fn axis_contact_policy_and_far_numeric_refusals_are_transactional() {
        let source = plate(0.0, Placement3::IDENTITY);
        let original = source.clone();
        assert!(matches!(
            source.with_through_bore(
                Bore::new([19.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
                &policy()
            ),
            Err(BoreError::Sidewall { .. })
        ));
        assert!(matches!(
            source.with_through_bore(
                Bore::new([0.0; 3], [1.0, 0.0, 1e-8], 2.0).unwrap(),
                &policy()
            ),
            Err(BoreError::Section(SectionError::NearParallel { .. }))
        ));
        assert!(matches!(
            source.with_through_bore(
                Bore::new([1e16, 0.0, 0.0], [0.1, 0.0, 1.0], 2.0).unwrap(),
                &policy()
            ),
            Err(BoreError::Section(SectionError::SupportGap { .. }))
        ));
        assert_eq!(source, original);
        assert!(Bore::new([0.0; 3], [0.0; 3], 1.0).is_err());
        assert!(Bore::new([0.0; 3], [0.0, 0.0, 1.0], f64::NAN).is_err());
    }
    #[test]
    fn checked_band_refuses_loose_body_seam_gaps_and_wrong_rim_pairing() {
        let result = plate(0.0, Placement3::IDENTITY)
            .with_through_bore(
                Bore::new([0.0; 3], [0.25, 0.0, 1.0], 2.0).unwrap(),
                &policy(),
            )
            .unwrap();
        let validation = ValidationPolicy { max_gap: 1e-9 };
        let wall = result.report().wall;
        let seam = result.report().seam;
        let mut parts = result.body().clone().into_parts();
        let Curve::Line(line) = parts.edges[seam.index()].regular().unwrap().0.clone() else {
            unreachable!()
        };
        let shifted = exedra_math::add(line.origin(), [0.25, 0.0, 0.0]);
        *parts.edges[seam.index()].regular_mut().unwrap().0 =
            Curve::line(shifted, line.direction()).unwrap();
        let loose = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 5.0 }).unwrap();
        assert!(
            matches!(loose.cylindrical_band(wall,&validation),Err(crate::body::CylindricalBandError::Agreement{bound,..}) if bound>=0.25)
        );
        let mut parts = result.body().clone().into_parts();
        let upper = result.report().rims[1];
        let vertex = parts.edges[upper.index()].start;
        *parts.edges[upper.index()].regular_mut().unwrap().1 =
            ParameterRange::new(core::f64::consts::PI, 3.0 * core::f64::consts::PI).unwrap();
        parts.vertices[vertex.index()] = parts.edges[upper.index()]
            .regular()
            .unwrap()
            .0
            .evaluate(core::f64::consts::PI)
            .unwrap()
            .point;
        let edge = &parts.edges[seam.index()];
        let a = parts.vertices[edge.start.index()];
        let b = parts.vertices[edge.end.index()];
        *parts.edges[seam.index()].regular_mut().unwrap().0 = Curve::line(a, sub(b, a)).unwrap();
        let loose = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 5.0 }).unwrap();
        assert!(
            matches!(loose.cylindrical_band(wall,&validation),Err(crate::body::CylindricalBandError::Agreement{bound,..}) if bound>1.0)
        );
    }

    #[test]
    fn bore_scale_axis_reversal_and_placement_metamorphic_must_succeed() {
        for size in [1e-6, 1.0, 1e6] {
            for degrees in [0.0_f64, 30.0, 60.0] {
                let angle = degrees.to_radians();
                let outline = Loop::new(alloc::vec![
                    Segment::line([35.0 * size, -25.0 * size]),
                    Segment::line([35.0 * size, 25.0 * size]),
                    Segment::line([-35.0 * size, 25.0 * size]),
                    Segment::line([-35.0 * size, -25.0 * size])
                ])
                .unwrap();
                let profile =
                    Profile::new(outline, alloc::vec![], &ProfilePolicy::new(1e-8 * size)).unwrap();
                let policy = BorePolicy {
                    separation: 0.1 * size,
                    geometry_gap: 1e-8 * size,
                    min_axis_dot: 0.1,
                };
                let point = [-10.0 * size * angle.tan(), 0.0, 0.0];
                let direction = [angle.sin(), 0.0, angle.cos()];
                let reference_profile = Loop::new(alloc::vec![
                    Segment::line([35.0, -25.0]),
                    Segment::line([35.0, 25.0]),
                    Segment::line([-35.0, 25.0]),
                    Segment::line([-35.0, -25.0])
                ])
                .unwrap();
                let reference = Extrusion::new(
                    Profile::new(reference_profile, alloc::vec![], &ProfilePolicy::new(1e-8))
                        .unwrap(),
                    Placement3::IDENTITY,
                    20.0,
                )
                .unwrap()
                .with_through_bore(
                    Bore::new([-10.0 * angle.tan(), 0.0, 0.0], direction, 6.0).unwrap(),
                    &BorePolicy {
                        separation: 0.1,
                        geometry_gap: 1e-8,
                        min_axis_dot: 0.1,
                    },
                )
                .unwrap();
                for frame in [
                    Placement3::IDENTITY,
                    Placement3::rotate_y_then_translate(
                        0.35,
                        100.0 * size,
                        -70.0 * size,
                        30.0 * size,
                    ),
                ] {
                    let source = Extrusion::new(profile.clone(), frame, 20.0 * size).unwrap();
                    for sign in [-1.0, 1.0] {
                        let result = source
                            .with_through_bore(
                                Bore::new(point, direction.map(|v| sign * v), 6.0 * size).unwrap(),
                                &policy,
                            )
                            .unwrap();
                        assert!(
                            (result.volume() / (size * size * size) / reference.volume() - 1.0)
                                .abs()
                                < 1e-10,
                            "volume scales cubically: scale={size} angle={degrees}"
                        );
                        assert!(
                            (result.surface_area() / (size * size) / reference.surface_area()
                                - 1.0)
                                .abs()
                                < 1e-10,
                            "area scales quadratically: scale={size} angle={degrees}"
                        );
                        let display = crate::realize::RealizationPolicy {
                            tolerance: 0.025 * size,
                            geometry_gap: 1e-8 * size,
                            ..Default::default()
                        };
                        let mesh = result.body().realize(frame, &display).unwrap();
                        assert!(
                            mesh.report.within_tolerance(),
                            "scale={size} angle={degrees} sign={sign}: {:?}",
                            mesh.report
                        );
                        assert!(mesh.mesh.validate_deep().is_empty());
                        assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
                    }
                }
            }
        }
    }

    #[test]
    fn far_local_translation_and_occurrence_precision_are_visible() {
        let source = plate(1e6, Placement3::IDENTITY);
        let large_gap = BorePolicy {
            geometry_gap: 1e-6,
            ..policy()
        };
        let result = source
            .with_through_bore(
                Bore::new([1e6, 0.0, 0.0], [0.25, 0.0, 1.0], 2.0).unwrap(),
                &large_gap,
            )
            .unwrap();
        assert!(result.report().max_gap > 1e-9 && result.report().max_gap <= 1e-6);
        let source = plate(0.0, Placement3::translate(1e6 + 0.03, 0.0, 0.0));
        let result = source
            .with_through_bore(
                Bore::new([0.0; 3], [0.25, 0.0, 1.0], 2.0).unwrap(),
                &policy(),
            )
            .unwrap();
        let display = crate::realize::RealizationPolicy {
            tolerance: 0.02,
            geometry_gap: 1e-9,
            ..Default::default()
        };
        let realized = result.body().realize(result.frame(), &display).unwrap();
        assert!(!realized.report.within_tolerance(), "{:?}", realized.report);
        assert!(realized.report.max_vertex_displacement > 0.02);
    }
}
