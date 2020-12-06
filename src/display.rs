use super::board_layout;
use board_layout::TraitBoardLayout;

#[inline(always)]
pub fn empty_label<'a>(
    board_layout: &'a board_layout::BoardLayout<'a>,
    row: i8,
    col: i8,
) -> &'a str {
    if row == board_layout.star_row() && col == board_layout.star_col() {
        return "*";
    }
    let premium = board_layout.premium_at(row, col);
    match premium.word_multiplier {
        3 => "=",
        2 => "-",
        1 => match premium.letter_multiplier {
            3 => "\"",
            2 => "\'",
            1 => " ",
            _ => panic!(),
        },
        _ => panic!(),
    }
}
