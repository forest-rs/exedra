// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pocket validation independent of native table construction.

use super::{End, ExtrusionError, Pocket, PocketError};
use alloc::vec::Vec;
use exedra_curves::profile::{Profile, ProfilePolicy};

pub(super) fn checked_depth(
    length: f64,
    pocket: &Pocket,
    index: usize,
    policy: &ProfilePolicy,
) -> Result<(), ExtrusionError> {
    if !(pocket.depth.is_finite()
        && pocket.depth > 0.0
        && length - pocket.depth >= policy.min_separation)
    {
        return Err(ExtrusionError::Pocket {
            pocket: index,
            error: PocketError::InvalidDepth {
                depth: pocket.depth,
                length,
                remaining: length - pocket.depth,
                min_separation: policy.min_separation,
            },
        });
    }
    Ok(())
}

pub(super) fn checked_pair(
    profile: &Profile,
    length: f64,
    pockets: [&Pocket; 2],
    indices: [usize; 2],
    policy: &ProfilePolicy,
) -> Result<Option<[usize; 2]>, ExtrusionError> {
    let [pocket, existing] = pockets;
    let [index, other] = indices;
    if existing.end == pocket.end {
        return Ok(None);
    }
    let mut holes = profile.holes().to_vec();
    holes.push(existing.outline.clone());
    holes.push(pocket.outline.clone());
    if Profile::new(profile.outer().clone(), holes, policy).is_ok() {
        return Ok(None);
    }
    let (end_depth, start_depth) = if pocket.end == End::End {
        (pocket.depth, existing.depth)
    } else {
        (existing.depth, pocket.depth)
    };
    // Use the same floor coordinates stored by native construction. Comparing
    // their remaining thickness directly avoids rounding away a small web.
    let remaining = (length - end_depth) - start_depth;
    if remaining < policy.min_separation {
        return Err(ExtrusionError::Pocket {
            pocket: index,
            error: PocketError::BreaksThrough {
                other,
                remaining,
                min_separation: policy.min_separation,
            },
        });
    }
    Ok(Some(if pocket.end == End::End {
        [index, other]
    } else {
        [other, index]
    }))
}

pub(super) fn checked_webs(
    profile: &Profile,
    length: f64,
    pockets: &[Pocket],
    pocket: &Pocket,
    policy: &ProfilePolicy,
) -> Result<Vec<[usize; 2]>, ExtrusionError> {
    let index = pockets.len();
    checked_depth(length, pocket, index, policy)?;
    let mut holes = profile.holes().to_vec();
    holes.extend(
        pockets
            .iter()
            .filter(|p| p.end == pocket.end)
            .map(|p| p.outline.clone()),
    );
    holes.push(pocket.outline.clone());
    Profile::new(profile.outer().clone(), holes, policy).map_err(|error| {
        ExtrusionError::Pocket {
            pocket: index,
            error: PocketError::Arrangement(error),
        }
    })?;
    let mut webs = Vec::new();
    for (other, existing) in pockets.iter().enumerate() {
        webs.extend(checked_pair(
            profile,
            length,
            [pocket, existing],
            [index, other],
            policy,
        )?);
    }
    Ok(webs)
}
