// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Line and circular-arc 2D profiles shared by Exedra geometry heads.
//!
//! The crate owns deterministic circular sampling counts: the smallest edge
//! count meeting a chord tolerance under explicit topology and work limits.
//! Trig always routes through `libm`, so results are bit-identical on every
//! platform and build mode.
//!
//! ```
//! use exedra_profile::{CircularEdgeConstraints, circular_edge_count};
//!
//! let constraints = CircularEdgeConstraints::new(8, 4096).with_edge_multiple(4);
//! let edges = circular_edge_count(25.0, core::f64::consts::TAU, 0.01, constraints)?;
//! assert_eq!(edges % 4, 0);
//! # Ok::<(), exedra_profile::CircularSamplingError>(())
//! ```

#![no_std]

mod sampling;

pub use sampling::{CircularEdgeConstraints, CircularSamplingError, circular_edge_count};
