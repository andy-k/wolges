// Copyright (C) 2020-2026 Andy Kurnia.

use rand::SeedableRng;
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Write},
    num::NonZeroU32,
    process::ExitCode,
};
use wolges::{
    anagrams::ArmTables,
    endgame::{EndgameSolver, Limits},
    equity::Equity,
    error,
    game_config::{GameConfig, make_english_game_config},
    game_state::{CheckGameEnded, GameState},
    klv::Klv,
    kwg::{self, Kwg, Node24, RunLetters},
    movegen::{BoardSnapshot, GenMovesParams, KurniaMoveGenerator, PassPolicy, Play},
};

const USAGE: &str = "aspiration_sweep corpus <kwg> <32-byte seed> <games>\n\
aspiration_sweep solve <kwg> <positions.jsonl> [start width in points]\n\
aspiration_sweep sweep <kwg> <positions.jsonl> <width>...\n\
aspiration_sweep sweep <kwg> <positions.jsonl> <first>..=<last>";

#[derive(Deserialize, Serialize)]
struct Position {
    game: usize,
    ply: usize,
    mover: u8,
    board: Vec<u8>,
    racks: [Vec<u8>; 2],
    passes: u16,
    zeros: u16,
}

fn autoplay(
    gc: &GameConfig,
    kwg: &std::sync::Arc<Kwg<Node24>>,
    seed: &str,
    games: usize,
    out: &mut impl Write,
) -> error::Returns<()> {
    let seed = seed
        .as_bytes()
        .try_into()
        .map_err(|_| "seed must be 32 bytes")?;
    let mut rng = Xoshiro256PlusPlus::from_seed(seed);
    let mut game = GameState::new(gc);
    let klv = Klv::<Node24>::empty()?;
    let mut generator = KurniaMoveGenerator::new(gc);
    let vocab = wolges::movegen::Vocab::new(gc, kwg)?;
    let mut scores = vec![0; usize::from(gc.num_players())];
    for game_idx in 0..games {
        game.reset_and_draw_tiles_double_ended(gc, &mut rng);
        for ply in 0.. {
            if game.bag.is_empty() {
                serde_json::to_writer(
                    &mut *out,
                    &Position {
                        game: game_idx,
                        ply,
                        mover: game.turn,
                        board: game.board_tiles.to_vec(),
                        racks: [game.players[0].rack.clone(), game.players[1].rack.clone()],
                        passes: game.pass_turns,
                        zeros: game.zero_turns,
                    },
                )?;
                writeln!(out)?;
            }
            generator.gen_moves_unfiltered(&GenMovesParams {
                board_snapshot: &BoardSnapshot {
                    board_tiles: &game.board_tiles,
                    game_config: gc,
                    vocab: &vocab,
                    anagrams: None,
                    rack_lengths: None,
                    klv: &klv,
                },
                rack: &game.current_player().rack,
                max_gen: 1,
                num_exchanges_by_this_player: game.current_player().num_exchanges,
                pass_policy: PassPolicy::OnlyWhenForced,
                dynamic_leaves: None,
            })?;
            game.play(gc, &mut rng, &generator.plays[0].play)?;
            if !matches!(
                game.check_game_ended(gc, &mut scores)?,
                CheckGameEnded::NotEnded
            ) {
                break;
            }
            game.next_turn()?;
        }
    }
    Ok(())
}

fn read_positions(path: &str, gc: &GameConfig) -> error::Returns<Vec<Position>> {
    let dim = gc.board_layout().dim();
    let area = dim.rows as usize * dim.cols as usize;
    let records = BufReader::new(File::open(path)?);
    let mut positions = Vec::new();
    for (index, line) in records.lines().enumerate() {
        let pos: Position = serde_json::from_str(&line?)?;
        if pos.mover >= gc.num_players()
            || pos.board.len() != area
            || pos.board.iter().any(|&tile| {
                let letter = tile & 0x7f;
                tile != 0 && (letter == 0 || letter >= gc.alphabet().len())
            })
            || pos.racks.iter().any(|rack| {
                rack.len() > usize::from(gc.rack_size())
                    || rack.iter().any(|&tile| tile >= gc.alphabet().len())
            })
        {
            return Err(format!("invalid endgame position on line {}", index + 1).into());
        }
        positions.push(pos);
    }
    Ok(positions)
}

struct Measured {
    value: Equity,
    nodes: u64,
    first: Option<Play>,
}

struct Search<'a> {
    gc: &'a GameConfig,
    vocab: wolges::movegen::Vocab<'a, Node24>,
    tables: ArmTables,
    run_letters: RunLetters<'a>,
}

impl<'a> Search<'a> {
    fn new(gc: &'a GameConfig, kwg: &'a std::sync::Arc<Kwg<Node24>>) -> error::Returns<Self> {
        let tables = ArmTables::build(kwg, gc, false)?;
        let run_letters = match tables.rack_lengths() {
            Some(racks) => racks.run_letters().of(kwg)?,
            None => kwg.run_letters(),
        };
        Ok(Self {
            gc,
            vocab: wolges::movegen::Vocab::new(gc, kwg)?,
            tables,
            run_letters,
        })
    }

    fn measure(&self, pos: &Position, width: Option<NonZeroU32>) -> error::Returns<Measured> {
        let mut solver = EndgameSolver::<Node24, Node24>::with_generator(
            self.gc,
            &self.vocab,
            &self.tables,
            self.run_letters.clone(),
            KurniaMoveGenerator::new(self.gc),
        )?;
        solver.init_after_scoreless_turns(
            &pos.board,
            [&pos.racks[0], &pos.racks[1]],
            pos.passes,
            pos.zeros,
        );
        let solved = match width {
            Some(width) => solver.solve_with_aspiration_window_points(pos.mover, width)?,
            None => solver.solve_within(pos.mover, &Limits::NONE)?,
        };
        let Some(value) = solved.value.filter(|_| solved.exact) else {
            return Err(format!("game {}, ply {} did not solve exactly", pos.game, pos.ply).into());
        };
        let nodes = solver.searched();
        let mut pv = Vec::new();
        solver.collect_pv(pos.mover, &mut pv)?;
        Ok(Measured {
            value,
            nodes,
            first: pv.into_iter().next().map(|(_, play)| play),
        })
    }
}

fn write_rows(
    search: &Search<'_>,
    positions: &[Position],
    width: Option<NonZeroU32>,
    out: &mut impl Write,
) -> error::Returns<()> {
    for pos in positions {
        let Measured {
            value,
            nodes,
            first,
        } = search.measure(pos, width)?;
        let play = first.as_ref().map(|play| match play {
            Play::Exchange { tiles } => serde_json::json!(["exchange", &**tiles]),
            Play::Place {
                down,
                lane,
                idx,
                word,
                score,
            } => serde_json::json!(["place", down, lane, idx, &**word, score.raw()]),
        });
        write!(
            out,
            "{}\t{}\t{}\t{}\t{nodes}\t",
            pos.game,
            pos.ply,
            pos.mover,
            value.raw()
        )?;
        serde_json::to_writer(&mut *out, &play)?;
        writeln!(out)?;
    }
    Ok(())
}

fn sweep_widths(args: &[String]) -> error::Returns<Box<dyn Iterator<Item = NonZeroU32>>> {
    if args.is_empty() {
        return Err(USAGE.into());
    }
    if let [range] = args
        && let Some((first, last)) = range.split_once("..=")
    {
        let first = first.parse::<NonZeroU32>()?.get();
        let last = last.parse::<NonZeroU32>()?.get();
        return Ok(Box::new((first..=last).filter_map(NonZeroU32::new)));
    }
    let widths = args
        .iter()
        .map(|arg| arg.parse())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Box::new(widths.into_iter()))
}

fn sweep(
    search: &Search<'_>,
    positions: &[Position],
    widths: impl Iterator<Item = NonZeroU32>,
    out: &mut impl Write,
) -> error::Returns<()> {
    let reference = positions
        .iter()
        .map(|pos| search.measure(pos, None))
        .collect::<error::Returns<Vec<_>>>()?;
    writeln!(out, "positions\t{}", positions.len())?;
    writeln!(
        out,
        "width\tnodes\texact values match\tfirst-play differences"
    )?;
    for width in widths {
        let mut nodes = 0u64;
        let mut changed = 0;
        for (pos, want) in positions.iter().zip(&reference) {
            let got = search.measure(pos, Some(width))?;
            if got.value != want.value {
                return Err(format!(
                    "width {width} differs at game {}, ply {}",
                    pos.game, pos.ply
                )
                .into());
            }
            nodes += got.nodes;
            changed += usize::from(got.first != want.first);
        }
        writeln!(out, "{width}\t{nodes}\tyes\t{changed}")?;
        out.flush()?;
    }
    Ok(())
}

fn run(args: &[String], out: &mut impl Write) -> error::Returns<()> {
    if args == ["--help"] {
        writeln!(out, "{USAGE}")?;
        return Ok(());
    }
    let [mode, lexicon, args @ ..] = args else {
        return Err(USAGE.into());
    };
    let gc = make_english_game_config();
    let kwg = std::sync::Arc::new(kwg::kbwg_from_bytes_alloc(std::fs::read(lexicon)?, false)?);
    if !kwg.fits_alphabet(gc.alphabet().len()) {
        return Err("the kwg has tiles outside the English alphabet".into());
    }
    match (mode.as_str(), args) {
        ("corpus", [seed, games]) => autoplay(&gc, &kwg, seed, games.parse()?, out),
        ("solve", [path] | [path, _]) => {
            let width = args.get(1).map(|width| width.parse()).transpose()?;
            write_rows(
                &Search::new(&gc, &kwg)?,
                &read_positions(path, &gc)?,
                width,
                out,
            )
        }
        ("sweep", [path, widths @ ..]) => {
            let widths = sweep_widths(widths)?;
            sweep(
                &Search::new(&gc, &kwg)?,
                &read_positions(path, &gc)?,
                widths,
                out,
            )
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let out = io::stdout().lock();
    let mut out = BufWriter::new(out);
    match run(&args, &mut out).and_then(|()| Ok(out.flush()?)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(out, "{err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{read_positions, sweep_widths};
    use std::num::NonZeroU32;

    #[test]
    fn an_empty_corpus_reads_as_no_positions() {
        let dir = format!(
            "{}/target/aspiration_sweep-{}",
            env!("CARGO_MANIFEST_DIR"),
            std::process::id()
        );
        std::fs::create_dir_all(&dir).unwrap();
        let path = format!("{dir}/empty.jsonl");
        std::fs::write(&path, "").unwrap();
        let gc = wolges::game_config::make_english_game_config();
        assert!(read_positions(&path, &gc).unwrap().is_empty());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn sweep_widths_follow_the_request() {
        let parsed = |args: &[&str]| {
            sweep_widths(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
                .map(|widths| widths.map(NonZeroU32::get).collect::<Vec<_>>())
        };
        assert_eq!(parsed(&["3", "1", "3"]).unwrap(), [3, 1, 3]);
        assert_eq!(parsed(&["2..=4"]).unwrap(), [2, 3, 4]);
        assert!(parsed(&["2..=1"]).unwrap().is_empty());
        assert_eq!(
            parsed(&[&format!("{0}..={0}", u32::MAX)]).unwrap(),
            [u32::MAX]
        );
        for args in [&[][..], &["0"], &["0..=1"], &["2..=4", "5"]] {
            assert!(parsed(args).is_err());
        }
    }
}
