// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Versioned canonical fingerprints used by setout's durable identities.

use alloc::vec::Vec;
use core::fmt;

/// Canonical-encoding version for all fingerprints emitted by this release.
///
/// This is a public compatibility boundary. Any change to field order, integer
/// encoding, or the hash construction must increment this value and provide a
/// migration note for persisted keys.
pub const FINGERPRINT_SCHEMA_VERSION: u32 = 2;

/// A deterministic fixed-width content fingerprint.
///
/// Setout needs fixed-size structural identity but does not need a security
/// primitive. The implementation uses two independently seeded FNV-1a 128-bit
/// lanes over a versioned canonical byte stream. This keeps the foundational
/// crate dependency-free beyond its units substrate and makes the exact protocol
/// reviewable here. Hash collisions are still treated as internal errors wherever
/// an arena also retains the canonical preimage.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Returns the canonical bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Constructs a fingerprint from canonical bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Fingerprint({self})")
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Builder for setout's canonical byte encoding.
///
/// Every variable-width value is length-prefixed and every enum uses an
/// explicitly assigned tag. Callers must never encode Rust discriminants,
/// allocation order, or debug strings.
#[derive(Clone, Debug)]
pub struct CanonicalEncoder {
    bytes: Vec<u8>,
}

impl CanonicalEncoder {
    /// Starts an encoding in `domain`.
    #[must_use]
    pub fn new(domain: &str) -> Self {
        let mut encoder = Self { bytes: Vec::new() };
        encoder.u32(FINGERPRINT_SCHEMA_VERSION);
        encoder.str(domain);
        encoder
    }

    /// Appends one byte.
    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// Appends a boolean using the stable tags `0` and `1`.
    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    /// Appends a little-endian `u32`.
    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Appends a little-endian `u64`.
    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Appends a little-endian `u128`.
    pub fn u128(&mut self, value: u128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Appends a little-endian `i64`.
    pub fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Appends a little-endian `i128`.
    pub fn i128(&mut self, value: i128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Appends length-prefixed bytes.
    pub fn bytes(&mut self, value: &[u8]) {
        let length = u32::try_from(value.len()).expect("canonical fields are capped at u32::MAX");
        self.u32(length);
        self.bytes.extend_from_slice(value);
    }

    /// Appends a length-prefixed UTF-8 string.
    pub fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    /// Appends an already-computed fixed-width fingerprint.
    pub fn fingerprint(&mut self, value: Fingerprint) {
        self.bytes.extend_from_slice(value.as_bytes());
    }

    /// Finishes the deterministic 256-bit fingerprint.
    #[must_use]
    pub fn finish(self) -> Fingerprint {
        const FNV128_OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
        const FNV128_PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
        // The second lane is deliberately unrelated to the standard offset.
        // Prefixing its input would allocate another buffer; a distinct seed is
        // equivalent for this non-cryptographic, fixed-domain construction.
        const SECOND_OFFSET: u128 = 0xd6e8_feb8_6659_fd93_a5a3_56d9_5a5a_5a5a;

        fn lane(bytes: &[u8], mut hash: u128) -> u128 {
            for byte in bytes {
                hash ^= u128::from(*byte);
                hash = hash.wrapping_mul(FNV128_PRIME);
            }
            hash
        }

        let first = lane(&self.bytes, FNV128_OFFSET);
        let second = lane(&self.bytes, SECOND_OFFSET);
        let mut bytes = [0; 32];
        bytes[..16].copy_from_slice(&first.to_le_bytes());
        bytes[16..].copy_from_slice(&second.to_le_bytes());
        Fingerprint(bytes)
    }
}
