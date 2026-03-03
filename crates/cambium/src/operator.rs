// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Edit-operator trait boundary for Cambium.

use exedra::Txn;

use crate::{OpContext, OpError, OpReport};

/// Primary operator trait for topology/attribute edits.
///
/// Operators mutate mesh state through a transaction and return an operator
/// report. Transaction commit/abort is orchestrated by the runner.
///
/// This trait is intentionally not object-safe in v0.1 because `Params` is an
/// associated type. Cambium currently favors static dispatch.
pub trait EditOperator {
    /// Input parameter payload for this operator.
    type Params;

    /// Stable dot-separated operator identifier (for example: `"uv.planar"`).
    fn name(&self) -> &'static str;

    /// Applies one operator pass into an in-flight transaction.
    fn apply(
        &self,
        txn: &mut Txn<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<OpReport, OpError>;
}

#[cfg(test)]
mod tests {
    use super::EditOperator;
    use crate::{Artifacts, OpContext, OpError, OpReport};
    use exedra::Txn;

    struct NoopOperator;

    impl EditOperator for NoopOperator {
        type Params = ();

        fn name(&self) -> &'static str {
            "test.noop"
        }

        fn apply(
            &self,
            _txn: &mut Txn<'_>,
            _params: &Self::Params,
            _ctx: &mut OpContext,
        ) -> Result<OpReport, OpError> {
            Ok(OpReport::new(self.name(), Artifacts::default()))
        }
    }

    #[test]
    fn operator_name_uses_stable_namespace_shape() {
        let op = NoopOperator;
        assert_eq!(op.name(), "test.noop");
    }
}
