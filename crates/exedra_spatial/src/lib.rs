// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic spatial indexing primitives for the Exedra workspace.
//!
//! This crate currently provides:
//! - [`Aabb`] for axis-aligned bounds,
//! - [`Octree`] with flat arena storage and deterministic child order,
//! - visitor-driven adaptive construction via [`OctreeVisitor`],
//! - deterministic depth-first and breadth-first traversal,
//! - leaf-neighbor queries via [`CellAdjacency`],
//! - incremental leaf refinement without rebuilding the whole tree.
//!
//! The crate is intentionally geometry-agnostic. It owns spatial indexing, not
//! scalar-field evaluation or mesh extraction.
//!
//! [`Octree`] is an append-only arena rather than a general query accelerator.
//! Cell IDs remain stable as leaves are refined; neighbor queries currently
//! scan the stored leaves and return matches in storage order.
//!
//! # Example
//!
//! Build one subdivision level and attach each leaf's depth as its payload:
//!
//! ```
//! use exedra_spatial::{Aabb, CellRef, Octree, OctreeVisitor};
//!
//! struct OneLevel;
//!
//! impl OctreeVisitor for OneLevel {
//!     type Payload = u8;
//!
//!     fn should_subdivide(&mut self, cell: CellRef) -> bool {
//!         cell.depth == 0
//!     }
//!
//!     fn make_leaf_payload(&mut self, cell: CellRef) -> Self::Payload {
//!         cell.depth
//!     }
//! }
//!
//! let bounds = Aabb::new([0.0; 3], [1.0; 3]).expect("ordered bounds");
//! let tree = Octree::build(bounds, 1, &mut OneLevel);
//! assert_eq!(tree.leaf_ids().len(), 8);
//! assert!(tree.leaf_ids().iter().all(|&id| tree.cell(id).unwrap().payload() == Some(&1)));
//! ```

#![no_std]
extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// Stable octree cell identifier within one [`Octree`].
///
/// IDs are arena-local. Refinement never changes an existing ID, but an ID
/// from one tree has no meaning in another tree.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct CellId(u32);

impl CellId {
    /// Creates an ID from a zero-based index.
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        Self(index)
    }

    /// Returns the zero-based index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Axis-aligned bounding box.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum corner.
    pub min: [f32; 3],
    /// Maximum corner.
    pub max: [f32; 3],
}

impl Aabb {
    /// Creates a finite bounding box when `min <= max` on every axis.
    #[must_use]
    pub fn new(min: [f32; 3], max: [f32; 3]) -> Option<Self> {
        let bounds = Self { min, max };
        bounds.is_valid().then_some(bounds)
    }

    /// Returns whether both corners are finite and ordered on every axis.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.min
            .iter()
            .chain(&self.max)
            .all(|component| component.is_finite())
            && self.min.iter().zip(self.max).all(|(min, max)| *min <= max)
    }

    /// Returns the box center.
    #[must_use]
    pub fn center(self) -> [f32; 3] {
        [
            self.min[0].midpoint(self.max[0]),
            self.min[1].midpoint(self.max[1]),
            self.min[2].midpoint(self.max[2]),
        ]
    }

    /// Returns the side lengths on each axis.
    #[must_use]
    pub fn extent(self) -> [f32; 3] {
        [
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        ]
    }

    /// Returns the volume.
    #[must_use]
    pub fn volume(self) -> f32 {
        let [dx, dy, dz] = self.extent();
        dx * dy * dz
    }

    /// Returns the surface area.
    #[must_use]
    pub fn surface_area(self) -> f32 {
        let [dx, dy, dz] = self.extent();
        2.0 * (dx * dy + dy * dz + dz * dx)
    }

    /// Returns whether `point` lies inside or on the boundary.
    #[must_use]
    pub fn contains(self, point: [f32; 3]) -> bool {
        point[0] >= self.min[0]
            && point[0] <= self.max[0]
            && point[1] >= self.min[1]
            && point[1] <= self.max[1]
            && point[2] >= self.min[2]
            && point[2] <= self.max[2]
    }

    /// Returns whether this box intersects `other`, including shared boundary.
    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        self.min[0] <= other.max[0]
            && self.max[0] >= other.min[0]
            && self.min[1] <= other.max[1]
            && self.max[1] >= other.min[1]
            && self.min[2] <= other.max[2]
            && self.max[2] >= other.min[2]
    }

    /// Returns the merged box covering `self` and `other`.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self {
            min: [
                self.min[0].min(other.min[0]),
                self.min[1].min(other.min[1]),
                self.min[2].min(other.min[2]),
            ],
            max: [
                self.max[0].max(other.max[0]),
                self.max[1].max(other.max[1]),
                self.max[2].max(other.max[2]),
            ],
        }
    }

    /// Splits this box into 8 octants.
    ///
    /// Child order is deterministic and uses XYZ bit order:
    /// - bit 0: high X,
    /// - bit 1: high Y,
    /// - bit 2: high Z.
    #[must_use]
    pub fn split(self) -> [Self; 8] {
        let center = self.center();
        core::array::from_fn(|index| {
            let x_hi = (index & 0b001) != 0;
            let y_hi = (index & 0b010) != 0;
            let z_hi = (index & 0b100) != 0;
            Self {
                min: [
                    if x_hi { center[0] } else { self.min[0] },
                    if y_hi { center[1] } else { self.min[1] },
                    if z_hi { center[2] } else { self.min[2] },
                ],
                max: [
                    if x_hi { self.max[0] } else { center[0] },
                    if y_hi { self.max[1] } else { center[1] },
                    if z_hi { self.max[2] } else { center[2] },
                ],
            }
        })
    }
}

/// One octree cell stored in the flat [`Octree`] arena.
#[derive(Clone, Debug, PartialEq)]
pub struct OctreeCell<P> {
    /// Stable cell ID.
    pub id: CellId,
    /// Spatial bounds of this cell.
    pub bounds: Aabb,
    /// Tree depth, starting at zero for the root.
    pub depth: u8,
    parent: Option<CellId>,
    children: Option<[CellId; 8]>,
    payload: Option<P>,
}

impl<P> OctreeCell<P> {
    /// Returns the parent cell when this is not the root.
    #[must_use]
    pub fn parent(&self) -> Option<CellId> {
        self.parent
    }

    /// Returns the child cells when this cell has been subdivided.
    #[must_use]
    pub fn children(&self) -> Option<[CellId; 8]> {
        self.children
    }

    /// Returns the leaf payload when this is a leaf cell.
    #[must_use]
    pub fn payload(&self) -> Option<&P> {
        self.payload.as_ref()
    }

    /// Returns mutable access to the leaf payload when this is a leaf cell.
    #[must_use]
    pub fn payload_mut(&mut self) -> Option<&mut P> {
        self.payload.as_mut()
    }

    /// Returns whether this cell has children.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.children.is_none()
    }
}

/// Lightweight immutable view passed to [`OctreeVisitor`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CellRef {
    /// Stable cell ID.
    pub id: CellId,
    /// Spatial bounds of the cell.
    pub bounds: Aabb,
    /// Tree depth of the cell.
    pub depth: u8,
    /// Parent cell when present.
    pub parent: Option<CellId>,
}

/// Visitor-driven octree construction and refinement contract.
pub trait OctreeVisitor {
    /// Payload type stored on leaf cells.
    type Payload;

    /// Returns whether `cell` should subdivide further.
    fn should_subdivide(&mut self, cell: CellRef) -> bool;

    /// Builds the payload stored on a leaf cell.
    fn make_leaf_payload(&mut self, cell: CellRef) -> Self::Payload;
}

/// Spatial adjacency class for neighbor queries.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CellAdjacency {
    /// Cells share a full face.
    Face,
    /// Cells share only an edge (not a full face).
    Edge,
    /// Cells share only a corner.
    Corner,
}

/// Octree refinement error.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RefineError {
    /// Referenced cell does not exist.
    MissingCell(CellId),
    /// Referenced cell has already been subdivided.
    NotLeaf(CellId),
    /// Referenced leaf is already at or beyond the requested maximum depth.
    MaxDepth {
        /// Leaf that cannot be subdivided under this request.
        cell: CellId,
        /// Requested maximum tree depth.
        max_depth: u8,
    },
}

impl core::fmt::Display for RefineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingCell(cell) => write!(f, "octree cell {cell:?} does not exist"),
            Self::NotLeaf(cell) => write!(f, "octree cell {cell:?} is not a leaf"),
            Self::MaxDepth { cell, max_depth } => {
                write!(
                    f,
                    "octree leaf {cell:?} is already at maximum depth {max_depth}"
                )
            }
        }
    }
}

impl core::error::Error for RefineError {}

/// Flat, append-only adaptive octree with deterministic traversal and refinement.
#[derive(Clone, Debug, PartialEq)]
pub struct Octree<P> {
    cells: Vec<OctreeCell<P>>,
}

impl<P> Octree<P> {
    /// Builds one tree from `root_bounds`, `max_depth`, and a visitor.
    #[must_use]
    pub fn build<V>(root_bounds: Aabb, max_depth: u8, visitor: &mut V) -> Self
    where
        V: OctreeVisitor<Payload = P>,
    {
        let mut tree = Self {
            cells: Vec::from([OctreeCell {
                id: CellId::from_index(0),
                bounds: root_bounds,
                depth: 0,
                parent: None,
                children: None,
                payload: None,
            }]),
        };
        tree.build_subtree(CellId::from_index(0), max_depth, visitor);
        tree
    }

    /// Returns the root cell.
    #[must_use]
    pub fn root(&self) -> &OctreeCell<P> {
        &self.cells[0]
    }

    /// Returns the root cell ID.
    #[must_use]
    pub const fn root_id(&self) -> CellId {
        CellId::from_index(0)
    }

    /// Returns the number of cells stored in this tree.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Returns whether the tree has no cells.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Returns one cell by stable ID.
    #[must_use]
    pub fn cell(&self, id: CellId) -> Option<&OctreeCell<P>> {
        self.cells.get(id.index() as usize)
    }

    /// Returns mutable access to one cell by stable ID.
    #[must_use]
    pub fn cell_mut(&mut self, id: CellId) -> Option<&mut OctreeCell<P>> {
        self.cells.get_mut(id.index() as usize)
    }

    /// Returns all leaf cell IDs in deterministic storage order.
    #[must_use]
    pub fn leaf_ids(&self) -> Vec<CellId> {
        self.cells
            .iter()
            .filter(|cell| cell.is_leaf())
            .map(|cell| cell.id)
            .collect()
    }

    /// Returns all cell IDs in deterministic depth-first pre-order.
    #[must_use]
    pub fn depth_first_ids(&self) -> Vec<CellId> {
        let mut out = Vec::with_capacity(self.cells.len());
        self.collect_depth_first(CellId::from_index(0), &mut out);
        out
    }

    /// Returns all cell IDs in deterministic breadth-first order.
    #[must_use]
    pub fn breadth_first_ids(&self) -> Vec<CellId> {
        let mut out = Vec::with_capacity(self.cells.len());
        let mut queue = VecDeque::from([CellId::from_index(0)]);
        while let Some(id) = queue.pop_front() {
            out.push(id);
            if let Some(children) = self.cell(id).and_then(OctreeCell::children) {
                queue.extend(children);
            }
        }
        out
    }

    /// Refines one existing leaf without rebuilding the whole tree.
    ///
    /// # Errors
    ///
    /// Returns [`RefineError`] when `id` is missing, names an internal cell,
    /// or is already at the requested maximum depth.
    pub fn refine_leaf<V>(
        &mut self,
        id: CellId,
        max_depth: u8,
        visitor: &mut V,
    ) -> Result<[CellId; 8], RefineError>
    where
        V: OctreeVisitor<Payload = P>,
    {
        let depth = {
            let cell = self.cell(id).ok_or(RefineError::MissingCell(id))?;
            if !cell.is_leaf() {
                return Err(RefineError::NotLeaf(id));
            }
            cell.depth
        };

        if depth >= max_depth {
            return Err(RefineError::MaxDepth {
                cell: id,
                max_depth,
            });
        }

        let children = self.insert_children(id);
        for child in children {
            self.build_subtree(child, max_depth, visitor);
        }
        Ok(children)
    }

    /// Returns leaf neighbors adjacent to `id` by the requested adjacency mode.
    ///
    /// This correctness-first query scans all stored leaves and returns matches
    /// in stable arena order. It returns an empty vector when `id` is missing or
    /// names an internal cell.
    #[must_use]
    pub fn leaf_neighbors(&self, id: CellId, adjacency: CellAdjacency) -> Vec<CellId> {
        let Some(target) = self.cell(id) else {
            return Vec::new();
        };
        if !target.is_leaf() {
            return Vec::new();
        }

        self.cells
            .iter()
            .filter(|cell| cell.id != id && cell.is_leaf())
            .filter(|cell| adjacency_matches(target.bounds, cell.bounds, adjacency))
            .map(|cell| cell.id)
            .collect()
    }

    fn build_subtree<V>(&mut self, id: CellId, max_depth: u8, visitor: &mut V)
    where
        V: OctreeVisitor<Payload = P>,
    {
        let cell_ref = self.cell_ref(id);
        let should_subdivide = cell_ref.depth < max_depth && visitor.should_subdivide(cell_ref);
        if should_subdivide {
            let children = self.insert_children(id);
            for child in children {
                self.build_subtree(child, max_depth, visitor);
            }
        } else {
            let payload = visitor.make_leaf_payload(cell_ref);
            self.cell_mut(id)
                .expect("live leaf should remain valid")
                .payload = Some(payload);
        }
    }

    fn collect_depth_first(&self, id: CellId, out: &mut Vec<CellId>) {
        out.push(id);
        if let Some(children) = self.cell(id).and_then(OctreeCell::children) {
            for child in children {
                self.collect_depth_first(child, out);
            }
        }
    }

    fn cell_ref(&self, id: CellId) -> CellRef {
        let cell = self.cell(id).expect("requested cell should be live");
        CellRef {
            id: cell.id,
            bounds: cell.bounds,
            depth: cell.depth,
            parent: cell.parent,
        }
    }

    fn insert_children(&mut self, id: CellId) -> [CellId; 8] {
        let (bounds, depth) = {
            let parent = self.cell_mut(id).expect("parent should be live");
            parent.payload = None;
            (parent.bounds, parent.depth)
        };
        let child_bounds = bounds.split();
        let children = core::array::from_fn(|index| {
            let child_id = CellId::from_index(usize_to_u32(self.cells.len()));
            self.cells.push(OctreeCell {
                id: child_id,
                bounds: child_bounds[index],
                depth: depth + 1,
                parent: Some(id),
                children: None,
                payload: None,
            });
            child_id
        });
        self.cell_mut(id)
            .expect("parent should remain live")
            .children = Some(children);
        children
    }
}

fn adjacency_matches(a: Aabb, b: Aabb, adjacency: CellAdjacency) -> bool {
    let touch_x = axis_touches(a.min[0], a.max[0], b.min[0], b.max[0]);
    let touch_y = axis_touches(a.min[1], a.max[1], b.min[1], b.max[1]);
    let touch_z = axis_touches(a.min[2], a.max[2], b.min[2], b.max[2]);

    let overlap_x = axis_overlaps(a.min[0], a.max[0], b.min[0], b.max[0]);
    let overlap_y = axis_overlaps(a.min[1], a.max[1], b.min[1], b.max[1]);
    let overlap_z = axis_overlaps(a.min[2], a.max[2], b.min[2], b.max[2]);

    let touching_axes = touch_x as u8 + touch_y as u8 + touch_z as u8;
    let overlapping_axes = overlap_x as u8 + overlap_y as u8 + overlap_z as u8;

    match adjacency {
        CellAdjacency::Face => touching_axes == 1 && overlapping_axes == 2,
        CellAdjacency::Edge => touching_axes == 2 && overlapping_axes == 1,
        CellAdjacency::Corner => touching_axes == 3,
    }
}

fn axis_overlaps(a_min: f32, a_max: f32, b_min: f32, b_max: f32) -> bool {
    a_min < b_max && a_max > b_min
}

fn axis_touches(a_min: f32, a_max: f32, b_min: f32, b_max: f32) -> bool {
    a_max == b_min || b_max == a_min
}

fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("octree cell index overflowed u32")
}

#[cfg(test)]
mod tests {
    use super::{Aabb, CellAdjacency, CellRef, Octree, OctreeVisitor, RefineError};

    #[derive(Default)]
    struct DepthVisitor;

    impl OctreeVisitor for DepthVisitor {
        type Payload = u8;

        fn should_subdivide(&mut self, cell: CellRef) -> bool {
            cell.depth < 1
        }

        fn make_leaf_payload(&mut self, cell: CellRef) -> Self::Payload {
            cell.depth
        }
    }

    #[derive(Default)]
    struct SelectiveVisitor;

    impl OctreeVisitor for SelectiveVisitor {
        type Payload = u8;

        fn should_subdivide(&mut self, cell: CellRef) -> bool {
            cell.depth == 0 || (cell.depth == 1 && cell.bounds.min == [0.0, 0.0, 0.0])
        }

        fn make_leaf_payload(&mut self, cell: CellRef) -> Self::Payload {
            cell.depth
        }
    }

    #[test]
    fn aabb_split_and_merge_are_deterministic() {
        let bounds = Aabb::new([0.0, 0.0, 0.0], [2.0, 4.0, 6.0]).expect("valid bounds");
        let children = bounds.split();

        assert_eq!(
            children[0],
            Aabb::new([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]).expect("valid child")
        );
        assert_eq!(
            children[7],
            Aabb::new([1.0, 2.0, 3.0], [2.0, 4.0, 6.0]).expect("valid child")
        );
        let merged = children.into_iter().fold(children[0], Aabb::merge);
        assert_eq!(merged, bounds);
        assert_eq!(
            bounds.surface_area(),
            2.0 * (2.0 * 4.0 + 4.0 * 6.0 + 2.0 * 6.0)
        );
    }

    #[test]
    fn aabb_constructor_rejects_nonfinite_and_reversed_coordinates() {
        // A successfully constructed box must remain safe for split, overlap,
        // and midpoint operations; NaN must not evade ordering comparisons.
        assert_eq!(Aabb::new([f32::NAN, 0.0, 0.0], [1.0; 3]), None);
        assert_eq!(Aabb::new([0.0; 3], [f32::INFINITY, 1.0, 1.0]), None);
        assert_eq!(Aabb::new([1.0, 0.0, 0.0], [0.0, 1.0, 1.0]), None);

        let wide = Aabb::new([f32::MAX / 2.0; 3], [f32::MAX; 3]).expect("finite ordered bounds");
        assert!(wide.center().iter().all(|component| component.is_finite()));
    }

    #[test]
    fn octree_traversal_orders_are_deterministic() {
        let bounds = Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).expect("valid bounds");
        let mut visitor = DepthVisitor;
        let tree = Octree::build(bounds, 3, &mut visitor);

        assert_eq!(tree.len(), 9);
        assert_eq!(
            tree.depth_first_ids()
                .into_iter()
                .map(|id| id.index())
                .collect::<alloc::vec::Vec<_>>(),
            alloc::vec![0, 1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(
            tree.breadth_first_ids()
                .into_iter()
                .map(|id| id.index())
                .collect::<alloc::vec::Vec<_>>(),
            alloc::vec![0, 1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert!(tree.leaf_ids().iter().all(|id| *id != tree.root_id()));
    }

    #[test]
    fn incremental_refinement_adds_children_without_rebuild() {
        let bounds = Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).expect("valid bounds");
        let mut visitor = DepthVisitor;
        let mut tree = Octree::build(bounds, 0, &mut visitor);
        assert_eq!(tree.len(), 1);

        let children = tree
            .refine_leaf(tree.root_id(), 1, &mut visitor)
            .expect("root leaf should refine");
        assert_eq!(tree.len(), 9);
        assert_eq!(
            tree.root()
                .children()
                .expect("root should now have children"),
            children
        );
        assert_eq!(
            tree.refine_leaf(tree.root_id(), 1, &mut visitor),
            Err(RefineError::NotLeaf(tree.root_id()))
        );
    }

    #[test]
    fn refinement_at_the_depth_limit_is_a_typed_no_op() {
        // Returning the leaf eight times would look like successful
        // subdivision and could keep a caller's refinement loop alive forever.
        let bounds = Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).expect("valid bounds");
        let mut visitor = DepthVisitor;
        let mut tree = Octree::build(bounds, 0, &mut visitor);
        let root = tree.root_id();
        let before = tree.clone();

        assert_eq!(
            tree.refine_leaf(root, 0, &mut visitor),
            Err(RefineError::MaxDepth {
                cell: root,
                max_depth: 0,
            })
        );
        assert_eq!(tree, before);
    }

    #[test]
    fn leaf_neighbors_find_face_and_edge_adjacency_across_depths() {
        let bounds = Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).expect("valid bounds");
        let mut visitor = SelectiveVisitor;
        let tree = Octree::build(bounds, 2, &mut visitor);

        let fine_leaf = tree
            .leaf_ids()
            .into_iter()
            .find(|id| tree.cell(*id).expect("leaf should exist").bounds.min == [0.25, 0.25, 0.0])
            .expect("expected fine leaf");
        let coarse_face_neighbor = tree
            .leaf_ids()
            .into_iter()
            .find(|id| tree.cell(*id).expect("leaf should exist").bounds.min == [0.5, 0.0, 0.0])
            .expect("expected coarse face neighbor");

        let face_neighbors = tree.leaf_neighbors(fine_leaf, CellAdjacency::Face);
        assert!(face_neighbors.contains(&coarse_face_neighbor));

        let edge_neighbors = tree.leaf_neighbors(fine_leaf, CellAdjacency::Edge);
        assert!(edge_neighbors.iter().any(|id| {
            tree.cell(*id).expect("neighbor should exist").bounds.min == [0.5, 0.5, 0.0]
        }));
    }

    #[test]
    fn adaptive_leaves_partition_the_root_and_keep_parent_links() {
        // A flat arena is useful only if adaptive insertion preserves both the
        // spatial partition and the parent/child topology that stable IDs name.
        let bounds = Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).expect("valid bounds");
        let mut visitor = SelectiveVisitor;
        let tree = Octree::build(bounds, 2, &mut visitor);

        let leaf_volume = tree
            .leaf_ids()
            .iter()
            .map(|&id| tree.cell(id).expect("leaf should exist").bounds.volume())
            .sum::<f32>();
        assert_eq!(leaf_volume, bounds.volume());

        for id in tree.depth_first_ids() {
            let cell = tree.cell(id).expect("traversal IDs should be live");
            if let Some(parent) = cell.parent() {
                assert!(
                    tree.cell(parent)
                        .and_then(|parent| parent.children())
                        .is_some_and(|children| children.contains(&id)),
                    "cell {id:?} is absent from its recorded parent {parent:?}"
                );
            }
        }
    }

    #[test]
    fn adaptive_neighbor_queries_are_symmetric_and_canonical() {
        // Cross-depth adjacency is an undirected spatial relation; this traps
        // asymmetric overlap/touch predicates and unstable result ordering.
        let bounds = Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).expect("valid bounds");
        let mut visitor = SelectiveVisitor;
        let tree = Octree::build(bounds, 2, &mut visitor);

        for adjacency in [
            CellAdjacency::Face,
            CellAdjacency::Edge,
            CellAdjacency::Corner,
        ] {
            for leaf in tree.leaf_ids() {
                let neighbors = tree.leaf_neighbors(leaf, adjacency);
                assert!(neighbors.windows(2).all(|pair| pair[0] < pair[1]));
                for neighbor in neighbors {
                    assert!(tree.leaf_neighbors(neighbor, adjacency).contains(&leaf));
                }
            }
        }
    }
}
