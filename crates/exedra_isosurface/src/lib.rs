// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Surface meshes extracted from implicit fields.
//!
//! Fields are defined in [`exedra_implicit`]; this crate turns their zero
//! sets into meshes. [`delaunay_mesh`] meshes a field by restricted Delaunay
//! refinement: near-isotropic triangles close to the surface, sharp along
//! its creases and corners, and closed into a manifold around every vertex,
//! optionally labelled with the regions of the parts they lie on.
//!
//! # Example
//!
//! Mesh a box drilled through, with sharp edges where the bore leaves it:
//!
//! ```
//! use exedra_implicit::{Aabb, Cuboid, Cylinder, Difference};
//! use exedra_isosurface::{DelaunayParams, delaunay_mesh};
//!
//! let drilled = Difference::new(
//!     Cuboid::new([0.0; 3], [0.6; 3]).unwrap(),
//!     Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.3, 1.0).unwrap(),
//! );
//! let bounds = Aabb::new([-1.0; 3], [1.0; 3]).unwrap();
//! let result = delaunay_mesh(&drilled, &DelaunayParams::new(bounds, 0.125)).unwrap();
//!
//! assert!(result.report.is_complete());
//! assert_eq!(result.report.corners, 8);
//! let mesh = result.mesh().expect("a closed manifold");
//! assert!(mesh.validate_deep().is_empty());
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_isosurface requires either the `std` or `libm` feature");

pub mod delaunay;

pub use delaunay::{
    CreaseParams, DelaunayError, DelaunayParams, DelaunayReport, DelaunayResult, FieldWork,
    Refinements, delaunay_mesh,
};
