use super::matrix;

#[derive(Clone, Copy)]
pub struct Premium {
    pub word_multiplier: i8,
    pub tile_multiplier: i8,
}

static TWS: Premium = Premium {
    word_multiplier: 3,
    tile_multiplier: 1,
};
static DWS: Premium = Premium {
    word_multiplier: 2,
    tile_multiplier: 1,
};
static TLS: Premium = Premium {
    word_multiplier: 1,
    tile_multiplier: 3,
};
static DLS: Premium = Premium {
    word_multiplier: 1,
    tile_multiplier: 2,
};
static FVS: Premium = Premium {
    word_multiplier: 1,
    tile_multiplier: 1,
};

pub struct StaticBoardLayout<'a> {
    premiums: &'a [Premium],
    dim: matrix::Dim,
    star_row: i8,
    star_col: i8,
    is_symmetric: bool,
}

pub enum BoardLayout<'a> {
    Static(StaticBoardLayout<'a>),
}

impl<'a> BoardLayout<'a> {
    #[inline(always)]
    pub fn dim(&self) -> matrix::Dim {
        match self {
            BoardLayout::Static(x) => x.dim,
        }
    }

    #[inline(always)]
    pub fn star_row(&self) -> i8 {
        match self {
            BoardLayout::Static(x) => x.star_row,
        }
    }

    #[inline(always)]
    pub fn star_col(&self) -> i8 {
        match self {
            BoardLayout::Static(x) => x.star_col,
        }
    }

    #[inline(always)]
    pub fn premiums(&self) -> &[Premium] {
        match self {
            BoardLayout::Static(x) => &x.premiums,
        }
    }

    // This should return false if any of these is true:
    // - dim.rows != dim.cols
    // - exists (r,c) premium at (r,c) != premium at (c,r)
    // - star_row != star_col
    #[inline(always)]
    pub fn is_symmetric(&self) -> bool {
        match self {
            BoardLayout::Static(x) => x.is_symmetric,
        }
    }
}

pub static COMMON_BOARD_LAYOUT: BoardLayout = BoardLayout::Static(StaticBoardLayout {
    premiums: &[
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
    ],
    dim: matrix::Dim { rows: 5, cols: 15 },
    star_row: 7,
    star_col: 7,
    is_symmetric: true,
});
