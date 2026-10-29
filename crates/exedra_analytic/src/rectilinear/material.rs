// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    Operand, RectilinearError, RectilinearPolicy, classify, contains, decode, encode, grid::Grid,
    inspect, interval_product, interval_sum, neighbor, used_points,
};
use crate::{
    body::{Body, FaceId, ValidationPolicy},
    validation::{
        BoundaryValidationError, BoundaryValidationPolicy, BoundaryValidationReport,
        TrimValidation, UnsupportedTrim,
    },
};
use alloc::{vec, vec::Vec};

/// Independent topology/trim and finite arrangement limits for material validation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MaterialValidationPolicy {
    /// Local support, trim separation, native scratch and pair-work limits.
    pub boundary: BoundaryValidationPolicy,
    /// Coordinate spacing, cell scratch and face-scan limits. No Boolean is built.
    pub arrangement: RectilinearPolicy,
    /// Maximum flat cell/region/link/ownership scratch, admitted before allocation.
    /// Coordinate and face/chart preparation have separate native-table limits.
    pub max_cell_scratch_bytes: usize,
}
impl Default for MaterialValidationPolicy {
    fn default() -> Self {
        Self {
            boundary: BoundaryValidationPolicy::default(),
            arrangement: RectilinearPolicy::default(),
            max_cell_scratch_bytes: 128 * 1024 * 1024,
        }
    }
}

/// A checked, connected orthogonal solid with explicitly owned cavity shells.
///
/// Every input face patch matches exactly one outward material transition in an
/// exact plane arrangement. All three ray windings agree in every open cell;
/// boundary links are single cycles, and distinct shells never touch. The
/// certificate concerns the retained axis-aligned plane/line family only.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialValidationReport {
    /// Complete native topology and trim evidence, including shell face lists.
    pub boundary: BoundaryValidationReport,
    /// Shell index bounding the unbounded exterior.
    pub outer_shell: usize,
    /// Shell indices bounding distinct closed voids; no void contains another.
    pub void_shells: Vec<usize>,
    /// Exact input coordinate-plane counts along X, Y and Z.
    pub planes: [usize; 3],
    /// Open arrangement cells classified without constructing output geometry.
    pub cells: usize,
    /// Actual face entries scanned during winding and face-transition checks.
    pub face_tests: usize,
    /// Face-scan upper bound admitted before allocating arrangement scratch.
    pub face_test_bound: usize,
    /// Material volume from exact arrangement boxes.
    pub volume: f64,
    /// Outward-rounded enclosure of volume arithmetic.
    pub volume_bounds: [f64; 2],
    /// Boundary area from exact arrangement rectangles, including cavity walls.
    pub surface_area: f64,
    /// Outward-rounded enclosure of area arithmetic.
    pub surface_area_bounds: [f64; 2],
    /// Conservative coordinate arithmetic allowance; no geometry is moved.
    pub arithmetic_allowance: f64,
    /// Exact coordinate-plane vertices in the flat geometric-link table (at most eight per cell).
    pub plane_vertices: usize,
    /// Reserved bytes in that table, including shell ownership and six-ray links.
    pub link_bytes: usize,
    /// Peak reserved bytes in flat arrangement/grid arrays. Excludes face/chart
    /// preparation, retained report vectors and allocator overhead.
    pub arrangement_scratch_bytes: usize,
}

/// Why the bounded rectilinear material certificate could not be established.
#[derive(Clone, Debug, PartialEq)]
pub enum MaterialValidationError {
    /// Native topology, support or face trim validation failed.
    Boundary(BoundaryValidationError),
    /// Unsupported axis geometry, spacing, winding, finite arithmetic or work limit.
    Arrangement(RectilinearError),
    /// A local trim arrangement did not meet the requested independent policy.
    UncertifiedTrim {
        /// Refused native face.
        face: FaceId,
        /// Structured reason its trim arrangement remains unresolved.
        reason: UnsupportedTrim,
    },
    /// Flat cell/region/link/ownership scratch exceeds the caller's byte budget.
    ScratchBudget {
        /// Required bytes; absent if their count overflowed.
        required: Option<usize>,
        /// Maximum admitted bytes before allocation.
        maximum: usize,
    },
    /// One connected positive-volume material region is required.
    MaterialComponents {
        /// Number of regions connected through positive-area cell faces.
        found: usize,
    },
    /// A face patch is duplicated, internal, missing or has the wrong material sense.
    FaceTransition {
        /// Input face when one is present; absent for a missing boundary patch.
        face: Option<FaceId>,
        /// Coordinate-plane axis.
        axis: usize,
        /// Grid plane index and two transverse interval indices.
        patch: [usize; 3],
    },
    /// Distinct topological shells touch in the geometric arrangement.
    ShellContact {
        /// First shell index in the boundary report.
        first: usize,
        /// Second shell index in the boundary report.
        second: usize,
    },
    /// A geometric boundary vertex has no single manifold link cycle.
    BoundaryLink {
        /// Exact coordinate-plane indices of the vertex.
        vertex: [usize; 3],
    },
    /// A shell does not bound exactly one exterior or distinct closed void region.
    ShellOwnership {
        /// Refused shell index in the boundary report.
        shell: usize,
    },
}
impl core::fmt::Display for MaterialValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "rectilinear material validation: {self:?}")
    }
}
impl core::error::Error for MaterialValidationError {}
impl From<RectilinearError> for MaterialValidationError {
    fn from(value: RectilinearError) -> Self {
        Self::Arrangement(value)
    }
}

#[derive(Clone)]
struct Link {
    adjacent: [u8; 6],
    shell: usize,
}
impl Link {
    fn add(
        &mut self,
        first: usize,
        second: usize,
        shell: usize,
    ) -> Result<(), MaterialValidationError> {
        if self.shell != usize::MAX && self.shell != shell {
            return Err(MaterialValidationError::ShellContact {
                first: self.shell,
                second: shell,
            });
        }
        self.shell = shell;
        self.adjacent[first] |= 1 << second;
        self.adjacent[second] |= 1 << first;
        Ok(())
    }
    fn is_cycle(&self) -> bool {
        let active = self
            .adjacent
            .iter()
            .enumerate()
            .fold(0_u8, |mask, (i, &edges)| {
                mask | if edges == 0 { 0 } else { 1 << i }
            });
        if active == 0 {
            return true;
        }
        if self.adjacent.iter().any(|&n| n != 0 && n.count_ones() != 2) {
            return false;
        }
        let mut pending = 1_u8 << active.trailing_zeros();
        let mut seen = 0_u8;
        while pending != 0 {
            let i = pending.trailing_zeros() as usize;
            pending &= !(1 << i);
            seen |= 1 << i;
            pending |= self.adjacent[i] & !seen;
        }
        seen == active
    }
}

/// Certifies one connected rectilinear material region and its disjoint cavities.
///
/// No boundary is reconstructed, tessellated or healed. Finite plane cells prove
/// whole regions because no input boundary crosses a cell interior. Every input
/// face is also checked against the resulting transitions: winding/counts alone
/// cannot hide duplicate, internal or oppositely cancelling faces. Scratch is
/// flat and bounded by input-table, cell and face-test policies.
///
/// # Errors
/// Refuses unsupported geometry, invalid/touching/nested material, disconnected
/// solids, unresolved spacing/arithmetic and exhausted work budgets explicitly.
pub fn validate_material<F, E>(
    body: &Body<F, E>,
    policy: &MaterialValidationPolicy,
) -> Result<MaterialValidationReport, MaterialValidationError> {
    let p = &policy.arrangement;
    if policy.max_cell_scratch_bytes == 0
        || p.max_cells == 0
        || p.max_face_tests == 0
        || !p.minimum_spacing.is_finite()
        || p.minimum_spacing <= 0.0
        || !p.geometry_gap.is_finite()
        || p.geometry_gap < 0.0
    {
        return Err(RectilinearError::InvalidPolicy.into());
    }
    let boundary = body
        .validate_boundary(&policy.boundary)
        .map_err(MaterialValidationError::Boundary)?;
    let faces = inspect(
        body,
        Operand::Left,
        &ValidationPolicy {
            max_gap: p.geometry_gap,
        },
    )?;
    for (i, &status) in boundary.face_trims.iter().enumerate() {
        if let TrimValidation::Unsupported(reason) = status {
            return Err(MaterialValidationError::UncertifiedTrim {
                face: FaceId(crate::usize_to_u32(i)),
                reason,
            });
        }
    }
    let grid = Grid::new(used_points(body), p, faces.len())?;
    let face_test_bound = grid.face_test_bound;
    let vertex_count = grid
        .planes
        .into_iter()
        .try_fold(1_usize, usize::checked_mul);
    let cell_bytes = grid
        .cells
        .checked_mul(size_of::<bool>() + size_of::<usize>());
    let queue_bytes = grid.cells.checked_mul(size_of::<usize>());
    let link_bytes = vertex_count.and_then(|n| n.checked_mul(size_of::<Link>()));
    let ownership_bytes = faces.len().checked_mul(size_of::<usize>()).and_then(|n| {
        boundary
            .shells
            .len()
            .checked_mul(size_of::<Option<usize>>() + size_of::<usize>())
            .and_then(|m| n.checked_add(m))
    });
    let required = cell_bytes.and_then(|n| {
        let phase = queue_bytes?.max(link_bytes?.checked_add(ownership_bytes?)?);
        n.checked_add(phase)
    });
    if required.is_none_or(|n| n > policy.max_cell_scratch_bytes || n > isize::MAX as usize) {
        return Err(MaterialValidationError::ScratchBudget {
            required,
            maximum: policy.max_cell_scratch_bytes.min(isize::MAX as usize),
        });
    }
    let vertex_count = vertex_count.expect("admitted scratch");
    let mut material = vec![false; grid.cells];
    let mut volume = 0.0;
    let mut volume_bounds = [0.0; 2];
    for (i, filled) in material.iter_mut().enumerate() {
        let cell = decode(i, grid.dimensions);
        let point = core::array::from_fn(|axis| grid.references[axis][cell[axis]]);
        *filled = classify(&faces, point, Operand::Left, cell)?;
        if *filled {
            let mut v = 1.0;
            let mut bound = [1.0; 2];
            for (axis, &index) in cell.iter().enumerate() {
                let length = grid.coordinates[axis][index + 1] - grid.coordinates[axis][index];
                v *= length;
                bound = interval_product(bound, [length.next_down().max(0.0), length.next_up()]);
            }
            volume += v;
            volume_bounds = interval_sum(volume_bounds, bound);
        }
    }
    let (regions, cavities, queue_bytes) = regions(&material, grid.dimensions)?;
    let mut face_shell = vec![usize::MAX; faces.len()];
    for (shell, entry) in boundary.shells.iter().enumerate() {
        for &face in &entry.faces {
            face_shell[face.index()] = shell;
        }
    }
    let mut links = vec![
        Link {
            adjacent: [0; 6],
            shell: usize::MAX
        };
        vertex_count
    ];
    let mut shell_region = vec![None; boundary.shells.len()];
    let mut face_tests = grid.cells * faces.len() * 3;
    let mut surface_area = 0.0;
    let mut surface_area_bounds = [0.0; 2];
    for axis in 0..3 {
        let a = (axis + 1) % 3;
        let b = (axis + 2) % 3;
        for plane in 0..grid.planes[axis] {
            for u in 0..grid.dimensions[a] {
                for v in 0..grid.dimensions[b] {
                    let patch = [plane, u, v];
                    let refusal =
                        |face| MaterialValidationError::FaceTransition { face, axis, patch };
                    let uv = [grid.references[a][u], grid.references[b][v]];
                    let mut found = None;
                    face_tests += faces.len();
                    for face in &faces {
                        if face.axis == axis
                            && face.coordinate == grid.coordinates[axis][plane]
                            && contains(face, uv)
                        {
                            if found.is_some() {
                                return Err(refusal(Some(face.id)));
                            }
                            found = Some(face);
                        }
                    }
                    let cell_at = |i| {
                        let mut cell = [0; 3];
                        cell[axis] = i;
                        cell[a] = u;
                        cell[b] = v;
                        encode(cell, grid.dimensions)
                    };
                    let below = plane.checked_sub(1).map(cell_at);
                    let above = (plane < grid.dimensions[axis]).then(|| cell_at(plane));
                    let filled = |cell: Option<usize>| cell.is_some_and(|i| material[i]);
                    let negative = filled(below);
                    let positive = filled(above);
                    if negative == positive {
                        if let Some(face) = found {
                            return Err(refusal(Some(face.id)));
                        }
                        continue;
                    }
                    let face = found.ok_or_else(|| refusal(None))?;
                    if (face.sign > 0) != negative {
                        return Err(refusal(Some(face.id)));
                    }
                    let shell = face_shell[face.id.index()];
                    let empty = if negative { above } else { below };
                    let region = empty.map_or(0, |i| regions[i]);
                    if shell_region[shell].is_some_and(|previous| previous != region) {
                        return Err(MaterialValidationError::ShellOwnership { shell });
                    }
                    shell_region[shell] = Some(region);
                    let lengths = [a, b].map(|axis| {
                        let index = if axis == a { u } else { v };
                        grid.coordinates[axis][index + 1] - grid.coordinates[axis][index]
                    });
                    surface_area += lengths[0] * lengths[1];
                    surface_area_bounds = interval_sum(
                        surface_area_bounds,
                        interval_product(
                            [lengths[0].next_down().max(0.0), lengths[0].next_up()],
                            [lengths[1].next_down().max(0.0), lengths[1].next_up()],
                        ),
                    );
                    for da in 0..2 {
                        for db in 0..2 {
                            let mut vertex = [0; 3];
                            vertex[axis] = plane;
                            vertex[a] = u + da;
                            vertex[b] = v + db;
                            links[encode(vertex, grid.planes)].add(
                                2 * a + usize::from(da == 0),
                                2 * b + usize::from(db == 0),
                                shell,
                            )?;
                        }
                    }
                }
            }
        }
    }
    for (i, link) in links.iter().enumerate() {
        if !link.is_cycle() {
            return Err(MaterialValidationError::BoundaryLink {
                vertex: decode(i, grid.planes),
            });
        }
    }
    let mut outer_shell = None;
    let mut void_shells = vec![usize::MAX; cavities];
    for (shell, region) in shell_region.into_iter().enumerate() {
        match region {
            Some(0) if outer_shell.is_none() => outer_shell = Some(shell),
            Some(region)
                if region > 0 && region <= cavities && void_shells[region - 1] == usize::MAX =>
            {
                void_shells[region - 1] = shell;
            }
            _ => return Err(MaterialValidationError::ShellOwnership { shell }),
        }
    }
    if void_shells.contains(&usize::MAX) || outer_shell.is_none() {
        return Err(MaterialValidationError::ShellOwnership { shell: 0 });
    }
    if !volume.is_finite()
        || !surface_area.is_finite()
        || !volume_bounds[1].is_finite()
        || !surface_area_bounds[1].is_finite()
        || volume <= 0.0
    {
        return Err(RectilinearError::NumericLimit.into());
    }
    let grid_bytes = grid
        .coordinates
        .iter()
        .chain(&grid.references)
        .map(|v| v.capacity() * size_of::<f64>())
        .sum::<usize>();
    let common_bytes = grid_bytes
        + material.capacity() * size_of::<bool>()
        + regions.capacity() * size_of::<usize>();
    let link_bytes = links.capacity() * size_of::<Link>();
    let arrangement_scratch_bytes = common_bytes
        + queue_bytes.max(
            link_bytes
                + face_shell.capacity() * size_of::<usize>()
                + boundary.shells.len() * size_of::<Option<usize>>()
                + void_shells.capacity() * size_of::<usize>(),
        );
    Ok(MaterialValidationReport {
        boundary,
        outer_shell: outer_shell.expect("checked"),
        void_shells,
        planes: grid.planes,
        cells: grid.cells,
        face_tests,
        face_test_bound,
        volume,
        volume_bounds,
        surface_area,
        surface_area_bounds,
        arithmetic_allowance: grid.arithmetic,
        plane_vertices: vertex_count,
        link_bytes,
        arrangement_scratch_bytes,
    })
}

// Empty regions reaching any side of the finite arrangement share the unbounded
// exterior (0), even when they connect only outside the arrangement's box.
fn regions(
    material: &[bool],
    dimensions: [usize; 3],
) -> Result<(Vec<usize>, usize, usize), MaterialValidationError> {
    let mut regions = vec![usize::MAX; material.len()];
    let mut queue = Vec::with_capacity(material.len());
    let mut cavities = 0;
    let mut components = 0;
    for start in 0..material.len() {
        if regions[start] != usize::MAX {
            continue;
        }
        let filled = material[start];
        let mut exterior = false;
        queue.clear();
        queue.push(start);
        regions[start] = usize::MAX - 1;
        let mut read = 0;
        while read < queue.len() {
            let cell = decode(queue[read], dimensions);
            read += 1;
            for axis in 0..3 {
                exterior |= cell[axis] == 0 || cell[axis] + 1 == dimensions[axis];
                for positive in [false, true] {
                    if let Some(next) = neighbor(cell, axis, positive, dimensions) {
                        let i = encode(next, dimensions);
                        if regions[i] == usize::MAX && material[i] == filled {
                            regions[i] = usize::MAX - 1;
                            queue.push(i);
                        }
                    }
                }
            }
        }
        let region = if filled {
            components += 1;
            usize::MAX - 2
        } else if exterior {
            0
        } else {
            cavities += 1;
            cavities
        };
        for &i in &queue {
            regions[i] = region;
        }
    }
    if components != 1 {
        return Err(MaterialValidationError::MaterialComponents { found: components });
    }
    Ok((regions, cavities, queue.capacity() * size_of::<usize>()))
}
