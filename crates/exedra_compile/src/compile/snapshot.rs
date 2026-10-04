// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{CompiledParts, PartFingerprint, PolicyFingerprint};
use crate::{PartId, PartInput};

/// Compiled geometry does not correspond to an input list's current part records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompilationMismatch {
    /// The input list and compilation have different part counts.
    PartCount {
        /// Number of parts registered in the input list.
        expected: usize,
        /// Number of compiled entries supplied.
        actual: usize,
    },
    /// A local handle refers to different source geometry.
    PartContent {
        /// The mismatched input list-local handle.
        part: PartId,
        /// Fingerprint of the current input list part source.
        expected: PartFingerprint,
        /// Fingerprint of the supplied compiled entry.
        actual: PartFingerprint,
    },
}

impl core::fmt::Display for CompilationMismatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PartCount { expected, actual } => write!(
                f,
                "input list has {expected} parts but compilation has {actual}"
            ),
            Self::PartContent { part, .. } => {
                write!(
                    f,
                    "compiled content does not match input list part {part:?}"
                )
            }
        }
    }
}

impl core::error::Error for CompilationMismatch {}

impl CompiledParts {
    /// Identity of the evaluation and extraction policy used for this snapshot.
    #[must_use]
    pub fn policy_fingerprint(&self) -> PolicyFingerprint {
        self.policy
    }

    /// Checks that every compiled entry corresponds to the input list's part source.
    ///
    /// Placements, frame instances, keys, metadata and material bindings may change
    /// without invalidating geometry. Reordered handles or replaced source content
    /// require a matching compilation. The supplied snapshot determines the policy;
    /// callers can compare [`Self::policy_fingerprint`] with their intended policy.
    ///
    /// This compares identities captured by immutable inputs. It does not rescan
    /// baked attributes, tessellate or measure placed bounds. Use it at snapshot/export
    /// boundaries; reading local poses directly needs no geometry validation walk.
    /// Geometry reports remain accessible through [`Self::report`], including reports
    /// for partial evaluations. Matching content alone does not certify completeness.
    ///
    /// # Errors
    ///
    /// Reports a part-count mismatch first, then the first mismatched part source.
    pub fn validate_for(&self, inputs: &[PartInput<'_>]) -> Result<(), CompilationMismatch> {
        if inputs.len() != self.parts.len() {
            return Err(CompilationMismatch::PartCount {
                expected: inputs.len(),
                actual: self.parts.len(),
            });
        }
        for (index, (def, compiled)) in inputs.iter().zip(self.parts.iter()).enumerate() {
            let expected = def.fingerprint();
            if expected != compiled.fingerprint {
                return Err(CompilationMismatch::PartContent {
                    part: PartId(crate::len_u32(index)),
                    expected,
                    actual: compiled.fingerprint,
                });
            }
        }
        Ok(())
    }
}
