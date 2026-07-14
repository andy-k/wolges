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
    // flat Pascal triangle (reused from prob), for the closed-form rank binomials
    // (a binomial coefficient C(n,k) counts the ways to choose k of n).
    pascal: crate::prob::Pascal,
    // size_offset[s] = number of multisets of size < s = C(s+L-1, L); len rack_size+2.
    size_offset: Vec<u64>,
}

impl MultisetLattice {
    pub fn new(num_letters: usize, rack_size: usize) -> Self {
        assert!((1..=MAX_LETTERS).contains(&num_letters));
        let n_max = rack_size + num_letters;
        // need rows 0..=n_max, i.e. n_max + 1 rows.
        let pascal = crate::prob::Pascal::with_rows(n_max + 1);
        // size_offset[s] = C(s+L-1, L), the count of multisets of size < s.
        let mut size_offset = vec![0u64; rack_size + 2];
        for (s, slot) in size_offset.iter_mut().enumerate() {
            *slot = pascal.binom(s + num_letters - 1, num_letters);
        }
        Self {
            num_letters,
            rack_size,
            pascal,
            size_offset,
        }
    }

    #[inline(always)]
    fn c(&self, n: usize, k: usize) -> u64 {
        self.pascal.binom(n, k)
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.size_offset[self.rack_size + 1] as usize
    }
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline(always)]
    pub fn num_letters(&self) -> usize {
        self.num_letters
    }
    #[inline(always)]
    pub fn rack_size(&self) -> usize {
        self.rack_size
    }
    /// First lattice index of the full (size == rack_size) racks. The full-rack
    /// block is [full_rack_start(), len()); those are the only entries Step 3's
    /// draw-average reads, so best_equity_table only fills that block.
    #[inline(always)]
    pub fn full_rack_start(&self) -> usize {
        self.size_offset[self.rack_size] as usize
    }

    /// Rank of a multiset given as a per-letter tally (len num_letters). Returns
    /// !0 if the total size exceeds rack_size (outside the lattice).
    #[inline(always)]
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
    #[inline(always)]
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
                // the sheet has the exchange floor baked in (entries are >= 0; an
                // unreached or negative-scoring P is 0). See best_equity_table.
                let sv = self.sheet[pr as usize];
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
/// (size == rack_size; the only entries Step 3 reads). Fills the full-rack block
/// of `out` (a lattice-length, lattice-indexed buffer); entries outside that block
/// are left untouched (never read). The buffer is caller-owned so it is allocated
/// ONCE and reused across boards, not per call. Alloc-free per rack (stack scratch
/// and closed-form rank), recursing only over R's nonzero letters. Validated to
/// match naive_best_equity on the full-rack block. The kept-side split is not
/// materialized: the entering attribution comes from the draw-average in
/// leave_value_by_draw, not from a per-rack argmax.
pub fn best_equity_table(lat: &MultisetLattice, sheet: &[i32], leave: &[i32], out: &mut [i32]) {
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];
    // Rank BOTH sides of each split incrementally instead of an O(L) lat.rank per
    // split. The lattice rank of a multiset is size_offset[size] + within, where
    // within = sum over letters t (except the implicit last one, t == n-1) of a
    // binomial run whose argument is the SUFFIX size from t (= size - prefix). So
    // if R's nonzero letters are added HIGH-to-LOW, each letter's contribution is
    // known the moment it is added (its suffix size is fixed), and within/size are
    // carried by value down the recursion -- the base case is O(1) and p[]/k[] need
    // not be materialized. Constant context, so rec carries only the changing letter
    // index and carried ranks -- no clippy::too_many_arguments. The kept-side split
    // is not stored: the entering attribution comes from the draw-average in
    // leave_value_by_draw.
    struct Ctx<'a> {
        nz: &'a [(usize, u8)],
        n: usize,
        lat: &'a MultisetLattice,
        sheet: &'a [i32],
        leave: &'a [i32],
        best: &'a mut i32,
    }
    impl Ctx<'_> {
        fn rec(&mut self, i: usize, s_p: usize, within_p: u64, s_k: usize, within_k: u64) {
            if i == 0 {
                // disposing the played tiles P is worth max(best word score, 0): you
                // can always EXCHANGE them for 0 (pre-endgame, bag non-empty). That
                // floor is baked into the sheet at build time (init 0; a word only
                // RAISES an entry), so an unreached or negative-scoring P reads as 0 +
                // leave(K) = the exchange-keep-K value.
                // SAFETY: s_p, s_k <= rack_size and within_p, within_k are valid
                // within-size offsets, so pr, kr are in-range lattice indices
                // (< sheet.len() == leave.len()).
                let pr = (self.lat.size_offset[s_p] + within_p) as usize;
                let kr = (self.lat.size_offset[s_k] + within_k) as usize;
                let v = unsafe { *self.sheet.get_unchecked(pr) }
                    + unsafe { *self.leave.get_unchecked(kr) };
                if v > *self.best {
                    *self.best = v;
                }
                return;
            }
            // process R's nonzero letters from highest index (nz is ascending) down.
            let (t, cnt) = self.nz[i - 1];
            let parts = self.n - 1 - t;
            for cp in 0..=cnt {
                let ck = cnt - cp;
                // within contribution of this letter to each side. rem_t (the rank's
                // suffix size from t) = (size from higher letters) + (this count). The
                // last letter (t == n-1) is the rank's implicit remainder: no within.
                let (mut dwp, mut dwk) = (0u64, 0u64);
                if t + 1 < self.n {
                    let mut a = s_p + cp as usize;
                    for _ in 0..cp {
                        dwp += self.lat.c(a + parts - 1, parts - 1);
                        a -= 1;
                    }
                    let mut b = s_k + ck as usize;
                    for _ in 0..ck {
                        dwk += self.lat.c(b + parts - 1, parts - 1);
                        b -= 1;
                    }
                }
                self.rec(
                    i - 1,
                    s_p + cp as usize,
                    within_p + dwp,
                    s_k + ck as usize,
                    within_k + dwk,
                );
            }
        }
    }
    let lo = lat.full_rack_start();
    let mut nz: [(usize, u8); MAX_LETTERS] = [(0, 0); MAX_LETTERS];
    for (off, slot) in out[lo..].iter_mut().enumerate() {
        let ridx = lo + off;
        lat.unrank_into(ridx, &mut r[..n]);
        let mut m = 0;
        for (t, &c) in r[..n].iter().enumerate() {
            if c > 0 {
                nz[m] = (t, c);
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
            best: &mut best,
        }
        .rec(m, 0, 0, 0, 0);
        *slot = best;
    }
}

/// Full-rack attribution: credit each full rack R's best_equity(R), weighted
/// by the probability w(R) of drawing R from the unseen pool, to EVERY subrack
/// S <= R. Accumulates num[S] += w(R)*best[R] and den[S] += w(R) over the
/// full-rack block; the caller forms leave(S) = num[S]/den[S]. This is the
/// standard leave-gen (gilles-style) attribution -- a rack's equity is
/// apportioned onto all its subracks, including the usually-PLAYED ones --
/// done exhaustively over every rack the board can draw. Contrast
/// leave_value_by_draw, which is the entering attribution (credit only the
/// held-entering leave). num/den are caller-owned and zeroed per board.
/// Subracks are enumerated with the same high-to-low incremental rank as
/// best_equity_table (base case is O(1)).
pub fn apportion_table(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    num: &mut [f64],
    den: &mut [f64],
) {
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];
    // Constant + per-rack context, so rec carries only the changing letter index and
    // carried subrack rank -- no clippy::too_many_arguments. w/we are the rack's draw
    // weight and weighted equity; num/den accumulate.
    struct Ctx<'a> {
        nz: &'a [(usize, u8)],
        n: usize,
        lat: &'a MultisetLattice,
        w: f64,
        we: f64,
        num: &'a mut [f64],
        den: &'a mut [f64],
    }
    impl Ctx<'_> {
        fn rec(&mut self, i: usize, s_s: usize, within_s: u64) {
            if i == 0 {
                let sr = (self.lat.size_offset[s_s] + within_s) as usize;
                // SAFETY: sr is the rank of a sub-multiset of a size<=rack_size rack,
                // so it is a valid lattice index (< num.len() == den.len()).
                unsafe {
                    *self.num.get_unchecked_mut(sr) += self.we;
                    *self.den.get_unchecked_mut(sr) += self.w;
                }
                return;
            }
            let (t, cnt) = self.nz[i - 1];
            let parts = self.n - 1 - t;
            for cs in 0..=cnt {
                let mut dw = 0u64;
                if t + 1 < self.n {
                    let mut a = s_s + cs as usize;
                    for _ in 0..cs {
                        dw += self.lat.c(a + parts - 1, parts - 1);
                        a -= 1;
                    }
                }
                self.rec(i - 1, s_s + cs as usize, within_s + dw);
            }
        }
    }
    let lo = lat.full_rack_start();
    let mut nz: [(usize, u8); MAX_LETTERS] = [(0, 0); MAX_LETTERS];
    for ridx in lo..lat.len() {
        lat.unrank_into(ridx, &mut r[..n]);
        // weight w(R) = prod_t C(unseen[t], R[t]); 0 if R is not drawable from the
        // unseen pool (then it contributes nothing, as in the draw-average).
        let mut w = 1.0f64;
        let mut m = 0;
        let mut drawable = true;
        for (t, &c) in r[..n].iter().enumerate() {
            if c > 0 {
                nz[m] = (t, c);
                m += 1;
                w *= n_choose_k(unseen[t] as u64, c as u64) as f64;
                if w == 0.0 {
                    drawable = false;
                    break;
                }
            }
        }
        if !drawable {
            continue;
        }
        // SAFETY: ridx is in the full-rack block, where best is filled.
        let e = unsafe { *best.get_unchecked(ridx) } as f64;
        Ctx {
            nz: &nz[..m],
            n,
            lat,
            w,
            we: w * e,
            num,
            den,
        }
        .rec(m, 0, 0);
    }
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
                // SAFETY: ri != !0 and r has size rack_size, so ri < best.len().
                *self.num += w * unsafe { *self.best.get_unchecked(ri as usize) } as i128;
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
        // sheet has the exchange floor baked in (init 0; entries non-negative).
        let mut sheet = vec![0i32; lat.len()];
        sheet[lat.rank(&[1, 0]) as usize] = 5_000;
        sheet[lat.rank(&[0, 1]) as usize] = 3_000;
        let mut leave = vec![0i32; lat.len()];
        leave[lat.rank(&[1, 0]) as usize] = 4_000;
        leave[lat.rank(&[0, 1]) as usize] = 1_000;
        // rack [1,1]: play0 keep1 = 5+1 = 6 ; play1 keep0 = 3+4 = 7 ;
        // play both keep nothing = 0 (exchange floor). best = 7.
        let (eq, kept) = naive_best_equity(&lat, &sheet, &leave, &[1, 1]);
        assert_eq!(eq, 7_000);
        assert_eq!(kept, vec![0u8]);
    }

    #[test]
    fn fast_conv_matches_naive() {
        let lat = MultisetLattice::new(4, 4);
        // sheet has the exchange floor baked in: init 0, entries non-negative
        // (the empty/pass entry is 0 from the init).
        let mut sheet = vec![0i32; lat.len()];
        let mut leave = vec![0i32; lat.len()];
        for idx in 0..lat.len() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            if (h & 3) != 0 {
                sheet[idx] = h.rem_euclid(20_000);
            }
            leave[idx] = h.rem_euclid(8_000) - 4_000;
        }
        let mut best = vec![UNPLAYABLE; lat.len()];
        best_equity_table(&lat, &sheet, &leave, &mut best);
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

    #[test]
    fn apportion_matches_naive() {
        // 3 letters, rack 3. Pseudo-random best[] over full racks; an unseen pool.
        let lat = MultisetLattice::new(3, 3);
        let unseen = [4u8, 3u8, 2u8];
        let mut best = vec![UNPLAYABLE; lat.len()];
        for (idx, slot) in best.iter_mut().enumerate().skip(lat.full_rack_start()) {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            *slot = h.rem_euclid(20_000) - 5_000;
        }
        let mut num = vec![0f64; lat.len()];
        let mut den = vec![0f64; lat.len()];
        apportion_table(&lat, &best, &unseen, &mut num, &mut den);
        // naive: enumerate full racks, brute-force every subrack, apportion w(R).
        let n = lat.num_letters();
        let mut num_naive = vec![0f64; lat.len()];
        let mut den_naive = vec![0f64; lat.len()];
        for (ridx, &bval) in best.iter().enumerate().skip(lat.full_rack_start()) {
            let rk = lat.tally(ridx);
            let mut w = 1.0f64;
            for t in 0..n {
                w *= n_choose_k(unseen[t] as u64, rk[t] as u64) as f64;
            }
            if w == 0.0 {
                continue;
            }
            let e = bval as f64;
            let mut s = vec![0u8; n];
            // every subrack S <= R (independent per-letter count 0..=rk[t]).
            for a in 0..=rk[0] {
                for b in 0..=rk[1] {
                    for c in 0..=rk[2] {
                        s[0] = a;
                        s[1] = b;
                        s[2] = c;
                        let sr = lat.rank(&s) as usize;
                        num_naive[sr] += w * e;
                        den_naive[sr] += w;
                    }
                }
            }
        }
        for idx in 0..lat.len() {
            assert!(
                (num[idx] - num_naive[idx]).abs() <= 1e-6 * num_naive[idx].abs().max(1.0),
                "num mismatch at {idx}: {} vs {}",
                num[idx],
                num_naive[idx]
            );
            assert!(
                (den[idx] - den_naive[idx]).abs() <= 1e-6 * den_naive[idx].abs().max(1.0),
                "den mismatch at {idx}: {} vs {}",
                den[idx],
                den_naive[idx]
            );
        }
    }
}
