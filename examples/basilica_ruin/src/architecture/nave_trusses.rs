// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_assembly::{PartId, compose as compose_placement};
use exedra_constructive::ir::{Placement3, Plane3, Recipe};
use exedra_math::{add, cross, norm, normalize, scale, sub};
use joiner::{
    Construction, Element, Evidence, EvidenceClass, EvidenceSource, Member, Node, OrientedBox,
    Relation, RelationKind, Rule, RuleApplication, RuleContext, compose,
};
use joiner_timber::{
    HeelParams, HeelRule, HousedBearingParams, KingPostTieParams, KingPostTieRule, Length,
    RafterToKingPostRule, StrutToKingPostRule, StrutToRafterRule,
};
use setout_joiner::{ResolvedElementGeometry, lower_offset, lower_rational_iotas};

use super::BuildContext;
use crate::{
    BasilicaSetout, NAVE_TRUSS_EAST_STATION_KEY, PlanSection, RoofSide, names,
    truss_member_instance_key, west_truss_station_key,
};

const TIE_WIDTH: f64 = 0.30;
const TIE_DEPTH: f64 = 0.34;
const TIE_END_RELISH: f64 = 0.18;
const KING_POST_WIDTH: f64 = 0.36;
const BRACE_WIDTH: f64 = 0.20;
const BRACE_DEPTH: f64 = 0.18;
const STRUT_FOOT_ABOVE_TIE: f64 = 0.35;
const STRUT_POST_HOUSING_DEPTH: Length = length_millimeters(80);
const STRUT_RAFTER_HOUSING_DEPTH: Length = length_millimeters(40);
const RAFTER_HEAD_HOUSING_DEPTH: Length = length_millimeters(120);
const STRUT_CARRIER_RELISH: Length = length_millimeters(10);
const RAFTER_HEAD_CARRIER_RELISH: Length = length_millimeters(5);

const fn length_millimeters(value: u64) -> Length {
    match Length::millimeters(value) {
        Some(value) => value,
        None => panic!("authored basilica dimensions must be positive millimeters"),
    }
}

struct MemberParts {
    tie: PartId,
    rafter_north: PartId,
    rafter_south: PartId,
    king_post: PartId,
    brace_north: PartId,
    brace_south: PartId,
    key: PartId,
    key_frame: Placement3,
}

#[derive(Copy, Clone)]
struct TrussMemberTemplate {
    key_suffix: &'static str,
    part: PartId,
    placement: Placement3,
}

struct FittedMemberRecipes {
    tie: Recipe,
    rafter_north: Recipe,
    rafter_south: Recipe,
    king_post: Recipe,
    brace_north: Recipe,
    brace_south: Recipe,
    key: Recipe,
    key_frame: Placement3,
    #[cfg(test)]
    authored_rafter_south: Recipe,
    #[cfg(test)]
    authored_brace_south: Recipe,
}

struct TrussGeometry {
    half_nave: f64,
    #[cfg(test)]
    roof_sin: f64,
    #[cfg(test)]
    roof_cos: f64,
    #[cfg(test)]
    roof_peak: f64,
    rafter_length: f64,
    rafter_width: f64,
    rafter_depth: f64,
    #[cfg(test)]
    roof_clearance: f64,
    north_rafter_frame: Placement3,
    south_rafter_frame: Placement3,
    tie_base: f64,
    king_post_base: f64,
    king_post_height: f64,
    brace_length: f64,
    north_brace_frame: Placement3,
    south_brace_frame: Placement3,
}

/// Adds the open timber frames that visibly carry the two nave roof slopes.
///
/// Each principal rafter is lowered through `setout_joiner` from the exact wall
/// seat and ridge claims. The visible clearance is then applied once, normal to
/// that resolved extent. The missing west slot repeats the authored south-west
/// clerestory and roof loss rather than placing intact structure beneath it.
pub(super) fn build(context: &mut BuildContext, plan: &PlanSection, setout: &BasilicaSetout) {
    let geometry = TrussGeometry::from_setout(setout);
    let parts = add_member_parts(context, &geometry);
    let members = member_templates(&parts, &geometry);

    // `setout_generate` owns the repeated station identities and exact
    // coordinates; this adapter owns the one-to-seven assembly expansion. A
    // station remains a topology item rather than becoming a procedural
    // assembly node, so every timber member stays independently addressable.
    for station in setout.west_truss_stations().items() {
        instantiate_station(
            context,
            &west_truss_station_key(station.label()),
            lower_rational_iotas(station.position()),
            &members,
        );
    }
    // The east segment has one authored survivor, not a repeat. Giving it an
    // exact named datum avoids misrepresenting a singleton as a degenerate
    // linear invocation merely to reuse the generator API.
    instantiate_station(
        context,
        NAVE_TRUSS_EAST_STATION_KEY,
        lower_offset(plan.nave_truss_east),
        &members,
    );
}

fn instantiate_station(
    context: &mut BuildContext,
    station_key: &str,
    x: f64,
    members: &[TrussMemberTemplate],
) {
    let station_placement = Placement3::translate(x, 0.0, 0.0);
    for member in members {
        context.add_instance(
            &truss_member_instance_key(station_key, member.key_suffix),
            member.part,
            compose_placement(&station_placement, &member.placement),
            names::roles::NAVE_TRUSS_MEMBER,
        );
    }
}

impl TrussGeometry {
    fn from_setout(setout: &BasilicaSetout) -> Self {
        let roof = setout.roof();
        let north = setout
            .principal_rafter_geometry(RoofSide::North)
            .expect("accepted north rafter binding resolves");
        let south = setout
            .principal_rafter_geometry(RoofSide::South)
            .expect("accepted south rafter binding resolves");
        let roof_clearance = roof.principal_rafter_reveal.as_meters();
        let north_rafter_frame = recessed_member_frame(&north, roof_clearance);
        let south_rafter_frame = recessed_member_frame(&south, roof_clearance);
        let rafter_width = north.extent.size[1];
        let rafter_depth = north.extent.size[2];
        let tie_base = roof.wall_plate_top.as_meters() - TIE_DEPTH;
        let tie_top = tie_base + TIE_DEPTH;
        let king_tenon_length = KingPostTieParams::default().tenon_length.as_meters();
        let rafter_head_housing_depth = RAFTER_HEAD_HOUSING_DEPTH.as_meters();
        let strut_post_housing_depth = STRUT_POST_HOUSING_DEPTH.as_meters();
        let strut_rafter_housing_depth = STRUT_RAFTER_HOUSING_DEPTH.as_meters();
        // The extent includes the rule's male tenon. Its tip is one tenon
        // length below the tie top, so the derived shoulder lands exactly on
        // that face and the tenon occupies the matching mortise.
        let king_post_base = tie_top - king_tenon_length;

        // The resolved rafters originally run to the exact ridge claim, then
        // the visible-reveal offset moves that line sideways as well as down.
        // Derive each setback from the recessed endpoint and the desired
        // shoulder coordinate rather than assuming it still lies on y=0.
        let full_length = north.extent.size[0];
        let north_full = extent_from_placement(
            north_rafter_frame,
            [full_length, rafter_width, rafter_depth],
        );
        let south_full = extent_from_placement(
            south_rafter_frame,
            [full_length, rafter_width, rafter_depth],
        );
        let full_head = |rafter: &OrientedBox| {
            rafter.anchor([full_length, rafter_width * 0.5, rafter_depth * 0.5])
        };
        let desired_north_y =
            KING_POST_WIDTH * 0.5 - north_full.axes[0][1].abs() * rafter_head_housing_depth;
        let desired_south_y = -desired_north_y;
        let north_setback = (full_head(&north_full)[1] - desired_north_y) / north_full.axes[0][1];
        let south_setback = (full_head(&south_full)[1] - desired_south_y) / south_full.axes[0][1];
        assert!(
            (north_setback - south_setback).abs() < 1.0e-12,
            "symmetric roof setout must produce paired rafter-head setbacks: {north_setback} != {south_setback}"
        );
        assert!(
            north_setback > 0.0 && north_setback < full_length,
            "rafter-head setback must lie inside the resolved rafter: {north_setback} of {full_length}"
        );
        let rafter_length = full_length - north_setback;
        let north_rafter = extent_from_placement(
            north_rafter_frame,
            [rafter_length, rafter_width, rafter_depth],
        );
        let south_rafter = extent_from_placement(
            south_rafter_frame,
            [rafter_length, rafter_width, rafter_depth],
        );
        let north_head =
            north_rafter.anchor([rafter_length, rafter_width * 0.5, rafter_depth * 0.5]);
        let south_head =
            south_rafter.anchor([rafter_length, rafter_width * 0.5, rafter_depth * 0.5]);
        let head_vertical_radius = north_rafter.axes[1][2].abs() * rafter_width * 0.5
            + north_rafter.axes[2][2].abs() * rafter_depth * 0.5;
        // Relish above the complete oblique rafter section keeps both deep
        // head pockets enclosed instead of clipping them at the post top.
        let king_post_top = north_head[2].max(south_head[2]) + head_vertical_radius + 0.01;
        let king_post_height = king_post_top - king_post_base;

        // A strut endpoint is the internal bearing shoulder, not the visible
        // carrier surface. Offsetting both ends by their housing depths keeps
        // the member straight while making the rule's setout contract exact.
        let brace_geometry = |rafter: &OrientedBox, side: f64| {
            let target_surface = rafter.anchor([rafter_length * 0.58, rafter_width * 0.5, 0.0]);
            let foot_surface = [
                0.0,
                side * KING_POST_WIDTH * 0.5,
                tie_top + STRUT_FOOT_ABOVE_TIE,
            ];
            let axis = normalize(sub(target_surface, foot_surface))
                .expect("strut bearing surfaces are distinct");
            let foot = sub(foot_surface, scale(axis, strut_post_housing_depth));
            let target = add(target_surface, scale(axis, strut_rafter_housing_depth));
            let frame = member_frame_between(foot, target, BRACE_WIDTH, BRACE_DEPTH);
            (norm(sub(target, foot)), frame)
        };
        let (north_brace_length, north_brace_frame) = brace_geometry(&north_rafter, 1.0);
        let (south_brace_length, south_brace_frame) = brace_geometry(&south_rafter, -1.0);
        assert!(
            (north_brace_length - south_brace_length).abs() < 1.0e-12,
            "symmetric roof setout must produce paired brace lengths: {north_brace_length} != {south_brace_length}"
        );

        Self {
            half_nave: roof.half_span.as_meters(),
            #[cfg(test)]
            roof_sin: north.extent.axes[0][2],
            #[cfg(test)]
            roof_cos: -north.extent.axes[0][1],
            #[cfg(test)]
            roof_peak: roof.ridge_height.as_meters(),
            rafter_length,
            rafter_width,
            rafter_depth,
            #[cfg(test)]
            roof_clearance,
            north_rafter_frame,
            south_rafter_frame,
            tie_base,
            king_post_base,
            king_post_height,
            brace_length: north_brace_length,
            north_brace_frame,
            south_brace_frame,
        }
    }
}

fn member_frame_between(from: [f64; 3], to: [f64; 3], width: f64, depth: f64) -> Placement3 {
    let along = normalize(sub(to, from)).expect("authored member has positive length");
    let across = [1.0, 0.0, 0.0];
    let normal = normalize(cross(along, across)).expect("member is not parallel to world x");
    let origin = sub(
        sub(from, scale(across, width * 0.5)),
        scale(normal, depth * 0.5),
    );
    let mut placement = Placement3::from_axes(along, across, normal, origin);
    for value in placement.rows.iter_mut().flatten() {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    placement
}

fn recessed_member_frame(resolved: &ResolvedElementGeometry, clearance: f64) -> Placement3 {
    let extent = &resolved.extent;
    // SegmentMemberBinding centers depth on the exact endpoint line. Moving
    // inward by half the member depth plus the reveal places its outer face at
    // exactly the specified clearance beneath the roof underside.
    let origin = sub(
        extent.origin,
        scale(extent.axes[2], extent.size[2] * 0.5 + clearance),
    );
    let mut placement =
        Placement3::from_axes(extent.axes[0], extent.axes[1], extent.axes[2], origin);
    // Matrix composition and direct placement must agree bit-for-bit for the
    // assembly fingerprint. IEEE -0.0 is geometrically identical but hashes
    // differently, so normalize only signed zero at this lowering boundary.
    for value in placement.rows.iter_mut().flatten() {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    placement
}

fn add_member_parts(context: &mut BuildContext, geometry: &TrussGeometry) -> MemberParts {
    let fitted = fitted_member_recipes(geometry);
    MemberParts {
        tie: context.add_part(names::parts::NAVE_TRUSS_TIE_BEAM, fitted.tie, "aged-timber"),
        rafter_north: context.add_part(
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER,
            fitted.rafter_north,
            "aged-timber",
        ),
        rafter_south: context.add_part(
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER_SOUTH,
            fitted.rafter_south,
            "aged-timber",
        ),
        king_post: context.add_part(
            names::parts::NAVE_TRUSS_KING_POST,
            fitted.king_post,
            "aged-timber",
        ),
        brace_north: context.add_part(
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE,
            fitted.brace_north,
            "aged-timber",
        ),
        brace_south: context.add_part(
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE_SOUTH,
            fitted.brace_south,
            "aged-timber",
        ),
        key: context.add_part(
            names::parts::NAVE_TRUSS_KING_POST_KEY,
            fitted.key,
            "aged-timber",
        ),
        key_frame: fitted.key_frame,
    }
}

/// Composes one canonical station before the recipes enter the repeated
/// assembly. The graph exists only at this integration seam: assembly still
/// shares seven immutable parts, while both sides of every joint come from the
/// same rule output rather than from independently authored solids.
fn fitted_member_recipes(geometry: &TrussGeometry) -> FittedMemberRecipes {
    const SOURCE: &str = "basilica-truss-rule-assumption";
    let evidence = Evidence::new(SOURCE, EvidenceClass::ModernEngineeringInference);
    let mut construction = Construction::new();
    construction
        .add_evidence_source(EvidenceSource::new(
            SOURCE,
            EvidenceClass::ModernEngineeringInference,
            "urn:exedra:basilica-ruin:nave-truss-rules",
            "Deterministic reconstruction fixture; not a claim about surviving joinery",
        ))
        .expect("fresh evidence source");

    let tie = extent_from_placement(
        tie_frame(geometry),
        [
            geometry.half_nave * 2.0 + 2.0 * TIE_END_RELISH,
            TIE_WIDTH,
            TIE_DEPTH,
        ],
    );
    let north = extent_from_placement(
        rafter_frame(geometry, true),
        [
            geometry.rafter_length,
            geometry.rafter_width,
            geometry.rafter_depth,
        ],
    );
    let south = extent_from_placement(
        rafter_frame(geometry, false),
        [
            geometry.rafter_length,
            geometry.rafter_width,
            geometry.rafter_depth,
        ],
    );
    let king = extent_from_placement(
        king_post_frame(geometry),
        [geometry.king_post_height, KING_POST_WIDTH, KING_POST_WIDTH],
    );
    let brace_north = extent_from_placement(
        brace_frame(geometry, true),
        [geometry.brace_length, BRACE_WIDTH, BRACE_DEPTH],
    );
    let brace_south = extent_from_placement(
        brace_frame(geometry, false),
        [geometry.brace_length, BRACE_WIDTH, BRACE_DEPTH],
    );

    for (key, role, extent) in [
        ("tie", "tie-beam", tie.clone()),
        ("rafter-north", "principal-rafter", north.clone()),
        ("rafter-south", "principal-rafter", south.clone()),
        ("king-post", "king-post", king.clone()),
        ("brace-north", "strut", brace_north.clone()),
        ("brace-south", "strut", brace_south.clone()),
    ] {
        construction
            .add_element(
                Element::new(key, role, "aged-timber", extent, evidence.clone()).with_member(),
            )
            .expect("canonical truss element is unique");
    }

    let north_heel = north.anchor([0.0, north.size[1] * 0.5, north.size[2] * 0.5]);
    let south_heel = south.anchor([0.0, south.size[1] * 0.5, south.size[2] * 0.5]);
    let north_head = north.anchor([north.size[0], north.size[1] * 0.5, north.size[2] * 0.5]);
    let south_head = south.anchor([south.size[0], south.size[1] * 0.5, south.size[2] * 0.5]);
    let king_tip = king.anchor([0.0, king.size[1] * 0.5, king.size[2] * 0.5]);
    let king_shoulder = [0.0, 0.0, geometry.tie_base + TIE_DEPTH];
    let king_top = king.anchor([king.size[0], king.size[1] * 0.5, king.size[2] * 0.5]);
    let brace_north_foot =
        brace_north.anchor([0.0, brace_north.size[1] * 0.5, brace_north.size[2] * 0.5]);
    let brace_north_head = brace_north.anchor([
        brace_north.size[0],
        brace_north.size[1] * 0.5,
        brace_north.size[2] * 0.5,
    ]);
    let brace_south_foot =
        brace_south.anchor([0.0, brace_south.size[1] * 0.5, brace_south.size[2] * 0.5]);
    let brace_south_head = brace_south.anchor([
        brace_south.size[0],
        brace_south.size[1] * 0.5,
        brace_south.size[2] * 0.5,
    ]);
    for (key, point) in [
        ("heel-north", north_heel),
        ("heel-south", south_heel),
        ("head-north", north_head),
        ("head-south", south_head),
        ("king-tip", king_tip),
        ("king-shoulder", king_shoulder),
        ("king-top", king_top),
        ("brace-north-foot", brace_north_foot),
        ("brace-north-head", brace_north_head),
        ("brace-south-foot", brace_south_foot),
        ("brace-south-head", brace_south_head),
    ] {
        construction
            .add_node(Node::new(key, point))
            .expect("canonical truss node is unique");
    }
    // The tie has one identity and one centreline. Its king-post relation uses
    // a separate point on the top face; relation incidence is tested against
    // the analytic extent while the member still runs honestly heel-to-heel.
    construction
        .add_member(Member::new(
            "tie-heels",
            "tie",
            "heel-south",
            "heel-north",
            evidence.clone(),
        ))
        .expect("heel centreline is unique");
    for (member, element, from, to) in [
        ("rafter-north", "rafter-north", "heel-north", "head-north"),
        ("rafter-south", "rafter-south", "heel-south", "head-south"),
        ("king-post", "king-post", "king-tip", "king-top"),
        (
            "brace-north",
            "brace-north",
            "brace-north-foot",
            "brace-north-head",
        ),
        (
            "brace-south",
            "brace-south",
            "brace-south-foot",
            "brace-south-head",
        ),
    ] {
        construction
            .add_member(Member::new(member, element, from, to, evidence.clone()))
            .expect("carried member is unique");
    }

    for (relation, node, participants, detail) in [
        (
            "heel-north",
            "heel-north",
            &["tie-heels", "rafter-north"][..],
            "housed-heel",
        ),
        (
            "heel-south",
            "heel-south",
            &["rafter-south", "tie-heels"][..],
            "housed-heel",
        ),
        (
            "king-post-tie",
            "king-shoulder",
            &["tie-heels", "king-post"][..],
            "keyed-through-tenon",
        ),
        (
            "brace-north-foot",
            "brace-north-foot",
            &["king-post", "brace-north"][..],
            "housed-bearing",
        ),
        (
            "brace-south-foot",
            "brace-south-foot",
            &["brace-south", "king-post"][..],
            "housed-bearing",
        ),
        (
            "brace-north-head",
            "brace-north-head",
            &["rafter-north", "brace-north"][..],
            "housed-bearing",
        ),
        (
            "brace-south-head",
            "brace-south-head",
            &["brace-south", "rafter-south"][..],
            "housed-bearing",
        ),
        (
            "rafter-north-head",
            "head-north",
            &["king-post", "rafter-north"][..],
            "housed-bearing",
        ),
        (
            "rafter-south-head",
            "head-south",
            &["rafter-south", "king-post"][..],
            "housed-bearing",
        ),
    ] {
        construction
            .add_relation(Relation::new(
                relation,
                RelationKind::member_member(node, participants),
                detail,
                evidence.clone(),
            ))
            .expect("canonical truss relation is unique");
    }

    apply_rule(
        &mut construction,
        "fit-heel-north",
        "heel-north",
        &HeelRule,
        &HeelParams::default(),
        &evidence,
    );
    apply_rule(
        &mut construction,
        "fit-heel-south",
        "heel-south",
        &HeelRule,
        &HeelParams::default(),
        &evidence,
    );
    apply_rule(
        &mut construction,
        "fit-king-post-tie",
        "king-post-tie",
        &KingPostTieRule,
        &KingPostTieParams::default(),
        &evidence,
    );
    let strut_post_params = HousedBearingParams {
        housing_depth: STRUT_POST_HOUSING_DEPTH,
        minimum_carrier_relish: STRUT_CARRIER_RELISH,
        ..HousedBearingParams::default()
    };
    let strut_rafter_params = HousedBearingParams {
        housing_depth: STRUT_RAFTER_HOUSING_DEPTH,
        minimum_carrier_relish: STRUT_CARRIER_RELISH,
        ..HousedBearingParams::default()
    };
    let rafter_head_params = HousedBearingParams {
        housing_depth: RAFTER_HEAD_HOUSING_DEPTH,
        minimum_carrier_relish: RAFTER_HEAD_CARRIER_RELISH,
        ..HousedBearingParams::default()
    };
    for side in ["north", "south"] {
        apply_rule(
            &mut construction,
            &format!("fit-brace-{side}-foot"),
            &format!("brace-{side}-foot"),
            &StrutToKingPostRule,
            &strut_post_params,
            &evidence,
        );
        apply_rule(
            &mut construction,
            &format!("fit-brace-{side}-head"),
            &format!("brace-{side}-head"),
            &StrutToRafterRule,
            &strut_rafter_params,
            &evidence,
        );
        apply_rule(
            &mut construction,
            &format!("fit-rafter-{side}-head"),
            &format!("rafter-{side}-head"),
            &RafterToKingPostRule,
            &rafter_head_params,
            &evidence,
        );
    }

    let tie = compose_element(&construction, "tie");
    let north = compose_element(&construction, "rafter-north");
    #[cfg(test)]
    let authored_rafter_south = compose_element(&construction, "rafter-south");
    // Existing proper-rigid station frames differ from a world-Y reflection
    // by a local reflection across the section's mid-width. Put that
    // handedness in constructive geometry, where Mirror repairs winding,
    // rather than in an assembly transform or an exporter-specific fix.
    let south = north
        .mirrored(Plane3 {
            normal: [0.0, 1.0, 0.0],
            distance: geometry.rafter_width * 0.5,
        })
        .expect("rafter mid-width plane is finite");
    let brace_north = compose_element(&construction, "brace-north");
    #[cfg(test)]
    let authored_brace_south = compose_element(&construction, "brace-south");
    // The brace frames use a shared world-X section axis, so their world-Y
    // reflection appears across local depth instead. Mirroring through its
    // mid-depth keeps the canonical box and any future asymmetric edits in
    // the expected south-side position.
    let brace_south = brace_north
        .mirrored(Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: BRACE_DEPTH * 0.5,
        })
        .expect("brace mid-depth plane is finite");
    let key = construction
        .element("king-post-tie-key")
        .expect("keyed rule generates the transverse key");
    let mut key_frame = key.extent.placement();
    for value in key_frame.rows.iter_mut().flatten() {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    FittedMemberRecipes {
        tie,
        rafter_north: north,
        rafter_south: south,
        king_post: compose_element(&construction, "king-post"),
        brace_north,
        brace_south,
        key: compose_element(&construction, "king-post-tie-key"),
        key_frame,
        #[cfg(test)]
        authored_rafter_south,
        #[cfg(test)]
        authored_brace_south,
    }
}

fn apply_rule<R: Rule>(
    construction: &mut Construction,
    application: &str,
    relation: &str,
    rule: &R,
    params: &R::Params,
    evidence: &Evidence,
) {
    let output = {
        let context = RuleContext::new(construction, relation).expect("fresh relation resolves");
        let applicability = rule.assess(&context);
        assert!(
            applicability.is_suitable(),
            "{} refused {relation}: {:?}",
            rule.key(),
            applicability.rejections()
        );
        rule.instantiate(&context, params)
            .unwrap_or_else(|error| panic!("{} failed on {relation}: {error}", rule.key()))
    };
    construction
        .apply(RuleApplication::new(
            application,
            rule.key(),
            relation,
            evidence.clone(),
            output,
        ))
        .expect("assessed rule output merges atomically");
}

fn compose_element(construction: &Construction, key: &str) -> Recipe {
    compose(
        construction,
        construction
            .element(key)
            .unwrap_or_else(|| panic!("missing canonical truss element {key}")),
    )
    .unwrap_or_else(|error| panic!("could not compose canonical truss element {key}: {error}"))
}

fn extent_from_placement(placement: Placement3, size: [f64; 3]) -> OrientedBox {
    let rows = placement.rows;
    OrientedBox {
        origin: [rows[0][3], rows[1][3], rows[2][3]],
        axes: [
            [rows[0][0], rows[1][0], rows[2][0]],
            [rows[0][1], rows[1][1], rows[2][1]],
            [rows[0][2], rows[1][2], rows[2][2]],
        ],
        size,
    }
}

fn member_templates(parts: &MemberParts, geometry: &TrussGeometry) -> [TrussMemberTemplate; 7] {
    [
        TrussMemberTemplate {
            key_suffix: "tie-beam",
            part: parts.tie,
            placement: tie_frame(geometry),
        },
        TrussMemberTemplate {
            key_suffix: "principal-rafter-north",
            part: parts.rafter_north,
            placement: rafter_frame(geometry, true),
        },
        TrussMemberTemplate {
            key_suffix: "principal-rafter-south",
            part: parts.rafter_south,
            placement: rafter_frame(geometry, false),
        },
        TrussMemberTemplate {
            key_suffix: "king-post",
            part: parts.king_post,
            placement: king_post_frame(geometry),
        },
        TrussMemberTemplate {
            key_suffix: "king-post-key",
            part: parts.key,
            placement: parts.key_frame,
        },
        TrussMemberTemplate {
            key_suffix: "diagonal-brace-north",
            part: parts.brace_north,
            placement: brace_frame(geometry, true),
        },
        TrussMemberTemplate {
            key_suffix: "diagonal-brace-south",
            part: parts.brace_south,
            placement: brace_frame(geometry, false),
        },
    ]
}

fn tie_frame(geometry: &TrussGeometry) -> Placement3 {
    Placement3::from_axes(
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [
            TIE_WIDTH * 0.5,
            -geometry.half_nave - TIE_END_RELISH,
            geometry.tie_base,
        ],
    )
}

fn rafter_frame(geometry: &TrussGeometry, north: bool) -> Placement3 {
    if north {
        geometry.north_rafter_frame
    } else {
        geometry.south_rafter_frame
    }
}

fn king_post_frame(geometry: &TrussGeometry) -> Placement3 {
    // `joiner`'s member convention puts length on local x. The cyclic
    // [world-z, world-x, world-y] basis is proper (determinant +1), so the
    // vertical post needs neither a reflected instance nor a special recipe.
    Placement3::from_axes(
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [
            -KING_POST_WIDTH * 0.5,
            -KING_POST_WIDTH * 0.5,
            geometry.king_post_base,
        ],
    )
}

fn brace_frame(geometry: &TrussGeometry, north: bool) -> Placement3 {
    if north {
        geometry.north_brace_frame
    } else {
        geometry.south_brace_frame
    }
}

#[cfg(test)]
mod tests {
    use exedra_assembly::{PartSource, assembly_fingerprint};
    use exedra_constructive::evaluate::evaluate;
    use exedra_constructive::ir::NodeKind;
    use exedra_constructive::tessellate::EvalPolicy;
    use setout::Count;

    use super::*;
    use crate::output::{bounds_for_path, build_scenario};
    use crate::{BasilicaPremises, instances_with_role, resolve_instance_path};

    const FRAME_PREFIXES: [&str; 6] = [
        "nave-truss-west-start",
        "nave-truss-west-interior-000001",
        "nave-truss-west-interior-000003",
        "nave-truss-west-interior-000004",
        "nave-truss-west-end",
        "nave-truss-east",
    ];

    #[test]
    fn generated_stations_match_the_accepted_floating_geometry() {
        // Exact generated stations deliberately replace ordinal instance
        // names, but their order, materials, fitted recipes, and geometry must
        // remain equal to the accepted gallery (within the lowering epsilon).
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default roof resolves");
        let plan = setout.plan();
        let geometry = TrussGeometry::from_setout(&setout);

        let mut patterned = BuildContext::new();
        build(&mut patterned, plan, &setout);

        let mut legacy = BuildContext::new();
        let parts = add_member_parts(&mut legacy, &geometry);
        let west_bays = u32::try_from(plan.nave_truss_bays.get()).unwrap();
        let west_pitch = (plan.crossing_west.as_meters() - 4.0) / f64::from(west_bays);
        for slot in 0..=west_bays {
            if slot != 2 {
                add_legacy_frame(
                    &mut legacy,
                    &parts,
                    &geometry,
                    "west",
                    slot,
                    2.0 + f64::from(slot) * west_pitch,
                );
            }
        }
        add_legacy_frame(
            &mut legacy,
            &parts,
            &geometry,
            "east",
            0,
            (plan.crossing_east.as_meters() + plan.length.as_meters()) * 0.5,
        );

        let patterned = patterned.finish();
        let legacy = legacy.finish();
        assert_eq!(patterned.instances().len(), 42);
        for (patterned_part, legacy_part) in patterned.parts().iter().zip(legacy.parts()) {
            assert_eq!(patterned_part.key(), legacy_part.key());
            assert_eq!(
                patterned_part.default_materials(),
                legacy_part.default_materials()
            );
            let (PartSource::Recipe(patterned_recipe), PartSource::Recipe(legacy_recipe)) =
                (patterned_part.source(), legacy_part.source())
            else {
                panic!("truss test uses recipe parts");
            };
            assert_eq!(
                patterned_recipe.recipe_fingerprint(),
                legacy_recipe.recipe_fingerprint(),
                "{}",
                patterned_part.key()
            );
        }
        for ((_, patterned), (_, legacy)) in patterned
            .instances_with_ids()
            .zip(legacy.instances_with_ids())
        {
            assert_eq!(patterned.part(), legacy.part());
            for (generated, accepted) in patterned
                .placement()
                .rows
                .into_iter()
                .flatten()
                .zip(legacy.placement().rows.into_iter().flatten())
            {
                assert!(
                    (generated - accepted).abs() <= 1.0e-12,
                    "{} placement moved: {generated} != {accepted}",
                    patterned.key()
                );
            }
            assert_eq!(
                patterned.metadata(),
                legacy.metadata(),
                "{}",
                patterned.key()
            );
        }
        assert_ne!(
            assembly_fingerprint(&patterned),
            assembly_fingerprint(&legacy)
        );
    }

    fn add_legacy_frame(
        context: &mut BuildContext,
        parts: &MemberParts,
        geometry: &TrussGeometry,
        segment: &str,
        slot: u32,
        x: f64,
    ) {
        let prefix = format!("nave-truss-{segment}-{slot:02}");
        for (suffix, part, placement) in [
            (
                "tie-beam",
                parts.tie,
                legacy_station_placement(x, tie_frame(geometry)),
            ),
            (
                "principal-rafter-north",
                parts.rafter_north,
                legacy_station_placement(x, rafter_frame(geometry, true)),
            ),
            (
                "principal-rafter-south",
                parts.rafter_south,
                legacy_station_placement(x, rafter_frame(geometry, false)),
            ),
            (
                "king-post",
                parts.king_post,
                legacy_station_placement(x, king_post_frame(geometry)),
            ),
            (
                "king-post-key",
                parts.key,
                legacy_station_placement(x, parts.key_frame),
            ),
            (
                "diagonal-brace-north",
                parts.brace_north,
                legacy_station_placement(x, brace_frame(geometry, true)),
            ),
            (
                "diagonal-brace-south",
                parts.brace_south,
                legacy_station_placement(x, brace_frame(geometry, false)),
            ),
        ] {
            context.add_instance(
                &format!("{prefix}-{suffix}"),
                part,
                placement,
                names::roles::NAVE_TRUSS_MEMBER,
            );
        }
    }

    fn legacy_station_placement(x: f64, mut placement: Placement3) -> Placement3 {
        placement.rows[0][3] += x;
        placement
    }

    #[test]
    fn named_members_share_seven_fitted_parts_and_one_role() {
        // Six stations reuse one part per handed member shape plus the shared
        // tie, king post, and transverse key, while retaining one semantic
        // role for gallery selection.
        let scenario = build_scenario();
        let members = instances_with_role(&scenario.assembly, names::roles::NAVE_TRUSS_MEMBER);
        assert_eq!(members.len(), 42);

        for (key, expected_count) in [
            (names::parts::NAVE_TRUSS_TIE_BEAM, 6),
            (names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER, 6),
            (names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER_SOUTH, 6),
            (names::parts::NAVE_TRUSS_KING_POST, 6),
            (names::parts::NAVE_TRUSS_DIAGONAL_BRACE, 6),
            (names::parts::NAVE_TRUSS_DIAGONAL_BRACE_SOUTH, 6),
            (names::parts::NAVE_TRUSS_KING_POST_KEY, 6),
        ] {
            let part = scenario
                .assembly
                .part_by_key(key)
                .unwrap_or_else(|| panic!("missing stable truss part {key}"));
            assert_eq!(
                members
                    .iter()
                    .filter(|&&id| scenario.assembly.instance(id).unwrap().part() == part)
                    .count(),
                expected_count
            );
        }

        for id in members {
            let instance = scenario.assembly.instance(id).unwrap();
            let part = scenario.assembly.part(instance.part()).unwrap();
            let surface = part.slot_index("surface").expect("truss surface slot");
            assert_eq!(
                scenario.assembly.resolved_material(id, surface),
                Some("aged-timber")
            );
        }
    }

    #[test]
    fn every_generated_station_expands_to_one_complete_fitted_truss() {
        // Changing the station topology must scale the existing seven-member
        // construction atomically; a new station may not produce only the
        // easy box members or lose the generated king-post key.
        let premises = BasilicaPremises {
            nave_truss_bays: Count::new(6),
            ..BasilicaPremises::default()
        };
        let setout = BasilicaSetout::new(&premises).unwrap();
        let assembly = crate::build_basilica_assembly(&premises);
        let members = instances_with_role(&assembly, names::roles::NAVE_TRUSS_MEMBER);
        assert_eq!(members.len(), 49);

        for station in setout.west_truss_stations().items() {
            let station = west_truss_station_key(station.label());
            for suffix in crate::NAVE_TRUSS_MEMBER_SUFFIXES {
                let path = truss_member_instance_key(&station, suffix);
                assert!(
                    resolve_instance_path(&assembly, &path).is_some(),
                    "generated station is missing fitted member {path}"
                );
            }
        }
        for suffix in crate::NAVE_TRUSS_MEMBER_SUFFIXES {
            let path = truss_member_instance_key(NAVE_TRUSS_EAST_STATION_KEY, suffix);
            assert!(
                resolve_instance_path(&assembly, &path).is_some(),
                "exact east station is missing fitted member {path}"
            );
        }
    }

    #[test]
    fn member_recipes_are_clean_solids_and_placements_are_rigid() {
        // Every handed, rule-edited member and generated key must survive real
        // constructive evaluation; proper placements keep their winding valid
        // in every exporter.
        let scenario = build_scenario();
        for key in [
            names::parts::NAVE_TRUSS_TIE_BEAM,
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER,
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER_SOUTH,
            names::parts::NAVE_TRUSS_KING_POST,
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE,
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE_SOUTH,
            names::parts::NAVE_TRUSS_KING_POST_KEY,
        ] {
            let part = scenario
                .assembly
                .part(
                    scenario
                        .assembly
                        .part_by_key(key)
                        .expect("stable truss part exists"),
                )
                .unwrap();
            let PartSource::Recipe(recipe) = part.source() else {
                panic!("truss members stay recipe-backed");
            };
            let evaluated =
                evaluate(recipe, &EvalPolicy::default()).expect("truss recipe evaluates");
            assert!(evaluated.report.diagnostics.is_empty());
            assert_eq!(evaluated.bodies.len(), 1);
            assert!(evaluated.bodies[0].body.mesh.validate_deep().is_empty());
        }

        for id in instances_with_role(&scenario.assembly, names::roles::NAVE_TRUSS_MEMBER) {
            let placement = scenario.assembly.instance(id).unwrap().placement();
            assert!(
                placement
                    .rows
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite()),
                "truss placement must be finite"
            );
            assert!(
                (rotation_determinant(placement) - 1.0).abs() < 1.0e-12,
                "truss placement must be proper rigid: {:?}",
                placement.rows
            );
        }
    }

    #[test]
    fn south_members_are_structural_mirrors_with_proper_instances() {
        // The counterpart must retain the complete canonical recipe under one
        // winding-correct Mirror root. Distinct assembly parts are intentional,
        // but every station placement remains proper rigid.
        let scenario = build_scenario();
        let geometry = TrussGeometry::from_setout(
            &BasilicaSetout::new(&BasilicaPremises::default()).expect("default roof resolves"),
        );
        for (canonical_key, mirrored_key, normal, distance) in [
            (
                names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER,
                names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER_SOUTH,
                [0.0, 1.0, 0.0],
                geometry.rafter_width * 0.5,
            ),
            (
                names::parts::NAVE_TRUSS_DIAGONAL_BRACE,
                names::parts::NAVE_TRUSS_DIAGONAL_BRACE_SOUTH,
                [0.0, 0.0, 1.0],
                BRACE_DEPTH * 0.5,
            ),
        ] {
            let canonical = scenario
                .assembly
                .part_by_key(canonical_key)
                .and_then(|id| scenario.assembly.part(id))
                .expect("canonical member part exists");
            let mirrored = scenario
                .assembly
                .part_by_key(mirrored_key)
                .and_then(|id| scenario.assembly.part(id))
                .expect("mirrored member part exists");
            let (PartSource::Recipe(canonical), PartSource::Recipe(mirrored)) =
                (canonical.source(), mirrored.source())
            else {
                panic!("timber counterparts stay recipe-backed");
            };
            let NodeKind::Mirror { child, plane } = &mirrored
                .node(mirrored.root())
                .expect("mirror root exists")
                .kind
            else {
                panic!("south timber counterpart has one structural Mirror root");
            };
            assert_eq!(plane.normal, normal);
            assert_eq!(plane.distance, distance);
            assert_eq!(
                mirrored.fingerprint(*child),
                Some(canonical.recipe_fingerprint()),
                "the complete canonical recipe remains the mirror child"
            );
            assert_eq!(mirrored.nodes().len(), canonical.nodes().len() + 1);
        }

        // The old independently authored south members remain a test-only
        // oracle. Matching quantized local vertices proves the chosen planes
        // reproduce the rule-generated counterparts, even where equal bounds
        // alone could hide a pocket mirrored onto the wrong section face.
        let fitted = fitted_member_recipes(&geometry);
        for (mirrored, authored, label) in [
            (
                &fitted.rafter_south,
                &fitted.authored_rafter_south,
                "rafter",
            ),
            (&fitted.brace_south, &fitted.authored_brace_south, "brace"),
        ] {
            assert_eq!(
                quantized_vertices(mirrored),
                quantized_vertices(authored),
                "mirror-composed {label} must match the independently authored south-side oracle"
            );
        }
    }

    fn quantized_vertices(recipe: &Recipe) -> Vec<[u64; 3]> {
        let evaluated = evaluate(recipe, &EvalPolicy::default()).expect("oracle recipe evaluates");
        assert_eq!(evaluated.bodies.len(), 1, "oracle recipe is one solid");
        let mesh = &evaluated.bodies[0].body.mesh;
        let mut vertices: Vec<[u64; 3]> = mesh
            .vertices()
            .filter_map(|vertex| mesh.vertex_position(vertex))
            .map(|point| {
                point.map(|value| {
                    let quantized = (f64::from(value) * 1.0e8).round() / 1.0e8;
                    if quantized == 0.0 {
                        0.0_f64.to_bits()
                    } else {
                        quantized.to_bits()
                    }
                })
            })
            .collect();
        vertices.sort_unstable();
        vertices
    }

    #[test]
    fn rafters_clear_the_roof_underside_and_bear_through_the_ties() {
        // This pins the visible setout contract after applying joinery: heel
        // housings retain end relish, rafters retain roof reveal, the king
        // encloses both head pockets, and struts start above rather than in
        // the tie beam.
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default roof resolves");
        let geometry = TrussGeometry::from_setout(&setout);
        let roof = setout.roof();
        let scenario = build_scenario();

        for prefix in FRAME_PREFIXES {
            let tie_path = format!("{prefix}-tie-beam");
            let (tie_min, tie_max) =
                bounds_for_path(&scenario.compiled, &scenario.render_list, &tie_path);
            assert_close(tie_min[1], -geometry.half_nave - TIE_END_RELISH);
            assert_close(tie_max[1], geometry.half_nave + TIE_END_RELISH);
            assert!(
                tie_min[2] < roof.wall_head.as_meters()
                    && tie_max[2] >= roof.wall_plate_top.as_meters() - 1.0e-5
            );

            for side in ["north", "south"] {
                let path = format!("{prefix}-principal-rafter-{side}");
                let item = render_item(&scenario, &path);
                let body = &scenario.compiled.part(item.part).unwrap().bodies[item.body as usize];
                let normal_y = if side == "north" {
                    geometry.roof_sin
                } else {
                    -geometry.roof_sin
                };
                let max_plane_distance = body
                    .tri
                    .positions
                    .iter()
                    .map(|&position| transform_point(&item.world, position))
                    .map(|position| {
                        normal_y * position[1] + geometry.roof_cos * position[2]
                            - geometry.roof_cos * geometry.roof_peak
                    })
                    .fold(f64::NEG_INFINITY, f64::max);
                assert!(
                    max_plane_distance <= -geometry.roof_clearance + 1.0e-5,
                    "{path} protrudes into the roof: {max_plane_distance}"
                );
                assert!(
                    (max_plane_distance + geometry.roof_clearance).abs() < 1.0e-5,
                    "{path} must retain the designed roof clearance: {max_plane_distance}"
                );
                let (rafter_min, _) =
                    bounds_for_path(&scenario.compiled, &scenario.render_list, &path);
                assert!(
                    rafter_min[2] < tie_max[2] && rafter_min[2] > tie_min[2],
                    "{path} must overlap the tie bearing: {rafter_min:?} vs {tie_min:?}..{tie_max:?}"
                );
            }

            let (king_min, king_max) = bounds_for_path(
                &scenario.compiled,
                &scenario.render_list,
                &format!("{prefix}-king-post"),
            );
            assert!(king_min[2] < tie_max[2]);
            assert_close(
                king_max[2],
                geometry.king_post_base + geometry.king_post_height,
            );

            for side in ["north", "south"] {
                let (brace_min, brace_max) = bounds_for_path(
                    &scenario.compiled,
                    &scenario.render_list,
                    &format!("{prefix}-diagonal-brace-{side}"),
                );
                assert!(brace_min[2] > tie_max[2]);
                assert!(brace_max[2] > tie_max[2] + 1.0);
            }

            let (key_min, key_max) = bounds_for_path(
                &scenario.compiled,
                &scenario.render_list,
                &format!("{prefix}-king-post-key"),
            );
            assert!(key_min[2] < tie_min[2], "key is exposed below the tie");
            assert!(key_max[2] <= tie_min[2] + 1.0e-5);
        }
    }

    #[test]
    fn every_intact_frame_has_a_mirrored_rafter_pair_with_visible_roof_reveal() {
        // Each surviving station must contain the complete seven-piece truss;
        // its south recipe remains a structural mirror of the north recipe
        // while retaining proper-rigid instance placements and roof reveal.
        const MIN_VISIBLE_ROOF_REVEAL: f64 = 0.10;
        const MEMBER_SUFFIXES: [&str; 7] = [
            "tie-beam",
            "principal-rafter-north",
            "principal-rafter-south",
            "king-post",
            "king-post-key",
            "diagonal-brace-north",
            "diagonal-brace-south",
        ];

        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default roof resolves");
        let geometry = TrussGeometry::from_setout(&setout);
        let scenario = build_scenario();

        for prefix in FRAME_PREFIXES {
            let north_path = format!("{prefix}-principal-rafter-north");
            let south_path = format!("{prefix}-principal-rafter-south");
            let north_id = resolve_instance_path(&scenario.assembly, &north_path)
                .unwrap_or_else(|| panic!("missing paired north rafter {north_path}"));
            let south_id = resolve_instance_path(&scenario.assembly, &south_path)
                .unwrap_or_else(|| panic!("missing paired south rafter {south_path}"));
            assert_ne!(
                scenario.assembly.instance(north_id).unwrap().part(),
                scenario.assembly.instance(south_id).unwrap().part(),
                "{prefix} uses distinct mirror-composed parts instead of a reflected instance"
            );

            let (north_min, north_max) =
                bounds_for_path(&scenario.compiled, &scenario.render_list, &north_path);
            let (south_min, south_max) =
                bounds_for_path(&scenario.compiled, &scenario.render_list, &south_path);
            assert_close(north_min[0], south_min[0]);
            assert_close(north_max[0], south_max[0]);
            assert_close(north_min[1], -south_max[1]);
            assert_close(north_max[1], -south_min[1]);
            assert_close(north_min[2], south_min[2]);
            assert_close(north_max[2], south_max[2]);

            for (path, normal_y) in [
                (&north_path, geometry.roof_sin),
                (&south_path, -geometry.roof_sin),
            ] {
                let item = render_item(&scenario, path);
                let body = &scenario.compiled.part(item.part).unwrap().bodies[item.body as usize];
                let max_plane_distance = body
                    .tri
                    .positions
                    .iter()
                    .map(|&position| transform_point(&item.world, position))
                    .map(|position| {
                        normal_y * position[1] + geometry.roof_cos * position[2]
                            - geometry.roof_cos * geometry.roof_peak
                    })
                    .fold(f64::NEG_INFINITY, f64::max);
                let reveal = -max_plane_distance;
                assert!(
                    reveal >= MIN_VISIBLE_ROOF_REVEAL,
                    "{path} is present but visually swallowed by the roof: reveal={reveal}m"
                );
                assert_close(reveal, geometry.roof_clearance);
            }
        }

        for suffix in MEMBER_SUFFIXES {
            let omitted_path = format!("nave-truss-west-interior-000002-{suffix}");
            assert!(
                resolve_instance_path(&scenario.assembly, &omitted_path).is_none(),
                "only the authored west-02 frame may be omitted: {omitted_path}"
            );
        }
    }

    #[test]
    fn truss_stations_preserve_the_ruin_and_crossing_voids() {
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default basilica resolves");
        let crossing_west = setout.plan().crossing_west.as_meters();
        let crossing_east = setout.plan().crossing_east.as_meters();
        let scenario = build_scenario();
        assert!(
            resolve_instance_path(
                &scenario.assembly,
                "nave-truss-west-interior-000002-tie-beam"
            )
            .is_none()
        );

        // Select by stable semantic path: unrelated assembly insertions must
        // not silently change which elements this ruin-boundary test covers.
        for item in scenario
            .render_list
            .items
            .iter()
            .filter(|item| item.path.to_string().starts_with("nave-truss-"))
        {
            let path = item.path.to_string();
            let (min, max) = bounds_for_path(&scenario.compiled, &scenario.render_list, &path);
            if path.starts_with("nave-truss-west") {
                assert!(
                    max[0] <= 7.15 || min[0] >= 10.75,
                    "{path} enters the authored roof ruin: {min:?}..{max:?}"
                );
                assert!(max[0] < crossing_west, "{path} enters the crossing");
            } else {
                assert!(path.starts_with("nave-truss-east-"));
                assert!(min[0] > crossing_east, "{path} enters the crossing");
            }
        }
    }

    fn render_item<'a>(
        scenario: &'a crate::output::Scenario,
        path: &str,
    ) -> &'a exedra_assembly::RenderItem {
        scenario
            .render_list
            .items
            .iter()
            .find(|item| item.path.to_string() == path)
            .unwrap_or_else(|| panic!("missing render item {path}"))
    }

    fn transform_point(placement: &Placement3, point: [f32; 3]) -> [f64; 3] {
        let point = point.map(f64::from);
        let rows = &placement.rows;
        [
            rows[0][0] * point[0] + rows[0][1] * point[1] + rows[0][2] * point[2] + rows[0][3],
            rows[1][0] * point[0] + rows[1][1] * point[1] + rows[1][2] * point[2] + rows[1][3],
            rows[2][0] * point[0] + rows[2][1] * point[1] + rows[2][2] * point[2] + rows[2][3],
        ]
    }

    fn rotation_determinant(placement: &Placement3) -> f64 {
        let r = &placement.rows;
        r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0])
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
    }
}
