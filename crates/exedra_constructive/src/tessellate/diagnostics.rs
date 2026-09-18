// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{DiscretizedProfile, Profile2, TessellateError};
use crate::profile::SegTag;
use exedra_triangulate::{BoundaryEdge, TriError};

/// An offending sampled boundary edge and its authored profile segment.
///
/// Coordinates come from the operation's discretization. For curved segments,
/// this is evidence about the sampled chord, not an analytic intersection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileBoundaryEdge {
    /// Original cyclic edge in the sampled outer boundary or hole.
    pub sampled: BoundaryEdge,
    /// Authored segment index in that boundary's source loop.
    pub segment: u32,
    /// Authored segment tag, when present.
    pub tag: Option<SegTag>,
}

impl core::fmt::Display for ProfileBoundaryEdge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.sampled.hole {
            None => write!(f, "outer segment {}", self.segment)?,
            Some(hole) => write!(f, "hole {hole} segment {}", self.segment)?,
        }
        if let Some(tag) = self.tag {
            write!(f, " (tag {})", tag.0)?;
        }
        write!(f, " [sampled edge {}]", self.sampled.edge)
    }
}

pub(super) fn triangulation_error(
    profile: &Profile2,
    d: &DiscretizedProfile,
    error: TriError,
) -> TessellateError {
    if let TriError::BoundaryContact {
        first,
        second,
        kind,
    } = error
    {
        let source = |sampled: BoundaryEdge| {
            let (boundary, authored) = match sampled.hole {
                None => (&d.outer, profile.outer()),
                Some(hole) => (&d.holes[hole], &profile.holes()[hole]),
            };
            let segment = boundary.edge_seg[sampled.edge];
            ProfileBoundaryEdge {
                sampled,
                segment,
                tag: authored.segs()[segment as usize].tag,
            }
        };
        TessellateError::ProfileBoundaryContact {
            first: source(first),
            second: source(second),
            kind,
        }
    } else {
        TessellateError::Triangulate(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use crate::ir::{CapMode, LoftPolicy, Placement3};
    use crate::profile::{Loop2, Seg2};
    use crate::tessellate::{
        EvalPolicy, tessellate_extrude, tessellate_loft, tessellate_planar_face, tessellate_sweep,
    };
    use alloc::vec;
    use exedra_triangulate::BoundaryContactKind;

    #[test]
    fn ground_reaching_opening_reports_authored_segment_tags() {
        let outer = builders::rect_from_corner(4.0, 4.0).unwrap();
        let hole = Loop2::new(vec![
            Seg2::arc((3.0, 2.0), -1.0).tagged(SegTag(11)),
            Seg2::line((3.0, 0.0)).tagged(SegTag(12)),
            Seg2::line((2.0, 0.0)).tagged(SegTag(13)),
            Seg2::line((2.0, 2.0)).tagged(SegTag(10)),
        ])
        .unwrap();
        let profile = Profile2::new(outer.outer().clone(), vec![hole]).unwrap();
        for refinement in [None, Some(exedra_triangulate::RefineParams::default())] {
            let policy = EvalPolicy {
                cap_refinement: refinement,
                planar_face_refinement: refinement,
                ..EvalPolicy::default()
            };
            for result in [
                tessellate_extrude(&profile, &Placement3::IDENTITY, 1.0, CapMode::Both, &policy),
                tessellate_planar_face(&profile, &Placement3::IDENTITY, &policy),
                tessellate_sweep(
                    &profile,
                    &Placement3::IDENTITY,
                    &[[0.0; 3], [0.0, 0.0, 1.0]],
                    CapMode::Both,
                    &policy,
                ),
                tessellate_loft(
                    &[
                        (Placement3::IDENTITY, &profile),
                        (Placement3::translate(0.0, 0.0, 1.0), &profile),
                    ],
                    LoftPolicy::Ruled,
                    CapMode::Both,
                    &policy,
                ),
            ] {
                let Err(TessellateError::ProfileBoundaryContact {
                    first,
                    second,
                    kind,
                }) = result
                else {
                    panic!("expected source-tagged contact, got {result:?}");
                };
                assert_eq!(first.segment, 0);
                assert_eq!(first.tag, Some(SegTag(0)));
                assert_eq!(second.segment, 1);
                assert_eq!(second.tag, Some(SegTag(12)));
                assert!(
                    second.sampled.edge > second.segment as usize,
                    "arc sampling must not be mistaken for segment indices"
                );
                assert_eq!(kind, BoundaryContactKind::Touching);
            }
        }
    }
}
