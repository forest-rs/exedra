// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Addressable projection and guarded material edits for Exedra assemblies.
//!
//! This module owns the Addressable view and Exedra-specific material workflow
//! over an [`Assembly`](crate::Assembly). It explicitly does not own assembly
//! storage, geometry compilation, or a generic transaction policy.
//!
//! Bind an existing assembly with
//! [`Assembly::into_addressable`](crate::Assembly::into_addressable). From there
//! the usual workflow is:
//!
//! 1. obtain [`AddressableAssembly::root_locator`] or construct an exact or
//!    relative [`AssemblyLocator`];
//! 2. call [`TreeRuntime::resolve`](addressable_tree::TreeRuntime::resolve) or
//!    one of the `query_*` methods exposed through [`AddressableAssembly`];
//! 3. turn a resolved [`AssemblyLocation`] into a [`MaterialSlot`] endpoint or
//!    a revision-scoped instance handle;
//! 4. read an explanation, then use that observation to guard an atomic
//!    [`BindMaterial`] transaction.
//!
//! ```no_run
//! use addressable::{Endpoint, Guard, Query, SpaceId, Transaction};
//! use exedra_assembly::{
//!     Assembly, AssemblyAxis, AssemblyPredicate, AssemblySpace, BindMaterial,
//!     EditCapability, MaterialSlot,
//! };
//!
//! fn edit_dome(assembly: Assembly) {
//!     let mut space = assembly.into_addressable(SpaceId::<AssemblySpace>::new(7));
//!     let query = Query::one(space.root_locator())
//!         .traverse(AssemblyAxis::Descendants)
//!         .filter(AssemblyPredicate::part("crossing-dome"));
//!     let dome = space.query_one(&query).expect("one dome").into_value();
//!     let endpoint = Endpoint::new(dome.clone(), MaterialSlot::new("surface"));
//!     let observed = space.read_material(&endpoint).expect("slot reads");
//!     let guard = Guard::new(
//!         dome.referent().clone(),
//!         dome.revision(),
//!         observed.as_ref().map(|value| value.value().clone()),
//!         EditCapability::BindMaterial,
//!     );
//!     let edit = BindMaterial::new(endpoint, "restored-gold", guard);
//!     space
//!         .transact(Transaction::apply(dome.revision(), [edit]))
//!         .expect("guarded edit applies");
//! }
//! ```

mod material;
mod model;
mod space;

pub use material::{
    BindMaterial, EditCapability, MaterialChange, MaterialExplanation, MaterialProvenance,
    MaterialReason, MaterialSlot, MaterialSubject, TransactionConflict, TransactionReport,
    UndoMaterial,
};
pub use model::{
    AssemblyAxis, AssemblyLocation, AssemblyLocator, AssemblyOccurrence, AssemblyPredicate,
    AssemblyQuery, AssemblyReferent, AssemblyReferentParseError, AssemblyResolution, AssemblyView,
    AssemblyViewParseError, Measured, PartKey,
};
pub use space::{AddressableAssembly, ReadError};

impl crate::Assembly {
    /// Binds this assembly to a runtime Addressable space and revision.
    ///
    /// Build and author the assembly first, then supply a host-assigned
    /// [`SpaceId`](::addressable::SpaceId). The returned
    /// [`AddressableAssembly`] owns the assembly so every subsequent mutation
    /// can pass through its guarded transactions or revisioned `commit`
    /// gateway. Extracting the underlying assembly also returns the revision
    /// required to resume the same space safely.
    #[must_use]
    pub fn into_addressable(
        self,
        id: ::addressable::SpaceId<crate::AssemblySpace>,
    ) -> AddressableAssembly {
        AddressableAssembly::new(id, self)
    }
}
