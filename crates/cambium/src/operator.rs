// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Edit-operator trait boundary for Cambium.

use exedra::EditSession;
use exedra::Mesh;

use crate::plan::{PlanFingerprint, PlanHasher};
use crate::{OpContext, OpError, OpReport};

/// Primary operator trait for topology/attribute edits.
///
/// Operators mutate mesh state through a transaction and return an operator
/// report plus a typed output payload. Transaction commit/abort is
/// orchestrated by the runner.
///
/// Implemented by concrete operators such as [`UvPlanar`](crate::UvPlanar),
/// [`UvBox`](crate::UvBox), [`UvCylinder`](crate::UvCylinder),
/// [`TagFaceRegion`](crate::TagFaceRegion),
/// [`MarkEdgeSeam`](crate::MarkEdgeSeam), and
/// [`MarkEdgeSharp`](crate::MarkEdgeSharp).
///
/// This trait is intentionally not object-safe in v0.1 because `Params` is an
/// associated type. Cambium currently favors static dispatch.
pub trait EditOperator {
    /// Input parameter payload for this operator.
    type Params: Clone;
    /// Typed authoritative output payload for chaining.
    type Output;
    /// Deterministic compiled plan payload.
    type Plan: Clone;

    /// Stable dot-separated operator identifier (for example: `"uv.planar"`).
    fn name(&self) -> &'static str;

    /// Compiles deterministic operator intent from immutable mesh state.
    ///
    /// Default behavior is a direct clone of `params`.
    fn compile(
        &self,
        mesh: &Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError>;

    /// Applies an already-compiled plan.
    ///
    /// Default behavior forwards to [`Self::apply`] with the compiled payload.
    fn apply_plan(
        &self,
        txn: &mut EditSession<'_>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError>;

    /// Produces a deterministic fingerprint for a compiled plan.
    ///
    /// Operators with custom plan payloads should override this to include all
    /// semantically relevant fields.
    fn plan_fingerprint(&self, _plan: &Self::Plan) -> PlanFingerprint {
        let mut hasher = PlanHasher::new();
        hasher.write_str(self.name());
        hasher.finish()
    }

    /// Applies one operator pass into an in-flight transaction.
    fn apply(
        &self,
        txn: &mut EditSession<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError>;
}

#[cfg(test)]
mod tests {
    use super::EditOperator;
    use crate::{Artifacts, OpContext, OpError, OpReport};
    use exedra::{EditSession, Mesh};

    struct NoopOperator;

    impl EditOperator for NoopOperator {
        type Params = ();
        type Plan = ();
        type Output = ();

        fn name(&self) -> &'static str {
            "test.noop"
        }

        fn apply(
            &self,
            _txn: &mut EditSession<'_>,
            _params: &Self::Params,
            _ctx: &mut OpContext,
        ) -> Result<(OpReport, Self::Output), OpError> {
            Ok((OpReport::new(self.name(), Artifacts::default()), ()))
        }

        fn compile(
            &self,
            _mesh: &Mesh,
            _params: &Self::Params,
            _ctx: &mut OpContext,
        ) -> Result<Self::Plan, OpError> {
            Ok(())
        }

        fn apply_plan(
            &self,
            txn: &mut EditSession<'_>,
            _plan: &Self::Plan,
            ctx: &mut OpContext,
        ) -> Result<(OpReport, Self::Output), OpError> {
            self.apply(txn, &(), ctx)
        }
    }

    #[test]
    fn operator_name_uses_stable_namespace_shape() {
        let op = NoopOperator;
        assert_eq!(op.name(), "test.noop");
    }
}
