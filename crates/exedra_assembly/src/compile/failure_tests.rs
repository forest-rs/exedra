// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::format;
use alloc::string::ToString;

use crate::{Assembly, CompileError, CompilePolicy, PartCompiler};
use exedra_constructive::builders;
use exedra_constructive::evaluate::evaluate;
use exedra_constructive::ir::{
    CapMode, LoftPolicy, LoftSection, NodeKind, Placement3, Recipe, RecipeBuilder,
};
use exedra_constructive::loft::LoftError;
use exedra_constructive::tessellate::{EvalPolicy, TessellateError};

fn vessel(labeled: bool, transform: Placement3, unused_source: bool) -> Recipe {
    let mut builder = RecipeBuilder::new();
    if unused_source {
        builder.source_ref("unrelated:source");
    }
    let stations = [
        (0.0, 0.13),
        (0.06, 0.15),
        (0.20, 0.25),
        (0.43, 0.23),
        (0.60, 0.11),
        (0.64, 0.12),
    ];
    let sections = stations
        .into_iter()
        .enumerate()
        .map(|(i, (height, radius))| {
            let profile = builder.add_profile(builders::circle(radius).unwrap());
            let section = LoftSection::new(Placement3::translate(0.0, 0.0, height), profile);
            if labeled {
                section.with_source(builder.source_ref(&format!("vessel/station-{i}")))
            } else {
                section
            }
        })
        .collect();
    if labeled {
        let source = builder.source_ref("vessel/body");
        builder.with_source(source);
    }
    let loft = builder
        .add(NodeKind::Loft {
            sections,
            policy: LoftPolicy::Smooth,
            caps: CapMode::Both,
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Transform {
            child: loft,
            xf: transform,
        })
        .unwrap();
    builder.finish(root).unwrap()
}

#[test]
fn vessel_failure_retains_authored_context_and_actual_control_geometry() {
    let recipe = vessel(true, Placement3::translate(10.0, 20.0, 30.0), false);
    let mut assembly = Assembly::new();
    let part = assembly.add_recipe_part("storage-jar", recipe).unwrap();
    let error = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap_err();
    drop(assembly);
    let display = error.to_string();
    for label in [
        "storage-jar",
        "vessel/body",
        "vessel/station-4",
        "vessel/station-5",
        "tag 0",
    ] {
        assert!(display.contains(label), "{display}");
    }
    let CompileError::Evaluate {
        part: failed_part,
        part_key,
        error,
    } = error
    else {
        panic!("expected evaluation failure")
    };
    assert_eq!(failed_part, part);
    assert_eq!(part_key, "storage-jar");
    assert_eq!(error.source.as_deref(), Some("vessel/body"));
    let sections = error.loft_sections.unwrap();
    assert_eq!(sections.each_ref().map(|s| s.index), [4, 5]);
    assert_eq!(sections[0].source.as_deref(), Some("vessel/station-4"));
    assert_eq!(sections[1].source.as_deref(), Some("vessel/station-5"));
    let TessellateError::Loft(LoftError::Foldover {
        band,
        vertex,
        witness,
    }) = error.error
    else {
        panic!("expected foldover evidence")
    };
    assert_eq!((band, vertex, witness.control_edge), (4, 0, 1));
    for (point, expected) in [
        (witness.trajectory[0], [9.89, 20.0, 30.60]),
        (witness.trajectory[3], [9.88, 20.0, 30.64]),
    ] {
        for i in 0..3 {
            assert!((point[i] - expected[i]).abs() < 1e-12);
        }
    }
    let c = witness.trajectory;
    let k = witness.control_edge;
    let advance: f64 = (0..3)
        .map(|i| (c[k + 1][i] - c[k][i]) * (c[3][i] - c[0][i]))
        .sum();
    assert_eq!(witness.advance, advance);
    assert!(advance < 0.0);
    for point in witness.profile_points.unwrap() {
        assert_eq!(point.sampled.hole, None);
        assert_eq!(point.segment, 0);
        assert_eq!(point.tag, Some(exedra_constructive::profile::SegTag(0)));
    }
}

#[test]
fn unlabeled_context_is_explicit_and_text_round_trip_preserves_labels() {
    let unlabeled = vessel(false, Placement3::IDENTITY, false);
    let error = evaluate(&unlabeled, &EvalPolicy::default()).unwrap_err();
    assert_eq!(error.source, None);
    assert!(
        error
            .loft_sections
            .unwrap()
            .iter()
            .all(|s| s.source.is_none())
    );
    let labeled = vessel(true, Placement3::IDENTITY, false);
    assert_ne!(unlabeled.recipe_fingerprint(), labeled.recipe_fingerprint());
    let reordered = vessel(true, Placement3::IDENTITY, true);
    assert_eq!(labeled.recipe_fingerprint(), reordered.recipe_fingerprint());
    let text = exedra_constructive::text::dump_recipe(&labeled);
    let parsed = exedra_constructive::text::parse_recipe(&text).unwrap();
    assert_eq!(labeled.recipe_fingerprint(), parsed.recipe_fingerprint());
    assert_eq!(
        evaluate(&labeled, &EvalPolicy::default()).unwrap_err(),
        evaluate(&parsed, &EvalPolicy::default()).unwrap_err()
    );
    let expected = evaluate(&labeled, &EvalPolicy::default()).unwrap_err();
    let mut cache = exedra_constructive::cache::EvalCache::default();
    for _ in 0..2 {
        assert_eq!(
            exedra_constructive::evaluate::evaluate_with_cache(
                &labeled,
                &EvalPolicy::default(),
                &mut cache
            )
            .unwrap_err(),
            expected,
        );
    }
}

#[cfg(feature = "serde")]
#[test]
fn section_sources_round_trip_and_invalid_bindings_are_refused() {
    use exedra_constructive::interchange::{self, NodeKindDto};
    let recipe = vessel(true, Placement3::IDENTITY, false);
    let mut dto = interchange::to_dto(&recipe);
    let serialized = serde_json::to_string(&dto).unwrap();
    let parsed = interchange::from_dto(&serde_json::from_str(&serialized).unwrap()).unwrap();
    assert_eq!(recipe.recipe_fingerprint(), parsed.recipe_fingerprint());
    let expected = evaluate(&recipe, &EvalPolicy::default()).unwrap_err();
    assert_eq!(
        expected,
        evaluate(&parsed, &EvalPolicy::default()).unwrap_err()
    );
    // Change section provenance alone: it participates in identity even when
    // the node label and geometry remain identical.
    let NodeKindDto::Loft {
        section_sources, ..
    } = &mut dto.nodes[0].kind
    else {
        panic!("loft")
    };
    section_sources[0] = None;
    let mixed = interchange::from_dto(&dto).unwrap();
    assert_ne!(mixed.recipe_fingerprint(), recipe.recipe_fingerprint());
    let encoded = serde_json::to_string(&interchange::to_dto(&mixed)).unwrap();
    let decoded = interchange::from_dto(&serde_json::from_str(&encoded).unwrap()).unwrap();
    assert_eq!(mixed.recipe_fingerprint(), decoded.recipe_fingerprint());
    let NodeKindDto::Loft {
        section_sources, ..
    } = &mut dto.nodes[0].kind
    else {
        panic!("loft")
    };
    section_sources.pop();
    assert!(matches!(
        interchange::from_dto(&dto),
        Err(interchange::InterchangeError::Recipe(
            exedra_constructive::ir::RecipeError::InvalidParameter {
                what: "loft section sources"
            }
        ))
    ));
    let NodeKindDto::Loft {
        section_sources, ..
    } = &mut dto.nodes[0].kind
    else {
        panic!("loft")
    };
    *section_sources = alloc::vec![Some(u32::MAX); 6];
    assert!(matches!(
        interchange::from_dto(&dto),
        Err(interchange::InterchangeError::Recipe(
            exedra_constructive::ir::RecipeError::UnknownSource { source: u32::MAX }
        ))
    ));
    let NodeKindDto::Loft {
        section_sources, ..
    } = &mut dto.nodes[0].kind
    else {
        panic!("loft")
    };
    section_sources.clear();
    assert!(interchange::from_dto(&dto).is_ok());
}
