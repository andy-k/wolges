#[derive(Clone, Copy)]
pub struct Strider {
    pub base: i8,
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
            base: row * self.cols,
            step: 1,
            len: self.cols,
        }
    }

    #[inline(always)]
    pub fn down(&self, col: i8) -> Strider {
        Strider {
            base: col,
            step: self.cols,
            len: self.rows,
        }
    }

    #[inline(always)]
    pub fn at_row_col(&self, row: i8, col: i8) -> usize {
        (((row as isize) * (self.cols as isize)) + (col as isize)) as usize
    }

    #[inline(always)]
    pub fn transposable(&self) -> TransposableDim {
      TransposableDim{ pris:self.rows,secs:self.cols,transposed:false}
    }
}

#[derive(Clone, Copy)]
pub struct TransposableDim {
  pub pris:i8,
  pub secs:i8,
  pub transposed:bool,
}

impl TransposableDim {

    #[inline(always)]
    pub fn dim(&self) -> Dim {
    match self.transposed {
    false => Dim{rows:self.pris,cols:self.secs},
    true => Dim{cols:self.pris,rows:self.secs},
        }
    }

    #[inline(always)]
    pub fn across(&self, pri: i8) -> Strider {
    match self.transposed {
    false => self.dim().across(pri),
    true => self.dim().down(pri),
        }
    }

    #[inline(always)]
    pub fn down(&self, pri: i8) -> Strider {
    match self.transposed {
    false => self.dim().down(pri),
    true => self.dim().across(pri),
        }
    }

    #[inline(always)]
    pub fn at_pri_sec(&self, pri: i8, sec: i8) -> usize {
    match self.transposed {
    false => self.dim().at_row_col(pri, sec),
    true => self.dim().at_row_col(sec, pri),
        }
    }

    #[inline(always)]
    pub fn transpose(&self) -> TransposableDim {
      TransposableDim{ pris:self.secs,secs:self.pris,transposed:!self.transposed}
    }
}
