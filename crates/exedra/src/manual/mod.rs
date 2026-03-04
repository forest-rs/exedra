// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra Manual
//!
//! Documentation-only manual modules for Exedra architecture and usage.
//! This module is built only for docs (`cfg(doc)`).
//!
//! Current sections:
//! - [`model`]: half-edge model, OUTSIDE face semantics, IDs/corners.
//! - [`transactions`]: eager transaction behavior and change summaries.
//! - [`attributes`]: dense/sparse layers, domains, and built-ins.
//! - [`construction`]: mesh construction pathways and error semantics.
//! - [`validation`]: `validate_fast` vs `validate_deep` usage.
//! - [`render`]: deterministic render extraction and split behavior.

pub mod attributes;
pub mod construction;
pub mod model;
pub mod render;
pub mod transactions;
pub mod validation;
