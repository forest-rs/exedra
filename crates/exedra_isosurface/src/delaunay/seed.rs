// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Starting points on the surface, from a grid of signs.
//!
//! The grid is only sampled, never meshed. Blocks of cells whose value
//! bounds exclude zero hold no surface and are skipped whole. In the rest,
//! each cell edge whose ends disagree in sign gives a crossing, each cell
//! near a crease gives a point on it, and each cell whose corners agree is
//! probed for a feature thinner than the cell.

use alloc::vec::Vec;

use exedra_implicit::Aabb;

use super::Settings;
use super::sample::Sampler;

/// Cells along each side of a block tested as one.
const BLOCK: usize = 8;

/// Points on the surface and on its creases.
#[derive(Clone, Debug, Default)]
pub(super) struct Seeds {
    /// Crossings of cell edges.
    pub(super) edges: Vec<[f64; 3]>,
    /// Points on features thinner than a cell.
    pub(super) thin: Vec<[f64; 3]>,
    /// Points on creases.
    pub(super) creases: Vec<[f64; 3]>,
    /// Whether the interior reaches the grid's boundary.
    pub(super) clipped: bool,
    /// Blocks of cells tested, and those proven to hold no surface.
    pub(super) blocks: usize,
    pub(super) empty_blocks: usize,
}

/// The lattice the signs are sampled on.
#[derive(Copy, Clone, Debug)]
pub(super) struct Grid {
    pub(super) origin: [f64; 3],
    pub(super) cells: [usize; 3],
    /// The side of a cell along each axis, at most the size asked for.
    pub(super) spacing: [f64; 3],
    /// The far corner, exactly.
    far: [f64; 3],
}

impl Grid {
    /// The lattice spanning exactly `bounds`, with cells no longer than
    /// `size` along any axis.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "positive cell counts checked against the limit by the caller"
    )]
    pub(super) fn covering(bounds: &Aabb, size: f64) -> Self {
        let extent = bounds.extent();
        let cells = extent.map(|e| {
            let cells = e / size;
            let whole = cells as usize;
            if (whole as f64) < cells {
                whole + 1
            } else {
                whole.max(1)
            }
        });
        Self {
            origin: bounds.min,
            cells,
            spacing: core::array::from_fn(|k| {
                if extent[k] > 0.0 {
                    extent[k] / cells[k] as f64
                } else {
                    size
                }
            }),
            far: core::array::from_fn(|k| {
                if extent[k] > 0.0 {
                    bounds.max[k]
                } else {
                    bounds.min[k] + size
                }
            }),
        }
    }

    /// The longest side of a cell.
    pub(super) fn cell(&self) -> f64 {
        self.spacing.into_iter().fold(0.0, f64::max)
    }

    #[expect(clippy::cast_precision_loss, reason = "lattice indices are small")]
    pub(super) fn point(&self, index: [usize; 3]) -> [f64; 3] {
        core::array::from_fn(|k| {
            if index[k] == self.cells[k] {
                self.far[k]
            } else {
                self.origin[k] + index[k] as f64 * self.spacing[k]
            }
        })
    }

    /// The far corner of the lattice.
    pub(super) fn end(&self) -> [f64; 3] {
        self.point(self.cells)
    }
}

pub(super) fn seed(sampler: &Sampler<'_>, settings: &Settings, grid: &Grid) -> Seeds {
    let mut seeds = Seeds::default();
    let blocks = grid.cells.map(|n| n.div_ceil(BLOCK));
    let mut values = Vec::new();
    for bi in 0..blocks[0] {
        for bj in 0..blocks[1] {
            for bk in 0..blocks[2] {
                let low = [bi * BLOCK, bj * BLOCK, bk * BLOCK];
                let high: [usize; 3] =
                    core::array::from_fn(|k| (low[k] + BLOCK).min(grid.cells[k]));
                seed_block(sampler, settings, grid, low, high, &mut values, &mut seeds);
            }
        }
    }
    seeds
}

fn seed_block(
    sampler: &Sampler<'_>,
    settings: &Settings,
    grid: &Grid,
    low: [usize; 3],
    high: [usize; 3],
    values: &mut Vec<f64>,
    seeds: &mut Seeds,
) {
    seeds.blocks += 1;
    let at_boundary = (0..3).any(|k| low[k] == 0 || high[k] == grid.cells[k]);
    let region = Aabb {
        min: grid.point(low),
        max: grid.point(high),
    };
    let bounds = sampler.interval(&region);
    if bounds.low() > 0.0 || bounds.high() < 0.0 {
        seeds.empty_blocks += 1;
        seeds.clipped |= at_boundary && bounds.high() < 0.0;
        return;
    }
    let size: [usize; 3] = core::array::from_fn(|k| high[k] - low[k] + 1);
    let slot =
        |[i, j, k]: [usize; 3]| ((i - low[0]) * size[1] + (j - low[1])) * size[2] + (k - low[2]);
    // The block's lattice in one query.
    let mut points = Vec::with_capacity(size[0] * size[1] * size[2]);
    for i in low[0]..=high[0] {
        for j in low[1]..=high[1] {
            for k in low[2]..=high[2] {
                points.push(grid.point([i, j, k]));
            }
        }
    }
    values.clear();
    values.resize(points.len(), 0.0);
    sampler.values(&points, values);
    for i in low[0]..=high[0] {
        for j in low[1]..=high[1] {
            for k in low[2]..=high[2] {
                let on_boundary =
                    (0..3).any(|a| [i, j, k][a] == 0 || [i, j, k][a] == grid.cells[a]);
                seeds.clipped |= on_boundary && values[slot([i, j, k])] < 0.0;
            }
        }
    }
    let h = grid.cell();
    for i in low[0]..high[0] {
        for j in low[1]..high[1] {
            for k in low[2]..high[2] {
                let corner = |c: usize| [i + (c >> 2), j + ((c >> 1) & 1), k + (c & 1)];
                let inside = (0..8).filter(|&c| values[slot(corner(c))] < 0.0).count();
                let origin = [i, j, k];
                let (p0, f0) = (grid.point(origin), values[slot(origin)]);
                if inside == 0 || inside == 8 {
                    if settings.thin_depth == 0 {
                        continue;
                    }
                    let cell = Aabb {
                        min: p0,
                        max: grid.point([i + 1, j + 1, k + 1]),
                    };
                    if let Some((p, value)) = sampler.probe(cell, inside == 8, settings.thin_depth)
                    {
                        seeds.thin.push(if value < 0.0 {
                            sampler.search(p, value, p0, f0, settings.tolerance)
                        } else {
                            sampler.search(p0, f0, p, value, settings.tolerance)
                        });
                    }
                    continue;
                }
                let far = grid.point([i + 1, j + 1, k + 1]);
                let middle = core::array::from_fn(|n| 0.5 * (p0[n] + far[n]));
                if let Some(tie) = sampler.crease(middle).ties[0]
                    && tie.distance < 1.5 * h
                    && let Some(x) = sampler.crease_point(middle, 2.0 * h)
                    && sampler.value(x).abs() < settings.on_surface
                {
                    seeds.creases.push(x);
                }
                for axis in 0..3 {
                    let mut other = origin;
                    other[axis] += 1;
                    let (pa, fa) = (p0, f0);
                    let (pb, fb) = (grid.point(other), values[slot(other)]);
                    if (fa < 0.0) == (fb < 0.0) {
                        continue;
                    }
                    seeds.edges.push(if fa < 0.0 {
                        sampler.search(pa, fa, pb, fb, settings.tolerance)
                    } else {
                        sampler.search(pb, fb, pa, fa, settings.tolerance)
                    });
                }
            }
        }
    }
}
