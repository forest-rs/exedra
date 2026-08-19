// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Seeded deterministic PRNG for oracle case generation.

/// `SplitMix64`: tiny, high-quality, and fully deterministic across platforms.
#[derive(Clone, Debug)]
pub(crate) struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Creates a generator from an explicit seed.
    #[must_use]
    pub(crate) const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next raw 64-bit value.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform f64 in `[0, 1)` built from the top 53 bits.
    pub(crate) fn next_f64(&mut self) -> f64 {
        #[expect(clippy::cast_precision_loss, reason = "53 bits convert to f64 exactly")]
        let mantissa = (self.next_u64() >> 11) as f64;
        mantissa / (1_u64 << 53) as f64
    }

    /// Uniform f64 in `[lo, hi)`.
    pub(crate) fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    /// Uniform index in `0..n` (n must be nonzero).
    pub(crate) fn index(&mut self, n: usize) -> usize {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "case counts are far below u64 range"
        )]
        {
            (self.next_u64() % n as u64) as usize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SplitMix64;

    #[test]
    fn splitmix_is_deterministic_and_known() {
        let mut rng = SplitMix64::new(1234);
        let a = rng.next_u64();
        let mut rng2 = SplitMix64::new(1234);
        assert_eq!(a, rng2.next_u64());
        // Distinct seeds diverge immediately.
        assert_ne!(a, SplitMix64::new(1235).next_u64());
    }

    #[test]
    fn unit_floats_stay_in_range() {
        let mut rng = SplitMix64::new(7);
        for _ in 0..1000 {
            let v = rng.next_f64();
            assert!((0.0..1.0).contains(&v));
        }
    }
}
