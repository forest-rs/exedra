// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reconstruction evidence attached to the exact basilica network.

use setout::{PropagationPlan, RootClaimSet};
use setout_reconstruction::{
    DerivationCharacter, MethodWarrant, ReconstructionCatalogue, ReconstructionCatalogueBuilder,
    SourceBasis, SourceRef,
};

use super::BasilicaSetoutError;

pub(super) fn build_catalogue(
    roots: &RootClaimSet,
    plan: &PropagationPlan,
) -> Result<ReconstructionCatalogue, BasilicaSetoutError> {
    let mut catalogue = ReconstructionCatalogueBuilder::new();
    // Catalogue every active root, including exact architectural constants.
    // A partial catalogue would make the whole-building network look more
    // evidentially complete than it is by hiding unannotated new premises.
    for key in roots.keys() {
        catalogue = catalogue.root(key.clone(), source_for_root(key.as_str()))?;
    }
    for step in plan.steps() {
        catalogue = catalogue.method(
            step.relation.clone(),
            step.method.clone(),
            MethodWarrant::new(
                "basilica/exact-setting-out",
                DerivationCharacter::Transparent,
                "exact arithmetic or integer-root setting-out",
            ),
        )?;
    }
    Ok(catalogue.finish())
}

fn source_for_root(key: &str) -> SourceRef {
    match key {
        "basilica/root/roof-zero" => SourceRef::new(
            "basilica-local-coordinate-datum",
            SourceBasis::Observed,
            "basilica local coordinate convention",
            "coordinate convention, not historical fabric",
        ),
        "basilica/root/length"
        | "basilica/root/nave-span"
        | "basilica/root/total-width"
        | "basilica/root/crossing-station"
        | "basilica/root/arcade-bays"
        | "basilica/root/nave-wall-height"
        | "basilica/root/aisle-wall-height"
        | "basilica/root/drum-radius"
        | "basilica/root/drum-height"
        | "basilica/root/dome-height"
        | "basilica/root/apse-radius" => SourceRef::new(
            "accepted-basilica-massing",
            SourceBasis::Documented,
            "accepted basilica gallery massing",
            "working reconstruction dimension, not a new survey claim",
        ),
        "basilica/root/roof-rise"
        | "basilica/root/wall-plate-height"
        | "basilica/root/wall-plate-width" => SourceRef::new(
            "hagia-paraskevi-roof-survey",
            SourceBasis::RegionalAnalogy,
            "regional roof survey analogy",
            "analogy supplies construction proportions, not direct evidence for this building",
        ),
        "basilica/root/roof-overhang" => SourceRef::new(
            "regional-tiled-roof-practice",
            SourceBasis::RegionalAnalogy,
            "regional tiled-roof practice",
            "eave projection is not directly observed",
        ),
        _ => SourceRef::new(
            "accepted-basilica-detailing",
            SourceBasis::ModernInference,
            "accepted basilica gallery detailing",
            "explicit modeling or legibility choice, not historical evidence",
        ),
    }
}
