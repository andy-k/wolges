#[derive(Clone, Copy)]
pub struct Dim {
    pub rows: i8,
    pub cols: i8,
}

impl Dim{
pub fn at_row_col(&self,row:i8,col :i8)->usize {(((row as isize)*(self.cols as isize))+(col as isize)) as usize}
}

#[derive(Clone, Copy)]
pub struct Across {
    dim: Dim,
    row: i8,
}

#[derive(Clone, Copy)]
pub struct Down {
    dim: Dim,
    col: i8,
}

pub trait Flippable {
    fn lanes(&self) -> i8;
    fn len(&self) -> i8;
    fn at(&self, col: i8) -> usize;
}

impl Flippable for Across {

    #[inline(always)]
    fn lanes(&self) -> i8 {
        self.dim.rows
    }

    #[inline(always)]
    fn len(&self) -> i8 {
        self.dim.cols
    }

    #[inline(always)]
    fn at(&self, col: i8) -> usize {
    self.dim.at_row_col(self.row, col)
    }
}

impl Flippable for Down {

    #[inline(always)]
    fn lanes(&self) -> i8 {
        self.dim.cols
    }

    #[inline(always)]
    fn len(&self) -> i8 {
        self.dim.rows
    }

    #[inline(always)]
    fn at(&self, row: i8) -> usize {
    self.dim.at_row_col(row, self.col)
    }
}
