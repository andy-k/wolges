// Copyright (C) 2020-2026 Andy Kurnia.

// the shell's session: the state its commands built, and the commands
// themselves, which save writes out and source replays to build the same state
// again

use rand::{Rng, SeedableRng};
use std::io::Write;
use wolges::{
    alphabet, anagrams, display, equity, error, game_config, game_state, infer, klv, kwg,
    move_filter, move_picker, movegen, notation, parallelism, play_scorer,
};

type GameRng = rand_xoshiro::Xoshiro256PlusPlus;

// how far, in points of equity, a sim takes the opponent's last move to fall
// short of their best when it weighs the racks that could have made it
const INFER_FUZZ: f64 = 3.105;

// a move of the game, with the rack its mover was given before it, if any
#[derive(Clone)]
struct Turn {
    rack: Option<Vec<u8>>,
    play: movegen::Play,
}

// a game: the bag dealt from its seed, its moves, how many of them the board
// shows, and a rack given to the side to move that has not played yet
#[derive(Clone)]
struct Game {
    seed: u64,
    turns: Vec<Turn>,
    at: usize,
    rack: Option<Vec<u8>>,
}

// the tables a session plays with stay for its whole life (a file is read once,
// by its path, and its anagram tables are built once), so that a sim, which
// borrows them, can be kept between commands
pub struct Session {
    game_config: &'static game_config::GameConfig,
    kwg: Option<(String, &'static kwg::Kwg<kwg::Node22>)>,
    klv: &'static klv::Klv<kwg::Node22>,
    kwgs_read: std::collections::HashMap<String, &'static kwg::Kwg<kwg::Node22>>,
    klvs_read: std::collections::HashMap<String, &'static klv::Klv<kwg::Node22>>,
    arm_tables_built: std::collections::HashMap<String, &'static anagrams::ArmTables>,
    infer_tables: Option<infer::InferTables>, // for klv, built at its first use
    game: Option<Game>,
    state: game_state::GameState,
    board_before_last: Option<(Vec<u8>, movegen::Play)>,
    final_scores: Option<Vec<i32>>,
    move_generator: movegen::KurniaMoveGenerator,
    infer_generator: movegen::KurniaMoveGenerator,
    play_scorer: play_scorer::PlayScorer,
    log: Vec<String>,
    quiet: bool, // no progress printed while a sim runs
}

// each command as it is typed, and what it does, for help
pub const COMMANDS: &[(&str, &str)] = &[
    ("lexicon FILE", "use the lexicon in FILE (a kwg)"),
    (
        "leaves FILE",
        "use the leave values in FILE (a klv2), or none for -",
    ),
    (
        "new [SEED]",
        "start a new game, its bag shuffled from SEED (a random one if not given)",
    ),
    (
        "rack TILES",
        "give the side to move these tiles, ? for a blank",
    ),
    (
        "play 8D WORD",
        "the side to move plays WORD across from 8D (D8 for down)",
    ),
    ("play exch TILES", "the side to move exchanges these tiles"),
    ("play pass", "the side to move passes"),
    (
        "bot",
        "the side to move plays the best move by equity (logged as play)",
    ),
    (
        "bot sim MS",
        "the side to move plays the best move of a sim of MS ms (logged as play)",
    ),
    ("back [N]", "take back N moves, 1 if not given"),
    ("forward [N]", "replay N moves taken back, 1 if not given"),
    (
        "show",
        "show the board, the racks, the scores, and the moves",
    ),
    (
        "gen [N]",
        "list the side to move's N best moves by equity, 10 if not given",
    ),
    ("log", "list the commands that built this session"),
    (
        "save FILE",
        "write those commands to FILE, a new file, for source to run",
    ),
    ("source FILE", "run the commands in FILE"),
    ("help", "this list"),
    ("exit", "leave the shell"),
];

impl Session {
    pub fn new(quiet: bool) -> error::Returns<Self> {
        let game_config: &'static _ = Box::leak(Box::new(game_config::make_english_game_config()));
        Ok(Self {
            game_config,
            kwg: None,
            klv: Box::leak(Box::new(klv::Klv::<kwg::Node22>::from_bytes_alloc(
                klv::EMPTY_KLV_BYTES,
            )?)),
            kwgs_read: Default::default(),
            klvs_read: Default::default(),
            arm_tables_built: Default::default(),
            infer_tables: None,
            game: None,
            state: game_state::GameState::new(game_config),
            board_before_last: None,
            final_scores: None,
            move_generator: movegen::KurniaMoveGenerator::new(game_config),
            infer_generator: movegen::KurniaMoveGenerator::new(game_config),
            play_scorer: play_scorer::PlayScorer::new(),
            log: Vec::new(),
            quiet,
        })
    }

    // run one command, its words split from line. A command that changes the
    // session is logged as line (new with the seed it used, bot as the move it
    // chose); a command that fails changes nothing
    pub fn run(&mut self, line: &str, words: &[String]) -> error::Returns<()> {
        let args = &words[1..];
        let logged = match words[0].as_str() {
            "lexicon" => {
                let [path] = args else {
                    return Err("lexicon FILE".into());
                };
                let kwg = match self.kwgs_read.get(path) {
                    Some(&kwg) => kwg,
                    None => {
                        let kwg: &'static _ = Box::leak(Box::new(
                            kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read(path)?)?,
                        ));
                        self.kwgs_read.insert(path.clone(), kwg);
                        kwg
                    }
                };
                self.use_kwg(path, kwg);
                line.to_string()
            }
            "leaves" => {
                let [path] = args else {
                    return Err("leaves FILE, or - for none".into());
                };
                let klv = match self.klvs_read.get(path) {
                    Some(&klv) => klv,
                    None => {
                        let bytes = if path == "-" {
                            klv::EMPTY_KLV_BYTES.to_vec()
                        } else {
                            std::fs::read(path)?
                        };
                        let klv = klv::Klv::<kwg::Node22>::from_bytes_alloc(&bytes)?;
                        self.game_config.check_leaves(klv.leave_range())?;
                        let klv: &'static _ = Box::leak(Box::new(klv));
                        self.klvs_read.insert(path.clone(), klv);
                        klv
                    }
                };
                if !std::ptr::eq(klv, self.klv) {
                    self.klv = klv;
                    self.infer_tables = None;
                }
                line.to_string()
            }
            "new" => {
                let seed = match args {
                    [] => GameRng::try_from_rng(&mut rand::rngs::SysRng)?.next_u64(),
                    [seed] => seed.parse::<u64>()?,
                    _ => return Err("new [SEED]".into()),
                };
                self.change(|game| {
                    *game = Some(Game {
                        seed,
                        turns: Vec::new(),
                        at: 0,
                        rack: None,
                    });
                    Ok(())
                })?;
                self.show()?;
                format!("new {seed}")
            }
            "rack" => {
                let [tiles] = args else {
                    return Err("rack TILES".into());
                };
                let mut rack = Vec::new();
                alphabet::AlphabetReader::new_for_racks(self.game_config.alphabet())
                    .set_word(tiles, &mut rack)?;
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    game.turns.truncate(game.at);
                    game.rack = Some(rack);
                    Ok(())
                })?;
                line.to_string()
            }
            "play" => {
                let play = self.read_play(args)?;
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    game.turns.truncate(game.at);
                    game.turns.push(Turn {
                        rack: game.rack.take(),
                        play,
                    });
                    game.at += 1;
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "back" | "forward" => {
                let back = words[0] == "back";
                let n = match args {
                    [] => 1,
                    [n] => n.parse::<usize>()?,
                    _ => return Err(format!("{} [N]", words[0]).into()),
                };
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    game.at = if back {
                        game.at
                            .checked_sub(n)
                            .ok_or("not that many moves to take back")?
                    } else {
                        Some(game.at + n)
                            .filter(|&at| at <= game.turns.len())
                            .ok_or("not that many moves to replay")?
                    };
                    game.rack = None;
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "bot" => {
                let sim_ms = match args {
                    [] => None,
                    [sim, ms] if sim == "sim" => {
                        let ms = ms.parse::<f64>()?;
                        let usable = ms.is_finite() && ms > 0.0;
                        if !usable {
                            return Err("bot sim MS: MS above 0".into());
                        }
                        Some(ms)
                    }
                    _ => return Err("bot, or bot sim MS".into()),
                };
                let play = self.computer_move(sim_ms)?;
                // run and logged as the move it chose, so that a replay makes
                // the same move however the sim went
                let line = format!("play {}", self.play_text(&play)?);
                writeln!(std::io::stdout(), "bot: {line}")?;
                let words = line
                    .split_whitespace()
                    .map(String::from)
                    .collect::<Vec<_>>();
                return self.run(&line, &words);
            }
            "show" => return self.show(),
            "gen" => {
                let n = match args {
                    [] => 10,
                    [n] => n.parse::<usize>()?,
                    _ => return Err("gen [N]".into()),
                };
                return self.best_moves(n);
            }
            "log" => {
                let mut out = std::io::stdout();
                for line in &self.log {
                    writeln!(out, "{line}")?;
                }
                return Ok(());
            }
            "save" => {
                let [path] = args else {
                    return Err("save FILE".into());
                };
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                for line in &self.log {
                    writeln!(file, "{line}")?;
                }
                return Ok(());
            }
            _ => return Err("no such command, help for the commands".into()),
        };
        self.log.push(logged);
        Ok(())
    }

    fn use_kwg(&mut self, path: &str, kwg: &'static kwg::Kwg<kwg::Node22>) {
        self.kwg = Some((path.to_string(), kwg));
        self.move_generator.reset_for_another_kwg();
        self.infer_generator.reset_for_another_kwg();
    }

    // the word graph, or an error saying to read one
    fn kwg(&self) -> error::Returns<&'static kwg::Kwg<kwg::Node22>> {
        Ok(self.kwg.as_ref().ok_or("no lexicon: lexicon FILE first")?.1)
    }

    // the word graph's anagram tables, built at their first use
    fn arm_tables(&mut self) -> error::Returns<&'static anagrams::ArmTables> {
        let (path, kwg) = self.kwg.as_ref().ok_or("no lexicon: lexicon FILE first")?;
        Ok(match self.arm_tables_built.get(path) {
            Some(&arm_tables) => arm_tables,
            None => {
                let arm_tables: &'static _ = Box::leak(Box::new(anagrams::ArmTables::build(
                    *kwg,
                    self.game_config,
                    false,
                )?));
                self.arm_tables_built.insert(path.clone(), arm_tables);
                arm_tables
            }
        })
    }

    // a change to the game, kept only if the game it makes can be rebuilt;
    // otherwise the game and its state are as they were
    fn change(
        &mut self,
        f: impl FnOnce(&mut Option<Game>) -> error::Returns<()>,
    ) -> error::Returns<()> {
        let before = self.game.clone();
        let result = f(&mut self.game).and_then(|()| self.rebuild());
        if result.is_err() {
            self.game = before;
            self.rebuild()?;
        }
        result
    }

    // the state after the game's shown moves, rebuilt from its seed, with the
    // rack given to the side to move
    fn rebuild(&mut self) -> error::Returns<()> {
        let Some(game) = &self.game else {
            return Ok(());
        };
        let mut rng = GameRng::seed_from_u64(game.seed);
        let mut state = game_state::GameState::new(self.game_config);
        state.reset_and_draw_tiles_double_ended(self.game_config, &mut rng);
        let mut final_scores = vec![0; self.game_config.num_players() as usize];
        let mut ended = false;
        let mut board_before_last = None;
        for turn in game.turns[..game.at].iter() {
            if ended {
                return Err("the game is over".into());
            }
            if let Some(rack) = &turn.rack {
                give_rack(&mut state, rack)?;
            }
            board_before_last = Some((state.board_tiles.to_vec(), turn.play.clone()));
            state.play(self.game_config, &mut rng, &turn.play)?;
            match state.check_game_ended(self.game_config, &mut final_scores)? {
                game_state::CheckGameEnded::NotEnded => state.next_turn(),
                _ => ended = true,
            }
        }
        if let Some(rack) = &game.rack {
            if ended {
                return Err("the game is over".into());
            }
            give_rack(&mut state, rack)?;
        }
        self.state = state;
        self.board_before_last = board_before_last;
        self.final_scores = ended.then_some(final_scores);
        Ok(())
    }

    // read a move as typed (pass, an exchange, or a coordinate and a word),
    // check it against the board, the rack, and the lexicon, and score it
    fn read_play(&mut self, args: &[String]) -> error::Returns<movegen::Play> {
        if self.game.is_none() {
            return Err("no game: new first".into());
        }
        if self.final_scores.is_some() {
            return Err("the game is over".into());
        }
        let kwg = self.kwg()?;
        let play = match args {
            [pass] if pass == "pass" => movegen::Play::Exchange {
                tiles: [][..].into(),
            },
            [exch, tiles] if exch == "exch" => {
                let mut v = Vec::new();
                alphabet::AlphabetReader::new_for_racks(self.game_config.alphabet())
                    .set_word(tiles, &mut v)?;
                movegen::Play::Exchange {
                    tiles: v[..].into(),
                }
            }
            [coord, word] => {
                let dim = self.game_config.board_layout().dim();
                let coord = notation::parse_coord(coord, dim).ok_or("no such coordinate")?;
                let mut v = Vec::new();
                notation::read_place_word(
                    &alphabet::AlphabetReader::new_for_plays(self.game_config.alphabet()),
                    &self.state.board_tiles,
                    dim,
                    &coord,
                    word,
                    &mut v,
                )?;
                movegen::Play::Place {
                    down: coord.down,
                    lane: coord.lane,
                    idx: coord.idx,
                    word: v[..].into(),
                    score: 0,
                }
            }
            _ => return Err("play 8D WORD, play D8 WORD, play exch TILES, or play pass".into()),
        };
        let board_snapshot = &movegen::BoardSnapshot {
            board_tiles: &self.state.board_tiles,
            game_config: self.game_config,
            kwg,
            anagrams: None,
            rack_lengths: None,
            klv: self.klv,
        };
        let mut play = self
            .play_scorer
            .validate_play(board_snapshot, &self.state, &play)?
            .unwrap_or(play);
        if let movegen::Play::Place { .. } = play {
            let recounted = self.play_scorer.compute_score(board_snapshot, &play);
            if let movegen::Play::Place { score, .. } = &mut play {
                *score = recounted;
            }
        }
        let alphabet = self.game_config.alphabet();
        let mut phonies = Vec::new();
        self.play_scorer
            .find_invalid_words(board_snapshot, &play, |word: &[u8]| {
                phonies.push(
                    word.iter()
                        .map(|&tile| alphabet.of_board(tile).unwrap_or("?"))
                        .collect::<String>(),
                )
            });
        if !phonies.is_empty() {
            return Err(format!("not in the lexicon: {}", phonies.join(", ")).into());
        }
        Ok(play)
    }

    fn show(&self) -> error::Returns<()> {
        let mut out = std::io::stdout();
        let Some(game) = &self.game else {
            writeln!(out, "no game: new to start one")?;
            return Ok(());
        };
        display::print_game_state(self.game_config, &self.state, None)?;
        // each move with the board before it, so that a tile played through
        // shows; moves taken back are marked
        if let Some((_, kwg)) = self.kwg {
            let alphabet = self.game_config.alphabet();
            let mut rng = GameRng::seed_from_u64(game.seed);
            let mut board = game_state::GameState::new(self.game_config);
            board.reset_and_draw_tiles_double_ended(self.game_config, &mut rng);
            for (i, turn) in game.turns.iter().enumerate() {
                if let Some(rack) = &turn.rack {
                    give_rack(&mut board, rack)?;
                }
                let mover = board.turn as usize;
                let rack = alphabet.fmt_rack(&board.players[mover].rack).to_string();
                let text = turn
                    .play
                    .fmt(&movegen::BoardSnapshot {
                        board_tiles: &board.board_tiles,
                        game_config: self.game_config,
                        kwg,
                        anagrams: None,
                        rack_lengths: None,
                        klv: self.klv,
                    })
                    .to_string();
                board.play(self.game_config, &mut rng, &turn.play)?;
                writeln!(
                    out,
                    "{}{:>3}. player {} {:<7} {} (total {})",
                    if i < game.at { " " } else { "~" },
                    i + 1,
                    mover + 1,
                    rack,
                    text,
                    equity::descale_score(board.players[mover].score),
                )?;
                board.next_turn();
            }
        }
        if let Some(final_scores) = &self.final_scores {
            writeln!(
                out,
                "game over, {}",
                final_scores
                    .iter()
                    .map(|&s| equity::descale_score(s).to_string())
                    .collect::<Vec<_>>()
                    .join(" to ")
            )?;
        }
        Ok(())
    }

    fn best_moves(&mut self, n: usize) -> error::Returns<()> {
        if self.game.is_none() {
            return Err("no game: new first".into());
        }
        let kwg = self.kwg()?;
        let arm_tables = self.arm_tables()?;
        let board_snapshot = &movegen::BoardSnapshot {
            board_tiles: &self.state.board_tiles,
            game_config: self.game_config,
            kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv: self.klv,
        };
        self.move_generator
            .gen_moves_unfiltered(&movegen::GenMovesParams {
                board_snapshot,
                rack: &self.state.current_player().rack,
                max_gen: n,
                num_exchanges_by_this_player: self.state.current_player().num_exchanges,
                pass_policy: movegen::PassPolicy::AsACandidate,
                dynamic_leaves: None,
            });
        let mut out = std::io::stdout();
        for (i, valued) in self.move_generator.plays.iter().enumerate() {
            writeln!(
                out,
                "{:>3}. {:>8.3} {}",
                i + 1,
                valued.equity.as_f64(),
                valued.play.fmt(board_snapshot),
            )?;
        }
        Ok(())
    }

    // the move the computer would make for the side to move: the generator's
    // best, or given a time, the sim's best after that many milliseconds
    // (drawing the opponent's rack from what their last move says they kept)
    fn computer_move(&mut self, sim_ms: Option<f64>) -> error::Returns<movegen::Play> {
        if self.game.is_none() {
            return Err("no game: new first".into());
        }
        if self.final_scores.is_some() {
            return Err("the game is over".into());
        }
        let kwg = self.kwg()?;
        let arm_tables = self.arm_tables()?;
        let board_snapshot = &movegen::BoardSnapshot {
            board_tiles: &self.state.board_tiles,
            game_config: self.game_config,
            kwg,
            anagrams: arm_tables.anagrams(),
            rack_lengths: arm_tables.rack_lengths(),
            klv: self.klv,
        };
        let rack = self.state.current_player().rack.clone();
        let Some(ms) = sim_ms else {
            move_picker::MovePicker::<kwg::Node22, kwg::Node22>::Hasty.pick_a_move(
                &mut move_filter::GenMoves::Unfiltered,
                &mut self.move_generator,
                board_snapshot,
                &self.state,
                &rack,
            )?;
            return Ok(self.move_generator.plays[0].play.clone());
        };
        let mut leaves = Vec::new();
        if let Some((board_before, last)) = &self.board_before_last {
            let tables = match &mut self.infer_tables {
                Some(tables) => tables,
                tables @ None => {
                    tables.insert(infer::InferTables::new(self.game_config, self.klv)?)
                }
            };
            leaves = infer::kept_leaves(
                tables,
                &mut self.infer_generator,
                &movegen::BoardSnapshot {
                    board_tiles: board_before,
                    game_config: self.game_config,
                    kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: self.klv,
                },
                &rack,
                last,
                INFER_FUZZ,
            )?;
        }
        move_picker::gen_simmer_candidates(
            &mut move_filter::GenMoves::Unfiltered,
            &mut self.move_generator,
            board_snapshot,
            &rack,
            self.state.current_player().num_exchanges,
            move_picker::SIMMER_FIELD,
            &mut Vec::new(),
        );
        // the time counts the sim alone; before each step, the time left is
        // turned into rollouts at the rate so far, which the pruning plans for
        let t0 = std::time::Instant::now();
        let mut simmer = move_picker::Simmer::<_, _, false>::new(
            self.game_config,
            kwg,
            arm_tables,
            self.klv,
            move_picker::SimmerParams {
                num_sim_iters: (ms as u64).max(16),
                num_sim_plies: move_picker::DEFAULT_NUM_SIM_PLIES,
                sim_threads: parallelism::available(),
                win_pct_table: None,
                quiet: true, // the shell prints its own lines
            },
        )?;
        // step up to the next look each time, so the threads share whole
        // blocks; unless quiet, print a line a second
        let num_candidates = self.move_generator.plays.len();
        let first = simmer.iters_to_next_look();
        let mut rollouts = (first as usize * num_candidates) as f64;
        let mut iterations = first;
        let mut ticked = 0;
        simmer.begin_decision(&self.move_generator, &self.state, first, &leaves)?;
        loop {
            let spent = t0.elapsed().as_secs_f64() * 1000.0;
            let in_play = simmer.active_stream_ids().count();
            if spent >= ms || in_play < 2 {
                break;
            }
            let secs = spent as u64 / 1000;
            if !self.quiet && secs > ticked {
                ticked = secs;
                writeln!(
                    std::io::stderr(),
                    "sim: {iterations} iterations, {in_play} left in, {spent:.0} of {ms} ms"
                )?;
            }
            simmer.plan_rollouts((ms - spent) * rollouts / spent);
            let step = simmer.iters_to_next_look();
            simmer.resume(&self.move_generator, step)?;
            rollouts += (step as usize * in_play) as f64;
            iterations += step;
        }
        writeln!(
            std::io::stdout(),
            "sim: {num_candidates} candidates, {iterations} iterations, {rollouts} rollouts, {} left in, {:.0} ms",
            simmer.active_stream_ids().count(),
            t0.elapsed().as_secs_f64() * 1000.0,
        )?;
        Ok(self.move_generator.plays[simmer.best_so_far()].play.clone())
    }

    // a move written as play takes it: a coordinate and a word (8D across, D8
    // down; a tile played through in parentheses), exch and its tiles, or pass
    fn play_text(&self, play: &movegen::Play) -> error::Returns<String> {
        Ok(match play {
            movegen::Play::Exchange { tiles } if tiles.is_empty() => "pass".into(),
            movegen::Play::Exchange { tiles } => {
                format!("exch {}", self.game_config.alphabet().fmt_rack(tiles))
            }
            movegen::Play::Place { .. } => {
                let shown = play
                    .fmt(&movegen::BoardSnapshot {
                        board_tiles: &self.state.board_tiles,
                        game_config: self.game_config,
                        kwg: self.kwg()?,
                        anagrams: None,
                        rack_lengths: None,
                        klv: self.klv,
                    })
                    .to_string();
                // without its score
                shown
                    .rsplit_once(' ')
                    .map_or(shown.clone(), |(text, _)| text.to_string())
            }
        })
    }
}

// the side to move given exactly these tiles, from the bag or the other racks
fn give_rack(state: &mut game_state::GameState, rack: &[u8]) -> error::Returns<()> {
    state.set_current_rack(rack);
    if state.current_player().rack.len() != rack.len() {
        return Err("those tiles are not all unseen".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wolges::build;

    // every two- and three-letter word of A, E, and T
    fn tiny_lexicon() -> kwg::Kwg<kwg::Node22> {
        const LETTERS: [u8; 3] = [1, 5, 20];
        let mut words = Vec::new();
        for &a in &LETTERS {
            for &b in &LETTERS {
                words.push([a, b][..].into());
                for &c in &LETTERS {
                    words.push([a, b, c][..].into());
                }
            }
        }
        words.sort_unstable();
        let bytes = build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &words,
        )
        .unwrap();
        kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&bytes).unwrap()
    }

    fn session() -> Session {
        let mut session = Session::new(true).unwrap();
        session.use_kwg("tiny", Box::leak(Box::new(tiny_lexicon())));
        session
    }

    fn run(session: &mut Session, line: &str) -> error::Returns<()> {
        let words = line
            .split_whitespace()
            .map(String::from)
            .collect::<Vec<_>>();
        session.run(line, &words)
    }

    // a state, for comparing: the board, each score and rack (sorted), the bag
    // (sorted), and whose turn it is
    type Snapshot = (Vec<u8>, Vec<(i32, Vec<u8>)>, Vec<u8>, u8);

    fn snapshot(session: &Session) -> Snapshot {
        let state = &session.state;
        let mut bag = state.bag.as_slice().to_vec();
        bag.sort_unstable();
        (
            state.board_tiles.to_vec(),
            state
                .players
                .iter()
                .map(|p| {
                    let mut rack = p.rack.clone();
                    rack.sort_unstable();
                    (p.score, rack)
                })
                .collect(),
            bag,
            state.turn,
        )
    }

    // AT through the star, EA under it (making TE down), then TEA down column H
    // from the T, the first mover's second A
    const GAME: &[&str] = &[
        "new 7",
        "rack AETAETE",
        "play 8G AT",
        "rack TEATEAT",
        "play 9H EA",
        "play H8 TEA",
    ];

    #[test]
    #[inline]
    fn a_sessions_log_replays_to_the_same_state() {
        let mut first = session();
        for line in GAME {
            run(&mut first, line).unwrap();
        }
        assert_eq!(first.log, GAME);
        let mut second = session();
        for line in first.log.clone() {
            run(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&first), snapshot(&second));
        // the first mover's moves: AT, doubled on the star, for 4, and TEA for
        // 3
        assert_eq!(first.state.players[0].score, equity::scale_score(7));
    }

    #[test]
    #[inline]
    fn back_and_forward_move_along_the_game_and_a_new_move_cuts_what_was_ahead() {
        let mut session = session();
        let mut states = Vec::new();
        for line in &GAME[..2] {
            run(&mut session, line).unwrap();
        }
        states.push(snapshot(&session));
        for line in &GAME[2..] {
            run(&mut session, line).unwrap();
            if line.starts_with("play") {
                states.push(snapshot(&session));
            }
        }
        run(&mut session, "back 2").unwrap();
        assert_eq!(snapshot(&session), states[1]);
        run(&mut session, "forward").unwrap();
        assert_eq!(snapshot(&session), states[2]);
        assert!(run(&mut session, "forward 2").is_err());
        assert!(run(&mut session, "back 4").is_err());
        // a different second move drops the third
        run(&mut session, "back").unwrap();
        run(&mut session, "rack EAT").unwrap();
        run(&mut session, "play 9G EAT").unwrap();
        assert!(run(&mut session, "forward").is_err());
        assert_eq!(session.game.as_ref().unwrap().turns.len(), 2);
    }

    #[test]
    #[inline]
    fn a_move_that_fails_changes_nothing_and_logs_nothing() {
        let mut session = session();
        for line in &GAME[..2] {
            run(&mut session, line).unwrap();
        }
        let before = snapshot(&session);
        for bad in [
            "play 8A AT",   // misses the star
            "play 8G ZA",   // Z is not on the rack
            "play 8H AE A", // three words
            "play 8G TATT", // a T short
            "rack ZZ",      // one Z in the bag
            "play H8 AE",   // fine
        ] {
            let result = run(&mut session, bad);
            if bad == "play H8 AE" {
                assert!(result.is_ok(), "{bad}");
            } else {
                assert!(result.is_err(), "{bad}");
                assert_eq!(snapshot(&session), before, "{bad}");
            }
        }
        assert_eq!(session.log.len(), 3);
    }

    #[test]
    #[inline]
    fn a_bots_move_is_logged_as_the_play_it_made_and_replays() {
        let mut first = session();
        for line in &GAME[..2] {
            run(&mut first, line).unwrap();
        }
        run(&mut first, "bot").unwrap();
        run(&mut first, "rack TEATEAT").unwrap();
        run(&mut first, "bot sim 30").unwrap();
        assert!(first.log[2].starts_with("play "), "{}", first.log[2]);
        assert!(first.log[4].starts_with("play "), "{}", first.log[4]);
        assert!(!first.log.iter().any(|line| line.starts_with("bot")));
        let mut second = session();
        for line in first.log.clone() {
            run(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&first), snapshot(&second));
        assert!(run(&mut first, "bot sim 0").is_err());
        assert!(run(&mut first, "bot think").is_err());
    }

    #[test]
    #[inline]
    fn a_word_not_in_the_lexicon_is_refused() {
        let mut session = session();
        run(&mut session, "new 7").unwrap();
        run(&mut session, "rack AETAETE").unwrap();
        assert!(run(&mut session, "play 8G AT").is_ok());
        run(&mut session, "rack EEE").unwrap();
        // placed rightly, but TEEE has four letters, and the tiny lexicon's
        // words have two or three
        let before = snapshot(&session);
        let error = run(&mut session, "play H8 TEEE").unwrap_err().to_string();
        assert_eq!(error, "not in the lexicon: TEEE");
        assert_eq!(snapshot(&session), before);
        assert!(run(&mut session, "play H8 TEE").is_ok());
    }

    #[test]
    #[inline]
    fn a_rack_may_hold_more_tiles_than_a_full_rack_or_fewer() {
        let mut first = session();
        run(&mut first, "new 7").unwrap();
        // playing all eight tiles of an overdrawn rack earns a full rack's
        // bonus: across row 8 from D, with D8 doubling an A and the star
        // doubling the word, 18 plus 50
        let play = movegen::Play::Place {
            down: false,
            lane: 7,
            idx: 3,
            word: [1, 5, 20, 1, 5, 20, 5, 1][..].into(),
            score: 0,
        };
        let score = first.play_scorer.compute_score(
            &movegen::BoardSnapshot {
                board_tiles: &first.state.board_tiles,
                game_config: first.game_config,
                kwg: first.kwg().unwrap(),
                anagrams: None,
                rack_lengths: None,
                klv: first.klv,
            },
            &play,
        );
        assert_eq!(score, equity::scale_score(68));
        // overdrawn: ten tiles; playing two leaves eight, and nothing is drawn
        run(&mut first, "rack AETAETEATE").unwrap();
        assert_eq!(first.state.players[0].rack.len(), 10);
        run(&mut first, "play 8G AT").unwrap();
        assert_eq!(first.state.players[0].rack.len(), 8);
        // underdrawn: two tiles
        run(&mut first, "rack EA").unwrap();
        assert_eq!(first.state.players[1].rack.len(), 2);
        run(&mut first, "play 9H EA").unwrap();
        let mut second = session();
        for line in first.log.clone() {
            run(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&second), snapshot(&first));
    }
}
