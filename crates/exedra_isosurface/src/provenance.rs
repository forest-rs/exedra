// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! How field provenance becomes a face region.
//!
//! [`ProvenanceField`](crate::ProvenanceField) reports where a value came
//! from in whatever form a field composes: a tag for a leaf, a
//! [`BinaryProvenance`] wrapping its operand's provenance for a combinator.
//! Region extraction needs one `u32` per face. [`ProjectRegion`] is the
//! explicit rule from one to the other, and it also says how firmly the
//! region was decided, so extraction can count weak attributions instead of
//! hiding them.

use crate::analytic::BinaryProvenance;

/// How firmly a sampled region was decided, weakest last.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Attribution {
    /// A single tagged source produced the value; nothing was chosen.
    Sole,
    /// A hard minimum or maximum chose one operand. The surface there is
    /// that operand's own.
    Winner,
    /// A smooth blend mixed both operands. The region names the larger
    /// contributor, but the surface belongs to both.
    Blended,
}

/// A face region together with how firmly it was decided.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct RegionSample {
    /// Region identifier written to `FACE_REGION`.
    pub region: u32,
    /// How firmly the region was decided.
    pub attribution: Attribution,
}

/// Projects provenance onto a face region.
///
/// Implemented for `u32` tags, which are [`Attribution::Sole`], and for
/// [`BinaryProvenance`] of any projectable provenance, which keeps its
/// operand's region and the weaker of its operand's attribution and its own.
/// Implement it for caller provenance types to extract them with regions.
pub trait ProjectRegion {
    /// The region this provenance names.
    fn project_region(&self) -> RegionSample;
}

impl ProjectRegion for u32 {
    fn project_region(&self) -> RegionSample {
        RegionSample {
            region: *self,
            attribution: Attribution::Sole,
        }
    }
}

impl<P: ProjectRegion> ProjectRegion for BinaryProvenance<P> {
    fn project_region(&self) -> RegionSample {
        let inner = self.value.project_region();
        let own = if self.blended {
            Attribution::Blended
        } else {
            Attribution::Winner
        };
        RegionSample {
            region: inner.region,
            attribution: inner.attribution.max(own),
        }
    }
}

/// Patches attributed at each strength by region extraction.
///
/// Each count is of crossing patches, one per sign-changing grid edge,
/// before triangulation. All are zero for extraction without regions.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct RegionAttribution {
    /// Patches whose region came from a single source.
    pub sole: usize,
    /// Patches whose region a hard combinator chose.
    pub winner: usize,
    /// Patches whose region names the larger contributor to a blend.
    pub blended: usize,
}

impl RegionAttribution {
    pub(crate) fn record(&mut self, attribution: Attribution) {
        match attribution {
            Attribution::Sole => self.sole += 1,
            Attribution::Winner => self.winner += 1,
            Attribution::Blended => self.blended += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Attribution, ProjectRegion, RegionAttribution};
    use crate::analytic::{
        BinaryProvenance, OperandSource, SmoothUnion, SphereField, TaggedField, Union,
    };
    use crate::{
        Aabb, DualContourParams, EdgeSearchParams, ExtractionLimits, QefParams,
        dual_contour_with_regions,
    };

    fn tagged(center: [f32; 3], radius: f32, tag: u32) -> TaggedField<SphereField, u32> {
        TaggedField {
            field: SphereField::new(center, radius).unwrap(),
            provenance: tag,
        }
    }

    fn attribution<F>(field: &F) -> (RegionAttribution, [bool; 2])
    where
        F: crate::ProvenanceField,
        F::Provenance: ProjectRegion,
    {
        let params = DualContourParams {
            root_bounds: Aabb::new([-2.5, -1.5, -1.5], [2.5, 1.5, 1.5]).unwrap(),
            max_depth: 5,
            thin_feature_depth: 0,
            cell_budget: None,
            limits: ExtractionLimits::default(),
            witness_limit: 0,
            vertex_merge_tolerance: 0.0,
            edge_search: EdgeSearchParams::default(),
            qef: QefParams::default(),
        };
        let result = dual_contour_with_regions(field, &params).unwrap();
        let regions = result
            .mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .unwrap();
        let seen = [1, 2].map(|tag| {
            result
                .mesh
                .faces()
                .any(|f| regions.get(f.as_id()) == Some(&tag))
        });
        (result.report.region_attribution, seen)
    }

    #[test]
    fn nested_provenance_keeps_the_weakest_attribution() {
        let winner = BinaryProvenance {
            source: OperandSource::Left,
            value: 7_u32,
            blended: false,
        };
        assert_eq!(7_u32.project_region().attribution, Attribution::Sole);
        assert_eq!(winner.project_region().attribution, Attribution::Winner);
        let outer = BinaryProvenance {
            source: OperandSource::Right,
            value: BinaryProvenance {
                blended: true,
                ..winner
            },
            blended: false,
        };
        let sample = outer.project_region();
        assert_eq!(sample.region, 7);
        assert_eq!(sample.attribution, Attribution::Blended);
    }

    #[test]
    fn composites_extract_with_regions() {
        let (single, _) = attribution(&tagged([0.0; 3], 1.0, 1));
        assert!(single.sole > 0 && single.winner == 0 && single.blended == 0);

        let apart = Union::new(
            tagged([-1.2, 0.0, 0.0], 0.9, 1),
            tagged([1.2, 0.0, 0.0], 0.9, 2),
        );
        let (counts, seen) = attribution(&apart);
        assert_eq!(seen, [true, true]);
        assert!(counts.winner > 0 && counts.sole == 0 && counts.blended == 0);
    }

    #[test]
    fn smooth_junction_weakens_attribution_with_blend_width() {
        let (a, b) = (
            tagged([-0.7, 0.0, 0.0], 0.9, 1),
            tagged([0.7, 0.0, 0.0], 0.9, 2),
        );
        let (hard, _) = attribution(&SmoothUnion::new(a, b, 0.0));
        assert_eq!(hard.blended, 0, "a hard minimum blends nothing");
        let (narrow, seen) = attribution(&SmoothUnion::new(a, b, 0.15));
        let (wide, _) = attribution(&SmoothUnion::new(a, b, 0.6));
        assert_eq!(seen, [true, true]);
        assert!(narrow.blended > 0 && narrow.winner > narrow.blended);
        assert!(wide.blended > narrow.blended, "{narrow:?} vs {wide:?}");
        // Wherever a patch is not blended, one operand alone won it.
        assert_eq!(narrow.sole + wide.sole, 0);
    }
}
