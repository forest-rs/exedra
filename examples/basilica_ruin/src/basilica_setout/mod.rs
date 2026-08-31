// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact premises and resolved setting-out for the basilica example.

mod network;
mod premises;
mod sections;

pub use network::{BasilicaReconfiguration, BasilicaSetout, BasilicaSetoutError};
pub use premises::BasilicaPremises;
pub use sections::{
    AisleSection, CrossingSection, EastEndSection, LevelSection, PlanSection, RoofSection, RoofSide,
};
