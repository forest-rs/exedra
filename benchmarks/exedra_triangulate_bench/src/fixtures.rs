// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fixed benchmark fixtures and typed predicate scenarios.

use exedra_triangulate::PolygonInput;
use exedra_triangulate::predicates::{InCircle, IncirclePath, Orient2dPath, Orientation};

#[derive(Clone, Debug)]
pub(crate) struct Fixture {
    pub(crate) name: &'static str,
    pub(crate) role: FixtureRole,
    pub(crate) outer: Vec<[f64; 2]>,
    pub(crate) holes: Vec<Vec<[f64; 2]>>,
}

impl Fixture {
    pub(crate) fn vertex_count(&self) -> usize {
        self.outer.len() + self.holes.iter().map(Vec::len).sum::<usize>()
    }

    pub(crate) fn prepare(&self) -> PreparedFixture<'_> {
        PreparedFixture {
            fixture: self,
            hole_refs: self.holes.iter().map(Vec::as_slice).collect(),
        }
    }

    pub(crate) fn points(&self) -> Vec<[f64; 2]> {
        self.outer
            .iter()
            .chain(self.holes.iter().flatten())
            .copied()
            .collect()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum FixtureRole {
    ChoiceQuality,
    TieControl,
    InputConstraint,
}

impl FixtureRole {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::ChoiceQuality => "choice_quality",
            Self::TieControl => "tie_control",
            Self::InputConstraint => "input_constraint",
        }
    }
}

/// Deliberately acute boundary-only input used to verify honest exceptions.
#[cfg(test)]
pub(crate) fn input_limited_fixture() -> Fixture {
    Fixture {
        name: "input_limited_acute_corner",
        role: FixtureRole::InputConstraint,
        outer: vec![[0.0, 0.0], [20.0, 0.0], [0.001, 0.01]],
        holes: Vec::new(),
    }
}

pub(crate) struct PreparedFixture<'fixture> {
    fixture: &'fixture Fixture,
    hole_refs: Vec<&'fixture [[f64; 2]]>,
}

impl PreparedFixture<'_> {
    pub(crate) fn input(&self) -> PolygonInput<'_> {
        PolygonInput {
            outer: &self.fixture.outer,
            holes: &self.hole_refs,
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct CircleStep {
    cos: f64,
    sin: f64,
}

impl CircleStep {
    const N16: Self = Self {
        cos: 0.923_879_532_511_286_7,
        sin: 0.382_683_432_365_089_8,
    };
    const N64: Self = Self {
        cos: 0.995_184_726_672_196_9,
        sin: 0.098_017_140_329_560_6,
    };
    const N120: Self = Self {
        cos: 0.998_629_534_754_573_8,
        sin: 0.052_335_956_242_943_835,
    };
}

pub(crate) fn fixtures() -> Vec<Fixture> {
    let choice = vec![[4.9, 4.9], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
    vec![
        Fixture {
            name: "choice_driven_quad",
            role: FixtureRole::ChoiceQuality,
            outer: choice.clone(),
            holes: Vec::new(),
        },
        Fixture {
            name: "exact_cocircular_quad",
            role: FixtureRole::TieControl,
            outer: vec![[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]],
            holes: Vec::new(),
        },
        near_circle_fixture("near_circle_16", 16, CircleStep::N16),
        near_circle_fixture("near_circle_64", 64, CircleStep::N64),
        near_circle_fixture("near_circle_120", 120, CircleStep::N120),
        sparse_hole_fixture("sparse_rect_dense_hole_16", 16, CircleStep::N16),
        sparse_hole_fixture("sparse_rect_dense_hole_64", 64, CircleStep::N64),
        sparse_hole_fixture("sparse_rect_dense_hole_120", 120, CircleStep::N120),
        drill_collinear_fixture(),
        bridge_stress_fixture(),
        Fixture {
            name: "small_angle_wedge",
            role: FixtureRole::InputConstraint,
            outer: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.001, 0.01]],
            holes: Vec::new(),
        },
        Fixture {
            name: "choice_scale_down_2p-200",
            role: FixtureRole::ChoiceQuality,
            outer: scale_points(&choice, power_of_two(-200)),
            holes: Vec::new(),
        },
        Fixture {
            name: "choice_scale_up_2p200",
            role: FixtureRole::ChoiceQuality,
            outer: scale_points(&choice, power_of_two(200)),
            holes: Vec::new(),
        },
    ]
}

fn near_circle_fixture(name: &'static str, count: usize, step: CircleStep) -> Fixture {
    Fixture {
        name,
        role: FixtureRole::TieControl,
        outer: near_circle(count, 10.0, [0.0, 0.0], step),
        holes: Vec::new(),
    }
}

fn sparse_hole_fixture(name: &'static str, count: usize, step: CircleStep) -> Fixture {
    let mut hole = near_circle(count, 4.0, [0.0, 0.0], step);
    hole.reverse();
    Fixture {
        name,
        role: FixtureRole::InputConstraint,
        outer: vec![[-10.0, -6.0], [10.0, -6.0], [10.0, 6.0], [-10.0, 6.0]],
        holes: vec![hole],
    }
}

fn near_circle(count: usize, radius: f64, center: [f64; 2], step: CircleStep) -> Vec<[f64; 2]> {
    let mut points = Vec::with_capacity(count);
    let (mut x, mut y) = (radius, 0.0);
    for _ in 0..count {
        points.push([center[0] + x, center[1] + y]);
        (x, y) = (x * step.cos - y * step.sin, x * step.sin + y * step.cos);
    }
    points
}

fn drill_collinear_fixture() -> Fixture {
    let ring = [
        [13.0, 6.0],
        [12.0, 8.0],
        [10.0, 9.0],
        [8.0, 8.0],
        [7.0, 6.0],
        [8.0, 4.0],
        [10.0, 3.0],
        [12.0, 4.0],
    ];
    let mut hole = Vec::with_capacity(ring.len() * 2);
    for (index, &point) in ring.iter().enumerate() {
        let next = ring[(index + 1) % ring.len()];
        hole.push(point);
        hole.push([(point[0] + next[0]) / 2.0, (point[1] + next[1]) / 2.0]);
    }
    hole.reverse();
    Fixture {
        name: "drill_like_collinear_midpoints",
        role: FixtureRole::InputConstraint,
        outer: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 12.0], [0.0, 12.0]],
        holes: vec![hole],
    }
}

fn bridge_stress_fixture() -> Fixture {
    let mut circle = near_circle(16, 1.5, [15.0, 7.5], CircleStep::N16);
    circle.reverse();
    Fixture {
        name: "three_hole_bridge_stress",
        role: FixtureRole::TieControl,
        outer: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 15.0], [0.0, 15.0]],
        holes: vec![
            vec![[5.0, 5.0], [5.0, 8.0], [8.0, 8.0], [8.0, 5.0]],
            circle,
            vec![[22.0, 5.0], [24.0, 9.0], [26.0, 5.0]],
        ],
    }
}

fn scale_points(points: &[[f64; 2]], scale: f64) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|point| [point[0] * scale, point[1] * scale])
        .collect()
}

pub(crate) fn power_of_two(exponent: i32) -> f64 {
    assert!(
        (-1022..=1023).contains(&exponent),
        "benchmark power must be a normal finite f64"
    );
    let biased = u64::try_from(exponent + 1023).expect("biased exponent is nonnegative");
    f64::from_bits(biased << 52)
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct PredicateScenario {
    pub(crate) name: &'static str,
    pub(crate) points: [[f64; 2]; 3],
    pub(crate) orientation: Orientation,
    pub(crate) path: Orient2dPath,
}

pub(crate) fn predicate_scenarios() -> [PredicateScenario; 3] {
    [
        PredicateScenario {
            name: "orient2d_filter",
            points: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            orientation: Orientation::Ccw,
            path: Orient2dPath::Filter,
        },
        PredicateScenario {
            name: "orient2d_normalized_expansion",
            points: [[0.0, 0.0], [1e-300, 0.0], [0.0, 1e-300]],
            orientation: Orientation::Ccw,
            path: Orient2dPath::NormalizedExpansion,
        },
        PredicateScenario {
            name: "orient2d_dyadic",
            points: [[f64::from_bits(1), 0.0], [0.5, 0.5], [1.0, 1.0]],
            orientation: Orientation::Cw,
            path: Orient2dPath::Dyadic,
        },
    ]
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct IncircleScenario {
    pub(crate) name: &'static str,
    pub(crate) points: [[f64; 2]; 4],
    pub(crate) position: InCircle,
    pub(crate) path: IncirclePath,
}

pub(crate) fn incircle_scenarios() -> [IncircleScenario; 2] {
    [
        IncircleScenario {
            name: "incircle_filter",
            points: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.25, 0.25]],
            position: InCircle::Inside,
            path: IncirclePath::Filter,
        },
        IncircleScenario {
            name: "incircle_dyadic",
            points: [[0.0, 0.0], [1e100, 0.0], [0.0, 1e100], [1e100, 1e100]],
            position: InCircle::Cocircular,
            path: IncirclePath::Dyadic,
        },
    ]
}
