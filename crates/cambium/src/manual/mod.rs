// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cambium Manual
//!
//! Documentation-only manual modules for authoring and operating Cambium APIs.
//! This module is built only for docs (`cfg(doc)`).
//!
//! Current sections:
//! - [`operators`]: implementing deterministic edit operations and reports.
//! - [`selections`]: canonical face/edge selection semantics.
//! - [`diagnostics`]: diagnostic and error classification conventions.
//! - [`reporting`]: timings/stats/artifact reporting patterns.

pub mod diagnostics;
pub mod operators;
pub mod reporting;
pub mod selections;
