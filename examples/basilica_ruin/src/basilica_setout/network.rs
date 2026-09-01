// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The basilica's operative exact setting-out network.
//!
//! This module owns the building-specific premise set and stable quantity names.
//! Generic propagation remains in `setout`, construction lowering in
//! `setout_joiner`, and historical interpretation in `setout_reconstruction`.
//! Architecture systems receive resolved sections from here rather than
//! recomputing building datums independently.

use std::fmt;

use setout::{
    ArithmeticError, BuildError, Count, Evaluation, EvaluationDelta, EvaluationError,
    EvaluationScenario, EvaluationScenarioBuilder, IncrementalEvaluator, Length, NetworkDef,
    Offset, PlanError, Point3, PropagationPlan, Quantity, Rational, RootBuildError, RootClaimSet,
    ScenarioBuildError, WorkReport, compile_plan, evaluate,
};
use setout_generate::{
    DeltaError as GenerationDeltaError, FragmentDelta, GenerationError, LinearBayFragment,
    LinearFragment, MAX_LINEAR_STATIONS,
};
use setout_joiner::{
    BindingIndex, DirtyElement, ResolveError, ResolvedElementGeometry, SegmentMemberBinding,
};
use setout_reconstruction::{
    CatalogueError, ReconstructionAssessment, ReconstructionCatalogue, assess,
};

use super::{
    AisleSection, BasilicaPremises, CrossingSection, EastEndSection, LevelSection, PlanSection,
    RoofSection, RoofSide,
};

mod bindings;
mod catalogue;
mod definition;
mod generation;
mod resolve;
mod roots;

use bindings::build_binding_index;
use catalogue::build_catalogue;
use definition::build_definition;
use generation::{generate_arcade_bays, generate_buttress_stations, generate_west_truss_stations};
use resolve::{
    resolve_aisle_section, resolve_crossing_section, resolve_east_end_section,
    resolve_level_section, resolve_plan_section, resolve_roof_section,
};
use roots::build_roots;

// Exact generation deliberately bounds one fragment's work. Reject an arcade
// or truss interval count before setout evaluation if its endpoint-inclusive
// expansion would exceed that shared limit; the smaller bound also keeps every
// existing `u32` architecture inventory calculation safe.
const MAX_GENERATED_INTERVALS: Count = Count::new((MAX_LINEAR_STATIONS - 1) as u64);

#[derive(Clone, Debug)]
struct BasilicaQuantities {
    origin: Quantity<Offset>,
    length: Quantity<Length>,
    east_end: Quantity<Offset>,
    span: Quantity<Length>,
    half_span: Quantity<Length>,
    total_width: Quantity<Length>,
    half_total: Quantity<Length>,
    aisle_run: Quantity<Length>,
    wall_thickness: Quantity<Length>,
    crossing_station: Quantity<Length>,
    crossing_center: Quantity<Offset>,
    crossing_clearance: Quantity<Length>,
    crossing_half_width: Quantity<Length>,
    crossing_span: Quantity<Length>,
    crossing_west: Quantity<Offset>,
    crossing_east: Quantity<Offset>,
    west_nave_length: Quantity<Length>,
    east_nave_length: Quantity<Length>,
    arcade_bays: Quantity<Count>,
    west_arcade_bays: Quantity<Count>,
    east_arcade_bays: Quantity<Count>,
    arcade_end_clearance: Quantity<Length>,
    nave_truss_bays: Quantity<Count>,
    buttress_west_inset: Quantity<Length>,
    buttress_east_inset: Quantity<Length>,
    buttress_start: Quantity<Offset>,
    buttress_end: Quantity<Offset>,
    nave_truss_end_clearance: Quantity<Length>,
    nave_truss_west_start: Quantity<Offset>,
    nave_truss_west_end: Quantity<Offset>,
    nave_truss_east: Quantity<Offset>,
    nave_wall_height: Quantity<Length>,
    wall_head: Quantity<Offset>,
    aisle_wall_height: Quantity<Length>,
    aisle_wall_top: Quantity<Offset>,
    clerestory_base_height: Quantity<Length>,
    clerestory_base: Quantity<Offset>,
    clerestory_sill_height_from_ground: Quantity<Length>,
    clerestory_sill: Quantity<Offset>,
    clerestory_spring_height_from_ground: Quantity<Length>,
    clerestory_spring: Quantity<Offset>,
    clerestory_height: Quantity<Length>,
    clerestory_sill_height: Quantity<Length>,
    clerestory_spring_height: Quantity<Length>,
    crossing_spandrel_base_height: Quantity<Length>,
    crossing_spandrel_base: Quantity<Offset>,
    crossing_web_bottom_height: Quantity<Length>,
    crossing_web_bottom: Quantity<Offset>,
    crossing_web_middle_height: Quantity<Length>,
    crossing_web_middle: Quantity<Offset>,
    apse_wall_height: Quantity<Length>,
    apse_wall_top: Quantity<Offset>,
    aisle_roof_overhang: Quantity<Length>,
    aisle_roof_run: Quantity<Length>,
    aisle_roof_inner_lift: Quantity<Length>,
    aisle_roof_inner_height: Quantity<Offset>,
    aisle_roof_bearing_inset: Quantity<Length>,
    aisle_roof_bearing_height: Quantity<Offset>,
    aisle_roof_bearing_drop: Quantity<Length>,
    aisle_roof_pitch: Quantity<Rational>,
    aisle_roof_drop: Quantity<Length>,
    aisle_roof_slope_length: Quantity<Length>,
    aisle_roof_depth: Quantity<Length>,
    wall_plate_height: Quantity<Length>,
    wall_plate_top: Quantity<Offset>,
    rise: Quantity<Length>,
    ridge_height: Quantity<Offset>,
    pitch: Quantity<Rational>,
    rafter_length: Quantity<Length>,
    overhang: Quantity<Length>,
    overhang_drop: Quantity<Length>,
    roof_run: Quantity<Length>,
    roof_eave_height: Quantity<Offset>,
    roof_slope_length: Quantity<Length>,
    roof_skin_depth: Quantity<Length>,
    wall_plate_width: Quantity<Length>,
    principal_rafter_width: Quantity<Length>,
    principal_rafter_depth: Quantity<Length>,
    principal_rafter_reveal: Quantity<Offset>,
    drum_radius: Quantity<Length>,
    crossing_platform_inset: Quantity<Length>,
    platform_inner_radius: Quantity<Length>,
    cornice_overhang: Quantity<Length>,
    cornice_radius: Quantity<Length>,
    dome_overhang: Quantity<Length>,
    dome_radius: Quantity<Length>,
    drum_height: Quantity<Length>,
    dome_height: Quantity<Length>,
    drum_base_above_ridge: Quantity<Length>,
    drum_base: Quantity<Offset>,
    platform_height: Quantity<Length>,
    platform_base: Quantity<Offset>,
    drum_top: Quantity<Offset>,
    dome_top: Quantity<Offset>,
    apse_radius: Quantity<Length>,
    apse_inner_radius: Quantity<Length>,
    conch_overhang: Quantity<Length>,
    conch_radius: Quantity<Length>,
    conch_height: Quantity<Length>,
    north_wall_seat: Quantity<Point3>,
    south_wall_seat: Quantity<Point3>,
    ridge_point: Quantity<Point3>,
    north_roof_eave: Quantity<Point3>,
    south_roof_eave: Quantity<Point3>,
}

/// Evaluated, explainable setting-out hypothesis for the whole basilica.
#[derive(Clone, Debug)]
pub struct BasilicaSetout {
    definition: NetworkDef,
    quantities: BasilicaQuantities,
    roots: RootClaimSet,
    scenario: EvaluationScenario,
    plan: PropagationPlan,
    evaluation: Evaluation,
    plan_section: PlanSection,
    level_section: LevelSection,
    aisle_section: AisleSection,
    crossing_section: CrossingSection,
    east_end_section: EastEndSection,
    roof_section: RoofSection,
    outer_arcade_bays: LinearBayFragment,
    west_arcade_bays: LinearBayFragment,
    east_arcade_bays: LinearBayFragment,
    buttress_stations: LinearFragment,
    west_truss_stations: LinearFragment,
    catalogue: ReconstructionCatalogue,
    assessment: ReconstructionAssessment,
    bindings: BindingIndex,
}

impl BasilicaSetout {
    /// Builds and evaluates the basilica's operative setting-out hypothesis.
    pub fn new(premises: &BasilicaPremises) -> Result<Self, BasilicaSetoutError> {
        validate_topology_domain(premises)?;
        let (definition, quantities) = build_definition()?;
        let roots = build_roots(&definition, &quantities, premises)?;
        let scenario = EvaluationScenarioBuilder::new("basilica/operative")?
            .activate_all(&roots)
            .finish(&roots)?;
        let plan = compile_plan(&definition, &roots, &scenario)?;
        let evaluation = evaluate(&definition, &roots, &scenario, &plan)?;
        let plan_section = resolve_plan_section(&evaluation, &quantities)?;
        let level_section = resolve_level_section(&evaluation, &quantities)?;
        let aisle_section = resolve_aisle_section(&evaluation, &quantities)?;
        let crossing_section = resolve_crossing_section(&evaluation, &quantities)?;
        let east_end_section = resolve_east_end_section(&evaluation, &quantities)?;
        let roof_section = resolve_roof_section(&evaluation, &quantities)?;
        let arcade_bays = generate_arcade_bays(&plan_section)?;
        let buttress_stations = generate_buttress_stations(&plan_section)?;
        let west_truss_stations = generate_west_truss_stations(&plan_section)?;
        let catalogue = build_catalogue(&roots, &plan)?;
        let assessment = assess(
            evaluation.provenance(),
            evaluation.fingerprint(),
            &catalogue,
        );
        let bindings = build_binding_index(&quantities, &buttress_stations, &west_truss_stations);
        Ok(Self {
            definition,
            quantities,
            roots,
            scenario,
            plan,
            evaluation,
            plan_section,
            level_section,
            aisle_section,
            crossing_section,
            east_end_section,
            roof_section,
            outer_arcade_bays: arcade_bays.outer,
            west_arcade_bays: arcade_bays.west,
            east_arcade_bays: arcade_bays.east,
            buttress_stations,
            west_truss_stations,
            catalogue,
            assessment,
            bindings,
        })
    }

    /// Returns the exact resolved roof section.
    #[must_use]
    pub const fn roof(&self) -> &RoofSection {
        &self.roof_section
    }

    /// Returns the exact resolved plan massing.
    #[must_use]
    pub const fn plan(&self) -> &PlanSection {
        &self.plan_section
    }

    /// Returns exact bays spanning each exterior longitudinal arcade wall.
    #[must_use]
    pub const fn outer_arcade_bays(&self) -> &LinearBayFragment {
        &self.outer_arcade_bays
    }

    /// Returns exact pierced bays west of the physically open crossing.
    #[must_use]
    pub const fn west_arcade_bays(&self) -> &LinearBayFragment {
        &self.west_arcade_bays
    }

    /// Returns exact pierced bays east of the physically open crossing.
    #[must_use]
    pub const fn east_arcade_bays(&self) -> &LinearBayFragment {
        &self.east_arcade_bays
    }

    /// Returns the exact, semantically labeled aisle-buttress stations.
    #[must_use]
    pub const fn buttress_stations(&self) -> &LinearFragment {
        &self.buttress_stations
    }

    /// Returns the exact west nave-truss stations after ruin omissions.
    #[must_use]
    pub const fn west_truss_stations(&self) -> &LinearFragment {
        &self.west_truss_stations
    }

    /// Returns the exact resolved vertical datums.
    #[must_use]
    pub const fn levels(&self) -> &LevelSection {
        &self.level_section
    }

    /// Returns the exact resolved aisle-roof section.
    #[must_use]
    pub const fn aisle(&self) -> &AisleSection {
        &self.aisle_section
    }

    /// Returns the exact resolved crossing massing.
    #[must_use]
    pub const fn crossing(&self) -> &CrossingSection {
        &self.crossing_section
    }

    /// Returns the exact resolved east-end massing.
    #[must_use]
    pub const fn east_end(&self) -> &EastEndSection {
        &self.east_end_section
    }

    /// Returns core structural provenance and exactness.
    #[must_use]
    pub const fn evaluation(&self) -> &Evaluation {
        &self.evaluation
    }

    /// Returns the reconstruction-only sidecar assessment.
    #[must_use]
    pub const fn reconstruction(&self) -> &ReconstructionAssessment {
        &self.assessment
    }

    /// Returns a structural explanation of the ridge point.
    #[must_use]
    pub fn explain_ridge(&self) -> String {
        self.evaluation
            .provenance()
            .explain(self.quantities.ridge_point.key())
            .expect("the accepted roof always resolves its ridge")
    }

    /// Resolves the exact structural principal-rafter extent on one side.
    pub fn principal_rafter_geometry(
        &self,
        side: RoofSide,
    ) -> Result<ResolvedElementGeometry, ResolveError> {
        let (foot, width_reference) = match side {
            RoofSide::North => (self.quantities.north_wall_seat.clone(), [1.0, 0.0, 0.0]),
            RoofSide::South => (self.quantities.south_wall_seat.clone(), [-1.0, 0.0, 0.0]),
        };
        SegmentMemberBinding::new(
            foot,
            self.quantities.ridge_point.clone(),
            self.quantities.rafter_length.clone(),
            self.quantities.principal_rafter_width.clone(),
            self.quantities.principal_rafter_depth.clone(),
            width_reference,
        )
        .resolve(&self.evaluation)
    }

    /// Re-evaluates the same immutable definition and proves warm/fresh equivalence.
    pub fn reconfigure(
        &self,
        premises: &BasilicaPremises,
    ) -> Result<BasilicaReconfiguration, BasilicaSetoutError> {
        validate_topology_domain(premises)?;
        let roots = build_roots(&self.definition, &self.quantities, premises)?;
        let scenario = EvaluationScenarioBuilder::new(self.scenario.key.as_str())?
            .activate_all(&roots)
            .finish(&roots)?;
        let plan = compile_plan(&self.definition, &roots, &scenario)?;
        let warm = IncrementalEvaluator::new().successor(
            &self.definition,
            &roots,
            &scenario,
            &plan,
            &self.evaluation,
        )?;
        let fresh = evaluate(&self.definition, &roots, &scenario, &plan)?;
        if warm.fingerprint() != fresh.fingerprint() {
            return Err(BasilicaSetoutError::WarmFreshMismatch);
        }
        let plan_section = resolve_plan_section(&warm, &self.quantities)?;
        let level_section = resolve_level_section(&warm, &self.quantities)?;
        let aisle_section = resolve_aisle_section(&warm, &self.quantities)?;
        let crossing_section = resolve_crossing_section(&warm, &self.quantities)?;
        let east_end_section = resolve_east_end_section(&warm, &self.quantities)?;
        let roof = resolve_roof_section(&warm, &self.quantities)?;
        let arcade_bays = generate_arcade_bays(&plan_section)?;
        let outer_arcade_delta = self.outer_arcade_bays.delta_to(&arcade_bays.outer)?;
        let west_arcade_delta = self.west_arcade_bays.delta_to(&arcade_bays.west)?;
        let east_arcade_delta = self.east_arcade_bays.delta_to(&arcade_bays.east)?;
        let buttress_stations = generate_buttress_stations(&plan_section)?;
        let buttress_delta = self.buttress_stations.delta_to(&buttress_stations)?;
        let west_truss_stations = generate_west_truss_stations(&plan_section)?;
        let west_truss_delta = self.west_truss_stations.delta_to(&west_truss_stations)?;
        let delta = warm.delta_from(&self.evaluation);
        let topology_changed = [
            &outer_arcade_delta,
            &west_arcade_delta,
            &east_arcade_delta,
            &buttress_delta,
            &west_truss_delta,
        ]
        .into_iter()
        .any(|fragment| !fragment.added().is_empty() || !fragment.removed().is_empty());
        let dirty = self.bindings.dirty(&delta);
        Ok(BasilicaReconfiguration {
            plan: plan_section,
            levels: level_section,
            aisle: aisle_section,
            crossing: crossing_section,
            east_end: east_end_section,
            roof,
            outer_arcade_bays: arcade_bays.outer,
            outer_arcade_delta,
            west_arcade_bays: arcade_bays.west,
            west_arcade_delta,
            east_arcade_bays: arcade_bays.east,
            east_arcade_delta,
            buttress_stations,
            buttress_delta,
            west_truss_stations,
            west_truss_delta,
            delta,
            dirty,
            topology_changed,
            work: warm.work_report(),
            fingerprint: warm.fingerprint(),
        })
    }

    /// Returns the root-set fingerprint for diagnostics.
    #[must_use]
    pub const fn roots_fingerprint(&self) -> setout::Fingerprint {
        self.roots.fingerprint()
    }

    /// Returns the plan fingerprint for diagnostics.
    #[must_use]
    pub const fn plan_fingerprint(&self) -> setout::Fingerprint {
        self.plan.fingerprint()
    }

    /// Returns the sidecar catalogue fingerprint for diagnostics.
    #[must_use]
    pub const fn catalogue_fingerprint(&self) -> setout::Fingerprint {
        self.catalogue.fingerprint()
    }
}

/// Result of an incremental basilica-premise edit.
#[derive(Clone, Debug)]
pub struct BasilicaReconfiguration {
    /// Newly resolved exact plan massing.
    pub plan: PlanSection,
    /// Newly resolved exact vertical datums.
    pub levels: LevelSection,
    /// Newly resolved exact aisle-roof section.
    pub aisle: AisleSection,
    /// Newly resolved exact crossing massing.
    pub crossing: CrossingSection,
    /// Newly resolved exact east-end massing.
    pub east_end: EastEndSection,
    /// Newly resolved exact roof section.
    pub roof: RoofSection,
    /// Newly expanded exact exterior longitudinal arcade bays.
    pub outer_arcade_bays: LinearBayFragment,
    /// Stable item-level change from the previous exterior bay expansion.
    pub outer_arcade_delta: FragmentDelta,
    /// Newly expanded exact pierced bays west of the crossing.
    pub west_arcade_bays: LinearBayFragment,
    /// Stable item-level change from the previous west arcade expansion.
    pub west_arcade_delta: FragmentDelta,
    /// Newly expanded exact pierced bays east of the crossing.
    pub east_arcade_bays: LinearBayFragment,
    /// Stable item-level change from the previous east arcade expansion.
    pub east_arcade_delta: FragmentDelta,
    /// Newly expanded exact aisle-buttress stations.
    pub buttress_stations: LinearFragment,
    /// Stable item-level change from the previous buttress expansion.
    pub buttress_delta: FragmentDelta,
    /// Newly expanded exact west nave-truss stations.
    pub west_truss_stations: LinearFragment,
    /// Stable item-level change from the previous west truss expansion.
    pub west_truss_delta: FragmentDelta,
    /// Exact core quantity and claim changes.
    pub delta: EvaluationDelta,
    /// Named construction/assembly elements and Joiner channels affected.
    pub dirty: Box<[DirtyElement]>,
    /// Whether repeated assembly topology must be rebuilt, not merely updated.
    ///
    /// [`BasilicaReconfiguration::dirty`] can name only elements that already
    /// exist. The generated fragment deltas identify the exact bay, buttress,
    /// and west-truss additions and removals that require a topology rebuild.
    pub topology_changed: bool,
    /// Work reused versus recomputed.
    pub work: WorkReport,
    /// Warm evaluation fingerprint, already checked against fresh evaluation.
    pub fingerprint: setout::Fingerprint,
}

/// Failure to build, resolve, or reconfigure the basilica setting-out hypothesis.
#[derive(Debug)]
#[non_exhaustive]
pub enum BasilicaSetoutError {
    /// Network definition is invalid.
    Build(BuildError),
    /// An exact authored constant is outside its measurement representation.
    Arithmetic(ArithmeticError),
    /// Root claims are invalid.
    Roots(RootBuildError),
    /// Scenario is invalid.
    Scenario(ScenarioBuildError),
    /// Plan compilation failed.
    Plan(PlanError),
    /// Evaluation failed.
    Evaluation(EvaluationError),
    /// Strict access failed.
    Access(setout::AccessError),
    /// Reconstruction catalogue contains duplicate identity.
    Catalogue(CatalogueError),
    /// Exact topology generation failed.
    Generation(GenerationError),
    /// Generated fragments did not share the expected invocation identity.
    GenerationDelta(GenerationDeltaError),
    /// The east buttress anchor does not lie east of the west anchor.
    InvalidButtressExtent,
    /// The final west truss anchor does not lie east of its first anchor.
    InvalidNaveTrussExtent,
    /// Arcade end clearances leave no positive run for one wall segment.
    InvalidArcadeExtent,
    /// Warm and fresh evaluations did not agree.
    WarmFreshMismatch,
    /// The arcade repeat count exceeds one bounded generation fragment.
    ArcadeBayCountTooLarge {
        /// Authored repeat count.
        actual: Count,
        /// Largest count for which derived inventory arithmetic is defined.
        maximum: Count,
    },
    /// A west truss repeat count exceeds one bounded generation fragment.
    NaveTrussBayCountTooLarge {
        /// Authored repeat count.
        actual: Count,
        /// Largest count accepted by one linear fragment.
        maximum: Count,
    },
    /// A stable semantic key is invalid.
    Key(setout::KeyError),
}

macro_rules! error_from {
    ($source:ty, $variant:ident) => {
        impl From<$source> for BasilicaSetoutError {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}

error_from!(BuildError, Build);
error_from!(ArithmeticError, Arithmetic);
error_from!(RootBuildError, Roots);
error_from!(ScenarioBuildError, Scenario);
error_from!(PlanError, Plan);
error_from!(EvaluationError, Evaluation);
error_from!(setout::AccessError, Access);
error_from!(CatalogueError, Catalogue);
error_from!(setout::KeyError, Key);
error_from!(GenerationError, Generation);
error_from!(GenerationDeltaError, GenerationDelta);

impl fmt::Display for BasilicaSetoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => write!(formatter, "basilica definition failed: {error}"),
            Self::Arithmetic(error) => write!(formatter, "basilica premise failed: {error}"),
            Self::Roots(error) => write!(formatter, "basilica roots failed: {error}"),
            Self::Scenario(error) => write!(formatter, "basilica scenario failed: {error}"),
            Self::Plan(error) => write!(formatter, "basilica plan failed: {error}"),
            Self::Evaluation(error) => write!(formatter, "basilica evaluation failed: {error}"),
            Self::Access(error) => write!(formatter, "basilica access failed: {error}"),
            Self::Catalogue(error) => write!(formatter, "basilica catalogue failed: {error}"),
            Self::Generation(error) => write!(formatter, "basilica generation failed: {error}"),
            Self::GenerationDelta(error) => {
                write!(formatter, "basilica generation delta failed: {error}")
            }
            Self::InvalidButtressExtent => {
                formatter.write_str("basilica buttress anchors do not define a positive extent")
            }
            Self::InvalidNaveTrussExtent => {
                formatter.write_str("basilica west truss anchors do not define a positive extent")
            }
            Self::InvalidArcadeExtent => {
                formatter.write_str("basilica arcade clearances do not define a positive extent")
            }
            Self::WarmFreshMismatch => {
                formatter.write_str("warm basilica evaluation differs from fresh")
            }
            Self::ArcadeBayCountTooLarge { actual, maximum } => write!(
                formatter,
                "arcade bay count {} exceeds the architecture maximum {}",
                actual.get(),
                maximum.get()
            ),
            Self::NaveTrussBayCountTooLarge { actual, maximum } => write!(
                formatter,
                "nave truss bay count {} exceeds the architecture maximum {}",
                actual.get(),
                maximum.get()
            ),
            Self::Key(error) => write!(formatter, "basilica key failed: {error}"),
        }
    }
}

impl std::error::Error for BasilicaSetoutError {}

fn validate_topology_domain(premises: &BasilicaPremises) -> Result<(), BasilicaSetoutError> {
    if premises.arcade_bays > MAX_GENERATED_INTERVALS {
        return Err(BasilicaSetoutError::ArcadeBayCountTooLarge {
            actual: premises.arcade_bays,
            maximum: MAX_GENERATED_INTERVALS,
        });
    }
    if premises.nave_truss_bays > MAX_GENERATED_INTERVALS {
        return Err(BasilicaSetoutError::NaveTrussBayCountTooLarge {
            actual: premises.nave_truss_bays,
            maximum: MAX_GENERATED_INTERVALS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
