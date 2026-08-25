// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Trenched crossings in the secondary roof frame.
//!
//! These are two rules, not one configurable crossing notch. A purlin carried
//! by a principal rafter leaves the purlin whole and trenches the principal;
//! a common rafter carried by a purlin leaves the purlin whole and seats the
//! common rafter. The distinction fixes edit ownership, load direction, role
//! selection, and relish checks even though both cuts share analytic crossing
//! footprint math.

mod crossing;
mod principal_trench;
mod rafter_seat;

pub use principal_trench::{
    PURLIN_PRINCIPAL_TRENCH_RULE_KEY, PurlinPrincipalTrenchParams, PurlinToPrincipalTrenchRule,
};
pub use rafter_seat::{
    COMMON_RAFTER_PURLIN_SEAT_RULE_KEY, CommonRafterPurlinSeatParams, CommonRafterToPurlinSeatRule,
};

#[cfg(test)]
mod tests;
