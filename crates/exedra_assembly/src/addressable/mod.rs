// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Addressable projection and read-side material explanations for Exedra assemblies.
//!
//! This module owns the Addressable view and revisioned mutation gateway over
//! an [`Assembly`](crate::Assembly). It explicitly does not own assembly
//! storage, geometry compilation, or a generic guarded-mutation protocol.
//!
//! Bind an existing assembly with
//! [`Assembly::into_addressable`](crate::Assembly::into_addressable). From there
//! the usual workflow is:
//!
//! 1. obtain [`AddressableAssembly::root_locator`] or construct an exact or
//!    relative [`AssemblyLocator`];
//! 2. call [`TreeRuntime::resolve`](addressable_tree::TreeRuntime::resolve) or
//!    one of the `query_*` methods exposed through [`AddressableAssembly`];
//! 3. consume the location as a revision-scoped instance handle or pair it
//!    with a [`MaterialSlot`] to explain the effective material.
//!
//! ```no_run
//! use addressable::{Endpoint, Query, SpaceId};
//! use exedra_assembly::{
//!     Assembly, AssemblyAxis, AssemblyPredicate, AssemblySpace, MaterialSlot,
//! };
//!
//! fn inspect_dome(assembly: Assembly) {
//!     let space = assembly.into_addressable(SpaceId::<AssemblySpace>::new(7));
//!     let query = Query::one(space.root_locator())
//!         .traverse(AssemblyAxis::Descendants)
//!         .filter(AssemblyPredicate::part("crossing-dome"));
//!     let dome = space.query_one(&query).expect("one dome").into_value();
//!     let endpoint = Endpoint::new(dome, MaterialSlot::new("surface"));
//!     let explained = space
//!         .read_material(&endpoint)
//!         .expect("slot reads")
//!         .expect("material is authored");
//!     assert!(!explained.opinions().is_empty());
//! }
//! ```

mod material;
mod model;
mod space;

pub use material::{
    MaterialExplanation, MaterialProvenance, MaterialReason, MaterialSlot, MaterialSubject,
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
    /// passes through its revisioned [`AddressableAssembly::commit`] gateway.
    /// Extracting the underlying assembly also returns the revision required to
    /// resume the same space safely.
    #[must_use]
    pub fn into_addressable(
        self,
        id: ::addressable::SpaceId<crate::AssemblySpace>,
    ) -> AddressableAssembly {
        AddressableAssembly::new(id, self)
    }
}
