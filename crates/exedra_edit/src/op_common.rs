// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared internal helpers for operator implementations.

use alloc::string::String;
use alloc::vec;

use crate::{Artifacts, DiagCode, DiagLevel, Diagnostic, OpContext, OpError, OpErrorKind};

pub(crate) fn op_error(
    ctx: &OpContext,
    kind: OpErrorKind,
    code: DiagCode,
    message: impl Into<String>,
) -> OpError {
    OpError::new(
        kind,
        vec![Diagnostic::new(DiagLevel::Error, code, message)],
        Artifacts::new(
            ctx.policy.limits.max_artifact_items,
            ctx.policy.limits.max_artifact_bytes,
        ),
    )
}
