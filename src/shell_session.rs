// Copyright (C) 2020-2026 Andy Kurnia.

// the shell's session: the state its commands built, and the commands
// themselves, which save writes out and source replays to build the same state
// again

use crate::endgame_watch;
use rand::{Rng, SeedableRng};
use std::io::Write;
use wolges::{
    alphabet, anagrams, display, endgame, equity, error, game_config, game_state, infer, klv, kwg,
    move_filter, move_picker, movegen, notation, parallelism, play_scorer,
};

type GameRng = rand_xoshiro::Xoshiro256PlusPlus;

// how many iterations sim runs when given no count
const DEFAULT_SIM_ITERATIONS: u64 = 1000;

// how long endgame solves for when given no time, in seconds
const DEFAULT_ENDGAME_SECONDS: f64 = 10.0;

// a move of the game, with: the rack its mover was given before it, if any (for
// a seat given, what it was known to hold plus what the move needed); whether
// the mover's seat was given, so that they drew nothing anyone saw; whether its
// exchange was of tiles no one saw (exch N); and what was done to it afterward,
// in order
#[derive(Clone, PartialEq)]
struct Turn {
    rack: Option<Vec<u8>>,
    given: bool,
    play: movegen::Play,
    hidden: bool,
    after: Vec<After>,
}

// what can be done to a move after it is played: taking it back as a phony (the
// turn then becomes a pass, with the board and the rack as before it, and what
// was done to it before, an end declared, racks set, and time, is undone with
// it); points to its player (a challenge's bonus); the game lost by a player on
// a challenge of it; the game declared over (end); a player's rack set (none
// for a player with no tiles); on the move that ended the game, points a player
// lost for time; points its player was given beyond the board's score, or short
// of it (a misscore); and a note on it
#[derive(Clone, PartialEq)]
enum After {
    Withdraw,
    Bonus(i32),
    Lose(usize),
    End,
    Rack(usize, Vec<u8>),
    Time(usize, i32),
    Adjust(i32),
    Note(String),
}

// what a challenge of a move costs: under double, the challenger's turn if the
// move stands; under bonus, a number of points to its player if it stands, per
// word challenged or per challenge (single is a bonus of none, shown as +0);
// under triple, the game, which ends, lost by the challenger if the move stands
// and by its player (who takes it back) if it does not
#[derive(Clone, Copy, PartialEq)]
enum ChallengeRule {
    Double,
    Bonus { points: i32, per_word: bool },
    Triple,
}

impl Turn {
    // what the turn did to the game: a phony taken back is a pass
    fn played(&self) -> std::borrow::Cow<'_, movegen::Play> {
        if self.withdrawn() {
            std::borrow::Cow::Owned(movegen::Play::Exchange {
                tiles: [][..].into(),
            })
        } else {
            std::borrow::Cow::Borrowed(&self.play)
        }
    }

    fn withdrawn(&self) -> bool {
        self.after.contains(&After::Withdraw)
    }

    // the points its challenges that stood gave its player, if any did
    fn bonus(&self) -> Option<i32> {
        let bonus = |after: &After| match after {
            After::Bonus(points) => Some(*points),
            _ => None,
        };
        self.after
            .iter()
            .filter_map(bonus)
            .reduce(|sum, points| sum + points)
    }

    // the points its player was given beyond the board's score (below 0 for
    // short of it): the misscore's size
    fn adjust(&self) -> i32 {
        let adjust = |after: &After| match after {
            After::Adjust(points) => *points,
            _ => 0,
        };
        self.after.iter().map(adjust).sum()
    }

    // the player who lost the game on a challenge of it
    fn loser(&self) -> Option<usize> {
        self.after.iter().find_map(|after| match after {
            After::Lose(seat) => Some(*seat),
            _ => None,
        })
    }

    // what was done to it since it was taken back, if it was: all of it, if not
    fn since_withdrawn(&self) -> &[After] {
        let from = self
            .after
            .iter()
            .rposition(|after| *after == After::Withdraw);
        &self.after[from.map_or(0, |at| at + 1)..]
    }

    // whether the game was declared over after it
    fn ended(&self) -> bool {
        self.since_withdrawn().contains(&After::End)
    }

    // the racks set after it, each seat's last
    fn shown(&self) -> Vec<(usize, Vec<u8>)> {
        let racks = self
            .since_withdrawn()
            .iter()
            .filter_map(|after| match after {
                After::Rack(seat, tiles) => Some((*seat, tiles.clone())),
                _ => None,
            });
        last_each(racks.collect())
    }

    // what time cost each player
    fn time(&self) -> [i32; 2] {
        let mut time = [0; 2];
        for after in self.since_withdrawn() {
            if let After::Time(seat, points) = after {
                time[*seat] += points;
            }
        }
        time
    }
}

// of racks set, in the order set, each seat's last
fn last_each(racks: Vec<(usize, Vec<u8>)>) -> Vec<(usize, Vec<u8>)> {
    let mut last = Vec::<(usize, Vec<u8>)>::new();
    for (seat, tiles) in racks {
        match last.iter_mut().find(|(s, _)| *s == seat) {
            Some((_, held)) => *held = tiles,
            None => last.push((seat, tiles)),
        }
    }
    last
}

// a game: the bag dealt from its seed, the racks set before its first move (in
// the order set, each seat's last one counting), its moves, how many of them
// the board shows, a rack given to the side to move who has not played yet, and
// the lines what-ifs cut away
#[derive(Clone)]
struct Game {
    seed: u64,
    start: Vec<(usize, Vec<u8>)>,
    turns: Vec<Turn>,
    at: usize,
    rack: Option<Vec<u8>>,
    lines: Vec<Line>, // the games what-ifs cut away, each whole
}

// a game a what-if cut away: the racks set before its first move, and its moves
#[derive(Clone)]
struct Line {
    start: Vec<(usize, Vec<u8>)>,
    turns: Vec<Turn>,
}

impl Game {
    // drop the moves ahead of those shown, keeping the game as it was as a line
    // to go back to: whole, since a later cut can take away the moves the line
    // followed
    fn cut(&mut self) {
        if self.at < self.turns.len() {
            self.lines.push(Line {
                start: self.start.clone(),
                turns: self.turns.clone(),
            });
            self.turns.truncate(self.at);
        }
    }

    // how many first moves the game shares with a line, which is where they
    // part (none, if they start with different racks set)
    fn branch(&self, line: &Line) -> usize {
        if self.start != line.start {
            return 0;
        }
        self.turns
            .iter()
            .zip(&line.turns)
            .take_while(|(ours, theirs)| ours == theirs)
            .count()
    }
}

// who plays a seat: a person whose racks are dealt; a person whose racks are
// given, never dealt (only what is typed in, or what their moves show); or the
// computer, its racks dealt and hidden, playing the generator's best move or
// the best of a sim of so many milliseconds (once the bag is empty, by solving
// the endgame)
#[derive(Clone, Copy, PartialEq)]
enum Seat {
    Human,
    Given,
    Bot(Option<f64>),
}

// a sim of the position shown, run a command at a time: its candidates (the
// move picker's field and any added) and each candidate's stream in the simmer.
// Its budget is one no run reaches, so no cap prunes it: a candidate leaves
// only by sim drop or by losing to the leader beyond any doubt, and sim add
// takes either back.
struct Sim {
    move_generator: movegen::KurniaMoveGenerator,
    simmer: move_picker::Simmer<'static, kwg::Node22, kwg::Node22, true>,
    stream_of: Vec<u64>,
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
    loser: Option<usize>, // the seat that lost the game on a challenge
    move_generator: movegen::KurniaMoveGenerator,
    infer_generator: movegen::KurniaMoveGenerator,
    play_scorer: play_scorer::PlayScorer,
    sim: Option<Sim>, // of the position shown; any change to the game ends it
    seats: [Seat; 2],
    racks_given: [bool; 2], // by seat, whether its current rack is given
    names: [Option<(String, String)>; 2], // a seat's nickname and full name
    challenge_rule: ChallengeRule,
    printing: bool, // false while a file's game is read in
    log: Vec<String>,
    quiet: bool, // no progress printed while a sim or a solve runs
    memory: Option<endgame::MemoryLimit>, // the memory an endgame solve stays within
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
        "rack N TILES",
        "set player N's rack as it is now (- for an empty rack)",
    ),
    (
        "play 8D WORD",
        "the side to move plays WORD across from 8D (D8 for down)",
    ),
    ("play exch TILES", "the side to move exchanges these tiles"),
    ("play pass", "the side to move passes"),
    (
        "phony MOVE",
        "like play, but words not in the lexicon are allowed (and listed)",
    ),
    (
        "challenge [WORD ...]",
        "challenge the last move's words (only those given, if any): a phony is taken back, else the challenge rule applies",
    ),
    (
        "challenges RULE",
        "set the challenge rule: double (the challenger loses a turn), triple (whoever is wrong loses the game), bonus N [word | turn] (N points to the move's player per word or per challenge; the default is bonus 5 turn), or single (no cost, shown as +0)",
    ),
    (
        "withdraw",
        "take back the last move as a phony challenged off",
    ),
    ("bonus N", "give the last move's player N more points"),
    ("note TEXT", "add a note to the last move"),
    (
        "adjust POINTS",
        "the last move scored POINTS more than the board makes it (fewer if below 0): a misscore",
    ),
    (
        "time N POINTS",
        "once the game is over, player N loses POINTS for time",
    ),
    (
        "lost N",
        "player N lost the game on a challenge of the last move",
    ),
    (
        "end",
        "end the game here; the racks left count as at any end",
    ),
    (
        "player N human",
        "a person plays seat N, its racks dealt by the shell and shown (both seats, at first)",
    ),
    (
        "player N given",
        "a person plays seat N, its racks never dealt: only typed in, or shown by its moves",
    ),
    (
        "player N bot [sim MS]",
        "the computer plays seat N, its rack hidden, moving when its turn comes (with sim MS, by a sim of MS ms)",
    ),
    (
        "nickname N NICKNAME [FULL NAME]",
        "player N's nickname and full name in GCG (pN and player N if not given)",
    ),
    (
        "bot",
        "the side to move plays the best move by equity (logged as play)",
    ),
    (
        "bot sim MS",
        "the side to move plays the best move of a sim of MS ms, or of a solved endgame once the bag is empty (logged as play)",
    ),
    ("back [N]", "take back N moves, 1 if not given"),
    ("goto N", "show the game after move N (0 for its start)"),
    (
        "undo",
        "undo the last thing done to the move shown (the moves after it are kept as a line)",
    ),
    ("forward [N]", "replay N moves taken back, 1 if not given"),
    (
        "lines",
        "list the games that moves played after back cut away, and where each parts from this one",
    ),
    (
        "line K",
        "switch to line K (this game is kept as a line), shown where they part",
    ),
    (
        "sim [ITERATIONS]",
        "sim the position shown this many more iterations, 1000 if not given",
    ),
    (
        "sim add MOVE",
        "add MOVE (as play takes it) to the sim as a candidate",
    ),
    ("sim drop MOVE", "take a candidate out of the sim"),
    (
        "sim show [N]",
        "show the sim's N best candidates, 10 if not given",
    ),
    (
        "sim stop",
        "end the sim; any change to the game ends it too",
    ),
    (
        "endgame [SECONDS]",
        "with the bag empty, solve the endgame for the side to move within SECONDS (10 if not given)",
    ),
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
    (
        "gcg save FILE",
        "write the moves shown to FILE, a new GCG file (without the bag order or seed)",
    ),
    (
        "gcg load FILE",
        "start a new game from a GCG file's moves, logged as the commands it runs",
    ),
    (
        "gcg words OUT PATH...",
        "write to OUT, a new file, a line for each word each move of the GCG files (or a directory's .gcg files) formed",
    ),
    ("r, p, k, b", "short for rack, play, gen (kibitz), and show"),
    ("help", "this list"),
    ("exit", "leave the shell"),
];

impl Session {
    pub fn new(quiet: bool, memory: Option<endgame::MemoryLimit>) -> error::Returns<Self> {
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
            loser: None,
            move_generator: movegen::KurniaMoveGenerator::new(game_config),
            infer_generator: movegen::KurniaMoveGenerator::new(game_config),
            play_scorer: play_scorer::PlayScorer::new(),
            sim: None,
            seats: [Seat::Human; 2],
            racks_given: [false; 2],
            names: Default::default(),
            challenge_rule: ChallengeRule::Bonus {
                points: 5,
                per_word: false,
            },
            printing: true,
            log: Vec::new(),
            quiet,
            memory,
        })
    }

    // run one command, its words split from line; after a typed one (not read
    // from a file by source, whose plays are all logged), each bot whose turn
    // it is moves, if the game is at its last move, but not after an undo, or a
    // bot would play again what was just taken back
    pub fn run(&mut self, line: &str, words: &[String], typed: bool) -> error::Returns<()> {
        self.run_one(line, words)?;
        if typed && words[0] != "undo" {
            self.bots_move()?;
        }
        Ok(())
    }

    // while the side to move is the computer's, the game is not over, and its
    // last move is shown, the computer moves, logged as its play
    fn bots_move(&mut self) -> error::Returns<()> {
        loop {
            let Some(game) = &self.game else {
                return Ok(());
            };
            let mover = self.state.turn as usize;
            let Seat::Bot(sim_ms) = self.seats[mover] else {
                return Ok(());
            };
            if self.final_scores.is_some() || game.at != game.turns.len() {
                return Ok(());
            }
            let play = self.computer_move(sim_ms)?;
            let line = format!("play {}", self.play_text(&play)?);
            writeln!(std::io::stdout(), "player {}: {line}", mover + 1)?;
            let words = line
                .split_whitespace()
                .map(String::from)
                .collect::<Vec<_>>();
            self.run_one(&line, &words)?;
        }
    }

    // run one command. A command that changes the session is logged as line
    // (new with the seed it used, a short command as its long form, bot as the
    // move it chose, gcg load as the commands it runs), but sim commands are
    // not (they are analysis, which any change to the game ends); a command
    // that fails changes nothing, except gcg load, which keeps the game it made
    // up to the line that failed
    fn run_one(&mut self, line: &str, words: &[String]) -> error::Returns<()> {
        // short commands run and are logged as their long forms
        let longer = match words[0].as_str() {
            "r" => Some("rack"),
            "p" => Some("play"),
            "k" => Some("gen"),
            "b" => Some("show"),
            _ => None,
        };
        if let Some(longer) = longer {
            let words = std::iter::once(longer.to_string())
                .chain(words[1..].iter().cloned())
                .collect::<Vec<_>>();
            return self.run_one(&words.join(" "), &words);
        }
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
                        start: Vec::new(),
                        turns: Vec::new(),
                        at: 0,
                        rack: None,
                        lines: Vec::new(),
                    });
                    Ok(())
                })?;
                self.show()?;
                format!("new {seed}")
            }
            "rack" => {
                const USAGE: &str = "rack TILES, or rack N TILES, - for none";
                let (seat, tiles) = match args {
                    [tiles] => (None, tiles),
                    [n, tiles] => {
                        let n = n.parse::<usize>().map_err(|_| USAGE)?;
                        if !(1..=self.seats.len()).contains(&n) {
                            return Err(format!("player 1 to {}", self.seats.len()).into());
                        }
                        (Some(n - 1), tiles)
                    }
                    _ => return Err(USAGE.into()),
                };
                let mut rack = Vec::new();
                if tiles != "-" {
                    alphabet::AlphabetReader::new_for_racks(self.game_config.alphabet())
                        .set_word(tiles, &mut rack)?;
                }
                self.type_rack(seat, rack)?;
                line.to_string()
            }
            "play" | "phony" => {
                let phony = words[0] == "phony";
                if self.game.is_some() && self.seats[self.state.turn as usize] == Seat::Given {
                    // what a player whose seat is given held: what they were
                    // known to hold, plus whatever the move needs that was not
                    // there; for exch N, tiles no one saw
                    let (tiles, args, hidden) = self.tiles_needed(args)?;
                    let held = match hidden {
                        true => tiles,
                        false => completed(&self.state.current_player().rack, &tiles),
                    };
                    let before = self.game.clone();
                    self.change(|game| {
                        let game = game.as_mut().ok_or("no game: new first")?;
                        game.cut();
                        game.rack = Some(held);
                        Ok(())
                    })?;
                    if let Err(e) = self
                        .read_play(&args, phony)
                        .and_then(|play| self.push_turn(play, hidden))
                    {
                        self.game = before;
                        self.rebuild()?;
                        return Err(e);
                    }
                } else {
                    let play = self.read_play(args, phony)?;
                    self.push_turn(play, false)?;
                }
                self.show()?;
                line.to_string()
            }
            "end" => {
                // declare the game over here; the shell cannot see the end
                // while a seat given holds tiles it was not told of
                if !args.is_empty() {
                    return Err("end".into());
                }
                if self.final_scores.is_some() {
                    return Err("the game is over".into());
                }
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    game.cut();
                    game.rack = None;
                    let turn = game.turns.last_mut().ok_or("no move yet")?;
                    turn.after.push(After::End);
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "withdraw" | "bonus" | "challenge" => {
                // for a challenge, what the last move's words say: the board
                // before it and the move as played; the words challenged, the
                // ones given (each a word it made), else every word it made;
                // and whether they all stand
                let challenged = if words[0] == "challenge" {
                    let game = self.game.as_ref().ok_or("no game: new first")?;
                    let turn = game.at.checked_sub(1).map(|at| &game.turns[at]);
                    let turn = turn.ok_or("no move to challenge")?;
                    // a move can be challenged only once: not one the game was
                    // lost on, nor one taken back, nor one that stood a
                    // challenge
                    if turn.loser().is_some() {
                        return Err("the game was lost on a challenge".into());
                    }
                    if turn.withdrawn() {
                        return Err("the last move was taken back already".into());
                    }
                    if turn.bonus().is_some() {
                        return Err("the last move stood a challenge already".into());
                    }
                    if !matches!(turn.play, movegen::Play::Place { .. }) {
                        return Err("the last move made no word to challenge".into());
                    }
                    let (board_before, _) = self
                        .board_before_last
                        .as_ref()
                        .ok_or("no move to challenge")?;
                    let board_snapshot = &movegen::BoardSnapshot {
                        board_tiles: board_before,
                        game_config: self.game_config,
                        kwg: self.kwg()?,
                        anagrams: None,
                        rack_lengths: None,
                        klv: self.klv,
                    };
                    let mut made = Vec::<Vec<u8>>::new();
                    self.play_scorer
                        .words_all(board_snapshot, &turn.play, |word: &[u8]| {
                            if !made.iter().any(|w| w == word) {
                                made.push(word.to_vec());
                            }
                            true
                        });
                    let mut invalid = Vec::<Vec<u8>>::new();
                    self.play_scorer.find_invalid_words(
                        board_snapshot,
                        &turn.play,
                        |word: &[u8]| invalid.push(word.to_vec()),
                    );
                    let mut named = Vec::<Vec<u8>>::new();
                    let reader =
                        alphabet::AlphabetReader::new_for_words(self.game_config.alphabet());
                    for arg in args {
                        let mut word = Vec::new();
                        reader.set_word(arg, &mut word)?;
                        if !made.contains(&word) {
                            return Err(format!("{arg}: not a word the last move made").into());
                        }
                        if !named.contains(&word) {
                            named.push(word);
                        }
                    }
                    let counted = if named.is_empty() { &made } else { &named };
                    let stands = counted.iter().all(|word| !invalid.contains(word));
                    Some((stands, counted.len(), named))
                } else {
                    None
                };
                let words_stand = challenged.as_ref().map(|(stands, ..)| *stands);
                let bonus = match (words[0].as_str(), args, &challenged) {
                    ("bonus", [points], _) => Some(points.parse::<i32>()?),
                    ("bonus", _, _) => return Err("bonus POINTS".into()),
                    ("withdraw", [_, ..], _) => return Err("withdraw takes nothing more".into()),
                    (_, _, Some((true, count, _))) => match self.challenge_rule {
                        ChallengeRule::Bonus { points, per_word } => Some(if per_word {
                            points.checked_mul(*count as i32).ok_or("too many points")?
                        } else {
                            points
                        }),
                        ChallengeRule::Double | ChallengeRule::Triple => None,
                    },
                    _ => None,
                };
                // the words given, noted on the move after what the challenge
                // did
                let note = challenged
                    .as_ref()
                    .filter(|(.., named)| !named.is_empty())
                    .map(|(.., named)| {
                        let alphabet = self.game_config.alphabet();
                        let text = |word: &Vec<u8>| {
                            word.iter()
                                .map(|&tile| alphabet.of_board(tile).unwrap_or("?"))
                                .collect::<String>()
                        };
                        format!(
                            "challenged {}",
                            named.iter().map(text).collect::<Vec<_>>().join(" ")
                        )
                    });
                let withdraw = words[0] == "withdraw" || words_stand == Some(false);
                // after the game's last move there is no turn to lose
                let lose_turn = words_stand == Some(true)
                    && self.challenge_rule == ChallengeRule::Double
                    && self.final_scores.is_none();
                // under triple, the challenge ends the game: the challenger
                // (the side to move, counted by the moves made) loses if the
                // move stands, and the move's player loses if it does not
                let made = self.game.as_ref().map_or(0, |game| game.at);
                let seats = self.seats.len();
                let loser = match (words_stand, self.challenge_rule) {
                    (Some(true), ChallengeRule::Triple) => Some(made % seats),
                    (Some(false), ChallengeRule::Triple) => Some((made + 1) % seats),
                    _ => None,
                };
                if let Some(stands) = words_stand {
                    writeln!(
                        std::io::stdout(),
                        "{}",
                        match (stands, self.challenge_rule) {
                            (false, ChallengeRule::Triple) =>
                                "a phony: taken back, and its player loses the game".to_string(),
                            (false, _) => "a phony: taken back".to_string(),
                            (true, ChallengeRule::Double) =>
                                "it stands: the challenger loses the turn".to_string(),
                            (true, ChallengeRule::Bonus { .. }) =>
                                format!("it stands: {:+} to its player", bonus.unwrap_or(0)),
                            (true, ChallengeRule::Triple) =>
                                "it stands: the challenger loses the game".to_string(),
                        }
                    )?;
                }
                let challenger_given = self.seats[self.state.turn as usize] == Seat::Given;
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    let last = game.at.checked_sub(1).ok_or("no move before")?;
                    game.cut();
                    // a rack given to the challenger stays theirs, for the turn
                    // they lose, if they lose it; a game the challenge ends
                    // keeps no rack (it counts none)
                    let pending = game.rack.take();
                    let turn = &mut game.turns[last];
                    if turn.loser().is_some() {
                        return Err("the game was lost on a challenge".into());
                    }
                    if withdraw {
                        if turn.withdrawn() {
                            return Err("the last move was taken back already".into());
                        }
                        if !matches!(turn.play, movegen::Play::Place { .. }) {
                            return Err("only a move that placed tiles is taken back".into());
                        }
                        // taken back, the move ended nothing (the game goes on,
                        // and its time penalties go with it), and the racks set
                        // after it belong to a board that is gone
                        turn.after.push(After::Withdraw);
                    }
                    // a bonus given by hand adds to the move's bonus: a file
                    // may give one move more than one
                    if let Some(points) = bonus {
                        let sum = turn.bonus().unwrap_or(0).checked_add(points);
                        sum.ok_or("too many points")?;
                        turn.after.push(After::Bonus(points));
                    }
                    if let Some(seat) = loser {
                        turn.after.push(After::Lose(seat));
                    }
                    if let Some(note) = note {
                        turn.after.push(After::Note(note));
                    }
                    if lose_turn {
                        game.turns.push(Turn {
                            rack: pending,
                            given: challenger_given,
                            play: movegen::Play::Exchange {
                                tiles: [][..].into(),
                            },
                            hidden: false,
                            after: Vec::new(),
                        });
                        game.at += 1;
                    } else if loser.is_none() {
                        game.rack = pending;
                    }
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "lost" => {
                // what a challenge under triple does, by hand: player N lost
                // the game on a challenge of the last move, with the scores as
                // they are
                const USAGE: &str = "lost N, for player N";
                let [n] = args else {
                    return Err(USAGE.into());
                };
                let n = n.parse::<usize>().map_err(|_| USAGE)?;
                if !(1..=self.seats.len()).contains(&n) {
                    return Err(format!("player 1 to {}", self.seats.len()).into());
                }
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    let last = game.at.checked_sub(1).ok_or("no move before")?;
                    game.cut();
                    game.rack = None;
                    let turn = &mut game.turns[last];
                    if turn.loser().is_some() {
                        return Err("the game was lost on a challenge".into());
                    }
                    turn.after.push(After::Lose(n - 1));
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "nickname" => {
                const USAGE: &str = "nickname N NICKNAME [FULL NAME]";
                let [n, nickname, full @ ..] = args else {
                    return Err(USAGE.into());
                };
                let n = n.parse::<usize>().map_err(|_| USAGE)?;
                if !(1..=self.seats.len()).contains(&n) {
                    return Err(format!("player 1 to {}", self.seats.len()).into());
                }
                // a move's line starts with the nickname, so it must fit there
                if nickname.contains(':') || nickname.starts_with(['#', '>']) {
                    return Err("a nickname cannot hold a colon or start with # or >".into());
                }
                let full = if full.is_empty() {
                    format!("player {n}")
                } else {
                    full.join(" ")
                };
                self.names[n - 1] = Some((nickname.clone(), full));
                line.to_string()
            }
            "time" => {
                const USAGE: &str = "time N POINTS";
                let [n, points] = args else {
                    return Err(USAGE.into());
                };
                let n = n.parse::<usize>().map_err(|_| USAGE)?;
                if !(1..=self.seats.len()).contains(&n) {
                    return Err(format!("player 1 to {}", self.seats.len()).into());
                }
                let points = points.parse::<i32>().map_err(|_| USAGE)?;
                if points <= 0 {
                    return Err("time N POINTS: POINTS above 0".into());
                }
                if self.final_scores.is_none() {
                    return Err("a time penalty comes only once the game is over".into());
                }
                // kept with the move that ended the game, the last one shown
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    let last = game.at.checked_sub(1).ok_or("no move before")?;
                    game.cut();
                    let turn = &mut game.turns[last];
                    turn.time()[n - 1]
                        .checked_add(points)
                        .ok_or("too many points")?;
                    turn.after.push(After::Time(n - 1, points));
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "adjust" => {
                const USAGE: &str = "adjust POINTS (below 0 for fewer)";
                let [points] = args else {
                    return Err(USAGE.into());
                };
                let points = points.parse::<i32>().map_err(|_| USAGE)?;
                if points == 0 {
                    return Err("adjust POINTS: POINTS not 0".into());
                }
                // kept apart from the board's score, on the move it applies to
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    let last = game.at.checked_sub(1).ok_or("no move before")?;
                    game.cut();
                    let turn = &mut game.turns[last];
                    if !matches!(turn.play, movegen::Play::Place { .. }) || turn.withdrawn() {
                        return Err("only a move on the board can be misscored".into());
                    }
                    turn.adjust().checked_add(points).ok_or("too many points")?;
                    turn.after.push(After::Adjust(points));
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "note" => {
                if args.is_empty() {
                    return Err("note TEXT".into());
                }
                let text = args.join(" ");
                // a note that gcg load would read back as the shell's own (a
                // move's correct score) is refused
                let read = gcg_moves(&format!("#note {text}"));
                if !matches!(read.as_deref(), Ok([GcgMove { kind: GcgKind::Note(back), .. }]) if *back == text)
                {
                    return Err("that note would not read back from GCG as itself".into());
                }
                // on the move shown, after what was done to it so far
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    let last = game.at.checked_sub(1).ok_or("no move to note")?;
                    game.cut();
                    game.turns[last].after.push(After::Note(text));
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "lines" => {
                let game = self.game.as_ref().ok_or("no game: new first")?;
                let mut out = std::io::stdout();
                if game.lines.is_empty() {
                    writeln!(
                        out,
                        "no other lines: a move played after back keeps the game it cuts away as a line"
                    )?;
                }
                for (k, other) in game.lines.iter().enumerate() {
                    let branch = game.branch(other);
                    let ahead = other.turns.len() - branch;
                    writeln!(
                        out,
                        "{:>3}. after move {}, {} move{}{}",
                        k + 1,
                        branch,
                        ahead,
                        if ahead == 1 { "" } else { "s" },
                        if other.start == game.start {
                            ""
                        } else {
                            ", other racks set before the first"
                        }
                    )?;
                }
                return Ok(());
            }
            "line" => {
                let [k] = args else {
                    return Err("line K".into());
                };
                let k = k.parse::<usize>()?;
                // put the line in place of the game, which is kept as line K in
                // turn unless the line holds all of it, and show the board
                // where they part
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    if game.lines.is_empty() {
                        return Err("no other lines".into());
                    }
                    if k == 0 || k > game.lines.len() {
                        return Err(format!("line 1 to {}", game.lines.len()).into());
                    }
                    let other = game.lines.remove(k - 1);
                    let branch = game.branch(&other);
                    if other.start != game.start || !other.turns.starts_with(&game.turns) {
                        let kept = Line {
                            start: std::mem::take(&mut game.start),
                            turns: std::mem::take(&mut game.turns),
                        };
                        game.lines.insert(k - 1, kept);
                    }
                    game.start = other.start;
                    game.turns = other.turns;
                    game.at = branch;
                    game.rack = None;
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "challenges" => {
                const USAGE: &str =
                    "challenges double, triple, bonus POINTS [word | turn], or single";
                let args = args.iter().map(String::as_str).collect::<Vec<_>>();
                self.challenge_rule = match args.as_slice() {
                    ["double"] => ChallengeRule::Double,
                    ["triple"] => ChallengeRule::Triple,
                    // no cost for a word that stands, and a phony not
                    // challenged stays
                    ["single"] => ChallengeRule::Bonus {
                        points: 0,
                        per_word: false,
                    },
                    ["bonus", points, per @ ..] => ChallengeRule::Bonus {
                        points: points.parse().map_err(|_| USAGE)?,
                        per_word: match per {
                            [] | ["turn"] => false,
                            ["word"] => true,
                            _ => return Err(USAGE.into()),
                        },
                    },
                    _ => return Err(USAGE.into()),
                };
                line.to_string()
            }
            "player" => {
                const USAGE: &str =
                    "player N human, player N given, player N bot, or player N bot sim MS";
                let [n, kind @ ..] = args else {
                    return Err(USAGE.into());
                };
                let n = n.parse::<usize>().map_err(|_| USAGE)?;
                if !(1..=self.seats.len()).contains(&n) {
                    return Err(format!("player 1 to {}", self.seats.len()).into());
                }
                let seat = match kind {
                    [kind] if kind == "human" => Seat::Human,
                    [kind] if kind == "given" => Seat::Given,
                    [kind] if kind == "bot" => Seat::Bot(None),
                    [kind, sim, ms] if kind == "bot" && sim == "sim" => {
                        let ms = ms.parse::<f64>()?;
                        let usable = ms.is_finite() && ms > 0.0;
                        if !usable {
                            return Err("bot sim MS: MS above 0".into());
                        }
                        Seat::Bot(Some(ms))
                    }
                    _ => return Err(USAGE.into()),
                };
                // rebuild the game as that seat deals it: a seat given is dealt
                // nothing
                let before = std::mem::replace(&mut self.seats[n - 1], seat);
                if let Err(e) = self.rebuild() {
                    self.seats[n - 1] = before;
                    self.rebuild()?;
                    return Err(e);
                }
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
            "goto" => {
                let [n] = args else {
                    return Err("goto N".into());
                };
                let n = n.parse::<usize>()?;
                // the end of move N, the start of move N + 1
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    if n > game.turns.len() {
                        return Err(format!("goto 0 to {}", game.turns.len()).into());
                    }
                    game.at = n;
                    game.rack = None;
                    Ok(())
                })?;
                self.show()?;
                line.to_string()
            }
            "undo" => {
                if !args.is_empty() {
                    return Err("undo takes nothing more".into());
                }
                // undo the last thing done at the move shown: a rack given to
                // the side to move and not yet played from; else the last thing
                // done to the move after it was played; else the move itself,
                // its turn gone and the rack it was given given again; else, at
                // the start, the last rack set there. The moves after the one
                // shown are kept as a line
                self.change(|game| {
                    let game = game.as_mut().ok_or("no game: new first")?;
                    game.cut();
                    if game.rack.take().is_some() {
                        return Ok(());
                    }
                    let Some(last) = game.at.checked_sub(1) else {
                        return game.start.pop().map(drop).ok_or("nothing to undo".into());
                    };
                    if game.turns[last].after.pop().is_none() {
                        let turn = game.turns.pop().ok_or("nothing to undo")?;
                        game.at = last;
                        game.rack = turn.rack;
                    }
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
                return self.run_one(&line, &words);
            }
            "endgame" => {
                let seconds = match args {
                    [] => DEFAULT_ENDGAME_SECONDS,
                    [seconds] => seconds.parse::<f64>()?,
                    _ => return Err("endgame [SECONDS]".into()),
                };
                let usable = seconds.is_finite() && seconds > 0.0;
                if !usable {
                    return Err("endgame SECONDS: SECONDS above 0".into());
                }
                if self.game.is_none() {
                    return Err("no game: new first".into());
                }
                if self.final_scores.is_some() {
                    return Err("the game is over".into());
                }
                let kwg = self.kwg()?;
                let arm_tables = self.arm_tables()?;
                let state = self.endgame_state();
                let (mut solver, solved) = solve_endgame(
                    self.game_config,
                    kwg,
                    arm_tables,
                    &state,
                    &endgame::Limits {
                        time: Some(std::time::Duration::from_secs_f64(seconds)),
                        states: None,
                        memory: self.memory,
                    },
                    self.quiet,
                )?;
                writeln!(
                    std::io::stdout(),
                    "the side to move gains {} on the spread from here, {}",
                    solved.value,
                    if solved.exact {
                        "exactly".to_string()
                    } else {
                        format!("as {} moves deep saw it, cut short", solved.depth)
                    }
                )?;
                for line in self.endgame_line(&mut solver, &state)? {
                    writeln!(std::io::stdout(), "{line}")?;
                }
                return Ok(());
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
            "gcg" => {
                return match args {
                    [save, path] if save == "save" => {
                        let text = self.gcg_text()?;
                        let mut file = std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(path)?;
                        file.write_all(text.as_bytes())?;
                        Ok(())
                    }
                    [load, path] if load == "load" => {
                        let moves = gcg_moves(&gcg_decode(&std::fs::read(path)?)?)?;
                        self.load_moves(&moves)
                    }
                    [words, out, paths @ ..] if words == "words" && !paths.is_empty() => {
                        self.gcg_words(out, paths)
                    }
                    _ => Err("gcg save FILE, gcg load FILE, or gcg words OUT PATH...".into()),
                };
            }
            "sim" => {
                const USAGE: &str =
                    "sim [ITERATIONS], sim add MOVE, sim drop MOVE, sim show [N], or sim stop";
                match args.first().map(String::as_str) {
                    None => self.sim_run(DEFAULT_SIM_ITERATIONS)?,
                    Some("add") => {
                        let play = self.read_play(&args[1..], false)?;
                        self.sim_add(play)?;
                    }
                    Some("drop") => {
                        let play = self.read_play(&args[1..], false)?;
                        self.sim_drop(&play)?;
                    }
                    Some("show") => {
                        let n = match &args[1..] {
                            [] => 10,
                            [n] => n.parse::<usize>()?,
                            _ => return Err(USAGE.into()),
                        };
                        self.sim_show(n)?;
                    }
                    Some("stop") if args.len() == 1 => self.sim = None,
                    Some(iterations) if args.len() == 1 => {
                        let iterations = iterations.parse::<u64>().map_err(|_| USAGE)?;
                        self.sim_run(iterations)?;
                    }
                    _ => return Err(USAGE.into()),
                }
                return Ok(());
            }
            _ => return Err("no such command, help for the commands".into()),
        };
        self.log.push(logged);
        self.sim = None;
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
    // otherwise the game and its state stay as they were. A line that the game
    // now holds all of is dropped.
    fn change(
        &mut self,
        f: impl FnOnce(&mut Option<Game>) -> error::Returns<()>,
    ) -> error::Returns<()> {
        let before = self.game.clone();
        let result = f(&mut self.game).and_then(|()| {
            if let Some(game) = &mut self.game {
                let (start, turns) = (&game.start, &game.turns);
                game.lines
                    .retain(|line| line.start != *start || !turns.starts_with(&line.turns));
            }
            self.rebuild()
        });
        if result.is_err() {
            self.game = before;
            self.rebuild()?;
        }
        result
    }

    // play a move after the ones shown; the moves ahead are cut away and kept
    // as a line; hidden is true for an exchange of tiles no one saw
    fn push_turn(&mut self, play: movegen::Play, hidden: bool) -> error::Returns<()> {
        let given = self.seats[self.state.turn as usize] == Seat::Given;
        self.change(|game| {
            let game = game.as_mut().ok_or("no game: new first")?;
            game.cut();
            game.turns.push(Turn {
                rack: game.rack.take(),
                given,
                play,
                hidden,
                after: Vec::new(),
            });
            game.at += 1;
            Ok(())
        })
    }

    // set a rack: the side to move's, for their move, while the game goes on;
    // anyone else's (or anyone's once the game is over), as it is after the
    // moves shown (or before the first move, at the start). Set again, the rack
    // is these tiles. A game lost on a challenge counts no rack.
    fn type_rack(&mut self, seat: Option<usize>, tiles: Vec<u8>) -> error::Returns<()> {
        if self.loser.is_some() {
            return Err("the game was lost on a challenge: no rack counts".into());
        }
        let to_move = self.state.turn as usize;
        let seat = seat.unwrap_or(to_move);
        let going = self.final_scores.is_none();
        self.change(|game| {
            let game = game.as_mut().ok_or("no game: new first")?;
            game.cut();
            if seat == to_move && going {
                game.rack = Some(tiles);
                return Ok(());
            }
            match game.turns.last_mut() {
                Some(turn) => turn.after.push(After::Rack(seat, tiles)),
                None => game.start.push((seat, tiles)),
            }
            Ok(())
        })
    }

    // for a move as typed: the tiles it takes from its mover's rack, the move
    // to read, and whether those tiles are ones no one saw. A place move takes
    // its placed tiles (a blank as ?), and an exchange its tiles; exch N takes
    // any N from the bag, which the exchange puts back, and is read as exch
    // with those tiles.
    fn tiles_needed(&self, args: &[String]) -> error::Returns<(Vec<u8>, Vec<String>, bool)> {
        let alphabet = self.game_config.alphabet();
        let mut tiles = Vec::new();
        let mut args = args.to_vec();
        let mut hidden = false;
        match args.as_slice() {
            [exch, n] if exch == "exch" && n.bytes().all(|b| b.is_ascii_digit()) => {
                let n = n.parse::<usize>()?;
                let bag = self.state.bag.as_slice();
                if n == 0 || n > bag.len() {
                    return Err(format!("exch 1 to {} tiles", bag.len()).into());
                }
                tiles = bag[..n].to_vec();
                args[1] = alphabet.fmt_rack(&tiles).to_string();
                hidden = true;
            }
            [exch, text] if exch == "exch" => {
                alphabet::AlphabetReader::new_for_racks(alphabet).set_word(text, &mut tiles)?;
            }
            [coord, word] if coord != "exch" => {
                let dim = self.game_config.board_layout().dim();
                let coord = notation::parse_coord(coord, dim).ok_or("no such coordinate")?;
                notation::read_place_word(
                    &alphabet::AlphabetReader::new_for_plays(alphabet),
                    &self.state.board_tiles,
                    dim,
                    &coord,
                    word,
                    &mut tiles,
                )?;
                tiles.retain(|&tile| tile != 0);
                for tile in tiles.iter_mut() {
                    if *tile & 0x80 != 0 {
                        *tile = 0;
                    }
                }
            }
            _ => {}
        }
        Ok((tiles, args, hidden))
    }

    // the game as dealt from its seed, except that a seat given (as its first
    // move says, or before that move, as the seat is) is dealt nothing and its
    // tiles stay in the bag; then the racks set before the first move. The
    // seats take turns, player 1 first.
    fn dealt(&self, game: &Game) -> error::Returns<(Replay, GameRng)> {
        let mut rng = GameRng::seed_from_u64(game.seed);
        let mut state = game_state::GameState::new(self.game_config);
        state.reset_and_draw_tiles_double_ended(self.game_config, &mut rng);
        let mut given = [false; 2];
        for (seat, given) in given.iter_mut().enumerate() {
            *given = match game.turns.get(seat) {
                Some(turn) => turn.given,
                None => self.seats[seat] == Seat::Given,
            };
            if *given {
                let rack = std::mem::take(&mut state.players[seat].rack);
                state.bag.return_tiles(&rack);
            }
        }
        let mut replay = Replay {
            state,
            given,
            typed: Default::default(),
        };
        replay.type_racks(&last_each(game.start.clone()))?;
        Ok((replay, rng))
    }

    // rebuild the state after the moves shown from the game's seed, with the
    // rack given to the side to move. Their next move goes by their seat: a
    // seat dealt draws a full rack; a seat given holds only what is known of
    // its rack (a rack dealt before keeps only what was typed of it)
    fn rebuild(&mut self) -> error::Returns<()> {
        let Some(game) = &self.game else {
            return Ok(());
        };
        let (mut replay, mut rng) = self.dealt(game)?;
        let mut final_scores = None;
        let mut board_before_last = None;
        let mut loser = None;
        for turn in game.turns[..game.at].iter() {
            if final_scores.is_some() {
                return Err("the game is over".into());
            }
            let held = replay.ready_rack(self.game_config, turn)?;
            let board_tiles = replay.state.board_tiles.to_vec();
            board_before_last = Some((board_tiles, turn.played().into_owned()));
            replay.play_turn(self.game_config, &mut rng, turn)?;
            final_scores = replay.end_of_turn(self.game_config, turn, held)?;
            loser = turn.loser();
            if final_scores.is_none() {
                replay.state.next_turn();
            }
        }
        if final_scores.is_none() {
            let to_move = replay.state.turn as usize;
            let given = self.seats[to_move] == Seat::Given;
            if given && !replay.given[to_move] {
                replay.forget_drawn(to_move);
            }
            replay.given[to_move] = given;
            match &game.rack {
                Some(rack) => replay.give_rack(rack)?,
                None if !given => top_up(self.game_config, &mut replay.state, to_move),
                None => {}
            }
        } else if game.rack.is_some() {
            return Err("the game is over".into());
        }
        self.racks_given = replay.given;
        self.state = replay.state;
        self.board_before_last = board_before_last;
        self.final_scores = final_scores;
        self.loser = loser;
        Ok(())
    }

    // read a move as typed (pass, an exchange, or a coordinate and a word),
    // check it against the board, the rack, and (unless it may be a phony) the
    // lexicon, and score it
    fn read_play(&mut self, args: &[String], phony: bool) -> error::Returns<movegen::Play> {
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
            let words = phonies.join(", ");
            if !phony {
                return Err(format!("not in the lexicon: {words} (phony plays it anyway)").into());
            }
            writeln!(std::io::stdout(), "not in the lexicon: {words}")?;
        }
        Ok(play)
    }

    fn show(&self) -> error::Returns<()> {
        if !self.printing {
            return Ok(());
        }
        let mut out = std::io::stdout();
        let Some(game) = &self.game else {
            writeln!(out, "no game: new to start one")?;
            return Ok(());
        };
        let alphabet = self.game_config.alphabet();
        display::print_board(
            alphabet,
            self.game_config.board_layout(),
            &self.state.board_tiles,
        )?;
        // each player's score and rack, the computer's hidden as _s (a seat
        // given shows what it is known to hold); and, if the side to move is a
        // person, the tiles they have not seen
        let turn = self.state.turn as usize;
        for (i, player) in self.state.players.iter().enumerate() {
            writeln!(
                out,
                "player {} ({}): {}{}{}",
                i + 1,
                match self.seats[i] {
                    Seat::Human => "human".to_string(),
                    Seat::Given => "given".to_string(),
                    Seat::Bot(None) => "bot".to_string(),
                    Seat::Bot(Some(ms)) => format!("bot sim {ms}"),
                },
                equity::descale_score(player.score),
                match self.rack_text(i, &player.rack) {
                    rack if rack.is_empty() => rack,
                    rack => format!(" {rack}"),
                },
                if i == turn && self.final_scores.is_none() {
                    " (to move)"
                } else {
                    ""
                },
            )?;
        }
        if !matches!(self.seats[turn], Seat::Bot(_)) {
            let mut unseen = self.state.bag.as_slice().to_vec();
            for (i, player) in self.state.players.iter().enumerate() {
                if i != turn {
                    unseen.extend_from_slice(&player.rack);
                }
            }
            unseen.sort_unstable();
            writeln!(
                out,
                "unseen by player {}: {} {}",
                turn + 1,
                unseen.len(),
                alphabet.fmt_rack(&unseen)
            )?;
        }
        // each move with the board before it, so that a tile played through
        // shows, and what a challenge did to it (taken back, or a bonus); the
        // moves after those shown are marked ~
        if let Some((_, kwg)) = self.kwg {
            let (mut replay, mut rng) = self.dealt(game)?;
            for (i, turn) in game.turns.iter().enumerate() {
                let held = replay.ready_rack(self.game_config, turn)?;
                let board = &mut replay.state;
                let mover = board.turn as usize;
                let rack = self.rack_text(mover, &board.players[mover].rack);
                let text = match &turn.play {
                    // an exchange of tiles no one saw shows only how many
                    movegen::Play::Exchange { tiles } if turn.hidden => {
                        format!("Exch. {}", "_".repeat(tiles.len()))
                    }
                    play => play
                        .fmt(&movegen::BoardSnapshot {
                            board_tiles: &board.board_tiles,
                            game_config: self.game_config,
                            kwg,
                            anagrams: None,
                            rack_lengths: None,
                            klv: self.klv,
                        })
                        .to_string(),
                } + if turn.withdrawn() { ", taken back" } else { "" };
                let text = match turn.bonus() {
                    Some(bonus) => format!("{text}, {bonus:+} after a challenge"),
                    None => text,
                };
                // a misscore: what it was scored as
                let text = match &turn.play {
                    movegen::Play::Place { score, .. } if turn.adjust() != 0 => format!(
                        "{text}, scored as {}",
                        equity::descale_score(*score) + turn.adjust()
                    ),
                    _ => text,
                };
                replay.play_turn(self.game_config, &mut rng, turn)?;
                replay.end_of_turn(self.game_config, turn, held)?;
                writeln!(
                    out,
                    "{}{:>3}. player {} {:<7} {} (total {})",
                    if i < game.at { " " } else { "~" },
                    i + 1,
                    mover + 1,
                    rack,
                    text,
                    equity::descale_score(replay.state.players[mover].score),
                )?;
                // its notes, under it
                for after in &turn.after {
                    if let After::Note(note) = after {
                        writeln!(out, "      note: {note}")?;
                    }
                }
                replay.state.next_turn();
            }
        }
        if let Some(final_scores) = &self.final_scores {
            // what time cost each player, shown in the scores
            let time = game
                .at
                .checked_sub(1)
                .map_or([0; 2], |at| game.turns[at].time());
            writeln!(
                out,
                "game over, {}{}{}",
                final_scores
                    .iter()
                    .map(|&s| equity::descale_score(s).to_string())
                    .collect::<Vec<_>>()
                    .join(" to "),
                match self.loser {
                    Some(seat) => format!(", and player {} lost on a challenge", seat + 1),
                    None => String::new(),
                },
                time.iter()
                    .enumerate()
                    .filter(|&(_, &lost)| lost != 0)
                    .map(|(seat, lost)| format!(", player {} -{lost} for time", seat + 1))
                    .collect::<String>()
            )?;
        }
        Ok(())
    }

    // a seat's rack as show prints it: a person's as its tiles (? for a blank),
    // the computer's as an _ for each tile
    fn rack_text(&self, seat: usize, rack: &[u8]) -> String {
        match self.seats[seat] {
            Seat::Bot(_) => "_".repeat(rack.len()),
            _ => self.game_config.alphabet().fmt_rack(rack).to_string(),
        }
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
    // (drawing the opponent's rack from what their last move says they kept),
    // or once the bag is empty, the solved endgame's
    fn computer_move(&mut self, sim_ms: Option<f64>) -> error::Returns<movegen::Play> {
        if self.game.is_none() {
            return Err("no game: new first".into());
        }
        if self.final_scores.is_some() {
            return Err("the game is over".into());
        }
        let kwg = self.kwg()?;
        let arm_tables = self.arm_tables()?;
        // with the bag empty, both racks are known (a given seat's rack is all
        // the unseen tiles, once they fit in it), so solve rather than sim; the
        // solve reads no kept leaves, so none are inferred before it
        if let Some(ms) = sim_ms {
            let endgame_state = self.endgame_state();
            if endgame_state.bag.is_empty() {
                let (mut solver, _) = solve_endgame(
                    self.game_config,
                    kwg,
                    arm_tables,
                    &endgame_state,
                    &endgame::Limits {
                        time: Some(std::time::Duration::from_secs_f64(ms / 1000.0)),
                        states: None,
                        memory: self.memory,
                    },
                    self.quiet,
                )?;
                let mut line = Vec::new();
                solver.collect_pv(endgame_state.turn, &mut line);
                return Ok(line.first().ok_or("the solve found no move")?.1.clone());
            }
        }
        let leaves = match sim_ms {
            Some(_) => self.kept_leaves()?,
            None => Vec::new(),
        };
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
        let state = self.filled_state();
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
        simmer.reseed(self.position_seed());
        // run a few iterations first, short of the first look, to get a rate to
        // plan by; then step up to the next look each time, so the threads
        // share whole blocks and every look is planned; unless quiet, print a
        // line a second
        const FIRST: u64 = 4;
        let num_candidates = self.move_generator.plays.len();
        let mut rollouts = (FIRST as usize * num_candidates) as f64;
        let mut iterations = FIRST;
        let mut ticked = 0;
        simmer.begin_decision(&self.move_generator, &state, FIRST, &leaves)?;
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

    // the position shown with each other seat's rack topped up from the bag to
    // full (never past full; a rack given holds only what is known): a sim
    // draws the opponent's rack again each iteration, as many tiles as it finds
    // there
    fn filled_state(&self) -> game_state::GameState {
        let mut state = self.state.clone();
        let to_move = state.turn as usize;
        for seat in 0..self.seats.len() {
            if seat != to_move {
                top_up(self.game_config, &mut state, seat);
            }
        }
        state
    }

    // the position shown, for an endgame: if only one rack is given, it is
    // topped up from the bag to full (never past full), so that once the bag's
    // tiles fit it, all the unseen tiles are theirs; with both racks given,
    // whose the bag's tiles are is not known, and they stay in the bag
    fn endgame_state(&self) -> game_state::GameState {
        let mut state = self.state.clone();
        let given = self.racks_given;
        if let [seat] = (0..given.len())
            .filter(|&seat| given[seat])
            .collect::<Vec<_>>()[..]
        {
            top_up(self.game_config, &mut state, seat);
        }
        state
    }

    // a sim's seed for the position shown: the game's, plus how many moves the
    // board shows, so a sim of the same position repeats (the simmer scrambles
    // its seed for each iteration)
    fn position_seed(&self) -> u64 {
        self.game
            .as_ref()
            .map_or(0, |game| game.seed.wrapping_add(game.at as u64))
    }

    // the leaves the opponent's last move says they kept, each with its weight,
    // for a sim to draw their rack from; none before the first move
    fn kept_leaves(&mut self) -> error::Returns<Vec<(Vec<u8>, f64)>> {
        let Some((board_before, last)) = &self.board_before_last else {
            return Ok(Vec::new());
        };
        let kwg = self.kwg()?;
        let tables = match &mut self.infer_tables {
            Some(tables) => tables,
            tables @ None => tables.insert(infer::InferTables::new(self.game_config, self.klv)?),
        };
        infer::kept_leaves(
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
            &self.state.current_player().rack,
            last,
            infer::FUZZ,
        )
    }

    // the sim of the position shown, started if there is none, with the move
    // picker's field, the opponent's rack drawn as bot sim draws it, every
    // core, and the position's own seed, so that a sim of it repeats
    fn sim_mut(&mut self) -> error::Returns<&mut Sim> {
        if self.sim.is_none() {
            if self.game.is_none() {
                return Err("no game: new first".into());
            }
            if self.final_scores.is_some() {
                return Err("the game is over".into());
            }
            let kwg = self.kwg()?;
            let arm_tables = self.arm_tables()?;
            let leaves = self.kept_leaves()?;
            let mut move_generator = movegen::KurniaMoveGenerator::new(self.game_config);
            move_picker::gen_simmer_candidates(
                &mut move_filter::GenMoves::Unfiltered,
                &mut move_generator,
                &movegen::BoardSnapshot {
                    board_tiles: &self.state.board_tiles,
                    game_config: self.game_config,
                    kwg,
                    anagrams: arm_tables.anagrams(),
                    rack_lengths: arm_tables.rack_lengths(),
                    klv: self.klv,
                },
                &self.state.current_player().rack,
                self.state.current_player().num_exchanges,
                move_picker::SIMMER_FIELD,
                &mut Vec::new(),
            );
            let mut simmer = move_picker::Simmer::<_, _, true>::new(
                self.game_config,
                kwg,
                arm_tables,
                self.klv,
                move_picker::SimmerParams {
                    num_sim_iters: u64::MAX / 2,
                    num_sim_plies: move_picker::DEFAULT_NUM_SIM_PLIES,
                    sim_threads: parallelism::available(),
                    win_pct_table: None,
                    quiet: true, // the shell prints its own lines
                },
            )?;
            simmer.reseed(self.position_seed());
            simmer.begin_decision(&move_generator, &self.filled_state(), 0, &leaves)?;
            let stream_of = (0..move_generator.plays.len() as u64).collect();
            self.sim = Some(Sim {
                move_generator,
                simmer,
                stream_of,
            });
        }
        Ok(self.sim.as_mut().unwrap())
    }

    // run the sim this many more iterations, or until one candidate is left, a
    // step up to each look at a time (its rows are the same however the run is
    // split), printing a line a second unless quiet
    fn sim_run(&mut self, iterations: u64) -> error::Returns<()> {
        let t0 = std::time::Instant::now();
        let quiet = self.quiet;
        let Sim {
            move_generator,
            simmer,
            ..
        } = self.sim_mut()?;
        let mut done = 0;
        let mut ticked = 0;
        while done < iterations && simmer.active_stream_ids().nth(1).is_some() {
            let step = simmer.iters_to_next_look().min(iterations - done);
            simmer.resume(move_generator, step)?;
            done += step;
            let secs = t0.elapsed().as_secs();
            if !quiet && secs > ticked {
                ticked = secs;
                writeln!(
                    std::io::stderr(),
                    "sim: {done} of {iterations} more iterations, {} left in, {secs} s",
                    simmer.active_stream_ids().count()
                )?;
            }
        }
        writeln!(
            std::io::stdout(),
            "sim: {iterations} more iterations, {:.0} ms",
            t0.elapsed().as_secs_f64() * 1000.0
        )?;
        self.sim_show(10)
    }

    // make a move a candidate: a new one is rolled out from the next iteration,
    // and one that is out (dropped, or retired by the sim) comes back with what
    // it had; one already in is refused, ending the sim if it was started for
    // this
    fn sim_add(&mut self, play: movegen::Play) -> error::Returns<()> {
        let kwg = self.kwg()?;
        let score = match &play {
            movegen::Play::Place { score, .. } => *score,
            movegen::Play::Exchange { .. } => 0,
        };
        let equity = self.play_scorer.compute_equity(
            &movegen::BoardSnapshot {
                board_tiles: &self.state.board_tiles,
                game_config: self.game_config,
                kwg,
                anagrams: None,
                rack_lengths: None,
                klv: self.klv,
            },
            &self.state,
            &play,
            move_filter::LEAVE_SCALE_DENOM,
            score,
        );
        let begun = self.sim.is_none();
        let sim = self.sim_mut()?;
        match sim.move_generator.plays.iter().position(|v| v.play == play) {
            Some(index) => {
                let stream = sim.stream_of[index];
                if !sim.simmer.readmit_with_history(stream) {
                    if begun {
                        self.sim = None;
                    }
                    return Err("already a candidate".into());
                }
            }
            None => {
                sim.move_generator.plays.push(movegen::ValuedMove {
                    equity: equity::Equity::new(equity),
                    play,
                });
                let stream = sim.simmer.add_play(sim.move_generator.plays.len() - 1);
                sim.stream_of.push(stream);
            }
        }
        Ok(())
    }

    fn sim_drop(&mut self, play: &movegen::Play) -> error::Returns<()> {
        let sim = self.sim.as_mut().ok_or("no sim: sim first")?;
        let index = sim
            .move_generator
            .plays
            .iter()
            .position(|v| v.play == *play)
            .ok_or("not a candidate")?;
        if !sim.simmer.retire_stream(sim.stream_of[index]) {
            return Err("not in the sim, or the last candidate in it".into());
        }
        Ok(())
    }

    // list the sim's candidates, best mean first: the move, its means (win
    // chance, and the change in spread in points), its rollouts, and whether it
    // is still in
    fn sim_show(&self, n: usize) -> error::Returns<()> {
        let sim = self.sim.as_ref().ok_or("no sim: sim first")?;
        let board_snapshot = &movegen::BoardSnapshot {
            board_tiles: &self.state.board_tiles,
            game_config: self.game_config,
            kwg: self.kwg()?,
            anagrams: None,
            rack_lengths: None,
            klv: self.klv,
        };
        let active = sim.simmer.active_stream_ids().collect::<Vec<_>>();
        let mut out = std::io::stdout();
        for (rank, (index, _objective, equity, win_rate)) in
            sim.simmer.leaderboard(n).into_iter().enumerate()
        {
            let stream = sim.stream_of[index];
            writeln!(
                out,
                "{:>3}. {:<24} win {:>6.2}% spread {:>+8.2} rollouts {:>7} {}",
                rank + 1,
                sim.move_generator.plays[index]
                    .play
                    .fmt(board_snapshot)
                    .to_string(),
                100.0 * win_rate,
                equity,
                sim.simmer.stream_count(stream).unwrap_or(0.0),
                if active.contains(&stream) {
                    "in"
                } else {
                    "out"
                },
            )?;
        }
        Ok(())
    }

    // run one command as if read from a file: no bot moves after it
    fn run_line(&mut self, line: &str) -> error::Returns<()> {
        let words = line
            .split_whitespace()
            .map(String::from)
            .collect::<Vec<_>>();
        self.run_one(line, &words)
    }

    // make a new game of a GCG file's moves, from a fresh seed: its lines are
    // run and logged as the commands they mean, a play as phony (a word not in
    // the lexicon stands until challenged), each checked against the score and
    // total the file gives; a seat is given if its rack shows an _ on any line
    // or it has no line, and not given otherwise; then the racks it gives as
    // they stand. It stops at the first line that fails, keeping the game made
    // so far
    fn load_moves(&mut self, moves: &[GcgMove]) -> error::Returns<()> {
        // a lexicon other than the session's is noted; its words are read as
        // phonies
        for gcg_move in moves {
            if let GcgKind::Lexicon(lexicon) = &gcg_move.kind
                && self.lexicon_name() != Some(lexicon.as_str())
            {
                writeln!(
                    std::io::stdout(),
                    "the file's lexicon is {lexicon}, the session's {}",
                    self.lexicon_name().unwrap_or("none")
                )?;
            }
        }
        let seed = GameRng::try_from_rng(&mut rand::rngs::SysRng)?.next_u64();
        // show the game once it is read in, unless the session prints nothing
        // (as in the sessions gcg words reads files into)
        let printing = self.printing;
        self.printing = false;
        let loaded = (|| -> error::Returns<()> {
            self.run_line(&format!("new {seed}"))?;
            // the players' nicknames and full names, as the file gives them
            for gcg_move in moves {
                if let GcgKind::Player(nickname, full) = &gcg_move.kind {
                    let line = format!("nickname {} {nickname} {full}", gcg_move.seat + 1);
                    self.run_line(line.trim_end())?;
                }
            }
            let mut lines = vec![false; self.seats.len()];
            let mut unseen = vec![false; self.seats.len()];
            for gcg_move in moves {
                let seat = gcg_move.seat;
                if seat >= self.seats.len() {
                    return Err(format!(
                        "line {}: player 1 to {}",
                        gcg_move.line_number,
                        self.seats.len()
                    )
                    .into());
                }
                // a line that shows its player's rack (not #incomplete, whose
                // player is the side to move, known only once the moves are
                // made)
                if matches!(
                    gcg_move.kind,
                    GcgKind::Play(_)
                        | GcgKind::Withdraw
                        | GcgKind::Bonus(_)
                        | GcgKind::Out
                        | GcgKind::Shown
                        | GcgKind::Rack
                        | GcgKind::Time(_)
                ) {
                    lines[seat] = true;
                }
                if matches!(
                    gcg_move.kind,
                    GcgKind::Play(_) | GcgKind::Rack | GcgKind::Time(_)
                ) {
                    unseen[seat] |= gcg_move.rack.contains('_');
                }
            }
            for (seat, (&lines, &unseen)) in lines.iter().zip(&unseen).enumerate() {
                let given = unseen || !lines;
                if given != (self.seats[seat] == Seat::Given) {
                    let kind = if given { "given" } else { "human" };
                    self.run_line(&format!("player {} {kind}", seat + 1))?;
                }
            }
            let mut standing = Vec::new();
            let seats = self.seats.len();
            for gcg_move in moves {
                let line_number = gcg_move.line_number;
                // a move belongs to the side to move; a phony taken back or a
                // challenge's bonus belongs to the last mover (counted by the
                // moves made, since a game ended by its last move leaves that
                // mover to move)
                let made = self.game.as_ref().map_or(0, |game| game.at);
                let (seat, what) = match &gcg_move.kind {
                    GcgKind::Play(play) => (made % seats, format!("phony {play}")),
                    GcgKind::Withdraw => ((made + 1) % seats, "withdraw".to_string()),
                    GcgKind::Bonus(bonus) => ((made + 1) % seats, format!("bonus {bonus}")),
                    GcgKind::Rack | GcgKind::Incomplete => {
                        standing.push(gcg_move);
                        continue;
                    }
                    GcgKind::Lexicon(_) | GcgKind::Player(..) => continue,
                    // a time penalty belongs to its own player, the game over
                    // by then (end it, if the shell did not see it so: an end
                    // that counted no rack has no line)
                    GcgKind::Time(lost) => {
                        if self.final_scores.is_none() {
                            self.run_line("end").map_err(|e| on_line(line_number, &e))?;
                        }
                        (gcg_move.seat, format!("time {} {lost}", gcg_move.seat + 1))
                    }
                    // a note goes on the move before it; one before the first
                    // move has no move to go on, and is printed
                    GcgKind::Note(note) if note.is_empty() => continue,
                    GcgKind::Note(note) if made == 0 => {
                        let said = format!("a note before the first move, not kept: {note}");
                        writeln!(std::io::stdout(), "{}", on_line(line_number, &said))?;
                        continue;
                    }
                    GcgKind::Note(note) => {
                        self.run_line(&format!("note {note}"))
                            .map_err(|e| on_line(line_number, &e))?;
                        continue;
                    }
                    end => {
                        self.load_end(gcg_move, end)
                            .map_err(|e| on_line(line_number, &e))?;
                        continue;
                    }
                };
                if gcg_move.seat != seat {
                    return Err(on_line(
                        line_number,
                        format_args!(
                            "the file's player {} is in the place of player {}",
                            gcg_move.seat + 1,
                            seat + 1
                        ),
                    )
                    .into());
                }
                // an _ is a tile no one saw
                let known = gcg_move.rack.replace('_', "");
                if let GcgKind::Play(_) = gcg_move.kind
                    && !known.is_empty()
                {
                    self.run_line(&format!("rack {known}"))
                        .map_err(|e| on_line(line_number, &e))?;
                }
                self.run_line(&what).map_err(|e| on_line(line_number, &e))?;
                // a challenge's line gives the rack the move left
                if let GcgKind::Bonus(_) = gcg_move.kind
                    && !known.is_empty()
                {
                    self.run_line(&format!("rack {} {known}", seat + 1))
                        .map_err(|e| on_line(line_number, &e))?;
                }
                // a move whose score differs from the board's, by a note giving
                // the board's or by its line alone, is a misscore, adjusted; by
                // its line alone, it is also printed, so that a move read wrong
                // shows
                let game = self.game.as_ref().ok_or("no game")?;
                if let (GcgKind::Play(_), movegen::Play::Place { score, .. }) =
                    (&gcg_move.kind, &game.turns[game.at - 1].play)
                {
                    let board = equity::descale_score(*score);
                    if let Some(correct) = gcg_move.correct
                        && correct != board
                    {
                        return Err(on_line(
                            line_number,
                            format_args!("the board scores it {board}, the file's note {correct}"),
                        )
                        .into());
                    }
                    if gcg_move.score != board {
                        let adjust = gcg_move.score - board;
                        if gcg_move.correct.is_none() {
                            let said = format!(
                                "scored {} where the board makes {board}: a misscore",
                                gcg_move.score
                            );
                            writeln!(std::io::stdout(), "{}", on_line(line_number, &said))?;
                        }
                        self.run_line(&format!("adjust {adjust}"))
                            .map_err(|e| on_line(line_number, &e))?;
                    }
                }
                let game = self.game.as_ref().ok_or("no game")?;
                let turn = &game.turns[game.at - 1];
                let score = match (&gcg_move.kind, &turn.play) {
                    (GcgKind::Play(_), movegen::Play::Place { score, .. }) => {
                        equity::descale_score(*score) + turn.adjust()
                    }
                    _ => gcg_move.score,
                };
                // a time penalty's total is its player's final score
                let total = match (&gcg_move.kind, &self.final_scores) {
                    (GcgKind::Time(_), Some(final_scores)) => final_scores[seat],
                    _ => self.state.players[seat].score,
                };
                let total = equity::descale_score(total);
                if (score, total) != (gcg_move.score, gcg_move.total) {
                    return Err(on_line(
                        line_number,
                        format_args!(
                            "it scores {score} for a total of {total} here, {} for {} in the file",
                            gcg_move.score, gcg_move.total
                        ),
                    )
                    .into());
                }
            }
            for gcg_move in standing {
                let line_number = gcg_move.line_number;
                let known = gcg_move.rack.replace('_', "");
                // #incomplete: the side to move's rack, in a game not over
                let line = match gcg_move.kind {
                    GcgKind::Incomplete if self.final_scores.is_none() => format!("rack {known}"),
                    GcgKind::Incomplete => continue,
                    _ => format!("rack {} {known}", gcg_move.seat + 1),
                };
                if !known.is_empty() {
                    self.run_line(&line).map_err(|e| on_line(line_number, &e))?;
                }
            }
            Ok(())
        })();
        self.printing = printing;
        if printing {
            self.show()?;
        }
        loaded
    }

    // read a GCG file's end, line by line, with the game over (end, if the
    // shell did not see it so): either its player has no tiles and the other's
    // rack counts to them, or its player's rack is left and counts against
    // them. The rack is set (by rack, with - for none), and what it counts and
    // the total are checked against the file's.
    fn load_end(&mut self, gcg_move: &GcgMove, kind: &GcgKind) -> error::Returns<()> {
        if self.final_scores.is_none() {
            self.run_line("end")?;
        }
        let seat = gcg_move.seat;
        let (counted, sign) = match kind {
            GcgKind::Out => {
                self.run_line(&format!("rack {} -", seat + 1))?;
                (1 - seat, 2)
            }
            GcgKind::Shown => (seat, -1),
            _ => return Ok(()),
        };
        let rack = match gcg_move.rack.as_str() {
            "" => "-",
            rack => rack,
        };
        self.run_line(&format!("rack {} {rack}", counted + 1))?;
        let final_scores = self.final_scores.as_ref().ok_or("the game is not over")?;
        let cost = self
            .game_config
            .alphabet()
            .rack_score(&self.state.players[counted].rack);
        let (score, total) = (sign * cost, equity::descale_score(final_scores[seat]));
        if (score, total) != (gcg_move.score, gcg_move.total) {
            return Err(format!(
                "its rack counts {score} for a total of {total} here, {} for {} in the file",
                gcg_move.score, gcg_move.total
            )
            .into());
        }
        Ok(())
    }

    // the moves shown as GCG: a header (the players' nicknames and full names,
    // and the lexicon), then a line for each move (a tile played through as
    // '.', a blank in lowercase, an exchange as - and its tiles or - and how
    // many, and a pass as -) and a line for each thing done to it; then, if the
    // game is over, the racks it counted (the other's to the player with none,
    // or each rack left against its player) and what time cost, else each rack
    // as it is (#rack1, #rack2). GCG has no bag order or seed, and cannot say
    // that a seat showing no _ is given, that an end counted no rack, or that a
    // challenge lost the game.
    fn gcg_text(&self) -> error::Returns<String> {
        let game = self.game.as_ref().ok_or("no game: new first")?;
        let alphabet = self.game_config.alphabet();
        let mut text = String::from("#character-encoding UTF-8\n");
        // a move's line says whose move it is by the nickname alone
        let mut nicknames = Vec::<String>::new();
        for seat in 0..self.seats.len() {
            let (nickname, full) = self.gcg_name(seat);
            if nicknames.contains(&nickname) {
                return Err(
                    format!("two players are {nickname}: give one another nickname").into(),
                );
            }
            text.push_str(&format!("#player{} {nickname} {full}\n", seat + 1));
            nicknames.push(nickname);
        }
        if let Some(lexicon) = self.lexicon_name() {
            text.push_str(&format!("#lexicon {lexicon}\n"));
        }
        let (mut replay, mut rng) = self.dealt(game)?;
        let mut final_scores = None;
        for turn in &game.turns[..game.at] {
            let held = replay.ready_rack(self.game_config, turn)?;
            let board = &mut replay.state;
            let mover = board.turn as usize;
            let (play, score) = match &turn.play {
                movegen::Play::Exchange { tiles } if tiles.is_empty() => ("-".to_string(), 0),
                // for tiles no one saw, only how many
                movegen::Play::Exchange { tiles } if turn.hidden => {
                    (format!("-{}", tiles.len()), 0)
                }
                movegen::Play::Exchange { tiles } => (format!("-{}", alphabet.fmt_rack(tiles)), 0),
                movegen::Play::Place {
                    down,
                    lane,
                    idx,
                    word,
                    score,
                } => {
                    let mut text = if *down {
                        format!("{}{} ", display::column(*lane), idx + 1)
                    } else {
                        format!("{}{} ", lane + 1, display::column(*idx))
                    };
                    for &tile in word.iter() {
                        text.push_str(if tile == 0 {
                            "."
                        } else {
                            alphabet
                                .of_board(tile)
                                .ok_or("a tile not in the alphabet")?
                        });
                    }
                    (text, equity::descale_score(*score))
                }
            };
            // a given seat's rack: what it was known to hold (no tile of an
            // exchange no one saw)
            let held_tiles = &board.players[mover].rack;
            let known = if turn.hidden {
                &[][..]
            } else {
                &held_tiles[..]
            };
            let rack = rack_text(self.game_config, turn.given, known, held_tiles);
            let before = equity::descale_score(board.players[mover].score);
            // a misscore: the move as scored, then the board's score in a note
            // after its line (a note is about the line before it)
            let scored = score + turn.adjust();
            text.push_str(&format!(
                ">{}: {} {} +{} {}\n",
                nicknames[mover],
                rack,
                play,
                scored,
                before + scored
            ));
            if scored != score {
                text.push_str(&format!("{GCG_CORRECT}{score}\n"));
            }
            // the tiles the move left on its rack, which a challenge's line
            // gives before its taking back (what it drew goes back with it)
            let mut left = held_tiles.clone();
            if let movegen::Play::Place { word, .. } = &turn.play
                && turn.withdrawn()
            {
                let placed = word.iter().filter(|&&tile| tile != 0);
                let placed = placed.map(|&tile| if tile & 0x80 != 0 { 0 } else { tile });
                game_state::use_tiles(&mut left, placed)?;
            }
            replay.play_turn(self.game_config, &mut rng, turn)?;
            final_scores = replay.end_of_turn(self.game_config, turn, held)?;
            // what was done to it, a line each in the order done: a phony's
            // taking back (the total as before it), a challenge's bonus (the
            // rack after the move, and the total with it), or a note
            let after_rack = &replay.state.players[mover].rack;
            let after_rack = rack_text(self.game_config, turn.given, after_rack, after_rack);
            let left = rack_text(self.game_config, turn.given, &left, &left);
            let taken_back = turn.after.len() - turn.since_withdrawn().len();
            let mut total = before + scored;
            for (k, after) in turn.after.iter().enumerate() {
                match after {
                    After::Withdraw => {
                        total -= scored;
                        text.push_str(&format!(
                            ">{}: {} -- -{} {}\n",
                            nicknames[mover], rack, scored, total
                        ));
                    }
                    After::Bonus(points) => {
                        total += points;
                        let rack = if k < taken_back { &left } else { &after_rack };
                        text.push_str(&format!(
                            ">{}: {} (challenge) {:+} {}\n",
                            nicknames[mover], rack, points, total
                        ));
                    }
                    After::Note(note) => text.push_str(&format!("#note {note}\n")),
                    _ => {}
                }
            }
            if final_scores.is_some() {
                break;
            }
            replay.state.next_turn();
        }
        let board = &replay.state;
        let lost = game.at > 0 && game.turns[game.at - 1].loser().is_some();
        match final_scores {
            // a game lost on a challenge counts no rack
            Some(_) if lost => {}
            // the racks as they are, the side to move's as set, if it was
            None => {
                for (seat, player) in self.state.players.iter().enumerate() {
                    if !player.rack.is_empty() {
                        let (given, rack) = (self.racks_given[seat], &player.rack);
                        let rack = rack_text(self.game_config, given, rack, rack);
                        text.push_str(&format!("#rack{} {rack}\n", seat + 1));
                    }
                }
            }
            Some(final_scores) => {
                let last = &game.turns[game.at - 1];
                let shown = last.shown();
                let set_none = |seat| shown.iter().any(|(s, t)| *s == seat && t.is_empty());
                let none = (0..board.players.len()).find(|&seat| {
                    board.players[seat].rack.is_empty() && (!replay.given[seat] || set_none(seat))
                });
                // the totals before what time cost, which the time lines give
                let time = last.time();
                let total = |seat: usize| equity::descale_score(final_scores[seat]) + time[seat];
                for (seat, player) in board.players.iter().enumerate() {
                    let other = &board.players[1 - seat].rack;
                    if none == Some(seat) {
                        text.push_str(&format!(
                            ">{}: ({}) +{} {}\n",
                            nicknames[seat],
                            alphabet.fmt_rack(other),
                            2 * alphabet.rack_score(other),
                            total(seat)
                        ));
                    } else if none.is_none() && !player.rack.is_empty() {
                        // the rack, then the tiles that cost the points
                        text.push_str(&format!(
                            ">{}: {} ({}) -{} {}\n",
                            nicknames[seat],
                            alphabet.fmt_rack(&player.rack),
                            alphabet.fmt_rack(&player.rack),
                            alphabet.rack_score(&player.rack),
                            total(seat)
                        ));
                    }
                }
            }
        }
        // then what time cost each player: their last rack, (time), the points,
        // and their final score
        if let (Some(at), Some(final_scores)) = (game.at.checked_sub(1), &self.final_scores) {
            for (seat, &lost) in game.turns[at].time().iter().enumerate() {
                if lost != 0 {
                    text.push_str(&format!(
                        ">{}: {} (time) -{} {}\n",
                        nicknames[seat],
                        alphabet.fmt_rack(&board.players[seat].rack),
                        lost,
                        equity::descale_score(final_scores[seat])
                    ));
                }
            }
        }
        Ok(text)
    }

    // a solve's best line from the position it solved, one line a move as the
    // moves list shows them: the move's number from here, its player, their
    // rack as show prints it, and the move with its score
    fn endgame_line(
        &self,
        solver: &mut endgame::EndgameSolver<'static, kwg::Node22, kwg::Node22>,
        state: &game_state::GameState,
    ) -> error::Returns<Vec<String>> {
        let mut line = Vec::new();
        solver.collect_pv(state.turn, &mut line);
        let kwg = self.kwg()?;
        let mut state = state.clone();
        // with the bag empty, nothing is drawn
        let mut rng = GameRng::seed_from_u64(0);
        let mut text = Vec::new();
        for (i, (_, play)) in line.iter().enumerate() {
            let mover = state.turn as usize;
            text.push(format!(
                "{:>3}. player {} {:<7} {}",
                i + 1,
                mover + 1,
                self.rack_text(mover, &state.players[mover].rack),
                play.fmt(&movegen::BoardSnapshot {
                    board_tiles: &state.board_tiles,
                    game_config: self.game_config,
                    kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: self.klv,
                })
            ));
            state.play(self.game_config, &mut rng, play)?;
            state.next_turn();
        }
        Ok(text)
    }

    // a seat's names in GCG: its nickname and full name (pN and player N if not
    // given)
    fn gcg_name(&self, seat: usize) -> (String, String) {
        self.names[seat]
            .clone()
            .unwrap_or_else(|| (format!("p{}", seat + 1), format!("player {}", seat + 1)))
    }

    // the lexicon's name, from its file's name: CSW24 for CSW24.kwg
    fn lexicon_name(&self) -> Option<&str> {
        let (path, _) = self.kwg.as_ref()?;
        std::path::Path::new(path).file_stem()?.to_str()
    }

    // write the words each move of GCG files formed (paths, each a file or a
    // directory whose .gcg files are read in name order) to out, a new file, a
    // line a word as write_words writes them. Each file is read into a new
    // session with this one's lexicon and leaves, leaving this game untouched,
    // and its name is printed; a file that does not load prints where it
    // stopped, keeps the words of the moves before, and the next file goes on
    fn gcg_words(&self, out: &str, paths: &[String]) -> error::Returns<()> {
        let mut files = Vec::new();
        for path in paths {
            if std::fs::metadata(path)?.is_dir() {
                let mut found = std::fs::read_dir(path)?
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<Result<Vec<_>, _>>()?;
                found.retain(|file| {
                    let gcg = |extension: &std::ffi::OsStr| extension.eq_ignore_ascii_case("gcg");
                    file.extension().is_some_and(gcg)
                });
                found.sort();
                files.extend(found);
            } else {
                files.push(std::path::PathBuf::from(path));
            }
        }
        let mut written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out)?;
        let mut session = Session::new(true, self.memory)?;
        if let Some((path, kwg)) = &self.kwg {
            session.use_kwg(path, kwg);
        }
        session.klv = self.klv;
        session.printing = false;
        let mut stdout = std::io::stdout();
        for (k, file) in (1..).zip(&files) {
            let name = file.display().to_string();
            writeln!(stdout, "file {k} of {}: {name}", files.len())?;
            session.game = None;
            let loaded = (|| -> error::Returns<()> {
                let moves = gcg_moves(&gcg_decode(&std::fs::read(file)?)?)?;
                session.load_moves(&moves)
            })();
            session.write_words(&name, &mut written)?;
            if let Err(e) = loaded {
                writeln!(stdout, "{name}: {e}; the moves before kept")?;
            }
        }
        Ok(())
    }

    // write the words each move shown formed, one line a word, tab separated:
    // the file's name, the move's number, its player's nickname, the word (with
    // the tiles it played through, and a blank as its letter), main or cross (a
    // hook), and "taken back" for a phony withdrawn
    fn write_words(&self, name: &str, out: &mut impl Write) -> error::Returns<()> {
        let Some(game) = &self.game else {
            return Ok(());
        };
        let kwg = self.kwg()?;
        let alphabet = self.game_config.alphabet();
        let (mut replay, mut rng) = self.dealt(game)?;
        let mut words = play_scorer::PlayScorer::new();
        for (number, turn) in (1..).zip(&game.turns[..game.at]) {
            let held = replay.ready_rack(self.game_config, turn)?;
            let (nickname, _) = self.gcg_name(replay.state.turn as usize);
            let taken_back = if turn.withdrawn() { "\ttaken back" } else { "" };
            let mut kind = "main";
            let mut written = Ok(());
            words.words_all(
                &movegen::BoardSnapshot {
                    board_tiles: &replay.state.board_tiles,
                    game_config: self.game_config,
                    kwg,
                    anagrams: None,
                    rack_lengths: None,
                    klv: self.klv,
                },
                &turn.play,
                |word: &[u8]| {
                    // a move's own word comes first
                    let word = word
                        .iter()
                        .map(|&tile| alphabet.of_board(tile).unwrap_or("?"))
                        .collect::<String>();
                    if written.is_ok() {
                        written = writeln!(
                            out,
                            "{name}\t{number}\t{nickname}\t{word}\t{kind}{taken_back}"
                        );
                    }
                    kind = "cross";
                    true
                },
            );
            written?;
            replay.play_turn(self.game_config, &mut rng, turn)?;
            if replay.end_of_turn(self.game_config, turn, held)?.is_some() {
                break;
            }
            replay.state.next_turn();
        }
        Ok(())
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

// solve the side to move's endgame, once the bag is empty and both racks are
// known, within the limits, printing its progress unless quiet: returns the
// solver, whose best line can be read, and what it found
fn solve_endgame(
    game_config: &'static game_config::GameConfig,
    kwg: &'static kwg::Kwg<kwg::Node22>,
    arm_tables: &'static anagrams::ArmTables,
    state: &game_state::GameState,
    limits: &endgame::Limits,
    quiet: bool,
) -> error::Returns<(
    endgame::EndgameSolver<'static, kwg::Node22, kwg::Node22>,
    endgame::Solved,
)> {
    if !state.bag.is_empty() {
        return Err("the bag is not empty, and an endgame needs it to be".into());
    }
    let mut solver = endgame::EndgameSolver::new(game_config, kwg, arm_tables)?;
    solver.init(
        &state.board_tiles,
        [&state.players[0].rack, &state.players[1].rack],
    );
    if !quiet {
        solver.report_progress(endgame_watch::ticking(|progress, elapsed| {
            writeln!(std::io::stderr(), "endgame: {}", progress.after(elapsed))?;
            Ok(())
        }));
    }
    let solved = solver.solve_within(state.turn, limits)?;
    Ok((solver, solved))
}

// what a line of a GCG file does: a move, as play takes it; the move before
// taken back as a phony; points to its player after a challenge; that its
// player had no tiles at the end, so the other's rack counts to them; the rack
// its player showed at the end; a player's rack in a game not over (#rack1,
// #rack2), or the side to move's (#incomplete); once the game is over, the
// points its player lost for time; the lexicon the game was played with; a
// player's nickname and full name (#player1, #player2); or a note on the line
// before it
enum GcgKind {
    Play(String),
    Withdraw,
    Bonus(i32),
    Out,
    Shown,
    Rack,
    Incomplete,
    Time(i32),
    Lexicon(String),
    Player(String, String),
    Note(String),
}

// the start of gcg save's line for a misscore, before the board's score: GCG
// keeps the score as agreed, and this note the score the board makes
const GCG_CORRECT: &str = "#note the correct score is ";

// a line of a GCG file: its number, its player (by the order the players first
// move; for #rackN, where #playerN's nickname moves, else player N), the rack,
// what it does, and the score and total it gives
struct GcgMove {
    line_number: usize,
    seat: usize,
    rack: String,
    kind: GcgKind,
    score: i32,
    total: i32,
    correct: Option<i32>, // a move's score as the board makes it, if a note gives it
}

// decode a GCG file's text by the encoding it declares, UTF-8 or ISO 8859-1
// (one character per byte); if it declares none, UTF-8 if it is valid UTF-8,
// else ISO 8859-1. A UTF-8 byte order mark at the start is dropped first, or
// the # line it is on would not read as one
fn gcg_decode(bytes: &[u8]) -> error::Returns<String> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let latin1 = bytes.iter().map(|&b| char::from(b)).collect::<String>();
    let declared = latin1.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("#character-encoding"))
            .then(|| words.next().unwrap_or("").to_ascii_uppercase())
    });
    match declared.as_deref() {
        Some("ISO-8859-1") => Ok(latin1),
        Some("UTF-8") => Ok(String::from_utf8(bytes.to_vec())?),
        Some(other) => Err(format!("the encoding {other:?}: UTF-8 or ISO-8859-1 only").into()),
        None => Ok(String::from_utf8(bytes.to_vec()).unwrap_or(latin1)),
    }
}

// read a GCG file's moves (a rack left empty is one no one saw), phonies taken
// back, challenge bonuses, the racks its end counts, time penalties, the racks
// of a game not over (#rack1 and #rack2, for the players #player1 and #player2
// give, and #incomplete, for the side to move), the players' nicknames and full
// names, a move's score as the board makes it (the note after a misscore), the
// lexicon, and every other note (with the lines after it that are neither a
// pragma nor a move, as more of it), skipping every other # line
fn gcg_moves(text: &str) -> error::Returns<Vec<GcgMove>> {
    let mut moves = Vec::new();
    let mut players = Vec::<String>::new();
    let mut nicknames = [None, None];
    let mut note_open = false;
    for (line_number, line) in (1..).zip(text.lines()) {
        let fail = |what: &str| format!("line {line_number}: {what}: {line:?}");
        if line.starts_with('#') {
            note_open = false;
            let pragma = line.trim_end();
            // a misscore note: the board's score for the move before (without a
            // number, it is an ordinary note)
            if let Some(correct) = pragma.strip_prefix(GCG_CORRECT)
                && let Ok(correct) = correct.parse()
            {
                let played = moves
                    .iter_mut()
                    .rev()
                    .find(|m: &&mut GcgMove| matches!(m.kind, GcgKind::Play(_)));
                played.ok_or_else(|| fail("no move before"))?.correct = Some(correct);
                continue;
            }
            let words = pragma.split_whitespace().collect::<Vec<_>>();
            match words.as_slice() {
                [player @ ("#player1" | "#player2"), nickname, full @ ..] => {
                    let player = usize::from(*player == "#player2");
                    nicknames[player] = Some(nickname.to_string());
                    moves.push(GcgMove {
                        line_number,
                        seat: player,
                        rack: String::new(),
                        kind: GcgKind::Player(nickname.to_string(), full.join(" ")),
                        score: 0,
                        total: 0,
                        correct: None,
                    });
                    continue;
                }
                ["#incomplete", tiles] => {
                    moves.push(GcgMove {
                        line_number,
                        seat: 0,
                        rack: tiles.to_string(),
                        kind: GcgKind::Incomplete,
                        score: 0,
                        total: 0,
                        correct: None,
                    });
                    continue;
                }
                ["#lexicon", name, ..] => {
                    moves.push(GcgMove {
                        line_number,
                        seat: 0,
                        rack: String::new(),
                        kind: GcgKind::Lexicon(name.to_string()),
                        score: 0,
                        total: 0,
                        correct: None,
                    });
                    continue;
                }
                _ => {}
            }
            let rack = pragma.strip_prefix("#rack").map(str::split_whitespace);
            let (seat, rack, kind) = match rack {
                Some(mut tokens) => {
                    let n = tokens.next().and_then(|n| n.parse::<usize>().ok());
                    let Some(seat) = n.and_then(|n| n.checked_sub(1)) else {
                        continue;
                    };
                    match (tokens.next(), tokens.next()) {
                        (Some(rack), None) => (seat, rack, GcgKind::Rack),
                        (None, _) => continue,
                        _ => return Err(fail("not a rack this reads").into()),
                    }
                }
                // any other note, its text
                None => match pragma.strip_prefix("#note") {
                    Some(text) if text.is_empty() || text.starts_with(char::is_whitespace) => {
                        (0, "", GcgKind::Note(text.trim_start().to_string()))
                    }
                    _ => continue,
                },
            };
            note_open = matches!(kind, GcgKind::Note(_));
            moves.push(GcgMove {
                line_number,
                seat,
                rack: rack.to_string(),
                kind,
                score: 0,
                total: 0,
                correct: None,
            });
            continue;
        }
        let Some(rest) = line.strip_prefix('>') else {
            // a line that is neither a pragma nor a move, after a note: more of
            // the note
            let more = line.trim();
            if let (
                true,
                Some(GcgMove {
                    kind: GcgKind::Note(note),
                    ..
                }),
            ) = (note_open, moves.last_mut())
                && !more.is_empty()
            {
                if !note.is_empty() {
                    note.push(' ');
                }
                note.push_str(more);
            }
            continue;
        };
        note_open = false;
        let (player, rest) = rest.split_once(':').ok_or_else(|| fail("no colon"))?;
        let seat = match players.iter().position(|p| p == player) {
            Some(seat) => seat,
            None => {
                players.push(player.to_string());
                players.len() - 1
            }
        };
        let mut tokens = rest.split_whitespace().collect::<Vec<_>>();
        // a rack left empty: the line starts where the rack would end, at a
        // coordinate, an exchange or a pass, a phony taken back, or a bonus
        if let Some(first) = tokens.first()
            && (first.starts_with('-')
                || first.bytes().any(|b| b.is_ascii_digit())
                || ["(challenge)", "(time)"].contains(first))
        {
            tokens.insert(0, "_");
        }
        let (rack, kind, score, total) = match tokens.as_slice() {
            // a time penalty: the player's last rack, (time), the points
            [rack, "(time)", score, total] => {
                let lost = score
                    .parse::<i32>()
                    .ok()
                    .and_then(i32::checked_neg)
                    .filter(|&lost| lost > 0)
                    .ok_or_else(|| fail("a time penalty takes points"))?;
                (*rack, GcgKind::Time(lost), *score, *total)
            }
            // a challenge's bonus, after the rack the move left (none, for a
            // move that went out)
            [rack @ .., "(challenge)", score, total] if rack.len() < 2 => {
                let bonus = score.parse().map_err(|_| fail("no score"))?;
                let rack = rack.first().copied().unwrap_or("");
                (rack, GcgKind::Bonus(bonus), *score, *total)
            }
            // the end's racks: the other's, counted to the player with no
            // tiles, or a rack shown, with what it cost
            [counted, score, total] if counted.starts_with('(') && counted.ends_with(')') => {
                let rack = &counted[1..counted.len() - 1];
                (rack, GcgKind::Out, *score, *total)
            }
            [rack, counted, score, total] if counted.starts_with('(') => {
                (*rack, GcgKind::Shown, *score, *total)
            }
            [rack, coord, word, score, total] => (
                *rack,
                GcgKind::Play(format!("{coord} {word}")),
                *score,
                *total,
            ),
            [rack, "--", score, total] => (*rack, GcgKind::Withdraw, *score, *total),
            [rack, "-", score, total] => (*rack, GcgKind::Play("pass".into()), *score, *total),
            [rack, exch, score, total] if exch.starts_with('-') => (
                *rack,
                GcgKind::Play(format!("exch {}", &exch[1..])),
                *score,
                *total,
            ),
            _ => return Err(fail("not a move this reads").into()),
        };
        moves.push(GcgMove {
            line_number,
            seat,
            rack: rack.to_string(),
            kind,
            score: score.parse().map_err(|_| fail("no score"))?,
            total: total.parse().map_err(|_| fail("no total"))?,
            correct: None,
        });
    }
    // the player of #rackN or #playerN: the seat where #playerN's nickname
    // first moved, else the seat the other nickname did not move in, else seat
    // N
    let moved = |player: usize| {
        let nickname = nicknames[player].as_ref()?;
        players.iter().position(|p| p == nickname)
    };
    for gcg_move in moves.iter_mut() {
        let player = gcg_move.seat;
        if matches!(gcg_move.kind, GcgKind::Rack | GcgKind::Player(..)) && player < 2 {
            gcg_move.seat = moved(player)
                .or_else(|| moved(1 - player).and_then(|seat| 1usize.checked_sub(seat)))
                .unwrap_or(player);
        }
    }
    Ok(moves)
}

// an error message, prefixed with the number of the GCG line it is about
fn on_line(line_number: usize, what: impl std::fmt::Display) -> String {
    format!("line {line_number}: {what}")
}

// a rack as GCG writes it: its tiles, or for a seat given, the known tiles and
// an _ for each of the rest of a full rack (none past full)
fn rack_text(
    game_config: &game_config::GameConfig,
    given: bool,
    known: &[u8],
    held: &[u8],
) -> String {
    let alphabet = game_config.alphabet();
    if !given {
        return alphabet.fmt_rack(held).to_string();
    }
    let rest = (game_config.rack_size() as usize).saturating_sub(known.len());
    format!("{}{}", alphabet.fmt_rack(known), "_".repeat(rest))
}

// a game being replayed from its seed: its state; by seat, whether its rack is
// given (as its last move's was, or before its first move, as that move's or
// the seat's); and by seat, the tiles of its rack that were typed (a rack set,
// and what moves left of it), which setting another seat's rack never takes,
// unlike a tile the shell drew
struct Replay {
    state: game_state::GameState,
    given: [bool; 2],
    typed: [Vec<u8>; 2],
}

impl Replay {
    // play a turn's move (a pass, if it was taken back), then give its bonus
    // and its misscore to its mover
    fn play_turn(
        &mut self,
        game_config: &game_config::GameConfig,
        rng: &mut GameRng,
        turn: &Turn,
    ) -> error::Returns<()> {
        let state = &mut self.state;
        state.play(game_config, rng, &turn.played())?;
        let mover = state.turn as usize;
        // a misscore goes with the move, and is taken back with it
        let adjust = if turn.withdrawn() { 0 } else { turn.adjust() };
        state.players[mover].score = state.players[mover]
            .score
            .checked_add(equity::scale_score(turn.bonus().unwrap_or(0)))
            .and_then(|score| score.checked_add(equity::scale_score(adjust)))
            .ok_or("score overflow")?;
        Ok(())
    }

    // finish a move once it is played (held is how many tiles its mover held
    // before it): what the mover typed and kept stays typed; a seat given drew
    // nothing anyone saw, so what it drew goes back in the bag and it keeps its
    // leave; then the racks set after it apply. A challenge of it that lost the
    // game ended the game, with the scores as they are. Else the game is over
    // if it was declared over (end), if a dealt mover placed tiles and the bag
    // left them none, or after the variant's scoreless turns; then the tiles
    // left count: a player with none (dealt, or set to none) gets twice the
    // other's, and the other loses nothing; otherwise each player loses their
    // own. Either way, what time cost each player then comes off.
    fn end_of_turn(
        &mut self,
        game_config: &game_config::GameConfig,
        turn: &Turn,
        held: usize,
    ) -> error::Returns<Option<Vec<i32>>> {
        let state = &mut self.state;
        let mover = state.turn as usize;
        let (placed, used) = match &*turn.played() {
            movegen::Play::Place { word, .. } => (true, word.iter().filter(|&&t| t != 0).count()),
            movegen::Play::Exchange { tiles } => (false, tiles.len()),
        };
        let out = placed && !turn.given && state.players[mover].rack.is_empty();
        let rack = &mut state.players[mover].rack;
        let leave = held.saturating_sub(used).min(rack.len());
        self.typed[mover] = kept(&self.typed[mover], &rack[..leave]);
        if turn.given {
            let drawn = rack.split_off(leave);
            state.bag.return_tiles(&drawn);
        }
        let shown = turn.shown();
        self.type_racks(&shown)?;
        let state = &self.state;
        let mut final_scores = state.players.iter().map(|p| p.score).collect::<Vec<_>>();
        if turn.loser().is_none() {
            if !turn.ended() && !out && !state.scoreless_turns_ended(game_config) {
                return Ok(None);
            }
            let set_none = |seat| shown.iter().any(|(s, t)| *s == seat && t.is_empty());
            let none = (0..state.players.len()).find(|&seat| {
                state.players[seat].rack.is_empty() && (!self.given[seat] || set_none(seat))
            });
            let alphabet = game_config.alphabet();
            for (seat, (score, player)) in final_scores
                .iter_mut()
                .zip(state.players.iter())
                .enumerate()
            {
                *score = match none {
                    // two players: the one with no tiles gets twice the other's
                    Some(none) if none == seat => alphabet
                        .scaled_rack_score(&state.players[1 - seat].rack)
                        .checked_mul(2)
                        .and_then(|bonus| player.score.checked_add(bonus))
                        .ok_or("score overflow")?,
                    Some(_) => player.score,
                    None => player
                        .score
                        .checked_sub(alphabet.scaled_rack_score(&player.rack))
                        .ok_or("score overflow")?,
                };
            }
        }
        // then what time cost each player
        for (score, &lost) in final_scores.iter_mut().zip(turn.time().iter()) {
            *score = score
                .checked_sub(equity::scale_score(lost))
                .ok_or("score overflow")?;
        }
        Ok(Some(final_scores))
    }

    // set racks, in any order, each then counted as typed: each rack's tiles go
    // back in the bag first, then each new tile is taken from the bag, or else
    // from a dealt rack that is not being set and holds a copy the shell drew
    // (that rack drawing a replacement while the bag has one); never a typed
    // tile, and never from a rack given
    fn type_racks(&mut self, shown: &[(usize, Vec<u8>)]) -> error::Returns<()> {
        for (seat, _) in shown {
            let rack = std::mem::take(&mut self.state.players[*seat].rack);
            self.state.bag.return_tiles(&rack);
            self.typed[*seat].clear();
        }
        let set = shown.iter().map(|&(seat, _)| seat).collect::<Vec<_>>();
        for (seat, tiles) in shown {
            for &tile in tiles {
                self.take_tile(&set, tile)?;
                self.state.players[*seat].rack.push(tile);
            }
            self.typed[*seat].clone_from(tiles);
        }
        Ok(())
    }

    // take a tile out of the bag, or else out of a dealt rack (not one of
    // these) holding a copy the shell drew (more copies than were typed), which
    // draws a replacement while the bag has one: a typed tile outweighs one the
    // shell drew, even with none left to draw
    fn take_tile(&mut self, not: &[usize], tile: u8) -> error::Returns<()> {
        let state = &mut self.state;
        if state.bag.remove_tile(tile).is_some() {
            return Ok(());
        }
        let count = |tiles: &[u8]| tiles.iter().filter(|&&t| t == tile).count();
        let other = (0..state.players.len())
            .filter(|&seat| !self.given[seat] && !not.contains(&seat))
            .find(|&seat| count(&state.players[seat].rack) > count(&self.typed[seat]));
        let Some(other) = other else {
            return Err("those tiles are not all unseen".into());
        };
        let rack = &mut state.players[other].rack;
        let at = rack
            .iter()
            .rposition(|&t| t == tile)
            .ok_or("no such tile")?;
        rack.swap_remove(at);
        let held = rack.len() + 1;
        state
            .bag
            .replenish(&mut state.players[other].rack, held, other);
        Ok(())
    }

    // the side to move given exactly these tiles
    fn give_rack(&mut self, rack: &[u8]) -> error::Returns<()> {
        let mover = self.state.turn as usize;
        self.type_racks(&[(mover, rack.to_vec())])
    }

    // ready the mover's rack before their move, by the move's kind: the tiles
    // given for it, if any; else a seat dealt draws a full rack, and a seat
    // given holds what it was known to hold (of a rack dealt before, what was
    // typed). Returns how many tiles that is.
    fn ready_rack(
        &mut self,
        game_config: &game_config::GameConfig,
        turn: &Turn,
    ) -> error::Returns<usize> {
        let mover = self.state.turn as usize;
        if turn.given && !self.given[mover] {
            self.forget_drawn(mover);
        }
        self.given[mover] = turn.given;
        match &turn.rack {
            Some(rack) => self.give_rack(rack)?,
            None if !turn.given => top_up(game_config, &mut self.state, mover),
            None => {}
        }
        Ok(self.state.players[mover].rack.len())
    }

    // a dealt rack that becomes given keeps what was typed of it, and what the
    // shell drew goes back in the bag
    fn forget_drawn(&mut self, seat: usize) {
        let mut drawn = std::mem::take(&mut self.state.players[seat].rack);
        let known = kept(&self.typed[seat], &drawn);
        for tile in &known {
            if let Some(at) = drawn.iter().position(|t| t == tile) {
                drawn.swap_remove(at);
            }
        }
        self.state.bag.return_tiles(&drawn);
        self.state.players[seat].rack = known;
    }
}

// the typed tiles a leave still holds: when a move used a copy of a tile, a
// drawn copy is taken to have gone first
fn kept(typed: &[u8], leave: &[u8]) -> Vec<u8> {
    let mut spare = leave.to_vec();
    let mut kept = Vec::new();
    for &tile in typed {
        if let Some(at) = spare.iter().position(|&t| t == tile) {
            spare.swap_remove(at);
            kept.push(tile);
        }
    }
    kept
}

// draw a dealt rack up to full from the bag (never past full)
fn top_up(game_config: &game_config::GameConfig, state: &mut game_state::GameState, seat: usize) {
    let full = game_config.rack_size() as usize;
    state
        .bag
        .replenish(&mut state.players[seat].rack, full, seat);
}

// a rack known to hold these tiles, plus whatever a move needs that they lack
// (a blank as 0)
fn completed(known: &[u8], needed: &[u8]) -> Vec<u8> {
    let mut rack = known.to_vec();
    let mut spare = known.to_vec();
    for &tile in needed {
        match spare.iter().position(|&t| t == tile) {
            Some(at) => {
                spare.swap_remove(at);
            }
            None => rack.push(tile),
        }
    }
    rack
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
        let mut session = Session::new(true, None).unwrap();
        session.use_kwg("tiny", Box::leak(Box::new(tiny_lexicon())));
        session
    }

    // a line as typed
    fn run(session: &mut Session, line: &str) -> error::Returns<()> {
        let words = line
            .split_whitespace()
            .map(String::from)
            .collect::<Vec<_>>();
        session.run(line, &words, true)
    }

    // a line as source reads it from a file
    fn replay(session: &mut Session, line: &str) -> error::Returns<()> {
        let words = line
            .split_whitespace()
            .map(String::from)
            .collect::<Vec<_>>();
        session.run(line, &words, false)
    }

    #[test]
    #[inline]
    fn a_bot_seat_moves_when_its_turn_comes_and_a_replay_adds_no_move() {
        let mut first = session();
        for line in ["player 2 bot", "new 7", "rack AETAETE", "play 8G AT"] {
            run(&mut first, line).unwrap();
        }
        // the bot answered the typed move, logged as its play
        assert_eq!(first.game.as_ref().unwrap().turns.len(), 2);
        assert_eq!(first.log.len(), 5);
        assert!(first.log[4].starts_with("play "), "{}", first.log[4]);
        // read back as source reads it, the log replays the same game, with no
        // extra move
        let mut second = session();
        for line in first.log.clone() {
            replay(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&first), snapshot(&second));
        assert_eq!(second.game.as_ref().unwrap().turns.len(), 2);
        // its rack shows as an _ for each tile
        assert_eq!(first.rack_text(1, &[1, 2, 0]), "___");
        assert_eq!(first.rack_text(0, &[1, 2, 0]), "AB?");
    }

    #[test]
    #[inline]
    fn a_given_players_rack_is_what_is_given_and_what_their_moves_show() {
        let mut first = session();
        for line in ["player 2 given", "new 7", "rack AETAETE", "play 8G AT"] {
            run(&mut first, line).unwrap();
        }
        // player 2 is dealt nothing; given AT, they play EA: their rack was AT
        // plus the E the move needed, and afterward they hold the T and draw
        // nothing anyone sees
        assert!(first.state.players[1].rack.is_empty());
        run(&mut first, "r 2 AT").unwrap();
        run(&mut first, "p 9H EA").unwrap();
        assert_eq!(first.state.players[1].rack, [20]);
        let turn = &first.game.as_ref().unwrap().turns[1];
        assert_eq!(turn.rack.as_deref(), Some(&[1, 20, 5][..]));
        // after an exchange of tiles no one saw, nothing of the rack is known
        run(&mut first, "play H8 TEA").unwrap();
        run(&mut first, "play exch 3").unwrap();
        assert!(first.state.players[1].rack.is_empty());
        assert_eq!(first.state.players[1].num_exchanges, 1);
        assert!(first.game.as_ref().unwrap().turns[3].hidden);
        assert!(run(&mut first, "play exch 9").is_err());
        // logged as the long commands, and replayed the same
        assert!(first.log.contains(&"rack 2 AT".to_string()));
        assert!(first.log.contains(&"play 9H EA".to_string()));
        let mut second = session();
        for line in first.log.clone() {
            replay(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&first), snapshot(&second));
    }

    #[test]
    #[inline]
    fn at_an_end_each_rack_left_counts_as_known() {
        let mut first = session();
        for line in ["player 2 given", "new 7", "rack AETAETE", "play 8G AT"] {
            run(&mut first, line).unwrap();
        }
        run(&mut first, "rack 2 EE").unwrap();
        for _ in 0..6 {
            run(&mut first, "play pass").unwrap();
        }
        // after six scoreless turns, each rack left counts against its player,
        // player 2's being the EE they were given
        let alphabet = first.game_config.alphabet();
        let counted = |session: &Session| {
            let players = session.state.players.iter();
            players
                .map(|player| player.score - alphabet.scaled_rack_score(&player.rack))
                .collect::<Vec<_>>()
        };
        assert_eq!(first.state.players[1].rack, [5, 5]);
        assert_eq!(first.final_scores, Some(counted(&first)));
        // a rack set at the end counts as set; tiles that are not unseen are
        // refused
        assert!(run(&mut first, "rack 2 ZZZZZZZ").is_err());
        run(&mut first, "rack 2 EEA").unwrap();
        assert_eq!(first.final_scores, Some(counted(&first)));
        // player 2, with no tiles, gets twice player 1's, and player 1 loses
        // nothing
        run(&mut first, "rack 2 -").unwrap();
        let score = |seat: usize| first.state.players[seat].score;
        let twice = 2 * alphabet.scaled_rack_score(&first.state.players[0].rack);
        assert_eq!(first.final_scores, Some(vec![score(0), score(1) + twice]));
        let mut second = session();
        for line in first.log.clone() {
            replay(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&first), snapshot(&second));
        assert_eq!(first.final_scores, second.final_scores);
    }

    #[test]
    #[inline]
    fn end_says_the_game_is_over_and_the_racks_left_count() {
        let mut first = session();
        for line in ["player 2 given", "new 7", "rack AT", "play 8G AT"] {
            run(&mut first, line).unwrap();
        }
        assert!(first.final_scores.is_none());
        run(&mut first, "end").unwrap();
        assert!(run(&mut first, "end").is_err());
        // player 1's dealt rack counts against them; player 2's, with nothing
        // known, counts nothing
        let alphabet = first.game_config.alphabet();
        let left = alphabet.scaled_rack_score(&first.state.players[0].rack);
        let score = first.state.players[0].score;
        assert_eq!(first.final_scores, Some(vec![score - left, 0]));
        run(&mut first, "rack 2 EE").unwrap();
        let ee = alphabet.scaled_rack_score(&[5, 5]);
        assert_eq!(first.final_scores, Some(vec![score - left, -ee]));
        // with player 1 set to no tiles, they get twice player 2's
        run(&mut first, "rack 1 -").unwrap();
        assert_eq!(first.final_scores, Some(vec![score + 2 * ee, 0]));
        // taken back and replayed, it ends again
        run(&mut first, "back").unwrap();
        assert!(first.final_scores.is_none());
        run(&mut first, "forward").unwrap();
        assert_eq!(first.final_scores, Some(vec![score + 2 * ee, 0]));
        let mut second = session();
        for line in first.log.clone() {
            replay(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&first), snapshot(&second));
        assert_eq!(first.final_scores, second.final_scores);
    }

    #[test]
    #[inline]
    fn a_sim_draws_a_given_rack_full_from_the_bag() {
        let mut session = session();
        for line in [
            "player 2 given",
            "new 7",
            "rack AETAETE",
            "play 8G AT",
            "play 9H EA",
        ] {
            run(&mut session, line).unwrap();
        }
        // they hold nothing known; a sim of player 1's move draws them a full
        // rack
        assert!(session.state.players[1].rack.is_empty());
        let filled = session.filled_state();
        assert_eq!(filled.players[1].rack.len(), 7);
        assert_eq!(filled.bag.len() + 7, session.state.bag.len());
        run(&mut session, "sim 4").unwrap();
        assert!(session.computer_move(Some(50.0)).is_ok());
    }

    #[test]
    #[inline]
    fn a_rack_set_takes_a_tile_the_shell_drew_never_one_typed() {
        let mut first = session();
        run(&mut first, "new 7").unwrap();
        // player 1 takes every copy of a tile player 2 was dealt: player 2's
        // copies are taken, and they draw again
        let alphabet = first.game_config.alphabet();
        let tile = first.state.players[1].rack[0];
        let copies = vec![tile; alphabet.freq(tile) as usize];
        run(&mut first, &format!("rack {}", alphabet.fmt_rack(&copies))).unwrap();
        assert_eq!(first.state.players[0].rack, copies);
        assert_eq!(first.state.players[1].rack.len(), 7);
        assert!(!first.state.players[1].rack.contains(&tile));
        // once typed, none of them can go to player 2
        let one = format!("rack 2 {}", alphabet.fmt_rack(&[tile]));
        let error = run(&mut first, &one).unwrap_err().to_string();
        assert_eq!(error, "those tiles are not all unseen");
        // nor can what a move leaves of a typed rack: both blanks, kept
        let mut second = session();
        for line in ["new 7", "rack ??AT", "play 8G AT"] {
            run(&mut second, line).unwrap();
        }
        assert!(run(&mut second, "rack ?").is_err());
        assert!(run(&mut second, "rack E").is_ok());
        // with the bag empty, a drawn tile is still taken, leaving its rack one
        // short
        let game_config = second.game_config;
        let mut state = game_state::GameState::new(game_config);
        state.bag.set_from_iter(std::iter::empty());
        state.players[0].rack = vec![17, 1];
        let mut replay = Replay {
            state,
            given: [false; 2],
            typed: Default::default(),
        };
        replay.type_racks(&[(1, vec![17])]).unwrap();
        assert_eq!(replay.state.players[0].rack, [1]);
        assert_eq!(replay.state.players[1].rack, [17]);
        // and that Q, typed now, is not taken back
        assert!(replay.type_racks(&[(0, vec![17])]).is_err());
    }

    #[test]
    #[inline]
    fn a_seat_changed_in_a_game_keeps_the_moves_made_before() {
        // the computer moves, then the seat is made given
        let mut first = session();
        for line in ["player 2 bot", "new 7", "rack AETAETE", "play 8G AT"] {
            run(&mut first, line).unwrap();
        }
        assert_eq!(first.game.as_ref().unwrap().turns.len(), 2);
        run(&mut first, "player 2 given").unwrap();
        run(&mut first, "play pass").unwrap();
        // to move, they hold only what is known of their rack: none of the
        // tiles the shell drew for the computer
        assert!(first.state.players[1].rack.is_empty());
        run(&mut first, "play pass").unwrap();
        let mut second = session();
        for line in first.log.clone() {
            replay(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&second), snapshot(&first));
        // the seat is given, then the computer's, which draws a rack of its own
        // to move with
        let mut third = session();
        for line in [
            "player 2 given",
            "new 7",
            "rack AETAETE",
            "play 8G AT",
            "play 9H EA",
            "player 2 bot",
        ] {
            run(&mut third, line).unwrap();
        }
        assert!(third.state.players[1].rack.is_empty());
        run(&mut third, "play pass").unwrap();
        assert_eq!(third.game.as_ref().unwrap().turns.len(), 4);
        let mut fourth = session();
        for line in third.log.clone() {
            replay(&mut fourth, &line).unwrap();
        }
        assert_eq!(snapshot(&fourth), snapshot(&third));
        // given for a pass, then dealt as a person's: with the game over before
        // their next move, nothing is known of their rack, which counts nothing
        // and has not gone out
        let mut fifth = session();
        for line in [
            "player 2 given",
            "new 7",
            "rack AETAETE",
            "play 8G AT",
            "play pass",
            "player 2 human",
            "end",
        ] {
            run(&mut fifth, line).unwrap();
        }
        let alphabet = fifth.game_config.alphabet();
        let player = &fifth.state.players[0];
        let left = alphabet.scaled_rack_score(&player.rack);
        assert_eq!(fifth.final_scores, Some(vec![player.score - left, 0]));
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
    fn a_sim_runs_takes_candidates_in_and_out_and_ends_with_a_change() {
        let mut session = session();
        for line in &GAME[..2] {
            run(&mut session, line).unwrap();
        }
        let before = snapshot(&session);
        run(&mut session, "sim 32").unwrap();
        let sim = session.sim.as_ref().unwrap();
        let field = sim
            .move_generator
            .plays
            .iter()
            .map(|v| v.play.clone())
            .collect::<Vec<_>>();
        // the candidates still in were rolled out on every iteration; the
        // leader (whose difference with itself is zero) can never leave
        let active = sim.simmer.active_stream_ids().collect::<Vec<_>>();
        assert!(
            active
                .iter()
                .all(|&s| sim.simmer.stream_count(s) == Some(32.0))
        );
        let leader = sim.stream_of[sim.simmer.best_so_far()];
        let other = *active.iter().find(|&&s| s != leader).unwrap();
        // a candidate dropped sits out while the leader goes on
        let other_text = session.play_text(&field[other as usize]).unwrap();
        run(&mut session, &format!("sim drop {other_text}")).unwrap();
        run(&mut session, "sim 16").unwrap();
        let sim = session.sim.as_ref().unwrap();
        assert_eq!(sim.simmer.stream_count(other), Some(32.0));
        assert_eq!(sim.simmer.stream_count(leader), Some(48.0));
        assert!(run(&mut session, &format!("sim drop {other_text}")).is_err());
        // added back with what it had; a move outside the field joins as a new
        // candidate
        run(&mut session, &format!("sim add {other_text}")).unwrap();
        assert!(run(&mut session, &format!("sim add {other_text}")).is_err());
        let mut all = movegen::KurniaMoveGenerator::new(session.game_config);
        all.gen_moves_unfiltered(&movegen::GenMovesParams {
            board_snapshot: &movegen::BoardSnapshot {
                board_tiles: &session.state.board_tiles,
                game_config: session.game_config,
                kwg: session.kwg().unwrap(),
                anagrams: None,
                rack_lengths: None,
                klv: session.klv,
            },
            rack: &session.state.current_player().rack,
            max_gen: usize::MAX,
            num_exchanges_by_this_player: 0,
            pass_policy: movegen::PassPolicy::AsACandidate,
            dynamic_leaves: None,
        });
        let outside = all
            .plays
            .iter()
            .map(|v| v.play.clone())
            .find(|play| !field.contains(play))
            .expect("a move outside the field");
        let outside = session.play_text(&outside).unwrap();
        run(&mut session, &format!("sim add {outside}")).unwrap();
        run(&mut session, "sim 16").unwrap();
        let sim = session.sim.as_ref().unwrap();
        // the candidate added back went on from where it was
        assert_eq!(sim.simmer.stream_count(other), Some(48.0));
        assert_eq!(sim.simmer.stream_count(leader), Some(64.0));
        assert_eq!(
            sim.simmer.stream_count(*sim.stream_of.last().unwrap()),
            Some(16.0)
        );
        // the game is untouched and nothing is logged; then a move ends the sim
        assert_eq!(snapshot(&session), before);
        assert_eq!(session.log.len(), 2);
        run(&mut session, "play 8G AT").unwrap();
        assert!(session.sim.is_none());
        assert!(run(&mut session, "sim drop 8G AT").is_err());
        // with no sim, adding a move the field already holds is refused, and
        // leaves no sim
        run(&mut session, "sim 1").unwrap();
        let sim = session.sim.as_ref().unwrap();
        let first = session
            .play_text(&sim.move_generator.plays[0].play)
            .unwrap();
        run(&mut session, "sim stop").unwrap();
        assert!(run(&mut session, &format!("sim add {first}")).is_err());
        assert!(session.sim.is_none());
    }

    #[test]
    #[inline]
    fn a_given_seats_racks_go_out_as_gcg_and_come_back() {
        let mut first = session();
        for line in [
            "player 2 given",
            "new 7",
            "rack AETAETE",
            "play 8G AT",
            "rack 2 AT",
            "play 9H EA",
            "play H8 TEA",
            "play exch 3",
            "play pass",
            "play exch AE",
        ] {
            run(&mut first, line).unwrap();
        }
        let text = first.gcg_text().unwrap();
        // the known tiles (given AT, plus the E their move needed) and an _ for
        // each of the rest; an exchange of tiles no one saw, as how many; one
        // typed, as its tiles; and no note
        assert!(text.contains(">p2: ATE____ 9H EA +"), "{text}");
        assert!(text.contains(">p2: _______ -3 +0 "), "{text}");
        assert!(text.contains(">p2: AE_____ -AE +0 "), "{text}");
        assert!(!text.contains("#note"), "{text}");
        // the side to move's rack as it is, and none for player 2, who holds
        // nothing known
        let rack = first
            .game_config
            .alphabet()
            .fmt_rack(&first.state.players[0].rack);
        assert!(text.contains(&format!("\n#rack1 {rack}\n")), "{text}");
        assert!(!text.contains("#rack2"), "{text}");
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert!(second.seats[1] == Seat::Given);
        assert_eq!(snapshot(&second), snapshot(&first));
        assert!(second.log.contains(&"phony exch 3".to_string()));
        assert_eq!(second.gcg_text().unwrap(), text);
        // back at their leave, the T: it is written as known, the rest as _
        run(&mut first, "back 4").unwrap();
        let text = first.gcg_text().unwrap();
        assert!(text.contains("\n#rack2 T______\n"), "{text}");
        let mut third = session();
        third.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(snapshot(&third), snapshot(&first));
        assert_eq!(third.state.players[1].rack, [20]);
    }

    #[test]
    #[inline]
    fn a_seat_given_with_no_line_comes_back_given_and_one_known_in_full_human() {
        // player 2 never moved, and the game is over: nothing of theirs was
        // counted, and they have no line, so they come back given (dealt, their
        // rack would count)
        let mut first = session();
        for line in [
            "player 2 given",
            "new 7",
            "rack AETAETE",
            "play 8G AT",
            "end",
        ] {
            run(&mut first, line).unwrap();
        }
        let text = first.gcg_text().unwrap();
        assert!(!text.contains('_'), "{text}");
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert!(second.seats[1] == Seat::Given);
        assert_eq!(second.final_scores, first.final_scores);
        assert_eq!(snapshot(&second), snapshot(&first));
        // their racks given in full: no _ shows it, so they come back human,
        // with each rack and the end's count as the file gives them
        let mut first = session();
        for line in [
            "player 2 given",
            "new 7",
            "rack AETAETE",
            "play 8G AT",
            "rack 2 AETAETE",
            "play 9H EA",
            "end",
        ] {
            run(&mut first, line).unwrap();
        }
        let text = first.gcg_text().unwrap();
        assert!(text.contains(">p2: AETAETE 9H EA +"), "{text}");
        assert!(!text.contains('_'), "{text}");
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert!(second.seats[1] == Seat::Human);
        assert_eq!(second.final_scores, first.final_scores);
        assert_eq!(snapshot(&second), snapshot(&first));
        assert_eq!(second.gcg_text().unwrap(), text);
    }

    #[test]
    #[inline]
    fn six_scoreless_turns_end_the_gcg_with_each_rack_and_what_it_costs() {
        let mut first = session();
        run(&mut first, "new 7").unwrap();
        for _ in 0..6 {
            run(&mut first, "play pass").unwrap();
        }
        let text = first.gcg_text().unwrap();
        let alphabet = first.game_config.alphabet();
        for (seat, player) in first.state.players.iter().enumerate() {
            let rack = alphabet.fmt_rack(&player.rack).to_string();
            assert!(
                text.contains(&format!(">p{}: {rack} ({rack}) -", seat + 1)),
                "{text}"
            );
        }
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(second.final_scores, first.final_scores);
        assert_eq!(snapshot(&second), snapshot(&first));
    }

    #[test]
    #[inline]
    fn a_rack_shown_at_a_scoreless_end_goes_out_as_gcg_and_comes_back() {
        let mut first = session();
        for line in ["player 2 given", "new 7", "rack AETAETE", "play 8G AT"] {
            run(&mut first, line).unwrap();
        }
        for _ in 0..6 {
            run(&mut first, "play pass").unwrap();
        }
        // player 2's rack, nothing known and not shown, has no line and counts
        // nothing
        let ends = |text: &str, player: &str| {
            let lines = text.lines();
            lines
                .filter(|line| line.starts_with(player) && line.contains('('))
                .count()
        };
        let text = first.gcg_text().unwrap();
        assert_eq!((ends(&text, ">p1:"), ends(&text, ">p2:")), (1, 0), "{text}");
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(second.final_scores, first.final_scores);
        // once shown, it has its line, read back as the rack shown
        let alphabet = first.game_config.alphabet();
        let shown = alphabet
            .fmt_rack(&first.state.bag.as_slice()[..7])
            .to_string();
        run(&mut first, &format!("rack 2 {shown}")).unwrap();
        let text = first.gcg_text().unwrap();
        let cost = alphabet.rack_score(&first.state.players[1].rack);
        let line = format!(">p2: {shown} ({shown}) -{cost} ");
        assert!(text.contains(&line), "{text}");
        let mut third = session();
        third.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(third.final_scores, first.final_scores);
        assert!(third.log.contains(&format!("rack 2 {shown}")));
        // a cost the file gets wrong stops the load there
        let wrong = text.replace(&line, &format!(">p2: {shown} ({shown}) -{} ", cost + 1));
        let error = session()
            .load_moves(&gcg_moves(&wrong).unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("its rack counts"), "{error}");
    }

    #[test]
    #[inline]
    fn an_end_said_goes_out_as_gcg_and_comes_back() {
        let mut first = session();
        for line in [
            "player 2 given",
            "new 7",
            "rack AETAETE",
            "play 8G AT",
            "play 9H EA",
            "play H8 TEA",
            "end",
        ] {
            run(&mut first, line).unwrap();
        }
        // player 1's rack counts against them; player 2's, with nothing known,
        // counts nothing
        let alphabet = first.game_config.alphabet();
        let rack = alphabet.fmt_rack(&first.state.players[0].rack).to_string();
        let cost = alphabet.rack_score(&first.state.players[0].rack);
        let total = equity::descale_score(first.final_scores.as_ref().unwrap()[0]);
        let counted = format!(">p1: {rack} ({rack}) -{cost} {total}\n");
        let text = first.gcg_text().unwrap();
        assert!(text.ends_with(&counted), "{text}");
        let loads_back = |text: &str, first: &Session| {
            let mut second = session();
            second.load_moves(&gcg_moves(text).unwrap()).unwrap();
            assert_eq!(second.final_scores, first.final_scores);
            assert_eq!(snapshot(&second), snapshot(first));
            assert_eq!(second.gcg_text().unwrap(), text);
            second
        };
        assert!(loads_back(&text, &first).log.contains(&"end".to_string()));
        // with both shown, each rack counts against its player
        run(&mut first, "rack 2 EE").unwrap();
        let text = first.gcg_text().unwrap();
        assert!(text.contains(&counted), "{text}");
        assert!(text.contains("\n>p2: EE (EE) -2 "), "{text}");
        loads_back(&text, &first);
        // player 2 with no tiles gets twice player 1's rack, the only end line
        run(&mut first, "rack 2 -").unwrap();
        let text = first.gcg_text().unwrap();
        let score = equity::descale_score(first.final_scores.as_ref().unwrap()[1]);
        let twice = format!(">p2: ({rack}) +{} {score}\n", 2 * cost);
        assert!(text.ends_with(&twice), "{text}");
        assert!(!text.contains(&counted), "{text}");
        loads_back(&text, &first);
        // both seats given, nothing known of either: no rack to count
        let mut first = session();
        for line in [
            "player 1 given",
            "player 2 given",
            "new 7",
            "play 8G AT",
            "play 9H EA",
            "end",
        ] {
            run(&mut first, line).unwrap();
        }
        // GCG has no line for that end: it comes back not over, and the same
        // otherwise
        let text = first.gcg_text().unwrap();
        assert!(!text.contains('('), "{text}");
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert!(second.final_scores.is_none());
        assert_eq!(second.gcg_text().unwrap(), text);
        // player 1 has no tiles and nothing is known of player 2's: nothing to
        // count
        run(&mut first, "rack 1 -").unwrap();
        let text = first.gcg_text().unwrap();
        assert!(text.contains("\n>p1: () +0 4\n"), "{text}");
        loads_back(&text, &first);
    }

    #[test]
    #[inline]
    fn a_game_goes_out_as_gcg_and_comes_back_to_the_same_board_and_scores() {
        let mut first = session();
        for line in GAME {
            run(&mut first, line).unwrap();
        }
        for line in ["rack AAA", "play exch AA", "play pass"] {
            run(&mut first, line).unwrap();
        }
        let text = first.gcg_text().unwrap();
        assert!(text.starts_with("#character-encoding UTF-8\n#player1 p1 player 1\n"));
        assert!(text.contains(">p1: AETAETE 8G AT +4 4\n"), "{text}");
        // TEA through the T and the E already on column H
        assert!(text.contains(" H8 ..A +"), "{text}");
        assert!(text.contains(">p2: AAA -AA +0 "), "{text}");
        assert!(text.contains(" - +0 "), "{text}");
        // the game is not over: each rack as it is; and the lexicon, by its
        // file's name
        assert_eq!(text.matches("\n#rack").count(), 2, "{text}");
        assert!(text.contains("\n#lexicon tiny\n"), "{text}");
        let moves = gcg_moves(&text).unwrap();
        // the moves, the racks, the lexicon, and the players
        assert_eq!(moves.len(), 5 + 2 + 1 + 2);
        let lexicon = |m: &GcgMove| matches!(&m.kind, GcgKind::Lexicon(name) if name == "tiny");
        assert!(moves.iter().any(lexicon));
        let mut second = session();
        second.load_moves(&moves).unwrap();
        assert_eq!(snapshot(&second), snapshot(&first));
        assert_eq!(second.gcg_text().unwrap(), text);
        let scores = |s: &Session| s.state.players.iter().map(|p| p.score).collect::<Vec<_>>();
        // logged as the commands that made it: the players' names, a rack and a
        // play for each move, then each rack as it is
        assert!(second.log[0].starts_with("new "));
        assert_eq!(
            second.log[1..3],
            ["nickname 1 p1 player 1", "nickname 2 p2 player 2"]
        );
        assert_eq!(second.log.len(), 3 + 2 * 5 + 2);
        assert_eq!(second.log[3..5], ["rack AETAETE", "phony 8G AT"]);
        // a total that disagrees with its score stops the load there (a score
        // the board does not make is a misscore)
        let wrong = text.replace("8G AT +4 4", "8G AT +5 4");
        let mut third = session();
        let error = third.load_moves(&gcg_moves(&wrong).unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("scores 5 for a total of 5 here, 5 for 4"),
            "{error}"
        );
        // tiles played through in parentheses, as other programs write them,
        // even at the start of the word
        let through = text.replace(" H8 ..A +", " H8 (TE)A +");
        let mut fourth = session();
        fourth.load_moves(&gcg_moves(&through).unwrap()).unwrap();
        assert_eq!(fourth.state.board_tiles, first.state.board_tiles);
        assert_eq!(scores(&fourth), scores(&first));
        assert!(gcg_moves(">p1: ABC (time) +10 5").is_err());
        assert!(gcg_moves(">p1: ABC 8G").is_err());
    }

    #[test]
    #[inline]
    fn a_gcg_from_elsewhere_reads_an_empty_rack_and_the_racks_after_its_last_move() {
        let mut first = session();
        for line in GAME {
            run(&mut first, line).unwrap();
        }
        let text = first.gcg_text().unwrap();
        let scores = |s: &Session| s.state.players.iter().map(|p| p.score).collect::<Vec<_>>();
        // a rack left empty is one no one saw, so the seat is given
        let empty = text.replace(">p2: TEATEAT 9H", ">p2:  9H");
        assert_ne!(empty, text);
        let mut second = session();
        second.load_moves(&gcg_moves(&empty).unwrap()).unwrap();
        assert!(second.seats[1] == Seat::Given);
        assert_eq!(second.state.board_tiles, first.state.board_tiles);
        assert_eq!(scores(&second), scores(&first));
        // the side to move's rack, by #incomplete
        let mut incomplete = text
            .lines()
            .filter(|line| !line.starts_with("#rack"))
            .collect::<Vec<_>>()
            .join("\n");
        incomplete.push_str("\n#incomplete AAEETTT\n");
        let mut third = session();
        third.load_moves(&gcg_moves(&incomplete).unwrap()).unwrap();
        assert_eq!(third.log.last().unwrap(), "rack AAEETTT");
        // #rack1 is #player1's, here the second player to move
        let swapped = "#player1 bob\n#player2 ann\n>ann: AETAETE 8G AT +4 4\n#rack1 EEE\n";
        let moves = gcg_moves(swapped).unwrap();
        let last = moves.last().unwrap();
        assert!(matches!(last.kind, GcgKind::Rack));
        assert_eq!(last.seat, 1);
    }

    #[test]
    #[inline]
    fn a_gcg_reads_in_the_encoding_it_declares() {
        let latin1 = b"#character-encoding ISO-8859-1\n#player1 p1 Jos\xe9\n";
        assert!(gcg_decode(latin1).unwrap().contains("Jos\u{e9}"));
        // with none declared and not UTF-8, ISO 8859-1; with UTF-8 declared, it
        // must be valid UTF-8
        assert!(gcg_decode(&latin1[31..]).unwrap().contains("Jos\u{e9}"));
        assert!(gcg_decode(b"#character-encoding UTF-8\nJos\xe9\n").is_err());
        let utf8 = "#character-encoding UTF-8\nJos\u{e9}\n";
        assert!(gcg_decode(utf8.as_bytes()).unwrap().contains("Jos\u{e9}"));
        assert!(gcg_decode(b"#character-encoding KOI8-R\n").is_err());
        // a UTF-8 byte order mark at the start is dropped, so a file reads the
        // same with it as without, whatever its first line
        for text in [
            "#character-encoding UTF-8\n#player1 jo Jo\n",
            "#player1 jo Jo\n",
            ">jo: AETAETE 8G AT +4 4\n",
        ] {
            let marked = [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat();
            assert_eq!(gcg_decode(&marked).unwrap(), text);
        }
        // an encoding declared after the mark still counts
        assert!(gcg_decode(b"\xef\xbb\xbf#character-encoding UTF-8\nJos\xe9\n").is_err());
    }

    #[test]
    #[inline]
    fn a_misscore_is_kept_apart_and_goes_out_as_gcg_and_back() {
        let mut first = session();
        for line in &GAME[..3] {
            run(&mut first, line).unwrap();
        }
        // AT through the star makes 4; its player was given 5
        assert!(run(&mut first, "adjust 0").is_err());
        run(&mut first, "adjust 1").unwrap();
        assert_eq!(first.state.players[0].score, equity::scale_score(5));
        let turn = &first.game.as_ref().unwrap().turns[0];
        let four = equity::scale_score(4);
        assert!(matches!(turn.play, movegen::Play::Place { score, .. } if score == four));
        // the move as scored, then the board's score in a note
        let text = first.gcg_text().unwrap();
        let lines = ">p1: AETAETE 8G AT +5 5\n#note the correct score is 4\n";
        assert!(text.contains(lines), "{text}");
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(second.gcg_text().unwrap(), text);
        assert!(second.log.contains(&"adjust 1".to_string()));
        // without the note, as other programs write it, the same game loads
        let plain = text.replace("#note the correct score is 4\n", "");
        let mut third = session();
        third.load_moves(&gcg_moves(&plain).unwrap()).unwrap();
        assert_eq!(third.gcg_text().unwrap(), text);
        // a note the board does not agree with stops the load
        let wrong = text.replace("is 4\n", "is 3\n");
        assert!(session().load_moves(&gcg_moves(&wrong).unwrap()).is_err());
        // with a challenge's bonus after it, the note still follows the move's
        // line
        run(&mut second, "bonus 10").unwrap();
        let text = second.gcg_text().unwrap();
        assert!(text.contains(&format!("{lines}>p1: ")), "{text}");
        assert!(text.contains(" (challenge) +10 15\n"), "{text}");
        let mut fourth = session();
        fourth.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(fourth.gcg_text().unwrap(), text);
        // undo takes it back
        run(&mut first, "undo").unwrap();
        assert_eq!(first.state.players[0].score, four);
    }

    #[test]
    #[inline]
    fn notes_on_a_move_go_out_as_gcg_in_the_order_done_and_back() {
        let mut first = session();
        for line in &GAME[..3] {
            run(&mut first, line).unwrap();
        }
        // after the move, after a bonus, after a misscore
        for line in [
            "note a first look",
            "bonus 5",
            "note after   the bonus",
            "adjust 1",
            "note scored as 5",
        ] {
            run(&mut first, line).unwrap();
        }
        // a note that gcg load would read back as the shell's own, a move's
        // correct score, is refused
        assert!(run(&mut first, "note the correct score is 4").is_err());
        run(&mut first, "note the correct score is not 4").unwrap();
        // the correct-score note right after the move's line, then each thing
        // done, in order
        let text = first.gcg_text().unwrap();
        let lines =
            ">p1: AETAETE 8G AT +5 5\n#note the correct score is 4\n#note a first look\n>p1: ";
        assert!(text.contains(lines), "{text}");
        let lines = concat!(
            " (challenge) +5 10\n#note after the bonus\n#note scored as 5\n",
            "#note the correct score is not 4\n"
        );
        assert!(text.contains(lines), "{text}");
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(second.gcg_text().unwrap(), text);
        // undo takes the last note back
        run(&mut first, "undo").unwrap();
        assert!(!first.gcg_text().unwrap().contains("is not 4"), "{text}");
        // a bonus, then the move taken back: the challenge's line first, with
        // the tiles the move left
        let mut third = session();
        for line in GAME[..3].iter().chain(&["bonus 5", "withdraw"]) {
            run(&mut third, line).unwrap();
        }
        let text = third.gcg_text().unwrap();
        let lines = ">p1: AETEE (challenge) +5 9\n>p1: AETAETE -- -4 5\n";
        assert!(text.contains(lines), "{text}");
        let mut fourth = session();
        fourth.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(fourth.gcg_text().unwrap(), text);
        // from other programs: a note before the first move, printed and not
        // kept; a note of three lines; an empty note; and a line after a
        // pragma, which belongs to no note
        let text = concat!(
            "#player1 p1 player 1\n",
            "#player2 p2 player 2\n",
            "#note before any move\n",
            ">p1: AETAETE 8G AT +4 4\n",
            "#note on AT,\n",
            "said over\n",
            "\n",
            "  three lines\n",
            "#note\n",
            "#description not a note\n",
            "nor this\n",
        );
        let mut fifth = session();
        fifth.load_moves(&gcg_moves(text).unwrap()).unwrap();
        let turns = &fifth.game.as_ref().unwrap().turns;
        assert_eq!(turns.len(), 1);
        assert!(turns[0].after == [After::Note("on AT, said over three lines".into())]);
    }

    #[test]
    #[inline]
    fn a_challenge_of_the_words_named_costs_as_the_rule_says_and_goes_out_as_gcg() {
        // under AT, the phony EAEE, making AE and TA too
        let after = |rule: &str| {
            let mut session = session();
            let lines = [rule, "rack EAEE", "phony 9G EAEE"];
            for line in GAME[..3].iter().chain(&lines) {
                run(&mut session, line).unwrap();
            }
            session
        };
        let round_trip = |session: &Session| {
            let text = session.gcg_text().unwrap();
            let mut again = self::session();
            again.load_moves(&gcg_moves(&text).unwrap()).unwrap();
            assert_eq!(again.gcg_text().unwrap(), text);
            text
        };
        // its good words challenged, at 5 a word: the phony stays, and the
        // words challenged are noted
        let mut first = after("challenges bonus 5 word");
        assert!(run(&mut first, "challenge EAT").is_err());
        let before = first.state.players[1].score;
        run(&mut first, "challenge AE ta").unwrap();
        assert_eq!(
            first.state.players[1].score,
            before + equity::scale_score(10)
        );
        let text = round_trip(&first);
        assert!(text.contains(" (challenge) +10 "), "{text}");
        assert!(text.contains("\n#note challenged AE TA\n"), "{text}");
        // the words are chosen up front: once the move stands, the phony not
        // challenged cannot be challenged afterward
        let error = run(&mut first, "challenge EAEE").unwrap_err().to_string();
        assert_eq!(error, "the last move stood a challenge already");
        // challenging the phony, or every word by giving none, takes the move
        // back
        let mut second = after("challenges bonus 5 word");
        run(&mut second, "challenge EAEE").unwrap();
        assert!(second.game.as_ref().unwrap().turns[1].withdrawn());
        let mut third = after("challenges bonus 5 turn");
        run(&mut third, "challenge").unwrap();
        assert!(third.game.as_ref().unwrap().turns[1].withdrawn());
        // a good word challenged under single: no cost, shown as +0
        let mut fourth = after("challenges single");
        let before = fourth.state.players[1].score;
        run(&mut fourth, "challenge TA").unwrap();
        assert_eq!(fourth.state.players[1].score, before);
        let text = round_trip(&fourth);
        assert!(text.contains(" (challenge) +0 "), "{text}");
        for bad in [
            "challenges bonus",
            "challenges bonus 5 words",
            "challenges single 0",
        ] {
            assert!(run(&mut fourth, bad).is_err(), "{bad}");
        }
    }

    #[test]
    #[inline]
    fn the_words_each_move_formed_go_out_a_line_each() {
        // a GCG file's game, read in as gcg words reads one
        let words = |lines: &[&str]| {
            let mut first = session();
            for line in lines {
                run(&mut first, line).unwrap();
            }
            let text = first.gcg_text().unwrap();
            let mut second = session();
            second.printing = false;
            second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
            let mut out = Vec::new();
            second.write_words("g.gcg", &mut out).unwrap();
            String::from_utf8(out).unwrap()
        };
        // the move's own word first, with the tiles it played through, then
        // each hook
        let lines = concat!(
            "g.gcg\t1\tp1\tAT\tmain\n",
            "g.gcg\t2\tp2\tEA\tmain\n",
            "g.gcg\t2\tp2\tTE\tcross\n",
            "g.gcg\t3\tp1\tTEA\tmain\n",
        );
        assert_eq!(words(GAME), lines);
        // a phony taken back: its words, marked so
        let phony = ["rack EEE", "phony H8 TEEE", "challenge"];
        let text = words(&[&GAME[..3], &phony].concat());
        assert!(
            text.ends_with("g.gcg\t2\tp2\tTEEE\tmain\ttaken back\n"),
            "{text}"
        );
    }

    #[test]
    #[inline]
    fn players_nicknamed_go_out_as_gcg_and_come_back() {
        let mut first = session();
        run(&mut first, "nickname 1 ann Ann Example").unwrap();
        run(&mut first, "nickname 2 bob").unwrap();
        for bad in [
            "nickname 3 cy",
            "nickname 1 a:b",
            "nickname 1 #c",
            "nickname 1",
        ] {
            assert!(run(&mut first, bad).is_err(), "{bad}");
        }
        for line in GAME {
            run(&mut first, line).unwrap();
        }
        let text = first.gcg_text().unwrap();
        assert!(
            text.contains("\n#player1 ann Ann Example\n#player2 bob player 2\n"),
            "{text}"
        );
        assert!(text.contains("\n>ann: AETAETE 8G AT +4 4\n"), "{text}");
        assert!(text.contains("\n>bob: TEATEAT 9H EA +"), "{text}");
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(second.gcg_text().unwrap(), text);
        // a move's line tells its player by the nickname alone, so two players
        // cannot share one
        run(&mut second, "nickname 2 ann").unwrap();
        assert!(second.gcg_text().is_err());
    }

    #[test]
    #[inline]
    fn time_comes_off_a_final_score_and_goes_out_as_gcg_and_back() {
        let mut first = session();
        run(&mut first, "new 7").unwrap();
        assert!(run(&mut first, "time 1 10").is_err());
        for _ in 0..6 {
            run(&mut first, "play pass").unwrap();
        }
        let before = first.final_scores.clone().unwrap();
        run(&mut first, "time 2 10").unwrap();
        for bad in ["time 3 10", "time 1 0", "time 1"] {
            assert!(run(&mut first, bad).is_err(), "{bad}");
        }
        let after = first.final_scores.clone().unwrap();
        assert_eq!(after, [before[0], before[1] - equity::scale_score(10)]);
        // kept with the move that ended the game, through back and forward
        run(&mut first, "back").unwrap();
        assert!(first.final_scores.is_none());
        run(&mut first, "forward").unwrap();
        assert_eq!(first.final_scores.as_ref(), Some(&after));
        // its player's last rack, (time), the points, and the final score
        let text = first.gcg_text().unwrap();
        let alphabet = first.game_config.alphabet();
        let rack = alphabet.fmt_rack(&first.state.players[1].rack).to_string();
        let total = equity::descale_score(after[1]);
        assert!(
            text.ends_with(&format!(">p2: {rack} (time) -10 {total}\n")),
            "{text}"
        );
        let mut second = session();
        second.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(second.final_scores, Some(after));
        assert_eq!(second.gcg_text().unwrap(), text);
        // an end that counted no rack has no line, so the time line ends the
        // game
        let mut third = session();
        for line in [
            "player 1 given",
            "player 2 given",
            "new 7",
            "play 8G AT",
            "play 9H EA",
            "end",
            "time 1 10",
        ] {
            run(&mut third, line).unwrap();
        }
        let text = third.gcg_text().unwrap();
        assert!(text.contains(" (time) -10 "), "{text}");
        let mut fourth = session();
        fourth.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(fourth.final_scores, third.final_scores);
        assert_eq!(fourth.gcg_text().unwrap(), text);
    }

    #[test]
    #[inline]
    fn an_endgame_is_solved_once_the_bag_is_empty() {
        let mut session = session();
        run(&mut session, "new 7").unwrap();
        let error = run(&mut session, "endgame").unwrap_err().to_string();
        assert!(error.contains("bag is not empty"), "{error}");
        // the bag is empty: every tile but AT (the side to move's) and E is on
        // the board, in rows from the top, so that neither rack can make a word
        // on it
        let game_config = session.game_config;
        let alphabet = game_config.alphabet();
        let mut tiles = Vec::new();
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                tiles.push(if tile == 0 { 0x81 } else { tile });
            }
        }
        for held in [1, 20, 5] {
            let at = tiles.iter().position(|&t| t == held).unwrap();
            tiles.remove(at);
        }
        let mut state = game_state::GameState::new(game_config);
        state.board_tiles[..tiles.len()].copy_from_slice(&tiles);
        state.bag.set_from_iter(std::iter::empty());
        state.players[0].rack = vec![1, 20];
        state.players[1].rack = vec![5];
        session.state = state;
        run(&mut session, "endgame 5").unwrap();
        let (mut solver, solved) = solve_endgame(
            session.game_config,
            session.kwg().unwrap(),
            session.arm_tables().unwrap(),
            &session.state,
            &endgame::Limits {
                time: Some(std::time::Duration::from_secs(5)),
                states: None,
                memory: None,
            },
            false,
        )
        .unwrap();
        assert!(solved.exact);
        // nothing can be played, so the side to move passes
        let mut line = Vec::new();
        solver.collect_pv(0, &mut line);
        assert!(matches!(&line[0].1, movegen::Play::Exchange { tiles } if tiles.is_empty()));
        // the line as the moves list shows it, its players numbered as the
        // shell numbers them
        let text = session.endgame_line(&mut solver, &session.state).unwrap();
        assert!(text[0].starts_with("  1. player 1 AT "), "{text:?}");
        assert!(text[1].starts_with("  2. player 2 E "), "{text:?}");
        assert!(matches!(
            session.computer_move(Some(100.0)).unwrap(),
            movegen::Play::Exchange { tiles } if tiles.is_empty()
        ));
        // with player 2's seat given and nothing known, the bag's one tile, all
        // that is left, is theirs, also as the side to move (the position is
        // set here, not rebuilt, so whose racks are given is set with it)
        session.seats[1] = Seat::Given;
        session.racks_given[1] = true;
        let e = std::mem::take(&mut session.state.players[1].rack);
        session.state.bag.return_tiles(&e);
        let state = session.endgame_state();
        assert!(state.bag.is_empty());
        assert_eq!(state.players[1].rack, [5]);
        run(&mut session, "endgame 5").unwrap();
        session.state.turn = 1;
        assert_eq!(session.endgame_state().players[1].rack, [5]);
        run(&mut session, "endgame 5").unwrap();
        // both seats given: whose the tile is is not known, so there is no
        // endgame
        session.seats[0] = Seat::Given;
        session.racks_given[0] = true;
        let error = run(&mut session, "endgame 5").unwrap_err().to_string();
        assert!(error.contains("bag is not empty"), "{error}");
    }

    #[test]
    #[inline]
    fn the_computer_infers_nothing_for_a_solved_endgame() {
        // the position of the test above, after the opponent passed: the bag is
        // empty, and the board holds every tile but the side to move's AT and
        // E; an inference would read that pass
        let mut session = session();
        run(&mut session, "new 7").unwrap();
        let game_config = session.game_config;
        let alphabet = game_config.alphabet();
        let mut tiles = Vec::new();
        for tile in 0..alphabet.len() {
            for _ in 0..alphabet.freq(tile) {
                tiles.push(if tile == 0 { 0x81 } else { tile });
            }
        }
        for held in [1, 20, 5] {
            let at = tiles.iter().position(|&t| t == held).unwrap();
            tiles.remove(at);
        }
        let mut state = game_state::GameState::new(game_config);
        state.board_tiles[..tiles.len()].copy_from_slice(&tiles);
        state.bag.set_from_iter(std::iter::empty());
        state.players[0].rack = vec![1, 20];
        state.players[1].rack = vec![5];
        session.board_before_last = Some((
            state.board_tiles.to_vec(),
            movegen::Play::Exchange {
                tiles: [][..].into(),
            },
        ));
        session.state = state;
        assert!(matches!(
            session.computer_move(Some(100.0)).unwrap(),
            movegen::Play::Exchange { tiles } if tiles.is_empty()
        ));
        // the solve reads no kept leaves, so none were inferred: the inference
        // tables are built at their first use
        assert!(session.infer_tables.is_none());
    }

    #[test]
    #[inline]
    fn a_phony_challenged_off_goes_back_and_a_word_that_stands_costs_the_rule() {
        let mut session = session();
        for line in &GAME[..3] {
            run(&mut session, line).unwrap();
        }
        run(&mut session, "rack EEE").unwrap();
        let before = snapshot(&session);
        // TEEE has four letters, and none of the tiny lexicon's words do
        run(&mut session, "phony H8 TEEE").unwrap();
        run(&mut session, "challenge").unwrap();
        let after = snapshot(&session);
        assert_eq!(
            (&after.0, &after.1, &after.2),
            (&before.0, &before.1, &before.2)
        );
        assert_eq!(after.3, 0, "the phony's player has had their turn");
        assert!(session.game.as_ref().unwrap().turns[1].withdrawn());
        assert!(
            run(&mut session, "challenge").is_err(),
            "nothing new to challenge"
        );
        // a word that stands: five points to its player by default
        run(&mut session, "play 9H EA").unwrap();
        let points = session.state.players[0].score;
        run(&mut session, "challenge").unwrap();
        assert_eq!(
            session.state.players[0].score - points,
            equity::scale_score(5)
        );
        assert_eq!(session.state.turn, 1);
        // a move is challenged only once: once it stands, there is no second
        // challenge
        let error = run(&mut session, "challenge").unwrap_err().to_string();
        assert_eq!(error, "the last move stood a challenge already");
        // double: the challenger loses the turn
        run(&mut session, "challenges double").unwrap();
        run(&mut session, "rack EAT").unwrap();
        run(&mut session, "play 10H AT").unwrap();
        run(&mut session, "challenge").unwrap();
        assert_eq!(session.state.turn, 1, "player 1 lost the turn");
        // a turn lost, or any move that placed no tiles, has no word to
        // challenge, nor a phony to take back
        let error = run(&mut session, "challenge").unwrap_err().to_string();
        assert_eq!(error, "the last move made no word to challenge");
        let error = run(&mut session, "withdraw").unwrap_err().to_string();
        assert_eq!(error, "only a move that placed tiles is taken back");
        // the log replays to the same game, and so does its GCG
        let mut second = Session::new(true, None).unwrap();
        second.use_kwg("tiny", session.kwg().unwrap());
        for line in session.log.clone() {
            replay(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&second), snapshot(&session));
        let text = session.gcg_text().unwrap();
        assert!(text.contains(">p2: EEE H8 .EEE +"), "{text}");
        assert!(text.contains(">p2: EEE -- -"), "{text}");
        assert!(text.contains(" (challenge) +5 "), "{text}");
        let mut third = Session::new(true, None).unwrap();
        third.use_kwg("tiny", session.kwg().unwrap());
        third.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(snapshot(&third), snapshot(&session));
        assert_eq!(third.gcg_text().unwrap(), text);
        // a file may give one move more than one challenge line (a server bug
        // once gave four): they are read as given, and written back the same
        // way
        let mut fourth = self::session();
        for line in GAME[..3].iter().chain(&["bonus 5"; 4]) {
            run(&mut fourth, line).unwrap();
        }
        let text = fourth.gcg_text().unwrap();
        assert_eq!(text.matches(" (challenge) +5 ").count(), 4, "{text}");
        let mut fifth = self::session();
        fifth.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(snapshot(&fifth), snapshot(&fourth));
        assert_eq!(fifth.gcg_text().unwrap(), text);
    }

    #[test]
    #[inline]
    fn a_phony_said_to_go_out_and_challenged_off_ends_nothing() {
        let mut session = session();
        for line in [
            "player 2 given",
            "new 7",
            "rack TEAT",
            "phony 8G TEAT",
            "end",
        ] {
            run(&mut session, line).unwrap();
        }
        assert!(session.final_scores.is_some());
        run(&mut session, "challenge").unwrap();
        assert!(session.final_scores.is_none());
        assert!(!session.game.as_ref().unwrap().turns[0].ended());
    }

    #[test]
    #[inline]
    fn a_triple_challenge_ends_the_game_against_whoever_was_wrong() {
        // AT stands: player 2, who challenged it, loses
        let mut first = session();
        for line in &GAME[..3] {
            run(&mut first, line).unwrap();
        }
        run(&mut first, "challenges triple").unwrap();
        run(&mut first, "challenge").unwrap();
        assert_eq!(first.loser, Some(1));
        assert!(first.final_scores.is_some());
        assert!(run(&mut first, "play 9H EA").is_err(), "the game is over");
        assert!(run(&mut first, "rack 2 EA").is_err(), "no rack counts");
        assert!(run(&mut first, "challenge").is_err(), "lost already");
        assert!(run(&mut first, "lost 1").is_err(), "lost already");
        // its GCG: a loss counts no rack, and GCG has no line for the loss, so
        // it comes back not over, with the same board and scores
        let loads_back = |text: &str, first: &Session| {
            let mut second = session();
            second.load_moves(&gcg_moves(text).unwrap()).unwrap();
            assert_eq!(second.loser, None);
            assert!(second.final_scores.is_none());
            let (board, players, ..) = snapshot(&second);
            let scores = players.iter().map(|p| p.0).collect::<Vec<_>>();
            let (first_board, first_players, ..) = snapshot(first);
            assert_eq!(board, first_board);
            assert_eq!(
                scores,
                first_players.iter().map(|p| p.0).collect::<Vec<_>>()
            );
        };
        let text = first.gcg_text().unwrap();
        assert!(text.ends_with(">p1: AETAETE 8G AT +4 4\n"), "{text}");
        loads_back(&text, &first);
        // TEEE does not stand: it is taken back, and its player, player 2,
        // loses
        let mut second = session();
        for line in &GAME[..3] {
            run(&mut second, line).unwrap();
        }
        for line in [
            "challenges triple",
            "rack EEE",
            "phony H8 TEEE",
            "challenge",
        ] {
            run(&mut second, line).unwrap();
        }
        assert_eq!(second.loser, Some(1));
        assert!(second.game.as_ref().unwrap().turns[1].withdrawn());
        let text = second.gcg_text().unwrap();
        assert!(text.ends_with(">p2: EEE -- -4 0\n"), "{text}");
        loads_back(&text, &second);
        let mut third = session();
        for line in second.log.clone() {
            replay(&mut third, &line).unwrap();
        }
        assert_eq!(third.loser, Some(1));
        assert_eq!(snapshot(&third), snapshot(&second));
        // taking the challenged move back takes the end back with it
        run(&mut second, "back").unwrap();
        assert_eq!(second.loser, None);
        assert!(second.final_scores.is_none());
        // by hand, the same loss
        run(&mut second, "forward").unwrap();
        assert!(run(&mut second, "lost 3").is_err());
        let mut fourth = session();
        for line in &GAME[..3] {
            run(&mut fourth, line).unwrap();
        }
        run(&mut fourth, "lost 2").unwrap();
        assert_eq!(fourth.loser, Some(1));
        assert_eq!(fourth.final_scores, first.final_scores);
    }

    #[test]
    #[inline]
    fn undo_takes_back_the_last_thing_done_to_the_move_shown() {
        let at = |s: &Session| s.game.as_ref().map(|game| (game.at, game.turns.len()));
        let rack_given = |s: &Session| s.game.as_ref().is_some_and(|game| game.rack.is_some());
        let mut first = session();
        for line in GAME {
            run(&mut first, line).unwrap();
        }
        // goto N shows the game after move N; goto 0 shows its start
        run(&mut first, "goto 1").unwrap();
        assert_eq!(at(&first), Some((1, 3)));
        assert!(run(&mut first, "goto 4").is_err());
        run(&mut first, "goto 3").unwrap();
        // at the end, undo takes back, in turn: the end declared, the bonus,
        // then TEA (given no rack, so its turn goes), then EA (its rack given
        // again), then that rack
        let before = first.state.players[0].score;
        run(&mut first, "bonus 5").unwrap();
        run(&mut first, "end").unwrap();
        assert!(first.final_scores.is_some());
        run(&mut first, "undo").unwrap();
        assert!(first.final_scores.is_none());
        run(&mut first, "undo").unwrap();
        assert_eq!(first.state.players[0].score, before);
        assert_eq!(at(&first), Some((3, 3)));
        run(&mut first, "undo").unwrap();
        assert_eq!(at(&first), Some((2, 2)));
        assert!(!rack_given(&first));
        run(&mut first, "undo").unwrap();
        assert_eq!(at(&first), Some((1, 1)));
        assert!(rack_given(&first));
        run(&mut first, "undo").unwrap();
        assert!(!rack_given(&first));
        for _ in 0..2 {
            run(&mut first, "undo").unwrap();
        }
        assert_eq!(at(&first), Some((0, 0)));
        // at the start, undo takes back the racks set there, then has nothing
        // left
        run(&mut first, "rack 2 EEE").unwrap();
        assert_eq!(first.game.as_ref().unwrap().start.len(), 1);
        run(&mut first, "undo").unwrap();
        assert!(first.game.as_ref().unwrap().start.is_empty());
        assert!(run(&mut first, "undo").is_err());
        // undoing the taking back of a phony makes it stand again, and brings
        // back what the taking back undid with it, a rack set after it
        let mut second = session();
        for line in &GAME[..3] {
            run(&mut second, line).unwrap();
        }
        for line in ["rack EEE", "phony H8 TEEE", "rack 2 EE", "challenge"] {
            run(&mut second, line).unwrap();
        }
        assert!(second.game.as_ref().unwrap().turns[1].withdrawn());
        assert_eq!(second.state.players[1].rack, [5, 5, 5]);
        run(&mut second, "undo").unwrap();
        assert!(!second.game.as_ref().unwrap().turns[1].withdrawn());
        assert_eq!(second.state.players[1].rack, [5, 5]);
        // away from the end, the moves after the one shown are kept as a line
        let mut third = session();
        for line in GAME {
            run(&mut third, line).unwrap();
        }
        run(&mut third, "goto 2").unwrap();
        run(&mut third, "undo").unwrap();
        assert_eq!(at(&third), Some((1, 1)));
        run(&mut third, "line 1").unwrap();
        assert_eq!(at(&third), Some((1, 3)));
        // a bot to move after an undo waits
        let mut fourth = session();
        for line in ["player 2 bot", "new 7", "rack AETAETE", "play 8G AT"] {
            run(&mut fourth, line).unwrap();
        }
        assert_eq!(at(&fourth), Some((2, 2)));
        run(&mut fourth, "undo").unwrap();
        assert_eq!(at(&fourth), Some((1, 1)));
        // the log replays to the same game
        let mut fifth = session();
        for line in second.log.clone() {
            replay(&mut fifth, &line).unwrap();
        }
        assert_eq!(snapshot(&fifth), snapshot(&second));
    }

    #[test]
    #[inline]
    fn a_what_if_keeps_the_line_it_cut_to_go_back_to() {
        let mut first = session();
        for line in GAME {
            run(&mut first, line).unwrap();
        }
        let end = snapshot(&first);
        run(&mut first, "back 2").unwrap();
        run(&mut first, "rack EAT").unwrap();
        run(&mut first, "play 9G EAT").unwrap();
        let what_if = snapshot(&first);
        assert_eq!(first.game.as_ref().unwrap().lines.len(), 1);
        // the line that was cut, back in place, shown where it branched
        run(&mut first, "line 1").unwrap();
        assert_eq!(first.game.as_ref().unwrap().at, 1);
        run(&mut first, "forward 2").unwrap();
        assert_eq!(snapshot(&first), end);
        // and the what-if, kept in its place
        run(&mut first, "line 1").unwrap();
        run(&mut first, "forward").unwrap();
        assert_eq!(snapshot(&first), what_if);
        assert!(run(&mut first, "line 2").is_err());
        // the log replays to the same game, lines and all
        let mut second = session();
        for line in first.log.clone() {
            replay(&mut second, &line).unwrap();
        }
        assert_eq!(snapshot(&second), what_if);
        assert_eq!(second.game.as_ref().unwrap().lines.len(), 1);
    }

    #[test]
    #[inline]
    fn a_line_comes_back_whole_after_a_what_if_before_it() {
        let mut first = session();
        for line in GAME {
            run(&mut first, line).unwrap();
        }
        let end = snapshot(&first);
        // a what-if for the third move (AT under EA), then one for the second
        // (EAT, with the A under the T), which takes away the second move the
        // first line followed
        for line in [
            "back",
            "rack EAT",
            "play 10H AT",
            "back 2",
            "rack EAT",
            "play 9G EAT",
        ] {
            run(&mut first, line).unwrap();
        }
        assert_eq!(first.game.as_ref().unwrap().lines.len(), 2);
        // the game as first played, whole, shown where it parts from this one
        run(&mut first, "line 1").unwrap();
        assert_eq!(first.game.as_ref().unwrap().at, 1);
        run(&mut first, "forward 2").unwrap();
        assert_eq!(snapshot(&first), end);
    }

    #[test]
    #[inline]
    fn a_line_keeps_the_racks_set_before_its_first_move() {
        let mut first = session();
        for line in GAME {
            run(&mut first, line).unwrap();
        }
        let end = snapshot(&first);
        // back at the start, setting a rack for player 2 makes a what-if from
        // there
        run(&mut first, "back 3").unwrap();
        run(&mut first, "rack 2 EAT").unwrap();
        let what_if = snapshot(&first);
        assert_eq!(first.game.as_ref().unwrap().lines.len(), 1);
        // the game as played, without that rack, and back again
        run(&mut first, "line 1").unwrap();
        assert!(first.game.as_ref().unwrap().start.is_empty());
        run(&mut first, "forward 3").unwrap();
        assert_eq!(snapshot(&first), end);
        run(&mut first, "line 1").unwrap();
        assert_eq!(snapshot(&first), what_if);
        assert_eq!(first.game.as_ref().unwrap().lines.len(), 1);
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
        assert_eq!(error, "not in the lexicon: TEEE (phony plays it anyway)");
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
        // and in GCG, there and back
        let text = first.gcg_text().unwrap();
        assert!(text.contains(">p1: AETAETEATE 8G AT +4 4\n"), "{text}");
        assert!(text.contains(">p2: EA 9H EA +"), "{text}");
        let mut third = session();
        third.load_moves(&gcg_moves(&text).unwrap()).unwrap();
        assert_eq!(snapshot(&third), snapshot(&first));
    }
}
