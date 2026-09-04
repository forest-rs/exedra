// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Minimal helpers for deterministic joins over sorted key/count pairs.

/// Visits a merged key stream from two sorted `(key, count)` inputs.
///
/// For each distinct key, `visit` receives `(key, left_count, right_count)`.
pub(crate) fn for_each_count_join<K: Ord + Copy>(
    left: &[(K, usize)],
    right: &[(K, usize)],
    mut visit: impl FnMut(K, usize, usize),
) {
    let mut i = 0_usize;
    let mut j = 0_usize;
    while i < left.len() || j < right.len() {
        match (left.get(i), right.get(j)) {
            (Some((lk, lc)), Some((rk, rc))) if lk == rk => {
                i += 1;
                j += 1;
                visit(*lk, *lc, *rc);
            }
            (Some((lk, lc)), Some((rk, _))) if lk < rk => {
                i += 1;
                visit(*lk, *lc, 0);
            }
            (Some(_), Some((rk, rc))) => {
                j += 1;
                visit(*rk, 0, *rc);
            }
            (Some((lk, lc)), None) => {
                i += 1;
                visit(*lk, *lc, 0);
            }
            (None, Some((rk, rc))) => {
                j += 1;
                visit(*rk, 0, *rc);
            }
            (None, None) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::for_each_count_join;

    #[test]
    fn count_join_handles_equal_keys() {
        let left = vec![(1_u32, 2_usize), (3, 4)];
        let right = vec![(1_u32, 7_usize), (3, 9)];
        let mut merged = vec![];
        for_each_count_join(&left, &right, |key, l, r| merged.push((key, l, r)));
        assert_eq!(merged, vec![(1, 2, 7), (3, 4, 9)]);
    }

    #[test]
    fn count_join_handles_left_only_keys() {
        let left = vec![(1_u32, 2_usize), (4, 5)];
        let right = vec![(4_u32, 7_usize)];
        let mut merged = vec![];
        for_each_count_join(&left, &right, |key, l, r| merged.push((key, l, r)));
        assert_eq!(merged, vec![(1, 2, 0), (4, 5, 7)]);
    }

    #[test]
    fn count_join_handles_right_only_keys() {
        let left = vec![(2_u32, 3_usize)];
        let right = vec![(1_u32, 5_usize), (2, 7), (9, 11)];
        let mut merged = vec![];
        for_each_count_join(&left, &right, |key, l, r| merged.push((key, l, r)));
        assert_eq!(merged, vec![(1, 0, 5), (2, 3, 7), (9, 0, 11)]);
    }
}
