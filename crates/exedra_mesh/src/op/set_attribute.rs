// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::attributes::{AttrKey, AttributeElement, Domain, LayerValue};
use crate::session::AttributeWrite;
use crate::{ChangeSink, EditSession};

/// Structured failure from [`set_attribute`] and [`clear_attribute`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SetAttributeError {
    /// The element handle addresses another domain than the key.
    DomainMismatch {
        /// Domain of the key.
        expected: Domain,
        /// Domain of the element handle.
        found: Domain,
    },
    /// The element is stale or missing.
    ElementNotLive {
        /// Domain of the element.
        domain: Domain,
        /// Numeric slot index from the rejected ID.
        index: u32,
    },
    /// The key names a built-in layer listed in
    /// [`attr::RESERVED`](crate::attr::RESERVED); use its dedicated operation.
    Reserved {
        /// Name of the rejected key.
        name: &'static str,
    },
    /// A layer with this domain and name exists with another value type.
    TypeMismatch {
        /// Name of the rejected key.
        name: &'static str,
    },
}

impl fmt::Display for SetAttributeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DomainMismatch { expected, found } => {
                write!(f, "attribute expects a {expected:?} element, got {found:?}")
            }
            Self::ElementNotLive { domain, index } => {
                write!(f, "{domain:?} element is not live: {index}")
            }
            Self::Reserved { name } => write!(f, "attribute {name} is reserved"),
            Self::TypeMismatch { name } => {
                write!(f, "attribute {name} exists with another value type")
            }
        }
    }
}

impl core::error::Error for SetAttributeError {}

fn check_domain<T, E: AttributeElement>(key: AttrKey<T>) -> Result<(), SetAttributeError> {
    if key.domain() == E::DOMAIN {
        Ok(())
    } else {
        Err(SetAttributeError::DomainMismatch {
            expected: key.domain(),
            found: E::DOMAIN,
        })
    }
}

fn result<T, E: AttributeElement>(
    outcome: AttributeWrite,
    key: AttrKey<T>,
    element: E,
) -> Result<(), SetAttributeError> {
    match outcome {
        AttributeWrite::Written => Ok(()),
        AttributeWrite::NotLive => Err(SetAttributeError::ElementNotLive {
            domain: E::DOMAIN,
            index: element.id().index(),
        }),
        AttributeWrite::Reserved => Err(SetAttributeError::Reserved { name: key.name() }),
        AttributeWrite::TypeMismatch => Err(SetAttributeError::TypeMismatch { name: key.name() }),
    }
}

/// Writes one value of a caller-defined attribute layer.
///
/// `element` must address the key's domain: a [`VertexId`](crate::VertexId),
/// [`FaceId`](crate::FaceId), or [`HalfEdgeId`](crate::HalfEdgeId). A
/// half-edge may be a face corner or a boundary (OUTSIDE-face) half-edge;
/// render extraction reads only face corners. The layer is created sparse on
/// first write unless [`Mesh::define_dense_layer`](crate::Mesh::define_dense_layer)
/// registered it first. Prefer dense layers for values on (nearly) every
/// element, such as per-corner colors: sparse insertion in arbitrary order is
/// quadratic. The element is marked dirty in its domain.
///
/// Topology kernels carry caller-defined layers onto the elements they create
/// by the rule declared with
/// [`Mesh::set_layer_propagation`](crate::Mesh::set_layer_propagation).
/// Half-edge layers are corner data (see
/// [`Propagation`](crate::attributes::Propagation)). `exedra_mesh_ops` does
/// not carry them yet, in two ways:
///
/// - operations that edit a mesh in place through these kernels (face edits,
///   poke, patch and connect operations) clear and count the values of the
///   elements they delete, but their new elements start empty and uncounted;
/// - operations that return a fresh mesh (Booleans, stretch, sections, and
///   reflecting transforms) drop caller-defined layers and their rules
///   entirely, without a count.
pub fn set_attribute<T: LayerValue, E: AttributeElement, S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    key: AttrKey<T>,
    element: E,
    value: T,
) -> Result<(), SetAttributeError> {
    check_domain::<T, E>(key)?;
    let outcome = session.set_attribute_impl(key, element.id(), value);
    result(outcome, key, element)
}

/// Clears one value of a caller-defined attribute layer.
///
/// A sparse layer forgets the value; a dense layer resets it to the layer's
/// default. Clearing a layer that was never registered succeeds without
/// effect.
pub fn clear_attribute<T: LayerValue, E: AttributeElement, S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    key: AttrKey<T>,
    element: E,
) -> Result<(), SetAttributeError> {
    check_domain::<T, E>(key)?;
    let outcome = session.clear_attribute_impl(key, element.id());
    result(outcome, key, element)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::SetAttributeError;
    use crate::attributes::{AttrError, AttrKey, Domain};
    use crate::{ChangeSetBuilder, FaceId, Id, Mesh, MeshBuilder, attr, op};

    const WIND: AttrKey<[f32; 4]> = AttrKey::new(Domain::Vertex, "vertex.wind");
    const TAG: AttrKey<u32> = AttrKey::new(Domain::Face, "face.tag");

    fn triangle() -> Mesh {
        let mut builder = MeshBuilder::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            builder.push_vertex(p);
        }
        builder.add_face(&[0, 1, 2]).expect("face");
        builder.build().expect("build").mesh
    }

    #[test]
    fn writes_mark_dirty_and_bump_the_revision() {
        let mut mesh = triangle();
        let vertex = mesh.vertices().next().expect("vertex");
        let face = mesh.faces().next().expect("face");
        let corner = mesh.face_loop(face).next().expect("corner");
        let before = mesh.revision();
        let mut edit = mesh.edit_with(ChangeSetBuilder::new());
        op::set_attribute(&mut edit, WIND, vertex, [1.0, 2.0, 3.0, 4.0]).expect("vertex");
        op::set_attribute(&mut edit, TAG, face, 7).expect("face");
        op::set_attribute(&mut edit, attr::CORNER_UV1, corner, [0.5, 0.5]).expect("corner");
        let changes = edit.finish();
        assert_ne!(mesh.revision(), before);
        assert!(changes.dirty.has_dirty_vertices());
        assert!(changes.dirty.has_dirty_faces());
        assert!(changes.dirty.has_dirty_corners());
        assert_eq!(
            mesh.attrs()
                .sparse(WIND)
                .and_then(|l| l.get(vertex.as_id())),
            Some(&[1.0, 2.0, 3.0, 4.0])
        );
        assert_eq!(
            mesh.attrs().sparse(TAG).and_then(|l| l.get(face.as_id())),
            Some(&7)
        );

        let mut edit = mesh.edit();
        op::clear_attribute(&mut edit, TAG, face).expect("clear");
        let _: () = edit.finish();
        assert_eq!(
            mesh.attrs().sparse(TAG).and_then(|l| l.get(face.as_id())),
            None
        );
    }

    #[test]
    fn dense_layers_fill_and_reset_to_their_default() {
        let mut mesh = triangle();
        let vertices: Vec<_> = mesh.vertices().collect();
        let before = mesh.revision();
        mesh.define_dense_layer(WIND, [0.0, 0.0, 0.0, 1.0])
            .expect("define");
        assert_ne!(
            mesh.revision(),
            before,
            "registration changes carried output, so it advances the revision"
        );
        let registered = mesh.revision();
        assert_eq!(
            mesh.define_dense_layer(WIND, [0.0; 4]),
            Err(AttrError::AlreadyExists)
        );
        assert_eq!(mesh.revision(), registered, "failures leave the revision");

        let mut edit = mesh.edit();
        op::set_attribute(&mut edit, WIND, vertices[0], [9.0; 4]).expect("set");
        let _: () = edit.finish();
        let layer = mesh.attrs().dense(WIND).expect("dense");
        assert_eq!(layer.get(vertices[0].as_id()), Some(&[9.0; 4]));
        assert_eq!(layer.get(vertices[1].as_id()), Some(&[0.0, 0.0, 0.0, 1.0]));

        let mut edit = mesh.edit();
        op::clear_attribute(&mut edit, WIND, vertices[0]).expect("clear");
        let _: () = edit.finish();
        assert_eq!(
            mesh.attrs()
                .dense(WIND)
                .and_then(|l| l.get(vertices[0].as_id())),
            Some(&[0.0, 0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn rejects_reserved_mistyped_mismatched_and_stale_targets() {
        let mut mesh = triangle();
        let face = mesh.faces().next().expect("face");
        let vertex = mesh.vertices().next().expect("vertex");
        let corner = mesh.face_loop(face).next().expect("corner");
        assert_eq!(
            mesh.define_sparse_layer(attr::CORNER_UV),
            Err(AttrError::Reserved)
        );
        assert_eq!(
            mesh.define_dense_layer(attr::FACE_REGION, 0),
            Err(AttrError::Reserved)
        );

        let mut edit = mesh.edit();
        assert_eq!(
            op::set_attribute(&mut edit, attr::CORNER_UV, corner, [0.0, 0.0]),
            Err(SetAttributeError::Reserved { name: "corner.uv" })
        );
        // A vertex ID shares a slot index with live faces and corners; the
        // domain check refuses it before liveness is consulted.
        assert_eq!(
            op::set_attribute(&mut edit, TAG, vertex, 1),
            Err(SetAttributeError::DomainMismatch {
                expected: Domain::Face,
                found: Domain::Vertex,
            })
        );
        op::set_attribute(&mut edit, TAG, face, 1).expect("tag");
        let mistyped = AttrKey::<f32>::new(Domain::Face, "face.tag");
        assert_eq!(
            op::set_attribute(&mut edit, mistyped, face, 1.0),
            Err(SetAttributeError::TypeMismatch { name: "face.tag" })
        );
        assert_eq!(
            op::clear_attribute(&mut edit, mistyped, face),
            Err(SetAttributeError::TypeMismatch { name: "face.tag" })
        );
        let stale = FaceId::from(Id::new(99, core::num::NonZeroU32::MIN));
        assert_eq!(
            op::set_attribute(&mut edit, TAG, stale, 1),
            Err(SetAttributeError::ElementNotLive {
                domain: Domain::Face,
                index: 99
            })
        );
        assert_eq!(
            op::set_attribute(&mut edit, TAG, FaceId::OUTSIDE, 1),
            Err(SetAttributeError::ElementNotLive {
                domain: Domain::Face,
                index: FaceId::OUTSIDE.index()
            })
        );
        let _: () = edit.finish();
    }

    #[test]
    fn boundary_half_edges_and_unregistered_clears_are_accepted() {
        let mut mesh = triangle();
        let face = mesh.faces().next().expect("face");
        let corner = mesh.face_loop(face).next().expect("corner");
        let boundary = mesh.twin(corner).expect("boundary twin");
        let mut edit = mesh.edit();
        // Directed-edge data on a boundary half-edge is stored; extraction
        // reads only face corners.
        op::set_attribute(&mut edit, attr::CORNER_COLOR, boundary, [1.0; 4]).expect("boundary");
        op::clear_attribute(&mut edit, TAG, face).expect("never registered");
        let _: () = edit.finish();
        assert!(mesh.attrs().sparse(TAG).is_none());
        let (tri, stats) = mesh.to_trimesh(&crate::ExtractParams {
            attributes: alloc::vec![crate::ExtractAttribute::new(attr::CORNER_COLOR, [0.0; 4])],
            ..crate::ExtractParams::default()
        });
        assert_eq!(stats.attribute_fallback_count, 3);
        assert_eq!(
            tri.attribute(attr::CORNER_COLOR),
            Some(&crate::AttributeBuffer::Vec4(alloc::vec![[0.0; 4]; 3]))
        );
    }
}
