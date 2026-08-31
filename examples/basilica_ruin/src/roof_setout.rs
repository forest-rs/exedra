// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The basilica's one operative roof setting-out hypothesis.
//!
//! This module owns the building-specific premise set and stable quantity names.
//! Generic propagation remains in `setout`, construction lowering in
//! `setout_joiner`, and historical interpretation in `setout_reconstruction`.
//! Every accepted roof consumer receives a [`RoofSection`] from here rather than
//! recomputing pitch, ridge, eave, or rafter coordinates independently.

use std::fmt;

use exedra_math::{cross, normalize, sub};
use setout::{
    ArithmeticError, BuildError, ComposePoint, Evaluation, EvaluationDelta, EvaluationError,
    EvaluationScenario, EvaluationScenarioBuilder, IncrementalEvaluator, Knowledge, Length,
    NetworkBuilder, NetworkDef, Offset, OffsetByLength, OffsetDirection, Pitch, PlanError, Point3,
    PropagationPlan, Pythagorean, Quantity, QuantityPolicy, Rational, RootBuildError, RootClaimSet,
    RootClaimSetBuilder, ScaleLength, ScenarioBuildError, Sum, WorkReport, compile_plan, evaluate,
    quantize_length_meters, quantize_offset_meters,
};
use setout_joiner::{
    BindingIndex, DirtyChannel, DirtyElement, ResolveError, ResolvedElementGeometry,
    SegmentMemberBinding, lower_point,
};
use setout_reconstruction::{
    CatalogueError, DerivationCharacter, MethodWarrant, ReconstructionAssessment,
    ReconstructionCatalogue, ReconstructionCatalogueBuilder, SourceBasis, SourceRef, assess,
};

use crate::BasilicaParams;

const WALL_PLATE_HEIGHT_MM: u64 = 180;
const WALL_PLATE_WIDTH_MM: u64 = 300;
const ROOF_OVERHANG_MM: u64 = 350;
const ROOF_SKIN_DEPTH_MM: u64 = 280;
const PRINCIPAL_RAFTER_WIDTH_MM: u64 = 260;
const PRINCIPAL_RAFTER_DEPTH_MM: u64 = 240;
const PRINCIPAL_RAFTER_REVEAL_MM: i64 = 120;

/// Side of the symmetrical nave roof.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RoofSide {
    /// North slope, positive Y.
    North,
    /// South slope, negative Y.
    South,
}

impl RoofSide {
    /// Returns `1` for north and `-1` for south.
    #[must_use]
    pub const fn sign(self) -> f64 {
        match self {
            Self::North => 1.0,
            Self::South => -1.0,
        }
    }
}

/// Exact resolved section shared by the ruin and structure laboratory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoofSection {
    /// Clear nave span between wall centerlines.
    pub span: Length,
    /// Half of [`RoofSection::span`].
    pub half_span: Length,
    /// Masonry wall head before the timber wall plate.
    pub wall_head: Offset,
    /// Continuous wall-plate height.
    pub wall_plate_height: Length,
    /// Top bearing datum of the wall plate.
    pub wall_plate_top: Offset,
    /// Rise from the wall-plate bearing datum to the ridge.
    pub rise: Length,
    /// Exact ridge datum.
    pub ridge_height: Offset,
    /// Exact rise/run ratio.
    pub pitch: Rational,
    /// Principal-rafter length selected to the nearest iota with a root certificate.
    pub rafter_length: Length,
    /// Horizontal roof overhang beyond the wall centerline.
    pub overhang: Length,
    /// Vertical fall across the overhang at the shared pitch.
    pub overhang_drop: Length,
    /// Full horizontal run from ridge to outer eave.
    pub roof_run: Length,
    /// Underside datum at the outer eave.
    pub roof_eave_height: Offset,
    /// Full sloping roof-skin length from ridge to outer eave.
    pub roof_slope_length: Length,
    /// Modeled roof-skin thickness.
    pub roof_skin_depth: Length,
    /// Wall-plate width across the wall.
    pub wall_plate_width: Length,
    /// Principal-rafter width along the nave.
    pub principal_rafter_width: Length,
    /// Principal-rafter depth normal to the slope.
    pub principal_rafter_depth: Length,
    /// Deliberate visual reveal between principal rafters and roof skin.
    pub principal_rafter_reveal: Offset,
    /// North wall-plate seat point at X=0.
    pub north_wall_seat: Point3,
    /// South wall-plate seat point at X=0.
    pub south_wall_seat: Point3,
    /// Ridge point at X=0.
    pub ridge_point: Point3,
    /// North outer-eave underside point at X=0.
    pub north_roof_eave: Point3,
    /// South outer-eave underside point at X=0.
    pub south_roof_eave: Point3,
}

impl RoofSection {
    /// Returns the exact structural wall seat on one side.
    #[must_use]
    pub const fn wall_seat(&self, side: RoofSide) -> Point3 {
        match side {
            RoofSide::North => self.north_wall_seat,
            RoofSide::South => self.south_wall_seat,
        }
    }

    /// Returns the exact outer roof-skin eave point on one side.
    #[must_use]
    pub const fn roof_eave(&self, side: RoofSide) -> Point3 {
        match side {
            RoofSide::North => self.north_roof_eave,
            RoofSide::South => self.south_roof_eave,
        }
    }

    /// Returns the inward/up unit slope from a wall plate to the ridge.
    #[must_use]
    pub fn inward_slope(&self, side: RoofSide) -> [f64; 3] {
        normalize(sub(
            lower_point(self.ridge_point),
            lower_point(self.wall_seat(side)),
        ))
        .expect("resolved roof seats never coincide with the ridge")
    }

    /// Returns the outward/up unit normal of one roof slope.
    #[must_use]
    pub fn outward_normal(&self, side: RoofSide) -> [f64; 3] {
        let x_axis = [1.0, 0.0, 0.0];
        match side {
            RoofSide::North => cross(self.inward_slope(side), x_axis),
            RoofSide::South => cross(x_axis, self.inward_slope(side)),
        }
    }
}

#[derive(Clone, Debug)]
struct RoofQuantities {
    origin: Quantity<Offset>,
    span: Quantity<Length>,
    half_span: Quantity<Length>,
    wall_head: Quantity<Offset>,
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
    north_wall_seat: Quantity<Point3>,
    south_wall_seat: Quantity<Point3>,
    ridge_point: Quantity<Point3>,
    north_roof_eave: Quantity<Point3>,
    south_roof_eave: Quantity<Point3>,
}

/// Evaluated, explainable basilica roof hypothesis.
#[derive(Clone, Debug)]
pub struct BasilicaRoofSetout {
    definition: NetworkDef,
    quantities: RoofQuantities,
    roots: RootClaimSet,
    scenario: EvaluationScenario,
    plan: PropagationPlan,
    evaluation: Evaluation,
    section: RoofSection,
    catalogue: ReconstructionCatalogue,
    assessment: ReconstructionAssessment,
    bindings: BindingIndex,
}

impl BasilicaRoofSetout {
    /// Builds and evaluates the basilica's operative roof hypothesis.
    pub fn new(params: &BasilicaParams) -> Result<Self, RoofSetoutError> {
        let (definition, quantities) = build_definition()?;
        let roots = build_roots(&definition, &quantities, params)?;
        let scenario = EvaluationScenarioBuilder::new("basilica/roof/operative")?
            .activate_all(&roots)
            .finish(&roots)?;
        let plan = compile_plan(&definition, &roots, &scenario)?;
        let evaluation = evaluate(&definition, &roots, &scenario, &plan)?;
        let section = resolve_section(&evaluation, &quantities)?;
        let catalogue = build_catalogue(&roots, &plan)?;
        let assessment = assess(
            evaluation.provenance(),
            evaluation.fingerprint(),
            &catalogue,
        );
        let bindings = build_binding_index(&quantities);
        Ok(Self {
            definition,
            quantities,
            roots,
            scenario,
            plan,
            evaluation,
            section,
            catalogue,
            assessment,
            bindings,
        })
    }

    /// Returns the exact shared roof section.
    #[must_use]
    pub const fn section(&self) -> &RoofSection {
        &self.section
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
        params: &BasilicaParams,
    ) -> Result<RoofReconfiguration, RoofSetoutError> {
        let roots = build_roots(&self.definition, &self.quantities, params)?;
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
            return Err(RoofSetoutError::WarmFreshMismatch);
        }
        let section = resolve_section(&warm, &self.quantities)?;
        let delta = warm.delta_from(&self.evaluation);
        let dirty = self.bindings.dirty(&delta);
        Ok(RoofReconfiguration {
            section,
            delta,
            dirty,
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

/// Result of an incremental roof-parameter edit.
#[derive(Clone, Debug)]
pub struct RoofReconfiguration {
    /// Newly resolved exact roof section.
    pub section: RoofSection,
    /// Exact core quantity and claim changes.
    pub delta: EvaluationDelta,
    /// Named construction/assembly elements and Joiner channels affected.
    pub dirty: Box<[DirtyElement]>,
    /// Work reused versus recomputed.
    pub work: WorkReport,
    /// Warm evaluation fingerprint, already checked against fresh evaluation.
    pub fingerprint: setout::Fingerprint,
}

fn build_definition() -> Result<(NetworkDef, RoofQuantities), BuildError> {
    let mut network = NetworkBuilder::new();
    let origin =
        network.declare::<Offset>("basilica/roof/origin", QuantityPolicy::unrestricted())?;
    let span = network.declare::<Length>("basilica/roof/span", QuantityPolicy::positive())?;
    let half_span =
        network.declare::<Length>("basilica/roof/half-span", QuantityPolicy::positive())?;
    let north_half_span = network.declare::<Offset>(
        "basilica/roof/north-half-span",
        QuantityPolicy::unrestricted(),
    )?;
    let south_half_span = network.declare::<Offset>(
        "basilica/roof/south-half-span",
        QuantityPolicy::unrestricted(),
    )?;
    let wall_head =
        network.declare::<Offset>("basilica/roof/wall-head", QuantityPolicy::positive())?;
    let wall_plate_height = network.declare::<Length>(
        "basilica/roof/wall-plate-height",
        QuantityPolicy::positive(),
    )?;
    let wall_plate_top =
        network.declare::<Offset>("basilica/roof/wall-plate-top", QuantityPolicy::positive())?;
    let rise = network.declare::<Length>("basilica/roof/rise", QuantityPolicy::positive())?;
    let ridge_height =
        network.declare::<Offset>("basilica/roof/ridge-height", QuantityPolicy::positive())?;
    let pitch =
        network.declare::<Rational>("basilica/roof/pitch", QuantityPolicy::unrestricted())?;
    let rafter_length = network.declare::<Length>(
        "basilica/roof/principal-rafter-length",
        QuantityPolicy::positive(),
    )?;
    let overhang =
        network.declare::<Length>("basilica/roof/overhang", QuantityPolicy::positive())?;
    let overhang_drop =
        network.declare::<Length>("basilica/roof/overhang-drop", QuantityPolicy::positive())?;
    let roof_run =
        network.declare::<Length>("basilica/roof/full-run", QuantityPolicy::positive())?;
    let north_roof_run = network.declare::<Offset>(
        "basilica/roof/north-full-run",
        QuantityPolicy::unrestricted(),
    )?;
    let south_roof_run = network.declare::<Offset>(
        "basilica/roof/south-full-run",
        QuantityPolicy::unrestricted(),
    )?;
    let roof_total_rise =
        network.declare::<Length>("basilica/roof/full-rise", QuantityPolicy::positive())?;
    let roof_eave_height =
        network.declare::<Offset>("basilica/roof/eave-height", QuantityPolicy::positive())?;
    let roof_slope_length = network.declare::<Length>(
        "basilica/roof/skin-slope-length",
        QuantityPolicy::positive(),
    )?;
    let roof_skin_depth =
        network.declare::<Length>("basilica/roof/skin-depth", QuantityPolicy::positive())?;
    let wall_plate_width =
        network.declare::<Length>("basilica/roof/wall-plate-width", QuantityPolicy::positive())?;
    let principal_rafter_width = network.declare::<Length>(
        "basilica/roof/principal-rafter-width",
        QuantityPolicy::positive(),
    )?;
    let principal_rafter_depth = network.declare::<Length>(
        "basilica/roof/principal-rafter-depth",
        QuantityPolicy::positive(),
    )?;
    let principal_rafter_reveal = network.declare::<Offset>(
        "basilica/roof/principal-rafter-reveal",
        QuantityPolicy::non_negative(),
    )?;
    let north_wall_seat = network.declare::<Point3>(
        "basilica/roof/north-wall-seat",
        QuantityPolicy::unrestricted(),
    )?;
    let south_wall_seat = network.declare::<Point3>(
        "basilica/roof/south-wall-seat",
        QuantityPolicy::unrestricted(),
    )?;
    let ridge_point =
        network.declare::<Point3>("basilica/roof/ridge-point", QuantityPolicy::unrestricted())?;
    let north_roof_eave = network.declare::<Point3>(
        "basilica/roof/north-eave-point",
        QuantityPolicy::unrestricted(),
    )?;
    let south_roof_eave = network.declare::<Point3>(
        "basilica/roof/south-eave-point",
        QuantityPolicy::unrestricted(),
    )?;

    // Relation keys carry alphabetic phase prefixes only to make the intended
    // closed-form frontier obvious in diagnostics. Readiness, not the prefix,
    // remains the actual direction selector.
    network.relate(ScaleLength::new(
        "basilica/roof/a-half-span",
        span.clone(),
        half_span.clone(),
        Rational::new(1, 2).map_err(|_| BuildError::InvalidRelation)?,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/b-north-half-span",
        origin.clone(),
        half_span.clone(),
        north_half_span.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/c-south-half-span",
        origin.clone(),
        half_span.clone(),
        south_half_span.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/d-wall-plate-top",
        wall_head.clone(),
        wall_plate_height.clone(),
        wall_plate_top.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/e-ridge-height",
        wall_plate_top.clone(),
        rise.clone(),
        ridge_height.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(Pitch::new(
        "basilica/roof/f-pitch",
        half_span.clone(),
        rise.clone(),
        pitch.clone(),
    )?)?;
    network.relate(Pythagorean::new(
        "basilica/roof/g-principal-rafter",
        half_span.clone(),
        rise.clone(),
        rafter_length.clone(),
    )?)?;
    network.relate(Pitch::new(
        "basilica/roof/h-overhang-drop",
        overhang.clone(),
        overhang_drop.clone(),
        pitch.clone(),
    )?)?;
    network.relate(Sum::new(
        "basilica/roof/i-full-run",
        half_span.clone(),
        overhang.clone(),
        roof_run.clone(),
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/j-north-full-run",
        origin.clone(),
        roof_run.clone(),
        north_roof_run.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/k-south-full-run",
        origin.clone(),
        roof_run.clone(),
        south_roof_run.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/l-eave-height",
        wall_plate_top.clone(),
        overhang_drop.clone(),
        roof_eave_height.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(Sum::new(
        "basilica/roof/m-full-rise",
        rise.clone(),
        overhang_drop.clone(),
        roof_total_rise.clone(),
    )?)?;
    network.relate(Pythagorean::new(
        "basilica/roof/n-skin-slope",
        roof_run.clone(),
        roof_total_rise.clone(),
        roof_slope_length.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/o-north-wall-seat",
        origin.clone(),
        north_half_span.clone(),
        wall_plate_top.clone(),
        north_wall_seat.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/p-south-wall-seat",
        origin.clone(),
        south_half_span.clone(),
        wall_plate_top.clone(),
        south_wall_seat.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/q-ridge-point",
        origin.clone(),
        origin.clone(),
        ridge_height.clone(),
        ridge_point.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/r-north-eave",
        origin.clone(),
        north_roof_run.clone(),
        roof_eave_height.clone(),
        north_roof_eave.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/s-south-eave",
        origin.clone(),
        south_roof_run.clone(),
        roof_eave_height.clone(),
        south_roof_eave.clone(),
    )?)?;

    let quantities = RoofQuantities {
        origin,
        span,
        half_span,
        wall_head,
        wall_plate_height,
        wall_plate_top,
        rise,
        ridge_height,
        pitch,
        rafter_length,
        overhang,
        overhang_drop,
        roof_run,
        roof_eave_height,
        roof_slope_length,
        roof_skin_depth,
        wall_plate_width,
        principal_rafter_width,
        principal_rafter_depth,
        principal_rafter_reveal,
        north_wall_seat,
        south_wall_seat,
        ridge_point,
        north_roof_eave,
        south_roof_eave,
    };
    Ok((network.finish()?, quantities))
}

fn build_roots(
    definition: &NetworkDef,
    quantities: &RoofQuantities,
    params: &BasilicaParams,
) -> Result<RootClaimSet, RoofSetoutError> {
    let mut roots = RootClaimSetBuilder::new(definition);
    let (span, span_quantization) = quantize_length_meters(params.nave_width)?;
    let (wall_head, wall_quantization) = quantize_offset_meters(params.nave_wall_height)?;
    let (rise, rise_quantization) = quantize_length_meters(params.roof_rise)?;
    roots
        .author(
            "basilica/root/roof-zero",
            &quantities.origin,
            Knowledge::exact(Offset::ZERO),
        )?
        .author_quantized(
            "basilica/root/nave-span",
            &quantities.span,
            Knowledge::exact(span),
            span_quantization,
        )?
        .author_quantized(
            "basilica/root/nave-wall-head",
            &quantities.wall_head,
            Knowledge::exact(wall_head),
            wall_quantization,
        )?
        .author_quantized(
            "basilica/root/roof-rise",
            &quantities.rise,
            Knowledge::exact(rise),
            rise_quantization,
        )?
        .author(
            "basilica/root/wall-plate-height",
            &quantities.wall_plate_height,
            Knowledge::exact(exact_length_millimeters(WALL_PLATE_HEIGHT_MM)?),
        )?
        .author(
            "basilica/root/wall-plate-width",
            &quantities.wall_plate_width,
            Knowledge::exact(exact_length_millimeters(WALL_PLATE_WIDTH_MM)?),
        )?
        .author(
            "basilica/root/roof-overhang",
            &quantities.overhang,
            Knowledge::exact(exact_length_millimeters(ROOF_OVERHANG_MM)?),
        )?
        .author(
            "basilica/root/roof-skin-depth",
            &quantities.roof_skin_depth,
            Knowledge::exact(exact_length_millimeters(ROOF_SKIN_DEPTH_MM)?),
        )?
        .author(
            "basilica/root/principal-rafter-width",
            &quantities.principal_rafter_width,
            Knowledge::exact(exact_length_millimeters(PRINCIPAL_RAFTER_WIDTH_MM)?),
        )?
        .author(
            "basilica/root/principal-rafter-depth",
            &quantities.principal_rafter_depth,
            Knowledge::exact(exact_length_millimeters(PRINCIPAL_RAFTER_DEPTH_MM)?),
        )?
        .author(
            "basilica/root/principal-rafter-reveal",
            &quantities.principal_rafter_reveal,
            Knowledge::exact(exact_offset_millimeters(PRINCIPAL_RAFTER_REVEAL_MM)?),
        )?;
    Ok(roots.finish()?)
}

fn exact_length_millimeters(value: u64) -> Result<Length, ArithmeticError> {
    Length::millimeters(value).ok_or(ArithmeticError::Overflow)
}

fn exact_offset_millimeters(value: i64) -> Result<Offset, ArithmeticError> {
    Offset::millimeters(value).ok_or(ArithmeticError::Overflow)
}

fn resolve_section(
    evaluation: &Evaluation,
    quantities: &RoofQuantities,
) -> Result<RoofSection, setout::AccessError> {
    Ok(RoofSection {
        span: evaluation.exact(&quantities.span)?,
        half_span: evaluation.exact(&quantities.half_span)?,
        wall_head: evaluation.exact(&quantities.wall_head)?,
        wall_plate_height: evaluation.exact(&quantities.wall_plate_height)?,
        wall_plate_top: evaluation.exact(&quantities.wall_plate_top)?,
        rise: evaluation.exact(&quantities.rise)?,
        ridge_height: evaluation.exact(&quantities.ridge_height)?,
        pitch: evaluation.exact(&quantities.pitch)?,
        rafter_length: evaluation.exact(&quantities.rafter_length)?,
        overhang: evaluation.exact(&quantities.overhang)?,
        overhang_drop: evaluation.exact(&quantities.overhang_drop)?,
        roof_run: evaluation.exact(&quantities.roof_run)?,
        roof_eave_height: evaluation.exact(&quantities.roof_eave_height)?,
        roof_slope_length: evaluation.exact(&quantities.roof_slope_length)?,
        roof_skin_depth: evaluation.exact(&quantities.roof_skin_depth)?,
        wall_plate_width: evaluation.exact(&quantities.wall_plate_width)?,
        principal_rafter_width: evaluation.exact(&quantities.principal_rafter_width)?,
        principal_rafter_depth: evaluation.exact(&quantities.principal_rafter_depth)?,
        principal_rafter_reveal: evaluation.exact(&quantities.principal_rafter_reveal)?,
        north_wall_seat: evaluation.exact(&quantities.north_wall_seat)?,
        south_wall_seat: evaluation.exact(&quantities.south_wall_seat)?,
        ridge_point: evaluation.exact(&quantities.ridge_point)?,
        north_roof_eave: evaluation.exact(&quantities.north_roof_eave)?,
        south_roof_eave: evaluation.exact(&quantities.south_roof_eave)?,
    })
}

fn build_catalogue(
    roots: &RootClaimSet,
    plan: &PropagationPlan,
) -> Result<ReconstructionCatalogue, RoofSetoutError> {
    let mut catalogue = ReconstructionCatalogueBuilder::new();
    for (key, basis, source, limitation) in [
        (
            "basilica/root/roof-zero",
            SourceBasis::Observed,
            "basilica-local coordinate datum",
            "coordinate convention, not historical fabric",
        ),
        (
            "basilica/root/nave-span",
            SourceBasis::Documented,
            "accepted basilica massing",
            "working reconstruction dimension",
        ),
        (
            "basilica/root/nave-wall-head",
            SourceBasis::Documented,
            "accepted basilica clerestory massing",
            "working reconstruction datum",
        ),
        (
            "basilica/root/roof-rise",
            SourceBasis::RegionalAnalogy,
            "hagia-paraskevi-roof-survey",
            "analogy supplies pitch, not direct evidence for this building",
        ),
        (
            "basilica/root/wall-plate-height",
            SourceBasis::RegionalAnalogy,
            "hagia-paraskevi-roof-survey",
            "timber section is a reconstruction choice",
        ),
        (
            "basilica/root/wall-plate-width",
            SourceBasis::RegionalAnalogy,
            "hagia-paraskevi-roof-survey",
            "timber section is a reconstruction choice",
        ),
        (
            "basilica/root/roof-overhang",
            SourceBasis::RegionalAnalogy,
            "regional tiled-roof practice",
            "eave projection is not directly observed",
        ),
        (
            "basilica/root/roof-skin-depth",
            SourceBasis::ModernInference,
            "accepted visual roof build-up",
            "aggregated skin thickness, not a fabric claim",
        ),
        (
            "basilica/root/principal-rafter-width",
            SourceBasis::ModernInference,
            "tfec-modern-truss-detailing",
            "modern section sizing vocabulary",
        ),
        (
            "basilica/root/principal-rafter-depth",
            SourceBasis::ModernInference,
            "tfec-modern-truss-detailing",
            "modern section sizing vocabulary",
        ),
        (
            "basilica/root/principal-rafter-reveal",
            SourceBasis::ModernInference,
            "accepted visual legibility decision",
            "rendering clearance, not historical evidence",
        ),
    ] {
        let key = setout::RootClaimKey::new(key)?;
        if roots.claim_key(&key).is_some() {
            catalogue = catalogue.root(key, SourceRef::new(source, basis, source, limitation))?;
        }
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

fn build_binding_index(quantities: &RoofQuantities) -> BindingIndex {
    let mut bindings = BindingIndex::new();
    let skin_quantities = [
        quantities.north_roof_eave.key().clone(),
        quantities.south_roof_eave.key().clone(),
        quantities.ridge_point.key().clone(),
        quantities.roof_slope_length.key().clone(),
        quantities.roof_skin_depth.key().clone(),
    ];
    for element in [
        "nave-roof-north-west",
        "nave-roof-south-west-a",
        "nave-roof-south-west-b",
        "nave-roof-north-east",
        "nave-roof-south-east",
    ] {
        bindings.bind(
            element,
            skin_quantities.iter().cloned(),
            [DirtyChannel::Geometry, DirtyChannel::Contact],
        );
    }
    let plate_quantities = [
        quantities.half_span.key().clone(),
        quantities.wall_head.key().clone(),
        quantities.wall_plate_top.key().clone(),
        quantities.wall_plate_height.key().clone(),
        quantities.wall_plate_width.key().clone(),
    ];
    for element in [
        "nave-wall-plate-north-west",
        "nave-wall-plate-south-west-a",
        "nave-wall-plate-south-west-b",
        "nave-wall-plate-north-east",
        "nave-wall-plate-south-east",
        "wall-plate-north",
        "wall-plate-south",
    ] {
        bindings.bind(
            element,
            plate_quantities.iter().cloned(),
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
    }
    let rafter_quantities = [
        quantities.north_wall_seat.key().clone(),
        quantities.south_wall_seat.key().clone(),
        quantities.ridge_point.key().clone(),
        quantities.rafter_length.key().clone(),
        quantities.principal_rafter_width.key().clone(),
        quantities.principal_rafter_depth.key().clone(),
        quantities.principal_rafter_reveal.key().clone(),
    ];
    for station in [
        "nave-truss-west-00",
        "nave-truss-west-01",
        "nave-truss-west-03",
        "nave-truss-west-04",
        "nave-truss-west-05",
        "nave-truss-east-00",
    ] {
        for side in ["north", "south"] {
            bindings.bind(
                format!("{station}-principal-rafter-{side}"),
                rafter_quantities.iter().cloned(),
                [
                    DirtyChannel::Geometry,
                    DirtyChannel::Contact,
                    DirtyChannel::LoadPath,
                ],
            );
        }
        bindings.bind(
            format!("{station}-tie-beam"),
            [
                quantities.span.key().clone(),
                quantities.half_span.key().clone(),
                quantities.wall_head.key().clone(),
                quantities.wall_plate_top.key().clone(),
            ],
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
        bindings.bind(
            format!("{station}-king-post"),
            [
                quantities.wall_head.key().clone(),
                quantities.wall_plate_top.key().clone(),
                quantities.ridge_height.key().clone(),
                quantities.pitch.key().clone(),
            ],
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
    }
    for element in [
        "west-facade",
        "crossing-shoulder-west",
        "crossing-shoulder-east",
        "east-chancel-gable",
    ] {
        bindings.bind(
            element,
            [
                quantities.span.key().clone(),
                quantities.wall_plate_top.key().clone(),
                quantities.ridge_height.key().clone(),
            ],
            [DirtyChannel::Geometry],
        );
    }

    // The crossing's vertical datum is intentionally tied to the nave ridge.
    // A rise edit therefore moves the whole bearing stage, not merely the dome
    // skin; omitting these names would advertise an unsafe partial rebuild.
    let crossing_quantities = [quantities.ridge_height.key().clone()];
    for element in [
        "crossing-pier-south-west",
        "crossing-pier-north-west",
        "crossing-pier-south-east",
        "crossing-pier-north-east",
        "crossing-spandrel-south",
        "crossing-spandrel-north",
        "crossing-spandrel-west",
        "crossing-spandrel-east",
        "crossing-platform",
        "crossing-drum-cornice-base",
        "crossing-drum-cornice-top",
        "crossing-dome",
        "crossing-pendentive-north-east",
        "crossing-pendentive-north-west",
        "crossing-pendentive-south-west",
        "crossing-pendentive-south-east",
    ] {
        bindings.bind(
            element,
            crossing_quantities.iter().cloned(),
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
    }
    for face in 0..12 {
        bindings.bind(
            format!("crossing-drum-panel-{face:02}"),
            crossing_quantities.iter().cloned(),
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
    }

    // The structure lab is a second real consumer of the same exact roof
    // spine. These mappings are grouped by construction role so a delta names
    // the complete affected assembly while leaving the east apse unrelated.
    for side in ["south", "north"] {
        bindings.bind(
            format!("masonry-{side}"),
            [
                quantities.half_span.key().clone(),
                quantities.wall_head.key().clone(),
            ],
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
        let eave = if side == "south" {
            quantities.south_roof_eave.key().clone()
        } else {
            quantities.north_roof_eave.key().clone()
        };
        let roof_quantities = [
            eave,
            quantities.ridge_point.key().clone(),
            quantities.roof_slope_length.key().clone(),
            quantities.roof_skin_depth.key().clone(),
        ];
        for element in [
            format!("roof-covering-{side}"),
            format!("boarding-{side}"),
            format!("common-rafter-{side}-00"),
            format!("common-rafter-{side}-01"),
            format!("common-rafter-{side}-02"),
            format!("purlin-{side}-eave"),
            format!("purlin-{side}-mid"),
            format!("purlin-{side}-upper"),
        ] {
            bindings.bind(
                element,
                roof_quantities.iter().cloned(),
                [
                    DirtyChannel::Geometry,
                    DirtyChannel::Contact,
                    DirtyChannel::LoadPath,
                ],
            );
        }
    }
    let lab_frame_quantities = [
        quantities.span.key().clone(),
        quantities.half_span.key().clone(),
        quantities.wall_head.key().clone(),
        quantities.wall_plate_top.key().clone(),
        quantities.ridge_height.key().clone(),
        quantities.pitch.key().clone(),
        quantities.rafter_length.key().clone(),
    ];
    for station in ["west", "east"] {
        for element in [
            format!("tie-beam-{station}"),
            format!("principal-rafter-south-{station}"),
            format!("principal-rafter-north-{station}"),
            format!("king-post-{station}"),
            format!("strut-south-{station}"),
            format!("strut-north-{station}"),
        ] {
            bindings.bind(
                element,
                lab_frame_quantities.iter().cloned(),
                [
                    DirtyChannel::Geometry,
                    DirtyChannel::Contact,
                    DirtyChannel::LoadPath,
                ],
            );
        }
    }
    bindings
}

/// Failure to build, resolve, or reconfigure the basilica roof hypothesis.
#[derive(Debug)]
#[non_exhaustive]
pub enum RoofSetoutError {
    /// Network definition is invalid.
    Build(BuildError),
    /// A legacy floating parameter cannot be imported.
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
    /// Warm and fresh evaluations did not agree.
    WarmFreshMismatch,
    /// A stable semantic key is invalid.
    Key(setout::KeyError),
}

macro_rules! error_from {
    ($source:ty, $variant:ident) => {
        impl From<$source> for RoofSetoutError {
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

impl fmt::Display for RoofSetoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => write!(formatter, "roof definition failed: {error}"),
            Self::Arithmetic(error) => write!(formatter, "roof import failed: {error}"),
            Self::Roots(error) => write!(formatter, "roof roots failed: {error}"),
            Self::Scenario(error) => write!(formatter, "roof scenario failed: {error}"),
            Self::Plan(error) => write!(formatter, "roof plan failed: {error}"),
            Self::Evaluation(error) => write!(formatter, "roof evaluation failed: {error}"),
            Self::Access(error) => write!(formatter, "roof access failed: {error}"),
            Self::Catalogue(error) => write!(formatter, "roof catalogue failed: {error}"),
            Self::WarmFreshMismatch => {
                formatter.write_str("warm roof evaluation differs from fresh")
            }
            Self::Key(error) => write!(formatter, "roof key failed: {error}"),
        }
    }
}

impl std::error::Error for RoofSetoutError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roof_skin_passes_through_the_wall_plate_and_overhangs_below_it() {
        // The previous visual roof was pitched from its outer overhang directly
        // to the ridge. At the wall line it merely happened to pass near the
        // timber. This test guards the corrected construction logic: the exact
        // slope passes through the wall-plate top, then continues outward and
        // down by the same rational pitch.
        let setout = BasilicaRoofSetout::new(&BasilicaParams::default()).unwrap();
        let roof = setout.section();
        assert_eq!(roof.wall_plate_top, Offset::millimeters(11_180).unwrap());
        assert_eq!(roof.ridge_height, Offset::millimeters(14_380).unwrap());
        assert_eq!(roof.pitch, Rational::new(32, 45).unwrap());
        assert_eq!(
            roof.overhang_drop,
            Length::from_iota(2_240_000_000).unwrap()
        );
        assert_eq!(
            roof.roof_eave_height
                .checked_add_length(roof.overhang_drop)
                .unwrap(),
            roof.wall_plate_top
        );
        assert!(
            setout
                .explain_ridge()
                .contains("basilica/root/nave-wall-head")
        );
    }

    #[test]
    fn principal_rafter_is_lowered_once_from_exact_endpoints() {
        // This is the complete Setout→Joiner seam. The analytic extent, its
        // frame bits, and every provenance link come from the evaluated binding;
        // the test does not author a second world-space endpoint.
        let setout = BasilicaRoofSetout::new(&BasilicaParams::default()).unwrap();
        for side in [RoofSide::North, RoofSide::South] {
            let geometry = setout.principal_rafter_geometry(side).unwrap();
            assert!(geometry.extent.is_well_formed());
            assert_eq!(geometry.bindings.len(), 5);
            assert_eq!(
                geometry.extent.size[0],
                setout.section().rafter_length.as_meters()
            );
        }
    }

    #[test]
    fn roof_rise_edit_is_incremental_exact_and_excludes_unrelated_systems() {
        // A roof edit must recompute its dependent spine, report named geometry,
        // carry the crossing whose datum follows the ridge, and leave the apse
        // outside the dirty frontier. Warm/fresh equality is checked inside
        // `reconfigure` before the result is returned.
        let baseline = BasilicaRoofSetout::new(&BasilicaParams::default()).unwrap();
        let mut edited = BasilicaParams::default();
        edited.roof_rise += 0.25;
        let change = baseline.reconfigure(&edited).unwrap();
        assert_eq!(change.section.rise, Length::millimeters(3_450).unwrap());
        assert!(change.work.steps_reused > 0);
        assert!(change.work.steps_evaluated > 0);
        let dirty: Vec<_> = change
            .dirty
            .iter()
            .map(|item| item.element.as_ref())
            .collect();
        assert!(dirty.contains(&"nave-roof-north-west"));
        assert!(dirty.contains(&"east-chancel-gable"));
        assert!(dirty.contains(&"crossing-dome"));
        assert!(dirty.contains(&"principal-rafter-south-west"));
        assert!(!dirty.iter().any(|key| key.contains("apse")));
    }
}
