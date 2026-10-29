// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Line and circular-arc 2D profiles shared by Exedra geometry heads.
//!
//! A [`Profile`] is a counter-clockwise outer [`Loop`] with clockwise holes,
//! in `f64` model units. Segments are straight or circular arcs given by their
//! endpoints and either a bulge or an authoritative center/direction, so loops
//! close exactly; [`Arc::resolve`] derives the remaining circle parameters. Nothing here refers to recipes, policies or
//! meshes: heads lower their own profiles into this one through explicit
//! adapters.
//!
//! [`Profile::new`] validates the arrangement against an explicit
//! [`ProfilePolicy`]: winding, segments that cross, touch, overlap or come
//! closer than the separation, holes outside the outer loop or inside one
//! another. It reports the first violation with the segments involved and
//! never moves, merges or heals geometry.
//!
//! The crate also owns deterministic circular sampling counts:
//! [`circular_edge_count`] returns the smallest edge count meeting a chord
//! tolerance under explicit topology and work limits. Trig always routes
//! through `libm`, so results are bit-identical on every platform and build
//! mode.
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
extern crate alloc;

mod arc;
mod corner;
mod placement;
mod point_query;
mod profile;
mod region;
mod sampling;

mod input;
pub use input::IntoProfile;

pub use arc::{Arc, ArcError};
pub use corner::{
    Corner, CornerError, CornerKind, CornerResult, CornerSegment, CornerSelectionError,
    CornerShape, CornerSource, CornerTreatment,
};
pub use placement::ProfileTranslationError;
pub use point_query::{LoopPointQuery, PointQueryError};
pub use profile::{
    Curve, Loop, LoopError, LoopRef, Profile, ProfileError, ProfilePolicy, SegTag, Segment,
    SegmentRef,
};
pub use region::{
    InputSegment, Operand, RegionBudget, RegionError, RegionOp, RegionPolicy, RegionReport,
    RegionResult, RegionSegment, SegmentSource,
};
pub use sampling::{CircularEdgeConstraints, CircularSamplingError, circular_edge_count};
