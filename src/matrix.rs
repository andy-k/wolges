#[derive(Clone, Copy)]
pub struct Strider {
    pub base: i16,
    pub step: i8,
    pub len: i8,
}

impl Strider {
    #[inline(always)]
    pub fn len(&self) -> i8 {
        self.len
    }

    #[inline(always)]
    pub fn at(&self, idx: i8) -> usize {
        ((self.base as isize) + (idx as isize) * (self.step as isize)) as usize
    }
}

#[derive(Clone, Copy)]
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
    pub fn at_row_col(&self, row: i8, col: i8) -> usize {
        (((row as isize) * (self.cols as isize)) + (col as isize)) as usize
    }
}
