// Copyright (C) 2020-2023 Andy Kurnia.

use super::{bag, error, game_config, movegen};
use rand::prelude::*;

pub fn use_tiles<II: IntoIterator<Item = u8>>(
    rack: &mut Vec<u8>,
    tiles_iter: II,
) -> error::Returns<()> {
    for tile in tiles_iter {
        let pos = rack.iter().rposition(|&t| t == tile).ok_or("bad tile")?;
        rack.swap_remove(pos);
    }
    Ok(())
}

pub struct GamePlayer {
    pub score: i16,
    pub rack: Vec<u8>,
}

impl Clone for GamePlayer {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            score: self.score,
            rack: self.rack.clone(),
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.score.clone_from(&source.score);
        self.rack.clone_from(&source.rack);
    }
}

pub struct GameState {
    pub players: Box<[GamePlayer]>,
    pub board_tiles: Box<[u8]>,
    pub bag: bag::Bag,
    pub turn: u8,
    pub zero_turns: u16,
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
        }
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        self.players.clone_from(&source.players);
        self.board_tiles.clone_from(&source.board_tiles);
        self.bag.clone_from(&source.bag);
        self.turn.clone_from(&source.turn);
        self.zero_turns.clone_from(&source.zero_turns);
    }
}

impl GameState {
    // The other methods must be called with the same game_config.
    pub fn new(game_config: &game_config::GameConfig<'_>) -> Self {
        let board_layout = game_config.board_layout();
        let dim = board_layout.dim();
        let rack_size = game_config.rack_size() as usize;
        let alphabet = game_config.alphabet();
        Self {
            players: (0..game_config.num_players())
                .map(|_| GamePlayer {
                    score: 0,
                    rack: Vec::with_capacity(rack_size),
                })
                .collect(),
            board_tiles: vec![0u8; (dim.rows as usize) * (dim.cols as usize)].into_boxed_slice(),
            bag: bag::Bag::new(alphabet),
            turn: 0,
            zero_turns: 0,
        }
    }

    pub fn reset(&mut self) {
        for player in self.players.iter_mut() {
            player.score = 0;
            self.bag.0.extend_from_slice(&player.rack);
            player.rack.clear();
        }
        for &tile in self.board_tiles.iter().filter(|&&tile| tile != 0) {
            self.bag.0.push(tile & !((tile as i8) >> 7) as u8);
        }
        self.board_tiles.iter_mut().for_each(|m| *m = 0);
        self.turn = 0;
        self.zero_turns = 0;
    }

    pub fn reset_and_draw_tiles(
        &mut self,
        game_config: &game_config::GameConfig<'_>,
        mut rng: &mut dyn RngCore,
    ) {
        self.reset();
        self.bag.shuffle(&mut rng);
        for player in self.players.iter_mut() {
            self.bag
                .replenish(&mut player.rack, game_config.rack_size() as usize);
        }
    }

    // an opponent holding a desired tile not found in bag will draw another.
    // if desired tile is missing, final rack will be shorter.
    pub fn set_current_rack(&mut self, desired_rack: &[u8]) {
        self.bag
            .0
            .extend_from_slice(&self.players[self.turn as usize].rack);
        self.players[self.turn as usize].rack.clear();
        for &tile in desired_rack {
            match self.bag.0.iter().rposition(|&t| t == tile) {
                Some(pos) => {
                    self.bag.0.swap_remove(pos);
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
                                    self.bag.replenish(&mut self.players[i].rack, len);
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

    pub fn current_player(&self) -> &GamePlayer {
        &self.players[self.turn as usize]
    }

    pub fn play(
        &mut self,
        game_config: &game_config::GameConfig<'_>,
        mut rng: &mut dyn RngCore,
        play: &movegen::Play,
    ) -> error::Returns<()> {
        let current_player = &mut self.players[self.turn as usize];
        match play {
            movegen::Play::Exchange { tiles } => {
                use_tiles(&mut current_player.rack, tiles.iter().copied())?;
                self.bag
                    .replenish(&mut current_player.rack, game_config.rack_size() as usize);
                self.bag.put_back(&mut rng, tiles);
                self.zero_turns += 1;
            }
            movegen::Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => {
                let strider = game_config.board_layout().dim().lane(*down, *lane);

                // place the tiles
                for (i, &tile) in (*idx..).zip(word.iter()) {
                    if tile != 0 {
                        self.board_tiles[strider.at(i)] = tile;
                    }
                }

                current_player.score += score;
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
                self.bag
                    .replenish(&mut current_player.rack, game_config.rack_size() as usize);
                self.zero_turns = 0;
            }
        }
        Ok(())
    }

    pub fn next_turn(&mut self) {
        let num_players = self.players.len() as u8;
        self.turn += 1;
        self.turn -= num_players & -((self.turn >= num_players) as i8) as u8;
    }

    pub fn check_game_ended(
        &self,
        game_config: &game_config::GameConfig<'_>,
        final_scores: &mut [i16],
    ) -> CheckGameEnded {
        if self.current_player().rack.is_empty() {
            for (i, player) in self.players.iter().enumerate() {
                final_scores[i] = player.score;
            }
            if self.players.len() == 2 {
                final_scores[self.turn as usize] += 2 * game_config
                    .alphabet()
                    .rack_score(&self.players[(1 - self.turn) as usize].rack);
            } else {
                let mut earned = 0;
                for (i, player) in self.players.iter().enumerate() {
                    let this_rack = game_config.alphabet().rack_score(&player.rack);
                    final_scores[i] -= this_rack;
                    earned += this_rack;
                }
                final_scores[self.turn as usize] += earned;
            }
            CheckGameEnded::PlayedOut
        } else if self.zero_turns >= game_config.num_players() as u16 * 3 {
            for (i, player) in self.players.iter().enumerate() {
                final_scores[i] = player.score - game_config.alphabet().rack_score(&player.rack);
            }
            CheckGameEnded::ZeroScores
        } else {
            CheckGameEnded::NotEnded
        }
    }
}

pub enum CheckGameEnded {
    NotEnded,
    PlayedOut,
    ZeroScores,
}
