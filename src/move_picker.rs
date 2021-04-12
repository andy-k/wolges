// Copyright (C) 2020-2021 Andy Kurnia.

use super::{game_config, game_state, klv, kwg, move_filter, movegen, simmer, stats};

struct Candidate {
    play_index: usize,
    stats: stats::Stats,
}

pub struct Simmer<'a> {
    game_config: &'a game_config::GameConfig<'a>,
    kwg: &'a kwg::Kwg,
    klv: &'a klv::Klv,
    candidates: Vec<Candidate>,
    simmer: simmer::Simmer,
}

impl<'a> Simmer<'a> {
    pub fn new(
        game_config: &'a game_config::GameConfig,
        kwg: &'a kwg::Kwg,
        klv: &'a klv::Klv,
    ) -> Self {
        Self {
            game_config,
            kwg,
            klv,
            candidates: Vec::new(),
            simmer: simmer::Simmer::new(game_config),
        }
    }

    #[inline(always)]
    fn take_candidates(&mut self, num_plays: usize) -> Vec<Candidate> {
        let mut candidates = std::mem::take(&mut self.candidates);
        candidates.clear();
        candidates.reserve(num_plays);
        for idx in 0..num_plays {
            candidates.push(Candidate {
                play_index: idx,
                stats: stats::Stats::new(),
            });
        }
        candidates
    }
}

#[inline(always)]
fn top_candidate_play_index_by_mean(candidates: &[Candidate]) -> usize {
    candidates
        .iter()
        .max_by(|a, b| a.stats.mean().partial_cmp(&b.stats.mean()).unwrap())
        .unwrap()
        .play_index
}

pub struct Periods(pub u64);

impl Periods {
    #[inline(always)]
    pub fn update(&mut self, new_periods: u64) -> bool {
        if new_periods != self.0 {
            self.0 = new_periods;
            true
        } else {
            false
        }
    }
}

#[allow(clippy::large_enum_variant)]
pub enum MovePicker<'a> {
    Hasty,
    Simmer(Simmer<'a>),
}

unsafe impl Send for MovePicker<'_> {}

impl MovePicker<'_> {
    #[inline(always)]
    fn limit_surviving_candidates(
        candidates: &mut Vec<Candidate>,
        z: f64,
        max_candidates_allowed: usize,
    ) {
        while candidates.len() > max_candidates_allowed {
            // pruning regularly means binary heap is not justified here.
            // also, this means candidates are almost indistinguishable.
            candidates.swap_remove(
                candidates
                    .iter()
                    .enumerate()
                    .map(|(i, candidate)| (i, candidate.stats.ci_max(-z)))
                    .min_by(|(_, a), (_, b)| a.partial_cmp(&b).unwrap())
                    .unwrap()
                    .0,
            );
        }
    }

    #[inline(always)]
    pub async fn pick_a_move_async(
        &mut self,
        filtered_movegen: &mut move_filter::GenMoves<'_>,
        mut move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot<'_>,
        game_state: &game_state::GameState,
        rack: &[u8],
    ) {
        match self {
            MovePicker::Hasty => {
                filtered_movegen.gen_moves(&mut move_generator, board_snapshot, &rack, 1);
            }
            MovePicker::Simmer(simmer) => {
                let t0 = std::time::Instant::now();
                tokio::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;
                    println!("3 secs have passed");
                });
                filtered_movegen.gen_moves(&mut move_generator, board_snapshot, &rack, 100);
                simmer.simmer.prepare(simmer.game_config, &game_state, 2);
                let mut candidates = simmer.take_candidates(move_generator.plays.len());
                let num_sim_iters = 1000;
                let mut tick_periods = Periods(0);
                let mut prune_periods = Periods(0);
                let max_time_for_move_ms = 8000u64;
                let prune_interval_ms =
                    std::cmp::max(1, max_time_for_move_ms / candidates.len() as u64);
                const Z: f64 = 1.96; // 95% confidence interval
                for sim_iter in 1..=num_sim_iters {
                    tokio::task::yield_now().await;
                    let elapsed_time_ms = t0.elapsed().as_millis() as u64;
                    if tick_periods.update(elapsed_time_ms / 1000) {
                        println!(
                            "After {} seconds, doing iteration {} with {} candidates",
                            tick_periods.0,
                            sim_iter,
                            candidates.len()
                        );
                    }
                    simmer.simmer.prepare_iteration();
                    for candidate in candidates.iter_mut() {
                        let game_ended = simmer.simmer.simulate(
                            simmer.game_config,
                            simmer.kwg,
                            simmer.klv,
                            &move_generator.plays[candidate.play_index].play,
                        );
                        let final_spread = simmer.simmer.final_equity_spread();
                        let win_prob = simmer.simmer.compute_win_prob(game_ended, final_spread);
                        let sim_spread = final_spread - simmer.simmer.initial_score_spread as f32;
                        candidate.stats.update(
                            sim_spread as f64 + win_prob * simmer.simmer.win_prob_weightage(),
                        );
                    }
                    if sim_iter % 16 == 0
                        && prune_periods.update(elapsed_time_ms / prune_interval_ms)
                    {
                        let low_bar = candidates
                            .iter()
                            .map(|candidate| candidate.stats.ci_max(-Z))
                            .max_by(|a, b| a.partial_cmp(&b).unwrap())
                            .unwrap();
                        candidates.retain(|candidate| candidate.stats.ci_max(Z) >= low_bar);
                        Self::limit_surviving_candidates(
                            &mut candidates,
                            Z,
                            1 + (2 * max_time_for_move_ms.saturating_sub(elapsed_time_ms)
                                / prune_interval_ms) as usize,
                        );
                        if candidates.len() < 2 {
                            break;
                        }
                    }
                }
                let top_idx = candidates
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.stats.mean().partial_cmp(&b.stats.mean()).unwrap())
                    .unwrap()
                    .0;
                println!(
                    "top candidate mean = {} (sd={} count={} range {}..{}) took {:?}",
                    candidates[top_idx].stats.mean(),
                    candidates[top_idx].stats.standard_deviation(),
                    candidates[top_idx].stats.count(),
                    candidates[top_idx].stats.ci_max(-Z),
                    candidates[top_idx].stats.ci_max(Z),
                    t0.elapsed()
                );
                assert_eq!(
                    candidates[top_idx].play_index,
                    top_candidate_play_index_by_mean(&candidates)
                );
                move_generator
                    .plays
                    .swap(0, top_candidate_play_index_by_mean(&candidates));
                move_generator.plays.truncate(1);
                simmer.candidates = candidates;
            }
        }
    }

    #[inline(always)]
    #[tokio::main]
    pub async fn pick_a_move(
        &mut self,
        filtered_movegen: &mut move_filter::GenMoves<'_>,
        move_generator: &mut movegen::KurniaMoveGenerator,
        board_snapshot: &movegen::BoardSnapshot<'_>,
        game_state: &game_state::GameState,
        rack: &[u8],
    ) {
        self.pick_a_move_async(
            filtered_movegen,
            move_generator,
            board_snapshot,
            game_state,
            rack,
        )
        .await
    }
}
