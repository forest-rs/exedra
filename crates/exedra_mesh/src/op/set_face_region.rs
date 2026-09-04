// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::{ChangeSink, EditSession, FaceId};

/// Structured failure from [`set_face_region`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SetFaceRegionError {
    /// Face ID is stale or OUTSIDE.
    FaceNotLive {
        /// Numeric face slot index from the rejected ID.
        face: u32,
    },
}

impl fmt::Display for SetFaceRegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FaceNotLive { face } => write!(f, "face is not live: {face}"),
        }
    }
}

impl core::error::Error for SetFaceRegionError {}

/// Writes the built-in face-region layer.
pub fn set_face_region<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    face: FaceId,
    region: u32,
) -> Result<(), SetFaceRegionError> {
    if session.set_face_region_impl(face, region) {
        Ok(())
    } else {
        Err(SetFaceRegionError::FaceNotLive { face: face.index() })
    }
}
