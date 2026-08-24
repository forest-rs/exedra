// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Audited exact defaults for timber-rule parameters.

use exedra_measurements::Length;

/// Builds one audited compile-time default from whole millimeters.
pub(crate) const fn default_millimeters(value: u64) -> Length {
    match Length::millimeters(value) {
        Some(value) => value,
        None => panic!("timber-rule default must be a positive millimeter length"),
    }
}

/// Builds one audited compile-time default from whole micrometers.
pub(crate) const fn default_micrometers(value: u64) -> Length {
    match Length::micrometers(value) {
        Some(value) => value,
        None => panic!("timber-rule default must be a positive micrometer length"),
    }
}
