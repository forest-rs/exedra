// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Narrow adapters from the authoring graph to retained analytic operations.
use super::*;
use exedra_analytic::{
    body::{Body, EdgeGeometry, ValidationPolicy},
    geometry::{Curve, Surface},
    halfspace::{ClipError, ClipOutcome, ClipPolicy, HalfSpace, intersect_half_space_with_roles},
    rectilinear::{
        BooleanOperation, Operand, RectilinearError, RectilinearPolicy, boolean_rectilinear,
    },
};

fn operand(value: Operand) -> &'static str {
    match value {
        Operand::Left => "left",
        Operand::Right => "right",
    }
}
fn history(model: &Model) -> Vec<AuthoredSolidOperation> {
    if let Model::Body { operations, .. } = model {
        operations.clone()
    } else {
        Vec::new()
    }
}
fn operation_name(operation: BooleanOperation) -> &'static str {
    match operation {
        BooleanOperation::Union => "solid.union",
        BooleanOperation::Intersection => "solid.intersect",
        BooleanOperation::Difference => "solid.subtract",
    }
}

/// Translates plane/line tables while retaining their exact parameterization.
/// Different orientation matrices are refused; no near-axis snapping occurs.
fn right_in_left_frame(
    left: Placement3,
    right: Placement3,
    body: Body<Vec<SegTag>>,
    gap: f64,
    operation: &'static str,
    node: usize,
) -> Result<Body<Vec<SegTag>>, Error> {
    if left == right {
        return Ok(body);
    }
    if (0..3).any(|row| (0..3).any(|column| left.rows[row][column] != right.rows[row][column])) {
        let mut error=Error::new("boolean.frameMismatch",operation,"Boolean operands must have exactly equal orientation matrices; different translations are supported").node(node);
        error.context_mut().operand = Some("right");
        return Err(error);
    }
    let delta = core::array::from_fn::<_, 3, _>(|i| right.rows[i][3] - left.rows[i][3]);
    // Both frames are rigid and have identical orientation. This is R^T*(tr-tl).
    let offset = core::array::from_fn::<_, 3, _>(|i| {
        (0..3).map(|j| left.rows[j][i] * delta[j]).sum::<f64>()
    });
    let translate = |p: [f64; 3]| core::array::from_fn(|i| p[i] + offset[i]);
    let mut parts = body.into_parts();
    for point in &mut parts.vertices {
        *point = translate(*point);
    }
    for edge in &mut parts.edges {
        if let EdgeGeometry::Curve {
            curve: Curve::Line(line),
            range,
        } = &edge.geometry
        {
            let range = *range;
            edge.geometry = EdgeGeometry::Curve {
                curve: Curve::line(translate(line.origin()), line.direction()).map_err(|e| {
                    Error::new("boolean.numericLimit", operation, e.to_string()).node(node)
                })?,
                range,
            };
        }
    }
    for (id, face) in parts.faces.iter_mut().enumerate() {
        if !matches!(face.surface, Surface::Plane(_))
            || face.loops.iter().any(|id| {
                parts.loops[id.index()].coedges.iter().any(|coedge| {
                    !matches!(
                        parts.edges[coedge.edge.index()].regular(),
                        Some((Curve::Line(_), _))
                    )
                })
            })
        {
            let mut error = Error::new(
                "boolean.unsupportedFace",
                operation,
                "Translated Boolean input requires planar faces and line edges",
            )
            .node(node);
            let context = error.context_mut();
            context.operand = Some("right");
            context.face = Some(u32::try_from(id).expect("native face IDs fit u32"));
            return Err(error);
        }
        let mut frame = face.surface.frame();
        let origin = translate(frame.rows.map(|row| row[3]));
        for (row, value) in frame.rows.iter_mut().zip(origin) {
            row[3] = value;
        }
        face.surface = Surface::plane(frame)
            .map_err(|e| Error::new("boolean.numericLimit", operation, e.to_string()).node(node))?;
    }
    Body::try_from_parts(parts, &ValidationPolicy { max_gap: gap })
        .map_err(|e| Error::new("boolean.boundary", operation, e.to_string()).node(node))
}

pub(super) fn boolean(
    models: &[Model],
    left: u32,
    right: u32,
    operation: BooleanOperation,
    gap: f64,
    node: usize,
) -> Result<Model, Error> {
    let name = operation_name(operation);
    let a = &models[left as usize];
    let b = &models[right as usize];
    let left_body = a.geometry().to_body();
    let right_body = right_in_left_frame(
        a.geometry().frame(),
        b.geometry().frame(),
        b.geometry().to_body(),
        gap,
        name,
        node,
    )?;
    let refusal = |error: RectilinearError| {
        let code = match &error {
            RectilinearError::InvalidPolicy => "boolean.policy",
            RectilinearError::Boundary { .. } => "boolean.boundary",
            RectilinearError::OpenInput { .. } => "boolean.openInput",
            RectilinearError::UnsupportedFace { .. } => "boolean.unsupportedFace",
            RectilinearError::PlaneSpacing { .. } => "boolean.planeSpacing",
            RectilinearError::CellBudget { .. } => "boolean.cellBudget",
            RectilinearError::FaceTestBudget { .. } => "boolean.faceTestBudget",
            RectilinearError::InvalidMaterial { .. } => "boolean.invalidMaterial",
            RectilinearError::NonManifoldInput { .. } | RectilinearError::NonManifold => {
                "boolean.nonManifold"
            }
            _ => "boolean.numericLimit",
        };
        let mut diagnostic = Error::new(code, name, error.to_string()).node(node);
        let context = diagnostic.context_mut();
        match error {
            RectilinearError::Boundary {
                operand: source, ..
            } => context.operand = source.map(operand),
            RectilinearError::OpenInput { operand: source }
            | RectilinearError::NonManifoldInput { operand: source } => {
                context.operand = Some(operand(source));
            }
            RectilinearError::UnsupportedFace {
                operand: source,
                face,
            } => {
                context.operand = Some(operand(source));
                context.face = Some(face.0);
            }
            RectilinearError::InvalidMaterial {
                operand: source,
                cell,
            } => {
                context.operand = Some(operand(source));
                context.cell = Some(cell);
            }
            RectilinearError::CellBudget { required, maximum }
            | RectilinearError::FaceTestBudget { required, maximum } => {
                context.required = required;
                context.maximum = Some(maximum);
            }
            RectilinearError::PlaneSpacing { axis, required, .. } => {
                context.axis = Some(axis);
                context.minimum_separation = Some(required);
            }
            _ => {}
        }
        diagnostic
    };
    let result = boolean_rectilinear(
        &left_body,
        &right_body,
        operation,
        &RectilinearPolicy {
            max_cells: 32_768,
            max_face_tests: 4_000_000,
            minimum_spacing: gap,
            geometry_gap: gap,
        },
    )
    .map_err(refusal)?;
    if result.report.material_cells == 0 {
        return Err(Error::new(
            "boolean.empty",
            name,
            "The exact result is empty; authored document parts require positive geometry",
        )
        .node(node));
    }
    let mut sources = result.report.face_sources.iter();
    let body = result.body.map_roles(
        |()| {
            let mut tags = BTreeSet::new();
            for source in sources.next().expect("one correspondence per output face") {
                let source_body = match source.operand {
                    Operand::Left => &left_body,
                    Operand::Right => &right_body,
                };
                tags.extend(
                    source_body.faces()[source.face.index()]
                        .role
                        .iter()
                        .copied(),
                );
            }
            tags.into_iter().collect()
        },
        |()| (),
    );
    let integrals = Some([result.report.volume, result.report.surface_area]);
    let mut operations = history(a);
    for prior in history(b) {
        if !operations.iter().any(|op| op.node() == prior.node()) {
            operations.push(prior);
        }
    }
    operations.push(AuthoredSolidOperation::Rectilinear {
        node,
        left,
        right,
        report: result.report,
    });
    Ok(Model::Body {
        body,
        frame: a.geometry().frame(),
        machining: Vec::new(),
        operations,
        integrals,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Mirrors the bounded halfspace authoring node"
)]
pub(super) fn halfspace(
    source: &Model,
    base: u32,
    point: [f64; 3],
    normal: [f64; 3],
    feature: Option<String>,
    labels: &BTreeMap<String, SegTag>,
    gap: f64,
    node: usize,
) -> Result<Model, Error> {
    let operation = "solid.intersectHalfSpace";
    let halfspace = HalfSpace::new(point, normal)
        .map_err(|e| Error::new("halfspace.input", operation, e.to_string()).node(node))?;
    let role = feature
        .as_ref()
        .map(|name| labels[name])
        .into_iter()
        .collect();
    let result = intersect_half_space_with_roles(
        &source.geometry().to_body(),
        &halfspace,
        &ClipPolicy {
            geometry_gap: gap,
            vertex_clearance: gap,
        },
        role,
        (),
    )
    .map_err(|error| {
        let code = match &error {
            ClipError::InvalidPolicy => "halfspace.policy",
            ClipError::Boundary(_) => "halfspace.boundary",
            ClipError::OpenBoundary => "halfspace.openBoundary",
            ClipError::DisconnectedShell => "halfspace.disconnectedShell",
            ClipError::UnsupportedFace { .. } => "halfspace.unsupportedFace",
            ClipError::Nonconvex { .. } => "halfspace.nonconvex",
            ClipError::NearVertex { .. } => "halfspace.nearVertex",
            _ => "halfspace.numericLimit",
        };
        let mut diagnostic = Error::new(code, operation, error.to_string()).node(node);
        let context = diagnostic.context_mut();
        context.feature = feature.clone();
        match error {
            ClipError::UnsupportedFace { face } => context.face = Some(face.0),
            ClipError::Nonconvex {
                face,
                vertex,
                distance,
            } => {
                context.face = Some(face.0);
                context.vertex = Some(vertex.0);
                context.clearance = Some(distance);
            }
            ClipError::NearVertex {
                vertex,
                distance,
                required,
            } => {
                context.vertex = Some(vertex.0);
                context.clearance = Some(distance);
                context.required_clearance = Some(required);
            }
            _ => {}
        }
        diagnostic
    })?;
    if result.report.outcome == ClipOutcome::Empty {
        return Err(Error::new(
            "halfspace.empty",
            operation,
            "The exact result is empty; authored document parts require positive geometry",
        )
        .node(node));
    }
    let mut operations = history(source);
    operations.push(AuthoredSolidOperation::HalfSpace {
        node,
        base,
        point,
        normal,
        feature,
        report: result.report,
    });
    Ok(Model::Body {
        body: result.body,
        frame: source.geometry().frame(),
        machining: Vec::new(),
        operations,
        integrals: None,
    })
}
