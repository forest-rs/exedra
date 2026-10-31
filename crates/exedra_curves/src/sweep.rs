// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tubes and ribbons swept along framed stations.

use alloc::vec::Vec;

use exedra_math::{Real, add, cross, norm, scale, sub};

use crate::frames::perpendicular;
use crate::{Frame, Workspace};

/// Triangles swept along a curve's stations: a ring of
/// [`Self::ring`] vertices per station, vertex `station * ring + k`.
///
/// Triangles wind counter-clockwise seen from the side their vertex
/// normals face. No caps: an open tube's ends are left open.
///
/// Non-exhaustive, so it can carry more buffers later (texture
/// coordinates, say); a caller reuses one through [`Default`] and the
/// `_into` functions.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SweptMesh {
    /// Vertex positions.
    pub positions: Vec<[f64; 3]>,
    /// Unit vertex normals.
    pub normals: Vec<[f64; 3]>,
    /// Triangle vertex indices, three per triangle.
    pub indices: Vec<u32>,
    /// Vertices per station: a tube's sides, or 2 for a ribbon.
    pub ring: u32,
    /// What was swept.
    pub report: SweepReport,
}

impl SweptMesh {
    /// An empty mesh, as [`Default`] gives, in a `const` context.
    pub(crate) const fn new() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
            ring: 0,
            report: SweepReport {
                stations: 0,
                ring: 0,
                closed: false,
                vertices: 0,
                triangles: 0,
                collapsed: 0,
                unoriented: 0,
            },
        }
    }
}

/// The counts of a [`SweptMesh`], so a caller can check it without
/// reading its buffers.
///
/// Non-exhaustive, so it can report more later without breaking a caller
/// that reads its fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SweepReport {
    /// Stations swept.
    pub stations: u32,
    /// Vertices per station.
    pub ring: u32,
    /// Whether the last station joins the first.
    pub closed: bool,
    /// Vertices: `stations * ring`.
    pub vertices: u32,
    /// Triangles: `2 * (ring) * (stations - 1)` for an open tube, with one
    /// more band of `2 * ring` when closed; a ribbon is a tube of one band
    /// across (`2 * (stations - 1)`, plus 2 when closed).
    pub triangles: u32,
    /// Stations of zero radius or width: their ring collapses to the
    /// station's point, and is swept all the same.
    pub collapsed: u32,
    /// Ribbon stations whose normal was parallel to the tangent (within
    /// `2^-26` radians), zero or not finite: they take the frame's normal
    /// instead.
    pub unoriented: u32,
}

/// Why a sweep cannot be built.
///
/// Non-exhaustive: a later check may add a variant without breaking a
/// caller that reports the error through `Display` or a fallback arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SweepError {
    /// A tube needs at least three sides.
    Sides {
        /// The sides asked for.
        sides: u32,
    },
    /// The per-station values are not one per frame.
    Stations {
        /// The frames.
        frames: usize,
        /// The values given.
        values: usize,
    },
    /// A radius or width is negative or not finite. Zero is accepted, and
    /// reported as [`SweepReport::collapsed`]; a caller whose sizes come
    /// from interpolation that can undershoot (a Catmull-Rom chain of
    /// widths) chooses its own repair, such as clamping, before sweeping.
    Size {
        /// The first such station.
        station: usize,
    },
    /// More vertices than `u32` indices reach, or more triangles than a
    /// `u32` counts.
    TooLarge,
}

impl core::fmt::Display for SweepError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Sides { sides } => write!(f, "a tube needs 3 sides or more, not {sides}"),
            Self::Stations { frames, values } => {
                write!(f, "{values} station values for {frames} frames")
            }
            Self::Size { station } => {
                write!(f, "station {station}'s size is negative or not finite")
            }
            Self::TooLarge => f.write_str("more vertices or triangles than a u32 counts"),
        }
    }
}

impl core::error::Error for SweepError {}

/// A tube of `sides` sides around frames, of radius `radii[i]` at frame
/// `i`.
///
/// Ring vertex `k` of station `i` is at `point + radius * d` for the
/// radial direction `d = cos a * normal + sin a * binormal` and
/// `a = 2 pi k / sides`; the angles come from `exedra_math`'s `sin_cos`
/// (libm when enabled).
///
/// Its normal is the surface's, tilted from `d` against the tangent `T`
/// by the radius' slope along the centerline: `normalize(ds * d - dr * T)`
/// for the change in radius `dr` and the chord length `ds` from the
/// previous station to the next (one-sided at an open tube's ends,
/// wrapping when closed). It is exact for a frustum, a straight tube
/// whose radius varies linearly; coincident stations of different radii
/// face along `-T` or `T`, as the flat annulus between them does; and a
/// radius that does not change gives `d` itself. Along a bent centerline
/// the exact normal of a tube swept with rotation-minimizing frames is
/// `normalize((1 - r k) d - r' T)` for the curvature `k` toward `d`; the
/// `(1 - r k)` factor, which only rescales the radial part and is near 1
/// for a tube thin against its bend, is not modelled, so the normals on
/// the inside of a tight bend lean too little. The slope is a difference
/// across each station, so a radius whose slope changes at a station (a
/// crease, such as a cone meeting a cylinder) is shaded smooth there; a
/// caller that wants the crease sweeps the two sides separately.
///
/// Band `i` joins station `i` to `i + 1` (and the last to the first when
/// `closed`) with triangles `(i k, i k+1, i+1 k)` and
/// `(i k+1, i+1 k+1, i+1 k)`, outward-facing.
///
/// Radii are checked to be finite and nonnegative, because they are
/// authored or interpolated data where a negative or NaN value would
/// silently turn a ring inside out or erase it. Frames are not checked:
/// they are framing's output, and a non-finite frame point gives its ring
/// non-finite positions.
///
/// # Errors
///
/// Checked in this order, before anything is allocated:
/// [`SweepError::Sides`] below 3 sides, [`SweepError::Stations`] unless
/// there is one radius per frame, [`SweepError::Size`] for a negative or
/// non-finite radius, [`SweepError::TooLarge`] past `u32` vertices or
/// triangles.
pub fn tube(
    frames: &[Frame],
    radii: &[f64],
    sides: u32,
    closed: bool,
) -> Result<SweptMesh, SweepError> {
    let mut mesh = SweptMesh::default();
    tube_with(
        frames,
        radii,
        sides,
        closed,
        (&mut Vec::new(), &mut 0),
        &mut mesh,
    )?;
    Ok(mesh)
}

/// [`tube`] into a caller's mesh, whose buffers are cleared first, with
/// `workspace` as scratch. The ring's directions are kept in the
/// workspace and recomputed only when the side count changes, so a
/// retained caller sweeping at one side count calls no trigonometry after
/// its first tube.
///
/// # Errors
///
/// As [`tube`]; `out` is left empty.
pub fn tube_into(
    frames: &[Frame],
    radii: &[f64],
    sides: u32,
    closed: bool,
    workspace: &mut Workspace,
    out: &mut SweptMesh,
) -> Result<(), SweepError> {
    let ring = (&mut workspace.ring, &mut workspace.ring_sides);
    tube_with(frames, radii, sides, closed, ring, out)
}

/// [`tube_into`] with the ring's directions and their side count as
/// scratch.
fn tube_with(
    frames: &[Frame],
    radii: &[f64],
    sides: u32,
    closed: bool,
    (directions, ring_sides): (&mut Vec<(f64, f64)>, &mut u32),
    out: &mut SweptMesh,
) -> Result<(), SweepError> {
    clear(out);
    if sides < 3 {
        return Err(SweepError::Sides { sides });
    }
    check_count(frames.len(), radii.len())?;
    check_sizes(radii)?;
    let plan = plan(frames.len(), sides, closed, true)?;
    if *ring_sides != sides || directions.len() != sides as usize {
        directions.clear();
        directions.extend((0..sides).map(|k| {
            let angle = core::f64::consts::TAU * (f64::from(k) / f64::from(sides));
            let (sin, cos) = Real::sin_cos(angle);
            (cos, sin)
        }));
        *ring_sides = sides;
    }
    let directions = &*directions;
    let mesh = out;
    start(mesh, &plan, sides, closed);
    let last = frames.len().saturating_sub(1);
    for (i, (frame, &radius)) in frames.iter().zip(radii).enumerate() {
        if radius == 0.0 {
            mesh.report.collapsed += 1;
        }
        // The neighbouring stations, wrapping when closed and the station
        // itself at an open tube's ends.
        let previous = match (i, closed) {
            (0, true) => last,
            (0, false) => 0,
            _ => i - 1,
        };
        let next = match (i == last, closed) {
            (true, true) => 0,
            (true, false) => last,
            _ => i + 1,
        };
        let [radial, along] = slope_weights(
            radii[next] - radii[previous],
            distance(frames[previous].point, frame.point)
                + distance(frame.point, frames[next].point),
        );
        for &(cos, sin) in directions {
            let direction = add(scale(frame.normal, cos), scale(frame.binormal, sin));
            mesh.positions
                .push(add(frame.point, scale(direction, radius)));
            mesh.normals.push(if along == 0.0 {
                direction
            } else {
                sub(scale(direction, radial), scale(frame.tangent, along))
            });
        }
    }
    bands(mesh, &plan, sides);
    Ok(())
}

/// A flat ribbon along frames, `widths[i]` wide at frame `i`, facing
/// `normals[i]` (made perpendicular to the tangent), or the frame's normal
/// when `normals` is `None`.
///
/// Station `i` has vertices `point - w/2 * side` and `point + w/2 * side`
/// for `side = normalize(tangent x normal)`, both with the normal as
/// theirs, so the ribbon's front faces the normal. A normal parallel to
/// the tangent (within `2^-26` radians), zero or not finite takes the
/// frame's ([`SweepReport::unoriented`]). Widths are checked as a tube's
/// radii are, and frames are not.
///
/// # Errors
///
/// Checked in this order, before anything is allocated:
/// [`SweepError::Stations`] unless there is one width (and normal) per
/// frame, [`SweepError::Size`] for a negative or non-finite width,
/// [`SweepError::TooLarge`] past `u32` vertices or triangles.
pub fn ribbon(
    frames: &[Frame],
    normals: Option<&[[f64; 3]]>,
    widths: &[f64],
    closed: bool,
) -> Result<SweptMesh, SweepError> {
    let mut mesh = SweptMesh::default();
    ribbon_into(frames, normals, widths, closed, &mut mesh)?;
    Ok(mesh)
}

/// [`ribbon`] into a caller's mesh, whose buffers are cleared first.
///
/// # Errors
///
/// As [`ribbon`]; `out` is left empty.
pub fn ribbon_into(
    frames: &[Frame],
    normals: Option<&[[f64; 3]]>,
    widths: &[f64],
    closed: bool,
    out: &mut SweptMesh,
) -> Result<(), SweepError> {
    clear(out);
    check_count(frames.len(), widths.len())?;
    if let Some(normals) = normals {
        check_count(frames.len(), normals.len())?;
    }
    check_sizes(widths)?;
    let plan = plan(frames.len(), 2, closed, false)?;
    let mesh = out;
    start(mesh, &plan, 2, closed);
    for (i, (frame, &width)) in frames.iter().zip(widths).enumerate() {
        if width == 0.0 {
            mesh.report.collapsed += 1;
        }
        let authored = normals.map(|normals| normals[i]);
        let facing = authored.and_then(|n| perpendicular(n, frame.tangent));
        if authored.is_some() && facing.is_none() {
            mesh.report.unoriented += 1;
        }
        let facing = facing.unwrap_or(frame.normal);
        let side = cross(frame.tangent, facing);
        let half = scale(side, width * 0.5);
        mesh.positions.push(sub(frame.point, half));
        mesh.positions.push(add(frame.point, half));
        mesh.normals.push(facing);
        mesh.normals.push(facing);
    }
    bands(mesh, &plan, 2);
    Ok(())
}

/// The weights of the radial direction and of the tangent in a tube's
/// surface normal, `[ds, dr] / |[ds, dr]|`, for a change in radius `dr`
/// over the chord length `ds`; `[1, 0]` when the radius does not change.
fn slope_weights(dr: f64, ds: f64) -> [f64; 2] {
    if dr == 0.0 {
        return [1.0, 0.0];
    }
    // Divided by the larger first, so the squares cannot overflow; `dr` is
    // nonzero, so the larger is too.
    let largest = ds.max(dr.abs());
    let (ds, dr) = (ds / largest, dr / largest);
    let length = Real::sqrt(ds * ds + dr * dr);
    [ds / length, dr / length]
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    norm(sub(b, a))
}

/// One value per frame.
fn check_count(frames: usize, values: usize) -> Result<(), SweepError> {
    if values == frames {
        Ok(())
    } else {
        Err(SweepError::Stations { frames, values })
    }
}

/// Each size finite and nonnegative.
fn check_sizes(sizes: &[f64]) -> Result<(), SweepError> {
    match sizes
        .iter()
        .position(|size| !(size.is_finite() && *size >= 0.0))
    {
        Some(station) => Err(SweepError::Size { station }),
        None => Ok(()),
    }
}

/// A sweep's counts, checked before anything is allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plan {
    stations: u32,
    vertices: u32,
    /// Bands joining consecutive rings.
    bands: u32,
    /// Quads per band: the ring's vertices around a tube, one fewer across
    /// a ribbon.
    columns: u32,
    triangles: u32,
    indices: usize,
}

fn plan(stations: usize, ring: u32, closed: bool, around: bool) -> Result<Plan, SweepError> {
    let stations = u32::try_from(stations).map_err(|_| SweepError::TooLarge)?;
    let vertices = stations.checked_mul(ring).ok_or(SweepError::TooLarge)?;
    let bands = match (stations, closed) {
        (0 | 1, _) => 0,
        (s, false) => s - 1,
        (s, true) => s,
    };
    let columns = if around { ring } else { ring - 1 };
    let triangles = bands
        .checked_mul(columns)
        .and_then(|quads| quads.checked_mul(2))
        .ok_or(SweepError::TooLarge)?;
    let indices = usize::try_from(triangles)
        .ok()
        .and_then(|t| t.checked_mul(3))
        .ok_or(SweepError::TooLarge)?;
    Ok(Plan {
        stations,
        vertices,
        bands,
        columns,
        triangles,
        indices,
    })
}

/// Empties `mesh`, keeping its buffers.
fn clear(mesh: &mut SweptMesh) {
    mesh.positions.clear();
    mesh.normals.clear();
    mesh.indices.clear();
    mesh.ring = 0;
    mesh.report = SweepReport::default();
}

/// Reserves `mesh`'s buffers for the plan and fills in its counts.
fn start(mesh: &mut SweptMesh, plan: &Plan, ring: u32, closed: bool) {
    mesh.positions.reserve(plan.vertices as usize);
    mesh.normals.reserve(plan.vertices as usize);
    mesh.indices.reserve(plan.indices);
    mesh.ring = ring;
    mesh.report = SweepReport {
        stations: plan.stations,
        ring,
        closed,
        vertices: plan.vertices,
        triangles: plan.triangles,
        ..SweepReport::default()
    };
}

/// Joins consecutive rings: around (wrapping) for a tube, across for a
/// ribbon.
fn bands(mesh: &mut SweptMesh, plan: &Plan, ring: u32) {
    for band in 0..plan.bands {
        let a = band * ring;
        let b = ((band + 1) % plan.stations) * ring;
        for k in 0..plan.columns {
            let next = (k + 1) % ring;
            mesh.indices
                .extend_from_slice(&[a + k, a + next, b + k, a + next, b + next, b + k]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation_minimizing_frames;
    use exedra_math::dot;

    fn straight() -> Vec<Frame> {
        let points: Vec<[f64; 3]> = (0..3).map(|i| [0.0, 0.0, f64::from(i)]).collect();
        rotation_minimizing_frames(&points, false, Some([1.0, 0.0, 0.0])).0
    }

    #[test]
    fn a_tube_has_documented_counts_and_faces_out() {
        let frames = straight();
        let mesh = tube(&frames, &[0.5, 0.5, 0.0], 4, false).unwrap();
        assert_eq!(mesh.report.vertices, 12);
        assert_eq!(mesh.report.triangles, 2 * 4 * 2);
        assert_eq!(mesh.indices.len(), 48);
        assert_eq!(mesh.report.collapsed, 1);
        assert_eq!(mesh.positions[0], [0.5, 0.0, 0.0]);
        for triangle in mesh.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.positions[triangle[k] as usize]);
            let face = cross(sub(b, a), sub(c, a));
            let centroid = scale(add(add(a, b), c), 1.0 / 3.0);
            let outward = [centroid[0], centroid[1], 0.0];
            // Triangles reaching the collapsed ring may have no area.
            if norm(face) > 0.0 {
                assert!(dot(face, outward) > 0.0, "{triangle:?}");
            }
        }
        assert_eq!(
            tube(&frames, &[0.5], 4, false),
            Err(SweepError::Stations {
                frames: 3,
                values: 1
            })
        );
        assert_eq!(
            tube(&frames, &[0.5; 3], 2, false),
            Err(SweepError::Sides { sides: 2 })
        );
        let closed = tube(&frames, &[0.5; 3], 3, true).unwrap();
        assert_eq!(closed.report.triangles, 3 * 3 * 2);
    }

    #[test]
    fn a_ribbon_faces_its_normals() {
        let frames = straight();
        let normals = [[0.0, 1.0, 0.0]; 3];
        let mesh = ribbon(&frames, Some(&normals), &[1.0, 1.0, 1.0], false).unwrap();
        assert_eq!(mesh.report.vertices, 6);
        assert_eq!(mesh.report.triangles, 4);
        // side = tangent x normal = z x y = -x.
        assert_eq!(mesh.positions[0], [0.5, 0.0, 0.0]);
        assert_eq!(mesh.positions[1], [-0.5, 0.0, 0.0]);
        for triangle in mesh.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.positions[triangle[k] as usize]);
            assert!(dot(cross(sub(b, a), sub(c, a)), [0.0, 1.0, 0.0]) > 0.0);
        }
        let parallel = [[0.0, 0.0, 1.0]; 3];
        let fallback = ribbon(&frames, Some(&parallel), &[1.0; 3], false).unwrap();
        assert_eq!(fallback.report.unoriented, 3);
    }

    #[test]
    fn a_ribbon_normal_along_a_diagonal_tangent_is_unoriented() {
        // The projection of a normal parallel to the tangent leaves only
        // rounding: the ribbon keeps its width on the frame's normal.
        let points = [[0.0; 3], [1.0; 3]];
        let frames = rotation_minimizing_frames(&points, false, None).0;
        for normal in [[1.0; 3], [-1.0; 3], [1.0, 1.0, 1.0 + 1e-12]] {
            let mesh = ribbon(&frames, Some(&[normal; 2]), &[1.0; 2], false).unwrap();
            assert_eq!(mesh.report.unoriented, 2, "{normal:?}");
            for pair in mesh.positions.chunks(2) {
                let width = norm(sub(pair[1], pair[0]));
                assert!((width - 1.0).abs() < 1e-14, "{normal:?}: {pair:?}");
            }
            assert_eq!(mesh.normals[0], frames[0].normal);
        }
    }

    #[test]
    fn sizes_must_be_finite_and_nonnegative() {
        let frames = straight();
        for bad in [-0.5, f64::NAN, f64::INFINITY] {
            let sizes = [0.5, bad, 0.5];
            assert_eq!(
                tube(&frames, &sizes, 4, false),
                Err(SweepError::Size { station: 1 }),
                "{bad}"
            );
            assert_eq!(
                ribbon(&frames, None, &sizes, false),
                Err(SweepError::Size { station: 1 }),
                "{bad}"
            );
        }
        // Zero, of either sign, is a collapsed station.
        let mesh = tube(&frames, &[0.5, -0.0, 0.0], 4, false).unwrap();
        assert_eq!(mesh.report.collapsed, 2);
    }

    #[test]
    fn counts_are_planned_before_anything_is_allocated() {
        // Two stations of `u32::MAX` sides: refused before a ring table of
        // that many directions is built.
        let frames = &straight()[..2];
        assert_eq!(
            tube(frames, &[1.0; 2], u32::MAX, false),
            Err(SweepError::TooLarge)
        );
        // Vertices that fit `u32` whose triangles do not.
        assert_eq!(
            plan(1_400_000_000, 3, false, true),
            Err(SweepError::TooLarge)
        );
        assert_eq!(plan(usize::MAX, 2, false, false), Err(SweepError::TooLarge));
        let fits = plan(3, 4, true, true).unwrap();
        assert_eq!((fits.vertices, fits.triangles, fits.indices), (12, 24, 72));
    }

    /// Each normal is perpendicular to the surface's longitudinal edges at
    /// its vertex and faces away from the centerline.
    fn assert_surface_normals(mesh: &SweptMesh, frames: &[Frame]) {
        let ring = mesh.ring as usize;
        for (i, frame) in frames.iter().enumerate() {
            for k in 0..ring {
                let v = i * ring + k;
                let normal = mesh.normals[v];
                assert!((dot(normal, normal) - 1.0).abs() < 1e-14);
                let radial = sub(mesh.positions[v], frame.point);
                assert!(dot(normal, radial) >= 0.0, "station {i} vertex {k}");
                for j in [i.wrapping_sub(1), i + 1] {
                    let Some(other) = mesh.positions.get(j.wrapping_mul(ring) + k) else {
                        continue;
                    };
                    let edge = sub(*other, mesh.positions[v]);
                    let cosine = dot(normal, edge) / norm(edge);
                    assert!(cosine.abs() < 1e-12, "station {i} vertex {k}: {cosine}");
                }
            }
        }
    }

    #[test]
    fn a_frustum_has_surface_normals() {
        // Radius 1 to 2 over a unit length: the surface leans out at 45
        // degrees, so at angle zero the normal is (1, 0, -1) / sqrt 2.
        let points = [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let frames = rotation_minimizing_frames(&points, false, Some([1.0, 0.0, 0.0])).0;
        let mesh = tube(&frames, &[1.0, 2.0], 8, false).unwrap();
        let h = core::f64::consts::FRAC_1_SQRT_2;
        for v in [0, 8] {
            let n = mesh.normals[v];
            assert!((n[0] - h).abs() < 1e-15 && n[1] == 0.0 && (n[2] + h).abs() < 1e-15);
        }
        assert_surface_normals(&mesh, &frames);
        // A strong taper to a point over three stations, apex included.
        let points = [[0.0, 0.0, 0.0], [0.0, 0.0, 0.5], [0.0, 0.0, 1.0]];
        let frames = rotation_minimizing_frames(&points, false, None).0;
        let mesh = tube(&frames, &[3.0, 1.5, 0.0], 6, false).unwrap();
        assert_surface_normals(&mesh, &frames);
        // A constant radius keeps the radial direction exactly.
        let frames = straight();
        let mesh = tube(&frames, &[0.5; 3], 4, false).unwrap();
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(mesh.normals[i * 4], frame.normal);
            assert_eq!(mesh.normals[i * 4 + 1][2], 0.0);
        }
    }

    #[test]
    fn a_radius_step_at_a_repeated_point_faces_along_the_tangent() {
        // Coincident stations with different radii form a flat annulus,
        // whose normal is along the centerline.
        let points = [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
        let frames = rotation_minimizing_frames(&points, false, None).0;
        let mesh = tube(&frames, &[1.0, 1.0, 2.0], 4, false).unwrap();
        for k in 0..4 {
            let n = mesh.normals[8 + k];
            assert!((dot(n, [0.0, 0.0, 1.0]) + 1.0).abs() < 1e-15, "{n:?}");
        }
    }

    #[test]
    fn meshes_and_rings_are_reused() {
        let frames = straight();
        let mut workspace = Workspace::new();
        let mut mesh = SweptMesh::default();
        tube_into(&frames, &[0.5; 3], 6, true, &mut workspace, &mut mesh).unwrap();
        assert_eq!(mesh, tube(&frames, &[0.5; 3], 6, true).unwrap());
        let capacities = (mesh.positions.capacity(), mesh.indices.capacity());
        tube_into(
            &frames,
            &[0.5, 1.0, 0.5],
            6,
            false,
            &mut workspace,
            &mut mesh,
        )
        .unwrap();
        assert_eq!(mesh, tube(&frames, &[0.5, 1.0, 0.5], 6, false).unwrap());
        assert_eq!(
            (mesh.positions.capacity(), mesh.indices.capacity()),
            capacities
        );
        // A different side count rebuilds the ring.
        tube_into(&frames, &[0.5; 3], 4, false, &mut workspace, &mut mesh).unwrap();
        assert_eq!(mesh, tube(&frames, &[0.5; 3], 4, false).unwrap());
        // An error leaves the mesh empty.
        assert!(tube_into(&frames, &[0.5], 4, false, &mut workspace, &mut mesh).is_err());
        assert_eq!(mesh, SweptMesh::default());
        ribbon_into(&frames, None, &[1.0; 3], false, &mut mesh).unwrap();
        assert_eq!(mesh, ribbon(&frames, None, &[1.0; 3], false).unwrap());
    }
}
