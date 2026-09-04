// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Canonical region IDs for worked-example scenarios.

/// Untagged/default region.
///
/// Must stay aligned with `exedra_ops::REGION_UNTAGGED`.
pub const REGION_UNTAGGED: u32 = 0;
/// Worked-example footprint tag.
pub const REGION_FOOTPRINT: u32 = 1;
/// Worked-example outer wall tag.
pub const REGION_WALL_OUTER: u32 = 2;
/// Worked-example inner wall tag.
pub const REGION_WALL_INNER: u32 = 3;
/// Worked-example nave tag.
pub const REGION_NAVE: u32 = 4;
/// Worked-example left aisle tag.
pub const REGION_AISLE_L: u32 = 5;
/// Worked-example right aisle tag.
pub const REGION_AISLE_R: u32 = 6;
/// Worked-example apse tag.
pub const REGION_APSE: u32 = 7;
/// Worked-example drum tag.
pub const REGION_DRUM: u32 = 8;
/// Worked-example dome tag.
pub const REGION_DOME: u32 = 9;
