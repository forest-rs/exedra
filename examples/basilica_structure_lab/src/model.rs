// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The basilica's structural hypothesis, authored into a [`Construction`].
//!
//! The graph, its relation kinds, its validation, and its lowering all live in
//! the `joiner` crate now. What stays here is what is actually about *this
//! building*: the evidence sources, the one-bay geometry, the roles those
//! elements play, and the presentation of a graph record to a reader.

use std::fmt::Write as _;

use basilica_ruin::{BasilicaParams, BasilicaRoofSetout, RoofSide};
use exedra_math::{add, cross, norm, normalize, scale, sub};
use joiner::{
    Anchor, Construction, ContactMeaning, ContactPatch, Element, Evidence, EvidenceClass,
    EvidenceSource, Member, Node, OrientedBox, Relation, RelationKind, Rule, RuleApplication,
    RuleContext, Support, TransferEdge, TransferKind, TransferTarget, ValidationReport,
    is_witnessed, measure_contact, trace_to_ground, validate,
};
use joiner_timber::{
    CommonRafterPurlinSeatParams, CommonRafterToPurlinSeatRule, HeelParams, HeelRule,
    HousedBearingParams, KingPostTieParams, KingPostTieRule, Length, PurlinPrincipalTrenchParams,
    PurlinToPrincipalTrenchRule, RafterToKingPostRule, StrutToKingPostRule, StrutToRafterRule,
};

pub(crate) type Vec3 = [f64; 3];

/// The tolerance `joiner` measures contact at, republished for diagnostics.
pub(crate) const CONTACT_TOLERANCE: f64 = joiner::CONTACT_TOLERANCE;
const TIE_END_RELISH: f64 = 0.18;
const KING_POST_WIDTH: f64 = 0.36;
const PURLIN_TRENCH_DEPTH: Length = length_millimeters(30);
const COMMON_RAFTER_SEAT_DEPTH: Length = length_millimeters(20);

const fn length_millimeters(value: u64) -> Length {
    match Length::millimeters(value) {
        Some(value) => value,
        None => panic!("authored lab dimensions must be positive millimeters"),
    }
}

/// What part an element plays in this roof.
///
/// Basilica vocabulary, not `joiner` vocabulary: the construction layer sees
/// only the opaque label this maps to.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum ElementRole {
    RoofCovering,
    Boarding,
    CommonRafter,
    Purlin,
    PrincipalRafter,
    TieBeam,
    KingPost,
    Strut,
    WallPlate,
    Masonry,
}

impl ElementRole {
    pub(crate) const ALL: [Self; 10] = [
        Self::RoofCovering,
        Self::Boarding,
        Self::CommonRafter,
        Self::Purlin,
        Self::PrincipalRafter,
        Self::TieBeam,
        Self::KingPost,
        Self::Strut,
        Self::WallPlate,
        Self::Masonry,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::RoofCovering => "roof-covering",
            Self::Boarding => "boarding",
            Self::CommonRafter => "common-rafter",
            Self::Purlin => "purlin",
            Self::PrincipalRafter => "principal-rafter",
            Self::TieBeam => "tie-beam",
            Self::KingPost => "king-post",
            Self::Strut => "strut",
            Self::WallPlate => "wall-plate",
            Self::Masonry => "masonry",
        }
    }

    /// Recovers the role from the opaque label carried on a `joiner` element.
    pub(crate) fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.label() == label)
    }

    pub(crate) fn is_roof_skin(self) -> bool {
        matches!(self, Self::RoofCovering | Self::Boarding)
    }

    fn requires_member(self) -> bool {
        !matches!(self, Self::RoofCovering | Self::Boarding | Self::Masonry)
    }

    pub(crate) fn material(self) -> &'static str {
        match self {
            Self::RoofCovering => "semantic-roof-covering",
            Self::Boarding => "semantic-boarding",
            Self::CommonRafter => "semantic-common-rafter",
            Self::Purlin => "semantic-purlin",
            Self::PrincipalRafter => "semantic-principal-rafter",
            Self::TieBeam => "semantic-tie-beam",
            Self::KingPost => "semantic-king-post",
            Self::Strut => "semantic-strut",
            Self::WallPlate => "semantic-wall-plate",
            Self::Masonry => "semantic-masonry",
        }
    }
}

/// The connection hypothesis at a joint.
///
/// Travels as the relation's opaque detail label. Every variant used by the
/// primary trusses is instantiated by a concrete `joiner_timber` rule.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum JointKind {
    BearingSeat,
    CommonRafterSeat,
    HousedBearing,
    KeyedThroughTenon,
    PurlinTrench,
}

impl JointKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::BearingSeat => "bearing-seat",
            Self::CommonRafterSeat => "common-rafter-seat",
            Self::HousedBearing => "housed-bearing",
            Self::KeyedThroughTenon => "keyed-through-tenon",
            Self::PurlinTrench => "purlin-trench",
        }
    }
}

/// How a bearing face is formed.
///
/// Travels as the contact patch's opaque detail label; every one of these is
/// a [`ContactMeaning::Bearing`] as far as `joiner` is concerned.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum BearingKind {
    Surface,
    AnchorContact,
    WallHead,
}

impl BearingKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Surface => "surface",
            Self::AnchorContact => "anchor-contact",
            Self::WallHead => "wall-head",
        }
    }
}

fn evidence_source_key(class: EvidenceClass) -> &'static str {
    match class {
        EvidenceClass::Observed => "saint-catherine-sixth-century-roof",
        EvidenceClass::DocumentedReconstruction => "san-paolo-double-truss",
        EvidenceClass::RegionalAnalogy => "hagia-paraskevi-roof-survey",
        EvidenceClass::ModernEngineeringInference => "tfec-modern-truss-detailing",
        _ => "tfec-modern-truss-detailing",
    }
}

fn evidence(class: EvidenceClass) -> Evidence {
    Evidence::new(evidence_source_key(class), class)
}

/// Authoring helper: every mutation here is known-good by construction, so a
/// rejection is a bug in the hypothesis rather than a runtime condition.
struct Author {
    construction: Construction,
}

impl Author {
    fn new() -> Self {
        let mut construction = Construction::new();
        for source in [
            EvidenceSource::new(
                "saint-catherine-sixth-century-roof",
                EvidenceClass::Observed,
                "https://lsa.umich.edu/kelsey/research/past-field-projects/monastery-st-catharine.html",
                "Observed timber nave roof dated 548-565; not a complete joint specification",
            ),
            EvidenceSource::new(
                "san-paolo-double-truss",
                EvidenceClass::DocumentedReconstruction,
                "https://www.witpress.com/elibrary/wit-transactions-on-the-built-environment/191/37497",
                "Pre-1823 records support analysis of the lost Early Christian double truss",
            ),
            EvidenceSource::new(
                "hagia-paraskevi-roof-survey",
                EvidenceClass::RegionalAnalogy,
                "https://hdl.handle.net/11583/1956728",
                "Later eastern-Mediterranean truss, secondary timber, boarding, and tile evidence",
            ),
            EvidenceSource::new(
                "vernacular-trenched-purlin",
                EvidenceClass::RegionalAnalogy,
                "https://www.vernacularbuildingglossary.org.uk/a-z/purlin/",
                "Typology and examples of purlins trenched into principal rafters; not Byzantine attribution",
            ),
            EvidenceSource::new(
                "knole-common-rafter-purlin-joints",
                EvidenceClass::RegionalAnalogy,
                "https://heritagerecords.nationaltrust.org.uk/LibraryLinkWS/LLFiles/225169/original_225169.pdf",
                "Recorded lower purlins trenched into common rafters; a joint-form analogy, not building attribution",
            ),
            EvidenceSource::new(
                "tfec-modern-truss-detailing",
                EvidenceClass::ModernEngineeringInference,
                "https://timberframehq.com/wp-content/uploads/2021/12/TFEC-DG-1-2021.pdf",
                "Modern connection behavior and detailing vocabulary, not Byzantine evidence",
            ),
        ] {
            construction
                .add_evidence_source(source)
                .expect("evidence source keys are distinct");
        }
        Self { construction }
    }

    fn node(&mut self, key: String, point: Vec3) -> String {
        self.construction
            .add_node(Node::new(&key, point))
            .expect("node keys are distinct");
        key
    }

    fn node_point(&self, key: &str) -> Vec3 {
        self.construction
            .node(key)
            .expect("authored node exists")
            .point
    }

    fn element(
        &mut self,
        key: String,
        role: ElementRole,
        extent: OrientedBox,
        class: EvidenceClass,
        required_supports: usize,
    ) -> String {
        let mut element =
            Element::new(&key, role.label(), role.material(), extent, evidence(class))
                .with_required_supports(required_supports);
        if role.requires_member() {
            element = element.with_member();
        }
        self.construction
            .add_element(element)
            .expect("element keys are distinct");
        key
    }

    fn member(&mut self, key: String, element: &str, from: &str, to: &str, class: EvidenceClass) {
        self.construction
            .add_member(Member::new(&key, element, from, to, evidence(class)))
            .expect("member references resolve");
    }

    fn joint(
        &mut self,
        key: &str,
        node: &str,
        members: &[&str],
        kind: JointKind,
        class: EvidenceClass,
    ) {
        self.joint_with_evidence(key, node, members, kind, evidence(class));
    }

    fn joint_with_evidence(
        &mut self,
        key: &str,
        node: &str,
        members: &[&str],
        kind: JointKind,
        joint_evidence: Evidence,
    ) {
        self.construction
            .add_relation(Relation::new(
                key,
                RelationKind::member_member(node, members),
                kind.label(),
                joint_evidence,
            ))
            .expect("joint references resolve");
    }

    fn fit<R: Rule>(
        &mut self,
        application: &str,
        relation: &str,
        rule: &R,
        params: &R::Params,
        class: EvidenceClass,
    ) {
        self.fit_with_evidence(application, relation, rule, params, evidence(class));
    }

    fn fit_with_evidence<R: Rule>(
        &mut self,
        application: &str,
        relation: &str,
        rule: &R,
        params: &R::Params,
        application_evidence: Evidence,
    ) {
        let output = {
            let context = RuleContext::new(&self.construction, relation)
                .expect("the relation was registered immediately before fitting");
            let applicability = rule.assess(&context);
            assert!(
                applicability.is_suitable(),
                "{} refused {relation}: {:?}",
                rule.key(),
                applicability.rejections()
            );
            rule.instantiate(&context, params)
                .unwrap_or_else(|error| panic!("{} could not fit {relation}: {error}", rule.key()))
        };
        self.construction
            .apply(RuleApplication::new(
                application,
                rule.key(),
                relation,
                application_evidence,
                output,
            ))
            .expect("assessed rule output merges atomically");
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a bearing's explicit proof data is one record"
    )]
    fn bearing(
        &mut self,
        key: &str,
        carried: Anchor,
        carrier: Anchor,
        normal: Vec3,
        tangents: [Vec3; 2],
        minimum_overlap: [f64; 2],
        kind: BearingKind,
        class: EvidenceClass,
    ) {
        self.construction
            .add_contact(
                ContactPatch::new(
                    key,
                    carried,
                    carrier,
                    normal,
                    tangents,
                    ContactMeaning::Bearing,
                    evidence(class),
                )
                .with_minimum_overlap(minimum_overlap)
                .with_detail(kind.label()),
            )
            .expect("bearing references resolve");
    }

    fn support(&mut self, key: &str, element: &str, ground: &str) {
        self.construction
            .add_support(Support::fixed(key, element, ground))
            .expect("support references resolve");
    }

    fn transfer(&mut self, key: String, from: &str, to: TransferTarget, kind: TransferKind) {
        self.construction
            .add_transfer(TransferEdge::new(&key, from, to, kind))
            .expect("transfer references resolve");
    }
}

struct SideData {
    name: &'static str,
    side: f64,
    slope: Vec3,
    normal: Vec3,
    outer_eave: Vec3,
    principal_extent: OrientedBox,
    masonry: String,
    wall_plate: String,
    purlins: Vec<(&'static str, f64, String)>,
}

/// Authors one intact western bay as an evidence-labelled construction.
#[expect(
    clippy::too_many_lines,
    reason = "the bay is one hypothesis; splitting it would hide the geometry that ties it together"
)]
pub(crate) fn western_bay(params: &BasilicaParams) -> Construction {
    let mut author = Author::new();
    let roof_setout = BasilicaRoofSetout::new(params).expect("basilica roof must set out");
    let roof = roof_setout.section();

    let x0 = 0.0;
    let x1 = 4.0;
    let bay_length = x1 - x0;
    let half_nave = roof.half_span.as_metres();
    let wall_top = roof.wall_head.as_metres();
    let roof_length = roof.rafter_length.as_metres();

    let covering_depth = 0.08;
    let boarding_depth = 0.04;
    let common_depth = 0.16;
    let purlin_depth = 0.22;
    let principal_depth = roof.principal_rafter_depth.as_metres();
    let purlin_trench_depth = PURLIN_TRENCH_DEPTH.as_meters();
    let common_rafter_seat_depth = COMMON_RAFTER_SEAT_DEPTH.as_meters();
    // Setout endpoints describe the principal-rafter centerline. The visible
    // roof stack is smaller than the sum of the uncut section depths because
    // two deliberate overlaps become joints: purlins sink into the principals
    // and common rafters sink into the purlins. The rules below remove those
    // exact overlaps; no cutter is allowed to manufacture contact by moving a
    // member or by changing this setout arithmetic.
    let stack_depth =
        covering_depth + boarding_depth + common_depth + purlin_depth + principal_depth * 0.5
            - purlin_trench_depth
            - common_rafter_seat_depth;
    let common_width = 0.12;
    let purlin_width = 0.18;
    let principal_width = roof.principal_rafter_width.as_metres();
    let wall_plate_width = roof.wall_plate_width.as_metres();
    let wall_plate_height = roof.wall_plate_height.as_metres();
    let masonry_width = 0.45;
    let x_axis = [1.0, 0.0, 0.0];
    let z_axis = [0.0, 0.0, 1.0];

    let principal_geometry = [
        roof_setout
            .principal_rafter_geometry(RoofSide::South)
            .expect("south principal rafter resolves"),
        roof_setout
            .principal_rafter_geometry(RoofSide::North)
            .expect("north principal rafter resolves"),
    ];
    let mut side_data: Vec<SideData> = Vec::new();
    for (index, (side_name, side)) in [("south", -1.0), ("north", 1.0)].into_iter().enumerate() {
        let resolved = &principal_geometry[index];
        let slope = resolved.extent.axes[0];
        let normal = resolved.extent.axes[2];
        let wall_point = setout_joiner::lower_point(roof.wall_seat(if side < 0.0 {
            RoofSide::South
        } else {
            RoofSide::North
        }));
        let outer_eave = add(wall_point, scale(normal, stack_depth));

        let masonry = author.element(
            format!("masonry-{side_name}"),
            ElementRole::Masonry,
            OrientedBox {
                origin: [x0, side * half_nave - masonry_width * 0.5, 0.0],
                axes: [x_axis, [0.0, 1.0, 0.0], z_axis],
                size: [bay_length, masonry_width, wall_top],
            },
            EvidenceClass::DocumentedReconstruction,
            1,
        );
        let wall_plate = author.element(
            format!("wall-plate-{side_name}"),
            ElementRole::WallPlate,
            OrientedBox {
                origin: [x0, side * half_nave - wall_plate_width * 0.5, wall_top],
                axes: [x_axis, [0.0, 1.0, 0.0], z_axis],
                size: [bay_length, wall_plate_width, wall_plate_height],
            },
            EvidenceClass::RegionalAnalogy,
            1,
        );
        let plate_from = author.node(
            format!("node-wall-plate-{side_name}-west"),
            [x0, side * half_nave, wall_top + wall_plate_height * 0.5],
        );
        let plate_to = author.node(
            format!("node-wall-plate-{side_name}-east"),
            [x1, side * half_nave, wall_top + wall_plate_height * 0.5],
        );
        author.member(
            format!("wall-plate-{side_name}"),
            &wall_plate,
            &plate_from,
            &plate_to,
            EvidenceClass::RegionalAnalogy,
        );

        let covering = author.element(
            format!("roof-covering-{side_name}"),
            ElementRole::RoofCovering,
            OrientedBox {
                origin: sub(outer_eave, scale(normal, covering_depth)),
                axes: [x_axis, slope, normal],
                size: [bay_length, roof_length, covering_depth],
            },
            EvidenceClass::RegionalAnalogy,
            1,
        );
        let boarding = author.element(
            format!("boarding-{side_name}"),
            ElementRole::Boarding,
            OrientedBox {
                origin: sub(outer_eave, scale(normal, covering_depth + boarding_depth)),
                axes: [x_axis, slope, normal],
                size: [bay_length, roof_length, boarding_depth],
            },
            EvidenceClass::RegionalAnalogy,
            3,
        );
        author.bearing(
            &format!("bearing-covering-on-boarding-{side_name}"),
            Anchor::new(&covering, [bay_length * 0.5, roof_length * 0.5, 0.0]),
            Anchor::new(
                &boarding,
                [bay_length * 0.5, roof_length * 0.5, boarding_depth],
            ),
            normal,
            [x_axis, slope],
            [bay_length * 0.95, roof_length * 0.95],
            BearingKind::Surface,
            EvidenceClass::RegionalAnalogy,
        );
        author.transfer(
            format!("load-covering-to-boarding-{side_name}"),
            &covering,
            TransferTarget::element(&boarding),
            TransferKind::Contact,
        );

        let mut common_rafters = Vec::new();
        for (ordinal, x) in [x0 + 0.45, (x0 + x1) * 0.5, x1 - 0.45]
            .into_iter()
            .enumerate()
        {
            let key = format!("common-rafter-{side_name}-{ordinal:02}");
            let extent = OrientedBox {
                origin: add(
                    [x - common_width * 0.5, 0.0, 0.0],
                    sub(
                        outer_eave,
                        scale(normal, covering_depth + boarding_depth + common_depth),
                    ),
                ),
                axes: [slope, x_axis, normal],
                size: [roof_length, common_width, common_depth],
            };
            let element = author.element(
                key.clone(),
                ElementRole::CommonRafter,
                extent,
                EvidenceClass::RegionalAnalogy,
                3,
            );
            let from = author.node(
                format!("node-{key}-eave"),
                add(
                    [x, 0.0, 0.0],
                    sub(
                        outer_eave,
                        scale(normal, covering_depth + boarding_depth + common_depth * 0.5),
                    ),
                ),
            );
            let to = author.node(
                format!("node-{key}-ridge"),
                add(author.node_point(&from), scale(slope, roof_length)),
            );
            author.member(key, &element, &from, &to, EvidenceClass::RegionalAnalogy);
            author.bearing(
                &format!("bearing-boarding-on-{side_name}-common-{ordinal:02}"),
                Anchor::new(&boarding, [x - x0, roof_length * 0.5, 0.0]),
                Anchor::new(
                    &element,
                    [roof_length * 0.5, common_width * 0.5, common_depth],
                ),
                normal,
                [x_axis, slope],
                [common_width * 0.95, roof_length * 0.95],
                BearingKind::Surface,
                EvidenceClass::RegionalAnalogy,
            );
            author.transfer(
                format!("load-boarding-{side_name}-to-common-{ordinal:02}"),
                &boarding,
                TransferTarget::element(&element),
                TransferKind::Contact,
            );
            common_rafters.push((element, x));
        }

        let mut purlins = Vec::new();
        for (position_name, t) in [
            ("eave", 0.42),
            ("mid", roof_length * 0.5),
            ("upper", roof_length - 0.42),
        ] {
            // This upper longitudinal timber is 420 mm below the apex. It is
            // an upper purlin, not a ridge beam; naming it as such keeps the
            // still-unmodeled apex joint from hiding behind a misleading role.
            let key = format!("purlin-{side_name}-{position_name}");
            let origin_at_t = add(outer_eave, scale(slope, t));
            let extent = OrientedBox {
                origin: sub(
                    add(
                        [x0 - principal_width * 0.5, 0.0, 0.0],
                        sub(origin_at_t, scale(slope, purlin_width * 0.5)),
                    ),
                    scale(
                        normal,
                        covering_depth + boarding_depth + common_depth + purlin_depth
                            - common_rafter_seat_depth,
                    ),
                ),
                axes: [x_axis, slope, normal],
                // The solid passes across the complete width of each
                // supporting principal. Its centreline still begins and ends
                // at the truss stations, leaving the physical overhang that a
                // true through trench requires instead of a half-width notch.
                size: [bay_length + principal_width, purlin_width, purlin_depth],
            };
            let element = author.element(
                key.clone(),
                ElementRole::Purlin,
                extent,
                EvidenceClass::RegionalAnalogy,
                2,
            );
            let from = author.node(
                format!("node-{key}-west"),
                add(
                    [x0, 0.0, 0.0],
                    sub(
                        origin_at_t,
                        scale(
                            normal,
                            covering_depth + boarding_depth + common_depth + purlin_depth * 0.5
                                - common_rafter_seat_depth,
                        ),
                    ),
                ),
            );
            let to = author.node(
                format!("node-{key}-east"),
                add(author.node_point(&from), [bay_length, 0.0, 0.0]),
            );
            author.member(key, &element, &from, &to, EvidenceClass::RegionalAnalogy);
            for (ordinal, (common, x)) in common_rafters.iter().enumerate() {
                let relation =
                    format!("joint-common-{side_name}-{ordinal:02}-on-{position_name}-purlin");
                let node = author.node(
                    format!("node-{relation}"),
                    add(
                        [*x, 0.0, 0.0],
                        sub(
                            origin_at_t,
                            scale(
                                normal,
                                covering_depth + boarding_depth + common_depth
                                    - common_rafter_seat_depth,
                            ),
                        ),
                    ),
                );
                let joint_evidence = Evidence::new(
                    "knole-common-rafter-purlin-joints",
                    EvidenceClass::RegionalAnalogy,
                );
                author.joint_with_evidence(
                    &relation,
                    &node,
                    &[common, element.as_str()],
                    JointKind::CommonRafterSeat,
                    joint_evidence.clone(),
                );
                author.fit_with_evidence(
                    &format!("fit-common-{side_name}-{ordinal:02}-on-{position_name}-purlin"),
                    &relation,
                    &CommonRafterToPurlinSeatRule,
                    &CommonRafterPurlinSeatParams {
                        seat_depth: COMMON_RAFTER_SEAT_DEPTH,
                        ..CommonRafterPurlinSeatParams::default()
                    },
                    joint_evidence,
                );
            }
            purlins.push((position_name, t, element));
        }

        side_data.push(SideData {
            name: side_name,
            side,
            slope,
            normal,
            outer_eave,
            principal_extent: resolved.extent.clone(),
            masonry,
            wall_plate,
            purlins,
        });
    }

    for (station_name, station_x) in [("west", x0), ("east", x1)] {
        let south_heel = author.node(
            format!("node-truss-{station_name}-heel-south"),
            [station_x, -half_nave, roof.wall_plate_top.as_metres()],
        );
        let north_heel = author.node(
            format!("node-truss-{station_name}-heel-north"),
            [station_x, half_nave, roof.wall_plate_top.as_metres()],
        );
        let ridge_inner = add(
            side_data[0].outer_eave,
            add(
                scale(side_data[0].slope, roof_length),
                scale(side_data[0].normal, -stack_depth),
            ),
        );
        let king_params = KingPostTieParams::default();
        let king_tenon_length = king_params.tenon_length.as_meters();
        let king_foot = author.node(
            format!("node-truss-{station_name}-king-tenon-tip"),
            [station_x, 0.0, wall_top + 0.30 - king_tenon_length],
        );
        let king_shoulder = author.node(
            format!("node-truss-{station_name}-king-shoulder"),
            [station_x, 0.0, wall_top + 0.30],
        );

        let tie = author.element(
            format!("tie-beam-{station_name}"),
            ElementRole::TieBeam,
            OrientedBox {
                origin: [station_x - 0.15, -half_nave - TIE_END_RELISH, wall_top],
                axes: [[0.0, 1.0, 0.0], x_axis, z_axis],
                size: [roof.span.as_metres() + 2.0 * TIE_END_RELISH, 0.30, 0.30],
            },
            EvidenceClass::ModernEngineeringInference,
            2,
        );
        let tie_member = format!("tie-beam-{station_name}");
        author.member(
            tie_member.clone(),
            &tie,
            &south_heel,
            &north_heel,
            EvidenceClass::ModernEngineeringInference,
        );
        for data in &side_data {
            let (name, side, masonry) = (data.name, data.side, data.masonry.as_str());
            let local_y = if side < 0.0 {
                TIE_END_RELISH
            } else {
                roof.span.as_metres() + TIE_END_RELISH
            };
            author.bearing(
                &format!("bearing-tie-{station_name}-on-{name}-masonry"),
                Anchor::new(&tie, [local_y, 0.15, 0.0]),
                Anchor::new(masonry, [station_x - x0, masonry_width * 0.5, wall_top]),
                z_axis,
                [x_axis, [0.0, 1.0, 0.0]],
                [0.14, 0.20],
                BearingKind::WallHead,
                EvidenceClass::ModernEngineeringInference,
            );
            author.transfer(
                format!("load-tie-{station_name}-to-{name}-masonry"),
                &tie,
                TransferTarget::element(masonry),
                TransferKind::Contact,
            );
        }

        let head_params = HousedBearingParams {
            // Opposing full-depth rafter housings must leave a sound web in
            // the king head; the 360 mm post is sized below so these 120 mm
            // pockets remain separate rather than silently unioning.
            housing_depth: length_millimeters(120),
            minimum_carrier_relish: length_millimeters(5),
            ..HousedBearingParams::default()
        };
        let head_housing_depth = head_params.housing_depth.as_meters();
        let mut station_principals: Vec<(&'static str, String, String)> = Vec::new();
        for data in &side_data {
            let (name, side, slope, normal) = (data.name, data.side, data.slope, data.normal);
            let wall_plate = data.wall_plate.as_str();
            let key = format!("principal-rafter-{name}-{station_name}");
            // The adapter is the sole exact-to-f64 lowering for principal
            // rafters. A station only adds its longitudinal translation.
            let mut extent = data.principal_extent.clone();
            extent.origin[0] += station_x;
            // Each rafter terminates on an internal bearing shoulder rather
            // than overlapping its mirror at the ridge centreline. For a
            // vertical-sided king head, the setback that leaves the requested
            // housing depth follows directly from the rafter's plan slope.
            let head_setback = KING_POST_WIDTH * 0.5 / slope[1].abs() - head_housing_depth;
            assert!(
                head_setback > 0.0 && head_setback < extent.size[0],
                "rafter-head setback must lie inside {key}: {head_setback} of {}",
                extent.size[0]
            );
            extent.size[0] -= head_setback;
            let head_point =
                extent.anchor([extent.size[0], extent.size[1] * 0.5, extent.size[2] * 0.5]);
            let head_node = author.node(
                format!("node-principal-{name}-{station_name}-head-shoulder"),
                head_point,
            );
            let principal = author.element(
                key.clone(),
                ElementRole::PrincipalRafter,
                extent,
                EvidenceClass::ModernEngineeringInference,
                1,
            );
            author.member(
                key.clone(),
                &principal,
                if side < 0.0 { &south_heel } else { &north_heel },
                &head_node,
                EvidenceClass::ModernEngineeringInference,
            );
            author.bearing(
                &format!("bearing-principal-{name}-{station_name}-on-wall-plate"),
                Anchor::new(
                    &principal,
                    [0.0, principal_width * 0.5, principal_depth * 0.5],
                ),
                Anchor::new(
                    wall_plate,
                    [station_x - x0, wall_plate_width * 0.5, wall_plate_height],
                ),
                normal,
                [x_axis, slope],
                [principal_width * 0.45, 0.12],
                BearingKind::AnchorContact,
                EvidenceClass::ModernEngineeringInference,
            );
            author.transfer(
                format!("load-principal-{name}-{station_name}-to-wall-plate"),
                &principal,
                TransferTarget::element(wall_plate),
                TransferKind::Contact,
            );
            for (position_name, _, purlin) in &data.purlins {
                let relation =
                    format!("joint-{position_name}-purlin-{name}-on-principal-{station_name}");
                let purlin_extent = author
                    .construction
                    .element(purlin)
                    .expect("purlin was authored before its supporting truss")
                    .extent
                    .clone();
                let node = author.node(
                    format!("node-{relation}"),
                    purlin_extent.anchor([
                        station_x - x0 + principal_width * 0.5,
                        purlin_width * 0.5,
                        0.0,
                    ]),
                );
                let joint_evidence =
                    Evidence::new("vernacular-trenched-purlin", EvidenceClass::RegionalAnalogy);
                author.joint_with_evidence(
                    &relation,
                    &node,
                    &[purlin, principal.as_str()],
                    JointKind::PurlinTrench,
                    joint_evidence.clone(),
                );
                author.fit_with_evidence(
                    &format!("fit-{position_name}-purlin-{name}-on-principal-{station_name}"),
                    &relation,
                    &PurlinToPrincipalTrenchRule,
                    &PurlinPrincipalTrenchParams {
                        trench_depth: PURLIN_TRENCH_DEPTH,
                        ..PurlinPrincipalTrenchParams::default()
                    },
                    joint_evidence,
                );
            }
            station_principals.push((name, key, head_node));
        }

        // The modeled member includes its tenon: it begins at the tip inside
        // the tie, while the shoulder and strut feet remain on the tie top.
        let king_base = [station_x, 0.0, wall_top + 0.30 - king_tenon_length];
        // The head projects above both internal rafter shoulders so their
        // complete depth remains surrounded by timber at the bearing plane.
        let king_top = [station_x, 0.0, ridge_inner[2] + 0.12];
        let king_top_node =
            author.node(format!("node-truss-{station_name}-king-head-top"), king_top);
        let king = author.element(
            format!("king-post-{station_name}"),
            ElementRole::KingPost,
            beam_between(king_base, king_top, KING_POST_WIDTH, KING_POST_WIDTH),
            EvidenceClass::ModernEngineeringInference,
            1,
        );
        let king_member = format!("king-post-{station_name}");
        author.member(
            king_member.clone(),
            &king,
            &king_foot,
            &king_top_node,
            EvidenceClass::ModernEngineeringInference,
        );
        author.joint(
            &format!("joint-king-post-to-tie-{station_name}"),
            &king_shoulder,
            &[tie_member.as_str(), king_member.as_str()],
            JointKind::KeyedThroughTenon,
            EvidenceClass::ModernEngineeringInference,
        );
        author.fit(
            &format!("fit-king-post-tie-{station_name}"),
            &format!("joint-king-post-to-tie-{station_name}"),
            &KingPostTieRule,
            &king_params,
            EvidenceClass::ModernEngineeringInference,
        );
        let strut_post_params = HousedBearingParams {
            housing_depth: length_millimeters(80),
            minimum_carrier_relish: length_millimeters(10),
            ..HousedBearingParams::default()
        };
        let strut_rafter_params = HousedBearingParams {
            housing_depth: length_millimeters(40),
            minimum_carrier_relish: length_millimeters(10),
            ..HousedBearingParams::default()
        };
        let strut_post_housing_depth = strut_post_params.housing_depth.as_meters();
        let strut_rafter_housing_depth = strut_rafter_params.housing_depth.as_meters();
        for data in &side_data {
            let (name, side) = (data.name, data.side);
            let (_, principal_member, _) = station_principals
                .iter()
                .find(|(side_name, _, _)| *side_name == name)
                .expect("station principal exists");
            let principal_extent = author
                .construction
                .element(principal_member.as_str())
                .expect("station principal extent exists")
                .extent
                .clone();
            let target_surface = principal_extent.anchor([
                principal_extent.size[0] * 0.58,
                principal_extent.size[1] * 0.5,
                0.0,
            ]);
            let foot_surface = [
                station_x,
                side * KING_POST_WIDTH * 0.5,
                wall_top + 0.30 + 0.35,
            ];
            let strut_axis = normalize(sub(target_surface, foot_surface))
                .expect("strut bearing faces are distinct");
            // Housing depth belongs to each carrier, not to the visible strut
            // length. Moving both endpoints inward along the common axis
            // keeps the authored member straight and places each endpoint on
            // its rule's internal bearing shoulder.
            let foot = sub(foot_surface, scale(strut_axis, strut_post_housing_depth));
            let target = add(
                target_surface,
                scale(strut_axis, strut_rafter_housing_depth),
            );
            let foot_node = author.node(
                format!("node-strut-{name}-{station_name}-post-shoulder"),
                foot,
            );
            let target_node = author.node(
                format!("node-strut-{name}-{station_name}-rafter-shoulder"),
                target,
            );
            let strut = author.element(
                format!("strut-{name}-{station_name}"),
                ElementRole::Strut,
                beam_between(foot, target, 0.18, 0.16),
                EvidenceClass::ModernEngineeringInference,
                1,
            );
            let strut_member = format!("strut-{name}-{station_name}");
            author.member(
                strut_member.clone(),
                &strut,
                &foot_node,
                &target_node,
                EvidenceClass::ModernEngineeringInference,
            );
            author.joint(
                &format!("joint-strut-foot-{name}-{station_name}"),
                &foot_node,
                &[king_member.as_str(), strut_member.as_str()],
                JointKind::HousedBearing,
                EvidenceClass::ModernEngineeringInference,
            );
            author.fit(
                &format!("fit-strut-foot-{name}-{station_name}"),
                &format!("joint-strut-foot-{name}-{station_name}"),
                &StrutToKingPostRule,
                &strut_post_params,
                EvidenceClass::ModernEngineeringInference,
            );
            author.joint(
                &format!("joint-strut-to-principal-{name}-{station_name}"),
                &target_node,
                &[strut_member.as_str(), principal_member.as_str()],
                JointKind::HousedBearing,
                EvidenceClass::ModernEngineeringInference,
            );
            author.fit(
                &format!("fit-strut-to-principal-{name}-{station_name}"),
                &format!("joint-strut-to-principal-{name}-{station_name}"),
                &StrutToRafterRule,
                &strut_rafter_params,
                EvidenceClass::ModernEngineeringInference,
            );
        }

        for (name, principal_member, head_node) in &station_principals {
            author.joint(
                &format!("joint-principal-head-{name}-{station_name}"),
                head_node,
                &[principal_member.as_str(), king_member.as_str()],
                JointKind::HousedBearing,
                EvidenceClass::ModernEngineeringInference,
            );
            author.fit(
                &format!("fit-principal-head-{name}-{station_name}"),
                &format!("joint-principal-head-{name}-{station_name}"),
                &RafterToKingPostRule,
                &head_params,
                EvidenceClass::ModernEngineeringInference,
            );
        }
        for (index, heel) in [(0_usize, &south_heel), (1_usize, &north_heel)] {
            let name = side_data[index].name;
            author.joint(
                &format!("joint-heel-{name}-{station_name}"),
                heel,
                &[tie_member.as_str(), station_principals[index].1.as_str()],
                JointKind::BearingSeat,
                EvidenceClass::ModernEngineeringInference,
            );
            author.fit(
                &format!("fit-heel-{name}-{station_name}"),
                &format!("joint-heel-{name}-{station_name}"),
                &HeelRule,
                &HeelParams::default(),
                EvidenceClass::ModernEngineeringInference,
            );
        }
    }

    for data in &side_data {
        let (name, masonry, wall_plate) =
            (data.name, data.masonry.as_str(), data.wall_plate.as_str());
        author.bearing(
            &format!("bearing-wall-plate-{name}-on-masonry"),
            Anchor::new(wall_plate, [bay_length * 0.5, wall_plate_width * 0.5, 0.0]),
            Anchor::new(masonry, [bay_length * 0.5, masonry_width * 0.5, wall_top]),
            z_axis,
            [x_axis, [0.0, 1.0, 0.0]],
            [bay_length * 0.95, wall_plate_width * 0.90],
            BearingKind::WallHead,
            EvidenceClass::RegionalAnalogy,
        );
        author.transfer(
            format!("load-wall-plate-{name}-to-masonry"),
            wall_plate,
            TransferTarget::element(masonry),
            TransferKind::Contact,
        );
        let support = format!("support-ground-{name}-wall");
        author.support(&support, masonry, "ground");
        author.transfer(
            format!("load-masonry-{name}-to-ground"),
            masonry,
            TransferTarget::support(&support),
            TransferKind::Ground,
        );
    }

    author.construction
}

/// Runs `joiner`'s layered validation over the authored hypothesis.
pub(crate) fn check(construction: &Construction) -> ValidationReport {
    validate(construction)
}

/// One line of counts plus a signature that pins the authored graph.
pub(crate) fn stats_line(construction: &Construction) -> String {
    format!(
        "nodes={} elements={} members={} joints={} rule_applications={} part_edits={} bearings={} supports={} transfers={} evidence_sources={} signature={:016x}",
        construction.nodes().len(),
        construction
            .elements()
            .iter()
            .filter(|element| element.present)
            .count(),
        construction.members().len(),
        construction.relations().len(),
        construction.applications().len(),
        construction.part_edits().len(),
        construction.contacts().len(),
        construction.supports().len(),
        construction.transfers().len(),
        construction.evidence_sources().len(),
        deterministic_signature(construction)
    )
}

/// An FNV-1a fold over every stable key and every declared number.
///
/// Deliberately built from the graph's own vocabulary rather than from a
/// debug rendering, so a change to `joiner`'s internals cannot silently
/// move it.
pub(crate) fn deterministic_signature(construction: &Construction) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut fold = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for source in construction.evidence_sources() {
        fold(source.key.as_bytes());
        fold(source.class.label().as_bytes());
    }
    for node in construction.nodes() {
        fold(node.key.as_bytes());
        for value in node.point {
            fold(&value.to_bits().to_le_bytes());
        }
    }
    for element in construction.elements() {
        fold(element.key.as_bytes());
        fold(element.role.as_bytes());
        fold(element.material.as_bytes());
        fold(&[u8::from(element.present)]);
        fold(&element.required_supports.to_le_bytes());
        for value in element
            .extent
            .origin
            .into_iter()
            .chain(element.extent.size)
            .chain(element.extent.axes.into_iter().flatten())
        {
            fold(&value.to_bits().to_le_bytes());
        }
    }
    for member in construction.members() {
        fold(member.key.as_bytes());
        fold(member.element.as_bytes());
        fold(member.from.as_bytes());
        fold(member.to.as_bytes());
    }
    for relation in construction.relations() {
        fold(relation.key.as_bytes());
        fold(relation.kind.label().as_bytes());
        fold(relation.detail.as_bytes());
    }
    for application in construction.applications() {
        fold(application.key.as_bytes());
        fold(application.rule.as_bytes());
        fold(application.relation.as_bytes());
    }
    for edit in construction.part_edits() {
        fold(edit.target.as_bytes());
        fold(edit.op.label().as_bytes());
        fold(edit.op.tool().key.as_bytes());
        fold(&edit.op.tool().recipe.recipe_fingerprint().0.to_le_bytes());
        for value in edit.op.tool().placement.rows.into_iter().flatten() {
            fold(&value.to_bits().to_le_bytes());
        }
    }
    for contact in construction.contacts() {
        fold(contact.key.as_bytes());
        fold(contact.detail.as_bytes());
        fold(contact.carried.element.as_bytes());
        fold(contact.carrier.element.as_bytes());
        for value in contact
            .carried
            .local
            .into_iter()
            .chain(contact.carrier.local)
            .chain(contact.normal)
            .chain(contact.tangents.into_iter().flatten())
            .chain(contact.minimum_overlap)
        {
            fold(&value.to_bits().to_le_bytes());
        }
    }
    for support in construction.supports() {
        fold(support.key.as_bytes());
        fold(support.element.as_bytes());
        fold(support.ground.as_bytes());
    }
    for transfer in construction.transfers() {
        fold(transfer.key.as_bytes());
        fold(transfer.from.as_bytes());
        fold(transfer.kind.label().as_bytes());
        match &transfer.to {
            TransferTarget::Element(key) | TransferTarget::Support(key) => fold(key.as_bytes()),
            _ => {}
        }
    }
    hash
}

/// Prints one graph record, with the measurements behind its claim.
///
/// Selectors keep the lab's own vocabulary: `joint:` finds a member/member
/// relation and `bearing:` finds a contact patch, because that is what they
/// are in this building.
pub(crate) fn explain(
    construction: &Construction,
    selector: &str,
) -> Result<Option<String>, String> {
    let Some((category, key)) = selector.split_once(':') else {
        let categories = matching_categories(construction, selector);
        return match categories.as_slice() {
            [] => Ok(None),
            [category] => explain(construction, &format!("{category}:{selector}")),
            _ => Err(format!(
                "ambiguous structural key {selector:?}; use one of {}",
                categories
                    .iter()
                    .map(|category| format!("{category}:{selector}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
    };
    let explanation = match category {
        "element" => construction.element(key).map(|element| {
            let mut out = format!(
                "element {} role={} evidence={} source={} required_direct_supports={} present={}",
                element.key,
                element.role,
                element.evidence.class,
                element.evidence.source,
                element.required_supports,
                element.present
            );
            match trace_to_ground(construction, &element.key) {
                Some(path) => {
                    let _ = write!(out, "\nload path: {}", path.join(" -> "));
                }
                None => out.push_str("\nload path: none"),
            }
            out
        }),
        "node" => construction.node(key).map(|node| {
            let members: Vec<&str> = construction
                .members()
                .iter()
                .filter(|member| member.from == node.key || member.to == node.key)
                .map(|member| member.key.as_str())
                .collect();
            format!(
                "node {} point={:?} members={}",
                node.key,
                node.point,
                members.join(",")
            )
        }),
        "member" => construction.member(key).map(|member| {
            format!(
                "member {} element={} from={} to={} evidence={} source={}",
                member.key,
                member.element,
                member.from,
                member.to,
                member.evidence.class,
                member.evidence.source
            )
        }),
        "joint" => construction
            .relation(key)
            .and_then(|relation| match &relation.kind {
                RelationKind::MemberMember { node, members } => {
                    let geometry = construction
                        .applications()
                        .iter()
                        .find(|application| application.relation == relation.key)
                        .map_or_else(
                            || "uncut-connectivity-hypothesis".to_owned(),
                            |application| format!("rule-applied:{}", application.rule),
                        );
                    Some(format!(
                        "joint {} kind={} geometry={} node={} members={} evidence={} source={}",
                        relation.key,
                        relation.detail,
                        geometry,
                        node,
                        members.join(","),
                        relation.evidence.class,
                        relation.evidence.source
                    ))
                }
                _ => None,
            }),
        "bearing" => construction.contact(key).and_then(|contact| {
            let measurement = measure_contact(construction, contact)?;
            Some(format!(
                "bearing {} kind={} meaning={} geometry=analytic-anchor-only carried={} carrier={} evidence={} source={}\ncarried_anchor={:?} carrier_anchor={:?} normal={:?} tangents={:?}\ngap={:.12}m tangent_offsets={:?} tolerance={:.12}m overlaps={:?} minima={:?}",
                contact.key,
                contact.detail,
                contact.meaning.label(),
                contact.carried.element,
                contact.carrier.element,
                contact.evidence.class,
                contact.evidence.source,
                measurement.carried_point,
                measurement.carrier_point,
                contact.normal,
                contact.tangents,
                measurement.gap,
                measurement.tangent_offsets,
                CONTACT_TOLERANCE,
                measurement.overlap,
                contact.minimum_overlap
            ))
        }),
        "support" => construction.support(key).map(|support| {
            format!(
                "support {} element={} ground={} restraints={:?}",
                support.key, support.element, support.ground, support.restraints.translation
            )
        }),
        "transfer" => construction.transfer(key).map(|transfer| {
            let target = match &transfer.to {
                TransferTarget::Element(key) | TransferTarget::Support(key) => key.as_str(),
                _ => "unknown",
            };
            format!(
                "transfer {} kind={} from={} to={target} witnessed={}",
                transfer.key,
                transfer.kind.label(),
                transfer.from,
                is_witnessed(construction, transfer)
            )
        }),
        "evidence" => construction.evidence_source(key).map(|source| {
            format!(
                "evidence {} class={} url={}\nclaim: {}",
                source.key, source.class, source.url, source.note
            )
        }),
        _ => {
            return Err(format!(
                "unknown selector type {category:?}; expected element, node, member, joint, bearing, support, transfer, or evidence"
            ));
        }
    };
    Ok(explanation)
}

fn matching_categories(construction: &Construction, key: &str) -> Vec<&'static str> {
    let mut categories = Vec::new();
    if construction.element(key).is_some() {
        categories.push("element");
    }
    if construction.node(key).is_some() {
        categories.push("node");
    }
    if construction.member(key).is_some() {
        categories.push("member");
    }
    if construction.relation(key).is_some() {
        categories.push("joint");
    }
    if construction.contact(key).is_some() {
        categories.push("bearing");
    }
    if construction.support(key).is_some() {
        categories.push("support");
    }
    if construction.transfer(key).is_some() {
        categories.push("transfer");
    }
    if construction.evidence_source(key).is_some() {
        categories.push("evidence");
    }
    categories
}

fn beam_between(from: Vec3, to: Vec3, width: f64, depth: f64) -> OrientedBox {
    let delta = sub(to, from);
    let length = norm(delta);
    let along = scale(delta, 1.0 / length);
    let across = [1.0, 0.0, 0.0];
    let normal =
        normalize(cross(along, across)).expect("authored beam directions are non-degenerate");
    OrientedBox {
        origin: sub(
            sub(from, scale(across, width * 0.5)),
            scale(normal, depth * 0.5),
        ),
        axes: [along, across, normal],
        size: [length, width, depth],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Construction {
        western_bay(&BasilicaParams::default())
    }

    fn contact_normal(construction: &Construction, key: &str) -> Vec3 {
        construction.contact(key).expect("contact exists").normal
    }

    fn shift(construction: &mut Construction, key: &str, delta: Vec3) {
        let moved = construction
            .element(key)
            .expect("test element exists")
            .extent
            .translated(delta);
        construction
            .set_element_extent(key, moved)
            .expect("test element exists");
    }

    #[test]
    fn canonical_bay_is_clean_and_every_element_reaches_ground() {
        // The finished reference bay must satisfy every graph validator, and
        // generated keys count as real load-path elements rather than render
        // decorations that can be omitted from the support proof.
        let construction = model();
        let report = check(&construction);
        assert!(report.is_clean(), "{report}");
        assert_eq!(construction.elements().len(), 34);
        assert!(
            construction
                .elements()
                .iter()
                .all(|element| trace_to_ground(&construction, &element.key).is_some()),
            "every element has a named route to ground"
        );
    }

    #[test]
    fn canonical_element_order_and_graph_signature_are_pinned() {
        // Stable authoring order keeps OBJ groups, diagnostics, and generated
        // key identities deterministic across otherwise equivalent builds.
        let construction = model();
        let expected = [
            "masonry-south",
            "wall-plate-south",
            "roof-covering-south",
            "boarding-south",
            "common-rafter-south-00",
            "common-rafter-south-01",
            "common-rafter-south-02",
            "purlin-south-eave",
            "purlin-south-mid",
            "purlin-south-upper",
            "masonry-north",
            "wall-plate-north",
            "roof-covering-north",
            "boarding-north",
            "common-rafter-north-00",
            "common-rafter-north-01",
            "common-rafter-north-02",
            "purlin-north-eave",
            "purlin-north-mid",
            "purlin-north-upper",
            "tie-beam-west",
            "principal-rafter-south-west",
            "principal-rafter-north-west",
            "king-post-west",
            "joint-king-post-to-tie-west-key",
            "strut-south-west",
            "strut-north-west",
            "tie-beam-east",
            "principal-rafter-south-east",
            "principal-rafter-north-east",
            "king-post-east",
            "joint-king-post-to-tie-east-key",
            "strut-south-east",
            "strut-north-east",
        ];
        assert_eq!(
            construction
                .elements()
                .iter()
                .map(|element| element.key.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            deterministic_signature(&construction),
            deterministic_signature(&model()),
            "the signature is a pure function of the authored graph"
        );
    }

    #[test]
    fn concrete_timber_rules_fit_primary_trusses_and_secondary_roof_crossings() {
        // The 18 primary-truss fits remain unchanged. Eighteen common-rafter
        // seats and twelve purlin trenches then make every secondary crossing
        // constructive geometry rather than an analytic contact label.
        let construction = model();
        assert_eq!(construction.applications().len(), 48);
        for (rule, expected) in [
            (joiner_timber::HEEL_RULE_KEY, 4),
            (joiner_timber::KING_POST_TIE_RULE_KEY, 2),
            (joiner_timber::STRUT_KING_POST_RULE_KEY, 4),
            (joiner_timber::STRUT_RAFTER_RULE_KEY, 4),
            (joiner_timber::RAFTER_KING_POST_RULE_KEY, 4),
            (joiner_timber::COMMON_RAFTER_PURLIN_SEAT_RULE_KEY, 18),
            (joiner_timber::PURLIN_PRINCIPAL_TRENCH_RULE_KEY, 12),
        ] {
            assert_eq!(
                construction
                    .applications()
                    .iter()
                    .filter(|application| application.rule == rule)
                    .count(),
                expected,
                "{rule}"
            );
        }
        for station in ["west", "east"] {
            assert_eq!(
                construction
                    .part_edits_for(&format!("tie-beam-{station}"))
                    .count(),
                3,
                "two heel housings and one king-post mortise"
            );
            assert_eq!(
                construction
                    .part_edits_for(&format!("king-post-{station}"))
                    .count(),
                7,
                "two tenon shoulders, one key slot, and four housed bearings"
            );
            for side in ["south", "north"] {
                assert_eq!(
                    construction
                        .part_edits_for(&format!("principal-rafter-{side}-{station}"))
                        .count(),
                    5,
                    "one heel seat, one strut housing, and three purlin trenches"
                );
                assert_eq!(
                    construction
                        .part_edits_for(&format!("strut-{side}-{station}"))
                        .count(),
                    0,
                    "full-section struts terminate on authored bearing shoulders"
                );
            }
        }
        let strut_head = construction
            .contact("contact-joint-strut-to-principal-south-west")
            .expect("fitted strut-head contact exists");
        assert_eq!(strut_head.carried.element, "principal-rafter-south-west");
        assert_eq!(strut_head.carrier.element, "strut-south-west");
        assert!(
            construction.transfers().iter().all(|transfer| transfer.key
                != "load-principal-rafter-south-west-through-joint-strut-to-principal-south-west"),
            "the physical bearing closes a truss triangle, so it must not close a cycle in the acyclic support graph"
        );
        for side in ["south", "north"] {
            for ordinal in 0..3 {
                assert_eq!(
                    construction
                        .part_edits_for(&format!("common-rafter-{side}-{ordinal:02}"))
                        .count(),
                    3,
                    "each common rafter is seated over all three purlins"
                );
            }
            for position in ["eave", "mid", "upper"] {
                assert_eq!(
                    construction
                        .part_edits_for(&format!("purlin-{side}-{position}"))
                        .count(),
                    0,
                    "full-section purlins are carried without being cut"
                );
            }
        }
    }

    #[test]
    fn every_concrete_timber_recipe_evaluates_cleanly() {
        // Assembly compilation reports only a part id. Evaluating every
        // primary and secondary fitted timber by stable element key makes a
        // failed crossing name the member and retains the constructive
        // diagnostics that explain it.
        let construction = model();
        for element in construction.elements().iter().filter(|element| {
            matches!(
                ElementRole::from_label(&element.role),
                Some(
                    ElementRole::PrincipalRafter
                        | ElementRole::CommonRafter
                        | ElementRole::Purlin
                        | ElementRole::TieBeam
                        | ElementRole::KingPost
                        | ElementRole::Strut
                )
            ) || element.role == "king-post-key"
        }) {
            let recipe = joiner::compose(&construction, element)
                .unwrap_or_else(|error| panic!("{} does not compose: {error}", element.key));
            let evaluated = exedra_constructive::evaluate::evaluate(
                &recipe,
                &exedra_constructive::tessellate::EvalPolicy::default(),
            )
            .unwrap_or_else(|error| panic!("{} does not evaluate: {error}", element.key));
            assert_eq!(
                evaluated.bodies.len(),
                1,
                "{}: {:?}",
                element.key,
                evaluated.report.diagnostics
            );
            assert!(
                evaluated
                    .report
                    .clean_at(exedra_constructive::evaluate::Severity::Warning),
                "{}: {:?}",
                element.key,
                evaluated.report.diagnostics
            );
            assert!(
                evaluated.bodies[0].body.mesh.validate_deep().is_empty(),
                "{}",
                element.key
            );
        }
    }

    #[test]
    fn south_covering_float_and_embed_are_rejected_by_the_same_contact() {
        let normal = contact_normal(&model(), "bearing-covering-on-boarding-south");

        let mut floating = model();
        shift(&mut floating, "roof-covering-south", scale(normal, 0.05));
        assert!(check(&floating).has("floating-contact", "bearing-covering-on-boarding-south"));

        let mut embedded = model();
        shift(&mut embedded, "roof-covering-south", scale(normal, -0.05));
        assert!(check(&embedded).has("embedded-contact", "bearing-covering-on-boarding-south"));
    }

    #[test]
    fn contact_tolerance_accepts_sub_epsilon_and_rejects_beyond_it() {
        let normal = contact_normal(&model(), "bearing-covering-on-boarding-south");
        for signed_distance in [CONTACT_TOLERANCE * 0.5, -CONTACT_TOLERANCE * 0.5] {
            let mut within = model();
            shift(
                &mut within,
                "roof-covering-south",
                scale(normal, signed_distance),
            );
            let report = check(&within);
            assert!(
                !report.issues.iter().any(|issue| {
                    issue.key == "bearing-covering-on-boarding-south"
                        && matches!(issue.code, "floating-contact" | "embedded-contact")
                }),
                "sub-epsilon offsets are coincident"
            );
        }
        for (signed_distance, expected) in [
            (CONTACT_TOLERANCE * 2.0, "floating-contact"),
            (-CONTACT_TOLERANCE * 2.0, "embedded-contact"),
        ] {
            let mut outside = model();
            shift(
                &mut outside,
                "roof-covering-south",
                scale(normal, signed_distance),
            );
            assert!(check(&outside).has(expected, "bearing-covering-on-boarding-south"));
        }
    }

    #[test]
    fn missing_south_mid_purlin_breaks_required_support_coverage() {
        let mut construction = model();
        construction
            .set_element_present("purlin-south-mid", false)
            .expect("test element exists");
        let report = check(&construction);
        assert!(report.has("insufficient-direct-supports", "common-rafter-south-00"));
        assert!(report.has_code("missing-contact-element"));
    }

    #[test]
    fn explanations_use_stable_named_paths() {
        let construction = model();
        let explanation = explain(&construction, "element:roof-covering-south")
            .expect("valid selector")
            .expect("named element explains");
        assert!(explanation.contains("roof-covering-south -> boarding-south"));
        assert!(explanation.ends_with("support-ground-south-wall -> ground"));
    }

    #[test]
    fn fake_bearing_transfer_cannot_repair_an_omitted_purlin() {
        let mut construction = model();
        construction
            .set_element_present("purlin-south-mid", false)
            .expect("test element exists");
        construction
            .add_transfer(TransferEdge::new(
                "load-fake-south-common-to-north-purlin",
                "common-rafter-south-00",
                TransferTarget::element("purlin-north-mid"),
                TransferKind::Contact,
            ))
            .expect("transfer references resolve");
        let report = check(&construction);
        assert!(report.has(
            "unwitnessed-contact-transfer",
            "load-fake-south-common-to-north-purlin"
        ));
        assert!(report.has("insufficient-direct-supports", "common-rafter-south-00"));
    }

    #[test]
    fn transfer_kind_cannot_claim_a_different_witness_semantics() {
        let mut construction = model();
        // The covering really does bear on the boarding; claiming the same
        // route as a joint does not make the joint exist.
        construction
            .add_transfer(TransferEdge::new(
                "load-covering-to-boarding-south-as-joint",
                "roof-covering-south",
                TransferTarget::element("boarding-south"),
                TransferKind::Joint,
            ))
            .expect("transfer references resolve");
        assert!(check(&construction).has(
            "unwitnessed-joint-transfer",
            "load-covering-to-boarding-south-as-joint"
        ));
    }

    #[test]
    fn duplicate_transfers_do_not_inflate_direct_support_multiplicity() {
        let mut construction = model();
        for key in ["purlin-south-mid", "purlin-south-upper"] {
            construction
                .set_element_present(key, false)
                .expect("test element exists");
        }
        for ordinal in 0..3 {
            construction
                .add_transfer(TransferEdge::new(
                    &format!("load-duplicate-common-to-eave-{ordinal}"),
                    "common-rafter-south-00",
                    TransferTarget::element("purlin-south-eave"),
                    TransferKind::Contact,
                ))
                .expect("transfer references resolve");
        }
        let report = check(&construction);
        assert!(report.has_code("duplicate-transfer-route"));
        assert!(report.has("insufficient-direct-supports", "common-rafter-south-00"));
    }

    #[test]
    fn a_ground_transfer_must_target_the_support_for_its_own_element() {
        let mut construction = model();
        construction
            .add_transfer(TransferEdge::new(
                "load-masonry-south-to-north-ground",
                "masonry-south",
                TransferTarget::support("support-ground-north-wall"),
                TransferKind::Ground,
            ))
            .expect("transfer references resolve");
        assert!(check(&construction).has(
            "unwitnessed-ground-transfer",
            "load-masonry-south-to-north-ground"
        ));
    }

    #[test]
    fn direct_transfer_cycle_is_rejected() {
        let mut construction = model();
        construction
            .add_transfer(TransferEdge::new(
                "load-cycle-south-masonry-to-covering",
                "masonry-south",
                TransferTarget::element("roof-covering-south"),
                TransferKind::Contact,
            ))
            .expect("transfer references resolve");
        assert!(check(&construction).has_code("load-transfer-cycle"));
    }

    #[test]
    fn a_moved_element_leaves_its_own_centreline_behind() {
        // Moving the fitted tie leaves both its authored member and the heel
        // relation nodes behind; rule-generated cuts do not mask that graph
        // incoherence.
        let mut construction = model();
        shift(&mut construction, "tie-beam-west", [0.0, 0.0, 2.0]);
        let report = check(&construction);
        assert!(report.has("member-endpoint-outside-extent", "tie-beam-west"));
        assert!(report.has("relation-not-incident-to-member", "joint-heel-south-west"));
    }

    #[test]
    fn typed_selectors_disambiguate_cross_category_keys() {
        let construction = model();
        let error = explain(&construction, "wall-plate-south")
            .expect_err("bare element/member key is ambiguous");
        assert!(error.contains("element:wall-plate-south"));
        assert!(error.contains("member:wall-plate-south"));
        for selector in ["element:wall-plate-south", "member:wall-plate-south"] {
            assert!(
                explain(&construction, selector)
                    .expect("valid selector")
                    .is_some()
            );
        }
    }

    #[test]
    fn bearing_and_evidence_explanations_include_measured_witnesses() {
        let construction = model();
        let bearing = explain(
            &construction,
            "bearing:bearing-principal-south-east-on-wall-plate",
        )
        .expect("valid selector")
        .expect("bearing exists");
        assert!(bearing.contains("geometry=analytic-anchor-only"));
        assert!(bearing.contains("kind=anchor-contact"));
        assert!(bearing.contains("gap="));
        assert!(bearing.contains("tolerance=0.000000001000m"));
        assert!(bearing.contains("overlaps="));
        assert!(bearing.contains("source=tfec-modern-truss-detailing"));

        let evidence = explain(&construction, "evidence:tfec-modern-truss-detailing")
            .expect("valid selector")
            .expect("evidence exists");
        assert!(evidence.contains("class=modern-engineering-inference"));
        assert!(evidence.contains("claim:"));

        let joint = explain(&construction, "joint:joint-principal-head-south-west")
            .expect("valid selector")
            .expect("joint exists");
        // The paired ridge interfaces are concrete housed bearings, so their
        // explanation must name the applied rule rather than an old apex
        // connectivity hypothesis.
        assert!(joint.contains("kind=housed-bearing"));
        assert!(joint.contains(&format!(
            "geometry=rule-applied:{}",
            joiner_timber::RAFTER_KING_POST_RULE_KEY
        )));

        let fitted = explain(&construction, "joint:joint-king-post-to-tie-west")
            .expect("valid selector")
            .expect("joint exists");
        // A concrete rule is reported by its stable key, so an explanation
        // cannot accidentally describe its edited geometry as uncut.
        assert!(fitted.contains(&format!(
            "geometry=rule-applied:{}",
            joiner_timber::KING_POST_TIE_RULE_KEY
        )));
    }
}
