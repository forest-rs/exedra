// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact rectangular boundary correspondence for original ruled support domains.
use super::{Body, EdgeId, FaceId, Sense};
use crate::geometry::{CurveSpan, ParameterRange, Surface};

#[derive(Copy, Clone, Debug)]
pub(crate) struct RuledSide {
    pub(crate) edge: EdgeId,
    /// Opposes the shared edge's canonical direction, in increasing chart U/V.
    pub(crate) reverse: bool,
}
#[derive(Copy, Clone, Debug)]
pub(crate) struct RuledRectangle {
    /// Bottom/right/top/left, individually in increasing U or V.
    pub(crate) sides: [RuledSide; 4],
    pub(crate) alignment: f64,
}
impl<F, E> Body<F, E> {
    pub(crate) fn ruled_rectangle(&self, id: FaceId) -> Option<RuledRectangle> {
        self.ruled_rectangle_with_source_budget(id, &mut crate::geometry::SourceBudget::default())
    }
    pub(crate) fn ruled_rectangle_with_source_budget(
        &self,
        id: FaceId,
        budget: &mut crate::geometry::SourceBudget,
    ) -> Option<RuledRectangle> {
        let face = self.face(id)?;
        let Surface::Ruled(ruled) = &face.surface else {
            return None;
        };
        if face.domain.v != ParameterRange::UNIT
            || face
                .domain
                .u
                .bounds()
                .into_iter()
                .any(|u| !ParameterRange::UNIT.contains(u))
            || face.loops.len() != 1
        {
            return None;
        }
        let uses = &self.boundary_loop(face.loops[0])?.coedges;
        if uses.len() != 4 {
            return None;
        }
        let boundaries = ruled.domain_boundaries_with_budget(face.domain, budget)?;
        let size = ruled
            .curves()
            .iter()
            .map(CurveSpan::magnitude)
            .fold(0.0, f64::max);
        let slack = 128.0 * crate::geometry::FRAME_TOLERANCE * size;
        let mut sides = [None; 4];
        let mut alignment = 0.0_f64;
        for co in uses {
            let edge = self.edge(co.edge)?;
            let (curve, range) = edge.regular()?;
            let mut found = None;
            for (slot, side) in sides.iter().enumerate() {
                if side.is_some() {
                    continue;
                }
                let reverse = co.reversed ^ (slot >= 2) ^ (face.sense == Sense::Reverse);
                let actual = CurveSpan::new(curve.clone(), range, reverse).ok()?;
                let bound = boundaries.alignment(slot, &actual, budget);
                if bound <= slack {
                    found = Some((
                        slot,
                        RuledSide {
                            edge: co.edge,
                            reverse,
                        },
                        bound,
                    ));
                    break;
                }
            }
            let (slot, side, bound) = found?;
            sides[slot] = Some(side);
            alignment = alignment.max(bound);
        }
        let sides = sides.map(|side| side.expect("four distinct matched slots"));
        let endpoint = |side: RuledSide, last: bool| {
            let edge = &self.edges()[side.edge.index()];
            if side.reverse ^ last {
                edge.end
            } else {
                edge.start
            }
        };
        for (a, alast, b, blast) in [
            (0, false, 3, false),
            (0, true, 1, false),
            (2, false, 3, true),
            (2, true, 1, true),
        ] {
            if endpoint(sides[a], alast) != endpoint(sides[b], blast) {
                return None;
            }
        }
        Some(RuledRectangle { sides, alignment })
    }
}
