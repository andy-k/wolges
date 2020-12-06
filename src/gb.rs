mod matrix;

#[derive(Clone, Copy)]
struct RowCol(i8, i8);

struct Premium {
  word_multiplier: i8,
  letter_multiplier: i8,
}

static TWS : Premium = Premium { word_multiplier: 3, letter_multiplier: 1 };
static DWS : Premium = Premium { word_multiplier: 2, letter_multiplier: 1 };
static TLS : Premium = Premium { word_multiplier: 1, letter_multiplier: 3 };
static DLS : Premium = Premium { word_multiplier: 1, letter_multiplier: 2 };
static FVS : Premium = Premium { word_multiplier: 1, letter_multiplier: 1 };

pub trait BoardLayout<'a> {
    fn dim(&self) -> matrix::Dim;
    fn at(&self, _: RowCol) -> Premium;
}

pub struct GenericBoardLayout<'a>(&'a [Premium], matrix::Dim);

impl<'a> BoardLayout<'a> for GenericBoardLayout<'a> {

    #[inline(always)]
    fn dim(&self) -> matrix::Dim { self.1 }

    #[inline(always)]
    fn at(&self, RowCol(row, col): RowCol) -> Premium {
        self.0[((row as isize) * (self.dim().cols as isize) + (col as isize)) as usize]
    }
}

pub static COMMON_BOARD_LAYOUT: GenericBoardLayout = GenericBoardLayout(&[
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
], matrix::Dim { rows: 15, cols: 15 });
