// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Assembly projection into the reusable Addressable tree runtime.

use core::ops::Deref;

use addressable::{AbsoluteAddress, Locator, ResolvedHandle, Revision, SpaceId};
use addressable_tree::{HostNode, TreeHost, TreeNode, TreeReadError, TreeRuntime};

use crate::{Assembly, AssemblySpace, Instance, InstanceAddress, InstanceId};

use super::{
    AssemblyLocator, AssemblyOccurrence, AssemblyPredicate, AssemblyReferent, AssemblyView, PartKey,
};

impl Assembly {
    fn projected_instance(&self, id: InstanceId) -> Option<HostNode<Self>> {
        let instance = self.instance(id)?;
        let part = self.part(instance.part())?;
        Some(TreeNode::new(
            AssemblyReferent::Part(PartKey::from(part.key())),
            AssemblyOccurrence::Instance(instance.address().clone()),
            instance.address().clone(),
            Some(id),
        ))
    }

    fn projected_root() -> HostNode<Self> {
        TreeNode::new(
            AssemblyReferent::Assembly,
            AssemblyOccurrence::Assembly,
            AbsoluteAddress::root(),
            None,
        )
    }
}

impl TreeHost for Assembly {
    type Space = AssemblySpace;
    type View = AssemblyView;
    type Referent = AssemblyReferent;
    type Occurrence = AssemblyOccurrence;
    type Handle = InstanceId;
    type Predicate = AssemblyPredicate;

    fn supports_view(&self, view: &AssemblyView) -> bool {
        *view == AssemblyView::Instances
    }

    fn node_at(&self, _view: &AssemblyView, address: &InstanceAddress) -> Option<HostNode<Self>> {
        if address.depth() == 0 {
            return Some(Self::projected_root());
        }
        self.instance_by_address(address)
            .and_then(|id| self.projected_instance(id))
    }

    fn nodes(&self, _view: &AssemblyView) -> impl Iterator<Item = HostNode<Self>> + '_ {
        core::iter::once(Self::projected_root()).chain(
            self.instances_with_ids()
                .filter_map(|(id, _)| self.projected_instance(id)),
        )
    }

    fn occurrences_of<'a>(
        &'a self,
        _view: &'a AssemblyView,
        referent: &'a AssemblyReferent,
    ) -> impl Iterator<Item = HostNode<Self>> + 'a {
        let root = matches!(referent, AssemblyReferent::Assembly).then(Self::projected_root);
        let part = referent
            .as_part()
            .and_then(|key| self.part_by_key(key.as_str()));
        root.into_iter().chain(
            self.instances_with_ids()
                .filter(move |(_, instance)| Some(instance.part()) == part)
                .filter_map(|(id, _)| self.projected_instance(id)),
        )
    }

    fn children<'a>(
        &'a self,
        _view: &'a AssemblyView,
        occurrence: &'a AssemblyOccurrence,
    ) -> impl Iterator<Item = HostNode<Self>> + 'a {
        let children = match occurrence {
            AssemblyOccurrence::Assembly => self.roots(),
            AssemblyOccurrence::Instance(address) => self
                .instance_by_address(address)
                .and_then(|id| self.instance(id))
                .map_or_else(|| &[] as &[InstanceId], Instance::children),
        };
        children
            .iter()
            .filter_map(|id| self.projected_instance(*id))
    }

    fn parent(
        &self,
        _view: &AssemblyView,
        occurrence: &AssemblyOccurrence,
    ) -> Option<HostNode<Self>> {
        let AssemblyOccurrence::Instance(address) = occurrence else {
            return None;
        };
        let instance = self.instance(self.instance_by_address(address)?)?;
        instance.parent().map_or_else(
            || Some(Self::projected_root()),
            |id| self.projected_instance(id),
        )
    }

    fn matches(&self, node: &HostNode<Self>, predicate: &AssemblyPredicate) -> bool {
        match predicate {
            AssemblyPredicate::Any => true,
            AssemblyPredicate::Part(key) => node.referent().as_part() == Some(key),
            AssemblyPredicate::MetadataEquals { key, value } => {
                let AssemblyOccurrence::Instance(address) = node.occurrence() else {
                    return false;
                };
                self.instance_by_address(address)
                    .and_then(|id| self.instance(id))
                    .is_some_and(|instance| {
                        instance
                            .metadata()
                            .iter()
                            .any(|(actual_key, actual_value)| {
                                actual_key == key.as_ref() && actual_value == value.as_ref()
                            })
                    })
            }
        }
    }
}

/// Addressable assembly API with Exedra's read-side material vocabulary.
///
/// Construct this with [`Assembly::into_addressable`]. Locator resolution,
/// pinned references, typed tree queries, traversal budgets, cardinality,
/// deduplication, ordering, cycle handling, and revision-scoped handles come
/// directly from [`TreeRuntime`] through this type's deref implementation.
/// Exedra adds effective-material explanations through
/// [`read_material`](Self::read_material).
#[derive(Clone, Debug)]
pub struct AddressableAssembly {
    pub(crate) runtime: TreeRuntime<Assembly>,
}

impl AddressableAssembly {
    /// Binds an assembly as a new runtime address-space instance.
    #[must_use]
    pub fn new(id: SpaceId<AssemblySpace>, assembly: Assembly) -> Self {
        Self {
            runtime: TreeRuntime::new(id, assembly),
        }
    }

    /// Restores an extracted assembly with its prior Addressable revision.
    ///
    /// Use the pair returned by [`Self::into_inner`]. New address-space
    /// instances should use [`Self::new`] instead.
    #[must_use]
    pub const fn resume(revision: Revision<AssemblySpace>, assembly: Assembly) -> Self {
        Self {
            runtime: TreeRuntime::resume(revision, assembly),
        }
    }

    /// Borrows the canonical assembly for domain reads and compilation.
    ///
    /// Mutating access is intentionally unavailable because an unobserved edit
    /// would invalidate locations, handles, pins, and guards.
    #[must_use]
    pub const fn assembly(&self) -> &Assembly {
        self.runtime.host()
    }

    /// Recovers the revision and canonical assembly, consuming this wrapper.
    ///
    /// Retain both values and pass them to [`Self::resume`] if the same
    /// address-space instance will be restored later.
    #[must_use]
    pub fn into_inner(self) -> (Revision<AssemblySpace>, Assembly) {
        self.runtime.into_host()
    }

    /// Applies one host mutation and advances the Addressable revision once.
    ///
    /// This gateway covers structural, metadata, part-content, and material
    /// authoring after an assembly has entered its runtime space. The revision
    /// advances conservatively even when the closure returns a value such as an
    /// `Err`.
    pub fn commit<T>(
        &mut self,
        mutation: impl FnOnce(&mut Assembly) -> T,
    ) -> (Revision<AssemblySpace>, T) {
        self.runtime.commit(mutation)
    }

    /// Returns an exact locator for the whole-assembly root `/`.
    #[must_use]
    pub fn root_locator(&self) -> AssemblyLocator {
        self.runtime.root_locator(AssemblyView::Instances)
    }

    /// Returns an exact locator for a current stable instance address.
    #[must_use]
    pub fn locator(&self, address: &InstanceAddress) -> Option<AssemblyLocator> {
        self.assembly().instance_by_address(address)?;
        Some(Locator::exact(
            self.id(),
            AssemblyView::Instances,
            address.clone(),
        ))
    }

    /// Returns an exact locator for a current runtime instance handle.
    #[must_use]
    pub fn locator_for_instance(&self, instance: InstanceId) -> Option<AssemblyLocator> {
        self.assembly()
            .instance(instance)
            .and_then(|instance| self.locator(instance.address()))
    }

    /// Reads an instance through a revision-scoped runtime handle.
    ///
    /// A handle acquired before a committed mutation is rejected; resolve its
    /// durable locator again to obtain a fresh handle.
    pub fn instance_by_handle(
        &self,
        handle: &ResolvedHandle<AssemblySpace, InstanceId>,
    ) -> Result<&Instance, ReadError> {
        if handle.space() != self.id() {
            return Err(TreeReadError::WrongSpace.into());
        }
        if handle.revision() != self.revision() {
            return Err(TreeReadError::StaleRevision {
                expected: handle.revision(),
                actual: self.revision(),
            }
            .into());
        }
        self.assembly()
            .instance(*handle.handle())
            .ok_or_else(|| TreeReadError::MissingOccurrence.into())
    }

    pub(crate) fn validate_instance_location(
        &self,
        location: &super::AssemblyLocation,
    ) -> Result<InstanceId, ReadError> {
        self.runtime
            .validate_location(location)
            .map_err(ReadError::from)?;
        let address = location
            .occurrence()
            .as_instance()
            .ok_or(ReadError::RootHasNoInstance)?;
        self.assembly()
            .instance_by_address(address)
            .ok_or_else(|| TreeReadError::MissingOccurrence.into())
    }
}

impl Deref for AddressableAssembly {
    type Target = TreeRuntime<Assembly>;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

/// Failure to consume a resolved location, endpoint, or runtime handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Generic Addressable tree-location validation failed.
    Tree(TreeReadError<AssemblySpace>),
    /// The synthetic root was used where an instance is required.
    RootHasNoInstance,
    /// The instance's part no longer exists.
    MissingReferent,
    /// The requested material slot is not declared by the instance's part.
    UnknownMaterialSlot,
    /// Host data violated Addressable's explanation invariant.
    InvalidExplanation,
}

impl From<TreeReadError<AssemblySpace>> for ReadError {
    fn from(error: TreeReadError<AssemblySpace>) -> Self {
        Self::Tree(error)
    }
}

#[cfg(test)]
mod tests {
    use addressable::{AbsoluteAddress, Query, Resolution, SpaceId};
    use exedra_constructive::ir::Placement3;

    use super::*;
    use crate::AssemblyAxis;

    #[test]
    fn empty_assembly_still_has_a_resolvable_query_root() {
        let space = AddressableAssembly::new(SpaceId::new(1), Assembly::new());
        let Resolution::Resolved(root) = space.resolve(&space.root_locator()) else {
            panic!("synthetic root must resolve");
        };
        assert_eq!(root.address(), &AbsoluteAddress::root());

        let children = space
            .query_many(&Query::many(space.root_locator()).traverse(AssemblyAxis::Children))
            .expect("empty root query succeeds");
        assert!(children.items().is_empty());
    }

    #[test]
    fn assembly_projection_uses_stored_addresses_and_runtime_queries() {
        let mut assembly = Assembly::new();
        let part = assembly
            .add_baked_part("panel", exedra::Mesh::default(), &[])
            .expect("part registers");
        let root = assembly
            .add_instance(None, "frame", part, Placement3::IDENTITY)
            .expect("root adds");
        let child = assembly
            .add_instance(Some(root), "panel", part, Placement3::IDENTITY)
            .expect("child adds");
        let child_address = assembly
            .instance(child)
            .expect("child exists")
            .address()
            .clone();
        let space = AddressableAssembly::new(SpaceId::new(7), assembly);

        let locator = space.locator(&child_address).expect("address is current");
        let Resolution::Resolved(location) = space.resolve(&locator) else {
            panic!("stored address resolves");
        };
        let handle = space
            .resolved_handle(&location)
            .expect("instance has a handle");
        assert_eq!(
            space
                .instance_by_handle(&handle)
                .expect("handle is fresh")
                .key(),
            "panel"
        );

        let descendants = space
            .query_many(&Query::many(space.root_locator()).traverse(AssemblyAxis::Descendants))
            .expect("tree query succeeds");
        assert_eq!(descendants.items().len(), 2);
    }

    #[test]
    fn structural_commit_and_resume_preserve_one_revision_clock() {
        let mut assembly = Assembly::new();
        let part = assembly
            .add_baked_part("panel", exedra::Mesh::default(), &[])
            .expect("part registers");
        let mut space = AddressableAssembly::new(SpaceId::new(9), assembly);
        let Resolution::Resolved(root) = space.resolve(&space.root_locator()) else {
            panic!("root resolves");
        };

        let (revision, instance) = space
            .commit(|assembly| assembly.add_instance(None, "panel", part, Placement3::IDENTITY));
        assert!(instance.is_ok());
        assert!(matches!(
            space.validate_location(&root),
            Err(TreeReadError::StaleRevision { .. })
        ));

        let (extracted_revision, assembly) = space.into_inner();
        assert_eq!(extracted_revision, revision);
        let resumed = AddressableAssembly::resume(extracted_revision, assembly);
        assert_eq!(resumed.revision(), revision);
        assert!(
            resumed
                .locator(&InstanceAddress::parse("/panel").expect("valid address"))
                .is_some()
        );
    }
}
