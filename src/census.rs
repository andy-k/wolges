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

    /// Like [`rank`], but over only the NON-ZERO letters: `items` is `(letter,
    /// count)` ascending by letter, every count > 0, and `s` is their total. The
    /// zero-count letters [`rank`] iterates contribute nothing to `within` and
    /// leave `rem` unchanged, so skipping them is identical -- but O(items) instead
    /// of O(num_letters). The blank-spelling recorder ranks one variant per call with
    /// only about 1 + (distinct played letters) non-zero entries, so this avoids
    /// re-scanning the whole (mostly empty) alphabet tally every variant.
    #[inline]
    pub fn rank_sparse(&self, s: usize, items: &[(u8, u8)]) -> u32 {
        self.rank_sparse_iter(s, items.iter().copied())
    }

    /// Iterator form of [`rank_sparse`]: ranks the ascending non-zero
    /// `(letter, count)` entries without requiring them materialized in a slice,
    /// so the blank-spelling recorder can rank a variant straight from its blank +
    /// kept-run iterators with no per-leave stack array.
    #[inline]
    pub fn rank_sparse_iter(&self, s: usize, items: impl Iterator<Item = (u8, u8)>) -> u32 {
        if s > self.rack_size {
            return !0;
        }
        let l = self.num_letters;
        let mut within: u64 = 0;
        let mut rem = s;
        for (letter, ct_raw) in items {
            let t = letter as usize;
            // the last letter (index l-1) has parts == 0 and contributes nothing;
            // items are ascending, so it can only be the final entry.
            if t >= l - 1 {
                break;
            }
            let ct = ct_raw as usize;
            let parts = l - 1 - t;
            for j in 0..ct {
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

/// Parent (add-one-tile) index table over the sub-full-rack multisets. For every
/// lattice index `idx` of size < rack_size and every letter `t`, `add(idx, t)` is
/// the rank of `multiset(idx)` with one more `t`. [`apportion_fused`] walks each
/// subrack up from the empty multiset one tile at a time by table lookup, instead
/// of recomputing the binomial rank-skip (`lat.c(...)`) per rack -- step 3 is about 95%
/// of a census board, so this is the bigger-pie win. Built once per process and
/// shared read-only across the board threads. Only sub-full-rack rows are stored
/// (a subrack never grows past rack_size, so `add` is never called on a full rack):
/// full_rack_start() * num_letters u32s (about 120 MB english, about 420 MB for a 33-letter
/// lattice).
pub struct AddTable {
    num_letters: usize,
    add: Vec<u32>,
}

impl AddTable {
    pub fn new(lat: &MultisetLattice) -> Self {
        let n = lat.num_letters();
        let rows = lat.full_rack_start();
        let mut add = vec![0u32; rows * n];
        let mut tally = vec![0u8; n];
        for (idx, row) in add.chunks_exact_mut(n).enumerate() {
            lat.unrank_into(idx, &mut tally);
            for (t, slot) in row.iter_mut().enumerate() {
                tally[t] += 1;
                *slot = lat.rank(&tally);
                tally[t] -= 1;
            }
        }
        Self {
            num_letters: n,
            add,
        }
    }

    /// Rank of `multiset(idx)` with one more tile of letter `t`. Caller guarantees
    /// `idx < full_rack_start()` (size < rack_size) and `t < num_letters`.
    #[inline(always)]
    pub fn add(&self, idx: usize, t: usize) -> usize {
        // SAFETY: add holds full_rack_start()*num_letters u32s; the caller's contract is
        // idx < full_rack_start() and t < num_letters, so idx*num_letters+t is
        // < add.len().
        unsafe { *self.add.get_unchecked(idx * self.num_letters + t) as usize }
    }
}

/// Subset-max (downward zeta): dst[X] = max over all subracks P <= X of src[P], for
/// every lattice index X. `src` and `dst` are lattice-length. One pass per letter, idx
/// low->high so each +1-tile superset add(idx, t) -- a strictly higher index (larger
/// size) -- reads an idx that has already folded in its own lower-count predecessors;
/// max is idempotent, so composing the n passes yields the full multiset subset-max.
/// Source indices are the sub-full-rack block (the only ones `add` accepts); writes
/// may land on full racks. The mirror of the superset-sum fold in apportion_fused.
/// Collapses a null-leave best_equity to the sheet subset-max, and precomputes
/// maxleave(R) = max_{K<=R} leave[K] for the word-scatter path.
pub fn subset_max_transform(lat: &MultisetLattice, add: &AddTable, src: &[i32], dst: &mut [i32]) {
    let n = lat.num_letters();
    let lo = lat.full_rack_start();
    dst.copy_from_slice(src);
    for t in 0..n {
        for idx in 0..lo {
            let sp = add.add(idx, t);
            // SAFETY: idx ranges 0..full_rack_start() (< lat.len()), so idx < dst.len(); dst
            // is the lat.len() destination buffer.
            let v = unsafe { *dst.get_unchecked(idx) };
            // SAFETY: sp = add.add(idx, t) is the rank of multiset(idx) plus one tile t
            // (idx < full_rack_start(), t < num_letters), a lattice index < lat.len()
            // = dst.len().
            let cur = unsafe { dst.get_unchecked_mut(sp) };
            if v > *cur {
                *cur = v;
            }
        }
    }
}

/// Word-scatter step 2 for the full-rack path (gens > 1): materialize best_equity(R)
/// for every drawable full rack R into `best`, exactly, by exploiting the exchange
/// floor (every non-word played multiset is worth 0). best(R) = max(maxleave(R), max
/// over WORD splits P (sheet[P] > 0) of sheet[P] + leave[R - P]). The caller seeds
/// `best` with maxleave (subset_max_transform of leave); this folds in each word: for
/// every P with sheet[P] > 0, scatter sheet[P] + leave[K] into best[P + K] for every
/// drawable complement K (P + K <= unseen, |P| + |K| == rack_size). Visits only word
/// splits (about half the subracks of a rack are words), trading the per-rack 2^distinct
/// rec_max descent for word-keyed scattered writes. `best` is lattice-indexed; only
/// the full-rack block is meaningful afterwards.
fn scatter_words(
    lat: &MultisetLattice,
    add: &AddTable,
    sheet: &[i32],
    leave: &[i32],
    unseen: &[u8],
    best: &mut [i32],
) {
    let n = lat.num_letters();
    let rack_size = lat.rack_size();
    // Enumerate the drawable complement K of a fixed word P: each letter t taken
    // 0..=min(unseen[t]-P[t], remaining) times, total == rack_size - |P|. Carry K's
    // lattice index (from 0) for leave[K] and R = P+K's index (from P) for best[R];
    // both advance one tile at a time by the add table (source size < rack_size). At a
    // complete K, scatter sheet[P]+leave[K] (max) into best[R]. avail[] = unseen-P, and
    // suffix_cap prunes branches that cannot still fill `remaining`.
    // Constant context for one word P's complement walk, so rec_k carries only the
    // changing draw position and the two lattice indices -- no
    // clippy::too_many_arguments. avail/suffix_cap/sp_val are fixed for this word; best
    // is the shared scatter target, reborrowed per word.
    struct Ctx<'a> {
        sp_val: i32,
        n: usize,
        add: &'a AddTable,
        leave: &'a [i32],
        avail: &'a [u8],
        suffix_cap: &'a [u32],
        best: &'a mut [i32],
    }
    impl Ctx<'_> {
        fn rec_k(&mut self, t: usize, remaining: usize, k_idx: usize, r_idx: usize) {
            if remaining == 0 {
                // SAFETY: k_idx is the rank of the drawn complement K (|K| <= rack_size), built up
                // from 0 one tile at a time via the add table, so < lat.len(); leave is
                // lat.len()-sized.
                let cand = self.sp_val + unsafe { *self.leave.get_unchecked(k_idx) };
                // SAFETY: r_idx is the rank of R = P+K (|R| = rack_size), built up from the word
                // index pj via the add table, so < lat.len(); best is lat.len()-sized.
                let cur = unsafe { self.best.get_unchecked_mut(r_idx) };
                if cand > *cur {
                    *cur = cand;
                }
                return;
            }
            if t == self.n || (self.suffix_cap[t] as usize) < remaining {
                return;
            }
            // c = 0: skip letter t.
            self.rec_k(t + 1, remaining, k_idx, r_idx);
            // c >= 1: take c of letter t into both K and R.
            let cap = (self.avail[t] as usize).min(remaining);
            let mut kk = k_idx;
            let mut rr = r_idx;
            for c in 1..=cap {
                kk = self.add.add(kk, t);
                rr = self.add.add(rr, t);
                self.rec_k(t + 1, remaining - c, kk, rr);
            }
        }
    }
    let mut p_tally = [0u8; MAX_LETTERS];
    let mut avail = [0u8; MAX_LETTERS];
    let mut suffix_cap = [0u32; MAX_LETTERS + 1];
    // Words live only at sizes 1..=rack_size (sheet[empty] is the 0 exchange floor).
    for pj in 1..lat.len() {
        // SAFETY: pj is the loop index over 1..lat.len(), so pj < lat.len() = sheet.len().
        let sp_val = unsafe { *sheet.get_unchecked(pj) };
        if sp_val <= 0 {
            continue;
        }
        lat.unrank_into(pj, &mut p_tally[..n]);
        let psize: usize = p_tally[..n].iter().map(|&c| c as usize).sum();
        let krem = rack_size - psize;
        // drawable complement budget per letter, and its suffix sums for pruning.
        for t in 0..n {
            avail[t] = unseen[t].saturating_sub(p_tally[t]);
        }
        suffix_cap[n] = 0;
        for t in (0..n).rev() {
            suffix_cap[t] = suffix_cap[t + 1] + (avail[t] as u32).min(krem as u32);
        }
        Ctx {
            sp_val,
            n,
            add,
            leave,
            avail: &avail,
            suffix_cap: &suffix_cap,
            best: &mut *best,
        }
        .rec_k(0, krem, 0, pj);
    }
}

/// The read-only per-board arrays [`apportion_fused`] consumes: the play-value sheet,
/// the current leave table, and the unseen draw pool. Grouped into one struct so the
/// function stays under the argument-count lint without an allow.
#[derive(Clone, Copy)]
pub struct ApportionBoard<'a> {
    pub sheet: &'a [i32],
    pub leave: &'a [i32],
    pub unseen: &'a [u8],
}

/// Output accumulators for [`apportion_fused`]: the per-board num/den arrays the
/// weighted rack values are summed into, bundled so the signature stays under the
/// argument-count lint. Both are lat.len()-sized and caller-owned.
pub struct ApportionOut<'a> {
    pub num: &'a mut [f64],
    pub den: &'a mut [f64],
}

/// Which best_equity / apportion strategy [`apportion_fused`] takes (see its doc): `zeta`
/// folds with the superset-sum transform instead of the per-rack push; `null_leave`
/// takes the gen-1 subset-max; `scatter` the gens > 1 word-scatter.
#[derive(Clone, Copy)]
pub struct ApportionMode {
    pub zeta: bool,
    pub null_leave: bool,
    pub scatter: bool,
}

/// Fused step 2 + step 3 for the full-rack path: for each full rack R, compute
/// best_equity(R) inline (max over splits) and account its weighted contribution
/// w(R)*best(R) to every subrack S <= R -- ONE lattice pass, no materialized best[]
/// array. Equivalent to best_equity_table followed by apportion_table (proven
/// by apportion_fused_matches_split). The entering path can NOT fuse this
/// way: its draw-average pulls best[S+d] at random across racks, so it needs best[]
/// fully materialized. num/den caller-owned, zeroed per board.
///
/// Two ways to apportion each rack's contribution to its subracks, selected by `zeta`:
///   * `zeta == false` -- PUSH: each drawable rack walks its own subrack lattice and
///     adds (w, w*best) to each. Cost scales with the number of drawable racks times
///     subracks-per-rack, so it is cheap when few racks are drawable (small pool).
///   * `zeta == true` -- ZETA (superset-sum) transform: seed num[R]=w*best, den[R]=w
///     on the full-rack block, then fold each rack into all its subracks with one
///     pass per letter (a single +1-tile suffix-sum via the add table, indices
///     high->low so each +1-tile superset is finalized first; composing over letters
///     yields num[S]=sum_{full R>=S} w*best, den[S]=sum w). Cost is the FIXED
///     O(full_rack_start * num_letters) regardless of pool -- a big win on full pools
///     (about 10x fewer subrack touches than the push) but wasteful on tiny pools (where
///     the push visits only a handful of racks). The caller picks `zeta` by pool size.
///
/// `null_leave` flags the gen-1 bootstrap where every leave is 0 (a null klv). Then
/// best_equity(R) = max over splits of sheet[P] + leave[R-P] collapses to the pure
/// SUBSET-MAX max_{P <= R} sheet[P], which one downward (subset-max) scan over
/// `maxsheet` computes for every rack in a single shared O(full_rack_start *
/// num_letters) pass -- replacing the per-rack `rec_max` descent (the dominant big-pool
/// cost) with an array read at each drawable rack. `maxsheet` is a caller-owned
/// lattice-length scratch, written only on this path. The scan is the same fixed cost
/// as the apportionment zeta, so it pays off on the same big pools; it is taken only
/// when `null_leave && zeta` (small pools keep the cheaper per-rack rec_max).
///
/// `scatter` requests the gens > 1 (leave != 0) analog: best_equity uses the same
/// precomputed-into-`maxsheet` read, but `maxsheet` is built by `scatter_words` (the
/// leave subset-max seed plus a word-keyed scatter) instead of the per-rack rec_max.
/// Also `zeta`-gated, and exact; an opt-in alternative to rec_max for big pools.
pub fn apportion_fused(
    lat: &MultisetLattice,
    add: &AddTable,
    board: &ApportionBoard,
    out: ApportionOut,
    maxsheet: &mut [i32],
    mode: ApportionMode,
) {
    let ApportionBoard {
        sheet,
        leave,
        unseen,
    } = *board;
    let ApportionMode {
        zeta,
        null_leave,
        scatter,
    } = mode;
    let n = lat.num_letters();
    // best_equity(R) is precomputed into `maxsheet` for every rack on two paths, both
    // replacing the per-rack rec_max descent with an array read at each drawable rack;
    // each rides the big-pool `zeta` gate (the per-rack rec_max is cheaper on small
    // pools, where few racks are drawable):
    //   * subset_max -- gen-1 null leave: best(R) = max_{P<=R} sheet[P], a single
    //     subset-max of the sheet.
    //   * scatter -- gens > 1: best(R) = max(maxleave(R), over word splits), seeded by
    //     the subset-max of the leave and folded by scatter_words.
    let subset_max = null_leave && zeta;
    let scatter_active = scatter && !null_leave && zeta;
    if subset_max {
        subset_max_transform(lat, add, sheet, maxsheet);
    } else if scatter_active {
        subset_max_transform(lat, add, leave, maxsheet);
        scatter_words(lat, add, sheet, leave, unseen, maxsheet);
    }
    let best_from_maxsheet = subset_max || scatter_active;
    // Constant context for the per-rack recursions and the drawable-rack enumeration;
    // best/w vary per rack and are passed as arguments -- no clippy::too_many_arguments.
    // nz is the incrementally-built rack buffer; num/den accumulate across racks.
    let rack_size = lat.rack_size();
    // suffix_cap[t] = total unseen tiles (capped at rack_size) from letter t onward;
    // prunes enum_drawable branches that can no longer fill a full rack.
    let mut suffix_cap = [0u32; MAX_LETTERS + 1];
    for t in (0..n).rev() {
        suffix_cap[t] = suffix_cap[t + 1] + (unseen[t] as u32).min(rack_size as u32);
    }
    struct Ctx<'a> {
        n: usize,
        unseen: &'a [u8],
        suffix_cap: &'a [u32],
        add: &'a AddTable,
        sheet: &'a [i32],
        leave: &'a [i32],
        num: &'a mut [f64],
        den: &'a mut [f64],
        zeta: bool,
        best_from_maxsheet: bool,
        maxsheet: &'a [i32],
        nz: &'a mut [(usize, u8)],
    }
    impl Ctx<'_> {
        // max over splits -> best_equity(R): build the played P and kept K subracks up
        // from the empty multiset by add-table lookup (one lookup per tile) instead of
        // the binomial rank-skip.
        fn rec_max(&self, i: usize, p_idx: usize, k_idx: usize, best: &mut i32) {
            if i == 0 {
                // SAFETY: p_idx and k_idx are the ranks of the played P and kept K subracks
                // (P+K = R, a full rack), built up from 0 via the add table, so each is
                // < lat.len(); sheet and leave are lat.len()-sized.
                let v = unsafe { *self.sheet.get_unchecked(p_idx) }
                    + unsafe { *self.leave.get_unchecked(k_idx) };
                if v > *best {
                    *best = v;
                }
                return;
            }
            let (t, cnt) = self.nz[i - 1];
            // split cnt tiles of letter t into cp played (-> P) and cnt - cp kept (-> K).
            let mut pk = p_idx;
            for cp in 0..=cnt {
                let mut kk = k_idx;
                for _ in 0..(cnt - cp) {
                    kk = self.add.add(kk, t);
                }
                self.rec_max(i - 1, pk, kk, best);
                if cp < cnt {
                    pk = self.add.add(pk, t);
                }
            }
        }
        // apportion (w, we) to every subrack: build S up from the empty multiset by
        // add-table lookup.
        fn apportion_rec(&mut self, i: usize, s_idx: usize, w: f64, we: f64) {
            if i == 0 {
                // SAFETY: s_idx is the rank of a subrack S of a full rack, built up from 0 via
                // the add table, so < lat.len(); num and den are lat.len()-sized.
                unsafe {
                    *self.num.get_unchecked_mut(s_idx) += we;
                    *self.den.get_unchecked_mut(s_idx) += w;
                }
                return;
            }
            let (t, cnt) = self.nz[i - 1];
            let mut idx = s_idx;
            for cs in 0..=cnt {
                self.apportion_rec(i - 1, idx, w, we);
                if cs < cnt {
                    idx = self.add.add(idx, t);
                }
            }
        }
        // Enumerate ONLY the full racks drawable from `unseen` (each letter t taken
        // 0..=min(unseen[t], remaining) times, total == rack_size), building the nz list
        // and draw-ways weight w(R) = prod_t C(unseen[t], R[t]) incrementally with f64
        // binomials -- no per-rack n_choose_k and no multiplying out a weight only to
        // find a later factor is zero. Impossible racks are never visited: a big win on
        // small-pool boards (most of lat.len() is undrawable there), and elsewhere it
        // trades the unrank for an incremental build. suffix_cap prunes branches that
        // cannot still reach a full rack. `idx` is the lattice rank of the partial rack
        // chosen so far, advanced one tile at a time by the add table; at a complete
        // rack it is the full-rack index, so the zeta path seeds num/den there directly
        // (no subrack walk) while the push path apportions from the empty multiset.
        fn enum_drawable(&mut self, t: usize, remaining: usize, w: f64, idx: usize, m: usize) {
            if remaining == 0 {
                // best_equity(R): precomputed at R's full-rack index in `maxsheet` on the
                // subset-max (gen-1) and word-scatter (gens > 1) paths, else the per-rack
                // max-over-splits descent.
                let best = if self.best_from_maxsheet {
                    // SAFETY: idx is the full-rack lattice index built up from 0 via the add
                    // table, so < lat.len(); maxsheet is lat.len()-sized (filled above).
                    unsafe { *self.maxsheet.get_unchecked(idx) }
                } else {
                    let mut b = UNPLAYABLE;
                    self.rec_max(m, 0, 0, &mut b);
                    b
                };
                if self.zeta {
                    // seed the superset-sum source on the full-rack index `idx`.
                    // SAFETY: idx is the full-rack lattice index built up from 0 via the add
                    // table, so < lat.len(); num and den are lat.len()-sized.
                    unsafe {
                        *self.num.get_unchecked_mut(idx) = w * best as f64;
                        *self.den.get_unchecked_mut(idx) = w;
                    }
                } else {
                    self.apportion_rec(m, 0, w, w * best as f64);
                }
                return;
            }
            if t == self.n || (self.suffix_cap[t] as usize) < remaining {
                return;
            }
            // c = 0: skip letter t (idx unchanged).
            self.enum_drawable(t + 1, remaining, w, idx, m);
            // c >= 1: take c of letter t; C(nt, c) built incrementally from C(nt, c-1)
            // and idx advanced one t at a time (source size < rack_size, so add is valid).
            let nt = self.unseen[t] as usize;
            let cap = nt.min(remaining);
            let mut binom = 1.0f64;
            let mut idx_c = idx;
            for c in 1..=cap {
                binom = binom * (nt - c + 1) as f64 / c as f64;
                idx_c = self.add.add(idx_c, t);
                self.nz[m] = (t, c as u8);
                self.enum_drawable(t + 1, remaining - c, w * binom, idx_c, m + 1);
            }
        }
        // Superset-sum (zeta) transform: fold every full rack's seeded (num, den) into
        // all of its subracks. One pass per letter t turns the arrays into the suffix-sum
        // along t's count (a single +1-tile step via the add table); iterating idx
        // high->low means the +1-tile superset add(idx, t) -- a strictly higher index
        // (larger size) -- is already finalized for the letters done so far, so composing
        // the n passes gives the full multiset superset-sum. Only sub-full-rack indices
        // are written; the full-rack seeds are the maximal elements and stay as
        // w*best / w (== best for a full-rack "leave").
        fn fold_zeta(&mut self, lo: usize) {
            for t in 0..self.n {
                for idx in (0..lo).rev() {
                    let sp = self.add.add(idx, t);
                    // SAFETY: sp = add.add(idx, t) with idx in 0..lo and t < num_letters is a
                    // lattice index < lat.len(); num and den are lat.len()-sized.
                    let (sn, sd) =
                        unsafe { (*self.num.get_unchecked(sp), *self.den.get_unchecked(sp)) };
                    // SAFETY: idx ranges 0..lo (< lat.len()); num and den are lat.len()-sized.
                    unsafe {
                        *self.num.get_unchecked_mut(idx) += sn;
                        *self.den.get_unchecked_mut(idx) += sd;
                    }
                }
            }
        }
    }
    let mut nz = [(0usize, 0u8); MAX_LETTERS];
    let mut ctx = Ctx {
        n,
        unseen,
        suffix_cap: &suffix_cap,
        add,
        sheet,
        leave,
        num: out.num,
        den: out.den,
        zeta,
        best_from_maxsheet,
        maxsheet,
        nz: &mut nz,
    };
    ctx.enum_drawable(0, rack_size, 1.0, 0, 0);
    if zeta {
        ctx.fold_zeta(lat.full_rack_start());
    }
}

/// Push form of `leave_value_by_draw` for the whole leave table at once. For
/// every full rack R and every split R = S (kept) + P (played), credit best[R]
/// to leave S weighted by the ways to draw the played part P from the unseen
/// pool with S removed, w = prod_t C(unseen[t]-S[t], P[t]); then leave(S) =
/// num[S]/den[S]. This is identical to `leave_value_by_draw(S)` for each S --
/// both are exact i128 sums and i128 addition is order-free -- but computed by
/// pushing from each R in one lattice walk instead of pulling a draw recursion
/// per leave: the entering analog of `apportion_fused`, about 20x faster than
/// the per-leave pull. The cost is memory: num/den are i128 (kept exact to
/// match the pull), so the caller's two lat_len arrays are 16 bytes/leave
/// each; reduce WOLGES_THREADS if the per-thread total is too large. A split
/// whose kept part S needs more of a letter than the unseen pool holds is
/// dropped per-split (its C is 0), matching the pull and the -generate
/// decompose.
pub fn entering_fused(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    num: &mut [i128],
    den: &mut [i128],
) {
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];
    // apportion best[R] to every subrack S, weighting by the draw-ways of the played part
    // P = R - S (carried incrementally as prod_t C(unseen[t], P[t])). Same incremental
    // subrack rank as apportion_fused's apportion_rec; only the weight differs. Constant
    // + per-rack context, so rec carries only the changing state -- no
    // clippy::too_many_arguments. nz and num/den are borrowed per rack.
    struct Ctx<'a> {
        n: usize,
        lat: &'a MultisetLattice,
        unseen: &'a [u8],
        nz: &'a [(usize, u8)],
        num: &'a mut [i128],
        den: &'a mut [i128],
    }
    impl Ctx<'_> {
        fn apportion_rec(&mut self, i: usize, s_s: usize, within_s: u64, w: i128, we: i128) {
            if i == 0 {
                let sr = (self.lat.size_offset[s_s] + within_s) as usize;
                // SAFETY: sr = size_offset[s_s]+within_s is the rank of a subrack S of a
                // size<=rack_size rack, < lat.len(); num and den are the lat.len() i128
                // buffers.
                unsafe {
                    *self.num.get_unchecked_mut(sr) += we;
                    *self.den.get_unchecked_mut(sr) += w;
                }
                return;
            }
            let (t, cnt) = self.nz[i - 1];
            let parts = self.n - 1 - t;
            for cs in 0..=cnt {
                // cs tiles of letter t are KEPT in S; the played cnt-cs are drawn from
                // the pool with the kept tiles removed (unseen[t] - cs). cs > unseen[t]
                // means S is not drawable here, so it contributes nothing.
                if cs > self.unseen[t] {
                    continue;
                }
                let cw = n_choose_k((self.unseen[t] - cs) as u64, (cnt - cs) as u64) as i128;
                if cw == 0 {
                    continue;
                }
                let mut dw = 0u64;
                if t + 1 < self.n {
                    let mut a = s_s + cs as usize;
                    for _ in 0..cs {
                        dw += self.lat.c(a + parts - 1, parts - 1);
                        a -= 1;
                    }
                }
                self.apportion_rec(i - 1, s_s + cs as usize, within_s + dw, w * cw, we * cw);
            }
        }
    }
    let lo = lat.full_rack_start();
    let mut nz: [(usize, u8); MAX_LETTERS] = [(0, 0); MAX_LETTERS];
    for ridx in lo..lat.len() {
        // SAFETY: ridx ranges over lo..lat.len() (lo = full_rack_start()), so ridx <
        // lat.len(); best is the lat.len() best_equity buffer.
        let b = unsafe { *best.get_unchecked(ridx) };
        lat.unrank_into(ridx, &mut r[..n]);
        let mut m = 0;
        for (t, &c) in r[..n].iter().enumerate() {
            if c > 0 {
                nz[m] = (t, c);
                m += 1;
            }
        }
        let mut ctx = Ctx {
            n,
            lat,
            unseen,
            nz: &nz[..m],
            num: &mut *num,
            den: &mut *den,
        };
        ctx.apportion_rec(m, 0, 0, 1, b as i128);
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

/// Blank-spelling sheet recorder: given ONE all-real GADDAG traversal (its recounted
/// `real_score` and the per-placed-tile `(letter, drop)` list from
/// `play_scorer::score_and_blank_deltas`, where `drop` is how much the score falls
/// if that tile becomes a blank), enumerate every feasible blank designation of
/// that traversal and raise the play-value `sheet` for each resulting played
/// multiset. This reproduces, without the wildcard descent, exactly what the
/// wildcard sheet build records: for each placed letter L appearing `placed_L`
/// times, at least `forced_L = max(0, placed_L - real_avail_L)` of them must be
/// blanks (too few real copies in the pool), and up to `num_blanks_eff` blanks
/// total may be used (optionally "wasting" a real tile as a blank, which the
/// wildcard path also produces). For a given blank count per letter, the best
/// (max) score blanks that letter's lowest-`drop` positions, so the deltas are
/// sorted ascending per letter and consumed cheapest-first. The played multiset
/// for a designation keeps `placed_L - blank_L` real copies of L plus the total
/// blanks at letter 0 -- matching the wildcard recorder, which keys a blank tile
/// as 0. `placed` is sorted in place. Pure: only the lattice and arithmetic.
pub fn record_blank_variants(
    lat: &MultisetLattice,
    sheet: &mut [i32],
    real_score: i32,
    placed: &mut [(u8, i32)],
    unseen_tally: &[u8],
    num_blanks_eff: usize,
) {
    let n = placed.len();
    if n == 0 {
        return;
    }
    let rack_size = lat.rack_size();
    if n > rack_size {
        return;
    }
    let num_letters = lat.num_letters();
    // group by letter (ascending), and within a letter by ascending drop so the
    // cheapest positions are blanked first.
    placed.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    // runs[r] = (letter, start index in `placed`, count, forced blanks).
    let mut runs = [(0u8, 0u8, 0u8, 0u8); MAX_LETTERS];
    let mut num_runs = 0;
    let mut total_forced = 0usize;
    let mut i = 0;
    while i < n {
        let letter = placed[i].0;
        let start = i;
        while i < n && placed[i].0 == letter {
            i += 1;
        }
        let count = i - start;
        let real_avail = (unseen_tally[letter as usize] as usize).min(rack_size);
        let forced = count.saturating_sub(real_avail);
        total_forced += forced;
        runs[num_runs] = (letter, start as u8, count as u8, forced as u8);
        num_runs += 1;
    }
    if total_forced > num_blanks_eff {
        // not enough blanks to make this word at all.
        return;
    }
    let leftover = num_blanks_eff - total_forced;
    let mut tally = [0u8; MAX_LETTERS];

    // Constant context for the blank-assignment recursion, so rec carries only the
    // changing run index, leftover blanks, running blank total and dropped score -- no
    // clippy::too_many_arguments. tally is the rank scratch; sheet is the output.
    struct Ctx<'a> {
        runs: &'a [(u8, u8, u8, u8)],
        num_runs: usize,
        placed: &'a [(u8, i32)],
        tally: &'a mut [u8],
        lat: &'a MultisetLattice,
        sheet: &'a mut [i32],
        real_score: i32,
    }
    impl Ctx<'_> {
        fn rec(&mut self, ri: usize, leftover: usize, blanks_total: usize, drop_acc: i32) {
            if ri == self.num_runs {
                // Rank the variant straight from its non-zero entries -- the blanks
                // (letter 0, ascending-first) then each run's kept real count (runs
                // ascending) -- with no materialized items array. Every placed tile
                // is either blanked or kept real, so the multiset size is the whole
                // played word, `placed.len()`.
                let size = self.placed.len();
                let items = (blanks_total > 0)
                    .then_some((0u8, blanks_total as u8))
                    .into_iter()
                    .chain(self.runs.iter().take(self.num_runs).filter_map(|r| {
                        let real = self.tally[r.0 as usize];
                        (real > 0).then_some((r.0, real))
                    }));
                let key = self.lat.rank_sparse_iter(size, items);
                if key != !0 {
                    let slot = &mut self.sheet[key as usize];
                    let val = self.real_score - drop_acc;
                    if val > *slot {
                        *slot = val;
                    }
                }
                return;
            }
            let (letter, start, count, forced) = self.runs[ri];
            let (letter, start, count, forced) = (
                letter as usize,
                start as usize,
                count as usize,
                forced as usize,
            );
            let max_extra = (count - forced).min(leftover);
            // drop for the mandatory `forced` cheapest blanks of this letter.
            let mut drop_run = 0i32;
            for e in &self.placed[start..start + forced] {
                drop_run += e.1;
            }
            for extra in 0..=max_extra {
                if extra > 0 {
                    // include the next-cheapest position as an optional blank.
                    drop_run += self.placed[start + forced + extra - 1].1;
                }
                let b = forced + extra;
                self.tally[letter] = (count - b) as u8;
                self.rec(
                    ri + 1,
                    leftover - extra,
                    blanks_total + b,
                    drop_acc + drop_run,
                );
            }
            self.tally[letter] = 0;
        }
    }

    Ctx {
        runs: &runs,
        num_runs,
        placed,
        tally: &mut tally[..num_letters],
        lat,
        sheet,
        real_score,
    }
    .rec(0, leftover, 0, 0);
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
    fn rank_sparse_matches_rank() {
        // rank_sparse over the non-zero letters must equal rank over the full tally.
        let lat = MultisetLattice::new(27, 7);
        let mut buf = vec![0u8; 27];
        for idx in (0..lat.len()).step_by(733) {
            lat.unrank_into(idx, &mut buf);
            let mut items = Vec::new();
            let mut s = 0usize;
            for (t, &c) in buf.iter().enumerate() {
                if c > 0 {
                    items.push((t as u8, c));
                    s += c as usize;
                }
            }
            assert_eq!(lat.rank_sparse(s, &items), idx as u32, "sparse idx {idx}");
        }
        // empty multiset: rank 0, no items.
        assert_eq!(lat.rank_sparse(0, &[]), lat.rank(&[0u8; 27]));
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
    fn entering_fused_matches_draw() {
        // the push form (entering_fused, whole table) must equal the pull form
        // (leave_value_by_draw, per leave) for every leave -- both exact i128.
        let lat = MultisetLattice::new(3, 3);
        let unseen = [4u8, 3u8, 2u8];
        let mut best = vec![UNPLAYABLE; lat.len()];
        for (idx, slot) in best.iter_mut().enumerate().skip(lat.full_rack_start()) {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            *slot = h.rem_euclid(20_000) - 5_000;
        }
        let mut num = vec![0i128; lat.len()];
        let mut den = vec![0i128; lat.len()];
        entering_fused(&lat, &best, &unseen, &mut num, &mut den);
        for idx in 0..lat.len() {
            let s = lat.tally(idx);
            let size: usize = s.iter().map(|&c| c as usize).sum();
            if size > lat.rack_size() {
                continue;
            }
            let pull = leave_value_by_draw(&lat, &best, &unseen, &s);
            let push = if den[idx] != 0 {
                (num[idx] / den[idx]) as i32
            } else {
                UNPLAYABLE
            };
            assert_eq!(pull, push, "leave {idx} {s:?}: pull {pull} push {push}");
        }
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

    #[test]
    fn apportion_fused_matches_split() {
        // the fused step2+step3 must equal best_equity_table then
        // apportion_table for BOTH modes (push and zeta). The push adds
        // in the same order as the reference; the zeta reorders the sums, but every
        // term is an integer (w * best) far below 2^53, so f64 addition is exact and
        // order-free -- exact equality still holds.
        let lat = MultisetLattice::new(4, 4);
        let unseen = [3u8, 2u8, 4u8, 1u8];
        let mut sheet = vec![0i32; lat.len()]; // >= 0, like a built sheet
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
        let mut num_a = vec![0f64; lat.len()];
        let mut den_a = vec![0f64; lat.len()];
        apportion_table(&lat, &best, &unseen, &mut num_a, &mut den_a);
        let add = AddTable::new(&lat);
        let mut maxsheet = vec![0i32; lat.len()];
        // leave is nonzero here, so null_leave is false. Cover the per-rack rec_max
        // path (scatter=false) and the word-scatter path (scatter=true, engages only
        // with zeta); both must reproduce the reference exactly.
        for scatter in [false, true] {
            for zeta in [false, true] {
                let mut num_b = vec![0f64; lat.len()];
                let mut den_b = vec![0f64; lat.len()];
                apportion_fused(
                    &lat,
                    &add,
                    &ApportionBoard {
                        sheet: &sheet,
                        leave: &leave,
                        unseen: &unseen,
                    },
                    ApportionOut {
                        num: &mut num_b,
                        den: &mut den_b,
                    },
                    &mut maxsheet,
                    ApportionMode {
                        zeta,
                        null_leave: false,
                        scatter,
                    },
                );
                for idx in 0..lat.len() {
                    assert_eq!(
                        num_a[idx], num_b[idx],
                        "num at {idx} (zeta={zeta}, scatter={scatter})"
                    );
                    assert_eq!(
                        den_a[idx], den_b[idx],
                        "den at {idx} (zeta={zeta}, scatter={scatter})"
                    );
                }
            }
        }
    }

    #[test]
    fn apportion_fused_null_leave_matches() {
        // gen-1 bootstrap: with an all-zero leave, best_equity(R) = max_{P<=R} sheet[P].
        // The subset-max fast path (null_leave=true, zeta=true) must equal the reference
        // (best_equity_table + apportion_table) AND the general rec_max path
        // (null_leave=false), confirming the subset-max is an exact stand-in for the
        // per-rack descent on a null leave.
        let lat = MultisetLattice::new(4, 4);
        let unseen = [3u8, 2u8, 4u8, 1u8];
        let mut sheet = vec![0i32; lat.len()]; // >= 0, like a built sheet
        let leave = vec![0i32; lat.len()]; // null klv -> all zero
        for (idx, slot) in sheet.iter_mut().enumerate() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            if (h & 3) != 0 {
                *slot = h.rem_euclid(20_000);
            }
        }
        let mut best = vec![UNPLAYABLE; lat.len()];
        best_equity_table(&lat, &sheet, &leave, &mut best);
        let mut num_a = vec![0f64; lat.len()];
        let mut den_a = vec![0f64; lat.len()];
        apportion_table(&lat, &best, &unseen, &mut num_a, &mut den_a);
        let add = AddTable::new(&lat);
        let mut maxsheet = vec![0i32; lat.len()];
        // subset_max engages only when null_leave && zeta; the other three combinations
        // fall back to rec_max. All four must reproduce the reference exactly.
        for null_leave in [false, true] {
            for zeta in [false, true] {
                let mut num_b = vec![0f64; lat.len()];
                let mut den_b = vec![0f64; lat.len()];
                apportion_fused(
                    &lat,
                    &add,
                    &ApportionBoard {
                        sheet: &sheet,
                        leave: &leave,
                        unseen: &unseen,
                    },
                    ApportionOut {
                        num: &mut num_b,
                        den: &mut den_b,
                    },
                    &mut maxsheet,
                    ApportionMode {
                        zeta,
                        null_leave,
                        scatter: false,
                    },
                );
                for idx in 0..lat.len() {
                    assert_eq!(
                        num_a[idx], num_b[idx],
                        "num at {idx} (zeta={zeta}, null_leave={null_leave})"
                    );
                    assert_eq!(
                        den_a[idx], den_b[idx],
                        "den at {idx} (zeta={zeta}, null_leave={null_leave})"
                    );
                }
            }
        }
    }

    #[test]
    fn record_blank_variants_enumerates_designations() {
        // lattice: blank=0, A=1, B=2, C=3; racks up to 4 tiles.
        let lat = MultisetLattice::new(4, 4);
        let key = |tally: &[u8]| lat.rank(tally) as usize;
        let run = |placed: &mut [(u8, i32)], unseen: &[u8], blanks: usize, real_score: i32| {
            let mut sheet = vec![0i32; lat.len()];
            record_blank_variants(&lat, &mut sheet, real_score, placed, unseen, blanks);
            sheet
        };

        // two A's placed (drops 4 and 10), 2 real A available, 1 blank: forced 0,
        // leftover 1 -> {A,A} (no blank) and {blank,A} (blank the cheaper A, drop 4).
        let sheet = run(&mut [(1, 10), (1, 4)], &[0, 2, 0, 0], 1, 100);
        assert_eq!(sheet[key(&[0, 2, 0, 0])], 100); // {A,A}
        assert_eq!(sheet[key(&[1, 1, 0, 0])], 96); // {blank,A}, dropped 4
        assert_eq!(sheet[key(&[2, 0, 0, 0])], 0); // {blank,blank}: only 1 blank, unreached

        // same word, only 1 real A but 2 blanks: forced 1, leftover 1 -> {blank,A}
        // and {blank,blank}; the all-real {A,A} is infeasible (one real A).
        let sheet = run(&mut [(1, 10), (1, 4)], &[0, 1, 0, 0], 2, 100);
        assert_eq!(sheet[key(&[0, 2, 0, 0])], 0); // {A,A} infeasible
        assert_eq!(sheet[key(&[1, 1, 0, 0])], 96); // {blank,A}: drop the cheaper (4)
        assert_eq!(sheet[key(&[2, 0, 0, 0])], 86); // {blank,blank}: drop both (4+10)

        // three distinct unavailable letters, only 2 blanks: forced 3 > 2 -> skip all.
        let sheet = run(&mut [(1, 8), (2, 5), (3, 3)], &[0, 0, 0, 0], 2, 70);
        assert!(sheet.iter().all(|&v| v == 0));

        // A + B, one real each, 1 blank: blank at most one of the two.
        let sheet = run(&mut [(1, 8), (2, 6)], &[0, 1, 1, 0], 1, 50);
        assert_eq!(sheet[key(&[0, 1, 1, 0])], 50); // {A,B} all real
        assert_eq!(sheet[key(&[1, 0, 1, 0])], 42); // {blank,B}: A is the blank, drop 8
        assert_eq!(sheet[key(&[1, 1, 0, 0])], 44); // {blank,A}: B is the blank, drop 6
        assert_eq!(sheet[key(&[2, 0, 0, 0])], 0); // two blanks unreached (1 blank)

        // max-merge: a higher-scoring word for the same multiset wins.
        let mut sheet = vec![0i32; lat.len()];
        record_blank_variants(
            &lat,
            &mut sheet,
            100,
            &mut [(1, 10), (1, 4)],
            &[0, 2, 0, 0],
            1,
        );
        record_blank_variants(
            &lat,
            &mut sheet,
            120,
            &mut [(1, 10), (1, 4)],
            &[0, 2, 0, 0],
            1,
        );
        assert_eq!(sheet[key(&[0, 2, 0, 0])], 120); // {A,A} from the better word
        assert_eq!(sheet[key(&[1, 1, 0, 0])], 116); // {blank,A} likewise
    }
}
