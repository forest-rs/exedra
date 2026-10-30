// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native BRep persistence in a caller-owned stage.
//!
//! This is Exedra's versioned boundary-table encoding on ordinary prims, not the
//! USD-BRep schema. It preserves f64 geometry, topology, explicit face-local trims,
//! numeric feature tags and occurrence placement. Restoring creates a fresh native
//! selection scope and validates the complete body before installation. Constructor
//! histories, constructor roles, analytic integral provenance and display meshes
//! are not encoded. Callers continue using the ordinary Layerstack stage client
//! for composition, transactions, layer export and reopen.

mod codec;
use alloc::{sync::Arc, vec::Vec};
use core::fmt;
use exedra_analytic::{
    body::{BodyError, ValidationPolicy},
    geometry::GeometryError,
};
use exedra_compile::NativeSource;
use layerstack::{
    EditTarget, LayerStore, PathId, PropertyPath, PropertySpec, PropertyType, Specifier, Stage,
    Time, Transaction, TypedArray, Value,
};

/// Exedra-specific native archive format version, a uniform `uint`.
pub const FORMAT: &str = "exedra:native:format";
/// Versioned integer topology, tags and geometry discriminants, a uniform `uint[]`.
pub const TOPOLOGY: &str = "exedra:native:topology";
/// Authoritative f64 support, parameters and placement, a uniform `double[]`.
pub const GEOMETRY: &str = "exedra:native:geometry";

/// Explicit input/output limits for native codec work and allocation.
/// Stage composition, file parsing and storage-adapter decoding retain their
/// caller-owned budgets; these limits apply to composed streams and decoded tables.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Maximum integer words, including all table references and feature tags.
    pub topology_words: usize,
    /// Maximum f64 values, including all geometric supports and trim parameters.
    pub geometry_values: usize,
    /// Maximum total decoded table and nested-array entries.
    pub decoded_items: usize,
    /// Maximum face/coedge support checks during boundary validation, counting
    /// every face-loop reference's coedges even when loops are shared or repeated.
    /// Checked from loop lengths before visiting the expanded boundary uses.
    pub max_boundary_checks: usize,
}
impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            topology_words: 4_000_000,
            geometry_values: 16_000_000,
            decoded_items: 1_000_000,
            max_boundary_checks: 4_000_000,
        }
    }
}
/// An exact native archive could not be captured or restored.
#[derive(Clone, Debug, PartialEq)]
pub enum ArchiveError {
    /// A required composed attribute is missing, blocked or has the wrong value kind.
    Attribute(&'static str),
    /// Deferred adapter storage has no pre-decode length available to this
    /// bounded reader. Materialize it under the caller's adapter budget first.
    DeferredAttribute(&'static str),
    /// The encoded version is not supported.
    Version(u32),
    /// A resource or decoded count exceeds the caller's limit.
    Budget(&'static str),
    /// An incomplete, surplus or invalid stream record was encountered.
    Encoding {
        /// Specific field or stream record being decoded.
        record: &'static str,
        /// Owning table, or `archive` for a header/count.
        table: &'static str,
        /// Zero-based owning table record index.
        index: usize,
        /// Integer-stream cursor at the refusal.
        word: usize,
        /// Geometry-stream cursor at the refusal.
        scalar: usize,
    },
    /// Native support geometry or parameter ranges are invalid.
    Geometry {
        /// Owning table record (edge, face or trim).
        record: &'static str,
        /// Owning record's zero-based table index.
        index: usize,
        /// Integer-stream cursor after reading the invalid geometry.
        word: usize,
        /// Geometry-stream cursor after reading the invalid geometry.
        scalar: usize,
        /// Violated supporting-geometry invariant.
        error: GeometryError,
    },
    /// References, closure, support agreement or explicit trim correspondence are invalid.
    Boundary(BodyError),
    /// The occurrence placement is not a finite rigid frame.
    Placement(exedra_constructive::ir::FrameError),
}
impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attribute(name) => write!(f, "unreadable native archive attribute {name}"),
            Self::Version(version) => {
                write!(f, "unsupported Exedra native archive version {version}")
            }
            Self::Budget(resource) => write!(f, "native archive exceeds {resource} limit"),
            Self::Encoding {
                record,
                table,
                index,
                word,
                scalar,
            } => write!(
                f,
                "invalid native archive {table} {index} {record} at integer {word}, scalar {scalar}"
            ),
            Self::DeferredAttribute(name) => write!(
                f,
                "native archive attribute {name} requires caller-budgeted materialization"
            ),
            Self::Geometry {
                record,
                index,
                word,
                scalar,
                error,
            } => write!(
                f,
                "native archive {record} {index} at integer {word}, scalar {scalar}: {error}"
            ),
            Self::Boundary(e) => e.fmt(f),
            Self::Placement(e) => e.fmt(f),
        }
    }
}
impl core::error::Error for ArchiveError {}

/// A portable native boundary snapshot, independent of a stage or compiler.
///
/// Public streams permit transport without a second stage client. Consumers
/// should use [`Self::restore`] for decoding and validation. Version 1 stores
/// occurrence frame, vertices, bounded edges, coedge loops, faces and sparse
/// face-local trim records in table order; no display data is present.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeArchive {
    /// Encoding version; currently `1`.
    pub version: u32,
    /// Integer table lengths, IDs, senses, feature tags and support discriminants.
    pub topology: Vec<u32>,
    /// Exact f64 geometry and supporting frames.
    pub geometry: Vec<f64>,
}
impl NativeArchive {
    /// Captures native geometry and all face tags, without realizing a display.
    /// Constructor roles become feature tags; occurrence placement stays separate.
    /// # Errors
    /// Refuses caller limits or a stream length beyond the version's u32 range.
    pub fn capture(source: &NativeSource, limits: &ArchiveLimits) -> Result<Self, ArchiveError> {
        codec::capture(source, limits)
    }
    /// Decodes, validates and installs a fresh construction-independent source.
    /// # Errors
    /// Refuses unsupported versions, malformed streams, limits, invalid native
    /// geometry or a non-rigid occurrence frame. No stage or old source is mutated.
    pub fn restore(
        &self,
        policy: &ValidationPolicy,
        limits: &ArchiveLimits,
    ) -> Result<NativeSource, ArchiveError> {
        codec::restore(self, policy, limits)
    }
}

/// Prepares creation of one ordinary prim carrying authoritative native geometry.
///
/// Apply the returned transaction through the caller's `LiveStage`; it rejects
/// an existing local prim atomically. Composition transforms, materials, units
/// and feature-name registries remain ordinary caller-owned stage data. Native
/// placement is encoded inside the archive and must not also be applied to its
/// body a second time when reopening.
/// # Errors
/// Refuses archive limits before producing any stage edits.
pub fn define_native_body(
    store: &mut dyn LayerStore,
    target: &EditTarget,
    path: PathId,
    source: &NativeSource,
    limits: &ArchiveLimits,
) -> Result<Transaction, ArchiveError> {
    let archive = NativeArchive::capture(source, limits)?;
    let mut tx = Transaction::new();
    tx.create_prim(target.prim(path), Specifier::Def, None);
    for (name, ty, value) in [
        (FORMAT, "uint", Value::UInt(archive.version)),
        (
            TOPOLOGY,
            "uint[]",
            Value::TypedArray(TypedArray::UInt(Arc::new(archive.topology))),
        ),
        (
            GEOMETRY,
            "double[]",
            Value::TypedArray(TypedArray::Double(Arc::new(archive.geometry))),
        ),
    ] {
        let property = PropertyPath::new(path, store.tokens().intern(name));
        tx.create_property(
            target.property(property),
            PropertySpec::typed_attribute(
                PropertyType::parse(ty).expect("built-in USD value type"),
            )
            .uniform()
            .with_default(value),
        );
    }
    Ok(tx)
}

/// Restores composed default-time native geometry from a caller-owned stage.
///
/// Stronger malformed values are errors; decoding never falls back to weaker
/// geometry, a mesh or a constructor approximation. The stage is read-only.
/// # Errors
/// Returns the missing attribute, version, stream, geometry or boundary diagnosis.
pub fn read_native_body(
    stage: &Stage,
    store: &dyn LayerStore,
    path: PathId,
    policy: &ValidationPolicy,
    limits: &ArchiveLimits,
) -> Result<NativeSource, ArchiveError> {
    let prim = stage
        .view(store)
        .prim(path)
        .ok_or(ArchiveError::Attribute(FORMAT))?;
    let read = |name| {
        prim.attribute(name)
            .and_then(|attribute| attribute.authored_value(Time::Default))
            .map(|value| value.value)
            .ok_or(ArchiveError::Attribute(name))
    };
    let Value::UInt(version) = read(FORMAT)? else {
        return Err(ArchiveError::Attribute(FORMAT));
    };
    if version != 1 {
        return Err(ArchiveError::Version(version));
    }
    let Value::TypedArray(topology) = read(TOPOLOGY)? else {
        return Err(ArchiveError::Attribute(TOPOLOGY));
    };
    let Value::TypedArray(geometry) = read(GEOMETRY)? else {
        return Err(ArchiveError::Attribute(GEOMETRY));
    };
    if matches!(topology, TypedArray::Deferred(_)) {
        return Err(ArchiveError::DeferredAttribute(TOPOLOGY));
    }
    if matches!(geometry, TypedArray::Deferred(_)) {
        return Err(ArchiveError::DeferredAttribute(GEOMETRY));
    }
    let TypedArray::UInt(topology) = topology else {
        return Err(ArchiveError::Attribute(TOPOLOGY));
    };
    let TypedArray::Double(geometry) = geometry else {
        return Err(ArchiveError::Attribute(GEOMETRY));
    };
    codec::restore_streams(version, &topology, &geometry, policy, limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_analytic::extrusion::Extrusion;
    use exedra_constructive::{builders, ir::Placement3, profile::SegTag};
    use layerstack::{InMemoryStore, Layer, LayerId, LiveStage, StageOptions};

    pub(super) fn source() -> NativeSource {
        let profile = builders::rect_centered(8.0, 6.0)
            .unwrap()
            .tagged(SegTag(17));
        NativeSource::from(
            Extrusion::from_profile(
                &profile,
                Placement3::translate(2.0, 3.0, 4.0),
                5.0,
                &exedra_profile::ProfilePolicy::new(1e-9),
            )
            .unwrap(),
        )
    }
    #[test]
    fn archive_restores_exact_tables_tags_and_fresh_selection_scope() {
        let source = source();
        let limits = ArchiveLimits::default();
        let policy = ValidationPolicy { max_gap: 1e-9 };
        let archive = NativeArchive::capture(&source, &limits).unwrap();
        let restored = archive.restore(&policy, &limits).unwrap();
        let expected = NativeSource::from_body(source.to_body(), source.frame()).unwrap();
        assert_eq!(restored.fingerprint(), expected.fingerprint());
        assert_eq!(restored.to_body(), source.to_body());
        assert_eq!(restored.frame(), source.frame());
        assert!(!restored.same_installation(&source));
        let mut malformed = archive.clone();
        malformed.geometry.pop();
        assert!(matches!(
            malformed.restore(&policy, &limits),
            Err(ArchiveError::Encoding { .. })
        ));
        let mut unknown = archive;
        unknown.version = 999;
        assert_eq!(
            unknown.restore(&policy, &limits).unwrap_err(),
            ArchiveError::Version(999)
        );
        assert!(matches!(
            NativeArchive::capture(
                &source,
                &ArchiveLimits {
                    geometry_values: 1,
                    ..limits
                }
            ),
            Err(ArchiveError::Budget(_))
        ));
    }

    #[test]
    fn shared_loop_expansion_is_budgeted_before_boundary_validation() {
        let mut parts = source().to_body().into_parts();
        let face = parts.faces[0].clone();
        let ring = &mut parts.loops[face.loops[0].index()];
        let coedges = ring.coedges.clone();
        ring.coedges = coedges.into_iter().cycle().take(32).collect();
        parts.faces = alloc::vec![face; 16];
        let policy = ValidationPolicy { max_gap: 1e-9 };
        let body = exedra_analytic::body::Body::try_from_parts(parts, &policy).unwrap();
        let source = NativeSource::from_body(body, Placement3::IDENTITY).unwrap();
        let limits = ArchiveLimits {
            max_boundary_checks: 512,
            ..ArchiveLimits::default()
        };
        let archive = NativeArchive::capture(&source, &limits).unwrap();
        archive.restore(&policy, &limits).unwrap();
        let refused = ArchiveLimits {
            max_boundary_checks: 511,
            ..limits
        };
        assert_eq!(
            NativeArchive::capture(&source, &refused),
            Err(ArchiveError::Budget("boundary checks"))
        );
        // An invalid policy would fail at the start of Body::validate. The work
        // budget must win before the shared loop's expanded uses are visited.
        assert!(matches!(
            archive.restore(&ValidationPolicy { max_gap: f64::NAN }, &refused),
            Err(ArchiveError::Budget("boundary checks"))
        ));
    }
    #[test]
    fn caller_stage_owns_atomic_creation_composed_readback_and_recovery() {
        let source = source();
        let limits = ArchiveLimits::default();
        let policy = ValidationPolicy { max_gap: 1e-9 };
        let mut store = InMemoryStore::default();
        let root = LayerId(1);
        store.insert_layer(Layer::new(root));
        let path = store.path("/Housing");
        let mut live = LiveStage::compose(&mut store, root, StageOptions::default());
        let tx = define_native_body(
            &mut store,
            &EditTarget::for_layer(root),
            path,
            &source,
            &limits,
        )
        .unwrap();
        live.apply(&mut store, &tx).unwrap();
        let restored = read_native_body(live.stage(), &store, path, &policy, &limits).unwrap();
        assert_eq!(restored.to_body(), source.to_body());
        assert!(
            live.apply(&mut store, &tx).is_err(),
            "existing authored prim must be preserved atomically"
        );
        let reopened = Stage::compose(&mut store, root, StageOptions::default());
        assert_eq!(
            read_native_body(&reopened, &store, path, &policy, &limits)
                .unwrap()
                .fingerprint(),
            restored.fingerprint()
        );
        let mut corrupt = Transaction::new();
        let property = PropertyPath::new(path, store.tokens().intern(FORMAT));
        corrupt.set_default(
            EditTarget::for_layer(root).property(property),
            Value::UInt(999),
        );
        let applied = live.apply(&mut store, &corrupt).unwrap();
        assert_eq!(
            read_native_body(live.stage(), &store, path, &policy, &limits).unwrap_err(),
            ArchiveError::Version(999)
        );
        live.apply(&mut store, &applied.inverse).unwrap();
        assert_eq!(
            read_native_body(live.stage(), &store, path, &policy, &limits)
                .unwrap()
                .fingerprint(),
            restored.fingerprint()
        );
        assert_eq!(
            source.to_body(),
            restored.to_body(),
            "failed restore never mutates retained native geometry"
        );
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use alloc::vec;
    use exedra_analytic::{
        body::{
            Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, FaceId, LoopId, LoopTrims,
            Sense, TrimCurve, VertexId,
        },
        geometry::{Curve, ParameterDomain, ParameterRange, Surface},
    };
    use exedra_constructive::{ir::Placement3, profile::SegTag};
    fn source() -> NativeSource {
        let range = ParameterRange::new(-0.4, core::f64::consts::TAU - 0.4).unwrap();
        let curve = Curve::ellipse(Placement3::translate(-0.0, 1.0, 2.0), 1.7, 1.2).unwrap();
        let p = curve.evaluate(range.bounds()[0]).unwrap().point;
        let body = Body::try_from_parts(
            BodyParts {
                vertices: vec![p],
                edges: vec![Edge::new(curve, range, VertexId(0), VertexId(0), ())],
                loops: vec![BoundaryLoop {
                    coedges: vec![Coedge {
                        edge: EdgeId(0),
                        reversed: false,
                    }],
                }],
                faces: vec![Face {
                    surface: Surface::plane(Placement3::translate(-0.0, 1.0, 2.0)).unwrap(),
                    domain: ParameterDomain {
                        u: ParameterRange::new(-1.7, 1.7).unwrap(),
                        v: ParameterRange::new(-1.2, 1.2).unwrap(),
                    },
                    sense: Sense::Forward,
                    loops: vec![LoopId(0)],
                    role: vec![SegTag(u32::MAX), SegTag(17)],
                }],
                trims: vec![LoopTrims {
                    loop_id: LoopId(0),
                    curves: vec![TrimCurve::Harmonic {
                        origin: [-0.0, 0.0],
                        linear: [0.0, -0.0],
                        cosine: [1.7, 0.0],
                        sine: [0.0, 1.2],
                        range,
                    }],
                }],
            },
            &ValidationPolicy { max_gap: 1e-9 },
        )
        .unwrap();
        assert_eq!(body.faces()[FaceId(0).index()].role.len(), 2);
        NativeSource::from_body(
            body,
            Placement3::rotate_y_then_translate(0.3, -0.0, 3.0, 7.0),
        )
        .unwrap()
    }
    #[test]
    fn explicit_trim_archive_preserves_bits_and_all_truncations_refuse() {
        let source = source();
        let limits = ArchiveLimits::default();
        let policy = ValidationPolicy { max_gap: 1e-9 };
        let archive = NativeArchive::capture(&source, &limits).unwrap();
        let restored = archive.restore(&policy, &limits).unwrap();
        let again = NativeArchive::capture(&restored, &limits).unwrap();
        assert_eq!(archive.topology, again.topology);
        assert_eq!(
            archive
                .geometry
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            again
                .geometry
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
        for n in 0..archive.topology.len() {
            let mut bad = archive.clone();
            bad.topology.truncate(n);
            assert!(bad.restore(&policy, &limits).is_err());
        }
        for n in 0..archive.geometry.len() {
            let mut bad = archive.clone();
            bad.geometry.truncate(n);
            assert!(bad.restore(&policy, &limits).is_err());
        }
        let mut bad = archive.clone();
        bad.topology.push(0);
        assert!(matches!(
            bad.restore(&policy, &limits),
            Err(ArchiveError::Encoding {
                record: "surplus data",
                ..
            })
        ));
        let mut bad = archive.clone();
        bad.geometry.push(0.0);
        assert!(matches!(
            bad.restore(&policy, &limits),
            Err(ArchiveError::Encoding {
                record: "surplus data",
                ..
            })
        ));
        let mut bad = archive.clone();
        bad.topology[0] = u32::MAX;
        assert!(matches!(
            bad.restore(
                &policy,
                &ArchiveLimits {
                    decoded_items: usize::MAX,
                    ..limits
                }
            ),
            Err(ArchiveError::Encoding {
                record: "vertex count",
                ..
            })
        ));
        let mut bad = archive.clone();
        bad.topology[2] = u32::MAX;
        assert!(matches!(
            bad.restore(&policy, &limits),
            Err(ArchiveError::Boundary(BodyError::MissingVertex { .. }))
        ));
        let mut bad = archive.clone();
        let n = bad.geometry.len();
        bad.geometry[n - 1] = bad.geometry[n - 2];
        assert!(matches!(
            bad.restore(&policy, &limits),
            Err(ArchiveError::Geometry {
                record: "trim",
                index: 0,
                ..
            })
        ));
        let mut bad = archive;
        bad.geometry[0] = f64::NAN;
        bad.topology[0] = u32::MAX;
        assert!(matches!(
            bad.restore(&policy, &limits),
            Err(ArchiveError::Placement(_))
        ));
    }
}

#[cfg(test)]
mod composition_recovery_tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use layerstack::{
        ArrayReadError, DeferredArraySource, InMemoryStore, Layer, LayerId, LayerOffset, LiveStage,
        StageOptions, SublayerEntry,
    };
    #[derive(Debug)]
    struct DeferredProbe {
        calls: AtomicUsize,
        error: ArrayReadError,
    }
    impl DeferredArraySource for DeferredProbe {
        fn materialize(&self) -> Result<&TypedArray, &ArrayReadError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(&self.error)
        }
        fn element_kind(&self) -> Value {
            Value::UInt(0)
        }
    }
    #[test]
    fn malformed_stronger_arrays_never_fall_back_or_materialize_unbounded_storage() {
        let source = tests::source();
        let limits = ArchiveLimits::default();
        let policy = ValidationPolicy { max_gap: 1e-9 };
        let mut store = InMemoryStore::default();
        let (root, weak) = (LayerId(1), LayerId(2));
        let mut layer = Layer::new(root);
        layer.sublayers_mut().push(SublayerEntry::with_asset(
            weak,
            "weak.usda",
            LayerOffset::IDENTITY,
        ));
        store.insert_layer(layer);
        store.insert_layer(Layer::new(weak));
        let path = store.path("/Housing");
        let mut live = LiveStage::compose(&mut store, root, StageOptions::default());
        let tx = define_native_body(
            &mut store,
            &EditTarget::for_layer(weak),
            path,
            &source,
            &limits,
        )
        .unwrap();
        live.apply(&mut store, &tx).unwrap();
        let mut stronger = Transaction::new();
        let target = EditTarget::for_layer(root);
        stronger.create_prim(target.prim(path), Specifier::Over, None);
        let property = PropertyPath::new(path, store.tokens().intern(GEOMETRY));
        stronger.create_property(
            target.property(property),
            PropertySpec::typed_attribute(PropertyType::parse("double[]").unwrap()).with_default(
                Value::TypedArray(TypedArray::Double(Arc::new(alloc::vec![0.0]))),
            ),
        );
        let applied = live.apply(&mut store, &stronger).unwrap();
        assert!(matches!(
            read_native_body(live.stage(), &store, path, &policy, &limits),
            Err(ArchiveError::Encoding { .. })
        ));
        live.apply(&mut store, &applied.inverse).unwrap();
        assert_eq!(
            read_native_body(live.stage(), &store, path, &policy, &limits)
                .unwrap()
                .to_body(),
            source.to_body()
        );
        let mut wrong = Transaction::new();
        wrong.create_prim(target.prim(path), Specifier::Over, None);
        wrong.create_property(
            target.property(property),
            PropertySpec::typed_attribute(PropertyType::parse("double[]").unwrap())
                .with_default(Value::UInt(7)),
        );
        assert!(
            live.apply(&mut store, &wrong).is_err(),
            "caller-owned transaction rejects the mismatched property type atomically"
        );
        assert_eq!(
            read_native_body(live.stage(), &store, path, &policy, &limits)
                .unwrap()
                .to_body(),
            source.to_body()
        );
        let probe = Arc::new(DeferredProbe {
            calls: AtomicUsize::new(0),
            error: ArrayReadError::BudgetExceeded { limit: 0 },
        });
        let mut deferred = Transaction::new();
        deferred.create_prim(target.prim(path), Specifier::Over, None);
        let property = PropertyPath::new(path, store.tokens().intern(TOPOLOGY));
        deferred.create_property(
            target.property(property),
            PropertySpec::typed_attribute(PropertyType::parse("uint[]").unwrap())
                .with_default(Value::TypedArray(TypedArray::Deferred(probe.clone()))),
        );
        live.apply(&mut store, &deferred).unwrap();
        probe.calls.store(0, Ordering::SeqCst);
        assert_eq!(
            read_native_body(live.stage(), &store, path, &policy, &limits).unwrap_err(),
            ArchiveError::DeferredAttribute(TOPOLOGY)
        );
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    }
}
