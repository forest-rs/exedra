// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Error types for the Fidget adapter layer.

use core::fmt;

/// Errors returned when constructing an [`crate::FidgetField`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FidgetFieldError {
    /// The wrapped shape depends on non-axis variables that cannot be supplied
    /// through the `ScalarField` seam.
    ExtraVariables {
        /// Number of unsupported extra variables.
        count: usize,
    },
}

impl fmt::Display for FidgetFieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExtraVariables { count } => {
                write!(
                    f,
                    "fidget shape depends on {count} extra variable(s) beyond x/y/z"
                )
            }
        }
    }
}

impl std::error::Error for FidgetFieldError {}
