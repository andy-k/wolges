// Copyright (C) 2020-2026 Andy Kurnia.

use super::{bag, equity, error, game_config, movegen};
use rand::prelude::*;

// each tile taken off the rack as swap_remove would, but moved past the rack's
// kept part rather than dropped, so a tile the rack lacks leaves it holding
// every tile it held (their order changed), and only a whole success shortens
// it
#[inline(always)]
pub fn use_tiles<II: IntoIterator<Item = u8>>(
    rack: &mut Vec<u8>,
    tiles_iter: II,
) -> error::Returns<()> {
    let mut kept = rack.len();
    for tile in tiles_iter {
        let pos = rack[..kept]
            .iter()
            .rposition(|&t| t == tile)
            .ok_or("bad tile")?;
        kept -= 1;
        rack.swap(pos, kept);
    }
    rack.truncate(kept);
    Ok(())
}

pub struct GamePlayer {
    pub score: i32,
    pub rack: Vec<u8>,
    pub num_exchanges: i16, // lifetime
}

impl Clone for GamePlayer {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            score: self.score,
            rack: self.rack.clone(),
            num_exchanges: self.num_exchanges,
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.score.clone_from(&source.score);
        self.rack.clone_from(&source.rack);
        self.num_exchanges.clone_from(&source.num_exchanges);
    }
}

pub struct GameState {
    pub players: Box<[GamePlayer]>,
    pub board_tiles: Box<[u8]>,
    pub bag: bag::Bag,
    pub turn: u8,
    pub zero_turns: u16,
    pub pass_turns: u16,
}

impl Clone for GameState {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            players: self.players.clone(),
            board_tiles: self.board_tiles.clone(),
            bag: self.bag.clone(),
            turn: self.turn,
            zero_turns: self.zero_turns,
            pass_turns: self.pass_turns,
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.players.clone_from(&source.players);
        self.board_tiles.clone_from(&source.board_tiles);
        self.bag.clone_from(&source.bag);
        self.turn.clone_from(&source.turn);
        self.zero_turns.clone_from(&source.zero_turns);
        self.pass_turns.clone_from(&source.pass_turns);
    }
}

impl GameState {
    // The other methods must be called with the same game_config.
    pub fn new(game_config: &game_config::GameConfig) -> Self {
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let rack_size = game_config.rack_size() as usize;
        let alphabet = game_config.alphabet();
        Self {
            players: (0..game_config.num_players())
                .map(|_| GamePlayer {
                    score: 0,
                    rack: Vec::with_capacity(rack_size),
                    num_exchanges: 0,
                })
                .collect(),
            board_tiles: vec![0u8; (dim.rows as usize) * (dim.cols as usize)].into_boxed_slice(),
            bag: bag::Bag::new(alphabet),
            turn: 0,
            zero_turns: 0,
            pass_turns: 0,
        }
    }

    pub fn reset(&mut self) {
        for player in self.players.iter_mut() {
            player.score = 0;
            player.rack.clear();
            player.num_exchanges = 0;
        }
        self.board_tiles.iter_mut().for_each(|m| *m = 0);
        self.bag.reset();
        self.turn = 0;
        self.zero_turns = 0;
        self.pass_turns = 0;
    }

    #[inline(always)]
    pub fn reset_and_draw_tiles<R: Rng + ?Sized>(
        &mut self,
        game_config: &game_config::GameConfig,
        rng: &mut R,
    ) {
        self.reset();
        self.bag.shuffle(rng);
        for player in self.players.iter_mut() {
            self.bag
                .replenish_back(&mut player.rack, game_config.rack_size() as usize);
        }
    }

    #[inline(always)]
    pub fn reset_and_draw_tiles_double_ended<R: Rng + ?Sized>(
        &mut self,
        game_config: &game_config::GameConfig,
        rng: &mut R,
    ) {
        self.reset();
        self.bag.shuffle(rng);
        for (i, player) in self.players.iter_mut().enumerate() {
            self.bag
                .replenish(&mut player.rack, game_config.rack_size() as usize, i);
        }
    }

    // an opponent holding a desired tile not found in bag will draw another.
    // if desired tile is missing, final rack will be shorter.
    #[inline(always)]
    pub fn set_current_rack(&mut self, desired_rack: &[u8]) {
        self.bag
            .return_tiles(&self.players[self.turn as usize].rack);
        self.players[self.turn as usize].rack.clear();
        for &tile in desired_rack {
            match self.bag.remove_tile(tile) {
                Some(()) => {
                    self.players[self.turn as usize].rack.push(tile);
                }
                None => {
                    for i in 0..self.players.len() {
                        if i != self.turn as usize {
                            match self.players[i].rack.iter().rposition(|&t| t == tile) {
                                Some(pos) => {
                                    let len = self.players[i].rack.len();
                                    self.players[i].rack.swap_remove(pos);
                                    self.players[self.turn as usize].rack.push(tile);
                                    self.bag.replenish(&mut self.players[i].rack, len, i);
                                    break;
                                }
                                None => continue,
                            }
                        }
                    }
                }
            }
        }
    }

    #[inline(always)]
    pub fn current_player(&self) -> &GamePlayer {
        &self.players[self.turn as usize]
    }

    #[inline(always)]
    pub fn play<R: Rng + ?Sized>(
        &mut self,
        game_config: &game_config::GameConfig,
        rng: &mut R,
        play: &movegen::Play,
    ) -> error::Returns<()> {
        let current_player = &mut self.players[self.turn as usize];
        match play {
            movegen::Play::Exchange { tiles } => {
                if tiles.is_empty() {
                    self.pass_turns += 1;
                    self.zero_turns += 1;
                } else {
                    use_tiles(&mut current_player.rack, tiles.iter().copied())?;
                    self.bag.replenish(
                        &mut current_player.rack,
                        game_config.rack_size() as usize,
                        self.turn as usize,
                    );
                    self.bag.put_back(rng, tiles);
                    self.pass_turns = 0;
                    current_player.num_exchanges += 1;
                    if game_config.exchanges_are_zeros() {
                        self.zero_turns += 1;
                    } else {
                        self.zero_turns = 0;
                    }
                }
            }
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let strider = game_config.board_layout().dim().lane(*down, *lane);

                // every check before any change, so a refused play changes
                // nothing but the rack's order
                let new_score = current_player
                    .score
                    .checked_add(*score)
                    .ok_or("score overflow")?;
                use_tiles(
                    &mut current_player.rack,
                    word.iter().filter_map(|&tile| {
                        if tile != 0 {
                            Some(tile & !((tile as i8) >> 7) as u8)
                        } else {
                            None
                        }
                    }),
                )?;
                current_player.score = new_score;

                // place the tiles
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        self.board_tiles[strider.at(i)] = tile;
                    }
                }

                self.bag.replenish(
                    &mut current_player.rack,
                    game_config.rack_size() as usize,
                    self.turn as usize,
                );
                self.zero_turns = 0;
                self.pass_turns = 0;
            }
        }
        Ok(())
    }

    // count a move challenged off the board: a scoreless turn, and a pass as
    // well where the config says challenges are passes, else a turn that was
    // not a pass, which ends a run of them
    #[inline(always)]
    pub fn challenged_off(&mut self, game_config: &game_config::GameConfig) {
        if game_config.challenges_are_passes() {
            self.pass_turns += 1;
        } else {
            self.pass_turns = 0;
        }
        self.zero_turns += 1;
    }

    #[inline(always)]
    pub fn next_turn(&mut self) {
        let num_players = self.players.len() as u8;
        self.turn += 1;
        self.turn -= num_players & -((self.turn >= num_players) as i8) as u8;
    }

    // whether turns that scored nothing, or passes, have ended the game
    #[inline(always)]
    pub fn scoreless_turns_ended(&self, game_config: &game_config::GameConfig) -> bool {
        (self.pass_turns > 0
            && game_config.num_passes_to_end() != 0
            && self.pass_turns >= game_config.num_passes_to_end() as u16)
            || (self.zero_turns > 0
                && game_config.num_zeros_to_end() != 0
                && self.zero_turns >= game_config.num_zeros_to_end() as u16
                && (game_config.zeros_can_end_empty_board()
                    || self.board_tiles.iter().any(|&tile| tile != 0)))
    }

    #[inline(always)]
    pub fn check_game_ended(
        &self,
        game_config: &game_config::GameConfig,
        final_scores: &mut [i32],
    ) -> error::Returns<CheckGameEnded> {
        if self.current_player().rack.is_empty() {
            for (i, player) in self.players.iter().enumerate() {
                final_scores[i] = player.score;
            }
            if self.players.len() == 2 {
                final_scores[self.turn as usize] = (2 * equity::SCALE)
                    .checked_mul(
                        game_config
                            .alphabet()
                            .rack_score(&self.players[(1 - self.turn) as usize].rack),
                    )
                    .and_then(|bonus| final_scores[self.turn as usize].checked_add(bonus))
                    .ok_or("score overflow")?;
            } else {
                let mut earned = 0i32;
                for (i, player) in self.players.iter().enumerate() {
                    let this_rack = game_config.alphabet().scaled_rack_score(&player.rack);
                    final_scores[i] = final_scores[i]
                        .checked_sub(this_rack)
                        .ok_or("score overflow")?;
                    earned = earned.checked_add(this_rack).ok_or("score overflow")?;
                }
                final_scores[self.turn as usize] = final_scores[self.turn as usize]
                    .checked_add(earned)
                    .ok_or("score overflow")?;
            }
            Ok(CheckGameEnded::PlayedOut)
        } else if self.scoreless_turns_ended(game_config) {
            for (i, player) in self.players.iter().enumerate() {
                final_scores[i] = player
                    .score
                    .checked_sub(game_config.alphabet().scaled_rack_score(&player.rack))
                    .ok_or("score overflow")?;
            }
            Ok(CheckGameEnded::ZeroScores)
        } else {
            Ok(CheckGameEnded::NotEnded)
        }
    }
}

#[derive(Clone, Copy)]
pub enum CheckGameEnded {
    NotEnded,
    PlayedOut,
    ZeroScores,
}

#[cfg(test)]
mod tests {
    use super::*;

    // a play the rack cannot make is refused and changes nothing: no tile on
    // the board, no score, the rack's tiles all still there
    #[test]
    fn a_refused_play_changes_nothing() {
        let game_config = game_config::make_english_game_config();
        let mut game_state = GameState::new(&game_config);
        game_state.players[0]
            .rack
            .extend_from_slice(&[1, 2, 3, 4, 5, 6, 7]);
        game_state.zero_turns = 3;
        let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0);
        // A, B, then a Z the rack lacks
        let play = movegen::Play::Place {
            down: false,
            lane: 7,
            idx: 7,
            word: [1, 2, 26][..].into(),
            score: 10,
        };
        assert!(game_state.play(&game_config, &mut rng, &play).is_err());
        assert!(game_state.board_tiles.iter().all(|&tile| tile == 0));
        assert_eq!(game_state.players[0].score, 0);
        assert_eq!(game_state.zero_turns, 3);
        let mut rack = game_state.players[0].rack.clone();
        rack.sort_unstable();
        assert_eq!(rack, [1, 2, 3, 4, 5, 6, 7]);
        // an exchange of a tile the rack lacks, the same
        let play = movegen::Play::Exchange {
            tiles: [7, 7][..].into(),
        };
        assert!(game_state.play(&game_config, &mut rng, &play).is_err());
        let mut rack = game_state.players[0].rack.clone();
        rack.sort_unstable();
        assert_eq!(rack, [1, 2, 3, 4, 5, 6, 7]);
    }

    // a whole success takes tiles off exactly as swap_remove did
    #[test]
    fn use_tiles_keeps_swap_removes_order() {
        let mut rack = vec![1, 2, 3, 1, 5, 6, 7];
        use_tiles(&mut rack, [1, 6]).unwrap();
        let mut old = vec![1, 2, 3, 1, 5, 6, 7];
        for tile in [1, 6] {
            let pos = old.iter().rposition(|&t| t == tile).unwrap();
            old.swap_remove(pos);
        }
        assert_eq!(rack, old);
    }
}
