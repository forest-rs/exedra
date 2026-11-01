// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit cumulative admission for original-source boundary proofs.
use super::super::{Curve, CurveSpan, ParameterDomain, ParameterRange, Ruled};
use exedra_curves::enclosure::PreparedNurbsEnclosure;

#[derive(Clone, Copy)]
pub(crate) struct SourceRefusal {
    pub(crate) resource: &'static str,
    pub(crate) required: u64,
    pub(crate) maximum: u64,
}
#[derive(Clone)]
pub(crate) struct SourceBudget {
    pub(crate) max_controls: usize,
    pub(crate) max_work: u64,
    pub(crate) controls: usize,
    pub(crate) metadata_work: u64,
    pub(crate) preparation_work: u64,
    pub(crate) enclosure_work: u64,
    pub(crate) refusal: Option<SourceRefusal>,
}
impl Default for SourceBudget {
    fn default() -> Self {
        Self::new(65_536, 1_000_000)
    }
}
impl SourceBudget {
    pub(crate) fn new(max_controls: usize, max_work: u64) -> Self {
        Self {
            max_controls,
            max_work,
            controls: 0,
            metadata_work: 0,
            preparation_work: 0,
            enclosure_work: 0,
            refusal: None,
        }
    }
    fn total(&self) -> u64 {
        self.metadata_work
            .saturating_add(self.preparation_work)
            .saturating_add(self.enclosure_work)
    }
    fn work(&mut self, amount: u64) -> bool {
        if self.refusal.is_some() {
            return false;
        }
        let required = self.total().checked_add(amount);
        if required.is_none_or(|required| required > self.max_work) {
            self.refusal = Some(SourceRefusal {
                resource: "source proof work",
                required: required.unwrap_or(u64::MAX),
                maximum: self.max_work,
            });
            false
        } else {
            true
        }
    }
    pub(crate) fn enclosure(&mut self) -> bool {
        if !self.work(1) {
            return false;
        }
        self.enclosure_work += 1;
        true
    }
    pub(crate) fn source(&mut self, source: &CurveSpan) -> Option<u64> {
        if self.refusal.is_some() {
            return None;
        }
        let count = match source.curve() {
            Curve::Nurbs(n) => n.curve().points().len(),
            Curve::Line(_) => 2,
            _ => return None,
        };
        let required = self.controls.checked_add(count);
        if required.is_none_or(|required| required > self.max_controls) {
            self.refusal = Some(SourceRefusal {
                resource: "source controls",
                required: required
                    .and_then(|required| u64::try_from(required).ok())
                    .unwrap_or(u64::MAX),
                maximum: u64::try_from(self.max_controls).unwrap_or(u64::MAX),
            });
            return None;
        }
        let metadata = match source.curve() {
            Curve::Nurbs(n) => u64::try_from(n.curve().knots().len()).ok()?,
            _ => 0,
        };
        if !self.work(metadata) {
            return None;
        }
        self.metadata_work += metadata;
        let preparation = match source.curve() {
            Curve::Nurbs(n) => {
                PreparedNurbsEnclosure::preparation_work_required(n.curve().view()).ok()?
            }
            _ => 0,
        };
        if !self.work(preparation) {
            return None;
        }
        self.controls = required.expect("source controls admitted");
        self.preparation_work += preparation;
        Some(preparation)
    }
    /// Inspect the known output preparation bound before cloning the result.
    /// Callback work during its later enclosures remains individually admitted.
    pub(crate) fn preflight<'a>(
        &mut self,
        faces: impl Iterator<Item = (&'a Ruled, ParameterDomain)>,
    ) -> bool {
        let mut planned = self.clone();
        let initial_preparation = self.preparation_work;
        let initial_metadata = self.metadata_work;
        for (source, domain) in faces {
            if domain.u == ParameterRange::UNIT {
                continue;
            }
            for curve in source.curves() {
                let before = planned.metadata_work;
                if planned.source(curve).is_none() {
                    self.refusal = planned.refusal;
                    return false;
                }
                // The eligibility metadata pass really happened now. Its later
                // preparation pass is accounted independently by source().
                let inspection = planned.metadata_work - before;
                if !self.work(inspection) {
                    return false;
                }
                self.metadata_work += inspection;
            }
        }
        // Actual metadata charges changed the starting budget; include them in
        // the known future preparation bound, without charging preparation yet.
        if !self.work(
            (planned.preparation_work - initial_preparation)
                .saturating_add(planned.metadata_work - initial_metadata),
        ) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maximal_policy_does_not_admit_overflowed_work_or_control_counts() {
        let mut work = SourceBudget::new(usize::MAX, u64::MAX);
        work.enclosure_work = u64::MAX;
        assert!(!work.enclosure());
        assert_eq!(work.enclosure_work, u64::MAX);
        assert!(work.refusal.is_some());
        let source = CurveSpan::new(
            Curve::line([0.; 3], [1., 0., 0.]).unwrap(),
            ParameterRange::UNIT,
            false,
        )
        .unwrap();
        let mut controls = SourceBudget::new(usize::MAX, u64::MAX);
        controls.controls = usize::MAX;
        assert!(controls.source(&source).is_none());
        assert_eq!(controls.controls, usize::MAX);
        assert!(controls.refusal.is_some());
    }
}
