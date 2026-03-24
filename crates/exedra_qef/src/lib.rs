// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Small quadratic error function solving for Exedra.
//!
//! The crate owns:
//! - plane-constraint accumulation via [`PlaneConstraint`],
//! - bounded solving with [`QefSolver`],
//! - rank-based feature classification via [`SharpnessClass`].
//!
//! The crate intentionally does not know about scalar fields, Hermite edge
//! sampling, octrees, or mesh output.

#![no_std]
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_qef requires either the `std` or `libm` feature");

/// One point-normal plane constraint used by the QEF solver.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlaneConstraint {
    /// One point on the plane.
    pub position: [f32; 3],
    /// Plane normal direction.
    pub normal: [f32; 3],
}

/// Bounding box used to anchor and clamp one QEF solve.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct QefBounds {
    /// Minimum corner.
    pub min: [f32; 3],
    /// Maximum corner.
    pub max: [f32; 3],
}

impl QefBounds {
    /// Creates bounds when `min <= max` on every axis.
    #[must_use]
    pub fn new(min: [f32; 3], max: [f32; 3]) -> Option<Self> {
        if min[0] > max[0] || min[1] > max[1] || min[2] > max[2] {
            return None;
        }
        Some(Self { min, max })
    }

    /// Returns the center point.
    #[must_use]
    pub fn center(self) -> [f32; 3] {
        [
            0.5 * (self.min[0] + self.max[0]),
            0.5 * (self.min[1] + self.max[1]),
            0.5 * (self.min[2] + self.max[2]),
        ]
    }

    /// Clamps `point` into the bounds.
    #[must_use]
    pub fn clamp(self, point: [f32; 3]) -> [f32; 3] {
        [
            point[0].clamp(self.min[0], self.max[0]),
            point[1].clamp(self.min[1], self.max[1]),
            point[2].clamp(self.min[2], self.max[2]),
        ]
    }
}

/// Rank-derived feature class for a local QEF solve.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SharpnessClass {
    /// One dominant plane or no constraints.
    Smooth,
    /// Two dominant planes meeting in an edge-like feature.
    Edge,
    /// Three dominant planes meeting in a corner-like feature.
    Corner,
}

/// Parameters controlling one QEF solve.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct QefParams {
    /// Relative cutoff applied against the largest eigenvalue.
    pub relative_eigenvalue_cutoff: f32,
    /// Maximum Jacobi sweeps used for the symmetric eigensolve.
    pub jacobi_sweeps: u8,
}

impl Default for QefParams {
    fn default() -> Self {
        Self {
            relative_eigenvalue_cutoff: 1.0e-3,
            jacobi_sweeps: 12,
        }
    }
}

/// Successful bounded QEF solution.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct QefResult {
    /// Solved point after null-space anchoring and bounds clamping.
    pub position: [f32; 3],
    /// Squared residual error at `position`.
    pub residual_error: f32,
    /// Effective matrix rank after eigenvalue thresholding.
    pub rank: u8,
    /// Rank-derived feature class.
    pub sharpness_class: SharpnessClass,
    /// Sorted eigenvalues in descending order.
    pub eigenvalues: [f32; 3],
    /// Whether the final point was clamped to `QefBounds`.
    pub was_clamped: bool,
}

/// QEF solve failure.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum QefSolveError {
    /// No usable plane constraints were accumulated.
    Empty,
}

/// Incremental QEF accumulator for bounded 3D solves.
#[derive(Clone, Debug, PartialEq)]
pub struct QefSolver {
    ata: [[f32; 3]; 3],
    atb: [f32; 3],
    btb: f32,
    constraint_count: u32,
}

impl Default for QefSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl QefSolver {
    /// Creates an empty accumulator.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ata: [[0.0; 3]; 3],
            atb: [0.0; 3],
            btb: 0.0,
            constraint_count: 0,
        }
    }

    /// Returns how many usable constraints were accumulated.
    #[must_use]
    pub const fn constraint_count(&self) -> u32 {
        self.constraint_count
    }

    /// Adds one point-normal plane constraint.
    ///
    /// Returns `true` when the constraint was accepted. Zero-length or
    /// non-finite normals are rejected because they do not define a stable
    /// plane.
    pub fn add(&mut self, position: [f32; 3], normal: [f32; 3]) -> bool {
        self.add_constraint(PlaneConstraint { position, normal })
    }

    /// Adds one constraint record.
    pub fn add_constraint(&mut self, constraint: PlaneConstraint) -> bool {
        let Some(unit_normal) = normalize3(constraint.normal) else {
            return false;
        };
        if !constraint
            .position
            .iter()
            .all(|component| component.is_finite())
        {
            return false;
        }

        let plane_distance = dot3(unit_normal, constraint.position);
        accumulate_outer(&mut self.ata, unit_normal);
        for (axis, component) in unit_normal.into_iter().enumerate() {
            self.atb[axis] += component * plane_distance;
        }
        self.btb += plane_distance * plane_distance;
        self.constraint_count += 1;
        true
    }

    /// Adds every constraint in `constraints`.
    pub fn add_constraints(&mut self, constraints: &[PlaneConstraint]) {
        for constraint in constraints {
            let _ = self.add_constraint(*constraint);
        }
    }

    /// Solves the accumulated QEF inside `bounds`.
    ///
    /// Low-rank solves pin the null-space dimensions to `bounds.center()`
    /// before the final point is clamped back into the bounds.
    pub fn solve(&self, bounds: QefBounds, params: &QefParams) -> Result<QefResult, QefSolveError> {
        self.solve_with_anchor(bounds, bounds.center(), params)
    }

    /// Solves the accumulated QEF inside `bounds`, pinning low-rank null-space
    /// dimensions to `anchor`.
    ///
    /// `anchor` is clamped into `bounds` before use so callers may pass
    /// slightly out-of-bounds mass points without destabilizing the solve.
    pub fn solve_with_anchor(
        &self,
        bounds: QefBounds,
        anchor: [f32; 3],
        params: &QefParams,
    ) -> Result<QefResult, QefSolveError> {
        if self.constraint_count == 0 {
            return Err(QefSolveError::Empty);
        }

        let (eigenvalues, eigenvectors) =
            symmetric_eigendecomposition(self.ata, params.jacobi_sweeps);
        let rank = effective_rank(eigenvalues, params.relative_eigenvalue_cutoff);
        let anchor = bounds.clamp(anchor);
        let anchor_coeffs = project_columns(eigenvectors, anchor);
        let rhs_coeffs = project_columns(eigenvectors, self.atb);

        let mut solution_coeffs = anchor_coeffs;
        for axis in 0..usize::from(rank) {
            solution_coeffs[axis] = rhs_coeffs[axis] / eigenvalues[axis];
        }

        let unconstrained = reconstruct_from_columns(eigenvectors, solution_coeffs);
        let position = bounds.clamp(unconstrained);
        let was_clamped = position != unconstrained;
        let residual_error = residual(self.ata, self.atb, self.btb, position);

        Ok(QefResult {
            position,
            residual_error,
            rank,
            sharpness_class: SharpnessClass::from_rank(rank),
            eigenvalues,
            was_clamped,
        })
    }
}

impl SharpnessClass {
    fn from_rank(rank: u8) -> Self {
        match rank {
            0 | 1 => Self::Smooth,
            2 => Self::Edge,
            _ => Self::Corner,
        }
    }
}

fn accumulate_outer(matrix: &mut [[f32; 3]; 3], vector: [f32; 3]) {
    for row in 0..3 {
        for col in 0..3 {
            matrix[row][col] += vector[row] * vector[col];
        }
    }
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize3(vector: [f32; 3]) -> Option<[f32; 3]> {
    if !vector.iter().all(|component| component.is_finite()) {
        return None;
    }
    let length_squared = dot3(vector, vector);
    if length_squared <= 0.0 {
        return None;
    }
    let inv_length = sqrt(length_squared).recip();
    Some([
        vector[0] * inv_length,
        vector[1] * inv_length,
        vector[2] * inv_length,
    ])
}

fn effective_rank(eigenvalues: [f32; 3], relative_cutoff: f32) -> u8 {
    let max_eigenvalue = eigenvalues[0].max(0.0);
    if max_eigenvalue <= f32::EPSILON {
        return 0;
    }

    let threshold = max_eigenvalue * relative_cutoff.max(0.0);
    let mut rank = 0_u8;
    for eigenvalue in eigenvalues {
        if eigenvalue > threshold && eigenvalue > f32::EPSILON {
            rank += 1;
        }
    }
    rank
}

fn project_columns(matrix: [[f32; 3]; 3], vector: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * vector[0] + matrix[1][0] * vector[1] + matrix[2][0] * vector[2],
        matrix[0][1] * vector[0] + matrix[1][1] * vector[1] + matrix[2][1] * vector[2],
        matrix[0][2] * vector[0] + matrix[1][2] * vector[1] + matrix[2][2] * vector[2],
    ]
}

fn reconstruct_from_columns(matrix: [[f32; 3]; 3], coeffs: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * coeffs[0] + matrix[0][1] * coeffs[1] + matrix[0][2] * coeffs[2],
        matrix[1][0] * coeffs[0] + matrix[1][1] * coeffs[1] + matrix[1][2] * coeffs[2],
        matrix[2][0] * coeffs[0] + matrix[2][1] * coeffs[1] + matrix[2][2] * coeffs[2],
    ]
}

fn residual(ata: [[f32; 3]; 3], atb: [f32; 3], btb: f32, point: [f32; 3]) -> f32 {
    let ata_point = [
        dot3(ata[0], point),
        dot3(ata[1], point),
        dot3(ata[2], point),
    ];
    let quadratic = dot3(point, ata_point);
    let linear = 2.0 * dot3(point, atb);
    (quadratic - linear + btb).max(0.0)
}

fn symmetric_eigendecomposition(
    matrix: [[f32; 3]; 3],
    jacobi_sweeps: u8,
) -> ([f32; 3], [[f32; 3]; 3]) {
    let mut diagonalized = matrix;
    let mut eigenvectors = identity3();

    for _ in 0..jacobi_sweeps {
        let Some((p, q, magnitude)) = largest_off_diagonal(diagonalized) else {
            break;
        };
        if magnitude <= 1.0e-10 {
            break;
        }
        apply_jacobi_rotation(&mut diagonalized, &mut eigenvectors, p, q);
    }

    let mut pairs = [
        (
            diagonalized[0][0].max(0.0),
            [eigenvectors[0][0], eigenvectors[1][0], eigenvectors[2][0]],
        ),
        (
            diagonalized[1][1].max(0.0),
            [eigenvectors[0][1], eigenvectors[1][1], eigenvectors[2][1]],
        ),
        (
            diagonalized[2][2].max(0.0),
            [eigenvectors[0][2], eigenvectors[1][2], eigenvectors[2][2]],
        ),
    ];
    sort_pairs_descending(&mut pairs);

    let sorted_eigenvalues = [pairs[0].0, pairs[1].0, pairs[2].0];
    let sorted_eigenvectors = [
        [pairs[0].1[0], pairs[1].1[0], pairs[2].1[0]],
        [pairs[0].1[1], pairs[1].1[1], pairs[2].1[1]],
        [pairs[0].1[2], pairs[1].1[2], pairs[2].1[2]],
    ];

    (sorted_eigenvalues, sorted_eigenvectors)
}

fn identity3() -> [[f32; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

fn largest_off_diagonal(matrix: [[f32; 3]; 3]) -> Option<(usize, usize, f32)> {
    let entries = [
        (0, 1, matrix[0][1].abs()),
        (0, 2, matrix[0][2].abs()),
        (1, 2, matrix[1][2].abs()),
    ];
    entries
        .into_iter()
        .max_by(|left, right| left.2.total_cmp(&right.2))
}

fn apply_jacobi_rotation(
    matrix: &mut [[f32; 3]; 3],
    eigenvectors: &mut [[f32; 3]; 3],
    p: usize,
    q: usize,
) {
    let app = matrix[p][p];
    let aqq = matrix[q][q];
    let apq = matrix[p][q];
    if apq == 0.0 {
        return;
    }

    let tau = (aqq - app) / (2.0 * apq);
    let tangent = if tau >= 0.0 {
        1.0 / (tau + sqrt(1.0 + tau * tau))
    } else {
        -1.0 / (-tau + sqrt(1.0 + tau * tau))
    };
    let cosine = 1.0 / sqrt(1.0 + tangent * tangent);
    let sine = tangent * cosine;

    for axis in [0_usize, 1, 2] {
        if axis == p || axis == q {
            continue;
        }
        let aip = matrix[axis][p];
        let aiq = matrix[axis][q];
        let rotated_ip = cosine * aip - sine * aiq;
        let rotated_iq = sine * aip + cosine * aiq;
        matrix[axis][p] = rotated_ip;
        matrix[p][axis] = rotated_ip;
        matrix[axis][q] = rotated_iq;
        matrix[q][axis] = rotated_iq;
    }

    matrix[p][p] = cosine * cosine * app - 2.0 * sine * cosine * apq + sine * sine * aqq;
    matrix[q][q] = sine * sine * app + 2.0 * sine * cosine * apq + cosine * cosine * aqq;
    matrix[p][q] = 0.0;
    matrix[q][p] = 0.0;

    for row in eigenvectors.iter_mut() {
        let vip = row[p];
        let viq = row[q];
        row[p] = cosine * vip - sine * viq;
        row[q] = sine * vip + cosine * viq;
    }
}

fn sort_pairs_descending(pairs: &mut [(f32, [f32; 3]); 3]) {
    if pairs[1].0 > pairs[0].0 {
        pairs.swap(0, 1);
    }
    if pairs[2].0 > pairs[1].0 {
        pairs.swap(1, 2);
    }
    if pairs[1].0 > pairs[0].0 {
        pairs.swap(0, 1);
    }
}

#[cfg(feature = "std")]
fn sqrt(value: f32) -> f32 {
    value.sqrt()
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
fn sqrt(value: f32) -> f32 {
    libm::sqrtf(value)
}

#[cfg(test)]
mod tests {
    use super::{PlaneConstraint, QefBounds, QefParams, QefSolveError, QefSolver, SharpnessClass};

    fn bounds() -> QefBounds {
        QefBounds::new([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]).expect("valid bounds")
    }

    #[test]
    fn planar_constraints_classify_as_smooth() {
        let mut solver = QefSolver::new();
        solver.add([0.0, -0.25, 0.3], [0.0, 1.0, 0.0]);
        solver.add([0.4, -0.25, -0.6], [0.0, 1.0, 0.0]);
        solver.add([-0.3, -0.25, 0.1], [0.0, 1.0, 0.0]);

        let result = solver
            .solve(bounds(), &QefParams::default())
            .expect("planar solve should succeed");

        assert_eq!(result.rank, 1);
        assert_eq!(result.sharpness_class, SharpnessClass::Smooth);
        assert!((result.position[1] + 0.25).abs() <= 1.0e-4);
        assert!(result.residual_error <= 1.0e-4);
    }

    #[test]
    fn orthogonal_planes_classify_as_edge() {
        let mut solver = QefSolver::new();
        solver.add([0.25, 0.0, 0.0], [1.0, 0.0, 0.0]);
        solver.add([0.0, -0.5, 0.0], [0.0, 1.0, 0.0]);

        let result = solver
            .solve(bounds(), &QefParams::default())
            .expect("edge solve should succeed");

        assert_eq!(result.rank, 2);
        assert_eq!(result.sharpness_class, SharpnessClass::Edge);
        assert!((result.position[0] - 0.25).abs() <= 1.0e-4);
        assert!((result.position[1] + 0.5).abs() <= 1.0e-4);
        assert!(result.position[2].abs() <= 1.0e-4);
    }

    #[test]
    fn three_planes_classify_as_corner() {
        let mut solver = QefSolver::new();
        solver.add([0.25, 0.0, 0.0], [1.0, 0.0, 0.0]);
        solver.add([0.0, -0.5, 0.0], [0.0, 1.0, 0.0]);
        solver.add([0.0, 0.0, 0.75], [0.0, 0.0, 1.0]);

        let result = solver
            .solve(bounds(), &QefParams::default())
            .expect("corner solve should succeed");

        assert_eq!(result.rank, 3);
        assert_eq!(result.sharpness_class, SharpnessClass::Corner);
        assert!((result.position[0] - 0.25).abs() <= 1.0e-4);
        assert!((result.position[1] + 0.5).abs() <= 1.0e-4);
        assert!((result.position[2] - 0.75).abs() <= 1.0e-4);
        assert!(result.residual_error <= 1.0e-4);
    }

    #[test]
    fn empty_solver_is_rejected() {
        let error = QefSolver::new()
            .solve(bounds(), &QefParams::default())
            .expect_err("empty solve should fail");
        assert_eq!(error, QefSolveError::Empty);
    }

    #[test]
    fn single_constraint_stays_on_anchor_in_null_space() {
        let mut solver = QefSolver::new();
        solver.add([0.0, 0.0, 0.25], [0.0, 0.0, 1.0]);

        let result = solver
            .solve(bounds(), &QefParams::default())
            .expect("single plane solve should succeed");

        assert_eq!(result.rank, 1);
        assert!((result.position[0] - bounds().center()[0]).abs() <= 1.0e-5);
        assert!((result.position[1] - bounds().center()[1]).abs() <= 1.0e-5);
        assert!((result.position[2] - 0.25).abs() <= 1.0e-5);
    }

    #[test]
    fn single_constraint_respects_custom_anchor_in_null_space() {
        let mut solver = QefSolver::new();
        solver.add([0.0, 0.0, 0.25], [0.0, 0.0, 1.0]);

        let result = solver
            .solve_with_anchor(bounds(), [0.4, -0.3, 0.0], &QefParams::default())
            .expect("single plane solve should succeed");

        assert_eq!(result.rank, 1);
        assert!((result.position[0] - 0.4).abs() <= 1.0e-5);
        assert!((result.position[1] + 0.3).abs() <= 1.0e-5);
        assert!((result.position[2] - 0.25).abs() <= 1.0e-5);
    }

    #[test]
    fn collinear_normals_remain_low_rank() {
        let mut solver = QefSolver::new();
        solver.add([0.3, 0.0, 0.0], [2.0, 0.0, 0.0]);
        solver.add([0.3, 0.2, -0.1], [4.0, 0.0, 0.0]);

        let result = solver
            .solve(bounds(), &QefParams::default())
            .expect("collinear normals should still solve");

        assert_eq!(result.rank, 1);
        assert_eq!(result.sharpness_class, SharpnessClass::Smooth);
        assert!((result.position[0] - 0.3).abs() <= 1.0e-4);
    }

    #[test]
    fn solve_clamps_outside_solution_into_bounds() {
        let mut solver = QefSolver::new();
        solver.add([2.5, 0.0, 0.0], [1.0, 0.0, 0.0]);

        let result = solver
            .solve(bounds(), &QefParams::default())
            .expect("solve should succeed");

        assert!(result.was_clamped);
        assert!((result.position[0] - 1.0).abs() <= 1.0e-5);
    }

    #[test]
    fn add_constraints_skips_invalid_normals() {
        let mut solver = QefSolver::new();
        solver.add_constraints(&[
            PlaneConstraint {
                position: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, 0.0],
            },
            PlaneConstraint {
                position: [0.1, 0.0, 0.0],
                normal: [1.0, 0.0, 0.0],
            },
        ]);

        assert_eq!(solver.constraint_count(), 1);
    }
}
