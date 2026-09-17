// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal timing helpers shared across runtime modules.

#[cfg(any(target_arch = "wasm32", feature = "std"))]
pub(crate) fn duration_nanos_u64(duration: core::time::Duration) -> u64 {
    duration.as_nanos().try_into().unwrap_or(u64::MAX)
}
