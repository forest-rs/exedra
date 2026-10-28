// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Retained f64 analytic geometry and shared boundary topology.
//!
//! [`geometry`] owns checked curves, surfaces and parameter domains. [`body`]
//! owns bounded edges and oriented face loops independently of construction
//! history. [`extrusion::Extrusion`], [`revolve::Revolution`], [`sweep::Sweep`],
//! [`loft::Loft`], [`bore::BoredPlate`] and [`AnalyticShell`] construct that model
//! while preserving source roles and explicit mesh realization correspondence.
//! Geometry, surface parameters and texture coordinates remain distinct.
//!
//! This is a bounded analytic foundation, not a general CAD Boolean or healing
//! kernel. Body validation checks incidence and analytic support agreement;
//! it does not certify global self-intersection or shell containment.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_analytic requires either the `std` or `libm` feature");

pub mod body;
pub mod bore;
pub mod bounds;
pub mod extrusion;
pub mod geometry;
pub mod loft;
pub mod machining;
mod planar;
mod profile_geometry;
pub mod realize;
pub mod revolve;
pub mod sweep;
pub use planar::{
    AnalyticFaceId, AnalyticLoopId, AnalyticShell, AnalyticShellBuilder, AnalyticVertexId,
    BuildError, EditError, PlanarFace, RectFrameParams, RectOpeningParams, RegionId,
    TessellateError, TessellateParams, TessellatedShell, rect_frame_xy,
};

/// One end of a profile construction's parameter span.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum End {
    /// The generating profile at the start of the construction.
    Start,
    /// The generating profile at the end of the construction.
    End,
}

fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("analytic table exceeds u32 local ID range")
}
