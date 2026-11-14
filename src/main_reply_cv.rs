// Copyright (C) 2020-2026 Andy Kurnia.

// reply-cv: how much the exact 1-ply, as a control variate, would narrow a
// sim's estimates. Each position is a game played greedily from a seeded deal
// for some turns. Its top candidates by static equity each get the exact mean
// of the opponent's best reply (infer::reply_to, timed), then iterations as
// wolges-wasm runs its sims (one deal an iteration for every candidate, the
// lean arm): each rollout's objective, spread, and chance to win, with its
// reply's equity at ply 1 (Simmer::reply_equity). It prints how many plain
// iterations one with the control variate is worth (1 / (1 - rho^2)), for each
// candidate alone and as its difference from the best, and the same with the
// exact reply's time counted against the rollouts' (net).
//
//   reply-cv <kwg> <klv2> <candidates> <iterations> <plies|max> <seed:turns>...
use rand::prelude::*;
use std::time::Instant;
use wolges::{anagrams, error, game_config, game_state, infer, klv, kwg, movegen, simmer};

type Kwg = kwg::Kwg<kwg::Node24>;
type Klv = klv::Klv<kwg::Node24>;

// an iteration's seed, as wolges-wasm's iteration_seed makes it. The mix is
// splitmix64's:
// https://prng.di.unimi.it/splitmix64.c
fn iteration_seed(seed: u64, iteration: u64) -> u64 {
    let mut z = seed.wrapping_add(iteration.wrapping_mul(0x9e3779b97f4a7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn snapshot<'a>(
    gc: &'a game_config::GameConfig,
    kwg: &'a Kwg,
    klv: &'a Klv,
    board_tiles: &'a [u8],
) -> movegen::BoardSnapshot<'a, kwg::Node24, kwg::Node24> {
    movegen::BoardSnapshot {
        board_tiles,
        game_config: gc,
        kwg,
        anagrams: None,
        rack_lengths: None,
        klv,
    }
}

// a game played greedily (the generator's best by static equity) for so many
// turns from the deal this seed makes
fn position(
    gc: &game_config::GameConfig,
    kwg: &Kwg,
    klv: &Klv,
    seed: u64,
    turns: usize,
) -> error::Returns<game_state::GameState> {
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut gs = game_state::GameState::new(gc);
    gs.reset_and_draw_tiles(gc, &mut rng);
    let mut mg = movegen::KurniaMoveGenerator::new(gc);
    let mut final_scores = vec![0; gc.num_players() as usize];
    for _ in 0..turns {
        mg.gen_moves_unfiltered_lean(&movegen::GenMovesParams {
            board_snapshot: &snapshot(gc, kwg, klv, &gs.board_tiles),
            rack: &gs.current_player().rack,
            max_gen: 1,
            num_exchanges_by_this_player: gs.current_player().num_exchanges,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let play = mg.plays[0].play.clone();
        gs.play(gc, &mut rng, &play)?;
        if !matches!(
            gs.check_game_ended(gc, &mut final_scores)?,
            game_state::CheckGameEnded::NotEnded
        ) {
            return Err("the game ended".into());
        }
        gs.next_turn();
    }
    Ok(gs)
}

// the position as a sim sees it (as wolges-wasm's sim_new makes it): the
// mover's rack, the bag in tile order holding every tile the mover cannot see,
// the other seats dealt from it
fn sim_state(gc: &game_config::GameConfig, real: &game_state::GameState) -> game_state::GameState {
    let turn = real.turn as usize;
    let mut gs = game_state::GameState::new(gc);
    gs.turn = real.turn;
    gs.zero_turns = real.zero_turns;
    gs.pass_turns = real.pass_turns;
    gs.board_tiles.copy_from_slice(&real.board_tiles);
    gs.players[turn]
        .rack
        .extend_from_slice(&real.players[turn].rack);
    let mut unseen = vec![0u8; gc.alphabet().len() as usize];
    let others = (real.players.iter().enumerate())
        .filter(|&(i, _)| i != turn)
        .flat_map(|(_, player)| player.rack.iter());
    for &tile in real.bag.as_slice().iter().chain(others) {
        unseen[tile as usize] += 1;
    }
    gs.bag.set_from_iter(
        (0u8..)
            .zip(unseen.iter())
            .flat_map(|(tile, &count)| std::iter::repeat_n(tile, count as usize)),
    );
    for i in 0..real.players.len() {
        gs.players[i].score = real.players[i].score;
        if i != turn {
            let n = real.players[i].rack.len();
            gs.bag.replenish(&mut gs.players[i].rack, n, i);
        }
    }
    gs
}

struct Moments {
    n: f64,
    mx: f64,
    my: f64,
    vx: f64,
    vy: f64,
    cxy: f64,
}

fn moments(xs: &[f64], ys: &[f64]) -> Moments {
    let n = xs.len() as f64;
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let (mut vx, mut vy, mut cxy) = (0.0, 0.0, 0.0);
    for (&x, &y) in xs.iter().zip(ys) {
        vx += (x - mx) * (x - mx);
        vy += (y - my) * (y - my);
        cxy += (x - mx) * (y - my);
    }
    Moments {
        n,
        mx,
        my,
        vx: vx / (n - 1.0),
        vy: vy / (n - 1.0),
        cxy: cxy / (n - 1.0),
    }
}

impl Moments {
    fn rho(&self) -> f64 {
        self.cxy / (self.vx * self.vy).sqrt()
    }

    // how many plain iterations the control variate's are worth: 1 / (1 - rho^2)
    fn worth(&self) -> f64 {
        1.0 / (1.0 - self.rho() * self.rho())
    }

    // worth, the exact reply's time counted: the plain iterations the same
    // wall time buys, over those spent on the rollouts alone
    fn worth_per_second(&self, rollouts_secs: f64, reply_secs: f64) -> f64 {
        self.worth() * rollouts_secs / (rollouts_secs + reply_secs)
    }

    // the estimate of x's mean with y's exact mean known
    fn adjusted(&self, my_exact: f64) -> f64 {
        self.mx - self.cxy / self.vy * (self.my - my_exact)
    }
}

fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() < 7 {
        return Err(
            "reply-cv <kwg> <klv2> <candidates> <iterations> <plies|max> <seed:turns>...".into(),
        );
    }
    let kwg_bytes = std::fs::read(&args[1])?;
    let kwg = Kwg::from_reader::<kwg::Node22>(kwg_bytes.len(), |at, piece| {
        piece.copy_from_slice(&kwg_bytes[at..at + piece.len()])
    })?;
    let klv = klv::kbwg_from_bytes_alloc(std::fs::read(&args[2])?)?;
    let num_candidates = args[3].parse::<usize>()?;
    let iterations = args[4].parse::<u64>()?;
    let plies = if args[5] == "max" {
        usize::MAX
    } else {
        args[5].parse::<usize>()?
    };
    let gc = game_config::make_english_game_config();
    let mut tables = infer::InferTables::new(&gc, &klv)?;
    let mut mg = movegen::KurniaMoveGenerator::new(&gc);
    let no_tables = &anagrams::ArmTables::None;
    println!(
        "kwg {} klv {} candidates {num_candidates} iterations {iterations} plies {}",
        args[1], args[2], args[5]
    );
    for spec in &args[6..] {
        let (seed, turns) = spec.split_once(':').ok_or("seed:turns")?;
        let (seed, turns) = (seed.parse::<u64>()?, turns.parse::<usize>()?);
        let real = position(&gc, &kwg, &klv, seed, turns)?;
        let turn = real.turn as usize;
        let rack = real.players[turn].rack.clone();
        println!();
        println!(
            "position {spec}: bag {} rack {:?} scores {:?}",
            real.bag.len(),
            rack,
            real.players
                .iter()
                .map(|p| p.score / 1000)
                .collect::<Vec<_>>()
        );
        mg.gen_moves_unfiltered_lean(&movegen::GenMovesParams {
            board_snapshot: &snapshot(&gc, &kwg, &klv, &real.board_tiles),
            rack: &rack,
            max_gen: num_candidates,
            num_exchanges_by_this_player: real.players[turn].num_exchanges,
            pass_policy: movegen::PassPolicy::OnlyWhenForced,
            dynamic_leaves: None,
        });
        let mut candidates = mg.plays.clone();
        candidates.sort_by_key(|m| std::cmp::Reverse(m.equity));
        // each candidate's exact mean reply, in points, and its time
        let mut exact = Vec::with_capacity(candidates.len());
        for candidate in &candidates {
            let start = Instant::now();
            let reply = infer::reply_to(
                &mut tables,
                &mut mg,
                &snapshot(&gc, &kwg, &klv, &real.board_tiles),
                &rack,
                &[],
                &candidate.play,
                1,
            )?;
            exact.push((reply, start.elapsed().as_secs_f64()));
        }
        let sgs = sim_state(&gc, &real);
        let mut sim = simmer::Simmer::new(&gc)?;
        sim.prepare(&gc, &sgs, plies, true, &[]);
        let pristine = sim.prepared_state().clone();
        // per candidate: objective, spread (points), win, reply equity (points)
        let mut rec = vec![(Vec::new(), Vec::new(), Vec::new(), Vec::new()); candidates.len()];
        let mut no_reply = vec![0u64; candidates.len()];
        let start = Instant::now();
        for iteration in 1..=iterations {
            sim.restore_prepared(&pristine);
            sim.reseed(iteration_seed(seed, iteration));
            sim.prepare_iteration();
            for (c, candidate) in candidates.iter().enumerate() {
                let ended = sim.simulate_lean(&gc, &kwg, no_tables, &klv, &candidate.play)?;
                let final_spread = sim.final_equity_spread()?;
                let win = sim.compute_win_prob(ended, final_spread, None);
                let spread = final_spread - sim.initial_score_spread;
                let objective = simmer::sim_objective(spread, win, sim.win_prob_weightage());
                match sim.reply_equity() {
                    Some(equity) => {
                        rec[c].0.push(objective);
                        rec[c].1.push(spread as f64 / 1000.0);
                        rec[c].2.push(win);
                        rec[c].3.push(equity.as_f64());
                    }
                    None => no_reply[c] += 1,
                }
            }
        }
        let rollout_secs =
            start.elapsed().as_secs_f64() / (iterations * candidates.len() as u64) as f64;
        let rollouts_secs = rollout_secs * iterations as f64;
        println!(
            "rollout {:.3} ms; weightage {}",
            rollout_secs * 1e3,
            sim.win_prob_weightage()
        );
        println!(
            "  # static  exact_reply  reply_ms  mean_reply  z      obj      obj_cv   worth_obj  net_obj  worth_spread  worth_win  no_reply"
        );
        for (c, candidate) in candidates.iter().enumerate() {
            let Some(reply) = exact[c].0 else {
                println!("{c:3} {} no exact reply", candidate.equity);
                continue;
            };
            let (obj, spread, win, y) = &rec[c];
            let m_obj = moments(obj, y);
            let m_spread = moments(spread, y);
            let m_win = moments(win, y);
            let z = (m_obj.my - reply.mean) / (m_obj.vy / m_obj.n).sqrt();
            println!(
                "{c:3} {:7.3} {:11.3} {:9.1} {:11.3} {:6.2} {:8.3} {:8.3} {:10.3} {:8.3} {:13.3} {:10.3} {:9}",
                candidate.equity.as_f64(),
                reply.mean,
                exact[c].1 * 1e3,
                m_obj.my,
                z,
                m_obj.mx,
                m_obj.adjusted(reply.mean),
                m_obj.worth(),
                m_obj.worth_per_second(rollouts_secs, exact[c].1),
                m_spread.worth(),
                m_win.worth(),
                no_reply[c]
            );
        }
        // each candidate against the best by the plain mean objective: the
        // differences, as a sim ranks its candidates
        let best = (0..candidates.len())
            .filter(|&c| !rec[c].0.is_empty())
            .max_by(|&a, &b| {
                let ma = rec[a].0.iter().sum::<f64>();
                let mb = rec[b].0.iter().sum::<f64>();
                ma.total_cmp(&mb)
            });
        if let Some(best) = best {
            println!(
                "  against candidate {best}: diff, diff_cv, worth_diff, net_diff, worth_diff_spread, worth_diff_win"
            );
            let diff = |xs: &[f64], ys: &[f64]| {
                (xs.iter().zip(ys)).map(|(a, b)| a - b).collect::<Vec<_>>()
            };
            for c in 0..candidates.len() {
                if c == best || exact[c].0.is_none() || rec[c].0.len() != rec[best].0.len() {
                    continue;
                }
                let e = diff(&rec[c].3, &rec[best].3);
                let m = moments(&diff(&rec[c].0, &rec[best].0), &e);
                let m_spread = moments(&diff(&rec[c].1, &rec[best].1), &e);
                let m_win = moments(&diff(&rec[c].2, &rec[best].2), &e);
                let exact_e = exact[c].0.unwrap().mean - exact[best].0.unwrap().mean;
                println!(
                    "{c:3} {:8.3} {:8.3} {:8.3} {:8.3} {:8.3} {:8.3}",
                    m.mx,
                    m.adjusted(exact_e),
                    m.worth(),
                    m.worth_per_second(2.0 * rollouts_secs, exact[c].1 + exact[best].1),
                    m_spread.worth(),
                    m_win.worth()
                );
            }
        }
    }
    Ok(())
}
