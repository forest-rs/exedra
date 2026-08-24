// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Concrete timber fitting rules for [`joiner`].
//!
//! This crate owns timber joint knowledge and the coordinated recipes that
//! realize it. It explicitly does not own the construction graph, generic
//! rule mechanism, constructive or Boolean algorithms, assembly diagnostics,
//! rendering, statics, connection capacity, or historical classification.
//!
//! ## One authored interface
//!
//! Every rule derives its mating geometry from one carried-member section.
//! Only receiving geometry uses [`FitClass::Clearance`]. Mortises and housings
//! use [`exedra_constructive::profile::Profile2::offset`]; the keyed slot keeps
//! its load-bearing faces line-to-line and applies clearance only transverse
//! to the key. The housed heel also shapes its carried rafter; a keyed
//! king-post tie fit shapes the post, mortises the tie, and generates the key;
//! full-section compression bearings cut only their carrier because the strut
//! or rafter already terminates on the authored internal shoulder.
//!
//! ```text
//! relation + extents
//!     -> nominal interface
//!        |-> carried-side shape, when the joint needs one
//!        `-> receiver cut (nominal + typed fit allowance)
//!     -> joiner::RuleOutput
//! ```
//!
//! Member extents follow `joiner`'s existing box convention: local x is the
//! member direction, while local y/z span its section. Each rule documents
//! whether its relation node is a shoulder or an endpoint. Housed bearings
//! accept either member end and derive the outward cutter direction from that
//! explicit endpoint, rather than participant ordering.
//!
//! Rule dimensions use exact, strictly positive [`Length`] values. Each
//! parameter family lowers those values together only when it reaches the
//! floating-point recipe-building boundary; coordinates and derived geometry
//! remain the responsibility of `joiner` and the geometry crates.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod fit;
mod heel;
mod king_post;
mod length;
mod participants;
mod strut;
mod tool;

pub use exedra_measurements::Length;
pub use fit::FitClass;
pub use heel::{HEEL_RULE_KEY, HeelParams, HeelRule};
pub use king_post::{KING_POST_TIE_RULE_KEY, KingPostTieParams, KingPostTieRule};
pub use strut::{
    HousedBearingParams, RAFTER_KING_POST_RULE_KEY, RafterToKingPostRule, STRUT_KING_POST_RULE_KEY,
    STRUT_RAFTER_RULE_KEY, StrutToKingPostRule, StrutToRafterRule,
};
