// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native point boundaries exported without invented three-dimensional curves.
use exedra_analytic::{
    body::{
        Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, Sense, ValidationPolicy,
        VertexId,
    },
    geometry::{Curve, ParameterDomain, ParameterRange, Surface},
};
use exedra_math::Placement3;
use exedra_step::{LengthUnit, StepOptions, export_body};
use std::{error::Error, path::Path};

fn sphere(hemisphere: bool, seam: bool) -> Body {
    let radius = 2.0;
    let low = if hemisphere {
        0.0
    } else {
        -std::f64::consts::FRAC_PI_2
    };
    let vertices = vec![
        if hemisphere {
            [radius, 0.0, 0.0]
        } else {
            [0.0, 0.0, -radius]
        },
        [0.0, 0.0, radius],
    ];
    let mut edges = Vec::new();
    if hemisphere {
        edges.push(Edge::new(
            Curve::circle(Placement3::IDENTITY, radius).unwrap(),
            ParameterRange::new(0.0, std::f64::consts::TAU).unwrap(),
            VertexId(0),
            VertexId(0),
            (),
        ));
    } else {
        edges.push(Edge::collapsed(VertexId(0), ()));
    }
    edges.push(Edge::collapsed(VertexId(1), ()));
    let mut ring = vec![Coedge {
        edge: EdgeId(0),
        reversed: false,
    }];
    if seam {
        let frame = if hemisphere {
            Placement3::from_axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0; 3])
        } else {
            Placement3::from_axes(
                [0.0, 0.0, -1.0],
                [1.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
                [0.0; 3],
            )
        };
        edges.push(Edge::new(
            Curve::circle(frame, radius).unwrap(),
            ParameterRange::new(
                0.0,
                if hemisphere {
                    std::f64::consts::FRAC_PI_2
                } else {
                    std::f64::consts::PI
                },
            )
            .unwrap(),
            VertexId(0),
            VertexId(1),
            (),
        ));
        ring.extend([
            Coedge {
                edge: EdgeId(2),
                reversed: false,
            },
            Coedge {
                edge: EdgeId(1),
                reversed: true,
            },
            Coedge {
                edge: EdgeId(2),
                reversed: true,
            },
        ]);
    }
    let mut loops = vec![BoundaryLoop { coedges: ring }];
    let mut faces = vec![Face {
        surface: Surface::sphere(Placement3::IDENTITY, radius).unwrap(),
        domain: ParameterDomain {
            u: ParameterRange::new(0.0, std::f64::consts::TAU).unwrap(),
            v: ParameterRange::new(low, std::f64::consts::FRAC_PI_2).unwrap(),
        },
        sense: Sense::Forward,
        loops: vec![LoopId(0)],
        role: (),
    }];
    if hemisphere {
        loops.push(BoundaryLoop {
            coedges: vec![Coedge {
                edge: EdgeId(0),
                reversed: true,
            }],
        });
        faces.push(Face {
            surface: Surface::plane(Placement3::IDENTITY).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(-radius, radius).unwrap(),
                v: ParameterRange::new(-radius, radius).unwrap(),
            },
            sense: Sense::Reverse,
            loops: vec![LoopId(1)],
            role: (),
        });
    }
    Body::try_from_parts(
        BodyParts {
            vertices,
            edges,
            loops,
            faces,
        },
        &ValidationPolicy { max_gap: 1e-10 },
    )
    .unwrap()
}

fn hollow_sector(angle: f64) -> Body {
    let (sin, cos) = angle.sin_cos();
    let vertices = vec![
        [0.0, 0.0, -2.0],
        [0.0, 0.0, 2.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, 1.0],
    ];
    let mut edges: Vec<_> = (0..4).map(|i| Edge::collapsed(VertexId(i), ())).collect();
    for (radius, start, end) in [(2.0, 0, 1), (1.0, 2, 3)] {
        for (s, c) in [(0.0, 1.0), (sin, cos)] {
            let frame =
                Placement3::from_axes([0.0, 0.0, -1.0], [c, s, 0.0], [s, -c, 0.0], [0.0; 3]);
            edges.push(Edge::new(
                Curve::circle(frame, radius).unwrap(),
                ParameterRange::new(0.0, std::f64::consts::PI).unwrap(),
                VertexId(start),
                VertexId(end),
                (),
            ));
        }
    }
    for (start, end) in [(2_usize, 0_usize), (1, 3)] {
        edges.push(Edge::new(
            Curve::line(
                vertices[start],
                exedra_math::sub(vertices[end], vertices[start]),
            )
            .unwrap(),
            ParameterRange::UNIT,
            VertexId(u32::try_from(start).unwrap()),
            VertexId(u32::try_from(end).unwrap()),
            (),
        ));
    }
    let loops = [
        vec![(0, false), (5, false), (1, true), (4, true)],
        vec![(2, true), (6, false), (3, false), (7, true)],
        vec![(4, false), (9, false), (6, true), (8, false)],
        vec![(5, true), (8, true), (7, false), (9, true)],
    ]
    .into_iter()
    .map(|ring| BoundaryLoop {
        coedges: ring
            .into_iter()
            .map(|(edge, reversed)| Coedge {
                edge: EdgeId(edge),
                reversed,
            })
            .collect(),
    })
    .collect();
    let spherical = ParameterDomain {
        u: ParameterRange::new(0.0, angle).unwrap(),
        v: ParameterRange::new(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2).unwrap(),
    };
    let planar = ParameterDomain {
        u: ParameterRange::new(0.0, 2.0).unwrap(),
        v: ParameterRange::new(-2.0, 2.0).unwrap(),
    };
    let faces = vec![
        Face {
            surface: Surface::sphere(Placement3::IDENTITY, 2.0).unwrap(),
            domain: spherical,
            sense: Sense::Forward,
            loops: vec![LoopId(0)],
            role: (),
        },
        Face {
            surface: Surface::sphere(Placement3::IDENTITY, 1.0).unwrap(),
            domain: spherical,
            sense: Sense::Reverse,
            loops: vec![LoopId(1)],
            role: (),
        },
        Face {
            surface: Surface::plane(Placement3::from_axes(
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0],
                [0.0; 3],
            ))
            .unwrap(),
            domain: planar,
            sense: Sense::Forward,
            loops: vec![LoopId(2)],
            role: (),
        },
        Face {
            surface: Surface::plane(Placement3::from_axes(
                [cos, sin, 0.0],
                [0.0, 0.0, 1.0],
                [sin, -cos, 0.0],
                [0.0; 3],
            ))
            .unwrap(),
            domain: planar,
            sense: Sense::Reverse,
            loops: vec![LoopId(3)],
            role: (),
        },
    ];
    Body::try_from_parts(
        BodyParts {
            vertices,
            edges,
            loops,
            faces,
        },
        &ValidationPolicy { max_gap: 1e-10 },
    )
    .unwrap()
}

pub(super) fn export_cases(
    out: &Path,
    expected: &mut Vec<serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    let cases = [
        (
            "sphere-poles",
            sphere(false, true),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-2.0; 3], [2.0; 3]],
            false,
        ),
        (
            "sphere-vertex-loop",
            sphere(false, false),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-2.0; 3], [2.0; 3]],
            false,
        ),
        (
            "sphere-placed",
            sphere(false, true),
            Placement3::from_axes(
                [0.0, 0.0, -1.0],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
                [100.0, -30.0, 7.0],
            ),
            LengthUnit::Metre,
            [[98.0, -32.0, 5.0], [102.0, -28.0, 9.0]],
            false,
        ),
        (
            "hemisphere-pole",
            sphere(true, true),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-2.0, -2.0, 0.0], [2.0, 2.0, 2.0]],
            true,
        ),
    ];
    for (name, body, placement, units, bounds, half) in cases {
        let file = export_body(
            &body,
            placement,
            &StepOptions {
                name,
                units,
                ..StepOptions::default()
            },
        )?;
        let pi = std::f64::consts::PI;
        super::save(
            out,
            expected,
            name,
            &file,
            if half {
                16.0 * pi / 3.0
            } else {
                32.0 * pi / 3.0
            },
            if half { 12.0 * pi } else { 16.0 * pi },
            bounds,
        )?;
        let fixture = expected.last_mut().unwrap();
        // STEP records and reader-reconstructed natural seams/poles differ.
        // Keep the serialized counts explicit; don't claim fake EDGE_CURVEs.
        fixture["serialized_edges"] = serde_json::json!(file.report.counts.edges);
        fixture["serialized_vertices"] = serde_json::json!(file.report.counts.vertices);
        fixture["edges"] = serde_json::json!(3);
        fixture["vertices"] = serde_json::json!(2);
        fixture["surface_counts"] = if half {
            serde_json::json!({"sphere":1,"plane":1})
        } else {
            serde_json::json!({"sphere":1})
        };
        fixture["witnesses"] = serde_json::json!([
            {"point":placement.transform_point([0.0,0.0,1.0]),"state":"inside"},
            {"point":placement.transform_point([0.0,0.0,3.0]),"state":"outside"}
        ]);
    }
    let angle = 1.8_f64;
    let body = hollow_sector(angle);
    let file = export_body(
        &body,
        Placement3::IDENTITY,
        &StepOptions {
            name: "hollow-sphere-sector",
            ..StepOptions::default()
        },
    )?;
    super::save(
        out,
        expected,
        "hollow-sphere-sector",
        &file,
        14.0 * angle / 3.0,
        10.0 * angle + 3.0 * std::f64::consts::PI,
        [[2.0 * angle.cos(), 0.0, -2.0], [2.0, 2.0, 2.0]],
    )?;
    let fixture = expected.last_mut().unwrap();
    fixture["serialized_edges"] = serde_json::json!(file.report.counts.edges);
    fixture["serialized_vertices"] = serde_json::json!(file.report.counts.vertices);
    fixture["edges"] = serde_json::json!(10);
    fixture["vertices"] = serde_json::json!(4);
    fixture["surface_counts"] = serde_json::json!({"sphere":2,"plane":2});
    fixture["witnesses"] = serde_json::json!([
        {"point":[1.5*(angle/2.0).cos(),1.5*(angle/2.0).sin(),0.0],"state":"inside"},
        {"point":[0.5*(angle/2.0).cos(),0.5*(angle/2.0).sin(),0.0],"state":"outside"},
        {"point":[-1.5,0.0,0.0],"state":"outside"}
    ]);
    Ok(())
}

/// Exports through actual authoring constructors, independently of display realization.
pub(super) fn export_constructed(
    out: &Path,
    expected: &mut Vec<serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    use exedra_analytic::revolve::Revolution;
    use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
    let pose = Placement3::from_axes(
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [100.0, -30.0, 7.0],
    );
    for (name, kind, scale, frame, units) in [
        (
            "authored-sphere",
            0,
            1.0,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
        (
            "authored-cone",
            1,
            1.0,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
        (
            "authored-reverse-cone-placed",
            2,
            1.0,
            pose,
            LengthUnit::Metre,
        ),
        ("authored-sphere-placed", 0, 1.0, pose, LengthUnit::Metre),
        (
            "authored-sphere-small",
            0,
            1e-6,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
        (
            "authored-cone-small",
            1,
            1e-6,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
        (
            "authored-sphere-large",
            0,
            1e6,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
        (
            "authored-cone-large",
            1,
            1e6,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
    ] {
        let radius = 2.0 * scale;
        let height = 3.0 * scale;
        let segments = match kind {
            0 => vec![
                Segment::line([0.0, -radius]),
                Segment::arc([0.0, radius], 1.0),
            ],
            1 => vec![
                Segment::line([radius, 0.0]),
                Segment::line([0.0, height]),
                Segment::line([0.0, 0.0]),
            ],
            _ => vec![
                Segment::line([radius, height]),
                Segment::line([0.0, height]),
                Segment::line([0.0, 0.0]),
            ],
        };
        let profile = Profile::new(
            Loop::new(segments)?,
            vec![],
            &ProfilePolicy::new(radius.max(height) * 1e-9),
        )?;
        let native = Revolution::new(profile, frame, std::f64::consts::TAU)?;
        let file = export_body(
            native.body(),
            native.frame(),
            &StepOptions {
                name,
                units,
                tolerance: (scale * 1e-9).max(1e-12),
                ..StepOptions::default()
            },
        )?;
        let local = if kind == 0 {
            [[-radius; 3], [radius; 3]]
        } else {
            [[-radius, 0.0, -radius], [radius, height, radius]]
        };
        // The occurrence above is an exact signed axis permutation.
        let corners: Vec<_> = (0..8)
            .map(|corner| {
                frame.transform_point(core::array::from_fn(|axis| {
                    local[usize::from(corner & (1 << axis) != 0)][axis]
                }))
            })
            .collect();
        let bounds = [
            core::array::from_fn(|axis| {
                corners
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::INFINITY, f64::min)
            }),
            core::array::from_fn(|axis| {
                corners
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::NEG_INFINITY, f64::max)
            }),
        ];
        super::save(
            out,
            expected,
            name,
            &file,
            native.volume(),
            native.surface_area(),
            bounds,
        )?;
        let fixture = expected.last_mut().unwrap();
        fixture["authoring"] = serde_json::json!({"constructor":"Revolution::new","profile":if kind==0 {serde_json::json!({"kind":"positiveHalfDisk","radius":radius})} else {serde_json::json!({"kind":"triangle","radius":radius,"height":height,"apex":if kind==1 {"end"} else {"start"}})},"sweep":std::f64::consts::TAU,"frame":frame.rows,"separation":radius.max(height)*1e-9});
        fixture["edges"] = serde_json::json!(3);
        fixture["vertices"] = serde_json::json!(2);
        fixture["surface_counts"] = if kind == 0 {
            serde_json::json!({"sphere":1})
        } else {
            serde_json::json!({"cone":1,"plane":1})
        };
        let center = if kind == 0 {
            [0.0; 3]
        } else {
            [0.0, height * if kind == 1 { 0.25 } else { 0.75 }, 0.0]
        };
        fixture["centroid"] = serde_json::json!(frame.transform_point(center));
        let inside = if kind == 0 {
            [0.0; 3]
        } else {
            [0.0, height * 0.5, 0.0]
        };
        fixture["witnesses"] = serde_json::json!([
            {"point":frame.transform_point(inside),"state":"inside"},
            {"point":frame.transform_point([radius*2.0,0.0,0.0]),"state":"outside"}
        ]);
    }
    Ok(())
}

pub(super) fn export_caps(
    out: &Path,
    expected: &mut Vec<serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    use exedra_analytic::revolve::Revolution;
    use exedra_profile::ProfilePolicy;
    let pose = Placement3::from_axes(
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [100.0, -30.0, 7.0],
    );
    let mut cases: Vec<_> = [
        (
            "authored-shallow-dome",
            0.25,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
        (
            "authored-hemisphere",
            2.0,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
        (
            "authored-tall-cap",
            3.5,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
        ),
        ("authored-hemisphere-placed", 2.0, pose, LengthUnit::Metre),
        (
            "authored-cap-full-sphere",
            4.0,
            pose,
            LengthUnit::Millimetre,
        ),
    ]
    .into_iter()
    .map(|(name, height, frame, units)| (name.to_owned(), 2.0, height, frame, units, 1.0))
    .collect();
    for (stem, radius, height) in [
        ("r25-h025", 25.0, 0.25),
        ("r25-h05", 25.0, 0.5),
        ("r60-h05", 60.0, 0.5),
    ] {
        for (label, scale) in [("small", 1e-6), ("unit", 1.0), ("large", 1e6)] {
            cases.push((
                format!("authored-shallow-{stem}-{label}"),
                radius * scale,
                height * scale,
                Placement3::IDENTITY,
                LengthUnit::Millimetre,
                scale,
            ));
        }
    }
    for (name, radius, height, frame, units, scale) in cases {
        let separation = 1e-9 * scale;
        let native =
            Revolution::spherical_cap(radius, height, frame, &ProfilePolicy::new(separation))?;
        let file = export_body(
            native.body(),
            native.frame(),
            &StepOptions {
                name: &name,
                units,
                tolerance: 1e-6 * scale,
                ..StepOptions::default()
            },
        )?;
        let radial = if height <= radius {
            f64::sqrt(height * (2.0 * radius - height))
        } else {
            radius
        };
        let local = [[-radial, 0.0, -radial], [radial, height, radial]];
        let corners: Vec<_> = (0..8)
            .map(|corner| {
                frame.transform_point(core::array::from_fn(|axis| {
                    local[usize::from(corner & (1 << axis) != 0)][axis]
                }))
            })
            .collect();
        let bounds = [
            core::array::from_fn(|axis| {
                corners
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::INFINITY, f64::min)
            }),
            core::array::from_fn(|axis| {
                corners
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::NEG_INFINITY, f64::max)
            }),
        ];
        super::save(
            out,
            expected,
            &name,
            &file,
            std::f64::consts::PI * height * height * (radius - height / 3.0),
            std::f64::consts::PI * height * (4.0 * radius - height),
            bounds,
        )?;
        let fixture = expected.last_mut().unwrap();
        fixture["authoring"] = serde_json::json!({"constructor":"Revolution::spherical_cap","radius":radius,"height":height,"frame":frame.rows,"separation":separation,"native_volume":native.volume(),"native_surface_area":native.surface_area()});
        fixture["max_native_support_gap"] = serde_json::json!(file.report.max_native_support_gap);
        fixture["frame_roundoff_bound"] = serde_json::json!(file.report.frame_roundoff_bound);
        fixture["parameter_roundoff_bound"] =
            serde_json::json!(file.report.parameter_roundoff_bound);
        if name.starts_with("authored-shallow-r") {
            fixture["bounds_method"] = serde_json::json!("analytic_cap_domains");
        }
        fixture["edges"] = serde_json::json!(3);
        fixture["vertices"] = serde_json::json!(2);
        fixture["centroid"] = serde_json::json!(frame.transform_point([
            0.0,
            height * (4.0 * radius - height) / (4.0 * (3.0 * radius - height)),
            0.0
        ]));
        fixture["surface_counts"] = if height == 2.0 * radius {
            serde_json::json!({"sphere":1})
        } else {
            serde_json::json!({"sphere":1,"plane":1})
        };
        fixture["witnesses"] = serde_json::json!([
            {"point":frame.transform_point([0.0,height*0.5,0.0]),"state":"inside"},
            {"point":frame.transform_point([0.0,-0.5*scale,0.0]),"state":"outside"},
            {"point":frame.transform_point([0.0,height+0.5*scale,0.0]),"state":"outside"}
        ]);
    }
    Ok(())
}
