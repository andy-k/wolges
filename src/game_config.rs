use super::{alphabet, board_layout};

pub struct StaticGameConfig<'a> {
    alphabet: &'a alphabet::Alphabet<'a>,
    board_layout: &'a board_layout::BoardLayout<'a>,
}

pub enum GameConfig<'a> {
    Static(StaticGameConfig<'a>),
}

impl<'a> GameConfig<'a> {
    #[inline(always)]
    pub fn alphabet(&self) -> &'a alphabet::Alphabet<'a> {
        match self {
            GameConfig::Static(x) => x.alphabet,
        }
    }

    #[inline(always)]
    pub fn board_layout(&self) -> &'a board_layout::BoardLayout<'a> {
        match self {
            GameConfig::Static(x) => x.board_layout,
        }
    }
}

pub static COMMON_ENGLISH_GAME_CONFIG: GameConfig = GameConfig::Static(StaticGameConfig {
    alphabet: &alphabet::ENGLISH_ALPHABET,
    board_layout: &board_layout::COMMON_BOARD_LAYOUT,
});
