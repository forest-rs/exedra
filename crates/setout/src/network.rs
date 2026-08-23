// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Immutable quantity and relation definitions.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

use crate::fingerprint::{CanonicalEncoder, Fingerprint};
use crate::key::{KeyError, MethodId, QuantityKey, RelationKey};
use crate::value::{
    ArithmeticError, Count, Domain, DomainTag, ExactnessTrace, Length, Point3, Rational,
    RootRounding, Round, Value,
};

/// Definition-local dense handle for a quantity.
///
/// Slots accelerate evaluation only. Persisted identity always uses
/// [`QuantityKey`], because a slot can change when a definition is rebuilt.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct QuantitySlot(u32);

impl QuantitySlot {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Typed handle to a quantity in one immutable network definition.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Quantity<T: Domain> {
    key: QuantityKey,
    slot: QuantitySlot,
    marker: PhantomData<fn() -> T>,
}

impl<T: Domain> Clone for Quantity<T> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            slot: self.slot,
            marker: PhantomData,
        }
    }
}

impl<T: Domain> Quantity<T> {
    /// Returns stable external identity.
    #[must_use]
    pub fn key(&self) -> &QuantityKey {
        &self.key
    }

    /// Erases the compile-time domain while retaining its runtime tag.
    #[must_use]
    pub fn erase(&self) -> AnyQuantity {
        AnyQuantity {
            key: self.key.clone(),
            slot: self.slot,
            domain: T::TAG,
        }
    }

    pub(crate) const fn slot(&self) -> QuantitySlot {
        self.slot
    }
}

/// Domain-erased quantity handle used by plans and provenance inspection.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AnyQuantity {
    key: QuantityKey,
    slot: QuantitySlot,
    domain: DomainTag,
}

impl AnyQuantity {
    /// Returns stable external identity.
    #[must_use]
    pub fn key(&self) -> &QuantityKey {
        &self.key
    }

    /// Returns the quantity's domain.
    #[must_use]
    pub const fn domain(&self) -> DomainTag {
        self.domain
    }

    pub(crate) const fn slot(&self) -> QuantitySlot {
        self.slot
    }
}

/// Admissibility policy attached to a quantity definition.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct QuantityPolicy<T: Domain> {
    kind: PolicyKind,
    marker: PhantomData<fn() -> T>,
}

impl<T: Domain> QuantityPolicy<T> {
    /// Accepts every value in the domain.
    #[must_use]
    pub const fn unrestricted() -> Self {
        Self {
            kind: PolicyKind::Unrestricted,
            marker: PhantomData,
        }
    }

    /// Requires a non-negative length or count.
    #[must_use]
    pub const fn non_negative() -> Self {
        Self {
            kind: PolicyKind::NonNegative,
            marker: PhantomData,
        }
    }

    /// Requires a strictly positive length or count.
    #[must_use]
    pub const fn positive() -> Self {
        Self {
            kind: PolicyKind::Positive,
            marker: PhantomData,
        }
    }
}

impl<T: Domain> Default for QuantityPolicy<T> {
    fn default() -> Self {
        Self::unrestricted()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum PolicyKind {
    Unrestricted,
    NonNegative,
    Positive,
}

impl PolicyKind {
    const fn code(self) -> u8 {
        match self {
            Self::Unrestricted => 0,
            Self::NonNegative => 1,
            Self::Positive => 2,
        }
    }

    pub(crate) fn accepts(self, value: &Value) -> bool {
        match (self, value) {
            (Self::Unrestricted, _) => true,
            (Self::NonNegative, Value::Length(value)) => value.iota() >= 0,
            (Self::Positive, Value::Length(value)) => value.iota() > 0,
            (Self::NonNegative, Value::Count(_)) => true,
            (Self::Positive, Value::Count(value)) => value.get() > 0,
            // Sign policies have no coherent meaning for other domains; the
            // builder rejects those pairings instead of silently accepting.
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct QuantityDef {
    pub(crate) quantity: AnyQuantity,
    pub(crate) policy: PolicyKind,
}

/// Immutable network of typed quantities and multi-way exact relations.
#[derive(Clone, Debug)]
pub struct NetworkDef {
    pub(crate) quantities: Vec<QuantityDef>,
    pub(crate) quantity_by_key: BTreeMap<QuantityKey, QuantitySlot>,
    pub(crate) relations: Vec<RelationDef>,
    fingerprint: Fingerprint,
}

impl NetworkDef {
    /// Returns the versioned canonical definition fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Finds a domain-erased quantity by stable key.
    #[must_use]
    pub fn quantity(&self, key: &QuantityKey) -> Option<&AnyQuantity> {
        self.quantity_by_key
            .get(key)
            .map(|slot| &self.quantities[slot.index()].quantity)
    }

    /// Number of declared quantities.
    #[must_use]
    pub fn quantity_count(&self) -> usize {
        self.quantities.len()
    }

    /// Number of declared relations.
    #[must_use]
    pub fn relation_count(&self) -> usize {
        self.relations.len()
    }

    pub(crate) fn quantity_def(&self, slot: QuantitySlot) -> &QuantityDef {
        &self.quantities[slot.index()]
    }
}

/// Builder for an immutable [`NetworkDef`].
#[derive(Debug, Default)]
pub struct NetworkBuilder {
    quantities: Vec<QuantityDef>,
    quantity_by_key: BTreeMap<QuantityKey, QuantitySlot>,
    relations: BTreeMap<RelationKey, RelationDef>,
}

impl NetworkBuilder {
    /// Starts an empty network definition.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares one stable, typed quantity.
    pub fn declare<T: Domain>(
        &mut self,
        key: &str,
        policy: QuantityPolicy<T>,
    ) -> Result<Quantity<T>, BuildError> {
        if !matches!(policy.kind, PolicyKind::Unrestricted)
            && !matches!(T::TAG, DomainTag::Length | DomainTag::Count)
        {
            return Err(BuildError::InvalidPolicy);
        }
        let key = QuantityKey::new(key)?;
        if self.quantity_by_key.contains_key(&key) {
            return Err(BuildError::DuplicateQuantity(key));
        }
        let index = u32::try_from(self.quantities.len()).map_err(|_| BuildError::TooManyItems)?;
        let slot = QuantitySlot(index);
        let quantity = Quantity {
            key: key.clone(),
            slot,
            marker: PhantomData,
        };
        self.quantity_by_key.insert(key.clone(), slot);
        self.quantities.push(QuantityDef {
            quantity: AnyQuantity {
                key,
                slot,
                domain: T::TAG,
            },
            policy: policy.kind,
        });
        Ok(quantity)
    }

    /// Adds one relation and all of its directed methods.
    pub fn relate<R: RelationSpec>(&mut self, relation: R) -> Result<(), BuildError> {
        let relation = relation.build()?;
        if self.relations.contains_key(&relation.key) {
            return Err(BuildError::DuplicateRelation(relation.key));
        }
        for participant in &relation.participants {
            let Some(definition) = self.quantities.get(participant.slot().index()) else {
                return Err(BuildError::ForeignQuantity);
            };
            if definition.quantity.key() != participant.key()
                || definition.quantity.domain() != participant.domain()
            {
                return Err(BuildError::ForeignQuantity);
            }
        }
        for method in &relation.methods {
            let target = self
                .quantities
                .get(method.target.index())
                .ok_or(BuildError::ForeignQuantity)?;
            if target.quantity.domain != method.operation.output_domain() {
                return Err(BuildError::DomainMismatch);
            }
            for input in &method.inputs {
                if self.quantities.get(input.index()).is_none() {
                    return Err(BuildError::ForeignQuantity);
                }
            }
        }
        self.relations.insert(relation.key.clone(), relation);
        Ok(())
    }

    /// Validates, canonicalizes, and freezes the definition.
    pub fn finish(self) -> Result<NetworkDef, BuildError> {
        let mut relations: Vec<_> = self.relations.into_values().collect();
        for relation in &mut relations {
            relation
                .methods
                .sort_by(|left, right| left.id.cmp(&right.id));
        }
        let mut encoder = CanonicalEncoder::new("setout/network-definition");
        let mut quantities: Vec<_> = self.quantities.iter().collect();
        quantities.sort_by(|left, right| left.quantity.key.cmp(&right.quantity.key));
        encoder.u32(u32::try_from(quantities.len()).map_err(|_| BuildError::TooManyItems)?);
        for definition in quantities {
            encoder.str(definition.quantity.key.as_str());
            encoder.u8(definition.quantity.domain.code());
            encoder.u8(definition.policy.code());
        }
        encoder.u32(u32::try_from(relations.len()).map_err(|_| BuildError::TooManyItems)?);
        for relation in &relations {
            relation.encode(&mut encoder);
        }
        let fingerprint = encoder.finish();
        Ok(NetworkDef {
            quantities: self.quantities,
            quantity_by_key: self.quantity_by_key,
            relations,
            fingerprint,
        })
    }
}

/// Error that prevents construction of an immutable definition.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BuildError {
    /// A semantic key is invalid.
    InvalidKey(KeyError),
    /// A quantity key is already declared.
    DuplicateQuantity(QuantityKey),
    /// A relation key is already declared.
    DuplicateRelation(RelationKey),
    /// A relation references a handle from another builder.
    ForeignQuantity,
    /// Relation input/output domains do not agree.
    DomainMismatch,
    /// A sign policy was attached to a non-ordered domain.
    InvalidPolicy,
    /// The definition exceeds a stable `u32` storage boundary.
    TooManyItems,
    /// A relation parameter is invalid.
    InvalidRelation,
}

impl From<KeyError> for BuildError {
    fn from(error: KeyError) -> Self {
        Self::InvalidKey(error)
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey(error) => write!(formatter, "invalid key: {error}"),
            Self::DuplicateQuantity(key) => write!(formatter, "duplicate quantity `{key}`"),
            Self::DuplicateRelation(key) => write!(formatter, "duplicate relation `{key}`"),
            Self::ForeignQuantity => formatter.write_str("relation references a foreign quantity"),
            Self::DomainMismatch => formatter.write_str("relation domains do not agree"),
            Self::InvalidPolicy => formatter.write_str("quantity policy is invalid for its domain"),
            Self::TooManyItems => formatter.write_str("definition exceeds u32 item capacity"),
            Self::InvalidRelation => formatter.write_str("relation parameters are invalid"),
        }
    }
}

impl core::error::Error for BuildError {}

#[doc(hidden)]
pub mod relation_private {
    pub trait Sealed {}
}

/// A typed relation that can register its directed methods in a network.
pub trait RelationSpec: relation_private::Sealed {
    #[doc(hidden)]
    fn build(self) -> Result<RelationDef, BuildError>;
}

/// Multi-way additive relation `left + right = total`.
#[derive(Clone, Debug)]
pub struct Sum<T: Domain> {
    key: RelationKey,
    left: Quantity<T>,
    right: Quantity<T>,
    total: Quantity<T>,
}

impl<T: Domain> Sum<T> {
    /// Creates an additive relation. Only `Length` and `Count` are currently supported.
    pub fn new(
        key: &str,
        left: Quantity<T>,
        right: Quantity<T>,
        total: Quantity<T>,
    ) -> Result<Self, BuildError> {
        Ok(Self {
            key: RelationKey::new(key)?,
            left,
            right,
            total,
        })
    }
}

impl<T: Domain> relation_private::Sealed for Sum<T> {}

impl<T: Domain> RelationSpec for Sum<T> {
    fn build(self) -> Result<RelationDef, BuildError> {
        let family = match T::TAG {
            DomainTag::Length => AddFamily::Length,
            DomainTag::Count => AddFamily::Count,
            _ => return Err(BuildError::InvalidRelation),
        };
        Ok(RelationDef {
            key: self.key,
            participants: vec![self.left.erase(), self.right.erase(), self.total.erase()],
            methods: vec![
                MethodDef::new(
                    "left-plus-right-to-total",
                    self.total.slot,
                    [self.left.slot, self.right.slot],
                    Operation::Add(family),
                )?,
                MethodDef::new(
                    "total-right-to-left",
                    self.left.slot,
                    [self.total.slot, self.right.slot],
                    Operation::Subtract(family),
                )?,
                MethodDef::new(
                    "total-left-to-right",
                    self.right.slot,
                    [self.total.slot, self.left.slot],
                    Operation::Subtract(family),
                )?,
            ],
        })
    }
}

/// Bidirectional relation `output = input × factor` for exact lengths.
#[derive(Clone, Debug)]
pub struct ScaleLength {
    key: RelationKey,
    input: Quantity<Length>,
    output: Quantity<Length>,
    factor: Rational,
    round: Round,
}

impl ScaleLength {
    /// Creates an exactly integral scale relation.
    pub fn new(
        key: &str,
        input: Quantity<Length>,
        output: Quantity<Length>,
        factor: Rational,
    ) -> Result<Self, BuildError> {
        Self::with_round(key, input, output, factor, Round::Exact)
    }

    /// Creates a scale relation with an explicit quantization policy.
    pub fn with_round(
        key: &str,
        input: Quantity<Length>,
        output: Quantity<Length>,
        factor: Rational,
        round: Round,
    ) -> Result<Self, BuildError> {
        if factor.numerator() == 0 {
            return Err(BuildError::InvalidRelation);
        }
        Ok(Self {
            key: RelationKey::new(key)?,
            input,
            output,
            factor,
            round,
        })
    }
}

impl relation_private::Sealed for ScaleLength {}

impl RelationSpec for ScaleLength {
    fn build(self) -> Result<RelationDef, BuildError> {
        Ok(RelationDef {
            key: self.key,
            participants: vec![self.input.erase(), self.output.erase()],
            methods: vec![
                MethodDef::new(
                    "input-times-factor-to-output",
                    self.output.slot,
                    [self.input.slot],
                    Operation::ScaleLength {
                        factor: self.factor,
                        round: self.round,
                    },
                )?,
                MethodDef::new(
                    "output-divided-factor-to-input",
                    self.input.slot,
                    [self.output.slot],
                    Operation::ScaleLength {
                        factor: self
                            .factor
                            .checked_reciprocal()
                            .map_err(|_| BuildError::InvalidRelation)?,
                        round: self.round,
                    },
                )?,
            ],
        })
    }
}

/// Bidirectional fixed offset relation `output = input + offset`.
#[derive(Clone, Debug)]
pub struct OffsetLength {
    key: RelationKey,
    input: Quantity<Length>,
    output: Quantity<Length>,
    offset: Length,
}

impl OffsetLength {
    /// Creates a fixed exact offset.
    pub fn new(
        key: &str,
        input: Quantity<Length>,
        output: Quantity<Length>,
        offset: Length,
    ) -> Result<Self, BuildError> {
        Ok(Self {
            key: RelationKey::new(key)?,
            input,
            output,
            offset,
        })
    }
}

impl relation_private::Sealed for OffsetLength {}

impl RelationSpec for OffsetLength {
    fn build(self) -> Result<RelationDef, BuildError> {
        Ok(RelationDef {
            key: self.key,
            participants: vec![self.input.erase(), self.output.erase()],
            methods: vec![
                MethodDef::new(
                    "input-plus-offset-to-output",
                    self.output.slot,
                    [self.input.slot],
                    Operation::OffsetLength(self.offset),
                )?,
                MethodDef::new(
                    "output-minus-offset-to-input",
                    self.input.slot,
                    [self.output.slot],
                    Operation::OffsetLength(
                        self.offset
                            .checked_neg()
                            .map_err(|_| BuildError::InvalidRelation)?,
                    ),
                )?,
            ],
        })
    }
}

/// Bidirectional equality between two quantities in the same domain.
#[derive(Clone, Debug)]
pub struct Equal<T: Domain> {
    key: RelationKey,
    left: Quantity<T>,
    right: Quantity<T>,
}

impl<T: Domain> Equal<T> {
    /// Creates an equality relation.
    pub fn new(key: &str, left: Quantity<T>, right: Quantity<T>) -> Result<Self, BuildError> {
        Ok(Self {
            key: RelationKey::new(key)?,
            left,
            right,
        })
    }
}

impl<T: Domain> relation_private::Sealed for Equal<T> {}

impl<T: Domain> RelationSpec for Equal<T> {
    fn build(self) -> Result<RelationDef, BuildError> {
        Ok(RelationDef {
            key: self.key,
            participants: vec![self.left.erase(), self.right.erase()],
            methods: vec![
                MethodDef::new(
                    "left-to-right",
                    self.right.slot,
                    [self.left.slot],
                    Operation::Identity(T::TAG),
                )?,
                MethodDef::new(
                    "right-to-left",
                    self.left.slot,
                    [self.right.slot],
                    Operation::Identity(T::TAG),
                )?,
            ],
        })
    }
}

/// Multi-way exact relation `rise = run × ratio`.
#[derive(Clone, Debug)]
pub struct Pitch {
    key: RelationKey,
    run: Quantity<Length>,
    rise: Quantity<Length>,
    ratio: Quantity<Rational>,
    round: Round,
}

impl Pitch {
    /// Creates a pitch relation requiring integral length results.
    pub fn new(
        key: &str,
        run: Quantity<Length>,
        rise: Quantity<Length>,
        ratio: Quantity<Rational>,
    ) -> Result<Self, BuildError> {
        Self::with_round(key, run, rise, ratio, Round::Exact)
    }

    /// Creates a pitch relation with an explicit length quantization policy.
    pub fn with_round(
        key: &str,
        run: Quantity<Length>,
        rise: Quantity<Length>,
        ratio: Quantity<Rational>,
        round: Round,
    ) -> Result<Self, BuildError> {
        Ok(Self {
            key: RelationKey::new(key)?,
            run,
            rise,
            ratio,
            round,
        })
    }
}

impl relation_private::Sealed for Pitch {}

impl RelationSpec for Pitch {
    fn build(self) -> Result<RelationDef, BuildError> {
        Ok(RelationDef {
            key: self.key,
            participants: vec![self.run.erase(), self.rise.erase(), self.ratio.erase()],
            methods: vec![
                MethodDef::new(
                    "run-and-ratio-to-rise",
                    self.rise.slot,
                    [self.run.slot, self.ratio.slot],
                    Operation::PitchRise { round: self.round },
                )?,
                MethodDef::new(
                    "rise-and-ratio-to-run",
                    self.run.slot,
                    [self.rise.slot, self.ratio.slot],
                    Operation::PitchRun { round: self.round },
                )?,
                MethodDef::new(
                    "run-and-rise-to-ratio",
                    self.ratio.slot,
                    [self.run.slot, self.rise.slot],
                    Operation::PitchRatio,
                )?,
            ],
        })
    }
}

/// Multi-way exact integer relation `leg_a² + leg_b² = hypotenuse²`.
#[derive(Clone, Debug)]
pub struct Pythagorean {
    key: RelationKey,
    leg_a: Quantity<Length>,
    leg_b: Quantity<Length>,
    hypotenuse: Quantity<Length>,
    round: Round,
}

impl Pythagorean {
    /// Creates an integer-root relation using nearest-iota selection.
    pub fn new(
        key: &str,
        leg_a: Quantity<Length>,
        leg_b: Quantity<Length>,
        hypotenuse: Quantity<Length>,
    ) -> Result<Self, BuildError> {
        Self::with_round(key, leg_a, leg_b, hypotenuse, Round::Nearest)
    }

    /// Creates an integer-root relation with explicit selection policy.
    pub fn with_round(
        key: &str,
        leg_a: Quantity<Length>,
        leg_b: Quantity<Length>,
        hypotenuse: Quantity<Length>,
        round: Round,
    ) -> Result<Self, BuildError> {
        Ok(Self {
            key: RelationKey::new(key)?,
            leg_a,
            leg_b,
            hypotenuse,
            round,
        })
    }
}

impl relation_private::Sealed for Pythagorean {}

impl RelationSpec for Pythagorean {
    fn build(self) -> Result<RelationDef, BuildError> {
        Ok(RelationDef {
            key: self.key,
            participants: vec![
                self.leg_a.erase(),
                self.leg_b.erase(),
                self.hypotenuse.erase(),
            ],
            methods: vec![
                MethodDef::new(
                    "legs-to-hypotenuse",
                    self.hypotenuse.slot,
                    [self.leg_a.slot, self.leg_b.slot],
                    Operation::Pythagorean {
                        subtract: false,
                        round: self.round,
                    },
                )?,
                MethodDef::new(
                    "hypotenuse-and-leg-b-to-leg-a",
                    self.leg_a.slot,
                    [self.hypotenuse.slot, self.leg_b.slot],
                    Operation::Pythagorean {
                        subtract: true,
                        round: self.round,
                    },
                )?,
                MethodDef::new(
                    "hypotenuse-and-leg-a-to-leg-b",
                    self.leg_b.slot,
                    [self.hypotenuse.slot, self.leg_a.slot],
                    Operation::Pythagorean {
                        subtract: true,
                        round: self.round,
                    },
                )?,
            ],
        })
    }
}

/// Closed-form relation between exact point components and a [`Point3`].
#[derive(Clone, Debug)]
pub struct ComposePoint {
    key: RelationKey,
    x: Quantity<Length>,
    y: Quantity<Length>,
    z: Quantity<Length>,
    point: Quantity<Point3>,
}

impl ComposePoint {
    /// Creates a point-composition relation.
    pub fn new(
        key: &str,
        x: Quantity<Length>,
        y: Quantity<Length>,
        z: Quantity<Length>,
        point: Quantity<Point3>,
    ) -> Result<Self, BuildError> {
        Ok(Self {
            key: RelationKey::new(key)?,
            x,
            y,
            z,
            point,
        })
    }
}

impl relation_private::Sealed for ComposePoint {}

impl RelationSpec for ComposePoint {
    fn build(self) -> Result<RelationDef, BuildError> {
        Ok(RelationDef {
            key: self.key,
            participants: vec![
                self.x.erase(),
                self.y.erase(),
                self.z.erase(),
                self.point.erase(),
            ],
            methods: vec![
                MethodDef::new(
                    "components-to-point",
                    self.point.slot,
                    [self.x.slot, self.y.slot, self.z.slot],
                    Operation::ComposePoint,
                )?,
                MethodDef::new(
                    "point-to-x",
                    self.x.slot,
                    [self.point.slot],
                    Operation::PointComponent(0),
                )?,
                MethodDef::new(
                    "point-to-y",
                    self.y.slot,
                    [self.point.slot],
                    Operation::PointComponent(1),
                )?,
                MethodDef::new(
                    "point-to-z",
                    self.z.slot,
                    [self.point.slot],
                    Operation::PointComponent(2),
                )?,
            ],
        })
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum AddFamily {
    Length,
    Count,
}

impl AddFamily {
    const fn code(self) -> u8 {
        match self {
            Self::Length => 0,
            Self::Count => 1,
        }
    }

    const fn domain(self) -> DomainTag {
        match self {
            Self::Length => DomainTag::Length,
            Self::Count => DomainTag::Count,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Operation {
    Add(AddFamily),
    Subtract(AddFamily),
    ScaleLength { factor: Rational, round: Round },
    OffsetLength(Length),
    Identity(DomainTag),
    PitchRise { round: Round },
    PitchRun { round: Round },
    PitchRatio,
    Pythagorean { subtract: bool, round: Round },
    ComposePoint,
    PointComponent(u8),
}

impl Operation {
    fn output_domain(&self) -> DomainTag {
        match self {
            Self::Add(family) | Self::Subtract(family) => family.domain(),
            Self::ScaleLength { .. }
            | Self::OffsetLength(_)
            | Self::PitchRise { .. }
            | Self::PitchRun { .. }
            | Self::Pythagorean { .. }
            | Self::PointComponent(_) => DomainTag::Length,
            Self::Identity(domain) => *domain,
            Self::PitchRatio => DomainTag::Rational,
            Self::ComposePoint => DomainTag::Point3,
        }
    }

    pub(crate) fn solve(&self, inputs: &[Value]) -> Result<SolveResult, ArithmeticError> {
        match self {
            Self::Add(AddFamily::Length) => {
                let [Value::Length(left), Value::Length(right)] = inputs else {
                    unreachable!("plan compiler validated length addition domains")
                };
                Ok(SolveResult::exact(Value::Length(left.checked_add(*right)?)))
            }
            Self::Subtract(AddFamily::Length) => {
                let [Value::Length(total), Value::Length(known)] = inputs else {
                    unreachable!("plan compiler validated length subtraction domains")
                };
                Ok(SolveResult::exact(Value::Length(
                    total.checked_sub(*known)?,
                )))
            }
            Self::Add(AddFamily::Count) => {
                let [Value::Count(left), Value::Count(right)] = inputs else {
                    unreachable!("plan compiler validated count addition domains")
                };
                let value = left
                    .get()
                    .checked_add(right.get())
                    .ok_or(ArithmeticError::Overflow)?;
                Ok(SolveResult::exact(Value::Count(Count::new(value))))
            }
            Self::Subtract(AddFamily::Count) => {
                let [Value::Count(total), Value::Count(known)] = inputs else {
                    unreachable!("plan compiler validated count subtraction domains")
                };
                let value = total
                    .get()
                    .checked_sub(known.get())
                    .ok_or(ArithmeticError::NegativeRadicand)?;
                Ok(SolveResult::exact(Value::Count(Count::new(value))))
            }
            Self::ScaleLength { factor, round } => {
                let [Value::Length(input)] = inputs else {
                    unreachable!("plan compiler validated scale domains")
                };
                let (value, exactness) = input.checked_scale(*factor, *round)?;
                Ok(SolveResult {
                    value: Value::Length(value),
                    exactness,
                })
            }
            Self::OffsetLength(offset) => {
                let [Value::Length(input)] = inputs else {
                    unreachable!("plan compiler validated offset domains")
                };
                Ok(SolveResult::exact(Value::Length(
                    input.checked_add(*offset)?,
                )))
            }
            Self::Identity(_) => {
                let [input] = inputs else {
                    unreachable!("identity has one input")
                };
                Ok(SolveResult::exact(input.clone()))
            }
            Self::PitchRise { round } => {
                let [Value::Length(run), Value::Rational(ratio)] = inputs else {
                    unreachable!("plan compiler validated pitch domains")
                };
                let (rise, exactness) = run.checked_scale(*ratio, *round)?;
                Ok(SolveResult {
                    value: Value::Length(rise),
                    exactness,
                })
            }
            Self::PitchRun { round } => {
                let [Value::Length(rise), Value::Rational(ratio)] = inputs else {
                    unreachable!("plan compiler validated pitch domains")
                };
                let (run, exactness) = rise.checked_scale(ratio.checked_reciprocal()?, *round)?;
                Ok(SolveResult {
                    value: Value::Length(run),
                    exactness,
                })
            }
            Self::PitchRatio => {
                let [Value::Length(run), Value::Length(rise)] = inputs else {
                    unreachable!("plan compiler validated pitch domains")
                };
                if run.iota() == 0 {
                    return Err(ArithmeticError::DivisionByZero);
                }
                let denominator = run.iota().unsigned_abs().into();
                let numerator = if run.iota().is_negative() {
                    -i128::from(rise.iota())
                } else {
                    i128::from(rise.iota())
                };
                Ok(SolveResult::exact(Value::Rational(Rational::new(
                    numerator,
                    denominator,
                )?)))
            }
            Self::Pythagorean { subtract, round } => {
                let [Value::Length(first), Value::Length(second)] = inputs else {
                    unreachable!("plan compiler validated pythagorean domains")
                };
                let first = first.iota().unsigned_abs();
                let second = second.iota().unsigned_abs();
                let first_squared = u128::from(first)
                    .checked_mul(u128::from(first))
                    .ok_or(ArithmeticError::Overflow)?;
                let second_squared = u128::from(second)
                    .checked_mul(u128::from(second))
                    .ok_or(ArithmeticError::Overflow)?;
                let radicand = if *subtract {
                    first_squared
                        .checked_sub(second_squared)
                        .ok_or(ArithmeticError::NegativeRadicand)?
                } else {
                    first_squared
                        .checked_add(second_squared)
                        .ok_or(ArithmeticError::Overflow)?
                };
                let floor = radicand.isqrt();
                let remainder = radicand - floor * floor;
                let selected = if remainder == 0 {
                    floor
                } else {
                    match round {
                        Round::Exact => {
                            return Err(ArithmeticError::NonIntegral {
                                exact: Rational::new(
                                    i128::try_from(radicand)
                                        .map_err(|_| ArithmeticError::Overflow)?,
                                    1,
                                )?,
                            });
                        }
                        Round::Down => floor,
                        Round::Up => floor.checked_add(1).ok_or(ArithmeticError::Overflow)?,
                        Round::Nearest => {
                            // Compare the radicand with (floor + 1/2)^2 without
                            // floating point: 4r against 4f² + 4f + 1.
                            let four_r =
                                radicand.checked_mul(4).ok_or(ArithmeticError::Overflow)?;
                            let threshold = floor
                                .checked_mul(floor)
                                .and_then(|value| value.checked_mul(4))
                                .and_then(|value| value.checked_add(floor.checked_mul(4)?))
                                .and_then(|value| value.checked_add(1))
                                .ok_or(ArithmeticError::Overflow)?;
                            if four_r >= threshold {
                                floor.checked_add(1).ok_or(ArithmeticError::Overflow)?
                            } else {
                                floor
                            }
                        }
                    }
                };
                let selected_i64 =
                    i64::try_from(selected).map_err(|_| ArithmeticError::Overflow)?;
                let exactness = if remainder == 0 {
                    ExactnessTrace::Exact
                } else {
                    ExactnessTrace::RootQuantization(RootRounding {
                        radicand,
                        floor_root: floor,
                        remainder,
                        selected_root: selected,
                        policy: *round,
                    })
                };
                Ok(SolveResult {
                    value: Value::Length(Length::from_iota(selected_i64)),
                    exactness,
                })
            }
            Self::ComposePoint => {
                let [Value::Length(x), Value::Length(y), Value::Length(z)] = inputs else {
                    unreachable!("plan compiler validated point domains")
                };
                Ok(SolveResult::exact(Value::Point3(Point3::new(*x, *y, *z))))
            }
            Self::PointComponent(component) => {
                let [Value::Point3(point)] = inputs else {
                    unreachable!("plan compiler validated point domains")
                };
                let value = match component {
                    0 => point.x,
                    1 => point.y,
                    2 => point.z,
                    _ => unreachable!("point component is constructed internally"),
                };
                Ok(SolveResult::exact(Value::Length(value)))
            }
        }
    }

    pub(crate) fn encode(&self, encoder: &mut CanonicalEncoder) {
        match self {
            Self::Add(family) => {
                encoder.u8(0);
                encoder.u8(family.code());
            }
            Self::Subtract(family) => {
                encoder.u8(1);
                encoder.u8(family.code());
            }
            Self::ScaleLength { factor, round } => {
                encoder.u8(2);
                factor.encode(encoder);
                encoder.u8(round.code());
            }
            Self::OffsetLength(offset) => {
                encoder.u8(3);
                offset.encode(encoder);
            }
            Self::Identity(domain) => {
                encoder.u8(4);
                encoder.u8(domain.code());
            }
            Self::PitchRise { round } => {
                encoder.u8(5);
                encoder.u8(round.code());
            }
            Self::PitchRun { round } => {
                encoder.u8(6);
                encoder.u8(round.code());
            }
            Self::PitchRatio => encoder.u8(7),
            Self::Pythagorean { subtract, round } => {
                encoder.u8(8);
                encoder.bool(*subtract);
                encoder.u8(round.code());
            }
            Self::ComposePoint => encoder.u8(9),
            Self::PointComponent(component) => {
                encoder.u8(10);
                encoder.u8(*component);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SolveResult {
    pub(crate) value: Value,
    pub(crate) exactness: ExactnessTrace,
}

impl SolveResult {
    fn exact(value: Value) -> Self {
        Self {
            value,
            exactness: ExactnessTrace::Exact,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MethodDef {
    pub(crate) id: MethodId,
    pub(crate) target: QuantitySlot,
    pub(crate) inputs: Vec<QuantitySlot>,
    pub(crate) operation: Operation,
}

impl MethodDef {
    fn new(
        id: &str,
        target: QuantitySlot,
        inputs: impl IntoIterator<Item = QuantitySlot>,
        operation: Operation,
    ) -> Result<Self, BuildError> {
        Ok(Self {
            id: MethodId::new(id)?,
            target,
            inputs: inputs.into_iter().collect(),
            operation,
        })
    }
}

#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct RelationDef {
    pub(crate) key: RelationKey,
    pub(crate) participants: Vec<AnyQuantity>,
    pub(crate) methods: Vec<MethodDef>,
}

impl RelationDef {
    fn encode(&self, encoder: &mut CanonicalEncoder) {
        encoder.str(self.key.as_str());
        encoder.u32(u32::try_from(self.methods.len()).expect("relation methods are bounded"));
        for method in &self.methods {
            encoder.str(method.id.as_str());
            encoder.str(self.key_for_slot(method.target).as_str());
            encoder.u32(u32::try_from(method.inputs.len()).expect("method inputs are bounded"));
            for input in &method.inputs {
                encoder.str(self.key_for_slot(*input).as_str());
            }
            method.operation.encode(encoder);
        }
    }

    fn key_for_slot(&self, slot: QuantitySlot) -> &QuantityKey {
        self.participants
            .iter()
            .find(|quantity| quantity.slot() == slot)
            .expect("every method slot is a relation participant")
            .key()
    }
}
