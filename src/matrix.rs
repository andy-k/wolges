// Copyright (C) 2020-2026 Andy Kurnia.

#[derive(Clone)]
pub struct Strider {
    base: i16,
    step: i8,
    len: i8,
}

impl Strider {
    #[inline(always)]
    pub fn base(&self) -> i16 {
        self.base
    }

    #[inline(always)]
    pub fn step(&self) -> i8 {
        self.step
    }

    #[inline(always)]
    pub fn len(&self) -> i8 {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    pub fn at(&self, idx: i8) -> usize {
        ((self.base as isize) + (idx as isize) * (self.step as isize)) as usize
    }
}

#[derive(Clone, Default)]
pub struct Dim {
    pub rows: i8,
    pub cols: i8,
}

impl Dim {
    #[inline(always)]
    pub fn across(&self, row: i8) -> Strider {
        Strider {
            base: (row as i16) * (self.cols as i16),
            step: 1,
            len: self.cols,
        }
    }

    #[inline(always)]
    pub fn down(&self, col: i8) -> Strider {
        Strider {
            base: col as i16,
            step: self.cols,
            len: self.rows,
        }
    }

    #[inline(always)]
    pub fn lane(&self, down: bool, lane: i8) -> Strider {
        if down {
            self.down(lane)
        } else {
            self.across(lane)
        }
    }

    #[inline(always)]
    pub fn at_row_col(&self, row: i8, col: i8) -> usize {
        (((row as isize) * (self.cols as isize)) + (col as isize)) as usize
    }
}
