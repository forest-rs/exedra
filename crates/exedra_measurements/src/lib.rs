// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact physical measurement values.
//!
//! [`Length`] represents a strictly positive physical size, while [`Offset`]
//! represents a signed displacement and may be zero. [`Angle`] represents a
//! nonnegative angular magnitude, while [`AngularOffset`] is signed. Linear
//! values store joto iotas (one ninth of a nanometer); angular values store
//! microarcseconds. Callers can therefore preserve exact authored values until
//! an explicit floating-point geometry boundary.
//!
//! This crate deliberately does not parse, format, or infer units, and it
//! does not own user-input quantization policy or geometry types.
//!
//! # Example
//!
//! ```
//! use exedra_measurements::{Angle, Length, Offset};
//!
//! let width = Length::millimeters(120).expect("a positive exact width");
//! let setback = Offset::millimeters(-25).expect("an exact signed setback");
//! let bevel = Angle::degrees(45).expect("an exact angle");
//!
//! assert_eq!(width.as_meters(), 0.12);
//! assert_eq!(setback.as_meters(), -0.025);
//! assert_eq!(bevel.as_degrees(), 45.0);
//! ```

#![no_std]

mod angle;
mod length;

pub use angle::{Angle, AngularOffset};
pub use length::{Length, Offset};
