use super::{alphabet, board_layout};

#[inline(always)]
pub fn empty_label<'a>(
    board_layout: &'a board_layout::BoardLayout<'a>,
    row: i8,
    col: i8,
) -> &'a str {
    if row == board_layout.star_row() && col == board_layout.star_col() {
        return "*";
    }
    let premium = board_layout.premiums()[board_layout.dim().at_row_col(row, col)];
    match premium.word_multiplier {
        3 => "=",
        2 => "-",
        1 => match premium.tile_multiplier {
            3 => "\"",
            2 => "\'",
            1 => " ",
            _ => panic!(),
        },
        _ => panic!(),
    }
}

#[inline(always)]
pub fn board_label<'a>(
    alphabet: &'a alphabet::Alphabet<'a>,
    board_layout: &'a board_layout::BoardLayout<'a>,
    board_tiles: &'a [u8],
    row: i8,
    col: i8,
) -> &'a str {
    alphabet
        .from_board(board_tiles[board_layout.dim().at_row_col(row, col)])
        .unwrap_or_else(|| empty_label(board_layout, row, col))
}
