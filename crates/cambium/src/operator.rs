// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Edit-operator trait boundary for Cambium.

use exedra::EditSession;
use exedra::Mesh;

use crate::plan::{PlanFingerprint, PlanHasher};
use crate::{OpContext, OpError, OpReport};

/// Canonical geometry domain owned by an operator.
///
/// This is intentionally separate from the operator's stable name. Stable names
/// describe discoverability and workflow identity; the domain tells Cambium
/// which canonical head owns the state being operated on.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum OperatorDomain {
    /// Polygon topology and authored mesh attributes (`exedra`).
    Mesh,
    /// Exact-ish analytic topology/geometry (future sibling crate).
    Analytic,
    /// Implicit field / scalar-field domain.
    Implicit,
    /// Point/curve domain.
    Points,
    /// Explicit conversion between canonical domains.
    Convert,
}

/// Primary operator trait for topology/attribute edits.
///
/// Operators mutate mesh state through an Exedra edit scope and return an
/// operator report plus a typed output payload. Scope finalization and optional
/// change recording are orchestrated by the runner.
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

    /// Canonical geometry domain owned by this operator.
    ///
    /// v0.1 operators are mesh-native by default.
    fn domain(&self) -> OperatorDomain {
        OperatorDomain::Mesh
    }

    /// Compiles deterministic operator intent from immutable mesh state.
    fn compile(
        &self,
        mesh: &Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError>;

    /// Applies an already-compiled plan.
    fn apply_plan<S: exedra::ChangeSink>(
        &self,
        txn: &mut EditSession<'_, S>,
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

    /// Applies one operator pass into an in-flight edit session.
    ///
    /// Default behavior uses the explicit lifecycle:
    /// `compile(txn.mesh(), params, ctx)` then `apply_plan(txn, plan, ctx)`.
    fn apply<S: exedra::ChangeSink>(
        &self,
        txn: &mut EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let plan = self.compile(txn.mesh(), params, ctx)?;
        self.apply_plan(txn, &plan, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::{EditOperator, OperatorDomain};
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

        fn compile(
            &self,
            _mesh: &Mesh,
            _params: &Self::Params,
            _ctx: &mut OpContext,
        ) -> Result<Self::Plan, OpError> {
            Ok(())
        }

        fn apply_plan<S: exedra::ChangeSink>(
            &self,
            _txn: &mut EditSession<'_, S>,
            _plan: &Self::Plan,
            _ctx: &mut OpContext,
        ) -> Result<(OpReport, Self::Output), OpError> {
            Ok((OpReport::new(self.name(), Artifacts::default()), ()))
        }
    }

    #[test]
    fn operator_name_uses_stable_namespace_shape() {
        let op = NoopOperator;
        assert_eq!(op.name(), "test.noop");
    }

    #[test]
    fn operator_domain_defaults_to_mesh() {
        let op = NoopOperator;
        assert_eq!(op.domain(), OperatorDomain::Mesh);
    }
}
