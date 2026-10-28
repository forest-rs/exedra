// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opt-in native construction charts, separate from immutable source identity.

use super::{NativeFamily, NativeGeometryRef, NativeSource};
use exedra_analytic::realize::{RealizationPolicy, RealizedBody};

/// Why a native construction chart was unavailable or refused.
#[derive(Clone, Debug, PartialEq)]
pub enum NativeChartError {
    /// Invalid authored metric or texture configuration.
    Invalid(exedra_constructive::chart::ChartError),
    /// This construction family has no native chart realization path yet.
    Unavailable {
        /// Requested source family.
        family: NativeFamily,
    },
    /// The requested chart operation does not match this source family.
    WrongOperation {
        /// Actual source family.
        family: NativeFamily,
    },
    /// The native revolution or its mapped coordinates were refused.
    Revolution(exedra_analytic::revolve::RevolutionUvError<exedra_constructive::chart::ChartError>),
    /// The native sweep or its mapped coordinates were refused.
    Sweep(exedra_analytic::sweep::SweepUvError<exedra_constructive::chart::ChartError>),
    /// The native loft or its mapped coordinates were refused.
    Loft(exedra_analytic::loft::LoftUvError<exedra_constructive::chart::ChartError>),
}
impl core::fmt::Display for NativeChartError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "native chart configuration: {e}"),
            Self::Unavailable { family } => write!(f, "native charts unavailable for {family:?}"),
            Self::WrongOperation { family } => {
                write!(f, "chart operation does not match native {family:?}")
            }
            Self::Revolution(e) => write!(f, "{e}"),
            Self::Sweep(e) => write!(f, "{e}"),
            Self::Loft(e) => write!(f, "{e}"),
        }
    }
}
impl core::error::Error for NativeChartError {}

impl NativeSource {
    /// Checks native chart configuration and family support without realizing.
    ///
    /// This constant-time preflight checks scalar transforms, operation kind
    /// and reference-section range. Actual construction distances and mapped
    /// coordinates are checked on a cache miss during realization, not walked
    /// again on a valid cache hit. Source identity and selection scope stay fixed.
    ///
    /// # Errors
    /// Returns a typed invalid configuration, wrong operation or unavailable family.
    pub fn check_chart(
        &self,
        chart: &exedra_constructive::chart::SurfaceChart,
    ) -> Result<(), NativeChartError> {
        use exedra_constructive::chart::{ChartError, LoftStations, SurfaceChart};
        chart.validate().map_err(NativeChartError::Invalid)?;
        let family = self.family();
        let kind = self
            .chart_kind()
            .ok_or(NativeChartError::Unavailable { family })?;
        if chart.kind() != kind {
            return Err(NativeChartError::WrongOperation { family });
        }
        if let (
            NativeGeometryRef::Loft(v),
            SurfaceChart::Loft {
                reference_section,
                stations,
                ..
            },
        ) = (self.geometry(), chart)
        {
            if *reference_section as usize >= v.sections().len() {
                return Err(NativeChartError::Invalid(
                    ChartError::InvalidReferenceSection {
                        section: *reference_section,
                    },
                ));
            }
            match stations {
                LoftStations::SectionIndex | LoftStations::DatumDistance => {}
                _ => return Err(NativeChartError::Unavailable { family }),
            }
        }
        Ok(())
    }

    pub(crate) fn realize_with_chart(
        &self,
        policy: &RealizationPolicy,
        chart: &exedra_constructive::chart::SurfaceChart,
    ) -> Result<RealizedBody, NativeChartError> {
        self.check_chart(chart)?;
        match self.geometry() {
            NativeGeometryRef::Revolution(v) => v
                .realize_with_uvs(
                    policy,
                    |_, _| 0,
                    |face, role, p| {
                        if matches!(
                            v.body().faces()[face.index()].surface,
                            exedra_analytic::geometry::Surface::Plane(_)
                        ) && (matches!(role, exedra_analytic::revolve::FaceRole::Cap { .. })
                            || v.body().faces()[face.index()].loops.len() == 1)
                        {
                            chart.map_cap_triangle(p)
                        } else {
                            chart.map_revolve_wall_triangle(p)
                        }
                    },
                )
                .map_err(NativeChartError::Revolution),
            NativeGeometryRef::Sweep(v) => v
                .realize_with_uvs(
                    policy,
                    |_, _| 0,
                    |role, p| match role {
                        exedra_analytic::sweep::FaceRole::Wall { .. } => {
                            chart.map_sweep_wall_triangle(p)
                        }
                        exedra_analytic::sweep::FaceRole::Cap { .. } => chart.map_cap_triangle(p),
                    },
                )
                .map_err(NativeChartError::Sweep),
            NativeGeometryRef::Loft(v) => {
                let exedra_constructive::chart::SurfaceChart::Loft {
                    reference_section, ..
                } = chart
                else {
                    unreachable!("preflight matched family")
                };
                let stations = chart
                    .loft_station_distances(
                        v.sections()
                            .iter()
                            .map(|(frame, _)| frame.transform_point([0.0; 3])),
                    )
                    .map_err(NativeChartError::Invalid)?;
                v.realize_with_uvs(
                    policy,
                    *reference_section as usize,
                    &stations,
                    |_, _| 0,
                    |role, p| match role {
                        exedra_analytic::loft::FaceRole::Wall { .. } => {
                            chart.map_loft_wall_triangle(p)
                        }
                        exedra_analytic::loft::FaceRole::Cap { .. } => chart.map_cap_triangle(p),
                    },
                )
                .map_err(NativeChartError::Loft)
            }
            NativeGeometryRef::Extrusion(_) | NativeGeometryRef::BoredPlate(_) => {
                unreachable!("preflight refused unavailable family")
            }
        }
    }
}
