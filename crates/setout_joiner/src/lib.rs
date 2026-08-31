// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Adapter from exact setting-out quantities to Joiner construction records.
//!
//! This crate owns one transition: strict resolution of [`setout`] bindings,
//! followed by a single exact-to-`f64` lowering into [`joiner::OrientedBox`]
//! extents and dirty-channel mappings. It explicitly does not own propagation,
//! construction rules, reconstruction policy, or Joiner validation.
//!
//! ```text
//! setout::Evaluation -- exact endpoints --> setout_joiner -- f64 extent --> joiner
//!                    -- EvaluationDelta --> binding index -- dirty keys ----^
//! ```
//!
//! A setout-managed element receives its extent here. Frontends must not author a
//! second placement or extent for the same element.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use exedra_math::{cross, dot, normalize, scale, sub};
use joiner::{Construction, ConstructionError, Element, Evidence, OrientedBox};
use setout::{
    AccessError, AnyQuantity, CanonicalEncoder, ClaimKey, Evaluation, EvaluationDelta, Fingerprint,
    Length, Offset, Point3, Quantity, QuantityKey, SupportRef,
};

/// One quantity-to-claim link retained beside materialized construction data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedBinding {
    /// Stable quantity identity.
    pub quantity: QuantityKey,
    /// Operative structural claim.
    pub claim: ClaimKey,
    /// Interned structural support of that claim.
    pub support: SupportRef,
}

/// Geometry resolved from one setout binding.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedElementGeometry {
    /// Concrete analytic extent consumed by Joiner.
    pub extent: OrientedBox,
    /// Exact quantity/claim links that produced the extent.
    pub bindings: Box<[ResolvedBinding]>,
    /// Fingerprint of the final IEEE-754 geometry bits.
    pub lowered_fingerprint: Fingerprint,
}

/// Binding for a prismatic member whose longitudinal centerline has exact endpoints.
#[derive(Clone, Debug)]
pub struct SegmentMemberBinding {
    foot: Quantity<Point3>,
    head: Quantity<Point3>,
    length: Quantity<Length>,
    width: Quantity<Length>,
    depth: Quantity<Length>,
    width_reference: [f64; 3],
}

impl SegmentMemberBinding {
    /// Creates a member binding.
    ///
    /// `width_reference` fixes roll about the exact endpoint axis. It is
    /// orthogonalized in a documented operation order and must not be parallel
    /// to the member. The extent is centered on the endpoint line in its width
    /// and depth directions.
    #[must_use]
    pub fn new(
        foot: Quantity<Point3>,
        head: Quantity<Point3>,
        length: Quantity<Length>,
        width: Quantity<Length>,
        depth: Quantity<Length>,
        width_reference: [f64; 3],
    ) -> Self {
        Self {
            foot,
            head,
            length,
            width,
            depth,
            width_reference,
        }
    }

    /// Strictly resolves and lowers this member.
    pub fn resolve(
        &self,
        evaluation: &Evaluation,
    ) -> Result<ResolvedElementGeometry, ResolveError> {
        let foot = evaluation.exact(&self.foot)?;
        let head = evaluation.exact(&self.head)?;
        let length = evaluation.exact(&self.length)?;
        let width = evaluation.exact(&self.width)?;
        let depth = evaluation.exact(&self.depth)?;
        let foot_f64 = lower_point(foot);
        let head_f64 = lower_point(head);
        let tangent =
            normalize(sub(head_f64, foot_f64)).ok_or(ResolveError::CoincidentEndpoints)?;
        // Gram-Schmidt is intentional here: a caller supplies a semantic roll
        // reference (for the basilica, the longitudinal X axis), not a second
        // allegedly exact frame. The fixed subtract→dot→scale→normalize order
        // is part of the lowered-bits fingerprint.
        let projected = scale(tangent, dot(self.width_reference, tangent));
        let width_axis = normalize(sub(self.width_reference, projected))
            .ok_or(ResolveError::ParallelWidthReference)?;
        let depth_axis =
            normalize(cross(tangent, width_axis)).ok_or(ResolveError::ParallelWidthReference)?;
        let length_f64 = lower_length(length);
        let endpoint_length = exedra_math::norm(sub(head_f64, foot_f64));
        let endpoint_error = (endpoint_length - length_f64).abs();
        // Exact integer Pythagorean may select a neighbouring iota for an
        // irrational root. Anything larger than two iota indicates that the
        // bound length and endpoints did not come from the same hypothesis.
        if endpoint_error > 2.0 * lower_length(Length::MIN) {
            return Err(ResolveError::LengthEndpointMismatch {
                declared: length_f64,
                endpoints: endpoint_length,
            });
        }
        let width_f64 = lower_length(width);
        let depth_f64 = lower_length(depth);
        let origin = sub(
            sub(foot_f64, scale(width_axis, width_f64 * 0.5)),
            scale(depth_axis, depth_f64 * 0.5),
        );
        let extent = OrientedBox {
            origin,
            axes: [tangent, width_axis, depth_axis],
            size: [length_f64, width_f64, depth_f64],
        };
        let bindings = resolve_bindings(
            evaluation,
            [
                self.foot.erase(),
                self.head.erase(),
                self.length.erase(),
                self.width.erase(),
                self.depth.erase(),
            ],
        )?;
        Ok(ResolvedElementGeometry {
            lowered_fingerprint: fingerprint_extent(&extent),
            extent,
            bindings,
        })
    }
}

/// Axis-aligned box binding with exact origin and dimensions.
#[derive(Clone, Debug)]
pub struct AxisAlignedBoxBinding {
    origin: Quantity<Point3>,
    size_x: Quantity<Length>,
    size_y: Quantity<Length>,
    size_z: Quantity<Length>,
}

impl AxisAlignedBoxBinding {
    /// Creates an axis-aligned exact box binding.
    #[must_use]
    pub fn new(
        origin: Quantity<Point3>,
        size_x: Quantity<Length>,
        size_y: Quantity<Length>,
        size_z: Quantity<Length>,
    ) -> Self {
        Self {
            origin,
            size_x,
            size_y,
            size_z,
        }
    }

    /// Strictly resolves and lowers this box.
    pub fn resolve(
        &self,
        evaluation: &Evaluation,
    ) -> Result<ResolvedElementGeometry, ResolveError> {
        let origin = evaluation.exact(&self.origin)?;
        let size = [
            evaluation.exact(&self.size_x)?,
            evaluation.exact(&self.size_y)?,
            evaluation.exact(&self.size_z)?,
        ];
        let extent = OrientedBox::axis_aligned(lower_point(origin), size.map(lower_length));
        let bindings = resolve_bindings(
            evaluation,
            [
                self.origin.erase(),
                self.size_x.erase(),
                self.size_y.erase(),
                self.size_z.erase(),
            ],
        )?;
        Ok(ResolvedElementGeometry {
            lowered_fingerprint: fingerprint_extent(&extent),
            extent,
            bindings,
        })
    }
}

/// Strictly lowers an exact length to meters.
#[must_use]
pub fn lower_length(value: Length) -> f64 {
    value.as_meters()
}

/// Strictly lowers an exact signed offset to meters.
#[must_use]
pub fn lower_offset(value: Offset) -> f64 {
    value.as_meters()
}

/// Strictly lowers one exact point to meters.
#[must_use]
pub fn lower_point(value: Point3) -> [f64; 3] {
    [
        lower_offset(value.x),
        lower_offset(value.y),
        lower_offset(value.z),
    ]
}

fn resolve_bindings<const N: usize>(
    evaluation: &Evaluation,
    quantities: [AnyQuantity; N],
) -> Result<Box<[ResolvedBinding]>, ResolveError> {
    let provenance = evaluation.provenance();
    quantities
        .into_iter()
        .map(|quantity| {
            let claim = provenance
                .operative(quantity.key())
                .ok_or(ResolveError::MissingProvenance)?;
            Ok(ResolvedBinding {
                quantity: quantity.key().clone(),
                claim: claim.key(),
                support: claim.support(),
            })
        })
        .collect()
}

fn fingerprint_extent(extent: &OrientedBox) -> Fingerprint {
    let mut encoder = CanonicalEncoder::new("setout-joiner/lowered-oriented-box");
    for value in extent
        .origin
        .iter()
        .chain(extent.axes.iter().flatten())
        .chain(extent.size.iter())
    {
        encoder.u64(value.to_bits());
    }
    encoder.finish()
}

/// Materializes the sole analytic extent of a setout-managed Joiner element.
pub fn add_resolved_element(
    construction: &mut Construction,
    key: &str,
    role: &str,
    material: &str,
    geometry: &ResolvedElementGeometry,
    evidence: Evidence,
    required_supports: usize,
) -> Result<(), ConstructionError> {
    construction.add_element(
        Element::new(key, role, material, geometry.extent.clone(), evidence)
            .with_required_supports(required_supports),
    )?;
    Ok(())
}

/// Stable, reversible-enough flattening of a setout path for Joiner's no-slash grammar.
///
/// Underscore is escaped along with slash, so `a/b`, `a_2fb`, and `a--b`
/// cannot collide. The `sq-` prefix reserves the namespace for adapter-owned
/// construction keys.
#[must_use]
pub fn joiner_key(quantity: &QuantityKey) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::from("sq-");
    for byte in quantity.as_str().bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.') {
            output.push(char::from(byte));
        } else {
            output.push('_');
            output.push(char::from(HEX[(byte >> 4) as usize]));
            output.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    output
}

/// Joiner invalidation channels affected by one resolved binding.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DirtyChannel {
    /// Analytic extent or realized geometry.
    Geometry,
    /// Contact positions or overlap.
    Contact,
    /// Member incidence or load path.
    LoadPath,
}

/// One construction element and the exact channels dirtied by a setout delta.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirtyElement {
    /// Stable Joiner element key.
    pub element: Box<str>,
    /// Sorted, deduplicated dirty channels.
    pub channels: Box<[DirtyChannel]>,
}

/// Quantity-granular dependency index from setout into Joiner.
#[derive(Clone, Debug, Default)]
pub struct BindingIndex {
    by_quantity: BTreeMap<QuantityKey, Vec<(Box<str>, DirtyChannel)>>,
}

impl BindingIndex {
    /// Creates an empty binding index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one element/channel dependency for every listed quantity.
    pub fn bind(
        &mut self,
        element: impl Into<Box<str>>,
        quantities: impl IntoIterator<Item = QuantityKey>,
        channels: impl IntoIterator<Item = DirtyChannel>,
    ) {
        let element = element.into();
        let channels: Vec<_> = channels.into_iter().collect();
        for quantity in quantities {
            let bindings = self.by_quantity.entry(quantity).or_default();
            bindings.extend(
                channels
                    .iter()
                    .copied()
                    .map(|channel| (element.clone(), channel)),
            );
        }
    }

    /// Maps an evaluation delta to stable construction dirty keys.
    #[must_use]
    pub fn dirty(&self, delta: &EvaluationDelta) -> Box<[DirtyElement]> {
        let mut dirty: BTreeMap<Box<str>, BTreeSet<DirtyChannel>> = BTreeMap::new();
        for quantity in &delta.quantities_changed {
            if let Some(bindings) = self.by_quantity.get(quantity) {
                for (element, channel) in bindings {
                    dirty.entry(element.clone()).or_default().insert(*channel);
                }
            }
        }
        dirty
            .into_iter()
            .map(|(element, channels)| DirtyElement {
                element,
                channels: channels.into_iter().collect(),
            })
            .collect()
    }
}

/// Failure to resolve an exact geometry binding.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ResolveError {
    /// Strict quantity access failed.
    Access(AccessError),
    /// Exact segment endpoints coincide.
    CoincidentEndpoints,
    /// The roll-reference vector is parallel to the segment.
    ParallelWidthReference,
    /// Bound exact length disagrees with the endpoint distance.
    LengthEndpointMismatch {
        /// Lowered exact length.
        declared: f64,
        /// Distance between lowered endpoints.
        endpoints: f64,
    },
    /// Evaluation had a value but no operative structural claim.
    MissingProvenance,
}

impl From<AccessError> for ResolveError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Access(error) => write!(formatter, "setout access failed: {error}"),
            Self::CoincidentEndpoints => formatter.write_str("segment endpoints coincide"),
            Self::ParallelWidthReference => {
                formatter.write_str("segment width reference is parallel to its endpoints")
            }
            Self::LengthEndpointMismatch {
                declared,
                endpoints,
            } => write!(
                formatter,
                "declared length {declared} m disagrees with endpoint distance {endpoints} m"
            ),
            Self::MissingProvenance => {
                formatter.write_str("operative binding provenance is missing")
            }
        }
    }
}

impl core::error::Error for ResolveError {}

#[cfg(test)]
mod tests {
    use super::*;
    use setout::{
        EvaluationScenarioBuilder, Knowledge, Length, NetworkBuilder, Offset, Point3, Quantity,
        QuantityPolicy, RootClaimSetBuilder, compile_plan, evaluate,
    };

    struct SegmentFixture {
        evaluation: Evaluation,
        foot: Quantity<Point3>,
        head: Quantity<Point3>,
        length: Quantity<Length>,
        width: Quantity<Length>,
        depth: Quantity<Length>,
    }

    fn fixture(declared_length_mm: u64) -> SegmentFixture {
        let mut network = NetworkBuilder::new();
        let foot = network
            .declare::<Point3>("frame/foot", QuantityPolicy::unrestricted())
            .unwrap();
        let head = network
            .declare::<Point3>("frame/head", QuantityPolicy::unrestricted())
            .unwrap();
        let length = network
            .declare::<Length>("frame/length", QuantityPolicy::positive())
            .unwrap();
        let width = network
            .declare::<Length>("frame/width", QuantityPolicy::positive())
            .unwrap();
        let depth = network
            .declare::<Length>("frame/depth", QuantityPolicy::positive())
            .unwrap();
        let definition = network.finish().unwrap();
        let mut roots = RootClaimSetBuilder::new(&definition);
        roots
            .author(
                "root/foot",
                &foot,
                Knowledge::exact(Point3::new(Offset::ZERO, Offset::ZERO, Offset::ZERO)),
            )
            .unwrap()
            .author(
                "root/head",
                &head,
                Knowledge::exact(Point3::new(
                    Offset::ZERO,
                    Offset::meters(3).unwrap(),
                    Offset::meters(4).unwrap(),
                )),
            )
            .unwrap()
            .author(
                "root/length",
                &length,
                Knowledge::exact(Length::millimeters(declared_length_mm).unwrap()),
            )
            .unwrap()
            .author(
                "root/width",
                &width,
                Knowledge::exact(Length::millimeters(200).unwrap()),
            )
            .unwrap()
            .author(
                "root/depth",
                &depth,
                Knowledge::exact(Length::millimeters(300).unwrap()),
            )
            .unwrap();
        let roots = roots.finish().unwrap();
        let scenario = EvaluationScenarioBuilder::new("frame/operative")
            .unwrap()
            .activate_all(&roots)
            .finish(&roots)
            .unwrap();
        let plan = compile_plan(&definition, &roots, &scenario).unwrap();
        let evaluation = evaluate(&definition, &roots, &scenario, &plan).unwrap();
        SegmentFixture {
            evaluation,
            foot,
            head,
            length,
            width,
            depth,
        }
    }

    fn binding(fixture: &SegmentFixture) -> SegmentMemberBinding {
        SegmentMemberBinding::new(
            fixture.foot.clone(),
            fixture.head.clone(),
            fixture.length.clone(),
            fixture.width.clone(),
            fixture.depth.clone(),
            [1.0, 0.0, 0.0],
        )
    }

    #[test]
    fn exact_segment_is_lowered_once_with_roll_and_provenance_intact() {
        let fixture = fixture(5_000);
        let first = binding(&fixture).resolve(&fixture.evaluation).unwrap();
        let second = binding(&fixture).resolve(&fixture.evaluation).unwrap();

        // A 3-4-5 fixture gives known rational directions and exposes swaps
        // between tangent, width, and depth after the one floating lowering.
        assert_point_close(first.extent.axes[0], [0.0, 0.6, 0.8]);
        assert_point_close(first.extent.axes[1], [1.0, 0.0, 0.0]);
        assert_point_close(first.extent.axes[2], [0.0, 0.8, -0.6]);
        assert_point_close(first.extent.size, [5.0, 0.2, 0.3]);
        assert_eq!(first.bindings.len(), 5);
        assert!(
            first
                .bindings
                .iter()
                .all(|link| { fixture.evaluation.provenance().claim(link.claim).is_some() })
        );
        assert_eq!(first.lowered_fingerprint, second.lowered_fingerprint);
        assert_eq!(first.extent, second.extent);
    }

    fn assert_point_close(actual: [f64; 3], expected: [f64; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 1.0e-12,
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn inconsistent_extent_is_rejected_and_delta_maps_only_bound_channels() {
        let fixture = fixture(4_900);
        assert!(matches!(
            binding(&fixture).resolve(&fixture.evaluation),
            Err(ResolveError::LengthEndpointMismatch { .. })
        ));

        let mut index = BindingIndex::new();
        index.bind(
            "principal-rafter",
            [fixture.foot.key().clone(), fixture.head.key().clone()],
            [DirtyChannel::Geometry, DirtyChannel::Contact],
        );
        index.bind(
            "unrelated-width-consumer",
            [fixture.width.key().clone()],
            [DirtyChannel::Geometry],
        );
        // A head-point edit dirties the rafter's geometry and contacts but not
        // a consumer bound solely to the unchanged width quantity.
        let dirty = index.dirty(&EvaluationDelta {
            quantities_changed: Box::new([fixture.head.key().clone()]),
            claims_changed: Box::new([]),
        });
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].element.as_ref(), "principal-rafter");
        assert_eq!(
            dirty[0].channels.as_ref(),
            [DirtyChannel::Geometry, DirtyChannel::Contact]
        );
    }
}
