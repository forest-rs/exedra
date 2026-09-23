// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Level-of-detail chains on parts.

use alloc::vec::Vec;

use super::{Assembly, AssemblyError, InstanceId, PartDef, PartId, PlacementSetId, SlotIndex};

/// A placed occurrence of a part that binds materials: one instance or a
/// placement set.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum Occurrence {
    /// A single instance.
    Instance(InstanceId),
    /// Every placement of a placement set.
    PlacementSet(PlacementSetId),
}

/// One level of a part's level-of-detail chain.
///
/// Coverage is the placed body's projected size as a fraction of the
/// viewport height (for example, a bounding sphere's projected diameter
/// divided by the viewport height). A level is drawn while coverage is at
/// least `min_coverage` and below the previous level's `min_coverage`. Over
/// the `crossfade` band just below `min_coverage`, this level fades out while
/// the next fades in; `0.0` switches instantly. Below the last level's
/// `min_coverage` the occurrence is not drawn, so a last level of `0.0` is
/// never culled.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LodLevel {
    /// The part drawn at this level.
    pub part: PartId,
    /// Smallest screen coverage at which this level is drawn.
    pub min_coverage: f32,
    /// Width of the fade band below `min_coverage`, in coverage units.
    pub crossfade: f32,
}

impl LodLevel {
    /// A level with an instant switch.
    #[must_use]
    pub const fn new(part: PartId, min_coverage: f32) -> Self {
        Self {
            part,
            min_coverage,
            crossfade: 0.0,
        }
    }

    /// This level with a fade band of `crossfade` below `min_coverage`.
    #[must_use]
    pub const fn with_crossfade(mut self, crossfade: f32) -> Self {
        self.crossfade = crossfade;
        self
    }
}

impl PartDef {
    /// The part's level-of-detail chain, finest first, or empty when the part
    /// has none. When present, level 0 is the part itself.
    #[must_use]
    pub fn lods(&self) -> &[LodLevel] {
        &self.lods
    }
}

impl Assembly {
    /// Sets (or, with an empty vector, clears) a part's level-of-detail
    /// chain.
    ///
    /// Levels are ordered finest first, and level 0 must be `part` itself.
    /// Lower levels are ordinary parts, compiled and cached like any other;
    /// they are never placed by themselves through the chain. Chains do not
    /// nest: a lower-level part cannot have its own chain, and a part with a
    /// chain cannot be a lower level elsewhere. Several chains may share a
    /// lower-level part, such as one impostor card. Instance and
    /// placement-set material bindings, then the owning part's defaults,
    /// apply to lower levels by slot name before a level's own defaults.
    ///
    /// # Errors
    ///
    /// Fails with [`AssemblyError::UnknownPart`] for an unknown part, or
    /// [`AssemblyError::InvalidLods`] when the chain breaks the rules above
    /// or its thresholds are not finite, non-negative and strictly
    /// decreasing, or a crossfade band reaches below the next level's
    /// threshold (or below zero for the last level).
    pub fn set_part_lods(
        &mut self,
        part: PartId,
        levels: Vec<LodLevel>,
    ) -> Result<(), AssemblyError> {
        if self.parts.get(part.0 as usize).is_none() {
            return Err(AssemblyError::UnknownPart(part));
        }
        if !levels.is_empty() {
            self.validate_lods(part, &levels)?;
        }
        // Chains select among already-compiled parts; no geometry changes,
        // so `content_generation` does not advance.
        self.parts[part.0 as usize].lods = levels;
        Ok(())
    }

    fn validate_lods(&self, part: PartId, levels: &[LodLevel]) -> Result<(), AssemblyError> {
        let invalid = |reason: &'static str| Err(AssemblyError::InvalidLods { part, reason });
        if levels[0].part != part {
            return invalid("level 0 must be the part itself");
        }
        if self
            .parts
            .iter()
            .any(|def| def.lods.iter().skip(1).any(|level| level.part == part))
        {
            return invalid("the part is a lower level of another chain");
        }
        for (i, level) in levels.iter().enumerate() {
            let Some(def) = self.parts.get(level.part.0 as usize) else {
                return Err(AssemblyError::UnknownPart(level.part));
            };
            if i > 0 {
                if level.part == part || levels[..i].iter().any(|l| l.part == level.part) {
                    return invalid("a part appears twice in the chain");
                }
                if !def.lods.is_empty() {
                    return invalid("a lower-level part has its own chain");
                }
            }
            if !level.min_coverage.is_finite() || level.min_coverage < 0.0 {
                return invalid("thresholds must be finite and non-negative");
            }
            if !level.crossfade.is_finite() || level.crossfade < 0.0 {
                return invalid("crossfades must be finite and non-negative");
            }
            let floor = levels.get(i + 1).map_or(0.0, |next| next.min_coverage);
            if let Some(next) = levels.get(i + 1)
                && next.min_coverage >= level.min_coverage
            {
                return invalid("thresholds must strictly decrease");
            }
            // In f64 so a band that exactly meets the next threshold is
            // accepted regardless of `f32` subtraction rounding.
            if f64::from(level.min_coverage) - f64::from(level.crossfade) < f64::from(floor) {
                return invalid("a crossfade band reaches past the next threshold");
            }
        }
        Ok(())
    }

    /// The material key `slot` of `level` resolves to for `occurrence`.
    ///
    /// `level` is the occurrence's own part or a lower level of its chain.
    /// A lower level's slot maps to the owning part's slot of the same name,
    /// then resolves through the occurrence's binding, else the owning part's
    /// default; a slot the owner does not name falls back to the level part's
    /// own default. For the occurrence's own part this is exactly
    /// [`Assembly::resolved_material`] (or
    /// [`Assembly::resolved_placement_material`]). Returns `None` for an
    /// unknown occurrence or part, or an unbound slot.
    #[must_use]
    pub fn resolved_level_material(
        &self,
        occurrence: Occurrence,
        level: PartId,
        slot: SlotIndex,
    ) -> Option<&str> {
        let owner = match occurrence {
            Occurrence::Instance(id) => self.instances.get(id.0 as usize)?.part?,
            Occurrence::PlacementSet(id) => self.placement_sets.get(id.0 as usize)?.part(),
        };
        let binding = |owned: SlotIndex| match occurrence {
            Occurrence::Instance(id) => self.instances[id.0 as usize].binding(owned),
            Occurrence::PlacementSet(id) => self.placement_sets[id.0 as usize].binding(owned),
        };
        self.parts.get(level.0 as usize)?;
        let owned = self.owner_slot(owner, level, slot);
        owned
            .and_then(binding)
            .or_else(|| owned.and_then(|o| self.parts[owner.0 as usize].default_material(o)))
            .or_else(|| self.parts[level.0 as usize].default_material(slot))
    }

    /// Maps a lower-level part's slot to the owning part's slot of the same
    /// name, through which instance and set bindings are looked up.
    pub(crate) fn owner_slot(
        &self,
        owner: PartId,
        level_part: PartId,
        slot: SlotIndex,
    ) -> Option<SlotIndex> {
        if owner == level_part {
            return Some(slot);
        }
        let name = self.parts[level_part.0 as usize]
            .slots
            .get(slot.0 as usize)?;
        self.parts[owner.0 as usize].slot_index(name)
    }
}

#[cfg(test)]
mod tests;
