// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::HashSet;
use std::fmt::Write as _;

use basilica_ruin::BasilicaParams;

pub(crate) type Vec3 = [f64; 3];

const CONTACT_TOLERANCE: f64 = 1.0e-9;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceClass {
    Observed,
    DocumentedReconstruction,
    RegionalAnalogy,
    ModernEngineeringInference,
}

impl core::fmt::Display for EvidenceClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let label = match self {
            Self::Observed => "observed",
            Self::DocumentedReconstruction => "documented-reconstruction",
            Self::RegionalAnalogy => "regional-analogy",
            Self::ModernEngineeringInference => "modern-engineering-inference",
        };
        f.write_str(label)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EvidenceSource {
    pub(crate) key: String,
    pub(crate) class: EvidenceClass,
    pub(crate) url: String,
    pub(crate) note: String,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct NodeId(pub(crate) usize);

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ElementId(pub(crate) usize);

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MemberId(pub(crate) usize);

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SupportId(pub(crate) usize);

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) key: String,
    pub(crate) point: Vec3,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum ElementRole {
    RoofCovering,
    Boarding,
    CommonRafter,
    Purlin,
    Ridge,
    PrincipalRafter,
    TieBeam,
    KingPost,
    Strut,
    WallPlate,
    Masonry,
}

impl ElementRole {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::RoofCovering => "roof-covering",
            Self::Boarding => "boarding",
            Self::CommonRafter => "common-rafter",
            Self::Purlin => "purlin",
            Self::Ridge => "ridge",
            Self::PrincipalRafter => "principal-rafter",
            Self::TieBeam => "tie-beam",
            Self::KingPost => "king-post",
            Self::Strut => "strut",
            Self::WallPlate => "wall-plate",
            Self::Masonry => "masonry",
        }
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
            Self::Purlin | Self::Ridge => "semantic-purlin",
            Self::PrincipalRafter => "semantic-principal-rafter",
            Self::TieBeam => "semantic-tie-beam",
            Self::KingPost => "semantic-king-post",
            Self::Strut => "semantic-strut",
            Self::WallPlate => "semantic-wall-plate",
            Self::Masonry => "semantic-masonry",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OrientedBox {
    pub(crate) origin: Vec3,
    pub(crate) axes: [Vec3; 3],
    pub(crate) size: Vec3,
}

impl OrientedBox {
    pub(crate) fn anchor(&self, local: Vec3) -> Vec3 {
        add(
            self.origin,
            add(
                scale(self.axes[0], local[0]),
                add(scale(self.axes[1], local[1]), scale(self.axes[2], local[2])),
            ),
        )
    }

    pub(crate) fn center(&self) -> Vec3 {
        self.anchor([self.size[0] * 0.5, self.size[1] * 0.5, self.size[2] * 0.5])
    }

    fn projection_interval(&self, axis: Vec3) -> (f64, f64) {
        let center = dot(self.center(), axis);
        let radius = (0..3)
            .map(|i| self.size[i] * 0.5 * dot(self.axes[i], axis).abs())
            .sum::<f64>();
        (center - radius, center + radius)
    }

    fn contains_point(&self, point: Vec3, tolerance: f64) -> bool {
        let delta = sub(point, self.origin);
        (0..3).all(|axis| {
            let coordinate = dot(delta, self.axes[axis]);
            coordinate >= -tolerance && coordinate <= self.size[axis] + tolerance
        })
    }

    fn contains_local(&self, local: Vec3, tolerance: f64) -> bool {
        finite(local)
            && (0..3)
                .all(|axis| local[axis] >= -tolerance && local[axis] <= self.size[axis] + tolerance)
    }

    #[cfg(test)]
    fn translate(&mut self, delta: Vec3) {
        self.origin = add(self.origin, delta);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Element {
    pub(crate) key: String,
    pub(crate) role: ElementRole,
    pub(crate) solid: OrientedBox,
    pub(crate) evidence: EvidenceClass,
    pub(crate) evidence_source: String,
    pub(crate) required_supports: usize,
    pub(crate) present: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Member {
    pub(crate) key: String,
    pub(crate) element: ElementId,
    pub(crate) from: NodeId,
    pub(crate) to: NodeId,
    pub(crate) evidence: EvidenceClass,
    pub(crate) evidence_source: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum JointKind {
    BearingSeat,
    MortiseTenon,
    HousedMortiseTenon,
    ThroughTenon,
}

#[derive(Clone, Debug)]
pub(crate) struct Joint {
    pub(crate) key: String,
    pub(crate) node: NodeId,
    pub(crate) members: Vec<MemberId>,
    pub(crate) kind: JointKind,
    pub(crate) evidence: EvidenceClass,
    pub(crate) evidence_source: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum BearingKind {
    Surface,
    CrossedSeat,
    AnchorContact,
    WallHead,
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct Anchor {
    pub(crate) element: ElementId,
    pub(crate) local: Vec3,
}

#[derive(Clone, Debug)]
pub(crate) struct Bearing {
    pub(crate) key: String,
    pub(crate) carried: Anchor,
    pub(crate) carrier: Anchor,
    pub(crate) normal: Vec3,
    pub(crate) tangents: [Vec3; 2],
    pub(crate) minimum_overlap: [f64; 2],
    pub(crate) kind: BearingKind,
    pub(crate) evidence: EvidenceClass,
    pub(crate) evidence_source: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct Restraints {
    pub(crate) translation: [bool; 3],
}

#[derive(Clone, Debug)]
pub(crate) struct Support {
    pub(crate) key: String,
    pub(crate) element: ElementId,
    pub(crate) ground: String,
    pub(crate) restraints: Restraints,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TransferKind {
    Bearing,
    Joint,
    Ground,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TransferTarget {
    Element(ElementId),
    Support(SupportId),
}

#[derive(Clone, Debug)]
pub(crate) struct LoadTransfer {
    pub(crate) key: String,
    pub(crate) from: ElementId,
    pub(crate) to: TransferTarget,
    pub(crate) kind: TransferKind,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StructuralModel {
    pub(crate) evidence_sources: Vec<EvidenceSource>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) elements: Vec<Element>,
    pub(crate) members: Vec<Member>,
    pub(crate) joints: Vec<Joint>,
    pub(crate) bearings: Vec<Bearing>,
    pub(crate) supports: Vec<Support>,
    pub(crate) transfers: Vec<LoadTransfer>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidationIssue {
    pub(crate) code: &'static str,
    pub(crate) key: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ValidationReport {
    pub(crate) issues: Vec<ValidationIssue>,
}

#[derive(Copy, Clone, Debug)]
struct BearingMeasurement {
    carried_point: Vec3,
    carrier_point: Vec3,
    gap: f64,
    tangent_offsets: [f64; 2],
    overlap: [f64; 2],
}

impl BearingMeasurement {
    fn anchors_aligned(self) -> bool {
        self.gap.abs() <= CONTACT_TOLERANCE
            && self
                .tangent_offsets
                .iter()
                .all(|offset| offset.abs() <= CONTACT_TOLERANCE)
    }
}

impl ValidationReport {
    pub(crate) fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

impl core::fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_clean() {
            return f.write_str("structural validation: clean");
        }
        writeln!(f, "structural validation: {} issue(s)", self.issues.len())?;
        for issue in &self.issues {
            writeln!(f, "{} {}: {}", issue.code, issue.key, issue.message)?;
        }
        Ok(())
    }
}

impl TransferKind {
    fn unwitnessed_code(self) -> &'static str {
        match self {
            Self::Bearing => "unwitnessed-bearing-transfer",
            Self::Joint => "unwitnessed-joint-transfer",
            Self::Ground => "unwitnessed-ground-transfer",
        }
    }
}

impl StructuralModel {
    pub(crate) fn western_bay(params: &BasilicaParams) -> Self {
        let mut builder = ModelBuilder::default();
        builder.add_evidence_sources();

        let x0 = 0.0;
        let x1 = 4.0;
        let bay_length = x1 - x0;
        let half_nave = params.nave_width * 0.5;
        let wall_top = params.nave_wall_height;
        let roof_length = (half_nave * half_nave + params.roof_rise * params.roof_rise).sqrt();
        let roof_cos = half_nave / roof_length;
        let roof_sin = params.roof_rise / roof_length;

        let covering_depth = 0.08;
        let boarding_depth = 0.04;
        let common_depth = 0.16;
        let purlin_depth = 0.22;
        let principal_depth = 0.24;
        let stack_depth =
            covering_depth + boarding_depth + common_depth + purlin_depth + principal_depth;
        let common_width = 0.12;
        let purlin_width = 0.18;
        let principal_width = 0.26;
        let wall_plate_width = 0.30;
        let wall_plate_height = 0.18;
        let masonry_width = 0.45;
        let x_axis = [1.0, 0.0, 0.0];
        let z_axis = [0.0, 0.0, 1.0];

        let mut side_data = Vec::new();
        for (side_name, side) in [("south", -1.0), ("north", 1.0)] {
            let slope = [0.0, -side * roof_cos, roof_sin];
            let normal = [0.0, side * roof_sin, roof_cos];
            let wall_point = [0.0, side * half_nave, wall_top + wall_plate_height];
            let outer_eave = add(wall_point, scale(normal, stack_depth));

            let masonry = builder.element(
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
            let wall_plate = builder.element(
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
            let plate_from = builder.node(
                format!("node-wall-plate-{side_name}-west"),
                [x0, side * half_nave, wall_top + wall_plate_height * 0.5],
            );
            let plate_to = builder.node(
                format!("node-wall-plate-{side_name}-east"),
                [x1, side * half_nave, wall_top + wall_plate_height * 0.5],
            );
            let wall_plate_member = builder.member(
                format!("wall-plate-{side_name}"),
                wall_plate,
                plate_from,
                plate_to,
                EvidenceClass::RegionalAnalogy,
            );

            let covering = builder.element(
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
            let boarding = builder.element(
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
            builder.bearing(
                format!("bearing-covering-on-boarding-{side_name}"),
                Anchor {
                    element: covering,
                    local: [bay_length * 0.5, roof_length * 0.5, 0.0],
                },
                Anchor {
                    element: boarding,
                    local: [bay_length * 0.5, roof_length * 0.5, boarding_depth],
                },
                normal,
                [x_axis, slope],
                [bay_length * 0.95, roof_length * 0.95],
                BearingKind::Surface,
                EvidenceClass::RegionalAnalogy,
            );
            builder.transfer(
                format!("load-covering-to-boarding-{side_name}"),
                covering,
                TransferTarget::Element(boarding),
                TransferKind::Bearing,
            );

            let mut common_rafters = Vec::new();
            for (ordinal, x) in [x0 + 0.45, (x0 + x1) * 0.5, x1 - 0.45]
                .into_iter()
                .enumerate()
            {
                let key = format!("common-rafter-{side_name}-{ordinal:02}");
                let solid = OrientedBox {
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
                let element = builder.element(
                    key.clone(),
                    ElementRole::CommonRafter,
                    solid,
                    EvidenceClass::RegionalAnalogy,
                    3,
                );
                let from = builder.node(
                    format!("node-{key}-eave"),
                    add(
                        [x, 0.0, 0.0],
                        sub(
                            outer_eave,
                            scale(normal, covering_depth + boarding_depth + common_depth * 0.5),
                        ),
                    ),
                );
                let to = builder.node(
                    format!("node-{key}-ridge"),
                    add(builder.nodes[from.0].point, scale(slope, roof_length)),
                );
                builder.member(key, element, from, to, EvidenceClass::RegionalAnalogy);
                builder.bearing(
                    format!("bearing-boarding-on-{side_name}-common-{ordinal:02}"),
                    Anchor {
                        element: boarding,
                        local: [x - x0, roof_length * 0.5, 0.0],
                    },
                    Anchor {
                        element,
                        local: [roof_length * 0.5, common_width * 0.5, common_depth],
                    },
                    normal,
                    [x_axis, slope],
                    [common_width * 0.95, roof_length * 0.95],
                    BearingKind::Surface,
                    EvidenceClass::RegionalAnalogy,
                );
                builder.transfer(
                    format!("load-boarding-{side_name}-to-common-{ordinal:02}"),
                    boarding,
                    TransferTarget::Element(element),
                    TransferKind::Bearing,
                );
                common_rafters.push((element, x));
            }

            let mut purlins = Vec::new();
            for (position_name, t) in [
                ("eave", 0.42),
                ("mid", roof_length * 0.5),
                ("ridge", roof_length - 0.42),
            ] {
                let key = if position_name == "ridge" {
                    format!("ridge-member-{side_name}")
                } else {
                    format!("purlin-{side_name}-{position_name}")
                };
                let role = if position_name == "ridge" {
                    ElementRole::Ridge
                } else {
                    ElementRole::Purlin
                };
                let origin_at_t = add(outer_eave, scale(slope, t));
                let solid = OrientedBox {
                    origin: sub(
                        sub(origin_at_t, scale(slope, purlin_width * 0.5)),
                        scale(
                            normal,
                            covering_depth + boarding_depth + common_depth + purlin_depth,
                        ),
                    ),
                    axes: [x_axis, slope, normal],
                    size: [bay_length, purlin_width, purlin_depth],
                };
                let element =
                    builder.element(key.clone(), role, solid, EvidenceClass::RegionalAnalogy, 2);
                let from = builder.node(
                    format!("node-{key}-west"),
                    add(
                        [x0, 0.0, 0.0],
                        sub(
                            origin_at_t,
                            scale(
                                normal,
                                covering_depth + boarding_depth + common_depth + purlin_depth * 0.5,
                            ),
                        ),
                    ),
                );
                let to = builder.node(
                    format!("node-{key}-east"),
                    add(builder.nodes[from.0].point, [bay_length, 0.0, 0.0]),
                );
                builder.member(key, element, from, to, EvidenceClass::RegionalAnalogy);
                for (ordinal, &(common, x)) in common_rafters.iter().enumerate() {
                    builder.bearing(
                        format!(
                            "bearing-common-{side_name}-{ordinal:02}-on-{position_name}-purlin"
                        ),
                        Anchor {
                            element: common,
                            local: [t, common_width * 0.5, 0.0],
                        },
                        Anchor {
                            element,
                            local: [x - x0, purlin_width * 0.5, purlin_depth],
                        },
                        normal,
                        [x_axis, slope],
                        [common_width * 0.95, purlin_width * 0.95],
                        BearingKind::CrossedSeat,
                        EvidenceClass::RegionalAnalogy,
                    );
                    builder.transfer(
                        format!("load-common-{side_name}-{ordinal:02}-to-{position_name}-purlin"),
                        common,
                        TransferTarget::Element(element),
                        TransferKind::Bearing,
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
                masonry,
                wall_plate,
                wall_plate_member,
                purlins,
            });
        }

        for (station_name, station_x) in [("west", x0), ("east", x1)] {
            let south_heel = builder.node(
                format!("node-truss-{station_name}-heel-south"),
                [station_x, -half_nave, wall_top + wall_plate_height],
            );
            let north_heel = builder.node(
                format!("node-truss-{station_name}-heel-north"),
                [station_x, half_nave, wall_top + wall_plate_height],
            );
            let ridge_inner = add(
                side_data[0].outer_eave,
                add(
                    scale(side_data[0].slope, roof_length),
                    scale(side_data[0].normal, -stack_depth),
                ),
            );
            let apex = builder.node(
                format!("node-truss-{station_name}-apex"),
                [station_x, 0.0, ridge_inner[2]],
            );
            let tie_center = builder.node(
                format!("node-truss-{station_name}-tie-center"),
                [station_x, 0.0, wall_top + 0.30],
            );

            let tie = builder.element(
                format!("tie-beam-{station_name}"),
                ElementRole::TieBeam,
                OrientedBox {
                    origin: [station_x - 0.15, -half_nave, wall_top],
                    axes: [[0.0, 1.0, 0.0], x_axis, z_axis],
                    size: [params.nave_width, 0.30, 0.30],
                },
                EvidenceClass::ModernEngineeringInference,
                2,
            );
            let tie_member = builder.member(
                format!("tie-beam-{station_name}"),
                tie,
                south_heel,
                north_heel,
                EvidenceClass::ModernEngineeringInference,
            );
            for data in &side_data {
                let local_y = if data.side < 0.0 {
                    0.0
                } else {
                    params.nave_width
                };
                builder.bearing(
                    format!("bearing-tie-{station_name}-on-{}-masonry", data.name),
                    Anchor {
                        element: tie,
                        local: [local_y, 0.15, 0.0],
                    },
                    Anchor {
                        element: data.masonry,
                        local: [station_x - x0, masonry_width * 0.5, wall_top],
                    },
                    z_axis,
                    [x_axis, [0.0, 1.0, 0.0]],
                    [0.14, 0.20],
                    BearingKind::WallHead,
                    EvidenceClass::ModernEngineeringInference,
                );
                builder.transfer(
                    format!("load-tie-{station_name}-to-{}-masonry", data.name),
                    tie,
                    TransferTarget::Element(data.masonry),
                    TransferKind::Bearing,
                );
            }

            let mut station_principals = Vec::new();
            for data in &side_data {
                let key = format!("principal-rafter-{}-{station_name}", data.name);
                let solid = OrientedBox {
                    origin: add(
                        [station_x - principal_width * 0.5, 0.0, 0.0],
                        sub(data.outer_eave, scale(data.normal, stack_depth)),
                    ),
                    axes: [data.slope, x_axis, data.normal],
                    size: [roof_length, principal_width, principal_depth],
                };
                let principal = builder.element(
                    key.clone(),
                    ElementRole::PrincipalRafter,
                    solid,
                    EvidenceClass::ModernEngineeringInference,
                    1,
                );
                let member = builder.member(
                    key,
                    principal,
                    if data.side < 0.0 {
                        south_heel
                    } else {
                        north_heel
                    },
                    apex,
                    EvidenceClass::ModernEngineeringInference,
                );
                builder.bearing(
                    format!(
                        "bearing-principal-{}-{station_name}-on-wall-plate",
                        data.name
                    ),
                    Anchor {
                        element: principal,
                        local: [0.0, principal_width * 0.5, 0.0],
                    },
                    Anchor {
                        element: data.wall_plate,
                        local: [station_x - x0, wall_plate_width * 0.5, wall_plate_height],
                    },
                    data.normal,
                    [x_axis, data.slope],
                    [principal_width * 0.45, 0.12],
                    BearingKind::AnchorContact,
                    EvidenceClass::ModernEngineeringInference,
                );
                builder.transfer(
                    format!("load-principal-{}-{station_name}-to-wall-plate", data.name),
                    principal,
                    TransferTarget::Element(data.wall_plate),
                    TransferKind::Bearing,
                );
                for &(position_name, t, purlin) in &data.purlins {
                    builder.bearing(
                        format!(
                            "bearing-{position_name}-purlin-{}-on-principal-{station_name}",
                            data.name
                        ),
                        Anchor {
                            element: purlin,
                            local: [station_x - x0, purlin_width * 0.5, 0.0],
                        },
                        Anchor {
                            element: principal,
                            local: [t, principal_width * 0.5, principal_depth],
                        },
                        data.normal,
                        [x_axis, data.slope],
                        [principal_width * 0.45, purlin_width * 0.90],
                        BearingKind::CrossedSeat,
                        EvidenceClass::ModernEngineeringInference,
                    );
                    builder.transfer(
                        format!(
                            "load-{position_name}-purlin-{}-to-principal-{station_name}",
                            data.name
                        ),
                        purlin,
                        TransferTarget::Element(principal),
                        TransferKind::Bearing,
                    );
                }
                station_principals.push((data.name, member, principal));
            }

            let king_base = [station_x, 0.0, wall_top + 0.30];
            let king_top = [station_x, 0.0, ridge_inner[2]];
            let king = builder.element(
                format!("king-post-{station_name}"),
                ElementRole::KingPost,
                beam_between(king_base, king_top, 0.26, 0.26),
                EvidenceClass::ModernEngineeringInference,
                1,
            );
            let king_member = builder.member(
                format!("king-post-{station_name}"),
                king,
                tie_center,
                apex,
                EvidenceClass::ModernEngineeringInference,
            );
            builder.transfer(
                format!("load-king-post-{station_name}-to-tie"),
                king,
                TransferTarget::Element(tie),
                TransferKind::Joint,
            );

            let mut strut_members = Vec::new();
            for data in &side_data {
                let target = add(
                    add(data.outer_eave, scale(data.slope, roof_length * 0.58)),
                    scale(data.normal, -stack_depth),
                );
                let foot = king_base;
                let target = [station_x, target[1], target[2]];
                let foot_node = builder.node(
                    format!("node-strut-{}-{station_name}-foot", data.name),
                    foot,
                );
                let target_node = builder.node(
                    format!("node-strut-{}-{station_name}-target", data.name),
                    target,
                );
                let strut = builder.element(
                    format!("strut-{}-{station_name}", data.name),
                    ElementRole::Strut,
                    beam_between(foot, target, 0.18, 0.16),
                    EvidenceClass::ModernEngineeringInference,
                    1,
                );
                let member = builder.member(
                    format!("strut-{}-{station_name}", data.name),
                    strut,
                    foot_node,
                    target_node,
                    EvidenceClass::ModernEngineeringInference,
                );
                builder.transfer(
                    format!("load-strut-{}-{station_name}-to-tie", data.name),
                    strut,
                    TransferTarget::Element(tie),
                    TransferKind::Joint,
                );
                let principal_member = station_principals
                    .iter()
                    .find(|(side_name, _, _)| *side_name == data.name)
                    .map(|(_, member, _)| *member)
                    .expect("station principal exists");
                builder.joint(
                    format!("joint-strut-to-principal-{}-{station_name}", data.name),
                    target_node,
                    vec![member, principal_member],
                    JointKind::HousedMortiseTenon,
                    EvidenceClass::ModernEngineeringInference,
                );
                strut_members.push(member);
            }

            builder.joint(
                format!("joint-apex-{station_name}"),
                apex,
                vec![
                    station_principals[0].1,
                    station_principals[1].1,
                    king_member,
                ],
                JointKind::MortiseTenon,
                EvidenceClass::ModernEngineeringInference,
            );
            builder.joint(
                format!("joint-king-post-to-tie-{station_name}"),
                tie_center,
                vec![tie_member, king_member, strut_members[0], strut_members[1]],
                JointKind::ThroughTenon,
                EvidenceClass::ModernEngineeringInference,
            );
            for (data, heel, principal) in [
                (&side_data[0], south_heel, station_principals[0].1),
                (&side_data[1], north_heel, station_principals[1].1),
            ] {
                builder.joint(
                    format!("joint-heel-{}-{station_name}", data.name),
                    heel,
                    vec![tie_member, principal, data.wall_plate_member],
                    JointKind::BearingSeat,
                    EvidenceClass::ModernEngineeringInference,
                );
            }
        }

        for data in &side_data {
            builder.bearing(
                format!("bearing-wall-plate-{}-on-masonry", data.name),
                Anchor {
                    element: data.wall_plate,
                    local: [bay_length * 0.5, wall_plate_width * 0.5, 0.0],
                },
                Anchor {
                    element: data.masonry,
                    local: [bay_length * 0.5, masonry_width * 0.5, wall_top],
                },
                z_axis,
                [x_axis, [0.0, 1.0, 0.0]],
                [bay_length * 0.95, wall_plate_width * 0.90],
                BearingKind::WallHead,
                EvidenceClass::RegionalAnalogy,
            );
            builder.transfer(
                format!("load-wall-plate-{}-to-masonry", data.name),
                data.wall_plate,
                TransferTarget::Element(data.masonry),
                TransferKind::Bearing,
            );
            let support = builder.support(
                format!("support-ground-{}-wall", data.name),
                data.masonry,
                "ground",
                Restraints {
                    translation: [true, true, true],
                },
            );
            builder.transfer(
                format!("load-masonry-{}-to-ground", data.name),
                data.masonry,
                TransferTarget::Support(support),
                TransferKind::Ground,
            );
        }

        builder.finish()
    }

    pub(crate) fn validate(&self) -> ValidationReport {
        let mut report = ValidationReport::default();
        validate_unique_keys(
            &mut report,
            "evidence",
            self.evidence_sources.iter().map(|value| value.key.as_str()),
        );
        validate_unique_keys(
            &mut report,
            "node",
            self.nodes.iter().map(|value| value.key.as_str()),
        );
        validate_unique_keys(
            &mut report,
            "element",
            self.elements.iter().map(|value| value.key.as_str()),
        );
        validate_unique_keys(
            &mut report,
            "member",
            self.members.iter().map(|value| value.key.as_str()),
        );
        validate_unique_keys(
            &mut report,
            "joint",
            self.joints.iter().map(|value| value.key.as_str()),
        );
        validate_unique_keys(
            &mut report,
            "bearing",
            self.bearings.iter().map(|value| value.key.as_str()),
        );
        validate_unique_keys(
            &mut report,
            "support",
            self.supports.iter().map(|value| value.key.as_str()),
        );
        validate_unique_keys(
            &mut report,
            "transfer",
            self.transfers.iter().map(|value| value.key.as_str()),
        );

        for source in &self.evidence_sources {
            if source.url.is_empty() || source.note.is_empty() {
                issue(
                    &mut report,
                    "invalid-evidence",
                    &source.key,
                    format!("{} evidence requires a URL and note", source.class),
                );
            }
        }
        for (node_index, node) in self.nodes.iter().enumerate() {
            if !finite(node.point) {
                issue(
                    &mut report,
                    "non-finite-node",
                    &node.key,
                    "node position must be finite".to_owned(),
                );
            }
            let id = NodeId(node_index);
            if !self
                .members
                .iter()
                .any(|member| member.from == id || member.to == id)
                && !self.joints.iter().any(|joint| joint.node == id)
            {
                issue(
                    &mut report,
                    "orphan-node",
                    &node.key,
                    "node is not referenced by a member or joint".to_owned(),
                );
            }
        }
        for element in &self.elements {
            if !finite(element.solid.origin)
                || !finite(element.solid.size)
                || element.solid.size.iter().any(|value| *value <= 0.0)
                || element.solid.axes.iter().any(|axis| !is_unit(*axis))
                || !orthogonal_frame(element.solid.axes)
            {
                issue(
                    &mut report,
                    "invalid-element-geometry",
                    &element.key,
                    format!(
                        "{} geometry must have finite positive extents and unit axes ({})",
                        element.role.label(),
                        element.evidence
                    ),
                );
            }
            self.validate_evidence_link(
                &mut report,
                &element.key,
                element.evidence,
                &element.evidence_source,
            );
        }
        for (element_index, element) in self.elements.iter().enumerate() {
            let member_count = self
                .members
                .iter()
                .filter(|member| member.element == ElementId(element_index))
                .count();
            if element.present && element.role.requires_member() && member_count == 0 {
                issue(
                    &mut report,
                    "orphan-element",
                    &element.key,
                    "structural element has no centerline member".to_owned(),
                );
            }
            if member_count > 1 {
                issue(
                    &mut report,
                    "duplicate-element-member",
                    &element.key,
                    format!("element is owned by {member_count} members"),
                );
            }
        }
        for member in &self.members {
            let Some(element) = self.elements.get(member.element.0) else {
                issue(
                    &mut report,
                    "dangling-member-reference",
                    &member.key,
                    "member references an invalid element".to_owned(),
                );
                continue;
            };
            let (Some(from), Some(to)) =
                (self.nodes.get(member.from.0), self.nodes.get(member.to.0))
            else {
                issue(
                    &mut report,
                    "dangling-member-reference",
                    &member.key,
                    "member references an invalid endpoint node".to_owned(),
                );
                continue;
            };
            if !element.present {
                issue(
                    &mut report,
                    "member-on-omitted-element",
                    &member.key,
                    format!("member references omitted element {}", element.key),
                );
            }
            if !element.role.requires_member() || member.key != element.key {
                issue(
                    &mut report,
                    "orphan-member",
                    &member.key,
                    format!(
                        "member does not uniquely name a member-bearing element {}",
                        element.key
                    ),
                );
            }
            if member.from == member.to || norm(sub(from.point, to.point)) <= CONTACT_TOLERANCE {
                issue(
                    &mut report,
                    "coincident-member-endpoints",
                    &member.key,
                    "member endpoints must be distinct".to_owned(),
                );
            }
            for (label, node) in [("from", from), ("to", to)] {
                if !element.solid.contains_point(node.point, CONTACT_TOLERANCE) {
                    issue(
                        &mut report,
                        "member-endpoint-outside-solid",
                        &member.key,
                        format!(
                            "{label} node {} lies outside emitted solid {}",
                            node.key, element.key
                        ),
                    );
                }
            }
            self.validate_evidence_link(
                &mut report,
                &member.key,
                member.evidence,
                &member.evidence_source,
            );
        }
        for joint in &self.joints {
            let Some(node) = self.nodes.get(joint.node.0) else {
                issue(
                    &mut report,
                    "dangling-joint-reference",
                    &joint.key,
                    "joint references an invalid node".to_owned(),
                );
                continue;
            };
            if joint.members.len() < 2 {
                issue(
                    &mut report,
                    "orphan-joint",
                    &joint.key,
                    "joint must connect at least two members".to_owned(),
                );
            }
            let mut unique_members = HashSet::new();
            for member_id in &joint.members {
                if !unique_members.insert(*member_id) {
                    issue(
                        &mut report,
                        "duplicate-joint-member",
                        &joint.key,
                        format!("member ID {} occurs more than once", member_id.0),
                    );
                }
                let Some(member) = self.members.get(member_id.0) else {
                    issue(
                        &mut report,
                        "dangling-joint-reference",
                        &joint.key,
                        format!("joint references invalid member ID {}", member_id.0),
                    );
                    continue;
                };
                let Some(element) = self.elements.get(member.element.0) else {
                    continue;
                };
                if !element.solid.contains_point(node.point, CONTACT_TOLERANCE) {
                    issue(
                        &mut report,
                        "joint-not-incident-to-member",
                        &joint.key,
                        format!("node {} is outside member solid {}", node.key, member.key),
                    );
                }
            }
            self.validate_evidence_link(
                &mut report,
                &joint.key,
                joint.evidence,
                &joint.evidence_source,
            );
        }
        for bearing in &self.bearings {
            let Some(carried) = self.elements.get(bearing.carried.element.0) else {
                issue(
                    &mut report,
                    "dangling-bearing-reference",
                    &bearing.key,
                    "carried element does not exist".to_owned(),
                );
                continue;
            };
            let Some(carrier) = self.elements.get(bearing.carrier.element.0) else {
                issue(
                    &mut report,
                    "dangling-bearing-reference",
                    &bearing.key,
                    "carrier element does not exist".to_owned(),
                );
                continue;
            };
            self.validate_evidence_link(
                &mut report,
                &bearing.key,
                bearing.evidence,
                &bearing.evidence_source,
            );
            if bearing.carried.element == bearing.carrier.element {
                issue(
                    &mut report,
                    "self-bearing",
                    &bearing.key,
                    "carried and carrier elements must differ".to_owned(),
                );
            }
            if !valid_bearing_frame(bearing.normal, bearing.tangents) {
                issue(
                    &mut report,
                    "invalid-bearing-frame",
                    &bearing.key,
                    "normal and tangents must form a finite orthonormal frame".to_owned(),
                );
            }
            if !carried
                .solid
                .contains_local(bearing.carried.local, CONTACT_TOLERANCE)
                || !carrier
                    .solid
                    .contains_local(bearing.carrier.local, CONTACT_TOLERANCE)
            {
                issue(
                    &mut report,
                    "bearing-anchor-out-of-bounds",
                    &bearing.key,
                    "bearing anchors must lie inside their referenced local solid bounds"
                        .to_owned(),
                );
            }
            if bearing
                .minimum_overlap
                .iter()
                .any(|minimum| !minimum.is_finite() || *minimum < 0.0)
            {
                issue(
                    &mut report,
                    "invalid-bearing-minimum",
                    &bearing.key,
                    "minimum overlaps must be finite and nonnegative".to_owned(),
                );
            }
            if !carried.present || !carrier.present {
                issue(
                    &mut report,
                    "missing-bearing-element",
                    &bearing.key,
                    format!(
                        "{} bearing references omitted carried or carrier geometry ({})",
                        bearing_kind_label(bearing.kind),
                        bearing.evidence
                    ),
                );
                continue;
            }
            let measurement = self.measure_bearing(bearing);
            if !measurement.anchors_aligned() {
                issue(
                    &mut report,
                    "misaligned-bearing-anchors",
                    &bearing.key,
                    format!(
                        "anchor frame offsets are normal={:.12} m, tangent-0={:.12} m, tangent-1={:.12} m",
                        measurement.gap,
                        measurement.tangent_offsets[0],
                        measurement.tangent_offsets[1]
                    ),
                );
            }
            if measurement.gap > CONTACT_TOLERANCE {
                issue(
                    &mut report,
                    "floating-bearing",
                    &bearing.key,
                    format!("signed gap is {:.12} m", measurement.gap),
                );
            } else if measurement.gap < -CONTACT_TOLERANCE {
                issue(
                    &mut report,
                    "embedded-bearing",
                    &bearing.key,
                    format!("signed penetration is {:.12} m", -measurement.gap),
                );
            }
            for (axis_index, overlap) in measurement.overlap.into_iter().enumerate() {
                if overlap + CONTACT_TOLERANCE < bearing.minimum_overlap[axis_index] {
                    issue(
                        &mut report,
                        "insufficient-bearing-overlap",
                        &bearing.key,
                        format!(
                            "axis {axis_index} overlap {overlap:.6} m is below {:.6} m",
                            bearing.minimum_overlap[axis_index]
                        ),
                    );
                }
            }
        }

        for (support_index, support) in self.supports.iter().enumerate() {
            if self
                .elements
                .get(support.element.0)
                .is_none_or(|element| !element.present)
                || support.ground.is_empty()
            {
                issue(
                    &mut report,
                    "invalid-support",
                    &support.key,
                    "support must reference an element and named ground".to_owned(),
                );
            }
            if !support.restraints.translation.iter().any(|value| *value) {
                issue(
                    &mut report,
                    "unrestrained-support",
                    &support.key,
                    "at least one translational restraint is required".to_owned(),
                );
            }
            let support_id = SupportId(support_index);
            if !self.transfers.iter().any(|transfer| {
                transfer.kind == TransferKind::Ground
                    && transfer.to == TransferTarget::Support(support_id)
            }) {
                issue(
                    &mut report,
                    "orphan-support",
                    &support.key,
                    "support is not targeted by a ground transfer".to_owned(),
                );
            }
        }
        let mut transfer_routes = HashSet::new();
        for transfer in &self.transfers {
            let from_valid = self
                .elements
                .get(transfer.from.0)
                .is_some_and(|element| element.present);
            let to_valid = match transfer.to {
                TransferTarget::Element(id) => self
                    .elements
                    .get(id.0)
                    .is_some_and(|element| element.present),
                TransferTarget::Support(id) => self.supports.get(id.0).is_some(),
            };
            if !from_valid || !to_valid {
                issue(
                    &mut report,
                    "missing-transfer-element",
                    &transfer.key,
                    format!(
                        "{:?} transfer references omitted or invalid graph state",
                        transfer.kind
                    ),
                );
            }
            if !transfer_routes.insert((transfer.from, transfer.to, transfer.kind)) {
                issue(
                    &mut report,
                    "duplicate-transfer-route",
                    &transfer.key,
                    "duplicate transfers do not add structural support multiplicity".to_owned(),
                );
            }
            if from_valid && to_valid && !self.transfer_is_witnessed(transfer) {
                issue(
                    &mut report,
                    transfer.kind.unwitnessed_code(),
                    &transfer.key,
                    format!(
                        "{:?} transfer has no matching physical graph witness",
                        transfer.kind
                    ),
                );
            }
        }

        for (index, element) in self.elements.iter().enumerate() {
            if !element.present {
                continue;
            }
            let support_targets: HashSet<TransferTarget> = self
                .transfers
                .iter()
                .filter(|transfer| transfer.from == ElementId(index))
                .filter(|transfer| self.transfer_is_witnessed(transfer))
                .map(|transfer| transfer.to)
                .collect();
            let support_count = support_targets.len();
            if support_count < element.required_supports {
                issue(
                    &mut report,
                    "insufficient-direct-supports",
                    &element.key,
                    format!(
                        "requires {} direct supports but has {support_count}",
                        element.required_supports
                    ),
                );
            }
            if self.trace_to_ground(ElementId(index)).is_none() {
                issue(
                    &mut report,
                    "no-ground-path",
                    &element.key,
                    "no directed load-transfer route reaches ground".to_owned(),
                );
            }
        }
        self.validate_acyclic(&mut report);
        report
    }

    fn validate_evidence_link(
        &self,
        report: &mut ValidationReport,
        record_key: &str,
        class: EvidenceClass,
        source_key: &str,
    ) {
        match self
            .evidence_sources
            .iter()
            .find(|source| source.key == source_key)
        {
            Some(source) if source.class == class => {}
            Some(source) => issue(
                report,
                "evidence-class-mismatch",
                record_key,
                format!(
                    "record class {class} does not match source {} class {}",
                    source.key, source.class
                ),
            ),
            None => issue(
                report,
                "missing-evidence-source",
                record_key,
                format!("evidence source {source_key:?} does not exist"),
            ),
        }
    }

    fn measure_bearing(&self, bearing: &Bearing) -> BearingMeasurement {
        let carried = &self.elements[bearing.carried.element.0];
        let carrier = &self.elements[bearing.carrier.element.0];
        let carried_point = carried.solid.anchor(bearing.carried.local);
        let carrier_point = carrier.solid.anchor(bearing.carrier.local);
        let anchor_delta = sub(carried_point, carrier_point);
        let overlap = bearing.tangents.map(|tangent| {
            interval_overlap(
                carried.solid.projection_interval(tangent),
                carrier.solid.projection_interval(tangent),
            )
        });
        BearingMeasurement {
            carried_point,
            carrier_point,
            gap: dot(anchor_delta, bearing.normal),
            tangent_offsets: bearing.tangents.map(|tangent| dot(anchor_delta, tangent)),
            overlap,
        }
    }

    fn transfer_is_witnessed(&self, transfer: &LoadTransfer) -> bool {
        let Some(from) = self.elements.get(transfer.from.0) else {
            return false;
        };
        if !from.present {
            return false;
        }
        match (transfer.kind, transfer.to) {
            (TransferKind::Bearing, TransferTarget::Element(to)) => {
                self.bearings.iter().any(|bearing| {
                    bearing.carried.element == transfer.from
                        && bearing.carrier.element == to
                        && self.bearing_is_valid_witness(bearing)
                })
            }
            (TransferKind::Joint, TransferTarget::Element(to)) => {
                let from_members: HashSet<MemberId> = self
                    .members
                    .iter()
                    .enumerate()
                    .filter_map(|(index, member)| {
                        (member.element == transfer.from).then_some(MemberId(index))
                    })
                    .collect();
                let to_members: HashSet<MemberId> = self
                    .members
                    .iter()
                    .enumerate()
                    .filter_map(|(index, member)| (member.element == to).then_some(MemberId(index)))
                    .collect();
                !from_members.is_empty()
                    && !to_members.is_empty()
                    && self.joints.iter().any(|joint| {
                        let Some(node) = self.nodes.get(joint.node.0) else {
                            return false;
                        };
                        let incident = |member_id: &MemberId| {
                            self.members
                                .get(member_id.0)
                                .and_then(|member| self.elements.get(member.element.0))
                                .is_some_and(|element| {
                                    element.present
                                        && element
                                            .solid
                                            .contains_point(node.point, CONTACT_TOLERANCE)
                                })
                        };
                        joint
                            .members
                            .iter()
                            .any(|id| from_members.contains(id) && incident(id))
                            && joint
                                .members
                                .iter()
                                .any(|id| to_members.contains(id) && incident(id))
                    })
            }
            (TransferKind::Ground, TransferTarget::Support(support)) => {
                self.supports.get(support.0).is_some_and(|candidate| {
                    candidate.element == transfer.from
                        && !candidate.ground.is_empty()
                        && candidate.restraints.translation.iter().any(|value| *value)
                })
            }
            _ => false,
        }
    }

    fn bearing_is_valid_witness(&self, bearing: &Bearing) -> bool {
        let (Some(carried), Some(carrier)) = (
            self.elements.get(bearing.carried.element.0),
            self.elements.get(bearing.carrier.element.0),
        ) else {
            return false;
        };
        if !carried.present
            || !carrier.present
            || bearing.carried.element == bearing.carrier.element
            || !valid_bearing_frame(bearing.normal, bearing.tangents)
            || !carried
                .solid
                .contains_local(bearing.carried.local, CONTACT_TOLERANCE)
            || !carrier
                .solid
                .contains_local(bearing.carrier.local, CONTACT_TOLERANCE)
            || bearing
                .minimum_overlap
                .iter()
                .any(|minimum| !minimum.is_finite() || *minimum < 0.0)
        {
            return false;
        }
        let measurement = self.measure_bearing(bearing);
        measurement.anchors_aligned()
            && measurement
                .overlap
                .iter()
                .zip(bearing.minimum_overlap.iter())
                .all(|(overlap, minimum)| overlap + CONTACT_TOLERANCE >= *minimum)
    }

    pub(crate) fn explain(&self, selector: &str) -> Result<Option<String>, String> {
        let Some((category, key)) = selector.split_once(':') else {
            let categories = self.matching_categories(selector);
            return match categories.as_slice() {
                [] => Ok(None),
                [category] => self.explain(&format!("{category}:{selector}")),
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
            "element" => self
                .elements
                .iter()
                .enumerate()
                .find(|(_, element)| element.key == key)
                .map(|(index, element)| {
                    let path = self.trace_to_ground(ElementId(index));
                    let mut out = format!(
                        "element {} role={} evidence={} source={} required_direct_supports={} present={}",
                        element.key,
                        element.role.label(),
                        element.evidence,
                        element.evidence_source,
                        element.required_supports,
                        element.present
                    );
                    if let Some(path) = path {
                        let _ = write!(out, "\nload path: {}", path.join(" -> "));
                    } else {
                        out.push_str("\nload path: none");
                    }
                    out
                }),
            "node" => self
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.key == key)
                .map(|(index, node)| {
                    let id = NodeId(index);
                    let members: Vec<&str> = self
                        .members
                        .iter()
                        .filter(|member| member.from == id || member.to == id)
                        .map(|member| member.key.as_str())
                        .collect();
                    format!(
                        "node {} point={:?} members={}",
                        node.key,
                        node.point,
                        members.join(",")
                    )
                }),
            "member" => self.members.iter().find(|member| member.key == key).map(|member| {
                format!(
                    "member {} element={} from={} to={} evidence={} source={}",
                    member.key,
                    self.elements[member.element.0].key,
                    self.nodes[member.from.0].key,
                    self.nodes[member.to.0].key,
                    member.evidence,
                    member.evidence_source
                )
            }),
            "joint" => self.joints.iter().find(|joint| joint.key == key).map(|joint| {
                let members: Vec<&str> = joint
                    .members
                    .iter()
                    .map(|id| self.members[id.0].key.as_str())
                    .collect();
                format!(
                    "joint {} kind={:?} geometry=uncut-connectivity-hypothesis node={} members={} evidence={} source={}",
                    joint.key,
                    joint.kind,
                    self.nodes[joint.node.0].key,
                    members.join(","),
                    joint.evidence,
                    joint.evidence_source
                )
            }),
            "bearing" => self
                .bearings
                .iter()
                .find(|bearing| bearing.key == key)
                .map(|bearing| {
                    let measurement = self.measure_bearing(bearing);
                    format!(
                        "bearing {} kind={} geometry=analytic-anchor-only carried={} carrier={} evidence={} source={}\ncarried_anchor={:?} carrier_anchor={:?} normal={:?} tangents={:?}\ngap={:.12}m tangent_offsets={:?} tolerance={:.12}m overlaps={:?} minima={:?}",
                        bearing.key,
                        bearing_kind_label(bearing.kind),
                        self.elements[bearing.carried.element.0].key,
                        self.elements[bearing.carrier.element.0].key,
                        bearing.evidence,
                        bearing.evidence_source,
                        measurement.carried_point,
                        measurement.carrier_point,
                        bearing.normal,
                        bearing.tangents,
                        measurement.gap,
                        measurement.tangent_offsets,
                        CONTACT_TOLERANCE,
                        measurement.overlap,
                        bearing.minimum_overlap
                    )
                }),
            "support" => self
                .supports
                .iter()
                .find(|support| support.key == key)
                .map(|support| {
                    format!(
                        "support {} element={} ground={} restraints={:?}",
                        support.key,
                        self.elements[support.element.0].key,
                        support.ground,
                        support.restraints.translation
                    )
                }),
            "transfer" => self
                .transfers
                .iter()
                .find(|transfer| transfer.key == key)
                .map(|transfer| {
                    let target = match transfer.to {
                        TransferTarget::Element(id) => self.elements[id.0].key.as_str(),
                        TransferTarget::Support(id) => self.supports[id.0].key.as_str(),
                    };
                    format!(
                        "transfer {} kind={:?} from={} to={target} witnessed={}",
                        transfer.key,
                        transfer.kind,
                        self.elements[transfer.from.0].key,
                        self.transfer_is_witnessed(transfer)
                    )
                }),
            "evidence" => self
                .evidence_sources
                .iter()
                .find(|source| source.key == key)
                .map(|source| {
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

    fn matching_categories(&self, key: &str) -> Vec<&'static str> {
        let mut categories = Vec::new();
        if self.elements.iter().any(|value| value.key == key) {
            categories.push("element");
        }
        if self.nodes.iter().any(|value| value.key == key) {
            categories.push("node");
        }
        if self.members.iter().any(|value| value.key == key) {
            categories.push("member");
        }
        if self.joints.iter().any(|value| value.key == key) {
            categories.push("joint");
        }
        if self.bearings.iter().any(|value| value.key == key) {
            categories.push("bearing");
        }
        if self.supports.iter().any(|value| value.key == key) {
            categories.push("support");
        }
        if self.transfers.iter().any(|value| value.key == key) {
            categories.push("transfer");
        }
        if self.evidence_sources.iter().any(|value| value.key == key) {
            categories.push("evidence");
        }
        categories
    }

    pub(crate) fn stats_line(&self) -> String {
        format!(
            "nodes={} elements={} members={} joints={} bearings={} supports={} transfers={} evidence_sources={} signature={:016x}",
            self.nodes.len(),
            self.elements
                .iter()
                .filter(|element| element.present)
                .count(),
            self.members.len(),
            self.joints.len(),
            self.bearings.len(),
            self.supports.len(),
            self.transfers.len(),
            self.evidence_sources.len(),
            self.deterministic_signature()
        )
    }

    pub(crate) fn deterministic_signature(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in format!("{self:?}").bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    fn trace_to_ground(&self, start: ElementId) -> Option<Vec<String>> {
        fn visit(
            model: &StructuralModel,
            current: ElementId,
            visiting: &mut HashSet<ElementId>,
        ) -> Option<Vec<String>> {
            if !visiting.insert(current) {
                return None;
            }
            for transfer in model
                .transfers
                .iter()
                .filter(|edge| edge.from == current && model.transfer_is_witnessed(edge))
            {
                match transfer.to {
                    TransferTarget::Support(id) => {
                        if let Some(support) = model.supports.get(id.0) {
                            let mut path = vec![model.elements[current.0].key.clone()];
                            path.push(support.key.clone());
                            path.push(support.ground.clone());
                            visiting.remove(&current);
                            return Some(path);
                        }
                    }
                    TransferTarget::Element(next) => {
                        if model
                            .elements
                            .get(next.0)
                            .is_some_and(|element| element.present)
                            && let Some(mut tail) = visit(model, next, visiting)
                        {
                            let mut path = vec![model.elements[current.0].key.clone()];
                            path.append(&mut tail);
                            visiting.remove(&current);
                            return Some(path);
                        }
                    }
                }
            }
            visiting.remove(&current);
            None
        }

        visit(self, start, &mut HashSet::new())
    }

    fn validate_acyclic(&self, report: &mut ValidationReport) {
        fn visit(
            model: &StructuralModel,
            current: ElementId,
            temporary: &mut HashSet<ElementId>,
            permanent: &mut HashSet<ElementId>,
        ) -> Option<ElementId> {
            if permanent.contains(&current) {
                return None;
            }
            if !temporary.insert(current) {
                return Some(current);
            }
            for next in model.transfers.iter().filter_map(|edge| {
                (edge.from == current)
                    .then_some(edge.to)
                    .and_then(|target| match target {
                        TransferTarget::Element(id)
                            if model
                                .elements
                                .get(id.0)
                                .is_some_and(|element| element.present) =>
                        {
                            Some(id)
                        }
                        TransferTarget::Support(_) => None,
                        TransferTarget::Element(_) => None,
                    })
            }) {
                if let Some(cycle) = visit(model, next, temporary, permanent) {
                    return Some(cycle);
                }
            }
            temporary.remove(&current);
            permanent.insert(current);
            None
        }

        let mut temporary = HashSet::new();
        let mut permanent = HashSet::new();
        for index in 0..self.elements.len() {
            if let Some(cycle) = visit(self, ElementId(index), &mut temporary, &mut permanent) {
                let cycle_key = self.elements.get(cycle.0).map_or_else(
                    || format!("element-id-{}", cycle.0),
                    |element| element.key.clone(),
                );
                issue(
                    report,
                    "load-transfer-cycle",
                    &cycle_key,
                    "directed load transfers must be acyclic".to_owned(),
                );
                break;
            }
        }
    }

    #[cfg(test)]
    fn translate_element(&mut self, key: &str, delta: Vec3) {
        self.elements
            .iter_mut()
            .find(|element| element.key == key)
            .expect("test element exists")
            .solid
            .translate(delta);
    }

    #[cfg(test)]
    fn omit_element(&mut self, key: &str) {
        self.elements
            .iter_mut()
            .find(|element| element.key == key)
            .expect("test element exists")
            .present = false;
    }
}

#[derive(Debug)]
struct SideData<'a> {
    name: &'a str,
    side: f64,
    slope: Vec3,
    normal: Vec3,
    outer_eave: Vec3,
    masonry: ElementId,
    wall_plate: ElementId,
    wall_plate_member: MemberId,
    purlins: Vec<(&'a str, f64, ElementId)>,
}

#[derive(Debug, Default)]
struct ModelBuilder {
    evidence_sources: Vec<EvidenceSource>,
    nodes: Vec<Node>,
    elements: Vec<Element>,
    members: Vec<Member>,
    joints: Vec<Joint>,
    bearings: Vec<Bearing>,
    supports: Vec<Support>,
    transfers: Vec<LoadTransfer>,
}

impl ModelBuilder {
    fn add_evidence_sources(&mut self) {
        self.evidence_sources.extend([
            EvidenceSource {
                key: "saint-catherine-sixth-century-roof".to_owned(),
                class: EvidenceClass::Observed,
                url: "https://lsa.umich.edu/kelsey/research/past-field-projects/monastery-st-catharine.html".to_owned(),
                note: "Observed timber nave roof dated 548-565; not a complete joint specification".to_owned(),
            },
            EvidenceSource {
                key: "san-paolo-double-truss".to_owned(),
                class: EvidenceClass::DocumentedReconstruction,
                url: "https://www.witpress.com/elibrary/wit-transactions-on-the-built-environment/191/37497".to_owned(),
                note: "Pre-1823 records support analysis of the lost Early Christian double truss".to_owned(),
            },
            EvidenceSource {
                key: "hagia-paraskevi-roof-survey".to_owned(),
                class: EvidenceClass::RegionalAnalogy,
                url: "https://hdl.handle.net/11583/1956728".to_owned(),
                note: "Later eastern-Mediterranean truss, secondary timber, boarding, and tile evidence".to_owned(),
            },
            EvidenceSource {
                key: "tfec-modern-truss-detailing".to_owned(),
                class: EvidenceClass::ModernEngineeringInference,
                url: "https://timberframehq.com/wp-content/uploads/2021/12/TFEC-DG-1-2021.pdf".to_owned(),
                note: "Modern connection behavior and detailing vocabulary, not Byzantine evidence".to_owned(),
            },
        ]);
    }

    fn node(&mut self, key: String, point: Vec3) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node { key, point });
        id
    }

    fn element(
        &mut self,
        key: String,
        role: ElementRole,
        solid: OrientedBox,
        evidence: EvidenceClass,
        required_supports: usize,
    ) -> ElementId {
        let id = ElementId(self.elements.len());
        self.elements.push(Element {
            key,
            role,
            solid,
            evidence,
            evidence_source: evidence_source_key(evidence).to_owned(),
            required_supports,
            present: true,
        });
        id
    }

    fn member(
        &mut self,
        key: String,
        element: ElementId,
        from: NodeId,
        to: NodeId,
        evidence: EvidenceClass,
    ) -> MemberId {
        let id = MemberId(self.members.len());
        self.members.push(Member {
            key,
            element,
            from,
            to,
            evidence,
            evidence_source: evidence_source_key(evidence).to_owned(),
        });
        id
    }

    fn joint(
        &mut self,
        key: String,
        node: NodeId,
        members: Vec<MemberId>,
        kind: JointKind,
        evidence: EvidenceClass,
    ) {
        self.joints.push(Joint {
            key,
            node,
            members,
            kind,
            evidence,
            evidence_source: evidence_source_key(evidence).to_owned(),
        });
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a bearing's explicit proof data is one record"
    )]
    fn bearing(
        &mut self,
        key: String,
        carried: Anchor,
        carrier: Anchor,
        normal: Vec3,
        tangents: [Vec3; 2],
        minimum_overlap: [f64; 2],
        kind: BearingKind,
        evidence: EvidenceClass,
    ) {
        self.bearings.push(Bearing {
            key,
            carried,
            carrier,
            normal,
            tangents,
            minimum_overlap,
            kind,
            evidence,
            evidence_source: evidence_source_key(evidence).to_owned(),
        });
    }

    fn support(
        &mut self,
        key: String,
        element: ElementId,
        ground: &str,
        restraints: Restraints,
    ) -> SupportId {
        let id = SupportId(self.supports.len());
        self.supports.push(Support {
            key,
            element,
            ground: ground.to_owned(),
            restraints,
        });
        id
    }

    fn transfer(&mut self, key: String, from: ElementId, to: TransferTarget, kind: TransferKind) {
        self.transfers.push(LoadTransfer {
            key,
            from,
            to,
            kind,
        });
    }

    fn finish(self) -> StructuralModel {
        StructuralModel {
            evidence_sources: self.evidence_sources,
            nodes: self.nodes,
            elements: self.elements,
            members: self.members,
            joints: self.joints,
            bearings: self.bearings,
            supports: self.supports,
            transfers: self.transfers,
        }
    }
}

fn beam_between(from: Vec3, to: Vec3, width: f64, depth: f64) -> OrientedBox {
    let delta = sub(to, from);
    let length = norm(delta);
    let along = scale(delta, 1.0 / length);
    let across = [1.0, 0.0, 0.0];
    let normal = normalize(cross(along, across));
    OrientedBox {
        origin: sub(
            sub(from, scale(across, width * 0.5)),
            scale(normal, depth * 0.5),
        ),
        axes: [along, across, normal],
        size: [length, width, depth],
    }
}

fn validate_unique_keys<'a>(
    report: &mut ValidationReport,
    category: &str,
    keys: impl Iterator<Item = &'a str>,
) {
    let mut seen = HashSet::new();
    for key in keys {
        if key.is_empty() {
            issue(
                report,
                "empty-key",
                category,
                format!("{category} keys must be non-empty"),
            );
        } else if !seen.insert(key) {
            issue(
                report,
                "duplicate-key",
                key,
                format!("duplicate {category} key"),
            );
        }
    }
}

fn issue(report: &mut ValidationReport, code: &'static str, key: &str, message: String) {
    report.issues.push(ValidationIssue {
        code,
        key: key.to_owned(),
        message,
    });
}

fn bearing_kind_label(kind: BearingKind) -> &'static str {
    match kind {
        BearingKind::Surface => "surface",
        BearingKind::CrossedSeat => "crossed-seat",
        BearingKind::AnchorContact => "anchor-contact",
        BearingKind::WallHead => "wall-head",
    }
}

fn evidence_source_key(class: EvidenceClass) -> &'static str {
    match class {
        EvidenceClass::Observed => "saint-catherine-sixth-century-roof",
        EvidenceClass::DocumentedReconstruction => "san-paolo-double-truss",
        EvidenceClass::RegionalAnalogy => "hagia-paraskevi-roof-survey",
        EvidenceClass::ModernEngineeringInference => "tfec-modern-truss-detailing",
    }
}

fn interval_overlap(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

pub(crate) fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: Vec3, factor: f64) -> Vec3 {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}

pub(crate) fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: Vec3) -> Vec3 {
    scale(a, 1.0 / norm(a))
}

fn finite(a: Vec3) -> bool {
    a.iter().all(|value| value.is_finite())
}

fn is_unit(a: Vec3) -> bool {
    finite(a) && (norm(a) - 1.0).abs() < 1.0e-9
}

fn orthogonal_frame(axes: [Vec3; 3]) -> bool {
    dot(axes[0], axes[1]).abs() <= 1.0e-9
        && dot(axes[0], axes[2]).abs() <= 1.0e-9
        && dot(axes[1], axes[2]).abs() <= 1.0e-9
}

fn valid_bearing_frame(normal: Vec3, tangents: [Vec3; 2]) -> bool {
    is_unit(normal)
        && tangents.iter().copied().all(is_unit)
        && orthogonal_frame([normal, tangents[0], tangents[1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> StructuralModel {
        StructuralModel::western_bay(&BasilicaParams::default())
    }

    fn has_issue(report: &ValidationReport, code: &str, key: &str) -> bool {
        report
            .issues
            .iter()
            .any(|issue| issue.code == code && issue.key == key)
    }

    fn element_id(model: &StructuralModel, key: &str) -> ElementId {
        ElementId(
            model
                .elements
                .iter()
                .position(|element| element.key == key)
                .expect("test element exists"),
        )
    }

    #[test]
    fn canonical_bay_is_clean_and_every_element_reaches_ground() {
        let model = model();
        let report = model.validate();
        assert!(report.is_clean(), "{report}");
        assert_eq!(model.elements.len(), 32);
        assert!(model.elements.iter().all(|element| {
            model
                .trace_to_ground(ElementId(
                    model
                        .elements
                        .iter()
                        .position(|candidate| candidate.key == element.key)
                        .expect("element exists"),
                ))
                .is_some()
        }));
    }

    #[test]
    fn canonical_element_order_and_graph_signature_are_pinned() {
        let model = model();
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
            "ridge-member-south",
            "masonry-north",
            "wall-plate-north",
            "roof-covering-north",
            "boarding-north",
            "common-rafter-north-00",
            "common-rafter-north-01",
            "common-rafter-north-02",
            "purlin-north-eave",
            "purlin-north-mid",
            "ridge-member-north",
            "tie-beam-west",
            "principal-rafter-south-west",
            "principal-rafter-north-west",
            "king-post-west",
            "strut-south-west",
            "strut-north-west",
            "tie-beam-east",
            "principal-rafter-south-east",
            "principal-rafter-north-east",
            "king-post-east",
            "strut-south-east",
            "strut-north-east",
        ];
        assert_eq!(
            model
                .elements
                .iter()
                .map(|element| element.key.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(model.deterministic_signature(), 2_151_536_415_430_394_313);
    }

    #[test]
    fn south_covering_float_and_embed_are_rejected_by_the_same_bearing() {
        let baseline = model();
        let bearing = baseline
            .bearings
            .iter()
            .find(|bearing| bearing.key == "bearing-covering-on-boarding-south")
            .expect("south covering bearing");
        let normal = bearing.normal;

        let mut floating = baseline.clone();
        floating.translate_element("roof-covering-south", scale(normal, 0.05));
        let floating_report = floating.validate();
        assert!(floating_report.issues.iter().any(|issue| {
            issue.code == "floating-bearing" && issue.key == "bearing-covering-on-boarding-south"
        }));

        let mut embedded = baseline;
        embedded.translate_element("roof-covering-south", scale(normal, -0.05));
        let embedded_report = embedded.validate();
        assert!(embedded_report.issues.iter().any(|issue| {
            issue.code == "embedded-bearing" && issue.key == "bearing-covering-on-boarding-south"
        }));
    }

    #[test]
    fn missing_south_mid_purlin_breaks_required_support_coverage() {
        let mut model = model();
        model.omit_element("purlin-south-mid");
        let report = model.validate();
        assert!(report.issues.iter().any(|issue| {
            issue.code == "insufficient-direct-supports" && issue.key == "common-rafter-south-00"
        }));
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "missing-bearing-element")
        );
    }

    #[test]
    fn explanations_use_stable_named_paths() {
        let model = model();
        let explanation = model
            .explain("element:roof-covering-south")
            .expect("valid selector")
            .expect("named element explains");
        assert!(explanation.contains("roof-covering-south -> boarding-south"));
        assert!(explanation.ends_with("support-ground-south-wall -> ground"));
    }

    #[test]
    fn fake_bearing_transfer_cannot_repair_an_omitted_purlin() {
        let mut model = model();
        model.omit_element("purlin-south-mid");
        model.transfers.push(LoadTransfer {
            key: "load-fake-south-common-to-north-purlin".to_owned(),
            from: element_id(&model, "common-rafter-south-00"),
            to: TransferTarget::Element(element_id(&model, "purlin-north-mid")),
            kind: TransferKind::Bearing,
        });
        let report = model.validate();
        assert!(has_issue(
            &report,
            "unwitnessed-bearing-transfer",
            "load-fake-south-common-to-north-purlin"
        ));
        assert!(has_issue(
            &report,
            "insufficient-direct-supports",
            "common-rafter-south-00"
        ));
    }

    #[test]
    fn transfer_kind_cannot_claim_a_different_witness_semantics() {
        let mut model = model();
        let transfer = model
            .transfers
            .iter_mut()
            .find(|transfer| transfer.key == "load-covering-to-boarding-south")
            .expect("covering bearing transfer exists");
        transfer.kind = TransferKind::Joint;
        let report = model.validate();
        assert!(has_issue(
            &report,
            "unwitnessed-joint-transfer",
            "load-covering-to-boarding-south"
        ));
        assert!(has_issue(
            &report,
            "insufficient-direct-supports",
            "roof-covering-south"
        ));
    }

    #[test]
    fn duplicate_transfers_do_not_inflate_direct_support_multiplicity() {
        let mut model = model();
        model.omit_element("purlin-south-mid");
        model.omit_element("ridge-member-south");
        let from = element_id(&model, "common-rafter-south-00");
        let to = element_id(&model, "purlin-south-eave");
        for ordinal in 0..3 {
            model.transfers.push(LoadTransfer {
                key: format!("load-duplicate-common-to-eave-{ordinal}"),
                from,
                to: TransferTarget::Element(to),
                kind: TransferKind::Bearing,
            });
        }
        let report = model.validate();
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "duplicate-transfer-route")
        );
        assert!(has_issue(
            &report,
            "insufficient-direct-supports",
            "common-rafter-south-00"
        ));
    }

    #[test]
    fn ground_transfer_must_target_the_support_for_its_own_element() {
        let mut model = model();
        let north_support = SupportId(
            model
                .supports
                .iter()
                .position(|support| support.key == "support-ground-north-wall")
                .expect("north support exists"),
        );
        let transfer = model
            .transfers
            .iter_mut()
            .find(|transfer| transfer.key == "load-masonry-south-to-ground")
            .expect("south ground transfer exists");
        transfer.to = TransferTarget::Support(north_support);
        let report = model.validate();
        assert!(has_issue(
            &report,
            "unwitnessed-ground-transfer",
            "load-masonry-south-to-ground"
        ));
        assert!(has_issue(
            &report,
            "orphan-support",
            "support-ground-south-wall"
        ));
        assert!(has_issue(&report, "no-ground-path", "masonry-south"));
    }

    #[test]
    fn direct_transfer_cycle_is_rejected() {
        let mut model = model();
        model.transfers.push(LoadTransfer {
            key: "load-cycle-south-masonry-to-covering".to_owned(),
            from: element_id(&model, "masonry-south"),
            to: TransferTarget::Element(element_id(&model, "roof-covering-south")),
            kind: TransferKind::Bearing,
        });
        let report = model.validate();
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "load-transfer-cycle")
        );
    }

    #[test]
    fn member_endpoint_must_remain_inside_its_emitted_solid() {
        let mut model = model();
        let member = model
            .members
            .iter()
            .find(|member| member.key == "wall-plate-south")
            .expect("wall plate member");
        model.nodes[member.from.0].point[2] += 2.0;
        let report = model.validate();
        assert!(has_issue(
            &report,
            "member-endpoint-outside-solid",
            "wall-plate-south"
        ));
    }

    #[test]
    fn member_endpoints_must_be_distinct() {
        let mut model = model();
        let member = model
            .members
            .iter_mut()
            .find(|member| member.key == "wall-plate-south")
            .expect("wall plate member");
        member.to = member.from;
        let report = model.validate();
        assert!(has_issue(
            &report,
            "coincident-member-endpoints",
            "wall-plate-south"
        ));
    }

    #[test]
    fn joint_node_must_touch_every_named_member_and_members_are_unique() {
        let mut displaced = model();
        let wrong_node = displaced
            .nodes
            .iter()
            .position(|node| node.key == "node-wall-plate-south-west")
            .map(NodeId)
            .expect("wrong node exists");
        displaced
            .joints
            .iter_mut()
            .find(|joint| joint.key == "joint-apex-west")
            .expect("apex joint")
            .node = wrong_node;
        let displaced_report = displaced.validate();
        assert!(has_issue(
            &displaced_report,
            "joint-not-incident-to-member",
            "joint-apex-west"
        ));

        let mut duplicate = model();
        let joint = duplicate
            .joints
            .iter_mut()
            .find(|joint| joint.key == "joint-apex-west")
            .expect("apex joint");
        joint.members.push(joint.members[0]);
        let duplicate_report = duplicate.validate();
        assert!(has_issue(
            &duplicate_report,
            "duplicate-joint-member",
            "joint-apex-west"
        ));
    }

    #[test]
    fn orphan_graph_records_and_members_on_omitted_elements_are_rejected() {
        let mut model = model();
        model.nodes.push(Node {
            key: "node-orphan".to_owned(),
            point: [1.0, 1.0, 1.0],
        });
        let existing = model.members[0].clone();
        model.members.push(Member {
            key: "member-orphan".to_owned(),
            ..existing
        });
        model.joints.push(Joint {
            key: "joint-orphan".to_owned(),
            node: NodeId(0),
            members: vec![MemberId(0)],
            kind: JointKind::BearingSeat,
            evidence: EvidenceClass::ModernEngineeringInference,
            evidence_source: evidence_source_key(EvidenceClass::ModernEngineeringInference)
                .to_owned(),
        });
        let masonry = element_id(&model, "masonry-south");
        model.supports.push(Support {
            key: "support-orphan".to_owned(),
            element: masonry,
            ground: "ground".to_owned(),
            restraints: Restraints {
                translation: [true, true, true],
            },
        });
        model.omit_element("wall-plate-south");
        let report = model.validate();
        assert!(has_issue(&report, "orphan-node", "node-orphan"));
        assert!(has_issue(&report, "orphan-member", "member-orphan"));
        assert!(has_issue(&report, "orphan-joint", "joint-orphan"));
        assert!(has_issue(&report, "orphan-support", "support-orphan"));
        assert!(has_issue(
            &report,
            "member-on-omitted-element",
            "wall-plate-south"
        ));
    }

    #[test]
    fn bearing_frame_anchors_and_overlap_minima_are_validated() {
        let mut bad_frame = model();
        bad_frame.bearings[0].normal = [2.0, 0.0, 0.0];
        assert!(has_issue(
            &bad_frame.validate(),
            "invalid-bearing-frame",
            &bad_frame.bearings[0].key
        ));

        let mut nonorthogonal = model();
        nonorthogonal.bearings[0].tangents[1] = nonorthogonal.bearings[0].tangents[0];
        assert!(has_issue(
            &nonorthogonal.validate(),
            "invalid-bearing-frame",
            &nonorthogonal.bearings[0].key
        ));

        let mut bad_anchor = model();
        bad_anchor.bearings[0].carried.local[0] = -1.0;
        assert!(has_issue(
            &bad_anchor.validate(),
            "bearing-anchor-out-of-bounds",
            &bad_anchor.bearings[0].key
        ));

        for minimum in [-0.01, f64::NAN] {
            let mut bad_minimum = model();
            bad_minimum.bearings[0].minimum_overlap[0] = minimum;
            assert!(has_issue(
                &bad_minimum.validate(),
                "invalid-bearing-minimum",
                &bad_minimum.bearings[0].key
            ));
        }
    }

    #[test]
    fn tangentially_separated_anchor_points_do_not_witness_a_bearing() {
        let mut model = model();
        let bearing = model
            .bearings
            .iter_mut()
            .find(|bearing| bearing.key == "bearing-principal-south-east-on-wall-plate")
            .expect("selected principal bearing exists");
        bearing.carrier.local[0] = 3.0;

        let report = model.validate();
        assert!(has_issue(
            &report,
            "misaligned-bearing-anchors",
            "bearing-principal-south-east-on-wall-plate"
        ));
        assert!(has_issue(
            &report,
            "unwitnessed-bearing-transfer",
            "load-principal-south-east-to-wall-plate"
        ));
        assert!(has_issue(
            &report,
            "insufficient-direct-supports",
            "principal-rafter-south-east"
        ));
    }

    #[test]
    fn dangling_transfer_cycle_is_reported_without_panicking() {
        let mut model = model();
        let dangling = ElementId(model.elements.len() + 8);
        model.transfers.push(LoadTransfer {
            key: "load-south-masonry-to-dangling".to_owned(),
            from: element_id(&model, "masonry-south"),
            to: TransferTarget::Element(dangling),
            kind: TransferKind::Bearing,
        });
        model.transfers.push(LoadTransfer {
            key: "load-dangling-self-cycle".to_owned(),
            from: dangling,
            to: TransferTarget::Element(dangling),
            kind: TransferKind::Joint,
        });

        let report = model.validate();
        assert!(has_issue(
            &report,
            "missing-transfer-element",
            "load-south-masonry-to-dangling"
        ));
        assert!(has_issue(
            &report,
            "missing-transfer-element",
            "load-dangling-self-cycle"
        ));
    }

    #[test]
    fn contact_tolerance_accepts_sub_epsilon_and_rejects_beyond_it() {
        let baseline = model();
        let normal = baseline
            .bearings
            .iter()
            .find(|bearing| bearing.key == "bearing-covering-on-boarding-south")
            .expect("covering bearing")
            .normal;
        for signed_distance in [CONTACT_TOLERANCE * 0.5, -CONTACT_TOLERANCE * 0.5] {
            let mut within = baseline.clone();
            within.translate_element("roof-covering-south", scale(normal, signed_distance));
            let report = within.validate();
            assert!(!report.issues.iter().any(|issue| {
                issue.key == "bearing-covering-on-boarding-south"
                    && matches!(issue.code, "floating-bearing" | "embedded-bearing")
            }));
        }
        for (signed_distance, expected) in [
            (CONTACT_TOLERANCE * 2.0, "floating-bearing"),
            (-CONTACT_TOLERANCE * 2.0, "embedded-bearing"),
        ] {
            let mut outside = baseline.clone();
            outside.translate_element("roof-covering-south", scale(normal, signed_distance));
            assert!(has_issue(
                &outside.validate(),
                expected,
                "bearing-covering-on-boarding-south"
            ));
        }
    }

    #[test]
    fn typed_selectors_disambiguate_cross_category_keys() {
        let model = model();
        let error = model
            .explain("wall-plate-south")
            .expect_err("bare element/member key is ambiguous");
        assert!(error.contains("element:wall-plate-south"));
        assert!(error.contains("member:wall-plate-south"));
        assert!(
            model
                .explain("element:wall-plate-south")
                .expect("valid selector")
                .is_some()
        );
        assert!(
            model
                .explain("member:wall-plate-south")
                .expect("valid selector")
                .is_some()
        );
    }

    #[test]
    fn bearing_and_evidence_explanations_include_measured_witnesses() {
        let model = model();
        let bearing = model
            .explain("bearing:bearing-principal-south-east-on-wall-plate")
            .expect("valid selector")
            .expect("bearing exists");
        assert!(bearing.contains("geometry=analytic-anchor-only"));
        assert!(bearing.contains("gap="));
        assert!(bearing.contains("tolerance=0.000000001000m"));
        assert!(bearing.contains("overlaps="));
        assert!(bearing.contains("source=tfec-modern-truss-detailing"));

        let evidence = model
            .explain("evidence:tfec-modern-truss-detailing")
            .expect("valid selector")
            .expect("evidence exists");
        assert!(evidence.contains("class=modern-engineering-inference"));
        assert!(evidence.contains("claim:"));
    }

    #[test]
    fn evidence_links_must_exist_and_match_the_declared_class() {
        let mut missing = model();
        missing.elements[0].evidence_source = "missing-source".to_owned();
        assert!(has_issue(
            &missing.validate(),
            "missing-evidence-source",
            &missing.elements[0].key
        ));

        let mut mismatch = model();
        mismatch.elements[0].evidence_source =
            evidence_source_key(EvidenceClass::ModernEngineeringInference).to_owned();
        assert!(has_issue(
            &mismatch.validate(),
            "evidence-class-mismatch",
            &mismatch.elements[0].key
        ));
    }
}
