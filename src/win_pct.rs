// Copyright (C) 2020-2026 Andy Kurnia.

// Empirical win-probability table backfilled from self-play, keyed by the
// count-state (bag, my, opp) -- the bag size and the two rack sizes -- that
// the player on move faces. It replaces a fixed sigmoid as a win_prob
// estimator and gives the census a win%-objective.
//
// Storage is a RAW SPARSE INTEGER histogram: per key, the (delta, count) pairs
// where future swings landed, where delta = final_spread - snapshot_spread
// from the mover's view. This is the composable num/den primitive -- merging
// two runs just adds counts -- and it is lossless and self-describing (no
// fixed spread cap or unseen grid is baked in; a key spans exactly its
// observed deltas).
//
// Method (the cumulative trick): static self-play is score-independent, so the
// trajectory from a state onward does not depend on the lead at that state.
// One game therefore informs every hypothetical lead: a state with lead s wins
// iff the future swing exceeds -s. Hence
//   win%(s, key) = P(delta > -s | key) + 0.5 P(delta == -s | key),
// monotone nondecreasing in s by construction. The reader symmetrizes each
// key's histogram (folding delta with -delta) so win%(0, key) == 0.5 and
// win%(s, key) + win%(-s, key) == 1 exactly, then reverse-cumulates into a
// dense per-key row for O(1) get.
//
// Why (bag, my, opp) and not a scalar unseen count: a scalar comingles the
// rack split in the endgame, which is exactly where a win_prob estimate
// matters most. The state space is L-shaped (bag > 0 implies both racks are
// full, since you draw back up; only when the bag empties do racks deplete),
// so the keyed form is the scalar midgame and exact in the endgame at little
// extra cost.

use std::collections::{BTreeMap, HashMap};

// (bag, my, opp). bag is u16 to cover larger tile sets; rack sizes fit u8.
pub type Key = (u16, u8, u8);

// CSV format tag and version on the self-describing header line.
const CSV_TAG: &str = "winpct";
const CSV_VERSION: &str = "2";

// Accumulates the raw per-key delta histogram. Composable via merge.
#[derive(Default)]
pub struct WinPctAccumulator {
    rows: BTreeMap<Key, BTreeMap<i32, u64>>,
}

impl WinPctAccumulator {
    pub fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
        }
    }

    // Record one snapshot from the mover's view: the player on move held a lead
    // of `spread` at count-state (bag, my, opp) and finished at `final_spread`.
    // Tallies the raw future swing delta = final_spread - spread.
    pub fn record(&mut self, bag: usize, my: usize, opp: usize, spread: i32, final_spread: i32) {
        let key = (bag as u16, my as u8, opp as u8);
        *self
            .rows
            .entry(key)
            .or_default()
            .entry(final_spread - spread)
            .or_insert(0) += 1;
    }

    // Add another accumulator's counts into this one (the composable merge).
    pub fn merge(&mut self, other: &WinPctAccumulator) {
        for (key, hist) in &other.rows {
            let dst = self.rows.entry(*key).or_default();
            for (&delta, &count) in hist {
                *dst.entry(delta).or_insert(0) += count;
            }
        }
    }

    // Raw sparse CSV: a structured header line, then one line per key
    //   bag,my,opp,total,delta:count,delta:count,...
    // (deltas ascending; total = sum of counts, a redundant checksum).
    pub fn to_csv(&self) -> String {
        use std::fmt::Write as _;
        let mut out = format!("{CSV_TAG},{CSV_VERSION},bag,my,opp\n");
        for (&(bag, my, opp), hist) in &self.rows {
            if hist.is_empty() {
                continue;
            }
            let total: u64 = hist.values().sum();
            let _ = write!(out, "{bag},{my},{opp},{total}");
            for (&delta, &count) in hist {
                let _ = write!(out, ",{delta}:{count}");
            }
            out.push('\n');
        }
        out
    }

    pub fn from_csv(s: &str) -> crate::error::Returns<WinPctAccumulator> {
        let mut lines = s.lines().filter(|l| !l.trim().is_empty());
        let header = lines
            .next()
            .ok_or_else(|| crate::error::new("win_pct: empty csv".into()))?;
        let mut h = header.trim().split(',');
        if h.next() != Some(CSV_TAG) || h.next() != Some(CSV_VERSION) {
            return_error!("win_pct: bad csv header tag/version".into());
        }
        if h.next() != Some("bag") || h.next() != Some("my") || h.next() != Some("opp") {
            return_error!("win_pct: csv header dims must be bag,my,opp".into());
        }
        let mut acc = WinPctAccumulator::new();
        for line in lines {
            let mut it = line.trim().split(',');
            let bag: u16 = it.next().unwrap_or("").parse()?;
            let my: u8 = it.next().unwrap_or("").parse()?;
            let opp: u8 = it.next().unwrap_or("").parse()?;
            let total: u64 = it.next().unwrap_or("").parse()?;
            let hist = acc.rows.entry((bag, my, opp)).or_default();
            let mut sum = 0u64;
            for tok in it {
                let (d, c) = tok
                    .split_once(':')
                    .ok_or_else(|| format!("win_pct: bad pair {tok:?}"))?;
                let delta: i32 = d.parse()?;
                let count: u64 = c.parse()?;
                *hist.entry(delta).or_insert(0) += count;
                sum += count;
            }
            if sum != total {
                return_error!(format!(
                    "win_pct: key ({bag},{my},{opp}) total {total} != sum {sum}"
                ));
            }
        }
        Ok(acc)
    }

    // Symmetrize and reverse-cumulate each key into a dense O(1) lookup row.
    pub fn finalize(&self) -> WinPctTable {
        let mut rows = HashMap::with_capacity(self.rows.len());
        for (&key, hist) in &self.rows {
            // cap = widest observed |delta|; the symmetric row spans [-cap, cap].
            let cap = match hist.keys().map(|d| d.unsigned_abs()).max() {
                Some(c) => c as i32,
                None => continue,
            };
            let width = (2 * cap + 1) as usize;
            let mut sym = vec![0u64; width];
            for (&delta, &count) in hist {
                sym[(delta + cap) as usize] += count;
                sym[(-delta + cap) as usize] += count;
            }
            let total = sym.iter().sum::<u64>() as f64;
            // For lead s the break-even swing is delta == -s; win iff delta > -s
            // with half credit at equality. Walk spreads s from -cap to cap;
            // index i = s + cap, break bucket j = width - 1 - i.
            let mut win = vec![0.0f32; width];
            let mut strictly_greater = 0u64;
            for i in 0..width {
                let h = sym[width - 1 - i];
                win[i] = ((strictly_greater as f64 + 0.5 * h as f64) / total) as f32;
                strictly_greater += h;
            }
            rows.insert(key, DenseRow { cap, win });
        }
        WinPctTable { rows }
    }
}

// Per key, a dense win% row over the key's observed spread range [-cap, cap];
// queries past either end saturate to 0.0 / 1.0.
struct DenseRow {
    cap: i32,
    win: Vec<f32>,
}

// Finalized win-probability lookup.
pub struct WinPctTable {
    rows: HashMap<Key, DenseRow>,
}

impl WinPctTable {
    // P(mover wins | lead `spread`, count-state (bag, my, opp)). An unsampled
    // key returns 0.5; a spread past the key's observed range saturates.
    pub fn get(&self, spread: i32, bag: usize, my: usize, opp: usize) -> f32 {
        self.get_opt(spread, bag, my, opp).unwrap_or(0.5)
    }

    // Like get, but distinguishes an unsampled key (None) from a sampled 0.5, so
    // a caller can fall back to its own estimator only where the table has no
    // data. A spread past the key's observed range still saturates to 0.0 / 1.0.
    pub fn get_opt(&self, spread: i32, bag: usize, my: usize, opp: usize) -> Option<f32> {
        match self.rows.get(&(bag as u16, my as u8, opp as u8)) {
            None => None,
            Some(row) if spread > row.cap => Some(1.0),
            Some(row) if spread < -row.cap => Some(0.0),
            Some(row) => Some(row.win[(spread + row.cap) as usize]),
        }
    }

    pub fn from_csv(s: &str) -> crate::error::Returns<WinPctTable> {
        Ok(WinPctAccumulator::from_csv(s)?.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    // Future swings symmetric about zero give a monotone, antisymmetric
    // table that is 0.5 at lead 0 and saturates past the range.
    #[test]
    fn finalize_is_monotone_symmetric_and_half_at_zero() {
        let mut acc = WinPctAccumulator::new();
        for &v in &[-30, -10, 10, 30] {
            acc.record(50, 7, 7, 0, v);
        }
        let t = acc.finalize();
        assert!(
            (t.get(0, 50, 7, 7) - 0.5).abs() < EPS,
            "got {}",
            t.get(0, 50, 7, 7)
        );
        // a lead past the widest observed swing is a certain win (and loss).
        assert!((t.get(999, 50, 7, 7) - 1.0).abs() < EPS);
        assert!((t.get(-999, 50, 7, 7) - 0.0).abs() < EPS);
        // monotone nondecreasing in spread.
        let mut prev = -1.0f32;
        for s in -60..=60 {
            let w = t.get(s, 50, 7, 7);
            assert!(w >= prev - EPS, "not monotone at s={s}: {w} < {prev}");
            prev = w;
        }
        // antisymmetric.
        for s in [7, 23, 41] {
            assert!((t.get(s, 50, 7, 7) + t.get(-s, 50, 7, 7) - 1.0).abs() < EPS);
        }
    }

    // A single observed swing informs every hypothetical lead, with a half
    // credit at the exact break-even (tie) and saturation just past it.
    #[test]
    fn cumulative_informs_all_leads() {
        let mut acc = WinPctAccumulator::new();
        acc.record(60, 7, 7, 0, -45); // one game swung -45 from this state.
        let t = acc.finalize();
        // lead 50: even the -45 swing leaves +5 -> certain win (past the range).
        assert!(
            (t.get(50, 60, 7, 7) - 1.0).abs() < EPS,
            "got {}",
            t.get(50, 60, 7, 7)
        );
        // lead 46: -45 swing leaves +1 -> still a win (past the range).
        assert!((t.get(46, 60, 7, 7) - 1.0).abs() < EPS);
        // lead 45: -45 swing is an exact tie -> half credit.
        assert!(
            (t.get(45, 60, 7, 7) - 0.75).abs() < EPS,
            "got {}",
            t.get(45, 60, 7, 7)
        );
        // lead 40: -45 swing loses, +45 swing wins -> 0.5.
        assert!(
            (t.get(40, 60, 7, 7) - 0.5).abs() < EPS,
            "got {}",
            t.get(40, 60, 7, 7)
        );
    }

    // Spreads past the observed range saturate to the extremes.
    #[test]
    fn get_saturates_out_of_range() {
        let mut acc = WinPctAccumulator::new();
        for &v in &[-30, -10, 10, 30] {
            acc.record(50, 7, 7, 0, v);
        }
        let t = acc.finalize();
        assert!((t.get(999_999, 50, 7, 7) - 1.0).abs() < EPS);
        assert!((t.get(-999_999, 50, 7, 7) - 0.0).abs() < EPS);
        // the widest observed lead is not yet a certain win (opp can still tie).
        assert!(
            (t.get(30, 50, 7, 7) - 0.875).abs() < EPS,
            "got {}",
            t.get(30, 50, 7, 7)
        );
    }

    // An unsampled key falls back to 0.5 everywhere.
    #[test]
    fn absent_key_is_half() {
        let mut acc = WinPctAccumulator::new();
        acc.record(50, 7, 7, 0, 10);
        let t = acc.finalize();
        for s in [-200, -1, 0, 1, 200] {
            assert!((t.get(s, 7, 7, 7) - 0.5).abs() < EPS, "key (7,7,7) s={s}");
        }
    }

    // Distinct keys keep their own swings -- one key's data does not
    // bleed into another. (50,7,7) sees tight swings; (50,6,7) wide ones, so the
    // same +10 lead is a certain win at the tight key but a coin flip at the
    // wide one.
    #[test]
    fn distinct_keys_independent() {
        let mut acc = WinPctAccumulator::new();
        for &v in &[-5, 5] {
            acc.record(50, 7, 7, 0, v);
        }
        for &v in &[-100, 100] {
            acc.record(50, 6, 7, 0, v);
        }
        let t = acc.finalize();
        assert!(
            (t.get(10, 50, 7, 7) - 1.0).abs() < EPS,
            "tight: {}",
            t.get(10, 50, 7, 7)
        );
        assert!(
            (t.get(10, 50, 6, 7) - 0.5).abs() < EPS,
            "wide: {}",
            t.get(10, 50, 6, 7)
        );
        // both stay 0.5 at lead 0 (symmetrized per key).
        assert!((t.get(0, 50, 7, 7) - 0.5).abs() < EPS);
        assert!((t.get(0, 50, 6, 7) - 0.5).abs() < EPS);
    }

    // Merging two accumulators equals recording every snapshot into one (the
    // composable num/den primitive).
    #[test]
    fn merge_is_additive() {
        let mut a = WinPctAccumulator::new();
        let mut b = WinPctAccumulator::new();
        let mut both = WinPctAccumulator::new();
        for &v in &[-30, -10, 5] {
            a.record(50, 7, 7, 0, v);
            both.record(50, 7, 7, 0, v);
        }
        for &v in &[10, 25, 60] {
            b.record(50, 7, 7, 0, v);
            both.record(50, 7, 7, 0, v);
            b.record(12, 3, 4, 0, v); // a key only b has
            both.record(12, 3, 4, 0, v);
        }
        a.merge(&b);
        let ta = a.finalize();
        let tb = both.finalize();
        for &(s, bag, my, opp) in &[
            (0, 50, 7, 7),
            (15, 50, 7, 7),
            (-15, 50, 7, 7),
            (5, 12, 3, 4),
        ] {
            assert!(
                (ta.get(s, bag, my, opp) - tb.get(s, bag, my, opp)).abs() < EPS,
                "merge mismatch at ({s},{bag},{my},{opp}): {} vs {}",
                ta.get(s, bag, my, opp),
                tb.get(s, bag, my, opp)
            );
        }
    }

    // The raw sparse format round-trips losslessly through CSV.
    #[test]
    fn csv_raw_round_trip() {
        let mut acc = WinPctAccumulator::new();
        for &v in &[-120, -40, -5, 0, 5, 40, 120] {
            acc.record(50, 7, 7, 0, v);
            acc.record(80, 7, 6, 3, v / 2 + 3); // nonzero snapshot spread too.
        }
        let acc2 = WinPctAccumulator::from_csv(&acc.to_csv()).unwrap();
        let t = acc.finalize();
        let t2 = acc2.finalize();
        for &(bag, my, opp) in &[(50u16, 7u8, 7u8), (80, 7, 6), (9, 9, 9)] {
            for s in [-300, -50, -3, 0, 3, 50, 300] {
                let (b, m, o) = (bag as usize, my as usize, opp as usize);
                assert!(
                    (t.get(s, b, m, o) - t2.get(s, b, m, o)).abs() < EPS,
                    "round-trip mismatch key ({bag},{my},{opp}) s={s}"
                );
            }
        }
    }

    // The header is a structured first line; no '#' comment lines anywhere.
    #[test]
    fn csv_header_is_structured() {
        let mut acc = WinPctAccumulator::new();
        acc.record(50, 7, 7, 0, 10);
        let csv = acc.to_csv();
        let first = csv.lines().next().unwrap();
        assert_eq!(first, "winpct,2,bag,my,opp");
        assert!(!csv.lines().any(|l| l.starts_with('#')), "no '#' comments");
    }

    // The english-winpct-combine pipeline: parse several raw CSVs, merge, and
    // the result equals one table built from all the records at once. Exercises
    // the CSV boundary (to_csv/from_csv) that the CLI crosses, not just an
    // in-memory merge.
    #[test]
    fn combine_csvs_sums_counts() {
        let mut a = WinPctAccumulator::new();
        let mut b = WinPctAccumulator::new();
        let mut both = WinPctAccumulator::new();
        for &v in &[-30, -10, 5, 40] {
            a.record(50, 7, 7, 0, v);
            both.record(50, 7, 7, 0, v);
        }
        for &v in &[10, 25, 60] {
            b.record(50, 7, 7, 0, v);
            both.record(50, 7, 7, 0, v);
            b.record(12, 3, 4, 2, v); // a key only b has
            both.record(12, 3, 4, 2, v);
        }
        let mut acc = WinPctAccumulator::from_csv(&a.to_csv()).unwrap();
        acc.merge(&WinPctAccumulator::from_csv(&b.to_csv()).unwrap());
        let tc = acc.finalize();
        let tb = both.finalize();
        for &(s, bag, my, opp) in &[(0, 50, 7, 7), (15, 50, 7, 7), (2, 12, 3, 4)] {
            assert!(
                (tc.get(s, bag, my, opp) - tb.get(s, bag, my, opp)).abs() < EPS,
                "combine mismatch at ({s},{bag},{my},{opp})"
            );
        }
    }
}
