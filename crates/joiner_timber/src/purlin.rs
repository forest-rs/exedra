// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Trenched crossings in the secondary roof frame.
//!
//! These are named rules with explicit receivers. A purlin carried
//! by a principal rafter leaves the purlin whole and trenches the principal;
//! a common rafter carried by a purlin leaves the purlin whole and seats the
//! common rafter. The distinction fixes edit ownership, load direction, role
//! selection, and relish checks even though both cuts share analytic crossing
//! footprint math. A circular purlin instead receives its own flat underside
//! seat; that contact uses a circle chord rather than a full-section width.

mod crossing;
mod principal_trench;
mod rafter_seat;
mod round_seat;

pub use principal_trench::{
    PURLIN_PRINCIPAL_TRENCH_RULE_KEY, PurlinPrincipalTrenchParams, PurlinToPrincipalTrenchRule,
};
pub use rafter_seat::{
    COMMON_RAFTER_PURLIN_SEAT_RULE_KEY, CommonRafterPurlinSeatParams, CommonRafterToPurlinSeatRule,
};

pub use round_seat::{ROUND_PURLIN_SEAT_RULE_KEY, RoundPurlinSeatParams, RoundPurlinSeatRule};

#[cfg(test)]
mod tests;
