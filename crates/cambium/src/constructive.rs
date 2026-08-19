// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Plan lifecycle for constructive recipes.
//!
//! Mirrors the mesh-operator compile/apply discipline for the constructive
//! head: `compile` binds a plan to the exact recipe content (and evaluation
//! policy) it was made from, and `apply` refuses to run against anything
//! else — staleness is detected exactly, through content fingerprints,
//! never guessed through timestamps or object identity.
//!
//! Evaluation is pure, so preview and apply produce identical output by
//! construction; both route through one code path and tests pin the parity.

use crate::context::Clock;
use crate::convert::{
    ConstructiveEvalError, ConstructiveEvalPolicy, ConstructiveToMeshOutput,
    ConstructiveToMeshParams, Recipe, RecipeFingerprint, constructive_recipe_to_mesh,
};
use crate::report::Timings;

/// A compiled recipe plan, bound to recipe content and evaluation policy.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RecipePlan {
    /// Content fingerprint of the recipe the plan was compiled from.
    pub fingerprint: RecipeFingerprint,
    /// The evaluation policy the plan will run under.
    pub policy: ConstructiveEvalPolicy,
}

/// Why a plan refused to apply.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RecipePlanError {
    /// The recipe's content changed since the plan was compiled.
    StaleRecipe {
        /// Fingerprint the plan was bound to.
        planned: RecipeFingerprint,
        /// The recipe's current fingerprint.
        current: RecipeFingerprint,
    },
    /// Evaluation failed.
    Eval(ConstructiveEvalError),
}

impl core::fmt::Display for RecipePlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::StaleRecipe { .. } => {
                write!(f, "recipe content changed since the plan was compiled")
            }
            Self::Eval(e) => write!(f, "evaluation failed: {e}"),
        }
    }
}

impl core::error::Error for RecipePlanError {}

/// The applied result plus lifecycle timings.
#[derive(Debug)]
pub struct RecipeRun {
    /// The conversion output (bodies, fingerprint, report, diagnostics).
    pub output: ConstructiveToMeshOutput,
    /// Lifecycle timings (`recipe.compile` / `recipe.apply` buckets).
    pub timings: Timings,
}

/// Compiles a plan from a recipe under a policy.
///
/// Cheap by design: the fingerprint already exists on the frozen recipe.
#[must_use]
pub fn compile_recipe(recipe: &Recipe, policy: ConstructiveEvalPolicy) -> RecipePlan {
    RecipePlan {
        fingerprint: recipe.recipe_fingerprint(),
        policy,
    }
}

/// Applies a compiled plan to a recipe.
///
/// # Errors
///
/// Refuses with [`RecipePlanError::StaleRecipe`] when the recipe's content
/// fingerprint differs from the plan's — the plan must be recompiled
/// against the edited recipe — and surfaces evaluation failures typed.
pub fn apply_recipe(recipe: &Recipe, plan: &RecipePlan) -> Result<RecipeRun, RecipePlanError> {
    let clock = Clock::new(Timings::DEFAULT_MAX_BUCKETS);
    {
        let _bucket = clock.bucket("recipe.compile");
        let current = recipe.recipe_fingerprint();
        if current != plan.fingerprint {
            return Err(RecipePlanError::StaleRecipe {
                planned: plan.fingerprint,
                current,
            });
        }
    }
    let output = {
        let _bucket = clock.bucket("recipe.apply");
        constructive_recipe_to_mesh(
            recipe,
            &ConstructiveToMeshParams {
                policy: plan.policy,
            },
        )
        .map_err(RecipePlanError::Eval)?
    };
    Ok(RecipeRun {
        output,
        timings: clock.timings(),
    })
}

/// Previews a plan: identical to [`apply_recipe`] by construction, because
/// recipe evaluation is a pure function — provided (and tested) as an
/// explicit lifecycle point so callers can mirror the mesh-operator
/// preview/apply discipline.
///
/// # Errors
///
/// Same contract as [`apply_recipe`].
pub fn preview_recipe(recipe: &Recipe, plan: &RecipePlan) -> Result<RecipeRun, RecipePlanError> {
    apply_recipe(recipe, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::builders;
    use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
    use exedra_testkit::golden::trimesh_signature;

    fn recipe(height: f64) -> Recipe {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
        let n = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height,
                caps: CapMode::Both,
            })
            .expect("valid");
        b.finish(n).expect("valid recipe")
    }

    #[test]
    fn preview_apply_parity() {
        let r = recipe(3.0);
        let plan = compile_recipe(&r, ConstructiveEvalPolicy::default());
        let preview = preview_recipe(&r, &plan).expect("previews");
        let applied = apply_recipe(&r, &plan).expect("applies");
        assert_eq!(preview.output.fingerprint, applied.output.fingerprint);
        assert_eq!(preview.output.report, applied.output.report);
        let sig = |run: &RecipeRun| {
            let (tri, _) = run.output.bodies[0]
                .body
                .mesh
                .to_trimesh(&exedra::ExtractParams::default());
            trimesh_signature(&tri)
        };
        assert_eq!(
            sig(&preview),
            sig(&applied),
            "preview == apply, bit for bit"
        );
    }

    #[test]
    fn stale_plans_are_rejected() {
        let plan = compile_recipe(&recipe(3.0), ConstructiveEvalPolicy::default());
        let edited = recipe(4.0);
        let result = apply_recipe(&edited, &plan);
        assert!(matches!(result, Err(RecipePlanError::StaleRecipe { .. })));
        // Recompiling against the edited recipe applies cleanly.
        let replanned = compile_recipe(&edited, ConstructiveEvalPolicy::default());
        assert!(apply_recipe(&edited, &replanned).is_ok());
    }

    #[test]
    fn runs_carry_lifecycle_timings() {
        let r = recipe(2.0);
        let plan = compile_recipe(&r, ConstructiveEvalPolicy::default());
        let run = apply_recipe(&r, &plan).expect("applies");
        let names: alloc::vec::Vec<&str> = run.timings.iter().map(|b| b.name).collect();
        assert!(names.contains(&"recipe.compile"));
        assert!(names.contains(&"recipe.apply"));
    }
}
