// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Aligned angular rows, stitched by exact integer parameter ordering.

#[derive(Copy, Clone)]
pub(super) struct Corner {
    pub(super) upper: bool,
    pub(super) index: u32,
}
fn lower(index: u32) -> Corner {
    Corner {
        upper: false,
        index,
    }
}
fn upper(index: u32) -> Corner {
    Corner { upper: true, index }
}

/// Zero segments denotes one collapsed pole position; ordinary rows retain
/// their closing endpoint. Index products widen before addition/multiplication,
/// including on wasm32. No per-band triangle storage is owned here.
pub(super) struct Band {
    a: u32,
    b: u32,
    i: u32,
    j: u32,
    second: bool,
}
impl Band {
    pub(super) const fn new(a: u32, b: u32) -> Self {
        Self {
            a,
            b,
            i: 0,
            j: 0,
            second: false,
        }
    }
}
impl Iterator for Band {
    type Item = [Corner; 3];
    fn next(&mut self) -> Option<Self::Item> {
        if self.a == 0 {
            if self.j == self.b {
                return None;
            }
            let j = self.j;
            self.j += 1;
            return Some([lower(0), upper(j + 1), upper(j)]);
        }
        if self.b == 0 {
            if self.i == self.a {
                return None;
            }
            let i = self.i;
            self.i += 1;
            return Some([lower(i), lower(i + 1), upper(0)]);
        }
        if self.i == self.a && self.j == self.b {
            return None;
        }
        let (i, j) = (self.i, self.j);
        if self.second {
            self.second = false;
            self.i += 1;
            self.j += 1;
            return Some([lower(i), upper(j + 1), upper(j)]);
        }
        let a = (u64::from(i) + 1) * u64::from(self.b);
        let b = (u64::from(j) + 1) * u64::from(self.a);
        if a == b {
            self.second = true;
            Some([lower(i), lower(i + 1), upper(j + 1)])
        } else if a < b {
            self.i += 1;
            Some([lower(i), lower(i + 1), upper(j)])
        } else {
            self.j += 1;
            Some([lower(i), upper(j + 1), upper(j)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zipper_parameter_products_are_widened_before_wasm32_arithmetic() {
        // Each row and their sum fit a modest vertex budget, while 65536²
        // does not fit usize on wasm32. Iteration must retain both endpoints.
        let mut count = 0;
        let mut last = None;
        for triangle in Band::new(65536, 65536) {
            assert!(triangle.iter().all(|c| c.index <= 65536));
            last = Some(triangle);
            count += 1;
        }
        assert_eq!(count, 131072);
        let last = last.unwrap();
        assert!(last.iter().any(|c| c.upper && c.index == 65536));
        assert!(last.iter().any(|c| !c.upper && c.index == 65535));
    }
}
