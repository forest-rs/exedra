// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra Edit Manual
//!
//! Documentation-only manual modules for authoring and operating Exedra Edit APIs.
//! This module is built only for docs (`cfg(doc)`).
//!
//! Current sections:
//! - [`mod@reference`]: operator reference (stable names, params, outputs).
//! - [`operators`]: implementing deterministic edit operations and reports.
//! - [`selections`]: canonical face/edge selection semantics.
//! - [`diagnostics`]: diagnostic and error classification conventions.
//! - [`reporting`]: timings/stats/artifact reporting patterns.
//! - [`surface`]: operation ownership.

pub mod diagnostics;
pub mod operators;
pub mod reference;
pub mod reporting;
pub mod selections;
pub mod surface;
