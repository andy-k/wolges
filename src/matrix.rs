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
    pub fn at(&self, idx: i8) -> crate::error::Returns<usize> {
        Ok(usize::try_from(self.linear(idx))?)
    }

    #[inline(always)]
    fn linear(&self, idx: i8) -> i32 {
        i32::from(self.base) + i32::from(idx) * i32::from(self.step)
    }

    #[inline(always)]
    pub(crate) fn at_in_lane(&self, idx: i8) -> usize {
        self.linear(idx).cast_unsigned() as usize
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
            base: i16::from(row) * i16::from(self.cols),
            step: 1,
            len: self.cols,
        }
    }

    #[inline(always)]
    pub fn down(&self, col: i8) -> Strider {
        Strider {
            base: i16::from(col),
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
    pub fn at_row_col(&self, row: i8, col: i8) -> crate::error::Returns<usize> {
        Ok(usize::try_from(self.linear(row, col))?)
    }

    #[inline(always)]
    fn linear(&self, row: i8, col: i8) -> i32 {
        i32::from(row) * i32::from(self.cols) + i32::from(col)
    }

    #[inline(always)]
    pub(crate) fn at_on_board(&self, row: i8, col: i8) -> usize {
        self.linear(row, col).cast_unsigned() as usize
    }

    #[inline(always)]
    pub(crate) fn check_span(
        &self,
        down: bool,
        lane: i8,
        idx: i8,
        len: usize,
    ) -> crate::error::Returns<()> {
        let (lanes, squares) = if down {
            (self.cols, self.rows)
        } else {
            (self.rows, self.cols)
        };
        let start = usize::try_from(idx)?;
        let end = start
            .checked_add(len)
            .ok_or("play length exceeds the lane")?;
        if !(0..lanes).contains(&lane) || end > usize::try_from(squares)? {
            crate::return_error!("play extends outside the board".to_string());
        }
        Ok(())
    }
}
