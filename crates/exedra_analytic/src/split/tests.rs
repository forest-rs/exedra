// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    body::{
        Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, Sense, ValidationPolicy,
        VertexId,
    },
    geometry::{Curve, CurveSpan, ParameterDomain, ParameterRange, Surface},
    realize::{RealizationPolicy, VertexSource},
};
use alloc::{vec, vec::Vec};
use exedra_math::Placement3;

#[test]
fn bounded_ruled_face_keeps_original_support_and_native_parameters() {
    let source = |z| Curve::line([0.0, 0.0, z], [4.0, 0.0, 0.0]).unwrap();
    let support = Surface::ruled(
        CurveSpan::new(source(0.0), ParameterRange::UNIT, false).unwrap(),
        CurveSpan::new(source(1.0), ParameterRange::UNIT, false).unwrap(),
    )
    .unwrap();
    let interval = ParameterRange::new(0.25, 0.75).unwrap();
    let points = vec![
        [1.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [3.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
    ];
    let edge = |curve, range, a, b| Edge::new(curve, range, VertexId(a), VertexId(b), ());
    let body = Body::try_from_parts(
        BodyParts {
            edges: vec![
                edge(source(0.0), interval, 0, 1),
                edge(
                    Curve::line(points[1], [0.0, 0.0, 1.0]).unwrap(),
                    ParameterRange::UNIT,
                    1,
                    2,
                ),
                edge(source(1.0), interval, 3, 2),
                edge(
                    Curve::line(points[0], [0.0, 0.0, 1.0]).unwrap(),
                    ParameterRange::UNIT,
                    0,
                    3,
                ),
            ],
            vertices: points,
            loops: vec![BoundaryLoop {
                coedges: (0..4)
                    .map(|i| Coedge {
                        edge: EdgeId(i),
                        reversed: i >= 2,
                    })
                    .collect(),
            }],
            faces: vec![Face {
                surface: support.clone(),
                domain: ParameterDomain {
                    u: interval,
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            }],
            trims: vec![],
        },
        &ValidationPolicy { max_gap: 1e-9 },
    )
    .unwrap();
    assert_eq!(body.faces()[0].surface, support);
    let output = body
        .realize(Placement3::IDENTITY, &RealizationPolicy::default())
        .unwrap();
    assert!(output.report.within_tolerance());
    for (_, source) in output.mesh_vertices {
        if let VertexSource::Face { uv, .. } = source {
            assert!((0.25..=0.75).contains(&uv[0]));
        }
    }
}

#[test]
fn native_ruled_split_shares_edges_keeps_support_and_composes() {
    use super::{RuledSplitPolicy, split_ruled_face_at_u};
    let source = crate::nurbs_tests::prism();
    let before = source.clone();
    let original = source.faces()[2].surface.clone();
    let result = split_ruled_face_at_u(
        &source,
        crate::body::FaceId(2),
        0.37,
        (),
        &RuledSplitPolicy::default(),
    )
    .unwrap();
    assert_eq!(source, before, "input remains unchanged");
    assert_eq!(result.body.vertices().len(), source.vertices().len() + 2);
    assert_eq!(result.body.edges().len(), source.edges().len() + 3);
    assert_eq!(result.body.faces().len(), source.faces().len() + 1);
    assert!(result.body.is_closed());
    for child in result.report.faces {
        assert_eq!(result.body.face(child.face).unwrap().surface, original);
        assert_eq!(result.body.face(child.face).unwrap().domain, child.domain);
    }
    for split in result.report.rims {
        let (curve, _) = source.edge(split.source).unwrap().regular().unwrap();
        for (child, range) in split.children.into_iter().zip(split.ranges) {
            let (result_curve, result_range) = result.body.edge(child).unwrap().regular().unwrap();
            assert_eq!(
                result_curve, curve,
                "defining controls and weights stay unchanged"
            );
            assert_eq!(result_range, range);
        }
    }
    let positions = source.vertices().to_vec();
    assert_eq!(&result.body.vertices()[..positions.len()], &positions);
    for tolerance in [0.02, 0.002] {
        let output = result
            .body
            .realize(
                Placement3::rotate_y_then_translate(0.2, 3., 4., 5.),
                &RealizationPolicy {
                    tolerance,
                    quality: Some(crate::realize::TriangleQuality::new(0.2)),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(output.report.within_tolerance(), "{:?}", output.report);
        assert!(output.mesh.boundary_loops().unwrap().is_empty());
        assert!(output.mesh.validate_deep().is_empty());
        let shared = output.edge_chains.get(result.report.ruling).unwrap();
        assert!(shared.len() >= 2);
        for &(vertex, origin) in &output.mesh_vertices {
            if let VertexSource::Face { face, uv } = origin {
                assert!(result.body.face(face).unwrap().domain.contains(uv));
                let expected = Placement3::rotate_y_then_translate(0.2, 3., 4., 5.)
                    .transform_point(result.body.evaluate_support(face, uv).unwrap().point);
                let actual = output.mesh.vertex_position(vertex).unwrap().map(f64::from);
                assert!(
                    exedra_math::norm(exedra_math::sub(expected, actual))
                        <= output.report.max_vertex_displacement + 1e-10
                );
            }
        }
    }
    let second = split_ruled_face_at_u(
        &result.body,
        result.report.faces[1].face,
        0.71,
        (),
        &RuledSplitPolicy::default(),
    )
    .unwrap();
    assert_eq!(
        second.body.faces()[second.report.faces[0].face.index()].surface,
        original
    );
    let mesh = second
        .body
        .realize(Placement3::IDENTITY, &RealizationPolicy::default())
        .unwrap();
    assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
    assert!(mesh.report.within_tolerance());
}

#[test]
fn source_proof_budgets_admit_exact_work_and_refuse_without_mutating_input() {
    use super::{RuledSplitError as Error, RuledSplitPolicy, split_ruled_face_at_u};
    use crate::body::FaceId;
    let source = crate::nurbs_tests::prism();
    let before = source.clone();
    let result =
        split_ruled_face_at_u(&source, FaceId(2), 0.37, (), &RuledSplitPolicy::default()).unwrap();
    let work = result.report.work;
    let total =
        work.source_metadata_work + work.source_preparation_work + work.source_enclosure_work;
    assert!(
        work.source_controls > 0
            && work.source_preparation_work > 0
            && work.source_enclosure_work > 0
    );
    let exact = RuledSplitPolicy {
        max_source_controls: work.source_controls,
        max_source_work: total,
        ..Default::default()
    };
    assert!(split_ruled_face_at_u(&source, FaceId(2), 0.37, (), &exact).is_ok());
    for policy in [
        RuledSplitPolicy {
            max_source_controls: 0,
            ..exact
        },
        RuledSplitPolicy {
            max_source_work: 0,
            ..exact
        },
        RuledSplitPolicy {
            max_source_work: total - 1,
            ..exact
        },
    ] {
        assert!(
            matches!(split_ruled_face_at_u(&source,FaceId(2),0.37,(),&policy),Err(Error::SourceBudget { required,maximum,.. }) if required>maximum)
        );
        assert_eq!(source, before);
    }
}

#[test]
fn refusals_are_contextual_and_transactional() {
    use super::{RuledSplitError as Error, RuledSplitPolicy, split_ruled_face_at_u};
    use crate::body::FaceId;
    let source = crate::nurbs_tests::prism();
    let before = source.clone();
    let policy = RuledSplitPolicy::default();
    for u in [f64::NAN, -0.1, 0.0, 1.0, 0.5 * policy.parameter_clearance] {
        assert!(matches!(
            split_ruled_face_at_u(&source, FaceId(2), u, (), &policy),
            Err(Error::Parameter { .. })
        ));
    }
    assert!(matches!(
        split_ruled_face_at_u(&source, FaceId(999), 0.5, (), &policy),
        Err(Error::MissingFace { face: FaceId(999) })
    ));
    assert!(matches!(
        split_ruled_face_at_u(&source, FaceId(0), 0.5, (), &policy),
        Err(Error::UnsupportedFace { .. })
    ));
    assert!(matches!(
        split_ruled_face_at_u(
            &source,
            FaceId(2),
            0.5,
            (),
            &RuledSplitPolicy {
                vertex_clearance: f64::NAN,
                ..policy
            }
        ),
        Err(Error::InvalidPolicy)
    ));
    let mut limited = policy;
    limited.validation.max_coedges = 1;
    assert!(matches!(
        split_ruled_face_at_u(&source, FaceId(2), 0.5, (), &limited),
        Err(Error::Budget {
            resource: "input coedges",
            ..
        })
    ));
    limited = policy;
    limited.validation.max_pair_tests = 64;
    assert!(matches!(
        split_ruled_face_at_u(&source, FaceId(2), 0.5, (), &limited),
        Err(Error::Budget {
            resource: "output trim pair work",
            ..
        })
    ));
    assert!(matches!(
        split_ruled_face_at_u(
            &source,
            FaceId(2),
            1e-4,
            (),
            &RuledSplitPolicy {
                vertex_clearance: 0.1,
                ..policy
            }
        ),
        Err(Error::NearVertex { .. })
    ));
    assert_eq!(source, before);
}

#[test]
fn caller_roles_and_explicit_planar_trim_occurrences_are_preserved() {
    use super::{RuledSplitPolicy, split_ruled_face_at_u};
    use crate::body::{FaceId, TrimCurve};
    let source = crate::nurbs_tests::prism();
    let mut parts = source.into_parts();
    crate::operation_trims::append_planar_loop(&mut parts, FaceId(0), LoopId(0)).unwrap();
    crate::operation_trims::append_planar_loop(&mut parts, FaceId(1), LoopId(1)).unwrap();
    let source = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 })
        .unwrap()
        .map_roles(|_| 17_u32, |_| 23_u32);
    let result =
        split_ruled_face_at_u(&source, FaceId(2), 0.38, 99, &RuledSplitPolicy::default()).unwrap();
    assert_eq!(result.body.edge(result.report.ruling).unwrap().role, 99);
    assert!(result.body.faces().iter().all(|face| face.role == 17));
    for rim in result.report.rims {
        assert!(
            rim.children
                .iter()
                .all(|&id| result.body.edge(id).unwrap().role == 23)
        );
        let trims = result
            .body
            .trims()
            .iter()
            .find(|t| t.loop_id == rim.neighbor_loop)
            .unwrap();
        for (co, trim) in result.body.loops()[rim.neighbor_loop.index()]
            .coedges
            .iter()
            .zip(&trims.curves)
        {
            if let Some(child) = rim.children.iter().position(|&id| id == co.edge) {
                let TrimCurve::Nurbs { range, curve } = trim else {
                    panic!("spline trim retained");
                };
                assert_eq!(*range, rim.ranges[child]);
                let Curve::Nurbs(original) = source.edge(rim.source).unwrap().regular().unwrap().0
                else {
                    unreachable!()
                };
                assert_eq!(curve.weights(), original.curve().weights());
                assert_eq!(curve.knots(), original.curve().knots());
            }
        }
    }
    assert_eq!(result.body.trims().len(), source.trims().len());
}

fn reversed_nonunit_prism(
    holes: Vec<Vec<crate::planar_extrusion::PlanarSegment>>,
) -> (Body<(), usize>, crate::body::FaceId) {
    use crate::planar_extrusion::{PlanarExtrusion, PlanarExtrusionPolicy, PlanarSegment};
    use exedra_curves::profile::SegTag;
    let curve = Curve::nurbs(
        crate::geometry::NurbsCurve::new(
            4,
            vec![2., 2., 2., 2., 5., 5., 5., 5.],
            vec![[0., 0., 0.], [1., 2., 0.], [3., 2., 0.], [4., 0., 0.]],
            Some(vec![1.25, 0.7, 1.4, 1.5]),
        )
        .unwrap(),
    )
    .unwrap();
    let outline = vec![
        PlanarSegment::line([0., 0.], [4., 0.]).unwrap(),
        PlanarSegment::new(
            CurveSpan::new(curve, ParameterRange::new(2., 5.).unwrap(), true).unwrap(),
        )
        .tagged(SegTag(7)),
    ];
    let solid = PlanarExtrusion::new(
        outline,
        holes,
        Placement3::IDENTITY,
        2.,
        &PlanarExtrusionPolicy::default(),
    )
    .unwrap();
    let crown = solid.faces_tagged(SegTag(7)).next().unwrap().0;
    let source = solid.into_body().map_roles(|_| (), |_| 7_usize);
    (source, crown)
}

#[test]
fn reversed_nonunit_generators_and_line_walls_keep_original_supports() {
    use super::{RuledSplitPolicy, split_ruled_face_at_u};
    let (source, crown) = reversed_nonunit_prism(vec![]);
    let original = source.faces()[crown.index()].surface.clone();
    let first =
        split_ruled_face_at_u(&source, crown, 0.37, 99, &RuledSplitPolicy::default()).unwrap();
    for rim in first.report.rims {
        assert_eq!(
            rim.parameter,
            ParameterRange::new(2., 5.).unwrap().at(1.0 - 0.37).unwrap()
        );
        assert_eq!(rim.ranges[0].bounds(), [rim.parameter, 5.]);
        assert_eq!(rim.ranges[1].bounds(), [2., rim.parameter]);
    }
    let second = split_ruled_face_at_u(
        &first.body,
        first.report.faces[1].face,
        0.73,
        99,
        &RuledSplitPolicy::default(),
    )
    .unwrap();
    for rim in second.report.rims {
        assert_eq!(
            rim.parameter,
            ParameterRange::new(2., 5.).unwrap().at(1.0 - 0.73).unwrap()
        );
    }
    for child in second.report.faces {
        assert_eq!(second.body.faces()[child.face.index()].surface, original);
    }
    for body in [&first.body, &second.body] {
        let mesh = body
            .realize(Placement3::IDENTITY, &RealizationPolicy::default())
            .unwrap();
        assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
        assert!(mesh.report.within_tolerance());
    }
    // The other wall has line generators and remains independently splittable.
    let line_split = split_ruled_face_at_u(
        &source,
        crate::body::FaceId(2),
        0.41,
        99,
        &RuledSplitPolicy::default(),
    )
    .unwrap();
    assert!(line_split.body.is_closed());
    let output = line_split
        .body
        .realize(Placement3::IDENTITY, &RealizationPolicy::default())
        .unwrap();
    assert!(output.mesh.boundary_loops().unwrap().is_empty());
}

#[test]
fn swapped_rows_preserve_hole_records_and_outward_geometry_across_splits() {
    use super::{RuledSplitPolicy, split_ruled_face_at_u};
    use crate::{body::FaceId, planar_extrusion::PlanarSegment};
    let points = [[1.5, 0.3], [1.5, 0.6], [2.5, 0.6], [2.5, 0.3]];
    let hole = (0..4)
        .map(|i| PlanarSegment::line(points[i], points[(i + 1) % 4]).unwrap())
        .collect();
    let (original, crown) = reversed_nonunit_prism(vec![hole]);
    let mut parts = original.clone().into_parts();
    let Surface::Ruled(support) = &parts.faces[crown.index()].surface else {
        unreachable!("tagged spline wall");
    };
    let [below, above] = support.curves().clone();
    assert!(below.reversed() && above.reversed());
    assert_eq!(below.range().bounds(), [2., 5.]);
    parts.faces[crown.index()].surface = Surface::ruled(above, below).unwrap();
    assert_eq!(parts.faces[crown.index()].sense, Sense::Forward);
    parts.faces[crown.index()].sense = Sense::Reverse;
    let source = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
    for uv in [[0., 0.], [0.37, 0.23], [0.71, 0.81], [1., 1.]] {
        let before = original
            .evaluate_support(crown, [uv[0], 1. - uv[1]])
            .unwrap();
        let after = source.evaluate_support(crown, uv).unwrap();
        assert!(exedra_math::norm(exedra_math::sub(before.point, after.point)) < 1e-12);
        assert!(exedra_math::norm(exedra_math::sub(before.normal, after.normal)) < 1e-12);
    }
    assert_eq!(
        source.loops(),
        original.loops(),
        "physical boundary is unchanged"
    );

    let cap_holes: Vec<_> = [FaceId(0), FaceId(1)]
        .map(|id| {
            let face = source.face(id).unwrap();
            assert_eq!(face.loops.len(), 2);
            face.loops[1]
        })
        .into();
    let hole_vertices: Vec<_> = cap_holes
        .iter()
        .flat_map(|&id| &source.boundary_loop(id).unwrap().coedges)
        .flat_map(|co| {
            let edge = source.edge(co.edge).unwrap();
            [edge.start, edge.end]
        })
        .collect();
    let hole_edges: Vec<_> = source
        .edges()
        .iter()
        .enumerate()
        .filter(|(_, edge)| {
            hole_vertices.contains(&edge.start) && hole_vertices.contains(&edge.end)
        })
        .map(|(i, _)| EdgeId(crate::usize_to_u32(i)))
        .collect();
    assert_eq!(hole_edges.len(), 12, "both rims and all four hole rulings");
    let hole_loops: Vec<_> = source
        .loops()
        .iter()
        .enumerate()
        .filter(|(_, ring)| ring.coedges.iter().all(|co| hole_edges.contains(&co.edge)))
        .map(|(i, _)| LoopId(crate::usize_to_u32(i)))
        .collect();
    assert_eq!(hole_loops.len(), 6, "two cap holes and four hole walls");
    let hole_trims: Vec<_> = source
        .trims()
        .iter()
        .filter(|trims| hole_loops.contains(&trims.loop_id))
        .collect();
    assert_eq!(hole_trims.len(), 2, "both cap holes have explicit trims");
    let before = source.clone();
    let policy = RuledSplitPolicy::default();
    let first = split_ruled_face_at_u(&source, crown, 0.37, 99, &policy).unwrap();
    let first_before = first.body.clone();
    let second =
        split_ruled_face_at_u(&first.body, first.report.faces[1].face, 0.73, 99, &policy).unwrap();
    assert_eq!(source, before, "first edit borrows the original body");
    assert_eq!(
        first.body, first_before,
        "repeat edit borrows the first result"
    );
    for result in [&first, &second] {
        for &id in &hole_edges {
            assert_eq!(result.body.edge(id), source.edge(id));
        }
        for &id in &hole_loops {
            assert_eq!(result.body.boundary_loop(id), source.boundary_loop(id));
        }
        for trims in &hole_trims {
            assert_eq!(
                result
                    .body
                    .trims()
                    .iter()
                    .find(|t| t.loop_id == trims.loop_id),
                Some(*trims)
            );
        }
        for cap in [FaceId(0), FaceId(1)] {
            assert_eq!(
                result.body.face(cap).unwrap().loops,
                source.face(cap).unwrap().loops
            );
        }
        let ruling = result.body.edge(result.report.ruling).unwrap();
        assert_eq!(ruling.start, result.report.rims[0].vertex);
        assert_eq!(ruling.end, result.report.rims[1].vertex);
        assert_eq!(result.body.vertices()[ruling.start.index()][2], 2.);
        assert_eq!(result.body.vertices()[ruling.end.index()][2], 0.);
        let occurrences: Vec<_> = result
            .body
            .loops()
            .iter()
            .flat_map(|ring| &ring.coedges)
            .filter(|co| co.edge == result.report.ruling)
            .collect();
        assert_eq!(occurrences.len(), 2);
        assert_ne!(occurrences[0].reversed, occurrences[1].reversed);
        for fragment in result.report.faces {
            assert_eq!(
                result.body.face(fragment.face).unwrap().surface,
                source.face(crown).unwrap().surface
            );
            assert_eq!(
                result.body.face(fragment.face).unwrap().sense,
                Sense::Reverse
            );
        }
        for tolerance in [0.02, 0.002] {
            let output = result
                .body
                .realize(
                    Placement3::IDENTITY,
                    &RealizationPolicy {
                        tolerance,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(output.report.within_tolerance(), "{:?}", output.report);
            assert!(output.mesh.boundary_loops().unwrap().is_empty());
            assert!(output.mesh.validate_deep().is_empty());
            for &(vertex, origin) in &output.mesh_vertices {
                if let VertexSource::Face { face, uv } = origin {
                    assert!(result.body.face(face).unwrap().domain.contains(uv));
                    let expected = result.body.evaluate_support(face, uv).unwrap().point;
                    let actual = output.mesh.vertex_position(vertex).unwrap().map(f64::from);
                    assert!(
                        exedra_math::norm(exedra_math::sub(expected, actual))
                            <= output.report.max_vertex_displacement + 1e-10
                    );
                }
            }
        }
    }
}

#[test]
fn detached_rim_occurrences_are_refused_without_rewriting_any_input() {
    use super::{RuledSplitError, RuledSplitPolicy, split_ruled_face_at_u};
    use crate::body::FaceId;
    let source = crate::nurbs_tests::prism();
    let mut parts = source.into_parts();
    let wall_loop = parts.faces[2].loops[0];
    // Detached records are explicitly admitted for stable edit IDs. This extra
    // occurrence is not claimed to be another embedded face or another solid.
    parts.loops.push(parts.loops[wall_loop.index()].clone());
    let source = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
    let policy = RuledSplitPolicy::default();
    source.validate_boundary(&policy.validation).unwrap();
    let before = source.clone();
    assert!(matches!(
        split_ruled_face_at_u(&source, FaceId(2), 0.37, (), &policy),
        Err(RuledSplitError::UnsupportedRim {
            edge: EdgeId(0),
            reason: "rim is also referenced by detached or repeated loop occurrences",
        })
    ));
    assert_eq!(source, before);
}

#[test]
fn inserted_ruling_is_shared_and_smooth_in_the_emitted_mesh() {
    use super::{RuledSplitPolicy, split_ruled_face_at_u};
    let source = crate::nurbs_tests::prism();
    let result = split_ruled_face_at_u(
        &source,
        crate::body::FaceId(2),
        0.37,
        (),
        &RuledSplitPolicy::default(),
    )
    .unwrap();
    let output = result
        .body
        .realize(Placement3::IDENTITY, &RealizationPolicy::default())
        .unwrap();
    let chain = output.edge_chains.get(result.report.ruling).unwrap();
    for pair in chain.windows(2) {
        let edge = output
            .mesh
            .half_edges()
            .find(|&edge| {
                let a = output.mesh.from_vertex(edge).unwrap().index();
                let b = output.mesh.to_vertex(edge).unwrap().index();
                (a == pair[0].index() && b == pair[1].index())
                    || (a == pair[1].index() && b == pair[0].index())
            })
            .unwrap();
        assert_eq!(
            output.mesh.edge_sharpness(edge),
            Some(0.0),
            "same supporting surface must not gain a shading crease"
        );
    }
}

#[test]
fn conic_rims_and_nonplanar_rim_neighbors_are_explicit_refusals() {
    use super::{RuledSplitError, RuledSplitPolicy, split_ruled_face_at_u};
    use crate::loft::{Loft, LoftPolicy};
    use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};
    let policy = ProfilePolicy::new(1e-9);
    let circle = Profile::new(
        Loop::new(vec![
            Segment::arc([-1., 0.], 1.),
            Segment::arc([1., 0.], 1.),
        ])
        .unwrap(),
        vec![],
        &policy,
    )
    .unwrap();
    let loft = Loft::from_profiles(
        [
            (Placement3::IDENTITY, &circle),
            (Placement3::translate(0., 0., 1.), &circle),
        ],
        Placement3::IDENTITY,
        &LoftPolicy::new(policy),
    )
    .unwrap();
    let face = crate::body::FaceId(
        u32::try_from(
            loft.body()
                .faces()
                .iter()
                .position(|f| matches!(f.surface, Surface::Ruled(_)))
                .unwrap(),
        )
        .unwrap(),
    );
    assert!(matches!(
        split_ruled_face_at_u(
            loft.body(),
            face,
            0.5,
            loft.body().edges()[0].role,
            &RuledSplitPolicy::default()
        ),
        Err(RuledSplitError::UnsupportedRim { .. })
    ));
    let rectangle = Profile::new(
        Loop::new(vec![
            Segment::line([1., 0.]),
            Segment::line([1., 1.]),
            Segment::line([0., 1.]),
            Segment::line([0., 0.]),
        ])
        .unwrap(),
        vec![],
        &policy,
    )
    .unwrap();
    let loft = Loft::from_profiles(
        [
            (Placement3::IDENTITY, &rectangle),
            (Placement3::translate(0., 0., 1.), &rectangle),
            (Placement3::translate(0., 0., 2.), &rectangle),
        ],
        Placement3::IDENTITY,
        &LoftPolicy::new(policy),
    )
    .unwrap();
    let face = crate::body::FaceId(
        u32::try_from(
            loft.body()
                .faces()
                .iter()
                .position(|f| matches!(f.surface, Surface::Ruled(_)))
                .unwrap(),
        )
        .unwrap(),
    );
    assert!(matches!(
        split_ruled_face_at_u(
            loft.body(),
            face,
            0.5,
            loft.body().edges()[0].role,
            &RuledSplitPolicy::default()
        ),
        Err(RuledSplitError::UnsupportedRim { .. })
    ));
}

#[test]
fn partial_ruled_admission_rejects_cancelling_actual_line_coefficients() {
    use crate::body::BodyError;
    // Exact original coefficients disagree by about 6.4 model units, despite
    // rounded evaluation producing the same 0/16 endpoints as the ideal rail.
    let direction = 2.0_f64.powi(60) + 256.;
    let start = 0.1_f64;
    let end = start.next_up();
    let origin = -(direction * start);
    let actual = Curve::line([0.8, origin, 0.], [0., direction, 0.]).unwrap();
    assert_eq!(actual.evaluate(start).unwrap().point, [0.8, 0., 0.]);
    assert_eq!(actual.evaluate(end).unwrap().point, [0.8, 16., 0.]);
    let source = |y| {
        CurveSpan::new(
            Curve::line([0., y, 0.], [1., 0., 0.]).unwrap(),
            ParameterRange::UNIT,
            false,
        )
        .unwrap()
    };
    let points = vec![[0.2, 0., 0.], [0.8, 0., 0.], [0.8, 16., 0.], [0.2, 16., 0.]];
    let edges = vec![
        Edge::new(
            source(0.).curve().clone(),
            ParameterRange::new(0.2, 0.8).unwrap(),
            VertexId(0),
            VertexId(1),
            (),
        ),
        Edge::new(
            actual,
            ParameterRange::new(start, end).unwrap(),
            VertexId(1),
            VertexId(2),
            (),
        ),
        Edge::new(
            source(16.).curve().clone(),
            ParameterRange::new(0.2, 0.8).unwrap(),
            VertexId(3),
            VertexId(2),
            (),
        ),
        Edge::new(
            Curve::line(points[0], [0., 16., 0.]).unwrap(),
            ParameterRange::UNIT,
            VertexId(0),
            VertexId(3),
            (),
        ),
    ];
    let result = Body::try_from_parts(
        BodyParts {
            vertices: points,
            edges,
            loops: vec![BoundaryLoop {
                coedges: (0..4)
                    .map(|edge| Coedge {
                        edge: EdgeId(edge),
                        reversed: edge >= 2,
                    })
                    .collect(),
            }],
            faces: vec![Face {
                surface: Surface::ruled(source(0.), source(16.)).unwrap(),
                domain: ParameterDomain {
                    u: ParameterRange::new(0.2, 0.8).unwrap(),
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            }],
            trims: vec![],
        },
        &ValidationPolicy { max_gap: 1e-9 },
    );
    match result {
        Err(BodyError::SurfaceGap {
            edge: EdgeId(1),
            bound,
            ..
        }) => assert!(bound > 1.),
        Err(BodyError::EndpointGap {
            edge: EdgeId(1),
            gap,
            ..
        }) => assert!(gap > 1.),
        other => panic!("cancelled native endpoint must refuse: {other:?}"),
    }
}

#[test]
fn split_rejects_cancelling_global_rail_when_admitting_partial_children() {
    use super::{RuledSplitError, RuledSplitPolicy, split_ruled_face_at_u};
    use crate::{
        body::{BodyError, EdgeGeometry, FaceId},
        planar_extrusion::{PlanarExtrusion, PlanarExtrusionPolicy, PlanarSegment},
        validation::BoundaryValidationError,
    };
    let points = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
    let outer = (0..4)
        .map(|i| PlanarSegment::line(points[i], points[(i + 1) % 4]).unwrap())
        .collect();
    let original = PlanarExtrusion::new(
        outer,
        vec![],
        Placement3::IDENTITY,
        16.,
        &PlanarExtrusionPolicy::default(),
    )
    .unwrap()
    .into_body()
    .map_roles(|_| (), |_| ());
    let mut parts = original.into_parts();
    let rail = parts
        .edges
        .iter()
        .position(|edge| {
            parts.vertices[edge.start.index()] == [1., 0., 0.]
                && parts.vertices[edge.end.index()] == [1., 0., 16.]
        })
        .unwrap();
    let direction = 2.0_f64.powi(60) + 256.;
    let start = 0.1_f64;
    parts.edges[rail].geometry = EdgeGeometry::Curve {
        curve: Curve::line([1., 0., -(direction * start)], [0., 0., direction]).unwrap(),
        range: ParameterRange::new(start, start.next_up()).unwrap(),
    };
    // Deliberately assemble malformed raw input. The public transaction must
    // refuse either initial admission or its partial children; this does not
    // codify the older full-domain precision miss as desired behavior.
    let body = Body { parts };
    let before = body.clone();
    let result = split_ruled_face_at_u(&body, FaceId(2), 0.37, (), &RuledSplitPolicy::default());
    match result {
        Err(
            RuledSplitError::OutputBoundary(BoundaryValidationError::Geometry(
                BodyError::SurfaceGap { edge, bound, .. },
            ))
            | RuledSplitError::InputBoundary(BoundaryValidationError::Geometry(
                BodyError::SurfaceGap { edge, bound, .. },
            )),
        ) => assert!(edge.index() == rail && bound > 1.),
        Err(RuledSplitError::InputBoundary(BoundaryValidationError::Geometry(
            BodyError::EndpointGap { edge, gap, .. },
        ))) => assert!(edge.index() == rail && gap > 1.),
        other => panic!("cancelled native rail must refuse: {other:?}"),
    }
    assert_eq!(body, before, "refused transaction leaves input unchanged");
}
