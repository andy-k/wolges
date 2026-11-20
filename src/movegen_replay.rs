// Copyright (C) 2020-2026 Andy Kurnia.

use crate::{game_args, parse_rack};
use std::io::{BufRead, Write};
use std::time::Instant;
use wolges::{anagrams, display, error, game_config, klv, kwg, movegen, return_error};

#[derive(clap::Args)]
#[command(about = "time top-one static move generation on a frozen chronological trace")]
pub(crate) struct ReplayArgs {
    kwg: String,
    klv: String,
    #[arg(help = "the trace as JSON lines")]
    trace: String,
    #[arg(long)]
    passes: std::num::NonZeroUsize,
    #[arg(long, help = "start each position with a fresh generator")]
    cold: bool,
}

#[derive(serde::Deserialize)]
struct Position {
    game: u64,
    ply: u32,
    fen: String,
    racks: [String; 2],
    turn: u8,
    bag: usize,
    exchanges: i16,
    reference: String,
    equity: i32,
}

struct Engine {
    game: game_config::GameConfig,
    kwg: kwg::Kwg<kwg::Node24>,
    klv: klv::Klv<kwg::Node24>,
    tables: anagrams::ArmTables,
}

impl Engine {
    fn load(
        options: &game_args::GameArgs,
        kbwg: bool,
        kwg_path: &str,
        klv_path: &str,
    ) -> error::Returns<Self> {
        let game = options.make_game_config()?;
        game.supports(game_config::Op::Generation)?;
        let dim = game.board_layout().dim();
        if (dim.rows, dim.cols, game.num_players()) != (15, 15, 2) {
            return_error!(
                "the chronological trace uses a 15 by 15 board and two players".to_string()
            );
        }
        let kwg = kwg::kbwg_from_bytes_alloc(std::fs::read(kwg_path)?, kbwg)?;
        if !kwg.fits_alphabet(game.alphabet().len()) {
            return_error!(format!("{kwg_path} has tiles outside the game's alphabet"));
        }
        let klv = klv::kbwg_from_bytes_alloc(std::fs::read(klv_path)?)?;
        game.check_klv(&klv)?;
        let tables = anagrams::ArmTables::build(&kwg, &game, false)?;
        Ok(Self {
            game,
            kwg,
            klv,
            tables,
        })
    }

    fn snapshot<'a>(
        &'a self,
        board: &'a [u8],
    ) -> movegen::BoardSnapshot<'a, kwg::Node24, kwg::Node24> {
        movegen::BoardSnapshot {
            board_tiles: board,
            game_config: &self.game,
            kwg: &self.kwg,
            klv: &self.klv,
            anagrams: self.tables.anagrams(),
            rack_lengths: self.tables.rack_lengths(),
        }
    }
}

fn generate(
    generator: &mut movegen::InlineGenerator<225>,
    snapshot: &movegen::BoardSnapshot<'_, kwg::Node24, kwg::Node24>,
    rack: &[u8],
    exchanges: i16,
) {
    generator.gen_moves_unfiltered(&movegen::GenMovesParams {
        board_snapshot: snapshot,
        rack,
        max_gen: 1,
        num_exchanges_by_this_player: exchanges,
        pass_policy: movegen::PassPolicy::OnlyWhenForced,
        dynamic_leaves: None,
    });
}

struct Prepared<'a> {
    position: Position,
    snapshot: movegen::BoardSnapshot<'a, kwg::Node24, kwg::Node24>,
    rack: Vec<u8>,
}

pub(crate) fn replay(
    options: &game_args::GameArgs,
    kbwg: bool,
    args: &ReplayArgs,
) -> error::Returns<()> {
    let started = Instant::now();
    let engine = Engine::load(options, kbwg, &args.kwg, &args.klv)?;
    let setup = started.elapsed();
    let alphabet = engine.game.alphabet();
    let mut parser = display::BoardFenParser::new(alphabet, engine.game.board_layout());
    let mut positions = Vec::new();
    let mut boards = Vec::new();
    for line in std::io::BufReader::new(std::fs::File::open(&args.trace)?).lines() {
        let line = line?;
        let position: Position = serde_json::from_str(&line)?;
        boards.push(parser.parse(&position.fen)?.to_vec());
        positions.push(position);
    }
    if positions.is_empty() {
        return_error!("the trace has no positions".to_string());
    }
    let mut prepared = Vec::with_capacity(positions.len());
    for (position, board) in positions.into_iter().zip(boards.iter()) {
        let mut pool = engine.game.tiles_off_board(board)?;
        let mut racks = position
            .racks
            .each_ref()
            .map(|rack| parse_rack(alphabet, rack));
        for rack in &racks {
            let Ok(rack) = rack else {
                return_error!(format!(
                    "invalid rack in game {} ply {}",
                    position.game, position.ply
                ));
            };
            if rack.len() > engine.game.rack_size() as usize {
                return_error!(format!(
                    "oversized rack in game {} ply {}",
                    position.game, position.ply
                ));
            }
            for &tile in rack {
                pool[tile as usize] = pool[tile as usize]
                    .checked_sub(1)
                    .ok_or("rack uses a tile outside the pool")?;
            }
        }
        if position.bag != pool.iter().map(|&count| count as usize).sum::<usize>() {
            return_error!(format!(
                "bag count differs in game {} ply {}",
                position.game, position.ply
            ));
        }
        let rack = std::mem::replace(
            racks
                .get_mut(position.turn as usize)
                .ok_or("invalid turn in trace")?,
            Ok(Vec::new()),
        )?;
        prepared.push(Prepared {
            position,
            snapshot: engine.snapshot(board),
            rack,
        });
    }

    let mut generator = movegen::InlineGenerator::<225>::try_new(&engine.game)?;
    let mut ties = 0;
    let mut elapsed = Vec::with_capacity(args.passes.get());
    for pass in 0..=args.passes.get() {
        let mut search_time = std::time::Duration::ZERO;
        for entry in &prepared {
            if args.cold {
                generator = movegen::InlineGenerator::try_new(&engine.game)?;
            }
            let started = Instant::now();
            generate(
                &mut generator,
                &entry.snapshot,
                &entry.rack,
                entry.position.exchanges,
            );
            search_time += started.elapsed();
            if pass == 0 {
                let best = &generator.plays[0];
                if best.equity.raw() != entry.position.equity {
                    return_error!(format!(
                        "game {} ply {}: {} has equity {}, expected {} for {}",
                        entry.position.game,
                        entry.position.ply,
                        best.play.fmt(&entry.snapshot),
                        best.equity,
                        wolges::equity::Equity::new(entry.position.equity),
                        entry.position.reference,
                    ));
                }
                ties += usize::from(
                    best.play.fmt(&entry.snapshot).to_string() != entry.position.reference,
                );
            }
        }
        if pass != 0 {
            elapsed.push(search_time.as_nanos());
        }
    }
    writeln!(
        std::io::stdout().lock(),
        "{}",
        serde_json::json!({
            "positions": prepared.len(),
            "equal_equity_different_move": ties,
            "setup_ns": setup.as_nanos(),
            "search_ns": elapsed,
            "cold": args.cold,
        }),
    )?;
    Ok(())
}
