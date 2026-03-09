use serde::{Deserialize, Serialize};

use crate::geom::Axis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightPair {
    pub a: u32,
    pub b: u32,
}

impl WeightPair {
    #[must_use]
    pub(crate) fn checked(self) -> Option<Self> {
        (self.a != 0 || self.b != 0).then_some(self)
    }
}

impl Default for WeightPair {
    fn default() -> Self {
        Self { a: 1, b: 1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Priority {
    pub shrink: u16,
    pub grow: u16,
}

impl Default for Priority {
    fn default() -> Self {
        Self { shrink: 1, grow: 1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeLimits {
    pub min_w: u32,
    pub min_h: u32,
    pub max_w: Option<u32>,
    pub max_h: Option<u32>,
}

impl Default for SizeLimits {
    fn default() -> Self {
        Self {
            min_w: 1,
            min_h: 1,
            max_w: None,
            max_h: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LeafMeta {
    pub limits: SizeLimits,
    pub priority: Priority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub min_w: u32,
    pub min_h: u32,
    pub max_w: Option<u32>,
    pub max_h: Option<u32>,
    pub leaf_count: u32,
    pub shrink_cost: u64,
    pub grow_cost: u64,
}

impl Summary {
    #[must_use]
    pub fn axis_limits(self, axis: Axis) -> (u32, Option<u32>) {
        match axis {
            Axis::X => (self.min_w, self.max_w),
            Axis::Y => (self.min_h, self.max_h),
        }
    }
}

#[must_use]
pub fn canonicalize_weights(a: u32, b: u32) -> WeightPair {
    match (a, b) {
        (0, 0) => panic!("invalid zero weight pair"),
        (0, _) => WeightPair { a: 0, b: 1 },
        (_, 0) => WeightPair { a: 1, b: 0 },
        _ => {
            let gcd = gcd(a, b);
            WeightPair {
                a: a / gcd,
                b: b / gcd,
            }
        }
    }
}

const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a
}
