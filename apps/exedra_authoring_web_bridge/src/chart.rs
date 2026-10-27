// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Wire representations of the existing kernel construction chart policy.
use exedra_constructive::chart::{ChartTransform, LoftStations, SurfaceChart};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputTransform {
    matrix: [[f64; 2]; 2],
    offset: [f64; 2],
}
impl From<InputTransform> for ChartTransform {
    fn from(value: InputTransform) -> Self {
        Self {
            matrix: value.matrix,
            offset: value.offset,
        }
    }
}
impl From<ChartTransform> for InputTransform {
    fn from(value: ChartTransform) -> Self {
        Self {
            matrix: value.matrix,
            offset: value.offset,
        }
    }
}
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum InputStations {
    SectionIndex,
    DatumDistance,
}
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum InputChart {
    Extrude {
        wall: InputTransform,
        caps: InputTransform,
    },
    Revolve {
        #[serde(rename = "referenceRadius")]
        reference_radius: f64,
        wall: InputTransform,
        caps: InputTransform,
    },
    Sweep {
        wall: InputTransform,
        caps: InputTransform,
    },
    Loft {
        #[serde(rename = "referenceSection")]
        reference_section: u32,
        #[serde(rename = "restLength")]
        rest_length: f64,
        stations: InputStations,
        wall: InputTransform,
        caps: InputTransform,
    },
}
impl From<InputChart> for SurfaceChart {
    fn from(value: InputChart) -> Self {
        match value {
            InputChart::Extrude { wall, caps } => Self::Extrude {
                wall: wall.into(),
                caps: caps.into(),
            },
            InputChart::Revolve {
                reference_radius,
                wall,
                caps,
            } => Self::Revolve {
                reference_radius,
                wall: wall.into(),
                caps: caps.into(),
            },
            InputChart::Sweep { wall, caps } => Self::Sweep {
                wall: wall.into(),
                caps: caps.into(),
            },
            InputChart::Loft {
                reference_section,
                rest_length,
                stations,
                wall,
                caps,
            } => Self::Loft {
                reference_section,
                rest_length,
                stations: match stations {
                    InputStations::SectionIndex => LoftStations::SectionIndex,
                    InputStations::DatumDistance => LoftStations::DatumDistance,
                },
                wall: wall.into(),
                caps: caps.into(),
            },
        }
    }
}
impl TryFrom<&SurfaceChart> for InputChart {
    type Error = crate::Error;
    fn try_from(value: &SurfaceChart) -> Result<Self, Self::Error> {
        Ok(match *value {
            SurfaceChart::Extrude { wall, caps } => Self::Extrude {
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChart::Revolve {
                reference_radius,
                wall,
                caps,
            } => Self::Revolve {
                reference_radius,
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChart::Sweep { wall, caps } => Self::Sweep {
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChart::Loft {
                reference_section,
                rest_length,
                stations,
                wall,
                caps,
            } => Self::Loft {
                reference_section,
                rest_length,
                stations: match stations {
                    LoftStations::SectionIndex => InputStations::SectionIndex,
                    LoftStations::DatumDistance => InputStations::DatumDistance,
                    _ => {
                        return Err(crate::Error::new(
                            "chart.stationMode",
                            "part.realize",
                            "This native station metric is not represented by the bridge",
                        ));
                    }
                },
                wall: wall.into(),
                caps: caps.into(),
            },
        })
    }
}

pub(crate) fn diagnostic(
    error: &exedra_compile::NativeChartError,
    message: alloc::string::String,
) -> crate::Error {
    use exedra_analytic::{loft::LoftUvError, revolve::RevolutionUvError, sweep::SweepUvError};
    use exedra_compile::NativeChartError;
    use exedra_constructive::chart::ChartError;
    let mut diagnostic = crate::Error::new("chart.realization", "part.realize", message);
    let mapping =
        |diagnostic: &mut crate::Error, face: exedra_analytic::body::FaceId, error: &ChartError| {
            diagnostic.context_mut().face = Some(face.0);
            diagnostic.code = match error {
                ChartError::NumericLimit => "chart.numericLimit",
                ChartError::WrongOperation => "chart.operation",
                _ => "chart.invalid",
            };
        };
    match error {
        NativeChartError::Invalid(error) => {
            diagnostic.code = if matches!(error, ChartError::NumericLimit) {
                "chart.numericLimit"
            } else {
                "chart.invalid"
            };
        }
        NativeChartError::Unavailable { family } => {
            diagnostic.code = "chart.unavailable";
            diagnostic.context_mut().family = Some(*family);
        }
        NativeChartError::WrongOperation { family } => {
            diagnostic.code = "chart.operation";
            diagnostic.context_mut().family = Some(*family);
        }
        NativeChartError::Revolution(error) => match error {
            RevolutionUvError::Mapping { face, error } => mapping(&mut diagnostic, *face, error),
            RevolutionUvError::Coordinates { face } => {
                diagnostic.context_mut().face = Some(face.0);
                diagnostic.code = "chart.coordinates";
            }
            RevolutionUvError::NonFiniteUv { face } => {
                diagnostic.context_mut().face = Some(face.0);
                diagnostic.code = "chart.numericLimit";
            }
            RevolutionUvError::Metric { .. } => diagnostic.code = "chart.metric",
            RevolutionUvError::Realization(_) => diagnostic.code = "realization.invalid",
        },
        NativeChartError::Sweep(error) => match error {
            SweepUvError::Mapping { face, error } => mapping(&mut diagnostic, *face, error),
            SweepUvError::Coordinates { face } => {
                diagnostic.context_mut().face = Some(face.0);
                diagnostic.code = "chart.coordinates";
            }
            SweepUvError::NonFiniteUv { face } => {
                diagnostic.context_mut().face = Some(face.0);
                diagnostic.code = "chart.numericLimit";
            }
            SweepUvError::Metric { .. } => diagnostic.code = "chart.metric",
            SweepUvError::Realization(_) => diagnostic.code = "realization.invalid",
        },
        NativeChartError::Loft(error) => match error {
            LoftUvError::Mapping { face, error } => mapping(&mut diagnostic, *face, error),
            LoftUvError::Coordinates { face } => {
                diagnostic.context_mut().face = Some(face.0);
                diagnostic.code = "chart.coordinates";
            }
            LoftUvError::NonFiniteUv { face } => {
                diagnostic.context_mut().face = Some(face.0);
                diagnostic.code = "chart.numericLimit";
            }
            LoftUvError::Metric => diagnostic.code = "chart.metric",
            LoftUvError::Realization(_) => diagnostic.code = "realization.invalid",
        },
    }
    diagnostic
}
