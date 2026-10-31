// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Scratch storage reused across calls.

use alloc::vec::Vec;

/// Scratch storage for the crate's `_into` functions.
///
/// A caller that evaluates many curves, or the same curves every frame,
/// keeps one `Workspace` and its output buffers and passes them to each
/// call: once they have grown to the largest curve's needs, nothing more
/// is allocated. The workspace holds no results; each call overwrites the
/// parts it uses and any call may grow it. The allocating functions use
/// fresh scratch of their own.
#[derive(Clone, Debug, Default)]
pub struct Workspace {
    /// Homogeneous NURBS controls for de Boor's scheme and blossoming.
    pub(crate) homogeneous: Vec<[f64; 4]>,
    /// Chord directions of a framed curve, `None` where points repeat.
    pub(crate) directions: Vec<Option<[f64; 3]>>,
    /// The first direction leaving each framed point.
    pub(crate) outgoing: Vec<Option<[f64; 3]>>,
}

impl Workspace {
    /// An empty workspace; it grows on first use.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            homogeneous: Vec::new(),
            directions: Vec::new(),
            outgoing: Vec::new(),
        }
    }
}
