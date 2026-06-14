// Copyright (C) 2020-2026 Andy Kurnia.

// Reset-board census core: a bounded-multiset lattice with closed-form O(L)
// ranking (no hashmap, no per-lookup allocation), a play-value sheet, the
// (max,+) best-equity convolution over that lattice, and no-replacement
// draw-averaging. All pure; movegen results are fed in as (multiset, score).

pub const UNPLAYABLE: i32 = i32::MIN / 2;

// Stack scratch width for tallies in the hot paths. Alphabets are far smaller.
const MAX_LETTERS: usize = 64;

/// Every multiset of `num_letters` letters with total size `0..=rack_size`,
/// indexed to a dense rank and back by a closed-form combinatorial number
/// system (size-grouped, then lexicographic by ascending per-letter count).
/// O(L) rank/unrank, no allocation, no stored table of multisets.
pub struct MultisetLattice {
    num_letters: usize,
    rack_size: usize,
    // binom[n][k] = C(n, k) for n in 0..=rack_size+num_letters, k in 0..=num_letters.
    binom: Vec<Vec<u64>>,
    // size_offset[s] = number of multisets of size < s = C(s+L-1, L); len rack_size+2.
    size_offset: Vec<u64>,
}

impl MultisetLattice {
    pub fn new(num_letters: usize, rack_size: usize) -> Self {
        assert!((1..=MAX_LETTERS).contains(&num_letters));
        let n_max = rack_size + num_letters;
        let k_max = num_letters;
        let mut binom = vec![vec![0u64; k_max + 1]; n_max + 1];
        for row in binom.iter_mut() {
            row[0] = 1; // C(n, 0) = 1
        }
        for n in 1..=n_max {
            for k in 1..=k_max.min(n) {
                // Pascal: C(n,k) = C(n-1,k-1) + C(n-1,k).
                binom[n][k] = binom[n - 1][k - 1] + binom[n - 1][k];
            }
        }
        // size_offset[s] = C(s+L-1, L), the count of multisets of size < s.
        let mut size_offset = vec![0u64; rack_size + 2];
        for (s, slot) in size_offset.iter_mut().enumerate() {
            let n = s + num_letters - 1;
            *slot = if num_letters <= n {
                binom[n][num_letters]
            } else {
                0
            };
        }
        Self {
            num_letters,
            rack_size,
            binom,
            size_offset,
        }
    }

    #[inline]
    fn c(&self, n: usize, k: usize) -> u64 {
        if k > n { 0 } else { self.binom[n][k] }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.size_offset[self.rack_size + 1] as usize
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    pub fn num_letters(&self) -> usize {
        self.num_letters
    }
    #[inline]
    pub fn rack_size(&self) -> usize {
        self.rack_size
    }
    /// First lattice index of the full (size == rack_size) racks. The full-rack
    /// block is [full_rack_start(), len()); those are the only entries Step 3's
    /// draw-average reads, so best_equity_table only fills that block.
    #[inline]
    pub fn full_rack_start(&self) -> usize {
        self.size_offset[self.rack_size] as usize
    }

    /// Rank of a multiset given as a per-letter tally (len num_letters). Returns
    /// !0 if the total size exceeds rack_size (outside the lattice).
    pub fn rank(&self, tally: &[u8]) -> u32 {
        let l = self.num_letters;
        let s: usize = tally.iter().map(|&c| c as usize).sum();
        if s > self.rack_size {
            return !0;
        }
        let mut within: u64 = 0;
        let mut rem = s;
        for (t, &ct_raw) in tally.iter().enumerate().take(l - 1) {
            let ct = ct_raw as usize;
            let parts = l - 1 - t; // letters after position t
            for j in 0..ct {
                // compositions of (rem - j) into `parts` letters
                within += self.c((rem - j) + parts - 1, parts - 1);
            }
            rem -= ct;
        }
        (self.size_offset[s] + within) as u32
    }

    /// Rank of a multiset given as sorted tile bytes (e.g. a played word's tiles).
    pub fn rank_bytes(&self, sorted_tiles: &[u8]) -> u32 {
        let mut tally = [0u8; MAX_LETTERS];
        for &t in sorted_tiles {
            let i = t as usize;
            if i >= self.num_letters {
                return !0;
            }
            tally[i] += 1;
        }
        self.rank(&tally[..self.num_letters])
    }

    /// Decode `idx` into a per-letter tally written to `out` (len num_letters).
    pub fn unrank_into(&self, idx: usize, out: &mut [u8]) {
        let l = self.num_letters;
        let mut s = 0usize;
        while s < self.rack_size && (self.size_offset[s + 1] as usize) <= idx {
            s += 1;
        }
        let mut r = idx - self.size_offset[s] as usize;
        let mut rem = s;
        for (t, slot) in out.iter_mut().enumerate().take(l - 1) {
            let parts = l - 1 - t;
            let mut ct = 0usize;
            loop {
                let ways = self.c((rem - ct) + parts - 1, parts - 1) as usize;
                if r < ways {
                    break;
                }
                r -= ways;
                ct += 1;
            }
            *slot = ct as u8;
            rem -= ct;
        }
        out[l - 1] = rem as u8;
    }

    /// Owned tally for `idx` (convenience for tests / cold paths).
    pub fn tally(&self, idx: usize) -> Vec<u8> {
        let mut out = vec![0u8; self.num_letters];
        self.unrank_into(idx, &mut out);
        out
    }
}

/// Naive best_equity(R)=max over P<=R of sheet[P]+leave[R-P]; returns
/// (equity_millipoints, kept_tiles_sorted) where kept=R-P* (entering target).
/// Reference implementation that best_equity_table is validated against.
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
                // exchange floor: disposing P is worth max(word score, 0). See
                // best_equity_table.
                let sv = self.sheet[pr as usize].max(0);
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

/// best_equity(R)=max over P<=R of sheet[P]+leave[R-P], for every FULL rack R
/// (size == rack_size; the only entries Step 3 reads). Returns a lattice-indexed
/// flat array with UNPLAYABLE outside the full-rack block. Alloc-free per rack
/// (stack scratch + closed-form rank), recursing only over R's nonzero letters.
/// Validated == naive_best_equity on the full-rack block. The kept-side split is
/// not materialized: the entering attribution comes from the draw-average
/// in leave_value_by_draw, not from a per-rack argmax.
pub fn best_equity_table(lat: &MultisetLattice, sheet: &[i32], leave: &[i32]) -> Vec<i32> {
    let n = lat.num_letters();
    let mut equity = vec![UNPLAYABLE; lat.len()];
    let mut r = [0u8; MAX_LETTERS];
    let mut p = [0u8; MAX_LETTERS];
    let mut k = [0u8; MAX_LETTERS]; // k = r - p, maintained incrementally
    // recurse over the (letter, count) of R's nonzero letters only. Constant +
    // per-rack context, so rec carries only the changing letter index -- no
    // clippy::too_many_arguments. p/k are the played/kept scratch; best accumulates.
    struct Ctx<'a> {
        nz: &'a [(usize, u8)],
        n: usize,
        lat: &'a MultisetLattice,
        sheet: &'a [i32],
        leave: &'a [i32],
        p: &'a mut [u8],
        k: &'a mut [u8],
        best: &'a mut i32,
    }
    impl Ctx<'_> {
        fn rec(&mut self, i: usize) {
            if i == self.nz.len() {
                let pr = self.lat.rank(&self.p[..self.n]);
                // disposing the played tiles P is worth max(best word score, 0): you
                // can always EXCHANGE them for 0 (pre-endgame, bag non-empty). So an
                // unplayable P contributes 0 + leave(K) = the exchange-keep-K value.
                // Without this floor, a leave K is only reachable when a word disposes
                // exactly R-K, so good leaves collapse to the mean (compression).
                let sv = self.sheet[pr as usize].max(0);
                let kr = self.lat.rank(&self.k[..self.n]);
                let v = sv + self.leave[kr as usize];
                if v > *self.best {
                    *self.best = v;
                }
                return;
            }
            let (t, cnt) = self.nz[i];
            for c in 0..=cnt {
                self.p[t] = c;
                self.k[t] = cnt - c;
                self.rec(i + 1);
            }
            self.p[t] = 0;
            self.k[t] = cnt; // restore for the caller's frame
        }
    }
    let lo = lat.full_rack_start();
    let mut nz: [(usize, u8); MAX_LETTERS] = [(0, 0); MAX_LETTERS];
    for (off, slot) in equity[lo..].iter_mut().enumerate() {
        let ridx = lo + off;
        lat.unrank_into(ridx, &mut r[..n]);
        let mut m = 0;
        for (t, &c) in r[..n].iter().enumerate() {
            if c > 0 {
                nz[m] = (t, c);
                k[t] = c; // k starts at r (p == 0)
                m += 1;
            }
        }
        let mut best = UNPLAYABLE;
        Ctx {
            nz: &nz[..m],
            n,
            lat,
            sheet,
            leave,
            p: &mut p,
            k: &mut k,
            best: &mut best,
        }
        .rec(0);
        // reset k's touched entries back to zero for the next rack.
        for &(t, _) in &nz[..m] {
            k[t] = 0;
        }
        *slot = best;
    }
    equity
}

/// leave_new(S)=sum_d ways(d)*best[S+d]/sum_d ways(d), where the completion d is
/// drawn from the unseen pool with the kept S removed, |d|=rack_size-|S| and
/// ways(d)=prod_t C(unseen[t]-S[t], d[t]). UNPLAYABLE if S is itself undrawable
/// (more of some letter than the unseen pool holds) or has no feasible completion.
/// Returns millipoints. Alloc-free (stack scratch).
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
    let mut d = [0u8; MAX_LETTERS];
    let mut r = [0u8; MAX_LETTERS];
    // Constant context for the draw enumeration (called once) -- no
    // clippy::too_many_arguments; rec carries only the changing draw position and
    // remaining count. d/r are the drawn-tile and full-rack scratch; num/den accumulate.
    struct Ctx<'a> {
        n: usize,
        lat: &'a MultisetLattice,
        best: &'a [i32],
        unseen: &'a [u8],
        s_tally: &'a [u8],
        d: &'a mut [u8],
        r: &'a mut [u8],
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
                    if w == 0 {
                        return;
                    }
                }
                for t in 0..self.n {
                    self.r[t] = self.s_tally[t] + self.d[t];
                }
                let ri = self.lat.rank(&self.r[..self.n]);
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
        r: &mut r,
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
            assert_eq!(lat.rank(&tally), idx as u32);
            assert!(tally.iter().map(|&c| c as usize).sum::<usize>() <= 2);
        }
    }

    #[test]
    fn lattice_roundtrips_english_sized() {
        // English-shaped: 27 letters, rack 7. Verify every rank round-trips.
        let lat = MultisetLattice::new(27, 7);
        assert_eq!(lat.len(), 5_379_616);
        let mut buf = vec![0u8; 27];
        for idx in (0..lat.len()).step_by(997) {
            lat.unrank_into(idx, &mut buf);
            assert_eq!(lat.rank(&buf), idx as u32, "roundtrip idx {idx}");
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
        // best_equity_table only fills the full-rack (size == rack_size) block.
        for (idx, &b) in best.iter().enumerate().skip(lat.full_rack_start()) {
            let tally = lat.tally(idx);
            assert_eq!(tally.iter().map(|&c| c as usize).sum::<usize>(), 4);
            let (naive, _) = naive_best_equity(&lat, &sheet, &leave, &tally);
            assert_eq!(b, naive, "mismatch at idx {idx} tally {tally:?}");
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
