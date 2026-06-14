// Copyright (C) 2020-2026 Andy Kurnia.

use crate::bites;
use crate::fash;

pub struct MultisetLattice {
    num_letters: usize,
    rack_size: usize,
    tallies: Vec<Vec<u8>>,
    rank_of: fash::MyHashMap<bites::Bites, u32>,
}

impl MultisetLattice {
    pub fn new(num_letters: usize, rack_size: usize) -> Self {
        let mut tallies = Vec::new();
        let mut rank_of = fash::MyHashMap::default();
        let mut tally = vec![0u8; num_letters];
        fn rec(
            pos: usize,
            remaining: usize,
            num_letters: usize,
            tally: &mut Vec<u8>,
            tallies: &mut Vec<Vec<u8>>,
            rank_of: &mut fash::MyHashMap<bites::Bites, u32>,
        ) {
            if pos == num_letters {
                let idx = tallies.len() as u32;
                let mut key = Vec::new();
                for (t, &c) in tally.iter().enumerate() {
                    for _ in 0..c {
                        key.push(t as u8);
                    }
                }
                rank_of.insert(key[..].into(), idx);
                tallies.push(tally.clone());
                return;
            }
            for c in 0..=remaining {
                tally[pos] = c as u8;
                rec(pos + 1, remaining - c, num_letters, tally, tallies, rank_of);
            }
            tally[pos] = 0;
        }
        rec(
            0,
            rack_size,
            num_letters,
            &mut tally,
            &mut tallies,
            &mut rank_of,
        );
        Self {
            num_letters,
            rack_size,
            tallies,
            rank_of,
        }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.tallies.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tallies.is_empty()
    }
    #[inline]
    pub fn num_letters(&self) -> usize {
        self.num_letters
    }
    #[inline]
    pub fn rack_size(&self) -> usize {
        self.rack_size
    }
    #[inline]
    pub fn tally(&self, idx: usize) -> &[u8] {
        &self.tallies[idx]
    }
    pub fn rank_bytes(&self, sorted_tiles: &[u8]) -> u32 {
        self.rank_of.get(sorted_tiles).copied().unwrap_or(!0)
    }
    pub fn rank(&self, tally: &[u8]) -> u32 {
        let mut key = Vec::new();
        for (t, &c) in tally.iter().enumerate() {
            for _ in 0..c {
                key.push(t as u8);
            }
        }
        self.rank_of.get(&key[..]).copied().unwrap_or(!0)
    }
}

pub const UNPLAYABLE: i32 = i32::MIN / 2;

/// Naive best_equity(R)=max over P<=R of sheet[P]+leave[R-P]; returns
/// (equity_millipoints, kept_tiles_sorted) where kept=R-P* (entering target).
pub fn naive_best_equity(
    lat: &MultisetLattice,
    sheet: &[i32],
    leave: &[i32],
    rack_tally: &[u8],
) -> (i32, Vec<u8>) {
    let n = lat.num_letters();
    let mut played = vec![0u8; n];
    let mut best = UNPLAYABLE;
    let mut best_kept = vec![0u8; n];
    // Constant context for the played-tile recursion, so rec carries only the changing
    // position -- no clippy::too_many_arguments. played/best/best_kept accumulate the
    // argmax across the whole descent, so they are borrowed for the driver call.
    struct Ctx<'a> {
        n: usize,
        lat: &'a MultisetLattice,
        sheet: &'a [i32],
        leave: &'a [i32],
        rack: &'a [u8],
        played: &'a mut [u8],
        best: &'a mut i32,
        best_kept: &'a mut [u8],
    }
    impl Ctx<'_> {
        fn rec(&mut self, pos: usize) {
            if pos == self.n {
                let pr = self.lat.rank(self.played);
                if pr == !0 {
                    return;
                }
                let sv = self.sheet[pr as usize];
                if sv <= UNPLAYABLE {
                    return;
                }
                let mut kept = vec![0u8; self.n];
                for (k, (&rc, &pc)) in kept
                    .iter_mut()
                    .zip(self.rack.iter().zip(self.played.iter()))
                {
                    *k = rc - pc;
                }
                let kr = self.lat.rank(&kept);
                if kr == !0 {
                    return;
                }
                let v = sv + self.leave[kr as usize];
                if v > *self.best {
                    *self.best = v;
                    self.best_kept.copy_from_slice(&kept);
                }
                return;
            }
            for c in 0..=self.rack[pos] {
                self.played[pos] = c;
                self.rec(pos + 1);
            }
            self.played[pos] = 0;
        }
    }
    Ctx {
        n,
        lat,
        sheet,
        leave,
        rack: rack_tally,
        played: &mut played,
        best: &mut best,
        best_kept: &mut best_kept,
    }
    .rec(0);
    let mut kept_tiles = Vec::new();
    for (t, &c) in best_kept.iter().enumerate() {
        for _ in 0..c {
            kept_tiles.push(t as u8);
        }
    }
    (best, kept_tiles)
}

pub struct BestEquityTable {
    pub equity: Vec<i32>,     // lattice-indexed best_equity(R)
    pub played: Vec<Vec<u8>>, // lattice-indexed argmax played tally (kept = R - played)
}

/// best_equity(R)=max over P<=R of sheet[P]+leave[R-P], for every R in the
/// lattice, with the argmax played split. Validated == naive_best_equity.
pub fn best_equity_table(lat: &MultisetLattice, sheet: &[i32], leave: &[i32]) -> BestEquityTable {
    let n = lat.num_letters();
    let mut equity = vec![UNPLAYABLE; lat.len()];
    let mut played = vec![vec![0u8; n]; lat.len()];
    // per-rank best via the slow reference call; kept plain here.
    for ridx in 0..lat.len() {
        let r = lat.tally(ridx).to_vec();
        let (v, kept) = naive_best_equity(lat, sheet, leave, &r);
        equity[ridx] = v;
        let mut p = vec![0u8; n];
        let mut kc = vec![0u8; n];
        for &kt in &kept {
            kc[kt as usize] += 1;
        }
        for t in 0..n {
            p[t] = r[t] - kc[t];
        }
        played[ridx] = p;
    }
    BestEquityTable { equity, played }
}

/// leave_new(S)=sum_d ways(d)*best[S+d]/sum_d ways(d), where the completion d is
/// drawn from the unseen pool with the kept S removed, |d|=rack_size-|S| and
/// ways(d)=prod_t C(unseen[t]-S[t], d[t]). UNPLAYABLE if S is itself undrawable
/// (more of some letter than the unseen pool holds) or has no feasible completion.
/// Returns millipoints.
pub fn leave_value_by_draw(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    s_tally: &[u8],
) -> i32 {
    let n = lat.num_letters();
    // The kept S is itself drawn from the unseen pool, so the completion draws from
    // unseen - S; if S holds more of any letter than the pool has, S is undrawable
    // and the leave is UNPLAYABLE.
    for t in 0..n {
        if s_tally[t] > unseen[t] {
            return UNPLAYABLE;
        }
    }
    let s_size: usize = s_tally.iter().map(|&c| c as usize).sum();
    let draw = lat.rack_size() - s_size;
    let mut num: i128 = 0;
    let mut den: i128 = 0;
    let mut d = vec![0u8; n];
    // Constant context for the draw enumeration (called once) -- no
    // clippy::too_many_arguments; rec carries only the changing draw position and
    // remaining count. d is the drawn-tile scratch; num/den accumulate.
    struct Ctx<'a> {
        n: usize,
        lat: &'a MultisetLattice,
        best: &'a [i32],
        unseen: &'a [u8],
        s_tally: &'a [u8],
        d: &'a mut [u8],
        num: &'a mut i128,
        den: &'a mut i128,
    }
    impl Ctx<'_> {
        fn rec(&mut self, pos: usize, remaining: usize) {
            if pos == self.n {
                if remaining != 0 {
                    return;
                }
                let mut w: i128 = 1;
                for t in 0..self.n {
                    w *= n_choose_k((self.unseen[t] - self.s_tally[t]) as u64, self.d[t] as u64)
                        as i128;
                }
                if w == 0 {
                    return;
                }
                let mut r = vec![0u8; self.n];
                for (rt, (&st, &dt)) in r.iter_mut().zip(self.s_tally.iter().zip(self.d.iter())) {
                    *rt = st + dt;
                }
                let ri = self.lat.rank(&r);
                if ri == !0 {
                    return;
                }
                *self.num += w * self.best[ri as usize] as i128;
                *self.den += w;
                return;
            }
            let hi = remaining.min(self.unseen[pos] as usize);
            for c in 0..=hi {
                self.d[pos] = c as u8;
                self.rec(pos + 1, remaining - c);
            }
            self.d[pos] = 0;
        }
    }
    Ctx {
        n,
        lat,
        best,
        unseen,
        s_tally,
        d: &mut d,
        num: &mut num,
        den: &mut den,
    }
    .rec(0, draw);
    if den == 0 {
        UNPLAYABLE
    } else {
        (num / den) as i32
    }
}

#[inline]
fn n_choose_k(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut num: u128 = 1;
    let mut den: u128 = 1;
    for i in 0..k {
        num *= (n - i) as u128;
        den *= (i + 1) as u128;
    }
    (num / den) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lattice_roundtrips_and_counts() {
        // 3 letters (incl blank=0), rack_size 2: multisets of size 0..=2.
        let lat = MultisetLattice::new(3, 2);
        // sizes: 1 (empty) + 3 (size1) + 6 (size2) = 10
        assert_eq!(lat.len(), 10);
        for idx in 0..lat.len() {
            let tally = lat.tally(idx);
            assert_eq!(lat.rank(tally), idx as u32);
            assert!(tally.iter().map(|&c| c as usize).sum::<usize>() <= 2);
        }
    }

    #[test]
    fn naive_best_equity_matches_hand_calc() {
        let lat = MultisetLattice::new(2, 2);
        let mut sheet = vec![UNPLAYABLE; lat.len()];
        sheet[lat.rank(&[0, 0]) as usize] = 0;
        sheet[lat.rank(&[1, 0]) as usize] = 5_000;
        sheet[lat.rank(&[0, 1]) as usize] = 3_000;
        let mut leave = vec![0i32; lat.len()];
        leave[lat.rank(&[1, 0]) as usize] = 4_000;
        leave[lat.rank(&[0, 1]) as usize] = 1_000;
        // rack [1,1]: play0 keep1 =5+1=6 ; play1 keep0 =3+4=7 ; play both = unplayable
        let (eq, kept) = naive_best_equity(&lat, &sheet, &leave, &[1, 1]);
        assert_eq!(eq, 7_000);
        assert_eq!(kept, vec![0u8]);
    }

    #[test]
    fn fast_conv_matches_naive() {
        let lat = MultisetLattice::new(4, 4);
        let mut sheet = vec![UNPLAYABLE; lat.len()];
        let mut leave = vec![0i32; lat.len()];
        for idx in 0..lat.len() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            if (h & 3) != 0 {
                sheet[idx] = h.rem_euclid(20_000) - 5_000;
            }
            leave[idx] = h.rem_euclid(8_000) - 4_000;
        }
        sheet[lat.rank(&[0, 0, 0, 0]) as usize] = 0; // pass always available
        let best = best_equity_table(&lat, &sheet, &leave);
        for idx in 0..lat.len() {
            let tally = lat.tally(idx).to_vec();
            let (naive, _) = naive_best_equity(&lat, &sheet, &leave, &tally);
            assert_eq!(
                best.equity[idx], naive,
                "mismatch at idx {idx} tally {tally:?}"
            );
        }
    }

    #[test]
    fn draw_average_weights_and_full_leave() {
        let lat = MultisetLattice::new(2, 2);
        let unseen = [1u8, 1u8];
        let mut best = vec![0i32; lat.len()];
        best[lat.rank(&[2, 0]) as usize] = 10_000;
        best[lat.rank(&[1, 1]) as usize] = 6_000;
        best[lat.rank(&[0, 2]) as usize] = 2_000;
        best[lat.rank(&[1, 0]) as usize] = 100;
        best[lat.rank(&[0, 1]) as usize] = 200;
        // S=empty: draw 2 from {0,1}; only [1,1] feasible (ways=1) -> 6_000.
        let e = leave_value_by_draw(&lat, &best, &unseen, &[0u8, 0u8]);
        assert_eq!(e, 6_000);
        // S=[1,1] full: draw 0 -> best[[1,1]]=6_000.
        let f = leave_value_by_draw(&lat, &best, &unseen, &[1u8, 1u8]);
        assert_eq!(f, 6_000);
    }
}
