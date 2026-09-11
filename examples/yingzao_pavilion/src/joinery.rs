// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Common construction authoring for the pavilion's explicit joint families.

use joiner::{Construction, Element, Evidence, EvidenceClass, EvidenceSource, Member, Node};

use crate::Result;

pub(crate) fn construction() -> Result<(Construction, Evidence)> {
    let mut construction = Construction::new();
    let evidence = Evidence::new("pavilion-design", EvidenceClass::ModernEngineeringInference);
    construction.add_evidence_source(EvidenceSource::new("pavilion-design", evidence.class,
        "https://link.springer.com/chapter/10.1007/978-3-031-81623-9_24",
        "Modern illustration; module and roof method informed by Yingzao Fashi, fits authored for this scene"))?;
    Ok((construction, evidence))
}

pub(crate) fn member(construction: &mut Construction, element: Element) -> Result<()> {
    let key = &element.key;
    let extent = &element.extent;
    let start = format!("{key}-start");
    let end = format!("{key}-end");
    for (name, station) in [(&start, 0.0), (&end, extent.size[0])] {
        construction.add_node(Node::new(
            name,
            extent.anchor([station, extent.size[1] * 0.5, extent.size[2] * 0.5]),
        ))?;
    }
    let member = Member::new(key, key, &start, &end, element.evidence.clone());
    construction.add_element(element.with_member())?;
    construction.add_member(member)?;
    Ok(())
}
