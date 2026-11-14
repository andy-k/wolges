// Copyright (C) 2020-2026 Andy Kurnia.

// a raw sparse histogram of future swings per (bag, my, opp) count-state, so two
// runs merge by adding counts. The reader symmetrizes each key, which is what
// makes win%(0) exactly 0.5, then reverse-cumulates for an O(1) read.

use std::collections::{BTreeMap, HashMap};

pub type Key = (u16, u8, u8);

pub struct WinPctAccumulator {
    rows: BTreeMap<Key, BTreeMap<i32, u64>>,
}

impl Default for WinPctAccumulator {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl WinPctAccumulator {
    pub fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
        }
    }

    #[inline(always)]
    pub fn record(&mut self, bag: usize, my: usize, opp: usize, spread: i32, final_spread: i32) {
        let key = (bag as u16, my as u8, opp as u8);
        *self
            .rows
            .entry(key)
            .or_default()
            .entry(final_spread - spread)
            .or_insert(0) += 1;
    }

    #[inline(always)]
    pub fn merge(&mut self, other: &WinPctAccumulator) -> crate::error::Returns<()> {
        for (key, hist) in &other.rows {
            let mine = self.rows.get(key).into_iter().flat_map(|h| h.values());
            if mine
                .chain(hist.values())
                .try_fold(0u64, |sum, &count| sum.checked_add(count))
                .is_none()
            {
                return_error!(format!("win_pct: key {key:?} count overflows"));
            }
        }
        for (key, hist) in &other.rows {
            let dst = self.rows.entry(*key).or_default();
            for (&delta, &count) in hist {
                *dst.entry(delta).or_insert(0) += count;
            }
        }
        Ok(())
    }

    #[inline(always)]
    pub fn to_csv<W: std::io::Write>(&self, w: W) -> crate::error::Returns<()> {
        // a row has a pair for each delta, so rows differ in length.
        let mut out = csv::WriterBuilder::new().flexible(true).from_writer(w);
        let mut record = Vec::new();
        for (&(bag, my, opp), hist) in &self.rows {
            if hist.is_empty() {
                continue;
            }
            let total: u64 = hist.values().sum();
            record.clear();
            record.push(bag.to_string());
            record.push(my.to_string());
            record.push(opp.to_string());
            record.push(total.to_string());
            for (&delta, &count) in hist {
                record.push(format!("{delta}:{count}"));
            }
            out.write_record(&record)?;
        }
        out.flush()?;
        Ok(())
    }

    #[inline(always)]
    pub fn from_csv<R: std::io::Read>(r: R) -> crate::error::Returns<WinPctAccumulator> {
        let mut acc = WinPctAccumulator::new();
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(r);
        for record in reader.records() {
            let record = record?;
            let mut field = record.iter();
            let bag: u16 = field.next().unwrap_or("").parse()?;
            let my: u8 = field.next().unwrap_or("").parse()?;
            let opp: u8 = field.next().unwrap_or("").parse()?;
            let total: u64 = field.next().unwrap_or("").parse()?;
            let hist = acc.rows.entry((bag, my, opp)).or_default();
            let mut sum = 0u64;
            for pair in field {
                let (d, c) = pair
                    .split_once(':')
                    .ok_or_else(|| format!("win_pct: bad pair {pair:?}"))?;
                let delta: i32 = d.parse()?;
                let count: u64 = c.parse()?;
                let cell = hist.entry(delta).or_insert(0);
                let (Some(cell_sum), Some(row_sum)) =
                    (cell.checked_add(count), sum.checked_add(count))
                else {
                    return_error!(format!("win_pct: key ({bag},{my},{opp}) count overflows"));
                };
                *cell = cell_sum;
                sum = row_sum;
            }
            if sum != total {
                return_error!(format!(
                    "win_pct: key ({bag},{my},{opp}) total {total} != sum {sum}"
                ));
            }
        }
        Ok(acc)
    }

    #[inline(always)]
    pub fn finalize(&self) -> crate::error::Returns<WinPctTable> {
        let mut rows = HashMap::new();
        rows.try_reserve(self.rows.len())?;
        for (&key, hist) in &self.rows {
            let mut cap = None;
            for &delta in hist.keys() {
                let Some(neg) = delta.checked_neg() else {
                    return_error!(format!(
                        "win_pct: key {key:?} delta {delta} has no negation"
                    ));
                };
                cap = cap.max(Some(delta.max(neg)));
            }
            let Some(cap) = cap else {
                continue;
            };
            let Some(width) = cap.checked_mul(2).and_then(|w| w.checked_add(1)) else {
                return_error!(format!("win_pct: key {key:?} cap {cap} cannot fit a row"));
            };
            let width = width as usize;
            let mut sym = Vec::new();
            sym.try_reserve_exact(width)?;
            sym.resize(width, 0u64);
            for (&delta, &count) in hist {
                for i in [delta + cap, -delta + cap] {
                    let cell = &mut sym[i as usize];
                    let Some(sum) = cell.checked_add(count) else {
                        return_error!(format!("win_pct: key {key:?} count overflows"));
                    };
                    *cell = sum;
                }
            }
            let mut total = 0u64;
            for &h in &sym {
                let Some(sum) = total.checked_add(h) else {
                    return_error!(format!("win_pct: key {key:?} total overflows"));
                };
                total = sum;
            }
            let total = total as f64;

            let mut win = Vec::new();
            win.try_reserve_exact(width)?;
            win.resize(width, 0.0f32);
            let mut strictly_greater = 0u64;
            for i in 0..width {
                let h = sym[width - 1 - i];
                win[i] = ((strictly_greater as f64 + 0.5 * h as f64) / total) as f32;
                strictly_greater += h;
            }
            rows.insert(key, DenseRow { cap, win });
        }
        Ok(WinPctTable { rows })
    }
}

struct DenseRow {
    cap: i32,
    win: Vec<f32>,
}

pub struct WinPctTable {
    rows: HashMap<Key, DenseRow>,
}

impl WinPctTable {
    #[inline(always)]
    pub fn get(&self, spread: i32, bag: usize, my: usize, opp: usize) -> f32 {
        self.get_opt(spread, bag, my, opp).unwrap_or(0.5)
    }

    #[inline(always)]
    pub fn get_opt(&self, spread: i32, bag: usize, my: usize, opp: usize) -> Option<f32> {
        match self.rows.get(&(bag as u16, my as u8, opp as u8)) {
            None => None,
            Some(row) if spread > row.cap => Some(1.0),
            Some(row) if spread < -row.cap => Some(0.0),
            Some(row) => Some(row.win[(spread + row.cap) as usize]),
        }
    }

    pub fn from_csv<R: std::io::Read>(r: R) -> crate::error::Returns<WinPctTable> {
        WinPctAccumulator::from_csv(r)?.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv_of(acc: &WinPctAccumulator) -> Vec<u8> {
        let mut out = Vec::new();
        acc.to_csv(&mut out).unwrap();
        out
    }

    const EPS: f32 = 1e-5;

    #[test]
    #[inline]
    fn finalize_is_monotone_symmetric_and_half_at_zero() {
        let mut acc = WinPctAccumulator::new();
        for &v in &[-30, -10, 10, 30] {
            acc.record(50, 7, 7, 0, v);
        }
        let t = acc.finalize().unwrap();
        assert!(
            (t.get(0, 50, 7, 7) - 0.5).abs() < EPS,
            "got {}",
            t.get(0, 50, 7, 7)
        );

        assert!((t.get(999, 50, 7, 7) - 1.0).abs() < EPS);
        assert!((t.get(-999, 50, 7, 7) - 0.0).abs() < EPS);

        let mut prev = -1.0f32;
        for s in -60..=60 {
            let w = t.get(s, 50, 7, 7);
            assert!(w >= prev - EPS, "not monotone at s={s}: {w} < {prev}");
            prev = w;
        }

        for s in [7, 23, 41] {
            assert!((t.get(s, 50, 7, 7) + t.get(-s, 50, 7, 7) - 1.0).abs() < EPS);
        }
    }

    #[test]
    #[inline]
    fn cumulative_informs_all_leads() {
        let mut acc = WinPctAccumulator::new();
        acc.record(60, 7, 7, 0, -45); // one game swung -45 from this state.
        let t = acc.finalize().unwrap();

        assert!(
            (t.get(50, 60, 7, 7) - 1.0).abs() < EPS,
            "got {}",
            t.get(50, 60, 7, 7)
        );

        assert!((t.get(46, 60, 7, 7) - 1.0).abs() < EPS);

        assert!(
            (t.get(45, 60, 7, 7) - 0.75).abs() < EPS,
            "got {}",
            t.get(45, 60, 7, 7)
        );

        assert!(
            (t.get(40, 60, 7, 7) - 0.5).abs() < EPS,
            "got {}",
            t.get(40, 60, 7, 7)
        );
    }

    #[test]
    #[inline]
    fn get_saturates_out_of_range() {
        let mut acc = WinPctAccumulator::new();
        for &v in &[-30, -10, 10, 30] {
            acc.record(50, 7, 7, 0, v);
        }
        let t = acc.finalize().unwrap();
        assert!((t.get(999_999, 50, 7, 7) - 1.0).abs() < EPS);
        assert!((t.get(-999_999, 50, 7, 7) - 0.0).abs() < EPS);

        assert!(
            (t.get(30, 50, 7, 7) - 0.875).abs() < EPS,
            "got {}",
            t.get(30, 50, 7, 7)
        );
    }

    #[test]
    #[inline]
    fn absent_key_is_half() {
        let mut acc = WinPctAccumulator::new();
        acc.record(50, 7, 7, 0, 10);
        let t = acc.finalize().unwrap();
        for s in [-200, -1, 0, 1, 200] {
            assert!((t.get(s, 7, 7, 7) - 0.5).abs() < EPS, "key (7,7,7) s={s}");
        }
    }

    #[test]
    #[inline]
    fn recording_under_one_key_does_not_move_another() {
        let mut acc = WinPctAccumulator::new();
        for &v in &[-5, 5] {
            acc.record(50, 7, 7, 0, v);
        }
        for &v in &[-100, 100] {
            acc.record(50, 6, 7, 0, v);
        }
        let t = acc.finalize().unwrap();
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

        assert!((t.get(0, 50, 7, 7) - 0.5).abs() < EPS);
        assert!((t.get(0, 50, 6, 7) - 0.5).abs() < EPS);
    }

    #[test]
    #[inline]
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
        a.merge(&b).unwrap();
        let ta = a.finalize().unwrap();
        let tb = both.finalize().unwrap();
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

    #[test]
    #[inline]
    fn csv_raw_round_trip() {
        let mut acc = WinPctAccumulator::new();
        for &v in &[-120, -40, -5, 0, 5, 40, 120] {
            acc.record(50, 7, 7, 0, v);
            acc.record(80, 7, 6, 3, v / 2 + 3); // nonzero snapshot spread too.
        }
        acc.record(20, 3, 7, 0, 15); // a row shorter than the others.
        let acc2 = WinPctAccumulator::from_csv(&csv_of(&acc)[..]).unwrap();
        let t = acc.finalize().unwrap();
        let t2 = acc2.finalize().unwrap();
        for &(bag, my, opp) in &[(50u16, 7u8, 7u8), (80, 7, 6), (20, 3, 7), (9, 9, 9)] {
            for s in [-300, -50, -3, 0, 3, 50, 300] {
                let (b, m, o) = (bag as usize, my as usize, opp as usize);
                assert!(
                    (t.get(s, b, m, o) - t2.get(s, b, m, o)).abs() < EPS,
                    "round-trip mismatch key ({bag},{my},{opp}) s={s}"
                );
            }
        }
    }

    #[test]
    fn csv_has_no_header_row() {
        let mut acc = WinPctAccumulator::new();
        acc.record(50, 7, 7, 0, 10);
        let csv = String::from_utf8(csv_of(&acc)).unwrap();
        assert_eq!(csv.lines().next().unwrap(), "50,7,7,1,10:1");
    }

    #[test]
    #[inline]
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
        let mut acc = WinPctAccumulator::from_csv(&csv_of(&a)[..]).unwrap();
        acc.merge(&WinPctAccumulator::from_csv(&csv_of(&b)[..]).unwrap())
            .unwrap();
        let tc = acc.finalize().unwrap();
        let tb = both.finalize().unwrap();
        for &(s, bag, my, opp) in &[(0, 50, 7, 7), (15, 50, 7, 7), (2, 12, 3, 4)] {
            assert!(
                (tc.get(s, bag, my, opp) - tb.get(s, bag, my, opp)).abs() < EPS,
                "combine mismatch at ({s},{bag},{my},{opp})"
            );
        }
    }

    #[test]
    fn delta_without_a_negation_is_refused() {
        assert!(WinPctTable::from_csv(&b"0,7,7,1,-2147483648:1\n"[..]).is_err());
    }

    #[test]
    fn row_too_wide_for_i32_is_refused() {
        let cap = i32::MAX / 2 + 1;
        let csv = format!("0,7,7,1,{cap}:1\n");
        assert!(WinPctTable::from_csv(csv.as_bytes()).is_err());
    }

    #[test]
    fn count_overflow_is_refused() {
        let max = u64::MAX;
        let one = format!("0,7,7,{max},5:{max}\n");
        assert!(WinPctTable::from_csv(one.as_bytes()).is_err());
        let zero = format!("0,7,7,{max},0:{max}\n");
        assert!(WinPctTable::from_csv(zero.as_bytes()).is_err());
        let two = format!("0,7,7,{max},5:{max},6:1\n");
        assert!(WinPctAccumulator::from_csv(two.as_bytes()).is_err());
        let same = format!("0,7,7,1,5:1\n0,7,7,{max},5:{max}\n");
        assert!(WinPctAccumulator::from_csv(same.as_bytes()).is_err());
    }

    #[test]
    fn merge_overflow_is_refused_and_leaves_both_unchanged() {
        let max = u64::MAX;
        let mut a =
            WinPctAccumulator::from_csv(format!("0,7,7,{max},5:{max}\n").as_bytes()).unwrap();
        let other = "0,7,7,1,6:1\n1,7,7,1,5:1\n";
        let b = WinPctAccumulator::from_csv(other.as_bytes()).unwrap();
        let before = csv_of(&a);
        assert!(a.merge(&b).is_err());
        assert_eq!(csv_of(&a), before);
        assert_eq!(csv_of(&b), other.as_bytes());
    }
}
