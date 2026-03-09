use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Axis {
    X,
    Y,
}

impl Axis {
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::X => Self::Y,
            Self::Y => Self::X,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Slot {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    #[must_use]
    pub fn left(self) -> i32 {
        self.x
    }

    #[must_use]
    pub fn right(self) -> i32 {
        self.x + i32::try_from(self.w).expect("rect width exceeds i32")
    }

    #[must_use]
    pub fn top(self) -> i32 {
        self.y
    }

    #[must_use]
    pub fn bottom(self) -> i32 {
        self.y + i32::try_from(self.h).expect("rect height exceeds i32")
    }

    #[must_use]
    pub fn extent(self, axis: Axis) -> u32 {
        match axis {
            Axis::X => self.w,
            Axis::Y => self.h,
        }
    }

    #[must_use]
    pub fn split(self, axis: Axis, lead_extent: u32) -> (Self, Self) {
        match axis {
            Axis::X => {
                let right_x = self.x + i32::try_from(lead_extent).expect("lead extent exceeds i32");
                (
                    Self {
                        x: self.x,
                        y: self.y,
                        w: lead_extent,
                        h: self.h,
                    },
                    Self {
                        x: right_x,
                        y: self.y,
                        w: self.w.saturating_sub(lead_extent),
                        h: self.h,
                    },
                )
            }
            Axis::Y => {
                let bottom_y =
                    self.y + i32::try_from(lead_extent).expect("lead extent exceeds i32");
                (
                    Self {
                        x: self.x,
                        y: self.y,
                        w: self.w,
                        h: lead_extent,
                    },
                    Self {
                        x: self.x,
                        y: bottom_y,
                        w: self.w,
                        h: self.h.saturating_sub(lead_extent),
                    },
                )
            }
        }
    }

    #[must_use]
    pub fn mirrored(self, axis: Axis, root: Self) -> Self {
        match axis {
            Axis::X => Self {
                x: root.right()
                    - i32::try_from(self.w).expect("rect width exceeds i32")
                    - (self.x - root.left()),
                y: self.y,
                w: self.w,
                h: self.h,
            },
            Axis::Y => Self {
                x: self.x,
                y: root.bottom()
                    - i32::try_from(self.h).expect("rect height exceeds i32")
                    - (self.y - root.top()),
                w: self.w,
                h: self.h,
            },
        }
    }

    #[must_use]
    pub fn center_twice_orth(self, axis: Axis) -> i64 {
        match axis {
            Axis::X => i64::from(self.top()) + i64::from(self.bottom()),
            Axis::Y => i64::from(self.left()) + i64::from(self.right()),
        }
    }
}

#[must_use]
pub fn orth_gap(a_start: i32, a_end: i32, b_start: i32, b_end: i32) -> u32 {
    if a_end <= b_start {
        u32::try_from(b_start - a_end).expect("gap negative")
    } else if b_end <= a_start {
        u32::try_from(a_start - b_end).expect("gap negative")
    } else {
        0
    }
}
