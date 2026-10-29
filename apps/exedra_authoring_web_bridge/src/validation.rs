// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit boundary inspection without a display conversion or global-solid claim.
use super::*;
use exedra_analytic::validation::{
    BoundaryValidationError, BoundaryValidationPolicy, TrimProof, TrimValidation, UnsupportedTrim,
};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Options {
    chart_separation: Option<f64>,
    max_coedges: Option<usize>,
    max_pair_tests: Option<usize>,
}

pub(super) fn inspect(source: &NativeSource, gap: f64, options: Options) -> Result<String, Error> {
    const OPERATION: &str = "part.validateBoundary";
    let policy = BoundaryValidationPolicy {
        max_gap: gap,
        chart_separation: options.chart_separation.unwrap_or(1e-8),
        max_coedges: options.max_coedges.unwrap_or(250_000),
        max_pair_tests: options.max_pair_tests.unwrap_or(2_000_000),
    };
    let report = source
        .to_body()
        .validate_boundary(&policy)
        .map_err(|error| {
            let code = match &error {
                BoundaryValidationError::InvalidPolicy => "boundary.policy",
                BoundaryValidationError::WorkLimit { .. } => "boundary.workLimit",
                BoundaryValidationError::Geometry(_) => "boundary.geometry",
                BoundaryValidationError::LoopOwnership { .. } => "boundary.loopOwnership",
                BoundaryValidationError::EdgeIncidence { .. } => "boundary.edgeIncidence",
                BoundaryValidationError::DisconnectedVertexLink { .. } => "boundary.vertexLink",
                BoundaryValidationError::Empty => "boundary.empty",
                _ => "boundary.arrangement",
            };
            let mut diagnostic = Error::new(code, OPERATION, error.to_string());
            let context = diagnostic.context_mut();
            match error {
                BoundaryValidationError::WorkLimit { required, maximum } => {
                    context.required = required;
                    context.maximum = Some(maximum);
                }
                BoundaryValidationError::LoopOwnership { loop_id, .. } => {
                    context.boundary_loop = Some(loop_id.0);
                }
                BoundaryValidationError::EdgeIncidence { edge, .. } => context.edge = Some(edge.0),
                BoundaryValidationError::DisconnectedVertexLink { vertex } => {
                    context.vertex = Some(vertex.0);
                }
                BoundaryValidationError::CollapsedLoop { face, loop_id }
                | BoundaryValidationError::ChartDomain { face, loop_id }
                | BoundaryValidationError::Winding { face, loop_id }
                | BoundaryValidationError::HoleContainment { face, loop_id } => {
                    context.face = Some(face.0);
                    context.boundary_loop = Some(loop_id.0);
                }
                BoundaryValidationError::PolygonArrangement { face, .. }
                | BoundaryValidationError::ChartLoop { face, .. }
                | BoundaryValidationError::CylinderWidth { face, .. } => {
                    context.face = Some(face.0);
                }
                _ => (),
            }
            diagnostic
        })?;
    let trims:Vec<_>=report.face_trims.iter().enumerate().map(|(face,status)|match status {
        TrimValidation::Certified(proof)=>serde_json::json!({"face":face,"status":"certified","proof":match proof {
            TrimProof::Polygon=>"polygon",TrimProof::PlanarEllipses=>"planarEllipses",TrimProof::CylindricalGraphs=>"cylindricalGraphs"}}),
        TrimValidation::Unsupported(reason)=>serde_json::json!({"face":face,"status":"unsupported","reason":match reason {
            UnsupportedTrim::MissingChart=>"missingChart",UnsupportedTrim::CurveFamily=>"curveFamily",
            UnsupportedTrim::SeparationUnresolved=>"separationUnresolved",UnsupportedTrim::NumericLimit=>"numericLimit"}}),
    }).collect();
    Ok(json(
        &serde_json::json!({"scope":"localBoundary","globalSolidValidity":"notChecked",
        "policy":{"maxGap":policy.max_gap,"chartSeparation":policy.chart_separation,"maxCoedges":policy.max_coedges,"maxPairTests":policy.max_pair_tests},
        "orientedManifold":true,"allTrimsChecked":report.all_trims_checked(),"faceTrims":trims,
        "shells":report.shells.iter().map(|shell|serde_json::json!({"faces":shell.faces.iter().map(|face|face.0).collect::<Vec<_>>(),
            "vertices":shell.vertices,"regularEdges":shell.regular_edges,"eulerCharacteristic":shell.euler_characteristic})).collect::<Vec<_>>(),
        "coedges":report.coedges,"pairTestBound":report.pair_test_bound,
        "maxGap":report.geometry.max_gap,"explicitTrimCoedges":report.geometry.trimmed_coedges,
        "maxTrimGap":report.geometry.max_trim_gap}),
    ))
}
