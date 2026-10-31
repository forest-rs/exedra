// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction-independent boundary tables at the authoring wire boundary.

use crate::Error;
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};
use exedra_analytic::{
    body::{
        Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, LoopTrims, Sense,
        TrimCurve, ValidationPolicy, VertexId,
    },
    geometry::{Curve, CurveSpan, GeometryError, ParameterDomain, ParameterRange, Surface},
};
use exedra_curves::profile::SegTag;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum InputCurve {
    Line {
        origin: [f64; 3],
        direction: [f64; 3],
    },
    Circle {
        frame: [[f64; 4]; 3],
        radius: f64,
    },
    Ellipse {
        frame: [[f64; 4]; 3],
        #[serde(rename = "semiMajor")]
        semi_major: f64,
        #[serde(rename = "semiMinor")]
        semi_minor: f64,
    },
}
impl InputCurve {
    fn lower(&self) -> Result<Curve, GeometryError> {
        match self {
            Self::Line { origin, direction } => Curve::line(*origin, *direction),
            Self::Circle { frame, radius } => {
                Curve::circle(exedra_math::Placement3 { rows: *frame }, *radius)
            }
            Self::Ellipse {
                frame,
                semi_major,
                semi_minor,
            } => Curve::ellipse(
                exedra_math::Placement3 { rows: *frame },
                *semi_major,
                *semi_minor,
            ),
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InputSpan {
    curve: InputCurve,
    range: [f64; 2],
    #[serde(default)]
    reversed: bool,
}
impl InputSpan {
    fn lower(&self) -> Result<CurveSpan, GeometryError> {
        CurveSpan::new(
            self.curve.lower()?,
            ParameterRange::new(self.range[0], self.range[1])?,
            self.reversed,
        )
    }
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum InputSurface {
    Plane {
        frame: [[f64; 4]; 3],
    },
    Cylinder {
        frame: [[f64; 4]; 3],
        radius: f64,
    },
    Cone {
        frame: [[f64; 4]; 3],
        radius: f64,
        slope: f64,
    },
    Sphere {
        frame: [[f64; 4]; 3],
        radius: f64,
    },
    Torus {
        frame: [[f64; 4]; 3],
        #[serde(rename = "majorRadius")]
        major_radius: f64,
        #[serde(rename = "minorRadius")]
        minor_radius: f64,
    },
    Ruled {
        curves: [InputSpan; 2],
    },
}
impl InputSurface {
    fn lower(&self) -> Result<Surface, GeometryError> {
        match self {
            Self::Plane { frame } => Surface::plane(exedra_math::Placement3 { rows: *frame }),
            Self::Cylinder { frame, radius } => {
                Surface::cylinder(exedra_math::Placement3 { rows: *frame }, *radius)
            }
            Self::Cone {
                frame,
                radius,
                slope,
            } => Surface::cone(exedra_math::Placement3 { rows: *frame }, *radius, *slope),
            Self::Sphere { frame, radius } => {
                Surface::sphere(exedra_math::Placement3 { rows: *frame }, *radius)
            }
            Self::Torus {
                frame,
                major_radius,
                minor_radius,
            } => Surface::torus(
                exedra_math::Placement3 { rows: *frame },
                *major_radius,
                *minor_radius,
            ),
            Self::Ruled { curves } => Surface::ruled(curves[0].lower()?, curves[1].lower()?),
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum InputEdge {
    Curve {
        curve: InputCurve,
        range: [f64; 2],
        start: u32,
        end: u32,
    },
    Collapsed {
        vertex: u32,
    },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputCoedge {
    edge: u32,
    #[serde(default)]
    reversed: bool,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum InputSense {
    Forward,
    Reverse,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFace {
    surface: InputSurface,
    u: [f64; 2],
    v: [f64; 2],
    sense: InputSense,
    loops: Vec<u32>,
    #[serde(default)]
    features: Vec<String>,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum InputTrim {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    Harmonic {
        origin: [f64; 2],
        linear: [f64; 2],
        cosine: [f64; 2],
        sine: [f64; 2],
        range: [f64; 2],
    },
}
impl InputTrim {
    fn lower(&self) -> Result<TrimCurve, GeometryError> {
        Ok(match self {
            Self::Line { start, end } => TrimCurve::Line {
                start: *start,
                end: *end,
            },
            Self::Harmonic {
                origin,
                linear,
                cosine,
                sine,
                range,
            } => TrimCurve::Harmonic {
                origin: *origin,
                linear: *linear,
                cosine: *cosine,
                sine: *sine,
                range: ParameterRange::new(range[0], range[1])?,
            },
        })
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputLoopTrims {
    #[serde(rename = "loop")]
    loop_id: u32,
    curves: Vec<InputTrim>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputBody {
    #[serde(default)]
    trims: Vec<InputLoopTrims>,
    vertices: Vec<[f64; 3]>,
    edges: Vec<InputEdge>,
    loops: Vec<Vec<InputCoedge>>,
    faces: Vec<InputFace>,
}
impl InputBody {
    pub(crate) fn work(&self) -> usize {
        self.trims
            .iter()
            .map(|trims| trims.curves.len())
            .fold(0_usize, usize::saturating_add)
            .saturating_add(self.vertices.len())
            .saturating_add(self.edges.len())
            .saturating_add(self.faces.len())
            .saturating_add(self.loops.len())
            .saturating_add(self.trims.len())
            .saturating_add(
                self.faces
                    .iter()
                    .map(|face| {
                        face.loops
                            .iter()
                            .map(|&loop_id| self.loops.get(loop_id as usize).map_or(0, Vec::len))
                            .fold(0_usize, usize::saturating_add)
                            .saturating_add(face.loops.len())
                            .saturating_add(face.features.len())
                    })
                    .fold(0_usize, usize::saturating_add),
            )
            .saturating_add(
                self.loops
                    .iter()
                    .map(Vec::len)
                    .fold(0_usize, usize::saturating_add),
            )
    }
    pub(crate) fn feature_names(&self) -> impl Iterator<Item = &str> {
        self.faces
            .iter()
            .flat_map(|face| face.features.iter().map(String::as_str))
    }
    pub(crate) fn lower(
        &self,
        labels: &BTreeMap<String, SegTag>,
        gap: f64,
    ) -> Result<Body<Vec<SegTag>>, Error> {
        let failure = |message: String| Error::new("body.invalid", "solid.body", message);
        let geometry = |e: GeometryError| failure(e.to_string());
        let edges = self
            .edges
            .iter()
            .map(|edge| {
                Ok(match edge {
                    InputEdge::Curve {
                        curve,
                        range,
                        start,
                        end,
                    } => Edge::new(
                        curve.lower().map_err(geometry)?,
                        ParameterRange::new(range[0], range[1]).map_err(geometry)?,
                        VertexId(*start),
                        VertexId(*end),
                        (),
                    ),
                    InputEdge::Collapsed { vertex } => Edge::collapsed(VertexId(*vertex), ()),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let loops = self
            .loops
            .iter()
            .map(|ring| BoundaryLoop {
                coedges: ring
                    .iter()
                    .map(|c| Coedge {
                        edge: EdgeId(c.edge),
                        reversed: c.reversed,
                    })
                    .collect(),
            })
            .collect();
        let faces = self
            .faces
            .iter()
            .map(|face| {
                Ok(Face {
                    surface: face.surface.lower().map_err(geometry)?,
                    domain: ParameterDomain {
                        u: ParameterRange::new(face.u[0], face.u[1]).map_err(geometry)?,
                        v: ParameterRange::new(face.v[0], face.v[1]).map_err(geometry)?,
                    },
                    sense: match face.sense {
                        InputSense::Forward => Sense::Forward,
                        InputSense::Reverse => Sense::Reverse,
                    },
                    loops: face.loops.iter().map(|&id| LoopId(id)).collect(),
                    role: face.features.iter().map(|name| labels[name]).collect(),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Body::try_from_parts(
            BodyParts {
                trims: self
                    .trims
                    .iter()
                    .map(|trims| {
                        Ok(LoopTrims {
                            loop_id: LoopId(trims.loop_id),
                            curves: trims
                                .curves
                                .iter()
                                .map(|curve| curve.lower().map_err(geometry))
                                .collect::<Result<Vec<_>, _>>()?,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?,
                vertices: self.vertices.clone(),
                edges,
                loops,
                faces,
            },
            &ValidationPolicy { max_gap: gap },
        )
        .map_err(|e| failure(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use serde_json::json;

    #[test]
    fn nested_boundary_records_are_charged_before_native_construction() {
        let face = json!({"surface":{"kind":"plane","frame":[[1,0,0,0],[0,1,0,0],[0,0,1,0]]},"u":[0,1],"v":[0,1],"sense":"forward","loops":[],"features":[]});
        let empty = json!({"vertices":[],"edges":[],"loops":[],"faces":[]});
        let mut inputs = vec![empty.clone(); 5];
        inputs[0]["loops"] = json!(vec![Vec::<u32>::new(); 8193]);
        inputs[1]["trims"] = json!(vec![json!({"loop":0,"curves":[]}); 8193]);
        inputs[2]["faces"] = json!([face.clone()]);
        inputs[2]["faces"][0]["loops"] = json!(vec![0_u32; 8193]);
        inputs[3]["faces"] = json!([face.clone()]);
        inputs[3]["faces"][0]["features"] = json!(vec!["stock"; 8193]);
        inputs[4]["loops"] = json!([vec![json!({"edge":0,"reversed":false}); 128]]);
        inputs[4]["faces"] = json!([face]);
        inputs[4]["faces"][0]["loops"] = json!(vec![0_u32; 64]);
        for body in inputs {
            let request = json!({"units":"mm","separation":1e-8,"profiles":[],"nodes":[{"kind":"body","body":body}],"roots":[{"name":"stock","node":0}]});
            let error = serde_json::from_value::<crate::authoring::Request>(request)
                .unwrap()
                .build()
                .unwrap_err();
            assert_eq!(error.code, "budget.nativeSegments");
        }
    }

    #[test]
    fn exact_boundary_tables_validate_without_constructor_reconstruction() {
        let vertices = [
            [0., 0., 0.],
            [4., 0., 0.],
            [4., 3., 0.],
            [0., 3., 0.],
            [0., 0., 2.],
            [4., 0., 2.],
            [4., 3., 2.],
            [0., 3., 2.],
        ];
        let pairs = [
            [0, 1],
            [1, 2],
            [2, 3],
            [3, 0],
            [4, 5],
            [5, 6],
            [6, 7],
            [7, 4],
            [0, 4],
            [1, 5],
            [2, 6],
            [3, 7],
        ];
        let edges:Vec<_>=pairs.map(|[start,end]|json!({"kind":"curve","curve":{"kind":"line","origin":vertices[start],"direction":core::array::from_fn::<_,3,_>(|i|vertices[end][i]-vertices[start][i])},"range":[0,1],"start":start,"end":end})).into();
        let loop_ = |entries: &[(u32, bool)]| {
            entries
                .iter()
                .map(|&(edge, reversed)| json!({"edge":edge,"reversed":reversed}))
                .collect::<Vec<_>>()
        };
        let loops = vec![
            loop_(&[(3, true), (2, true), (1, true), (0, true)]),
            loop_(&[(4, false), (5, false), (6, false), (7, false)]),
            loop_(&[(0, false), (9, false), (4, true), (8, true)]),
            loop_(&[(1, false), (10, false), (5, true), (9, true)]),
            loop_(&[(2, false), (11, false), (6, true), (10, true)]),
            loop_(&[(3, false), (8, false), (7, true), (11, true)]),
        ];
        let frames = [
            [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
            [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 2.]],
            [[1., 0., 0., 0.], [0., 0., -1., 0.], [0., 1., 0., 0.]],
            [[0., 0., 1., 4.], [0., 1., 0., 0.], [-1., 0., 0., 0.]],
            [[1., 0., 0., 0.], [0., 0., 1., 3.], [0., -1., 0., 0.]],
            [[0., 0., -1., 0.], [0., 1., 0., 0.], [1., 0., 0., 0.]],
        ];
        let domains = [
            [[0., 4.], [0., 3.]],
            [[0., 4.], [0., 3.]],
            [[0., 4.], [0., 2.]],
            [[-2., 0.], [0., 3.]],
            [[0., 4.], [-2., 0.]],
            [[0., 2.], [0., 3.]],
        ];
        let faces:Vec<_>=frames.iter().enumerate().map(|(i,frame)|json!({"surface":{"kind":"plane","frame":frame},"u":domains[i][0],"v":domains[i][1],"sense":if i==0 {"reverse"} else {"forward"},"loops":[i],"features":["stock","housing"]})).collect();
        let mut request = json!({"units":"mm","separation":1e-8,"profiles":[],"nodes":[{"kind":"body","body":{"vertices":vertices,"edges":edges,"loops":loops,"faces":faces}}],"roots":[{"name":"stock","node":0}]});
        let built = serde_json::from_value::<crate::authoring::Request>(request.clone())
            .unwrap()
            .build()
            .unwrap();
        let native = &built.roots[0].1;
        assert_eq!(native.family(), exedra_compile::NativeFamily::Body);
        assert!(
            matches!(native.geometry(),exedra_compile::NativeGeometryRef::Body{body,..} if body.is_closed())
        );
        assert_eq!(
            native
                .face_tags(exedra_analytic::body::FaceId(0))
                .unwrap()
                .count(),
            2
        );
        let value = crate::inspect::metadata(native, &built.names).unwrap();
        assert_eq!(value["faces"][0]["role"], "boundary");
        assert_eq!(value["stepSupport"]["status"], "supported");
        assert_eq!(value["measurements"]["volume"]["status"], "unavailable");
        request["nodes"][0]["body"]["edges"][0]["end"] = json!(999);
        let error = serde_json::from_value::<crate::authoring::Request>(request)
            .unwrap()
            .build()
            .unwrap_err();
        assert_eq!(error.code, "body.invalid");
        assert_eq!(error.operation, "solid.body");
    }
}
