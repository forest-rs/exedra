// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operation records with explicit extent and axis intent.
//!
//! Each record lowers to the [`NodeKind`] it names, without validation of its
//! own: [`crate::ir::RecipeBuilder::add_node`] checks the lowered node exactly
//! as it checks a literal one. A record with default extent and axis lowers to
//! the identical node, so it has the same fingerprint.
//!
//! The records settle conventions that the node kinds leave implicit:
//!
//! - [`Extrude`] names which way the solid runs from its profile plane
//!   ([`Extent`]); [`NodeKind::Extrude`] always runs along local +Z.
//! - [`Revolve`] names its axis ([`Axis`]); [`NodeKind::Revolve`] always turns
//!   about local +Y, while [`PrimitiveSpec::Cylinder`] runs along +Z.
//! - [`Primitive::centered_cuboid`] centers a box; [`PrimitiveSpec::Box`] has
//!   its minimum corner at the origin.
//!
//! ```
//! use exedra_constructive::builders;
//! use exedra_constructive::ir::{Node, RecipeBuilder};
//! use exedra_constructive::ops::{Csg, Extrude, Primitive};
//!
//! let mut b = RecipeBuilder::new();
//! let plate = b.add_profile(builders::rect_centered(80.0, 40.0)?);
//! let source = b.source_ref("example:plate");
//! let plate = b.add_node(Node::new(Extrude::symmetric(plate, 6.0)).with_source(source))?;
//! let pocket = b.add_node(Primitive::centered_cuboid([20.0, 10.0, 8.0]))?;
//! let root = b.add_node(Csg::difference(plate, [pocket]))?;
//! let recipe = b.finish(root)?;
//! assert_eq!(recipe.nodes().len(), 3);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

use alloc::vec::Vec;

use crate::ir::{CapMode, CsgOp, Node, NodeId, NodeKind, Placement3, PrimitiveSpec, ProfileId};

/// How far an extrusion runs from its profile plane, along local Z.
///
/// Caps keep the names of [`NodeKind::Extrude`]: [`CapMode::Start`] is the
/// low-Z end and [`CapMode::End`] the high-Z end, whichever way the solid runs.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Extent {
    /// From the profile plane to `+distance`.
    Forward(f64),
    /// From `-distance` to the profile plane.
    Backward(f64),
    /// From `-distance / 2` to `+distance / 2`: the profile plane is the
    /// midplane.
    Symmetric(f64),
}

impl Extent {
    /// The total distance and the offset of the low end along local Z.
    fn span(self) -> (f64, f64) {
        match self {
            Self::Forward(distance) => (distance, 0.0),
            Self::Backward(distance) => (distance, -distance),
            Self::Symmetric(distance) => (distance, -0.5 * distance),
        }
    }
}

/// Extrusion of a profile with an explicit [`Extent`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Extrude {
    /// The profile to extrude.
    pub profile: ProfileId,
    /// Placement of the profile plane.
    pub placement: Placement3,
    /// Direction and distance from the profile plane.
    pub extent: Extent,
    /// Which caps to close.
    pub caps: CapMode,
}

impl Extrude {
    /// Extrudes `profile` by `extent`, closed at both ends, unplaced.
    #[must_use]
    pub const fn new(profile: ProfileId, extent: Extent) -> Self {
        Self {
            profile,
            placement: Placement3::IDENTITY,
            extent,
            caps: CapMode::Both,
        }
    }

    /// Extrudes `distance` along +Z from the profile plane.
    #[must_use]
    pub const fn forward(profile: ProfileId, distance: f64) -> Self {
        Self::new(profile, Extent::Forward(distance))
    }

    /// Extrudes `distance` along -Z from the profile plane.
    #[must_use]
    pub const fn backward(profile: ProfileId, distance: f64) -> Self {
        Self::new(profile, Extent::Backward(distance))
    }

    /// Extrudes `distance` in total, centered on the profile plane.
    #[must_use]
    pub const fn symmetric(profile: ProfileId, distance: f64) -> Self {
        Self::new(profile, Extent::Symmetric(distance))
    }

    /// Places the profile plane.
    #[must_use]
    pub const fn placed(mut self, placement: Placement3) -> Self {
        self.placement = placement;
        self
    }

    /// Selects the caps to close.
    #[must_use]
    pub const fn caps(mut self, caps: CapMode) -> Self {
        self.caps = caps;
        self
    }
}

impl From<Extrude> for NodeKind {
    fn from(extrude: Extrude) -> Self {
        let (height, low) = extrude.extent.span();
        let placement = if low == 0.0 {
            extrude.placement
        } else {
            extrude.placement * Placement3::translate(0.0, 0.0, low)
        };
        Self::Extrude {
            profile: extrude.profile,
            placement,
            height,
            caps: extrude.caps,
        }
    }
}

/// A coordinate axis of a record's local frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Local +X.
    X,
    /// Local +Y.
    Y,
    /// Local +Z.
    Z,
}

/// Revolution of a radius-height profile about an explicit [`Axis`].
///
/// Profile x is the radius and profile y the height along the axis, as for
/// [`NodeKind::Revolve`]. The radius runs along +X for the Y and Z axes and
/// along +Y for the X axis; positive sweeps turn right-handed about the axis.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Revolve {
    /// The profile to revolve.
    pub profile: ProfileId,
    /// Placement of the revolution frame.
    pub placement: Placement3,
    /// Axis of revolution in the placed frame.
    pub axis: Axis,
    /// Sweep angle in radians, in `(0, 2*pi]`.
    pub sweep: f64,
    /// Which end caps to close for partial sweeps.
    pub caps: CapMode,
}

impl Revolve {
    /// A full turn of `profile` about local +Y, unplaced.
    #[must_use]
    pub const fn full(profile: ProfileId) -> Self {
        Self::partial(profile, core::f64::consts::TAU)
    }

    /// A partial turn of `sweep` radians about local +Y, closed at both ends.
    #[must_use]
    pub const fn partial(profile: ProfileId, sweep: f64) -> Self {
        Self {
            profile,
            placement: Placement3::IDENTITY,
            axis: Axis::Y,
            sweep,
            caps: CapMode::Both,
        }
    }

    /// Turns about `axis` of the placed frame instead of +Y.
    #[must_use]
    pub const fn about(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// Places the revolution frame.
    #[must_use]
    pub const fn placed(mut self, placement: Placement3) -> Self {
        self.placement = placement;
        self
    }

    /// Selects the caps to close for a partial sweep.
    #[must_use]
    pub const fn caps(mut self, caps: CapMode) -> Self {
        self.caps = caps;
        self
    }
}

impl From<Revolve> for NodeKind {
    fn from(revolve: Revolve) -> Self {
        // Exact axis permutations carry the native +Y axis onto the requested
        // one; the frames are right-handed, so no reflection is introduced.
        let placement = match revolve.axis {
            Axis::Y => revolve.placement,
            Axis::Z => {
                revolve.placement
                    * Placement3::from_axes(
                        [1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.0, -1.0, 0.0],
                        [0.0; 3],
                    )
            }
            Axis::X => {
                revolve.placement
                    * Placement3::from_axes(
                        [0.0, 1.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 0.0, -1.0],
                        [0.0; 3],
                    )
            }
        };
        Self::Revolve {
            profile: revolve.profile,
            placement,
            sweep: revolve.sweep,
            caps: revolve.caps,
        }
    }
}

/// A Boolean combination of earlier nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Csg {
    /// The operation.
    pub op: CsgOp,
    /// Operands in order.
    pub operands: Vec<NodeId>,
}

impl Csg {
    /// `stock` minus the union of `cutters`.
    #[must_use]
    pub fn difference(stock: NodeId, cutters: impl IntoIterator<Item = NodeId>) -> Self {
        let mut operands = Vec::from([stock]);
        operands.extend(cutters);
        Self {
            op: CsgOp::Difference,
            operands,
        }
    }

    /// The union of `operands`.
    #[must_use]
    pub fn union(operands: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            op: CsgOp::Union,
            operands: operands.into_iter().collect(),
        }
    }

    /// The intersection of `operands`.
    #[must_use]
    pub fn intersection(operands: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            op: CsgOp::Intersection,
            operands: operands.into_iter().collect(),
        }
    }
}

impl From<Csg> for NodeKind {
    fn from(csg: Csg) -> Self {
        Self::Csg {
            op: csg.op,
            operands: csg.operands,
        }
    }
}

/// A parametric primitive with a placement.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Primitive {
    /// Primitive parameters.
    pub spec: PrimitiveSpec,
    /// Placement of the primitive's local frame.
    pub placement: Placement3,
}

impl Primitive {
    /// A box with its minimum corner at the origin.
    #[must_use]
    pub const fn cuboid(size: [f64; 3]) -> Self {
        Self {
            spec: PrimitiveSpec::Box { size },
            placement: Placement3::IDENTITY,
        }
    }

    /// A box centered on the origin.
    #[must_use]
    pub fn centered_cuboid(size: [f64; 3]) -> Self {
        let [x, y, z] = size.map(|extent| -0.5 * extent);
        Self {
            spec: PrimitiveSpec::Box { size },
            placement: Placement3::translate(x, y, z),
        }
    }

    /// A capped cylinder along +Z with its base center at the origin and
    /// `segments` authored side segments.
    #[must_use]
    pub const fn cylinder(radius: f64, height: f64, segments: u32) -> Self {
        Self {
            spec: PrimitiveSpec::Cylinder {
                radius,
                height,
                segments,
            },
            placement: Placement3::IDENTITY,
        }
    }

    /// Places the primitive after its own origin convention.
    #[must_use]
    pub fn placed(mut self, placement: Placement3) -> Self {
        self.placement = placement * self.placement;
        self
    }
}

impl From<Primitive> for NodeKind {
    fn from(primitive: Primitive) -> Self {
        Self::Primitive {
            spec: primitive.spec,
            placement: primitive.placement,
        }
    }
}

macro_rules! into_node {
    ($($record:ty),*) => {$(
        impl From<$record> for Node {
            fn from(record: $record) -> Self {
                Self::new(NodeKind::from(record))
            }
        }
    )*};
}
into_node!(Extrude, Revolve, Csg, Primitive);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use crate::evaluate::evaluate;
    use crate::ir::{RecipeBuilder, RecipeError};
    use crate::tessellate::EvalPolicy;

    fn z_bounds(node: impl Into<Node>) -> [f64; 2] {
        let mut b = RecipeBuilder::new();
        b.add_profile(builders::rect_centered(2.0, 2.0).unwrap());
        let root = b.add_node(node).unwrap();
        let result = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
        let mesh = &result.bodies[0].body.mesh;
        mesh.vertices()
            .map(|v| f64::from(mesh.vertex_position(v).unwrap()[2]))
            .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], z| {
                [lo.min(z), hi.max(z)]
            })
    }

    #[test]
    fn extents_place_the_solid_relative_to_the_profile_plane() {
        let profile = ProfileId(0);
        let lifted = Placement3::translate(0.0, 0.0, 10.0);
        assert_eq!(
            z_bounds(Extrude::forward(profile, 4.0).placed(lifted)),
            [10.0, 14.0]
        );
        assert_eq!(
            z_bounds(Extrude::backward(profile, 4.0).placed(lifted)),
            [6.0, 10.0]
        );
        assert_eq!(
            z_bounds(Extrude::symmetric(profile, 4.0).placed(lifted)),
            [8.0, 12.0]
        );
        // A forward extent is the native node itself.
        assert_eq!(
            NodeKind::from(Extrude::forward(profile, 4.0).placed(lifted)),
            NodeKind::Extrude {
                profile,
                placement: lifted,
                height: 4.0,
                caps: CapMode::Both,
            }
        );
    }

    #[test]
    fn invalid_extents_are_refused_by_node_validation() {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_centered(2.0, 2.0).unwrap());
        for extent in [Extent::Symmetric(-1.0), Extent::Backward(f64::NAN)] {
            assert!(matches!(
                b.add_node(Extrude::new(profile, extent)),
                Err(RecipeError::InvalidParameter { .. })
            ));
        }
    }

    #[test]
    fn revolution_axes_are_exact_right_handed_frames() {
        let profile = ProfileId(0);
        for (axis, along) in [(Axis::X, 0), (Axis::Y, 1), (Axis::Z, 2)] {
            let NodeKind::Revolve { placement, .. } =
                NodeKind::from(Revolve::full(profile).about(axis))
            else {
                unreachable!()
            };
            assert_eq!(placement.linear_determinant(), 1.0);
            let image = placement.transform_point([0.0, 1.0, 0.0]);
            assert_eq!(
                image.map(f64::abs).iter().position(|v| *v == 1.0),
                Some(along)
            );
            assert!(
                image.iter().all(|v| *v >= 0.0),
                "{axis:?} keeps a positive axis"
            );
        }
    }

    #[test]
    fn centered_cuboids_and_placement_compose_after_the_origin_convention() {
        let shifted = Primitive::centered_cuboid([2.0, 2.0, 4.0])
            .placed(Placement3::translate(0.0, 0.0, 5.0));
        assert_eq!(z_bounds(shifted), [3.0, 7.0]);
        assert_eq!(
            Csg::difference(NodeId(0), [NodeId(1), NodeId(2)]).operands,
            [NodeId(0), NodeId(1), NodeId(2)]
        );
    }
}
