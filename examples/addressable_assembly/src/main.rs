// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runs the Addressable assembly tour and prints its stable milestones.

use addressable_assembly::run_tour;

fn main() {
    let tour = run_tour();
    println!("selected {} ({})", tour.address, tour.part);
    println!(
        "material {} ({:?}; {} authored opinions)",
        tour.material, tour.reason, tour.material_opinions
    );
}
