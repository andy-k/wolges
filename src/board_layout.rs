use super::matrix;

#[derive(Clone, Copy)]
struct Premium {
pub  word_multiplier: i8,
  pub letter_multiplier: i8,
}

static TWS : Premium = Premium { word_multiplier: 3, letter_multiplier: 1 };
static DWS : Premium = Premium { word_multiplier: 2, letter_multiplier: 1 };
static TLS : Premium = Premium { word_multiplier: 1, letter_multiplier: 3 };
static DLS : Premium = Premium { word_multiplier: 1, letter_multiplier: 2 };
static FVS : Premium = Premium { word_multiplier: 1, letter_multiplier: 1 };

pub trait BoardLayout<'a> {
    fn dim(&self) -> matrix::Dim;
    fn star_row(&self) -> i8;
    fn star_col(&self) -> i8;
    fn premium_at(&self,row:i8,col:i8) -> Premium;
}

impl<'a, T: BoardLayout<'a>> BoardLayout<'a> for &T {
    fn dim(&self) -> matrix::Dim {(*self).dim() }
    fn star_row(&self) -> i8{(*self).star_row() }
    fn star_col(&self) -> i8{(*self).star_col() }
    fn premium_at(&self,row:i8,col:i8) -> Premium {(*self).premium_at(row,col)}
}

pub struct GenericBoardLayout<'a> {
  premiums: &'a[Premium],
  dim: matrix::Dim,
  star_row: i8,
  star_col: i8,
}

impl<'a> BoardLayout<'a> for GenericBoardLayout<'a> {

    #[inline(always)]
    fn dim(&self) -> matrix::Dim { self.dim }

    #[inline(always)]
    fn star_row(&self) -> i8{ self.star_row }

    #[inline(always)]
    fn star_col(&self) -> i8{ self.star_col }

    #[inline(always)]
    fn premium_at(&self, row:i8,col:i8) -> Premium {
        self.premiums[self.dim().at_row_col(row, col)]
    }
}

pub static COMMON_BOARD_LAYOUT: GenericBoardLayout = GenericBoardLayout{
premiums:&[
TWS, FVS, FVS, DLS, FVS, FVS, FVS, TWS, FVS, FVS, FVS, DLS, FVS, FVS, TWS, //
FVS, DWS, FVS, FVS, FVS, TLS, FVS, FVS, FVS, TLS, FVS, FVS, FVS, DWS, FVS, //
FVS, FVS, DWS, FVS, FVS, FVS, DLS, FVS, DLS, FVS, FVS, FVS, DWS, FVS, FVS, //
DLS, FVS, FVS, DWS, FVS, FVS, FVS, DLS, FVS, FVS, FVS, DWS, FVS, FVS, DLS, //
FVS, FVS, FVS, FVS, DWS, FVS, FVS, FVS, FVS, FVS, DWS, FVS, FVS, FVS, FVS, //
FVS, TLS, FVS, FVS, FVS, TLS, FVS, FVS, FVS, TLS, FVS, FVS, FVS, TLS, FVS, //
FVS, FVS, DLS, FVS, FVS, FVS, DLS, FVS, DLS, FVS, FVS, FVS, DLS, FVS, FVS, //
TWS, FVS, FVS, DLS, FVS, FVS, FVS, DWS, FVS, FVS, FVS, DLS, FVS, FVS, TWS, //
FVS, FVS, DLS, FVS, FVS, FVS, DLS, FVS, DLS, FVS, FVS, FVS, DLS, FVS, FVS, //
FVS, TLS, FVS, FVS, FVS, TLS, FVS, FVS, FVS, TLS, FVS, FVS, FVS, TLS, FVS, //
FVS, FVS, FVS, FVS, DWS, FVS, FVS, FVS, FVS, FVS, DWS, FVS, FVS, FVS, FVS, //
DLS, FVS, FVS, DWS, FVS, FVS, FVS, DLS, FVS, FVS, FVS, DWS, FVS, FVS, DLS, //
FVS, FVS, DWS, FVS, FVS, FVS, DLS, FVS, DLS, FVS, FVS, FVS, DWS, FVS, FVS, //
FVS, DWS, FVS, FVS, FVS, TLS, FVS, FVS, FVS, TLS, FVS, FVS, FVS, DWS, FVS, //
TWS, FVS, FVS, DLS, FVS, FVS, FVS, TWS, FVS, FVS, FVS, DLS, FVS, FVS, TWS, //
], dim:matrix::Dim { rows: 15, cols: 15 },star_row: 7,star_col: 7};


/*
impl<'a> std::ops::Deref for GenericBoardLayout<'a> {
    type Target = &'a dyn BoardLayout<'a>;

    fn deref(&self) -> &Self::Target {
        &(self as Self::Target)
    }
}
*/

