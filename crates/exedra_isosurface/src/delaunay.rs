// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Surface meshing by restricted Delaunay refinement.
//!
//! [`delaunay_mesh`] meshes the zero set of an
//! [`exedra_implicit::ScalarField`] with triangles that are near-isotropic,
//! close to the surface, sharp along its creases, and closed into a
//! manifold around every vertex. It needs only what every field answers:
//! values, gradients, interval bounds, and the creases a formula's `min`,
//! `max` and `abs` mark.
//!
//! # How it works
//!
//! 1. **Seeds.** A grid of signs over the bounds gives points on the
//!    surface where cell edges cross it, points on creases near crossing
//!    cells, and points on features thinner than a cell, found by probing
//!    cells the field's interval and gradient bounds cannot prove empty.
//!    Blocks of cells whose value bounds exclude zero are skipped whole.
//! 2. **Creases.** Each crease is traced from its seeds, with steps as long
//!    as the crease stays within the distance tolerance and turns by less
//!    than the turning limit. Corners where creases meet are placed where
//!    the field and two ties are zero. Following Cheng, Dey and Levine's
//!    `DelPSC`, every crease sample becomes a weighted vertex protected by a
//!    ball that overlaps its neighbors, with spacing graded so neighboring
//!    balls differ in size by at most half.
//! 3. **Refinement.** In the regular tetrahedralization of the crease
//!    samples and seeds, each tetrahedron takes the field's sign at its
//!    orthocenter; the faces between inside and outside tetrahedra are the
//!    restricted Delaunay surface. A face too large, too badly shaped, too
//!    far from the surface or folded is refined by inserting where its dual
//!    edge meets the surface, and each vertex's faces must close into one
//!    disk around it. A face whose refinement point falls in a protecting
//!    ball shows that ball is too large, as does a crease segment the
//!    surface cuts across instead of following: that stretch of crease is
//!    split and the pass starts again.
//!
//! Every decision about the tetrahedralization is exact, so identical
//! inputs give identical meshes.

mod crease;
mod refine;
mod sample;
mod seed;
#[cfg(test)]
mod tests;

use alloc::vec::Vec;

use hashbrown::HashSet;

use exedra_implicit::{Aabb, ScalarField};
use exedra_mesh::{BuildError, FaceBuildAttrs, Mesh, MeshBuilder};

use crease::Creases;
use sample::Sampler;
use seed::Grid;

/// What [`delaunay_mesh`] meshes and how finely.
///
/// [`DelaunayParams::new`] derives every length from the cell size; the
/// fields can then be adjusted.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DelaunayParams {
    /// The region meshed. The surface should lie inside it: where it
    /// crosses the boundary, the mesh may be open or incomplete there, and
    /// the report says so.
    pub bounds: Aabb,
    /// The side of the cubes the signs are sampled on. Features thinner
    /// than this are found only where probing reaches them.
    pub cell_size: f64,
    /// The largest radius a face's surface Delaunay ball may have.
    pub max_size: f64,
    /// The smallest face radius refined for size, shape, distance or
    /// folding; below it only the disk condition refines, so tangencies and
    /// other degenerate spots cannot run away.
    pub min_size: f64,
    /// The largest ratio of a face's radius to its shortest edge. One
    /// bounds every angle below by 30 degrees.
    pub max_radius_edge_ratio: f64,
    /// How far a face's surface point may lie from its circumcenter: the
    /// surface's distance tolerance.
    pub max_distance: f64,
    /// The largest angle, in radians, between the field's normals at a
    /// face's corners.
    pub max_normal_angle: f64,
    /// How creases are traced and protected.
    pub crease: CreaseParams,
    /// How many halvings of a cell are probed for thin features; zero
    /// probes none.
    pub thin_feature_depth: u32,
    /// The most vertices a pass may hold before it stops refining.
    pub max_points: usize,
    /// The most passes, each after splitting balls found too large.
    pub max_passes: usize,
    /// Whether to label each triangle with the region of the
    /// [`Tagged`](exedra_implicit::Tagged) part it lies on, at one crease
    /// query per triangle.
    pub regions: bool,
}

/// How creases are traced and protected.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CreaseParams {
    /// The longest step between crease samples.
    pub max_step: f64,
    /// The shortest step between crease samples.
    pub min_step: f64,
    /// The most a crease's direction, or the normals of the pieces meeting
    /// there, may turn between neighboring samples, in radians.
    pub max_turn: f64,
    /// A protecting ball's radius as a fraction of the longer step beside
    /// it. Below one half, neighboring balls do not overlap; at one, they
    /// reach each other's centers.
    pub protect: f64,
}

impl DelaunayParams {
    /// Parameters for `bounds` sampled on cubes of side `cell_size`, with
    /// faces up to six cells across, within a fiftieth of a cell of the
    /// surface, and no angle below 30 degrees.
    #[must_use]
    pub fn new(bounds: Aabb, cell_size: f64) -> Self {
        let h = cell_size;
        Self {
            bounds,
            cell_size,
            max_size: 6.0 * h,
            min_size: 0.1 * h,
            max_radius_edge_ratio: 1.0,
            max_distance: 0.02 * h,
            max_normal_angle: 120.0_f64.to_radians(),
            crease: CreaseParams {
                max_step: 3.0 * h,
                min_step: 0.25 * h,
                max_turn: 15.0_f64.to_radians(),
                protect: 0.6,
            },
            thin_feature_depth: 3,
            max_points: 1 << 22,
            max_passes: 8,
            regions: false,
        }
    }
}

/// A meshed surface.
#[derive(Clone, Debug)]
pub struct DelaunayResult {
    /// Vertex positions.
    pub positions: Vec<[f64; 3]>,
    /// Triangles over `positions`, wound counterclockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
    /// Each triangle's region, when [`DelaunayParams::regions`] asks for
    /// them; otherwise empty.
    pub regions: Vec<u32>,
    /// Edges along the field's creases, each a pair of indices into
    /// `positions`: where the surface turns sharply.
    pub creases: Vec<[u32; 2]>,
    /// What the meshing did and how far it got.
    pub report: DelaunayReport,
}

impl DelaunayResult {
    /// The triangles as an Exedra mesh, with crease edges marked sharp and
    /// regions as face regions when there are any.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] when the triangles do not form a valid mesh,
    /// as when the report shows the disk condition left unmet.
    pub fn mesh(&self) -> Result<Mesh, BuildError> {
        let mut builder = MeshBuilder::new();
        for p in &self.positions {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "meshes store f32 positions"
            )]
            builder.push_vertex(p.map(|c| c as f32));
        }
        let creases: HashSet<[u32; 2]> = self
            .creases
            .iter()
            .map(|&[a, b]| [a.min(b), a.max(b)])
            .collect();
        for (index, triangle) in self.triangles.iter().enumerate() {
            let sharpness: [f32; 3] = core::array::from_fn(|k| {
                let (a, b) = (triangle[k], triangle[(k + 1) % 3]);
                if creases.contains(&[a.min(b), a.max(b)]) {
                    1.0
                } else {
                    0.0
                }
            });
            let attrs = FaceBuildAttrs {
                region: self.regions.get(index).copied(),
                edge_sharpness: sharpness.contains(&1.0).then_some(&sharpness[..]),
                ..FaceBuildAttrs::default()
            };
            builder.add_face_with_attrs(triangle, &attrs)?;
        }
        Ok(builder.build()?.mesh)
    }
}

/// What [`delaunay_mesh`] did and how far it got.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DelaunayReport {
    /// Passes run, each after splitting balls found too large.
    pub passes: usize,
    /// Crease segments split between passes.
    pub ball_splits: usize,
    /// Crease lines traced.
    pub crease_lines: usize,
    /// Crease samples, corners included, in the final pass.
    pub crease_samples: usize,
    /// Corners where creases meet or end.
    pub corners: usize,
    /// Seeds found on the surface: cell-edge crossings and thin features.
    pub seeds: usize,
    /// Blocks of cells tested, and those proven to hold no surface.
    pub blocks: usize,
    /// Blocks proven to hold no surface.
    pub empty_blocks: usize,
    /// Points inserted by refinement in the final pass, by reason.
    pub refinements: Refinements,
    /// Tetrahedra in the final pass.
    pub tetrahedra: usize,
    /// Vertices whose faces still do not close into one disk.
    pub disk_failures: usize,
    /// Crease segments, between neighboring samples or from a corner, that
    /// are not edges of the mesh, so the surface cuts across the crease
    /// there, though its protecting balls could still split.
    pub broken_creases: usize,
    /// Crease segments the surface cuts across where the protecting balls
    /// are already at [`CreaseParams::min_step`]: detail finer than the
    /// mesh resolves, as where a crease fades out or creases crowd a
    /// corner. A smaller minimum step resolves more.
    pub unresolved_creases: usize,
    /// Points found on the surface that lie on no face: a piece of surface
    /// small enough to fit inside one vertex's cell, which the mesh lost.
    pub isolated_vertices: usize,
    /// Protecting balls refinement needed to enter that could not split
    /// further.
    pub pending_balls: usize,
    /// Whether a pass stopped at [`DelaunayParams::max_points`].
    pub hit_point_limit: bool,
    /// Whether tracing creases stopped at its budget of samples, so some
    /// creases may not be followed.
    pub hit_crease_limit: bool,
    /// Whether the surface crosses the boundary of the bounds, beyond
    /// which nothing is sampled, so the mesh may be open or incomplete
    /// there.
    pub clipped: bool,
    /// Field queries made.
    pub work: FieldWork,
}

impl DelaunayReport {
    /// Whether refinement met every criterion: no limit was hit, no ball
    /// was left too large, every vertex's faces form a disk, every crease
    /// is followed by edges down to the minimum step, every point found on
    /// the surface lies on a face, and the surface lies inside the bounds.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.hit_point_limit
            && !self.hit_crease_limit
            && self.pending_balls == 0
            && self.disk_failures == 0
            && self.broken_creases == 0
            && self.isolated_vertices == 0
            && !self.clipped
    }
}

/// Points inserted by refinement, by the criterion that asked for them.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Refinements {
    /// Faces with a corner off the surface.
    pub off_surface: usize,
    /// Faces too large.
    pub size: usize,
    /// Faces too badly shaped.
    pub shape: usize,
    /// Faces too far from the surface.
    pub distance: usize,
    /// Faces whose corners' normals disagree.
    pub folded: usize,
    /// Faces refined so a vertex's faces close into a disk.
    pub disk: usize,
    /// Queued faces gone by the time they were reached.
    pub stale: usize,
    /// Queued faces skipped because a vertex already lay near their point.
    pub crowded: usize,
}

/// Field queries made.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldWork {
    /// Values at points.
    pub values: u64,
    /// Values with gradients at points.
    pub gradients: u64,
    /// Values with the creases nearest a point.
    pub creases: u64,
    /// Bounds over boxes, of values or of values and gradients.
    pub intervals: u64,
}

/// Why [`delaunay_mesh`] refused its parameters.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DelaunayError {
    /// A parameter is not finite, not positive where it must be, or out of
    /// its range.
    InvalidParameter {
        /// The parameter's name.
        name: &'static str,
    },
    /// The grid over the bounds would have more cells along an axis than
    /// meshing supports.
    TooManyCells,
}

impl core::fmt::Display for DelaunayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidParameter { name } => write!(f, "meshing parameter `{name}` is invalid"),
            Self::TooManyCells => f.write_str("the grid over the bounds has too many cells"),
        }
    }
}

impl core::error::Error for DelaunayError {}

/// The most cells along one axis of the sign grid.
const MAX_CELLS: f64 = 65_536.0;

/// Every length and limit, in absolute terms.
#[derive(Copy, Clone, Debug)]
struct Settings {
    cell: f64,
    /// The bounds grown by two cells: where the mesh can reach, and where
    /// creases are followed.
    reach: Aabb,
    /// How closely searches bracket a crossing.
    tolerance: f64,
    /// How small a value counts as on the surface for a searched point.
    on_search: f64,
    /// How small a value counts as on the surface for a crease point.
    on_surface: f64,
    seed_spacing: f64,
    max_size: f64,
    min_size: f64,
    max_ratio: f64,
    max_distance: f64,
    normal_limit: f64,
    max_step: f64,
    min_step: f64,
    max_turn: f64,
    protect: f64,
    thin_depth: u32,
    max_points: usize,
    max_crease_samples: usize,
    max_disk_rounds: usize,
}

impl Settings {
    fn new(params: &DelaunayParams) -> Result<Self, DelaunayError> {
        let positive = |name: &'static str, value: f64| {
            if value.is_finite() && value > 0.0 {
                Ok(value)
            } else {
                Err(DelaunayError::InvalidParameter { name })
            }
        };
        let h = positive("cell_size", params.cell_size)?;
        if params.bounds.extent().iter().any(|e| e / h > MAX_CELLS) {
            return Err(DelaunayError::TooManyCells);
        }
        let angle = positive("max_normal_angle", params.max_normal_angle)?;
        let protect = positive("crease.protect", params.crease.protect)?;
        let min_step = positive("crease.min_step", params.crease.min_step)?;
        let max_step = positive("crease.max_step", params.crease.max_step)?;
        if max_step < min_step {
            return Err(DelaunayError::InvalidParameter {
                name: "crease.max_step",
            });
        }
        if params.max_points == 0 {
            return Err(DelaunayError::InvalidParameter { name: "max_points" });
        }
        if params.max_passes == 0 {
            return Err(DelaunayError::InvalidParameter { name: "max_passes" });
        }
        let tolerance = h / 256.0;
        Ok(Self {
            cell: h,
            tolerance,
            on_search: 4.0 * tolerance,
            on_surface: 1.0e-5 * h,
            seed_spacing: 2.0 * h,
            max_size: positive("max_size", params.max_size)?,
            min_size: positive("min_size", params.min_size)?,
            max_ratio: positive("max_radius_edge_ratio", params.max_radius_edge_ratio)?,
            max_distance: positive("max_distance", params.max_distance)?,
            normal_limit: exedra_math::Real::sin_cos(angle).1,
            max_step,
            min_step,
            max_turn: positive("crease.max_turn", params.crease.max_turn)?,
            protect,
            thin_depth: params.thin_feature_depth,
            max_points: params.max_points,
            max_crease_samples: 1 << 16,
            max_disk_rounds: 20,
            reach: params
                .bounds
                .grown(2.0 * h)
                .ok_or(DelaunayError::InvalidParameter { name: "bounds" })?,
        })
    }
}

/// Meshes the zero set of `field` within `params.bounds`.
///
/// The result is always the best mesh reached; its report says whether
/// every criterion was met, and why not when one was not.
///
/// # Errors
///
/// Returns [`DelaunayError`] when a parameter is invalid.
pub fn delaunay_mesh(
    field: &dyn ScalarField,
    params: &DelaunayParams,
) -> Result<DelaunayResult, DelaunayError> {
    let settings = Settings::new(params)?;
    let sampler = Sampler::new(field);
    let grid = Grid::covering(&params.bounds, settings.cell);

    // A box two cells outside the grid holds everything.
    let (start, end) = (grid.origin, grid.end());
    let margin = 2.0 * settings.cell;
    let hull: [[f64; 2]; 3] = core::array::from_fn(|k| [start[k] - margin, end[k] + margin]);
    // Far from the origin, a margin can vanish in rounding or overflow.
    if hull
        .iter()
        .any(|&[lo, hi]| !(lo.is_finite() && hi.is_finite() && lo < hi))
    {
        return Err(DelaunayError::InvalidParameter { name: "bounds" });
    }

    let seeds = seed::seed(&sampler, &settings, &grid);
    let mut creases = Creases::trace(&sampler, &seeds.creases, &settings);
    let mut surface_seeds = seeds.thin.clone();
    surface_seeds.extend_from_slice(&seeds.edges);

    let mut report = DelaunayReport {
        crease_lines: creases.lines.len(),
        corners: creases.corners.len(),
        seeds: surface_seeds.len(),
        blocks: seeds.blocks,
        empty_blocks: seeds.empty_blocks,
        clipped: seeds.clipped,
        hit_crease_limit: creases.hit_sample_limit,
        ..DelaunayReport::default()
    };
    let mut last = false;
    loop {
        report.passes += 1;
        last |= report.passes >= params.max_passes;
        let pass = refine::refine(&sampler, &settings, hull, &creases, &surface_seeds, last);
        if last || pass.requests.is_empty() || pass.hit_point_limit {
            report.crease_samples = creases.samples();
            report.refinements = pass.refinements;
            report.tetrahedra = pass.tetrahedra;
            report.disk_failures = pass.disk_failures;
            report.broken_creases = pass.broken_creases;
            report.unresolved_creases = pass.unresolved_creases;
            report.isolated_vertices = pass.isolated_vertices;
            // The samples can miss where the surface crosses the bounds;
            // the mesh, which extends past them, cannot.
            report.clipped |= pass.positions.iter().any(|&p| !params.bounds.contains(p));
            report.pending_balls = pass.requests.len();
            report.hit_point_limit = pass.hit_point_limit;
            let regions = if params.regions {
                regions(&sampler, &pass.positions, &pass.triangles)
            } else {
                Vec::new()
            };
            report.work = sampler.work();
            return Ok(DelaunayResult {
                positions: pass.positions,
                triangles: pass.triangles,
                regions,
                creases: pass.creases,
                report,
            });
        }
        let splits = creases.split(&sampler, &pass.requests, &settings);
        // Balls that would not split: refine once more, to the end.
        last = splits == 0;
        report.ball_splits += splits;
    }
}

/// Each triangle's region: that of the part shaping the field at its
/// centroid, which lies on the triangle's side of any crease.
fn regions(sampler: &Sampler<'_>, positions: &[[f64; 3]], triangles: &[[u32; 3]]) -> Vec<u32> {
    triangles
        .iter()
        .map(|t| {
            let p = t.map(|i| positions[i as usize]);
            let centroid = core::array::from_fn(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0);
            sampler.crease(centroid).region
        })
        .collect()
}
