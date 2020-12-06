use super::matrix;

#[derive(Clone, Copy)]
pub struct Premium {
pub  word_multiplier: i8,
  pub letter_multiplier: i8,
}

static TWS : Premium = Premium { word_multiplier: 3, letter_multiplier: 1 };
static DWS : Premium = Premium { word_multiplier: 2, letter_multiplier: 1 };
static TLS : Premium = Premium { word_multiplier: 1, letter_multiplier: 3 };
static DLS : Premium = Premium { word_multiplier: 1, letter_multiplier: 2 };
static FVS : Premium = Premium { word_multiplier: 1, letter_multiplier: 1 };

pub trait TraitBoardLayout<'a> {
    fn dim(&self) -> matrix::Dim;
    fn star_row(&self) -> i8;
    fn star_col(&self) -> i8;
    fn premium_at(&self,row:i8,col:i8) -> Premium;
}

/*
impl<'a, T: TraitBoardLayout<'a>> TraitBoardLayout<'a> for &T {
    fn dim(&self) -> matrix::Dim {(*self).dim() }
    fn star_row(&self) -> i8{(*self).star_row() }
    fn star_col(&self) -> i8{(*self).star_col() }
    fn premium_at(&self,row:i8,col:i8) -> Premium {(*self).premium_at(row,col)}
}
*/

pub struct StaticBoardLayout<'a> {
  premiums: &'a[Premium],
  dim: matrix::Dim,
  star_row: i8,
  star_col: i8,
}

impl<'a> TraitBoardLayout<'a> for StaticBoardLayout<'a> {

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

pub enum BoardLayout<'a> {
  Static(StaticBoardLayout<'a>),
}

impl<'a> TraitBoardLayout<'a> for BoardLayout<'a> {

    #[inline(always)]
    fn dim(&self) -> matrix::Dim { match self { BoardLayout::Static(x) => x.dim() } }

    #[inline(always)]
    fn star_row(&self) -> i8{ match self { BoardLayout::Static(x) => x.star_row() } }

    #[inline(always)]
    fn star_col(&self) -> i8{ match self { BoardLayout::Static(x) => x.star_col() } }

    #[inline(always)]
    fn premium_at(&self, row:i8,col:i8) -> Premium {
        match self { BoardLayout::Static(x) => x.premium_at(row,col) }
    }
}

pub static COMMON_BOARD_LAYOUT: BoardLayout = BoardLayout :: Static(StaticBoardLayout{
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
], dim:matrix::Dim { rows: 15, cols: 15 },star_row: 7,star_col: 7});
