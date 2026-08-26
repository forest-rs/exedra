// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic named pattern expansion into concrete assemblies.
//!
//! This module plans linear occurrences and expands them immediately into an
//! [`Assembly`]. It retains no procedural nodes: paths, placements, bindings,
//! and metadata become ordinary concrete assembly instances, so the
//! `exedra-assembly-v1` interchange contract is unchanged.
//!
//! Ordinals are authored identity. Omitting ordinal `2` suppresses that
//! occurrence without changing the names or placements of ordinals `3` and
//! later. Expansion is occurrence-major in ascending ordinal order, then
//! member slice order. Every fallible condition is preflighted before the
//! first mutation; an error therefore leaves the input assembly unchanged.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub use exedra_assembly::{Assembly, AssemblyError, InstanceId, PartId};
pub use exedra_constructive::ir::Placement3;

/// Maximum authored slots accepted by one repeat/distribute planning call.
///
/// This bounds planning work even when most slots are omitted. Larger scenes
/// can be expressed as multiple semantically named patterns.
pub const MAX_PATTERN_OCCURRENCES: u32 = 65_536;

/// Maximum concrete instances appended by one named expansion call.
///
/// The limit keeps preflight memory and collision work calm and explicit.
pub const MAX_PATTERN_INSTANCES: u32 = 65_536;

/// Absolute tolerance used to validate the rotational part of placements.
///
/// Every column must have squared length one within this tolerance, every
/// pair of columns must be orthogonal within this tolerance, and the
/// determinant must be positive and within four tolerances of one.
pub const RIGID_PLACEMENT_TOLERANCE: f64 = 1.0e-10;

/// One authored occurrence and its placement relative to the pattern parent.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinearOccurrence {
    /// Stable authored ordinal used in the generated instance key.
    pub ordinal: u32,
    /// Finite proper-rigid placement applied outside each member's local
    /// placement.
    pub placement: Placement3,
}

/// A linear repeat described by a start point and translation step.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinearRepeat<'a> {
    /// Number of authored ordinal slots before omissions.
    pub count: u32,
    /// Parent-space position of ordinal zero.
    pub start: [f64; 3],
    /// Parent-space translation added per authored ordinal.
    pub step: [f64; 3],
    /// Strictly increasing authored ordinals to suppress.
    pub omitted: &'a [u32],
}

/// Which endpoints a linear distribution includes.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum EndpointPolicy {
    /// Include both the start and end anchors.
    IncludeBoth,
    /// Include the start anchor and exclude the end anchor.
    IncludeStart,
    /// Exclude the start anchor and include the end anchor.
    IncludeEnd,
    /// Exclude both anchors.
    ExcludeBoth,
}

/// A linear distribution between two parent-space anchors.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinearDistribute<'a> {
    /// Number of authored ordinal slots before omissions.
    pub count: u32,
    /// Parent-space start anchor.
    pub start: [f64; 3],
    /// Parent-space end anchor.
    pub end: [f64; 3],
    /// Explicit endpoint inclusion policy.
    pub endpoints: EndpointPolicy,
    /// Strictly increasing authored ordinals to suppress.
    pub omitted: &'a [u32],
}

/// One material-slot binding copied to every matching concrete instance.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct MaterialBinding<'a> {
    /// Declared slot name on the member's part.
    pub slot: &'a str,
    /// Opaque material key.
    pub material: &'a str,
}

/// One opaque metadata entry copied to every matching concrete instance.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct MetadataEntry<'a> {
    /// Opaque metadata key.
    pub key: &'a str,
    /// Opaque metadata value.
    pub value: &'a str,
}

/// One member of a repeated concrete structure.
///
/// An empty [`Self::key_suffix`] makes the member use the occurrence key
/// directly. A non-empty suffix appends `-suffix` to that key.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct InstanceTemplate<'a> {
    /// Stable key suffix within one occurrence, or empty for a single-part
    /// occurrence.
    pub key_suffix: &'a str,
    /// Registered concrete part to instantiate.
    pub part: PartId,
    /// Finite proper-rigid member placement local to the occurrence.
    pub placement: Placement3,
    /// Material bindings applied in declaration order.
    pub bindings: &'a [MaterialBinding<'a>],
    /// Metadata entries applied in declaration order.
    pub metadata: &'a [MetadataEntry<'a>],
}

/// Naming, parent, and members shared by a sequence of occurrences.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct NamedAssemblyPattern<'a> {
    /// Existing parent instance, or `None` for root instances.
    pub parent: Option<InstanceId>,
    /// Stable key prefix placed before `-ordinal`.
    pub key_prefix: &'a str,
    /// Caller-fixed minimum decimal width of the ordinal.
    ///
    /// The width is never inferred from the occurrence count, so growing a
    /// pattern does not rename earlier occurrences.
    pub ordinal_width: u8,
    /// Members inserted for every occurrence, in this exact order.
    pub members: &'a [InstanceTemplate<'a>],
}

/// Which stage supplied an invalid placement.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum PlacementSite {
    /// The authored occurrence placement.
    Occurrence,
    /// A member template's local placement.
    Member,
    /// The composed `occurrence ∘ member` placement.
    Composed,
}

/// Why a placement is invalid for concrete pattern expansion.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum PlacementViolation {
    /// At least one matrix element is NaN or infinity.
    NonFinite,
    /// The rotational determinant is negative.
    Reflecting,
    /// The rotational columns contain scale, shear, or degeneracy.
    NonRigid,
}

/// Typed failure while planning or instantiating an assembly pattern.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PatternError {
    /// A coordinate input contains NaN or infinity.
    NonFiniteCoordinate {
        /// Stable input name (`repeat.start`, `repeat.step`, and so on).
        input: &'static str,
    },
    /// An omitted ordinal falls outside the authored slot count.
    OmittedOrdinalOutOfRange {
        /// Invalid ordinal.
        ordinal: u32,
        /// Number of authored slots.
        count: u32,
    },
    /// Omitted ordinals must be strictly increasing and unique.
    OmittedOrdinalsNotIncreasing {
        /// Earlier adjacent ordinal.
        previous: u32,
        /// Later adjacent ordinal that did not increase.
        current: u32,
    },
    /// Including both endpoints needs at least two authored slots.
    InclusiveDistributionNeedsTwoSlots,
    /// A pattern must contain at least one member.
    EmptyMembers,
    /// A key prefix or suffix is invalid for an assembly instance key.
    InvalidKeyFragment {
        /// Stable fragment kind (`prefix` or `member suffix`).
        fragment: &'static str,
        /// Invalid fragment value.
        value: String,
    },
    /// Two members declare the same suffix.
    DuplicateMemberSuffix(String),
    /// A member declares one material slot more than once.
    DuplicateBindingSlot(String),
    /// A member declares one metadata key more than once.
    DuplicateMetadataKey(String),
    /// The requested pattern parent is not in the assembly.
    UnknownParent(InstanceId),
    /// A member references a part not registered in the assembly.
    UnknownPart(PartId),
    /// A member binding references an undeclared part slot.
    UnknownSlot {
        /// Member part whose slot table was consulted.
        part: PartId,
        /// Unknown slot name.
        slot: String,
    },
    /// Custom occurrences must be strictly increasing by ordinal.
    OccurrencesNotIncreasing {
        /// Earlier adjacent ordinal.
        previous: u32,
        /// Later adjacent ordinal that did not increase.
        current: u32,
    },
    /// An occurrence, member, or composed placement is not proper-rigid.
    InvalidPlacement {
        /// Stage at which the invalid placement was observed.
        site: PlacementSite,
        /// Authored occurrence ordinal, when the stage has one.
        ordinal: Option<u32>,
        /// Member suffix, empty for a single-part occurrence.
        member_suffix: String,
        /// Finiteness, reflection, or rigidity failure.
        violation: PlacementViolation,
    },
    /// A generated key collides with an existing or generated sibling key.
    DuplicateInstanceKey(String),
    /// Planning or expansion exceeds its documented work budget.
    ExpansionTooLarge {
        /// Requested authored slots or concrete instances.
        requested: usize,
        /// Applicable public work limit.
        limit: u32,
    },
    /// Appending would exceed the `u32` [`InstanceId`] domain.
    InstanceIdSpaceExhausted {
        /// Existing concrete instance count.
        existing: usize,
        /// Requested additional instance count.
        additions: usize,
    },
    /// Assembly mutation failed after successful preflight.
    ///
    /// Expansion applies to an isolated candidate assembly, so this error
    /// still leaves the caller's assembly unchanged.
    AssemblyMutation(AssemblyError),
}

impl core::fmt::Display for PatternError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonFiniteCoordinate { input } => {
                write!(f, "pattern coordinate input {input} must be finite")
            }
            Self::OmittedOrdinalOutOfRange { ordinal, count } => {
                write!(f, "omitted ordinal {ordinal} is outside count {count}")
            }
            Self::OmittedOrdinalsNotIncreasing { previous, current } => write!(
                f,
                "omitted ordinals must increase: {previous} precedes {current}"
            ),
            Self::InclusiveDistributionNeedsTwoSlots => {
                f.write_str("including both endpoints requires at least two slots")
            }
            Self::EmptyMembers => f.write_str("an assembly pattern needs at least one member"),
            Self::InvalidKeyFragment { fragment, value } => {
                write!(f, "invalid {fragment} {value:?}")
            }
            Self::DuplicateMemberSuffix(suffix) => {
                write!(f, "member suffix {suffix:?} is declared more than once")
            }
            Self::DuplicateBindingSlot(slot) => {
                write!(f, "material slot {slot:?} is bound more than once")
            }
            Self::DuplicateMetadataKey(key) => {
                write!(f, "metadata key {key:?} is declared more than once")
            }
            Self::UnknownParent(parent) => write!(f, "unknown pattern parent {parent:?}"),
            Self::UnknownPart(part) => write!(f, "unknown pattern member part {part:?}"),
            Self::UnknownSlot { part, slot } => {
                write!(f, "part {part:?} declares no slot named {slot:?}")
            }
            Self::OccurrencesNotIncreasing { previous, current } => write!(
                f,
                "occurrence ordinals must increase: {previous} precedes {current}"
            ),
            Self::InvalidPlacement {
                site,
                ordinal,
                member_suffix,
                violation,
            } => write!(
                f,
                "{site:?} placement for ordinal {ordinal:?}, member {member_suffix:?} is {violation:?}"
            ),
            Self::DuplicateInstanceKey(key) => {
                write!(f, "instance key {key:?} already exists in the sibling set")
            }
            Self::ExpansionTooLarge { requested, limit } => write!(
                f,
                "pattern work {requested} cannot be reserved within the documented limit {limit}"
            ),
            Self::InstanceIdSpaceExhausted {
                existing,
                additions,
            } => write!(
                f,
                "{existing} existing plus {additions} new instances exceed the InstanceId domain"
            ),
            Self::AssemblyMutation(error) => write!(f, "assembly mutation failed: {error}"),
        }
    }
}

impl core::error::Error for PatternError {}

/// Plans occurrences for a linear repeat.
///
/// Position `i` is computed directly as `start + i * step`; placements are
/// not accumulated from the previous occurrence. Omitted ordinals retain
/// their positions and names as holes in the authored sequence.
///
/// # Errors
///
/// Returns [`PatternError`] for non-finite coordinates, invalid omission
/// order/range, or a generated non-finite placement.
pub fn repeat_linear(spec: &LinearRepeat<'_>) -> Result<Vec<LinearOccurrence>, PatternError> {
    validate_occurrence_count(spec.count)?;
    validate_vector("repeat.start", spec.start)?;
    validate_vector("repeat.step", spec.step)?;
    build_occurrences(spec.count, spec.omitted, |ordinal| {
        let i = f64::from(ordinal);
        [
            spec.start[0] + i * spec.step[0],
            spec.start[1] + i * spec.step[1],
            spec.start[2] + i * spec.step[2],
        ]
    })
}

/// Plans occurrences distributed between two anchors.
///
/// `count` is the number of authored ordinal slots before omissions. The
/// endpoint policy determines the interpolation parameter for each ordinal;
/// omissions do not close the resulting spatial gap.
///
/// # Errors
///
/// Returns [`PatternError`] for non-finite coordinates, invalid omission
/// order/range, an inclusive-both singleton, or a generated non-finite
/// placement.
pub fn distribute_linear(
    spec: &LinearDistribute<'_>,
) -> Result<Vec<LinearOccurrence>, PatternError> {
    validate_occurrence_count(spec.count)?;
    validate_vector("distribute.start", spec.start)?;
    validate_vector("distribute.end", spec.end)?;
    if spec.count == 1 && spec.endpoints == EndpointPolicy::IncludeBoth {
        return Err(PatternError::InclusiveDistributionNeedsTwoSlots);
    }
    build_occurrences(spec.count, spec.omitted, |ordinal| {
        let i = f64::from(ordinal);
        let count = f64::from(spec.count);
        let t = match spec.endpoints {
            EndpointPolicy::IncludeBoth => i / (count - 1.0),
            EndpointPolicy::IncludeStart => i / count,
            EndpointPolicy::IncludeEnd => (i + 1.0) / count,
            EndpointPolicy::ExcludeBoth => (i + 1.0) / (count + 1.0),
        };
        interpolate(spec.start, spec.end, t)
    })
}

/// Expands named occurrences into ordinary concrete assembly instances.
///
/// A generated key is `prefix-ordinal` for an empty member suffix and
/// `prefix-ordinal-suffix` otherwise. The ordinal uses the caller-fixed
/// minimum decimal width. Placement composition is
/// `occurrence ∘ member`, matching [`exedra_assembly::compose`].
///
/// The function preflights every reference, key, binding, metadata
/// declaration, collision, occurrence order, work budget, `InstanceId` space,
/// and placement. Occurrence, member, and composed placements must all be
/// finite and proper-rigid. If it returns an error, `assembly` is unchanged.
/// On success it appends instances in occurrence-major, then member declaration
/// order and returns IDs in that same order.
///
/// Cheap shape gates run first: an empty member slice has constant-time
/// precedence, followed by occurrence/member work limits and their product.
/// No key, part, placement, omission, or collision traversal happens for an
/// over-budget shape.
///
/// # Errors
///
/// Returns [`PatternError`] when any preflight invariant fails.
pub fn instantiate_named(
    assembly: &mut Assembly,
    pattern: &NamedAssemblyPattern<'_>,
    occurrences: &[LinearOccurrence],
) -> Result<Vec<InstanceId>, PatternError> {
    let capacity = validate_expansion_shape(
        assembly.instances().len(),
        occurrences.len(),
        pattern.members.len(),
    )?;
    validate_pattern(assembly, pattern, occurrences)?;
    let mut prepared = Vec::new();
    prepared
        .try_reserve_exact(capacity)
        .map_err(|_| PatternError::ExpansionTooLarge {
            requested: capacity,
            limit: MAX_PATTERN_INSTANCES,
        })?;
    let mut generated = BTreeSet::new();
    let existing = sibling_keys(assembly, pattern.parent)?;
    for occurrence in occurrences {
        for (member_index, member) in pattern.members.iter().enumerate() {
            let key = instance_key(pattern, occurrence.ordinal, member.key_suffix);
            if existing.contains(key.as_str()) || !generated.insert(key.clone()) {
                return Err(PatternError::DuplicateInstanceKey(key));
            }
            let placement = exedra_assembly::compose(&occurrence.placement, &member.placement);
            validate_placement(
                &placement,
                PlacementSite::Composed,
                Some(occurrence.ordinal),
                member.key_suffix,
            )?;
            prepared.push(PreparedInstance {
                key,
                member_index,
                placement,
            });
        }
    }

    if prepared.is_empty() {
        return Ok(Vec::new());
    }
    let mut candidate = assembly.clone();
    let mut ids = Vec::new();
    ids.try_reserve_exact(prepared.len())
        .map_err(|_| PatternError::ExpansionTooLarge {
            requested: prepared.len(),
            limit: MAX_PATTERN_INSTANCES,
        })?;
    for prepared in prepared {
        let member = &pattern.members[prepared.member_index];
        let id = candidate
            .add_instance(
                pattern.parent,
                &prepared.key,
                member.part,
                prepared.placement,
            )
            .map_err(PatternError::AssemblyMutation)?;
        for binding in member.bindings {
            candidate
                .bind_material(id, binding.slot, binding.material)
                .map_err(PatternError::AssemblyMutation)?;
        }
        for entry in member.metadata {
            candidate
                .set_metadata(id, entry.key, entry.value)
                .map_err(PatternError::AssemblyMutation)?;
        }
        ids.push(id);
    }
    *assembly = candidate;
    Ok(ids)
}

struct PreparedInstance {
    key: String,
    member_index: usize,
    placement: Placement3,
}

fn build_occurrences(
    count: u32,
    omitted: &[u32],
    mut position: impl FnMut(u32) -> [f64; 3],
) -> Result<Vec<LinearOccurrence>, PatternError> {
    validate_occurrence_count(count)?;
    validate_omissions(count, omitted)?;
    let count_usize = usize::try_from(count).map_err(|_| PatternError::ExpansionTooLarge {
        requested: usize::MAX,
        limit: MAX_PATTERN_OCCURRENCES,
    })?;
    let capacity = count_usize - omitted.len();
    let mut occurrences = Vec::new();
    occurrences
        .try_reserve_exact(capacity)
        .map_err(|_| PatternError::ExpansionTooLarge {
            requested: capacity,
            limit: MAX_PATTERN_OCCURRENCES,
        })?;
    let mut omitted_index = 0;
    for ordinal in 0..count {
        if omitted.get(omitted_index) == Some(&ordinal) {
            omitted_index += 1;
            continue;
        }
        let point = position(ordinal);
        if point.iter().any(|value| !value.is_finite()) {
            return Err(PatternError::InvalidPlacement {
                site: PlacementSite::Occurrence,
                ordinal: Some(ordinal),
                member_suffix: String::new(),
                violation: PlacementViolation::NonFinite,
            });
        }
        occurrences.push(LinearOccurrence {
            ordinal,
            placement: Placement3::translate(point[0], point[1], point[2]),
        });
    }
    Ok(occurrences)
}

fn validate_occurrence_count(count: u32) -> Result<(), PatternError> {
    if count > MAX_PATTERN_OCCURRENCES {
        Err(PatternError::ExpansionTooLarge {
            requested: usize::try_from(count).unwrap_or(usize::MAX),
            limit: MAX_PATTERN_OCCURRENCES,
        })
    } else {
        Ok(())
    }
}

fn validate_vector(input: &'static str, vector: [f64; 3]) -> Result<(), PatternError> {
    if vector.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(PatternError::NonFiniteCoordinate { input })
    }
}

fn validate_omissions(count: u32, omitted: &[u32]) -> Result<(), PatternError> {
    for &ordinal in omitted {
        if ordinal >= count {
            return Err(PatternError::OmittedOrdinalOutOfRange { ordinal, count });
        }
    }
    for pair in omitted.windows(2) {
        if pair[0] >= pair[1] {
            return Err(PatternError::OmittedOrdinalsNotIncreasing {
                previous: pair[0],
                current: pair[1],
            });
        }
    }
    Ok(())
}

fn validate_pattern(
    assembly: &Assembly,
    pattern: &NamedAssemblyPattern<'_>,
    occurrences: &[LinearOccurrence],
) -> Result<(), PatternError> {
    validate_key_fragment("prefix", pattern.key_prefix, false)?;
    if let Some(parent) = pattern.parent
        && assembly.instance(parent).is_none()
    {
        return Err(PatternError::UnknownParent(parent));
    }

    let mut suffixes = BTreeSet::new();
    for member in pattern.members {
        validate_key_fragment("member suffix", member.key_suffix, true)?;
        if !suffixes.insert(member.key_suffix) {
            return Err(PatternError::DuplicateMemberSuffix(
                member.key_suffix.to_string(),
            ));
        }
        let part = assembly
            .part(member.part)
            .ok_or(PatternError::UnknownPart(member.part))?;
        validate_placement(
            &member.placement,
            PlacementSite::Member,
            None,
            member.key_suffix,
        )?;
        let mut slots = BTreeSet::new();
        for binding in member.bindings {
            if !slots.insert(binding.slot) {
                return Err(PatternError::DuplicateBindingSlot(binding.slot.to_string()));
            }
            if part.slot_index(binding.slot).is_none() {
                return Err(PatternError::UnknownSlot {
                    part: member.part,
                    slot: binding.slot.to_string(),
                });
            }
        }
        let mut metadata = BTreeSet::new();
        for entry in member.metadata {
            if !metadata.insert(entry.key) {
                return Err(PatternError::DuplicateMetadataKey(entry.key.to_string()));
            }
        }
    }

    for pair in occurrences.windows(2) {
        if pair[0].ordinal >= pair[1].ordinal {
            return Err(PatternError::OccurrencesNotIncreasing {
                previous: pair[0].ordinal,
                current: pair[1].ordinal,
            });
        }
    }
    for occurrence in occurrences {
        validate_placement(
            &occurrence.placement,
            PlacementSite::Occurrence,
            Some(occurrence.ordinal),
            "",
        )?;
    }
    Ok(())
}

fn validate_key_fragment(
    fragment: &'static str,
    value: &str,
    empty_allowed: bool,
) -> Result<(), PatternError> {
    if (!empty_allowed && value.is_empty()) || value.contains('/') {
        Err(PatternError::InvalidKeyFragment {
            fragment,
            value: value.to_string(),
        })
    } else {
        Ok(())
    }
}

fn sibling_keys(
    assembly: &Assembly,
    parent: Option<InstanceId>,
) -> Result<BTreeSet<&str>, PatternError> {
    let ids = match parent {
        Some(parent) => assembly
            .instance(parent)
            .ok_or(PatternError::UnknownParent(parent))?
            .children(),
        None => assembly.roots(),
    };
    Ok(ids
        .iter()
        .filter_map(|&id| assembly.instance(id))
        .map(exedra_assembly::Instance::key)
        .collect())
}

fn instance_key(pattern: &NamedAssemblyPattern<'_>, ordinal: u32, suffix: &str) -> String {
    let width = usize::from(pattern.ordinal_width);
    let occurrence = format!("{}-{ordinal:0width$}", pattern.key_prefix);
    if suffix.is_empty() {
        occurrence
    } else {
        format!("{occurrence}-{suffix}")
    }
}

fn interpolate(start: [f64; 3], end: [f64; 3], t: f64) -> [f64; 3] {
    if t == 0.0 {
        return start;
    }
    if t == 1.0 {
        return end;
    }
    let start_weight = 1.0 - t;
    [
        start_weight * start[0] + t * end[0],
        start_weight * start[1] + t * end[1],
        start_weight * start[2] + t * end[2],
    ]
}

fn validate_expansion_shape(
    existing: usize,
    occurrences: usize,
    members: usize,
) -> Result<usize, PatternError> {
    if members == 0 {
        return Err(PatternError::EmptyMembers);
    }
    let occurrence_limit = usize::try_from(MAX_PATTERN_OCCURRENCES).unwrap_or(usize::MAX);
    if occurrences > occurrence_limit {
        return Err(PatternError::ExpansionTooLarge {
            requested: occurrences,
            limit: MAX_PATTERN_OCCURRENCES,
        });
    }
    let instance_limit = usize::try_from(MAX_PATTERN_INSTANCES).unwrap_or(usize::MAX);
    if members > instance_limit {
        return Err(PatternError::ExpansionTooLarge {
            requested: members,
            limit: MAX_PATTERN_INSTANCES,
        });
    }
    let additions = occurrences
        .checked_mul(members)
        .ok_or(PatternError::ExpansionTooLarge {
            requested: usize::MAX,
            limit: MAX_PATTERN_INSTANCES,
        })?;
    validate_expansion_capacity(existing, additions)?;
    Ok(additions)
}

fn validate_expansion_capacity(existing: usize, additions: usize) -> Result<(), PatternError> {
    let limit = usize::try_from(MAX_PATTERN_INSTANCES).unwrap_or(usize::MAX);
    if additions > limit {
        return Err(PatternError::ExpansionTooLarge {
            requested: additions,
            limit: MAX_PATTERN_INSTANCES,
        });
    }
    let id_limit = usize::try_from(u32::MAX).unwrap_or(usize::MAX);
    if existing
        .checked_add(additions)
        .is_none_or(|total| total > id_limit)
    {
        return Err(PatternError::InstanceIdSpaceExhausted {
            existing,
            additions,
        });
    }
    Ok(())
}

fn validate_placement(
    placement: &Placement3,
    site: PlacementSite,
    ordinal: Option<u32>,
    member_suffix: &str,
) -> Result<(), PatternError> {
    if let Some(violation) = placement_violation(placement) {
        Err(PatternError::InvalidPlacement {
            site,
            ordinal,
            member_suffix: member_suffix.to_string(),
            violation,
        })
    } else {
        Ok(())
    }
}

fn placement_violation(placement: &Placement3) -> Option<PlacementViolation> {
    if placement
        .rows
        .iter()
        .any(|row| row.iter().any(|value| !value.is_finite()))
    {
        return Some(PlacementViolation::NonFinite);
    }
    let r = &placement.rows;
    let determinant = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
        - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
        + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
    if determinant < 0.0 {
        return Some(PlacementViolation::Reflecting);
    }
    let columns = [
        [r[0][0], r[1][0], r[2][0]],
        [r[0][1], r[1][1], r[2][1]],
        [r[0][2], r[1][2], r[2][2]],
    ];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let unit_columns = columns
        .iter()
        .all(|&column| (dot(column, column) - 1.0).abs() <= RIGID_PLACEMENT_TOLERANCE);
    let orthogonal = dot(columns[0], columns[1]).abs() <= RIGID_PLACEMENT_TOLERANCE
        && dot(columns[0], columns[2]).abs() <= RIGID_PLACEMENT_TOLERANCE
        && dot(columns[1], columns[2]).abs() <= RIGID_PLACEMENT_TOLERANCE;
    let proper_determinant = (determinant - 1.0).abs() <= 4.0 * RIGID_PLACEMENT_TOLERANCE;
    if unit_columns && orthogonal && proper_determinant {
        None
    } else {
        Some(PlacementViolation::NonRigid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use exedra_assembly::assembly_fingerprint;
    use exedra_constructive::builders;
    use exedra_constructive::ir::{CapMode, NodeKind, RecipeBuilder};

    fn add_part(assembly: &mut Assembly, key: &str) -> PartId {
        let mut builder = RecipeBuilder::new();
        let surface = builder.material_slot("surface");
        let profile = builder.add_profile(builders::rect(1.0, 1.0).expect("valid rectangle"));
        let root = builder
            .with_material(surface)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid extrusion");
        assembly
            .add_recipe_part(key, builder.finish(root).expect("valid recipe"))
            .expect("unique part")
    }

    fn x_positions(occurrences: &[LinearOccurrence]) -> Vec<(u32, f64)> {
        occurrences
            .iter()
            .map(|item| (item.ordinal, item.placement.rows[0][3]))
            .collect()
    }

    #[test]
    fn repeat_uses_authored_ordinals_across_omissions() {
        let occurrences = repeat_linear(&LinearRepeat {
            count: 6,
            start: [2.0, 3.0, 4.0],
            step: [1.5, 0.0, -0.25],
            omitted: &[2],
        })
        .expect("valid repeat");
        assert_eq!(
            x_positions(&occurrences),
            vec![(0, 2.0), (1, 3.5), (3, 6.5), (4, 8.0), (5, 9.5)]
        );
        assert_eq!(occurrences[2].placement.rows[2][3], 3.25);
    }

    #[test]
    fn every_distribution_endpoint_policy_is_exact() {
        let positions = |endpoints| {
            x_positions(
                &distribute_linear(&LinearDistribute {
                    count: 3,
                    start: [0.0, 2.0, 3.0],
                    end: [12.0, 2.0, 3.0],
                    endpoints,
                    omitted: &[],
                })
                .expect("valid distribution"),
            )
        };
        assert_eq!(
            positions(EndpointPolicy::IncludeBoth),
            vec![(0, 0.0), (1, 6.0), (2, 12.0)]
        );
        assert_eq!(
            positions(EndpointPolicy::IncludeStart),
            vec![(0, 0.0), (1, 4.0), (2, 8.0)]
        );
        assert_eq!(
            positions(EndpointPolicy::IncludeEnd),
            vec![(0, 4.0), (1, 8.0), (2, 12.0)]
        );
        assert_eq!(
            positions(EndpointPolicy::ExcludeBoth),
            vec![(0, 3.0), (1, 6.0), (2, 9.0)]
        );
    }

    #[test]
    fn distribution_omission_preserves_slot_position() {
        let occurrences = distribute_linear(&LinearDistribute {
            count: 5,
            start: [0.0, 0.0, 0.0],
            end: [8.0, 0.0, 0.0],
            endpoints: EndpointPolicy::IncludeBoth,
            omitted: &[1, 3],
        })
        .expect("valid distribution");
        assert_eq!(
            x_positions(&occurrences),
            vec![(0, 0.0), (2, 4.0), (4, 8.0)]
        );
    }

    #[test]
    fn distribution_preserves_endpoints_and_handles_extreme_interiors() {
        let occurrences = distribute_linear(&LinearDistribute {
            count: 3,
            start: [f64::MAX, 1.0e16, -7.0],
            end: [-f64::MAX, 1.0, 11.0],
            endpoints: EndpointPolicy::IncludeBoth,
            omitted: &[],
        })
        .expect("overflow-safe distribution");
        for axis in 0..3 {
            assert_eq!(
                occurrences[0].placement.rows[axis][3].to_bits(),
                [f64::MAX, 1.0e16, -7.0][axis].to_bits(),
                "start anchor must be bit-exact on axis {axis}"
            );
            assert_eq!(
                occurrences[2].placement.rows[axis][3].to_bits(),
                [-f64::MAX, 1.0, 11.0][axis].to_bits(),
                "end anchor must be bit-exact on axis {axis}"
            );
        }
        assert_eq!(occurrences[1].placement.rows[0][3], 0.0);
        assert!(
            occurrences[1]
                .placement
                .rows
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        );

        let endpoints = distribute_linear(&LinearDistribute {
            count: 2,
            start: [1.0e16, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
            endpoints: EndpointPolicy::IncludeBoth,
            omitted: &[],
        })
        .expect("exact endpoints");
        assert_eq!(
            endpoints[0].placement.rows[0][3].to_bits(),
            1.0e16_f64.to_bits()
        );
        assert_eq!(
            endpoints[1].placement.rows[0][3].to_bits(),
            1.0_f64.to_bits()
        );
    }

    #[test]
    fn named_multi_member_expansion_preserves_order_payload_and_composition() {
        let mut assembly = Assembly::new();
        let beam = add_part(&mut assembly, "beam");
        let post = add_part(&mut assembly, "post");
        let binding = [MaterialBinding {
            slot: "surface",
            material: "timber",
        }];
        let metadata = [MetadataEntry {
            key: "role",
            value: "frame_member",
        }];
        let members = [
            InstanceTemplate {
                key_suffix: "beam",
                part: beam,
                placement: Placement3::translate(0.0, 2.0, 0.0),
                bindings: &binding,
                metadata: &metadata,
            },
            InstanceTemplate {
                key_suffix: "post",
                part: post,
                placement: Placement3::translate(0.0, 0.0, 3.0),
                bindings: &binding,
                metadata: &metadata,
            },
        ];
        let pattern = NamedAssemblyPattern {
            parent: None,
            key_prefix: "frame-west",
            ordinal_width: 2,
            members: &members,
        };
        let occurrences = repeat_linear(&LinearRepeat {
            count: 4,
            start: [10.0, 0.0, 0.0],
            step: [5.0, 0.0, 0.0],
            omitted: &[1],
        })
        .expect("valid repeat");
        let ids = instantiate_named(&mut assembly, &pattern, &occurrences).expect("expands");
        let keys: Vec<&str> = ids
            .iter()
            .map(|&id| assembly.instance(id).expect("instance").key())
            .collect();
        assert_eq!(
            keys,
            [
                "frame-west-00-beam",
                "frame-west-00-post",
                "frame-west-02-beam",
                "frame-west-02-post",
                "frame-west-03-beam",
                "frame-west-03-post",
            ]
        );
        let last = assembly.instance(ids[5]).expect("last post");
        assert_eq!(last.placement().rows[0][3], 25.0);
        assert_eq!(last.placement().rows[2][3], 3.0);
        assert_eq!(last.metadata(), &[("role".into(), "frame_member".into())]);
        let slot = assembly
            .part(last.part())
            .expect("part")
            .slot_index("surface")
            .expect("slot");
        assert_eq!(
            last.binding(slot).or_else(|| assembly
                .part(last.part())
                .and_then(|part| part.default_material(slot))),
            Some("timber")
        );
        assert!(
            exedra_assembly::interchange::round_trips(&assembly),
            "expanded concrete instances must survive exedra-assembly-v1 rebuild"
        );
    }

    #[test]
    fn empty_suffix_uses_the_occurrence_key_directly() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "panel");
        let members = [InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &[],
            metadata: &[],
        }];
        let pattern = NamedAssemblyPattern {
            parent: None,
            key_prefix: "panel",
            ordinal_width: 3,
            members: &members,
        };
        let occurrences = repeat_linear(&LinearRepeat {
            count: 2,
            start: [0.0; 3],
            step: [1.0, 0.0, 0.0],
            omitted: &[],
        })
        .expect("repeat");
        let ids = instantiate_named(&mut assembly, &pattern, &occurrences).expect("expands");
        assert_eq!(assembly.instance(ids[0]).expect("first").key(), "panel-000");
        assert_eq!(
            assembly.instance(ids[1]).expect("second").key(),
            "panel-001"
        );
    }

    #[test]
    fn expansion_can_target_an_existing_parent() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        let parent = assembly
            .add_instance(None, "structure", part, Placement3::IDENTITY)
            .expect("parent");
        let members = [InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &[],
            metadata: &[],
        }];
        let occurrences = repeat_linear(&LinearRepeat {
            count: 2,
            start: [0.0; 3],
            step: [1.0, 0.0, 0.0],
            omitted: &[],
        })
        .expect("repeat");
        let ids = instantiate_named(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: Some(parent),
                key_prefix: "bay",
                ordinal_width: 2,
                members: &members,
            },
            &occurrences,
        )
        .expect("expands beneath parent");
        assert_eq!(assembly.instance(parent).expect("parent").children(), ids);
        assert_eq!(
            assembly
                .instance(ids[1])
                .expect("instance")
                .address()
                .to_string(),
            "/structure/bay-01"
        );
    }

    #[test]
    fn occurrence_rotation_composes_outside_member_offset() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        let members = [InstanceTemplate {
            key_suffix: "offset",
            part,
            placement: Placement3::translate(2.0, 0.0, 0.0),
            bindings: &[],
            metadata: &[],
        }];
        let occurrences = [LinearOccurrence {
            ordinal: 0,
            placement: Placement3::rotate_z_then_translate(
                core::f64::consts::FRAC_PI_2,
                10.0,
                0.0,
                0.0,
            ),
        }];
        let ids = instantiate_named(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: "rotated",
                ordinal_width: 2,
                members: &members,
            },
            &occurrences,
        )
        .expect("proper-rigid composition");
        let placed = assembly
            .instance(ids[0])
            .expect("placed member")
            .placement();
        assert!((placed.rows[0][3] - 10.0).abs() < 1.0e-12);
        assert!((placed.rows[1][3] - 2.0).abs() < 1.0e-12);
        assert!(placed.rows[2][3].abs() < 1.0e-12);
    }

    #[test]
    fn ordinal_width_is_a_minimum_across_decimal_growth() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        let members = [InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &[],
            metadata: &[],
        }];
        let ids = instantiate_named(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: "unit",
                ordinal_width: 2,
                members: &members,
            },
            &[
                LinearOccurrence {
                    ordinal: 99,
                    placement: Placement3::IDENTITY,
                },
                LinearOccurrence {
                    ordinal: 100,
                    placement: Placement3::translate(1.0, 0.0, 0.0),
                },
            ],
        )
        .expect("width is a minimum");
        assert_eq!(assembly.instance(ids[0]).expect("99").key(), "unit-99");
        assert_eq!(assembly.instance(ids[1]).expect("100").key(), "unit-100");
    }

    #[test]
    fn identical_inputs_produce_identical_concrete_assemblies() {
        fn build() -> Assembly {
            let mut assembly = Assembly::new();
            let part = add_part(&mut assembly, "unit");
            let metadata = [MetadataEntry {
                key: "role",
                value: "unit",
            }];
            let members = [InstanceTemplate {
                key_suffix: "",
                part,
                placement: Placement3::translate(0.0, 2.0, 0.0),
                bindings: &[],
                metadata: &metadata,
            }];
            let occurrences = distribute_linear(&LinearDistribute {
                count: 7,
                start: [1.0, 0.0, 0.0],
                end: [19.0, 0.0, 0.0],
                endpoints: EndpointPolicy::ExcludeBoth,
                omitted: &[3],
            })
            .expect("distribution");
            instantiate_named(
                &mut assembly,
                &NamedAssemblyPattern {
                    parent: None,
                    key_prefix: "unit",
                    ordinal_width: 2,
                    members: &members,
                },
                &occurrences,
            )
            .expect("expands");
            assembly
        }
        let first = build();
        let second = build();
        assert_eq!(assembly_fingerprint(&first), assembly_fingerprint(&second));
        let state = |assembly: &Assembly| {
            assembly
                .instances()
                .iter()
                .map(|item| (item.key().to_string(), item.placement().rows))
                .collect::<Vec<_>>()
        };
        assert_eq!(state(&first), state(&second));
    }

    #[derive(Debug, PartialEq)]
    struct InstanceState {
        key: String,
        part: PartId,
        parent: Option<InstanceId>,
        placement: [[f64; 4]; 3],
        bindings: Vec<(exedra_assembly::SlotIndex, String)>,
        metadata: Vec<(String, String)>,
        children: Vec<InstanceId>,
    }

    #[derive(Debug, PartialEq)]
    struct AssemblyState {
        fingerprint: u128,
        content_generation: u64,
        part_keys: Vec<String>,
        roots: Vec<InstanceId>,
        instances: Vec<InstanceState>,
        next_instance: u32,
    }

    fn assembly_state(assembly: &Assembly) -> AssemblyState {
        AssemblyState {
            fingerprint: assembly_fingerprint(assembly),
            content_generation: assembly.content_generation(),
            part_keys: assembly
                .parts()
                .iter()
                .map(|part| part.key().to_string())
                .collect(),
            roots: assembly.roots().to_vec(),
            instances: assembly
                .instances()
                .iter()
                .map(|instance| InstanceState {
                    key: instance.key().to_string(),
                    part: instance.part(),
                    parent: instance.parent(),
                    placement: instance.placement().rows,
                    bindings: instance.bindings().to_vec(),
                    metadata: instance.metadata().to_vec(),
                    children: instance.children().to_vec(),
                })
                .collect(),
            next_instance: u32::try_from(assembly.instances().len())
                .expect("test assemblies fit InstanceId"),
        }
    }

    fn assert_atomic_error(
        assembly: &mut Assembly,
        pattern: &NamedAssemblyPattern<'_>,
        occurrences: &[LinearOccurrence],
        expected: PatternError,
    ) {
        let before = assembly_state(assembly);
        assert_eq!(
            instantiate_named(assembly, pattern, occurrences),
            Err(expected)
        );
        assert_eq!(assembly_state(assembly), before);
    }

    #[test]
    fn all_instantiation_errors_are_preflighted_atomically() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        assembly
            .add_instance(None, "unit-01", part, Placement3::IDENTITY)
            .expect("seed collision");
        let valid_occurrence = [LinearOccurrence {
            ordinal: 0,
            placement: Placement3::IDENTITY,
        }];

        let no_members = NamedAssemblyPattern {
            parent: None,
            key_prefix: "empty",
            ordinal_width: 2,
            members: &[],
        };
        assert_atomic_error(
            &mut assembly,
            &no_members,
            &valid_occurrence,
            PatternError::EmptyMembers,
        );

        let member = InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &[],
            metadata: &[],
        };
        let members = [member];
        let invalid_prefix = NamedAssemblyPattern {
            parent: None,
            key_prefix: "bad/key",
            ordinal_width: 2,
            members: &members,
        };
        assert_atomic_error(
            &mut assembly,
            &invalid_prefix,
            &valid_occurrence,
            PatternError::InvalidKeyFragment {
                fragment: "prefix",
                value: "bad/key".into(),
            },
        );

        let invalid_suffix_members = [InstanceTemplate {
            key_suffix: "bad/suffix",
            ..member
        }];
        let invalid_suffix = NamedAssemblyPattern {
            parent: None,
            key_prefix: "valid",
            ordinal_width: 2,
            members: &invalid_suffix_members,
        };
        assert_atomic_error(
            &mut assembly,
            &invalid_suffix,
            &valid_occurrence,
            PatternError::InvalidKeyFragment {
                fragment: "member suffix",
                value: "bad/suffix".into(),
            },
        );

        let unknown_part_members = [InstanceTemplate {
            part: PartId(999),
            ..member
        }];
        let unknown_part = NamedAssemblyPattern {
            parent: None,
            key_prefix: "unknown",
            ordinal_width: 2,
            members: &unknown_part_members,
        };
        assert_atomic_error(
            &mut assembly,
            &unknown_part,
            &valid_occurrence,
            PatternError::UnknownPart(PartId(999)),
        );

        let unknown_parent = NamedAssemblyPattern {
            parent: Some(InstanceId(999)),
            key_prefix: "unknown-parent",
            ordinal_width: 2,
            members: &members,
        };
        assert_atomic_error(
            &mut assembly,
            &unknown_parent,
            &valid_occurrence,
            PatternError::UnknownParent(InstanceId(999)),
        );

        let bad_binding = [MaterialBinding {
            slot: "missing",
            material: "stone",
        }];
        let bad_binding_members = [InstanceTemplate {
            bindings: &bad_binding,
            ..member
        }];
        let bad_slot = NamedAssemblyPattern {
            parent: None,
            key_prefix: "bad-slot",
            ordinal_width: 2,
            members: &bad_binding_members,
        };
        assert_atomic_error(
            &mut assembly,
            &bad_slot,
            &valid_occurrence,
            PatternError::UnknownSlot {
                part,
                slot: "missing".into(),
            },
        );

        let collision = NamedAssemblyPattern {
            parent: None,
            key_prefix: "unit",
            ordinal_width: 2,
            members: &members,
        };
        assert_atomic_error(
            &mut assembly,
            &collision,
            &[
                valid_occurrence[0],
                LinearOccurrence {
                    ordinal: 1,
                    placement: Placement3::translate(1.0, 0.0, 0.0),
                },
            ],
            PatternError::DuplicateInstanceKey("unit-01".into()),
        );
        let rejected = exedra_assembly::InstanceAddress::parse("/unit-00").unwrap();
        assert!(
            assembly
                .instances()
                .iter()
                .all(|instance| instance.address() != &rejected),
            "a later collision must prevent every earlier insertion"
        );

        let bad_order = [
            LinearOccurrence {
                ordinal: 2,
                placement: Placement3::IDENTITY,
            },
            LinearOccurrence {
                ordinal: 1,
                placement: Placement3::IDENTITY,
            },
        ];
        let ordered = NamedAssemblyPattern {
            parent: None,
            key_prefix: "ordered",
            ordinal_width: 2,
            members: &members,
        };
        assert_atomic_error(
            &mut assembly,
            &ordered,
            &bad_order,
            PatternError::OccurrencesNotIncreasing {
                previous: 2,
                current: 1,
            },
        );

        let non_finite = [LinearOccurrence {
            ordinal: 0,
            placement: Placement3::translate(f64::NAN, 0.0, 0.0),
        }];
        assert_atomic_error(
            &mut assembly,
            &ordered,
            &non_finite,
            PatternError::InvalidPlacement {
                site: PlacementSite::Occurrence,
                ordinal: Some(0),
                member_suffix: String::new(),
                violation: PlacementViolation::NonFinite,
            },
        );
    }

    #[test]
    fn a_late_parent_child_collision_prevents_every_insertion() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        let parent = assembly
            .add_instance(None, "parent", part, Placement3::IDENTITY)
            .expect("parent");
        assembly
            .add_instance(Some(parent), "bay-01", part, Placement3::IDENTITY)
            .expect("existing later collision");
        let members = [InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &[],
            metadata: &[],
        }];
        let pattern = NamedAssemblyPattern {
            parent: Some(parent),
            key_prefix: "bay",
            ordinal_width: 2,
            members: &members,
        };
        let occurrences = [
            LinearOccurrence {
                ordinal: 0,
                placement: Placement3::IDENTITY,
            },
            LinearOccurrence {
                ordinal: 1,
                placement: Placement3::translate(1.0, 0.0, 0.0),
            },
        ];
        assert_atomic_error(
            &mut assembly,
            &pattern,
            &occurrences,
            PatternError::DuplicateInstanceKey("bay-01".into()),
        );
        let rejected = exedra_assembly::InstanceAddress::parse("/parent/bay-00").unwrap();
        assert!(
            assembly
                .instances()
                .iter()
                .all(|instance| instance.address() != &rejected)
        );
    }

    #[test]
    fn duplicate_member_payload_declarations_are_typed_and_atomic() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        let occurrences = [LinearOccurrence {
            ordinal: 0,
            placement: Placement3::IDENTITY,
        }];
        let duplicate_bindings = [
            MaterialBinding {
                slot: "surface",
                material: "a",
            },
            MaterialBinding {
                slot: "surface",
                material: "b",
            },
        ];
        let binding_members = [InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &duplicate_bindings,
            metadata: &[],
        }];
        assert_atomic_error(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: "binding",
                ordinal_width: 2,
                members: &binding_members,
            },
            &occurrences,
            PatternError::DuplicateBindingSlot("surface".into()),
        );

        let duplicate_metadata = [
            MetadataEntry {
                key: "role",
                value: "a",
            },
            MetadataEntry {
                key: "role",
                value: "b",
            },
        ];
        let metadata_members = [InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &[],
            metadata: &duplicate_metadata,
        }];
        assert_atomic_error(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: "metadata",
                ordinal_width: 2,
                members: &metadata_members,
            },
            &occurrences,
            PatternError::DuplicateMetadataKey("role".into()),
        );

        let duplicate_suffix_members = [
            InstanceTemplate {
                key_suffix: "same",
                part,
                placement: Placement3::IDENTITY,
                bindings: &[],
                metadata: &[],
            },
            InstanceTemplate {
                key_suffix: "same",
                part,
                placement: Placement3::IDENTITY,
                bindings: &[],
                metadata: &[],
            },
        ];
        assert_atomic_error(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: "suffix",
                ordinal_width: 2,
                members: &duplicate_suffix_members,
            },
            &occurrences,
            PatternError::DuplicateMemberSuffix("same".into()),
        );
    }

    #[test]
    fn occurrences_and_members_reject_reflection_shear_and_scale_atomically() {
        let reflection = Placement3 {
            rows: [
                [-1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        };
        let shear = Placement3 {
            rows: [
                [1.0, 0.25, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        };
        let scale = Placement3 {
            rows: [
                [2.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        };
        let zero_scale = Placement3 {
            rows: [
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        };
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        let valid_members = [InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &[],
            metadata: &[],
        }];
        let valid_pattern = NamedAssemblyPattern {
            parent: None,
            key_prefix: "occurrence",
            ordinal_width: 2,
            members: &valid_members,
        };
        for (placement, violation) in [
            (reflection, PlacementViolation::Reflecting),
            (shear, PlacementViolation::NonRigid),
            (scale, PlacementViolation::NonRigid),
            (zero_scale, PlacementViolation::NonRigid),
        ] {
            assert_atomic_error(
                &mut assembly,
                &valid_pattern,
                &[LinearOccurrence {
                    ordinal: 0,
                    placement,
                }],
                PatternError::InvalidPlacement {
                    site: PlacementSite::Occurrence,
                    ordinal: Some(0),
                    member_suffix: String::new(),
                    violation,
                },
            );
        }

        for (index, (placement, violation)) in [
            (reflection, PlacementViolation::Reflecting),
            (shear, PlacementViolation::NonRigid),
            (scale, PlacementViolation::NonRigid),
            (zero_scale, PlacementViolation::NonRigid),
        ]
        .into_iter()
        .enumerate()
        {
            let suffix = ["reflection", "shear", "scale", "zero-scale"][index];
            let members = [InstanceTemplate {
                key_suffix: suffix,
                part,
                placement,
                bindings: &[],
                metadata: &[],
            }];
            assert_atomic_error(
                &mut assembly,
                &NamedAssemblyPattern {
                    parent: None,
                    key_prefix: "member",
                    ordinal_width: 2,
                    members: &members,
                },
                &[LinearOccurrence {
                    ordinal: 0,
                    placement: Placement3::IDENTITY,
                }],
                PatternError::InvalidPlacement {
                    site: PlacementSite::Member,
                    ordinal: None,
                    member_suffix: suffix.into(),
                    violation,
                },
            );
        }
    }

    #[test]
    fn non_finite_composition_is_typed_and_atomic() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        let members = [InstanceTemplate {
            key_suffix: "offset",
            part,
            placement: Placement3::translate(f64::MAX, 0.0, 0.0),
            bindings: &[],
            metadata: &[],
        }];
        assert_atomic_error(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: "overflow",
                ordinal_width: 2,
                members: &members,
            },
            &[LinearOccurrence {
                ordinal: 0,
                placement: Placement3::translate(f64::MAX, 0.0, 0.0),
            }],
            PatternError::InvalidPlacement {
                site: PlacementSite::Composed,
                ordinal: Some(0),
                member_suffix: "offset".into(),
                violation: PlacementViolation::NonFinite,
            },
        );
    }

    #[test]
    fn planning_errors_are_typed() {
        assert_eq!(
            repeat_linear(&LinearRepeat {
                count: 2,
                start: [f64::INFINITY, 0.0, 0.0],
                step: [1.0, 0.0, 0.0],
                omitted: &[],
            }),
            Err(PatternError::NonFiniteCoordinate {
                input: "repeat.start"
            })
        );
        assert_eq!(
            repeat_linear(&LinearRepeat {
                count: 2,
                start: [0.0; 3],
                step: [1.0, 0.0, 0.0],
                omitted: &[2],
            }),
            Err(PatternError::OmittedOrdinalOutOfRange {
                ordinal: 2,
                count: 2
            })
        );
        assert_eq!(
            repeat_linear(&LinearRepeat {
                count: 3,
                start: [0.0; 3],
                step: [1.0, 0.0, 0.0],
                omitted: &[1, 1],
            }),
            Err(PatternError::OmittedOrdinalsNotIncreasing {
                previous: 1,
                current: 1
            })
        );
        assert_eq!(
            distribute_linear(&LinearDistribute {
                count: 1,
                start: [0.0; 3],
                end: [1.0, 0.0, 0.0],
                endpoints: EndpointPolicy::IncludeBoth,
                omitted: &[],
            }),
            Err(PatternError::InclusiveDistributionNeedsTwoSlots)
        );
        assert!(
            distribute_linear(&LinearDistribute {
                count: 0,
                start: [0.0; 3],
                end: [1.0, 0.0, 0.0],
                endpoints: EndpointPolicy::IncludeBoth,
                omitted: &[],
            })
            .expect("zero slots is an empty plan")
            .is_empty()
        );
        assert_eq!(
            repeat_linear(&LinearRepeat {
                count: MAX_PATTERN_OCCURRENCES + 1,
                start: [0.0; 3],
                step: [1.0, 0.0, 0.0],
                omitted: &[],
            }),
            Err(PatternError::ExpansionTooLarge {
                requested: usize::try_from(MAX_PATTERN_OCCURRENCES + 1)
                    .expect("test host fits u32"),
                limit: MAX_PATTERN_OCCURRENCES,
            })
        );
    }

    #[test]
    fn expansion_budget_and_instance_id_domain_are_preflighted() {
        let limit = usize::try_from(MAX_PATTERN_INSTANCES).expect("test host fits limit");
        assert_eq!(
            validate_expansion_capacity(0, limit + 1),
            Err(PatternError::ExpansionTooLarge {
                requested: limit + 1,
                limit: MAX_PATTERN_INSTANCES,
            })
        );
        let id_limit = usize::try_from(u32::MAX).expect("test host fits u32");
        assert_eq!(
            validate_expansion_capacity(id_limit, 1),
            Err(PatternError::InstanceIdSpaceExhausted {
                existing: id_limit,
                additions: 1,
            })
        );
    }

    #[test]
    fn work_budget_precedes_all_expensive_or_semantic_validation() {
        let mut assembly = Assembly::new();
        let part = add_part(&mut assembly, "unit");
        let member = InstanceTemplate {
            key_suffix: "",
            part,
            placement: Placement3::IDENTITY,
            bindings: &[],
            metadata: &[],
        };
        let occurrence_limit =
            usize::try_from(MAX_PATTERN_OCCURRENCES).expect("test host fits limit");
        let mut overbudget_occurrences = Vec::new();
        overbudget_occurrences.resize(
            occurrence_limit + 1,
            LinearOccurrence {
                ordinal: 0,
                placement: Placement3::IDENTITY,
            },
        );
        overbudget_occurrences[occurrence_limit].placement =
            Placement3::translate(f64::NAN, 0.0, 0.0);
        assert_atomic_error(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: "empty-precedence",
                ordinal_width: 2,
                members: &[],
            },
            &overbudget_occurrences,
            PatternError::EmptyMembers,
        );
        let members = [member];
        let occurrence_pattern = NamedAssemblyPattern {
            parent: None,
            key_prefix: "occurrence-budget",
            ordinal_width: 2,
            members: &members,
        };
        assert_atomic_error(
            &mut assembly,
            &occurrence_pattern,
            &overbudget_occurrences,
            PatternError::ExpansionTooLarge {
                requested: occurrence_limit + 1,
                limit: MAX_PATTERN_OCCURRENCES,
            },
        );

        let instance_limit = usize::try_from(MAX_PATTERN_INSTANCES).expect("test host fits limit");
        let invalid_member = InstanceTemplate {
            key_suffix: "bad/suffix",
            part: PartId(u32::MAX),
            ..member
        };
        let overbudget_members = vec![invalid_member; instance_limit + 1];
        let member_pattern = NamedAssemblyPattern {
            parent: None,
            key_prefix: "member-budget",
            ordinal_width: 2,
            members: &overbudget_members,
        };
        let member_error = PatternError::ExpansionTooLarge {
            requested: instance_limit + 1,
            limit: MAX_PATTERN_INSTANCES,
        };
        assert_atomic_error(
            &mut assembly,
            &member_pattern,
            &[LinearOccurrence {
                ordinal: 0,
                placement: Placement3::IDENTITY,
            }],
            member_error.clone(),
        );
        assert_atomic_error(&mut assembly, &member_pattern, &[], member_error);

        let product_members = vec![member; 256];
        let product_occurrences = vec![
            LinearOccurrence {
                ordinal: 0,
                placement: Placement3::IDENTITY,
            };
            257
        ];
        assert_atomic_error(
            &mut assembly,
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: "product-budget",
                ordinal_width: 2,
                members: &product_members,
            },
            &product_occurrences,
            PatternError::ExpansionTooLarge {
                requested: 256 * 257,
                limit: MAX_PATTERN_INSTANCES,
            },
        );

        let huge_omissions = vec![u32::MAX; occurrence_limit * 2];
        assert_eq!(
            repeat_linear(&LinearRepeat {
                count: MAX_PATTERN_OCCURRENCES + 1,
                start: [0.0; 3],
                step: [1.0, 0.0, 0.0],
                omitted: &huge_omissions,
            }),
            Err(PatternError::ExpansionTooLarge {
                requested: occurrence_limit + 1,
                limit: MAX_PATTERN_OCCURRENCES,
            })
        );
    }
}
