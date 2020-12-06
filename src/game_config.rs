use super::{alphabet,board_layout};

pub trait GameConfig<'a> {
fn alphabet(&self)->&'a dyn alphabet::Alphabet<'a>;
fn board_layout(&self)->&'a dyn board_layout::BoardLayout<'a>;
}

pub struct GenericGameConfig<'a> {
  alphabet : &'a (dyn alphabet::Alphabet<'a>+Sync),
  board_layout :&'a (dyn board_layout::BoardLayout<'a>+Sync),
}

impl<'a> GameConfig<'a> for GenericGameConfig<'a> {
fn alphabet(&self)->&'a dyn alphabet::Alphabet<'a>{self.alphabet}
fn board_layout(&self)->&'a dyn board_layout::BoardLayout<'a>{self.board_layout}
}

pub static COMMON_ENGLISH_GAME_CONFIG : GenericGameConfig = GenericGameConfig{
  alphabet:  &alphabet::ENGLISH_ALPHABET,
  board_layout:  &board_layout::COMMON_BOARD_LAYOUT,
};
