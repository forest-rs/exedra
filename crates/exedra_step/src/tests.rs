// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::{format, vec};
use exedra_analytic::{
    body::{BodyParts, BoundaryLoop, Coedge, Edge, Face, Sense},
    geometry::{Curve, CurveSpan, ParameterDomain, ParameterRange, Surface},
};

fn cube() -> Body {
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 3.0, 0.0],
        [0.0, 3.0, 0.0],
        [0.0, 0.0, 4.0],
        [2.0, 0.0, 4.0],
        [2.0, 3.0, 4.0],
        [0.0, 3.0, 4.0],
    ];
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ]
    .into_iter()
    .map(|(a, b)| Edge {
        geometry: exedra_analytic::body::EdgeGeometry::Curve {
            curve: Curve::line(vertices[a], exedra_math::sub(vertices[b], vertices[a])).unwrap(),
            range: ParameterRange::UNIT,
        },
        start: VertexId(u32::try_from(a).unwrap()),
        end: VertexId(u32::try_from(b).unwrap()),
        role: (),
    })
    .collect();
    let loops = [
        [(3, true), (2, true), (1, true), (0, true)],
        [(4, false), (5, false), (6, false), (7, false)],
        [(0, false), (9, false), (4, true), (8, true)],
        [(1, false), (10, false), (5, true), (9, true)],
        [(2, false), (11, false), (6, true), (10, true)],
        [(3, false), (8, false), (7, true), (11, true)],
    ]
    .into_iter()
    .map(|ring| BoundaryLoop {
        coedges: ring
            .map(|(edge, reversed)| Coedge {
                edge: EdgeId(edge),
                reversed,
            })
            .to_vec(),
    })
    .collect();
    let frames = [
        Placement3::IDENTITY,
        Placement3::translate(0.0, 0.0, 4.0),
        Placement3::from_axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0; 3]),
        Placement3::from_axes(
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
        ),
        Placement3::from_axes(
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [2.0, 3.0, 0.0],
        ),
        Placement3::from_axes(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
            [0.0, 3.0, 0.0],
        ),
    ];
    let faces = frames
        .into_iter()
        .enumerate()
        .map(|(i, frame)| Face {
            surface: Surface::plane(frame).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(0.0, if i == 3 || i == 5 { 3.0 } else { 2.0 }).unwrap(),
                v: ParameterRange::new(0.0, if i < 2 { 3.0 } else { 4.0 }).unwrap(),
            },
            sense: if i == 0 {
                Sense::Reverse
            } else {
                Sense::Forward
            },
            loops: vec![LoopId(u32::try_from(i).unwrap())],
            role: (),
        })
        .collect();
    Body::try_from_parts(
        BodyParts {
            trims: Vec::new(),
            vertices,
            edges,
            loops,
            faces,
        },
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap()
}
#[test]
fn complete_document_is_deterministic_shared_and_fully_referenced() {
    let body = cube();
    let options = StepOptions::default();
    let a = export_body(&body, Placement3::IDENTITY, &options).unwrap();
    assert_eq!(
        a,
        export_body(&body, Placement3::IDENTITY, &options).unwrap()
    );
    assert!(a.text.starts_with("ISO-10303-21;\nHEADER;"));
    assert!(a.text.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    assert!(a.text.contains("FILE_SCHEMA(('AUTOMOTIVE_DESIGN'))"));
    assert_eq!(
        (
            a.report.counts.vertices,
            a.report.counts.edges,
            a.report.counts.coedges,
            a.report.counts.faces
        ),
        (8, 12, 24, 6)
    );
    assert_eq!(a.text.matches("EDGE_CURVE(").count(), 12);
    assert_eq!(a.text.matches("ORIENTED_EDGE(").count(), 24);
    let entities: Vec<_> = a
        .text
        .lines()
        .filter(|line| line.starts_with('#'))
        .collect();
    assert_eq!(entities.len(), a.report.counts.entities as usize);
    for (i, line) in entities.iter().enumerate() {
        assert!(line.starts_with(&format!("#{}=", i + 1)));
    }
    for tail in a.text.split('#').skip(1) {
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let id = tail[..digits].parse::<u32>().unwrap();
        assert!((1..=a.report.counts.entities).contains(&id));
    }
    for (i, &id) in a.report.correspondence.faces.iter().enumerate() {
        assert_eq!(
            a.report
                .correspondence
                .face(FaceId(u32::try_from(i).unwrap())),
            id
        );
        assert!(entities[id.unwrap().0 as usize - 1].contains("ADVANCED_FACE("));
    }
    assert!(a.report.max_deviation <= options.tolerance);
}
#[test]
fn unused_native_entries_do_not_change_exported_geometry() {
    let body = cube();
    let original = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    let mut parts = body.into_parts();
    parts.vertices.push([1000.0, 0.0, 0.0]);
    let body = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap();
    let output = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    assert_eq!(output.text, original.text);
    assert_eq!(output.report.counts, original.report.counts);
    assert_eq!(output.report.correspondence.vertex(VertexId(8)), None);
}
#[test]
fn budgets_refuse_complete_output_without_changing_the_source() {
    let body = cube();
    let before = body.clone();
    let options = StepOptions::default();
    let output = export_body(&body, Placement3::IDENTITY, &options).unwrap();
    for (entities, bytes, resource) in [
        (1, options.max_bytes, "input tables"),
        (options.max_entities, output.text.len() - 1, "bytes"),
        (
            output.report.counts.entities - 1,
            options.max_bytes,
            "entities",
        ),
    ] {
        let error = export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                max_entities: entities,
                max_bytes: bytes,
                ..options
            },
        )
        .unwrap_err();
        assert!(matches!(error,StepError::Budget {resource:r,..} if r==resource));
    }
    assert!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                max_entities: output.report.counts.entities,
                max_bytes: output.text.len(),
                ..options
            }
        )
        .is_ok()
    );
    assert_eq!(body, before);
}
#[test]
fn placement_accuracy_timestamp_and_options_are_explicit() {
    let body = cube();
    let options = StepOptions::default();
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                tolerance: f64::NAN,
                ..options
            }
        ),
        Err(StepError::InvalidOptions)
    ));
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                timestamp: "2023-02-29T00:00:00Z",
                ..options
            }
        ),
        Err(StepError::Timestamp)
    ));
    assert!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                timestamp: "2024-02-29T12:23:45.123Z",
                ..options
            }
        )
        .is_ok()
    );
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                tolerance: 1e-16,
                ..options
            }
        ),
        Err(StepError::Accuracy { .. })
    ));
    let scaled = Placement3::from_axes([2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]);
    assert!(matches!(
        export_body(&body, scaled, &options),
        Err(StepError::Placement(_))
    ));
    let placed = export_body(&body, Placement3::translate(100.0, 20.0, -3.0), &options).unwrap();
    assert!(placed.text.contains("1.00000000000000000E2"));
    assert_ne!(
        placed.text,
        export_body(&body, Placement3::IDENTITY, &options)
            .unwrap()
            .text
    );
}
#[test]
fn unsupported_supports_and_open_or_multiple_shells_have_distinct_refusals() {
    let body = cube();
    let mut parts = body.clone().into_parts();
    parts.faces.pop();
    let open = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap();
    assert!(matches!(
        export_body(&open, Placement3::IDENTITY, &StepOptions::default()),
        Err(StepError::EdgeIncidence { .. })
    ));
    let mut parts = body.clone().into_parts();
    let a = CurveSpan::new(
        Curve::line([0.0, 0.0, 4.0], [2.0, 0.0, 0.0]).unwrap(),
        ParameterRange::UNIT,
        false,
    )
    .unwrap();
    let b = CurveSpan::new(
        Curve::line([0.0, 3.0, 4.0], [2.0, 0.0, 0.0]).unwrap(),
        ParameterRange::UNIT,
        false,
    )
    .unwrap();
    parts.faces[1].surface = Surface::ruled(a, b).unwrap();
    parts.faces[1].domain = ParameterDomain {
        u: ParameterRange::UNIT,
        v: ParameterRange::UNIT,
    };
    let ruled = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap();
    assert!(matches!(
        check_geometry_support(&ruled),
        Err(StepError::UnsupportedSurface {
            face: FaceId(1),
            family: "ruled"
        })
    ));
    assert!(matches!(
        export_body(&ruled, Placement3::IDENTITY, &StepOptions::default()),
        Err(StepError::UnsupportedSurface {
            face: FaceId(1),
            family: "ruled"
        })
    ));
    let mut parts = body.clone().into_parts();
    let mut other = body.into_parts();
    for p in &mut other.vertices {
        p[0] += 10.0;
    }
    for e in &mut other.edges {
        let Curve::Line(l) = e.regular().unwrap().0 else {
            unreachable!()
        };
        *e.regular_mut().unwrap().0 = Curve::line(
            exedra_math::add(l.origin(), [10.0, 0.0, 0.0]),
            l.direction(),
        )
        .unwrap();
        e.start.0 += 8;
        e.end.0 += 8;
    }
    for l in &mut other.loops {
        for c in &mut l.coedges {
            c.edge.0 += 12;
        }
    }
    for f in &mut other.faces {
        f.surface =
            Surface::plane(Placement3::translate(10.0, 0.0, 0.0) * f.surface.frame()).unwrap();
        for l in &mut f.loops {
            l.0 += 6;
        }
    }
    parts.vertices.extend(other.vertices);
    parts.edges.extend(other.edges);
    parts.loops.extend(other.loops);
    parts.faces.extend(other.faces);
    let multiple = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap();
    assert!(matches!(
        export_body(&multiple, Placement3::IDENTITY, &StepOptions::default()),
        Err(StepError::ShellCount { found: 2 })
    ));
}

#[test]
fn structural_support_is_separate_from_final_options_and_budget_validation() {
    let body = cube();
    assert_eq!(check_geometry_support(&body), Ok(()));
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                tolerance: 0.0,
                ..StepOptions::default()
            }
        ),
        Err(StepError::InvalidOptions)
    ));
    assert!(matches!(
        export_body(
            &body,
            Placement3::IDENTITY,
            &StepOptions {
                max_bytes: 1,
                ..StepOptions::default()
            }
        ),
        Err(StepError::Budget { .. })
    ));
}

fn sphere_with_poles(seam: bool) -> Body {
    let radius = 2.0;
    let mut edges = vec![
        Edge::collapsed(VertexId(0), ()),
        Edge::collapsed(VertexId(1), ()),
    ];
    let coedges = if seam {
        edges.push(Edge::new(
            Curve::circle(
                Placement3::from_axes(
                    [0.0, 0.0, -1.0],
                    [1.0, 0.0, 0.0],
                    [0.0, -1.0, 0.0],
                    [0.0; 3],
                ),
                radius,
            )
            .unwrap(),
            ParameterRange::new(0.0, core::f64::consts::PI).unwrap(),
            VertexId(0),
            VertexId(1),
            (),
        ));
        vec![
            Coedge {
                edge: EdgeId(0),
                reversed: false,
            },
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
        ]
    } else {
        vec![Coedge {
            edge: EdgeId(0),
            reversed: false,
        }]
    };
    Body::try_from_parts(
        BodyParts {
            trims: Vec::new(),
            vertices: vec![[0.0, 0.0, -radius], [0.0, 0.0, radius]],
            edges,
            loops: vec![BoundaryLoop { coedges }],
            faces: vec![Face {
                surface: Surface::sphere(Placement3::IDENTITY, radius).unwrap(),
                domain: ParameterDomain {
                    u: ParameterRange::new(0.0, core::f64::consts::TAU).unwrap(),
                    v: ParameterRange::new(
                        -core::f64::consts::FRAC_PI_2,
                        core::f64::consts::FRAC_PI_2,
                    )
                    .unwrap(),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            }],
        },
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-10 },
    )
    .unwrap()
}
#[test]
fn certified_poles_map_to_vertices_and_regular_meridian_identity_survives() {
    for seam in [false, true] {
        let body = sphere_with_poles(seam);
        assert_eq!(check_geometry_support(&body), Ok(()));
        let file = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
        for options in [
            StepOptions {
                max_bytes: file.text.len() - 1,
                ..StepOptions::default()
            },
            StepOptions {
                max_entities: file.report.counts.entities - 1,
                ..StepOptions::default()
            },
        ] {
            assert!(matches!(
                export_body(&body, Placement3::IDENTITY, &options),
                Err(StepError::Budget { .. })
            ));
        }
        let map = &file.report.correspondence;
        assert!(file.text.contains("SPHERICAL_SURFACE("));
        assert!(!file.text.contains("UNSUPPORTED"));
        assert_eq!(
            map.edge(EdgeId(0)),
            Some(StepEdgeEntity::CollapsedVertex(
                map.vertex(VertexId(0)).unwrap()
            ))
        );
        assert_eq!(file.report.counts.edges, u32::from(seam));
        assert_eq!(file.report.counts.coedges, 2 * u32::from(seam));
        if seam {
            assert_eq!(
                map.edge(EdgeId(1)),
                Some(StepEdgeEntity::CollapsedVertex(
                    map.vertex(VertexId(1)).unwrap()
                ))
            );
            let Some(StepEdgeEntity::Curve(curve)) = map.edge(EdgeId(2)) else {
                panic!("ordinary meridian must remain an edge")
            };
            assert!(file.text.contains(&format!("#{}=EDGE_CURVE(", curve.0)));
            assert!(file.text.contains("EDGE_LOOP("));
            assert!(!file.text.contains("VERTEX_LOOP("));
        } else {
            assert_eq!(
                map.edge(EdgeId(1)),
                None,
                "detached pole stays explicitly unused"
            );
            assert!(file.text.contains("VERTEX_LOOP("));
            assert!(!file.text.contains("EDGE_CURVE("));
        }
    }
}

#[test]
fn elliptic_rims_export_shared_exact_arc_chains_with_complete_correspondence() {
    use exedra_analytic::machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder};
    let source = cube();
    let result = subtract_through_cylinder(
        &source,
        &CylinderTool::new([0.5, 0.5, 0.0], [0.1, 0.15, 1.0], 0.1).unwrap(),
        &MachiningPolicy::default(),
    )
    .unwrap();
    let document =
        export_body(&result.body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    let map = &document.report.correspondence;
    for rim in result.report.rims {
        let entity = map.edge(rim).unwrap();
        assert!(matches!(
            entity,
            StepEdgeEntity::CurveChain { count: 8, .. }
        ));
        let fragments = entity.curve_entities().collect::<Vec<_>>();
        assert_eq!(fragments.len(), 8);
        for fragment in fragments {
            assert_eq!(
                document
                    .text
                    .matches(&format!("*,*,#{},", fragment.0))
                    .count(),
                2,
                "every fragment is shared by exactly two face occurrences"
            );
            assert!(
                document
                    .text
                    .contains(&format!("#{}=EDGE_CURVE(", fragment.0))
            );
        }
    }
    assert_eq!(document.report.counts.vertices, 24);
    assert_eq!(document.report.counts.edges, 29);
    assert_eq!(document.report.counts.coedges, 58);
    assert_eq!(document.text.matches("=VERTEX_POINT(").count(), 24);
    assert_eq!(document.text.matches("=EDGE_CURVE(").count(), 29);
    assert_eq!(document.text.matches("=ORIENTED_EDGE(").count(), 58);
    assert_eq!(document.text.matches("=ELLIPSE(").count(), 2);
    // The additional exact topology participates in the ordinary atomic budget.
    let options = StepOptions {
        max_entities: document.report.counts.entities - 1,
        ..Default::default()
    };
    assert!(matches!(
        export_body(&result.body, Placement3::IDENTITY, &options),
        Err(StepError::Budget {
            resource: "entities",
            ..
        })
    ));
}

#[test]
fn ellipse_fragment_policy_tracks_physical_units_and_exact_equal_axes() {
    use exedra_analytic::machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder};
    let source = cube();
    let oblique = subtract_through_cylinder(
        &source,
        &CylinderTool::new([0.5, 0.5, 0.0], [0.1, 0.15, 1.0], 0.1).unwrap(),
        &MachiningPolicy::default(),
    )
    .unwrap();
    for (units, count) in [(LengthUnit::Millimetre, 8), (LengthUnit::Metre, 16)] {
        let doc = export_body(
            &oblique.body,
            Placement3::IDENTITY,
            &StepOptions {
                units,
                ..Default::default()
            },
        )
        .unwrap();
        for rim in oblique.report.rims {
            assert!(
                matches!(doc.report.correspondence.edge(rim),Some(StepEdgeEntity::CurveChain{count:n,..}) if n==count)
            );
        }
    }
    let axial = subtract_through_cylinder(
        &source,
        &CylinderTool::new([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], 0.1).unwrap(),
        &MachiningPolicy::default(),
    )
    .unwrap();
    let mut parts = axial.body.into_parts();
    // Exercise the exact ellipse identity even when a constructor selected Circle.
    for rim in axial.report.rims {
        let (curve, _) = parts.edges[rim.index()].regular_mut().unwrap();
        if let Curve::Circle(c) = curve {
            *curve = Curve::ellipse(c.frame(), c.radius(), c.radius()).unwrap();
        }
    }
    let body = Body::try_from_parts(
        parts,
        &exedra_analytic::body::ValidationPolicy { max_gap: 1e-9 },
    )
    .unwrap();
    let doc = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
    for rim in axial.report.rims {
        assert!(matches!(
            doc.report.correspondence.edge(rim),
            Some(StepEdgeEntity::Curve(_))
        ));
    }
    assert_eq!(doc.text.matches("=ELLIPSE(").count(), 0);
    assert_eq!(doc.text.matches("=CIRCLE(").count(), 2);
    assert_eq!(doc.report.counts.edges, 15);
    assert_eq!(doc.report.counts.vertices, 10);
}
