// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Matched constant-section rail, prepared once for realization measurements.

use exedra_analytic::{realize::RealizationPolicy, sweep::Sweep};
use exedra_constructive::{
    builders,
    evaluate::evaluate,
    ir::{CapMode, NodeKind, Path3, Placement3, Recipe, RecipeBuilder, SectionLaw},
    path::PathCursor3,
    profile::Profile2,
    tessellate::EvalPolicy,
};
use exedra_curves::profile::ProfilePolicy;

pub(crate) fn profile() -> Profile2 {
    builders::rect_centered(20.0, 20.0).unwrap()
}
pub(crate) fn path() -> Path3 {
    PathCursor3::new([0.0; 3], [0.0, 1.0, 0.0])
        .tangent_arc(40.0, core::f64::consts::PI, [0.0, 0.0, 1.0])
        .tangent_arc(40.0, -core::f64::consts::FRAC_PI_2, [0.0, 0.0, 1.0])
        .line_by([-40.0, 0.0, 0.0])
        .finish([1.0, 0.0, 0.0])
        .unwrap()
}
pub(crate) fn native(profile: &Profile2, path: &Path3) -> Sweep {
    Sweep::from_profile(
        profile,
        path,
        Placement3::IDENTITY,
        &ProfilePolicy::new(1e-7),
    )
    .unwrap()
}
pub(crate) fn recipe(profile: Profile2, path: Path3) -> Recipe {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(profile);
    let root = b
        .add_node(NodeKind::Sweep {
            profile,
            path,
            section: SectionLaw::IDENTITY,
            caps: CapMode::Both,
        })
        .unwrap();
    b.finish(root).unwrap()
}
pub(crate) fn native_realization(sweep: &Sweep) -> exedra_analytic::realize::RealizedBody {
    let output = sweep.realize(&RealizationPolicy::default()).unwrap();
    assert!(
        output.report.within_tolerance(),
        "native fixture must meet tolerance"
    );
    output
}
pub(crate) fn recipe_realization(recipe: &Recipe) -> exedra_constructive::evaluate::Evaluation {
    let mut policy = EvalPolicy::default();
    // Match the outer rail's radius 50 to the recipe centerline's radius 40.
    policy.sweep_path.chord_tolerance = 0.008;
    let evaluated = evaluate(recipe, &policy).unwrap();
    assert_eq!(evaluated.bodies.len(), 1, "fixture must produce one mesh");
    evaluated
}
