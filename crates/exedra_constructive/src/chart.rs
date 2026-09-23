// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opt-in construction coordinates for extrusion, revolution, loft and sweep surfaces.
//!
//! Charts are authored on generating nodes, in their local recipe units. They
//! are carried by mesh corner UVs, so topology can stay shared across a texture
//! seam. Placements move geometry without changing these rest coordinates.
//! Profile distance means cumulative length of the sampled profile chords,
//! not exact analytic arc length; the original sampling policy is retained.
//! Realization refuses collapsed chart edges or triangle winding changes after
//! f32 storage. Construction coordinates must fit the triangulator predicate
//! envelope (`1e100`); stored UVs must be finite.
//!
//! ```
//! use exedra_constructive::{builders, chart::{ChartTransform, SurfaceChart},
//!     ir::{CapMode, NodeKind, Placement3, RecipeBuilder}};
//! let mut builder = RecipeBuilder::new();
//! let profile = builder.add_profile(builders::rect_from_corner(2.0, 1.0)?);
//! let root = builder.with_surface_chart(SurfaceChart::Extrude {
//!     wall: ChartTransform::IDENTITY, caps: ChartTransform::IDENTITY,
//! }).add(NodeKind::Extrude {
//!     profile, placement: Placement3::IDENTITY, height: 3.0, caps: CapMode::Both,
//! })?;
//! let recipe = builder.finish(root)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::discretize::{DiscretizePolicy, DiscretizedProfile};
use alloc::vec::Vec;
use exedra_triangulate::predicates::{MAX_COORDINATE, Orientation, orient2d};

/// Maps two construction coordinates to dimensionless texture coordinates.
///
/// `uv = matrix * coordinates + offset`. Matrix entries have units of texture
/// repeats per recipe unit. Negative entries reverse direction; exchanging rows
/// exchanges U/V. Offsets shift texture phase, not the geometric chart seam.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartTransform {
    /// Finite, nonsingular row-major linear mapping.
    pub matrix: [[f64; 2]; 2],
    /// Finite texture phase in repeats.
    pub offset: [f64; 2],
}
impl ChartTransform {
    /// One texture repeat per recipe unit, with unmodified axes and phase.
    pub const IDENTITY: Self = Self {
        matrix: [[1.0, 0.0], [0.0, 1.0]],
        offset: [0.0; 2],
    };

    /// Checks finiteness and invertibility without imposing a metric or handedness.
    pub fn validate(&self) -> Result<(), ChartError> {
        if self
            .matrix
            .iter()
            .flatten()
            .chain(&self.offset)
            .any(|x| !x.is_finite())
        {
            return Err(ChartError::InvalidTransform);
        }
        let [a, b] = self.matrix.map(|row| {
            let m = row[0].abs().max(row[1].abs());
            row.map(|v| v / m)
        });
        let det = a[0] * b[1] - a[1] * b[0];
        if !det.is_finite() || det == 0.0 {
            return Err(ChartError::InvalidTransform);
        }
        Ok(())
    }
    pub(crate) fn map_face(&self, points: &[[f64; 2]]) -> Result<Vec<[f32; 2]>, ChartError> {
        // Exact-sign predicates share the triangulator's declared coordinate
        // envelope. Refuse larger chart coordinates rather than extrapolating it.
        if points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
        {
            return Err(ChartError::NumericLimit);
        }
        let uv = points
            .iter()
            .map(|&p| self.map(p))
            .collect::<Result<Vec<_>, _>>()?;
        let [a, b] = self.matrix.map(|row| {
            let m = row[0].abs().max(row[1].abs());
            row.map(|v| v / m)
        });
        let reflected = orient2d([0.0; 2], a, b) == Orientation::Cw;
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            if points[i] != points[j] && uv[i] == uv[j] {
                return Err(ChartError::NumericLimit);
            }
        }
        for i in 1..points.len() - 1 {
            let raw = orient2d(points[0], points[i], points[i + 1]);
            if raw == Orientation::Collinear {
                continue;
            }
            let expected = match (raw, reflected) {
                (Orientation::Ccw, true) => Orientation::Cw,
                (Orientation::Cw, true) => Orientation::Ccw,
                _ => raw,
            };
            let stored = orient2d(
                uv[0].map(f64::from),
                uv[i].map(f64::from),
                uv[i + 1].map(f64::from),
            );
            if expected != stored {
                return Err(ChartError::NumericLimit);
            }
        }
        // Face extraction may rotate/reverse the corner loop or choose another
        // diagonal. Preserve weak convexity as well as this fan's triangles;
        // a single valid fan can otherwise hide a flipped alternative diagonal.
        let raw_winding = (1..points.len() - 1)
            .map(|i| orient2d(points[0], points[i], points[i + 1]))
            .find(|&o| o != Orientation::Collinear)
            .ok_or(ChartError::NumericLimit)?;
        let winding = match (raw_winding, reflected) {
            (Orientation::Ccw, true) => Orientation::Cw,
            (Orientation::Cw, true) => Orientation::Ccw,
            _ => raw_winding,
        };
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            let k = (i + 2) % points.len();
            let raw = orient2d(points[i], points[j], points[k]);
            let stored = orient2d(
                uv[i].map(f64::from),
                uv[j].map(f64::from),
                uv[k].map(f64::from),
            );
            if (raw != Orientation::Collinear && (raw != raw_winding || stored != winding))
                || (stored != Orientation::Collinear && stored != winding)
            {
                return Err(ChartError::NumericLimit);
            }
        }
        Ok(uv)
    }

    pub(crate) fn map(&self, point: [f64; 2]) -> Result<[f32; 2], ChartError> {
        let uv = core::array::from_fn(|i| {
            self.matrix[i][0] * point[0] + self.matrix[i][1] * point[1] + self.offset[i]
        });
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the checked f64-to-f32 chart boundary"
        )]
        let uv = uv.map(|x: f64| x as f32);
        if uv.iter().any(|x| !x.is_finite()) {
            return Err(ChartError::NumericLimit);
        }
        Ok(uv)
    }
}
impl Default for ChartTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Construction metric and texture mapping for one generating node.
///
/// Use [`crate::ir::RecipeBuilder::with_surface_chart`]. Other node kinds reject
/// charts until they define an explicit rest metric. All loops have independent
/// distance origins at their authored starts; use [`crate::profile::Loop2::with_seam`]
/// to choose these seams. Caps have independent planar charts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceChart {
    /// Wall coordinates are `(sampled profile distance, extrusion distance)`.
    Extrude {
        /// Wall mapping, shared by outer and hole loops in their authored winding.
        wall: ChartTransform,
        /// Cap mapping from local profile `(x, y)`, on both caps.
        caps: ChartTransform,
    },
    /// Wall coordinates are `(reference_radius * theta, sampled profile distance)`.
    ///
    /// Theta grows by the right-handed revolution convention. Its seam is the
    /// authored theta-zero meridian, local +X; placement controls its pose.
    /// The angular metric is exact only at the reference radius. Other radii,
    /// including collapsed pole fans, intentionally stretch this rest chart.
    Revolve {
        /// Finite positive rest radius in profile units.
        reference_radius: f64,
        /// Wall mapping in angular-distance/profile-distance order.
        wall: ChartTransform,
        /// Cap mapping from local profile `(radius, height)`, on both caps.
        caps: ChartTransform,
    },
    /// Loft walls use `(reference profile distance, authored longitudinal distance)`.
    ///
    /// Profile distance is measured along the jointly sampled reference section,
    /// in its own profile coordinates before placement. Corresponding points keep
    /// this U coordinate throughout the loft. V runs from zero to `rest_length`,
    /// placing each authored section by `stations`; smooth intermediate samples
    /// interpolate between their bounding sections. Differing sections and
    /// draping deliberately stretch this rest chart, as does uneven station
    /// spacing under [`LoftStations::SectionIndex`].
    Loft {
        /// Explicit zero-based section whose sampled perimeter supplies U.
        reference_section: u32,
        /// Finite positive total rest length in recipe units, not measured span.
        rest_length: f64,
        /// How authored sections divide `rest_length`.
        stations: LoftStations,
        /// Wall mapping in reference-profile/longitudinal order.
        wall: ChartTransform,
        /// Mapping from each cap's own local profile `(x, y)` coordinates.
        caps: ChartTransform,
    },
    /// Sweep walls use `(sampled profile distance, sampled path distance)`.
    ///
    /// Path distance is cumulative centerline chord length before placement,
    /// independent of the section datum and transported frame. Inner and outer
    /// rails at bends intentionally stretch this rest metric; miter cuts do not
    /// reset it. Closed paths have their longitudinal seam at authored station
    /// zero. This does not change the path's frame policy or geometry checks.
    Sweep {
        /// Wall mapping in profile-distance/path-distance order.
        wall: ChartTransform,
        /// Mapping from local profile `(x, y)` coordinates on either cap.
        caps: ChartTransform,
    },
}
impl SurfaceChart {
    /// Checks chart scalars; the recipe builder also checks the node kind.
    pub fn validate(&self) -> Result<(), ChartError> {
        if let Self::Revolve {
            reference_radius, ..
        } = self
            && (!reference_radius.is_finite() || *reference_radius <= 0.0)
        {
            return Err(ChartError::InvalidReferenceRadius);
        }
        if let Self::Loft { rest_length, .. } = self
            && (!rest_length.is_finite() || *rest_length <= 0.0)
        {
            return Err(ChartError::InvalidRestLength);
        }
        self.wall().validate()?;
        self.caps().validate()
    }
    pub(crate) fn wall(self) -> ChartTransform {
        match self {
            Self::Extrude { wall, .. }
            | Self::Revolve { wall, .. }
            | Self::Loft { wall, .. }
            | Self::Sweep { wall, .. } => wall,
        }
    }
    pub(crate) fn caps(self) -> ChartTransform {
        match self {
            Self::Extrude { caps, .. }
            | Self::Revolve { caps, .. }
            | Self::Loft { caps, .. }
            | Self::Sweep { caps, .. } => caps,
        }
    }
}

/// How a loft chart places its authored sections along `rest_length`.
#[derive(Copy, Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum LoftStations {
    /// Uniformly by section index: section `i` of `n` sits at `i / (n - 1)`.
    #[default]
    SectionIndex,
    /// In proportion to the chord length between consecutive section datums,
    /// the profile origins after each section's placement.
    ///
    /// Measured on the placements the loft receives, so rigid and uniformly
    /// scaled occurrences keep these ratios (up to rounding), while
    /// nonuniform placements change them. Coincident consecutive datums give
    /// a zero-length band and are refused.
    DatumDistance,
}

/// Original chart metric and profile sampling, retained as surface ancestry.
///
/// This describes the generating operation, not the metric after nonuniform
/// placement or deformation. It does not assert current UV coverage: operations
/// may discard attributes. Check the mesh corner layer or compiled
/// `RegionRange::has_uvs`. In particular mesh Booleans currently discard UVs.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSampling {
    /// Authored construction metric and coordinate transform.
    pub chart: SurfaceChart,
    /// Original profile discretization policy.
    pub profile: DiscretizePolicy,
    /// Total sampled chord length of each loop, outer then holes.
    /// Loft lengths refer to the authored reference section after joint sampling.
    pub loop_lengths: Vec<f64>,
    /// Longitudinal rest coordinate at every emitted station for lofts/sweeps.
    ///
    /// Closed sweeps include the closing endpoint coordinate even though its
    /// geometry reuses station zero. Loft values use authored uniform section
    /// parameters; sweep values use pre-placement centerline chords. Empty for
    /// extrusion/revolution. These are original construction evidence, not
    /// distances measured on the subsequently transformed surface.
    pub station_distances: Vec<f64>,
}
impl ChartSampling {
    pub(crate) fn approx_bytes(&self) -> usize {
        size_of::<Self>()
            + (self.loop_lengths.len() + self.station_distances.len()) * size_of::<f64>()
    }
}

/// Typed inability to author or realize a construction chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChartError {
    /// A transform is nonfinite or singular.
    InvalidTransform,
    /// The angular rest radius is nonfinite or nonpositive.
    InvalidReferenceRadius,
    /// The authored longitudinal rest length is nonfinite or nonpositive.
    InvalidRestLength,
    /// The requested reference section does not exist in this loft.
    InvalidReferenceSection {
        /// Requested zero-based section.
        section: u32,
    },
    /// The generating operation does not match the requested chart metric.
    WrongOperation,
    /// [`LoftStations::DatumDistance`] found two consecutive sections with
    /// coincident datums.
    CoincidentLoftDatums {
        /// Zero-based band between sections `band` and `band + 1`.
        band: u32,
    },
    /// Coordinates exceeded the exact-predicate/f32 range, or rounding collapsed
    /// a distinct chart edge or changed a nondegenerate face triangle's winding.
    NumericLimit,
}
impl core::fmt::Display for ChartError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidTransform => "surface chart transform must be finite and nonsingular",
            Self::InvalidReferenceRadius => {
                "surface chart reference radius must be finite and positive"
            }
            Self::InvalidRestLength => "surface chart rest length must be finite and positive",
            Self::CoincidentLoftDatums { band } => {
                return write!(
                    f,
                    "loft sections {band} and {} have coincident datums",
                    band + 1
                );
            }
            Self::InvalidReferenceSection { section } => {
                return write!(
                    f,
                    "surface chart reference section {section} does not exist"
                );
            }
            Self::WrongOperation => "surface chart metric does not match the generating operation",
            Self::NumericLimit => "surface chart exceeds numeric representation limits",
        })
    }
}
impl core::error::Error for ChartError {}

/// Transient construction data, allocated only when charting is requested.
pub(crate) struct ChartBuilder {
    pub(crate) sampling: ChartSampling,
    pub(crate) distances: Vec<Vec<f64>>,
    pub(crate) vertex_profile: Vec<[f64; 2]>,
    pub(crate) faces: Vec<Vec<[f32; 2]>>,
}
impl ChartBuilder {
    pub(crate) fn set_stations(&mut self, distances: Vec<f64>) -> Result<(), ChartError> {
        if distances.iter().any(|v| !v.is_finite())
            || distances.windows(2).any(|pair| pair[1] <= pair[0])
        {
            return Err(ChartError::NumericLimit);
        }
        self.sampling.station_distances = distances;
        Ok(())
    }

    pub(crate) fn wall_quad(&self, ring: usize, point: usize, band: usize) -> [[f64; 2]; 4] {
        let s = &self.distances[ring];
        let v = &self.sampling.station_distances;
        [
            [s[point], v[band]],
            [s[point + 1], v[band]],
            [s[point + 1], v[band + 1]],
            [s[point], v[band + 1]],
        ]
    }

    pub(crate) fn new(
        chart: SurfaceChart,
        d: &DiscretizedProfile,
        profile: DiscretizePolicy,
    ) -> Result<Self, ChartError> {
        chart.validate()?;
        let mut distances = Vec::new();
        let mut loop_lengths = Vec::new();
        for ring in core::iter::once(&d.outer).chain(&d.holes) {
            let mut s = alloc::vec![0.0];
            for i in 0..ring.points.len() {
                let a = ring.points[i];
                let b = ring.points[(i + 1) % ring.points.len()];
                let next = s[i] + libm::hypot(b[0] - a[0], b[1] - a[1]);
                if !next.is_finite() || next <= s[i] {
                    return Err(ChartError::NumericLimit);
                }
                s.push(next);
            }
            loop_lengths.push(*s.last().expect("closed profile"));
            distances.push(s);
        }
        Ok(Self {
            sampling: ChartSampling {
                chart,
                profile,
                loop_lengths,
                station_distances: Vec::new(),
            },
            distances,
            vertex_profile: Vec::new(),
            faces: Vec::new(),
        })
    }
}

#[cfg(test)]
#[path = "chart_tests.rs"]
mod tests;
