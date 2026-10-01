// Copyright (C) 2020-2026 Andy Kurnia.

pub const UNPLAYABLE: i32 = i32::MIN / 2;

const MAX_LETTERS: usize = 64;

pub struct MultisetLattice {
    num_letters: usize,
    rack_size: usize,
    pascal: crate::prob::Pascal,
    size_offset: Vec<u64>,
}

impl MultisetLattice {
    #[inline(always)]
    pub fn new(num_letters: usize, rack_size: usize) -> Self {
        assert!((1..=MAX_LETTERS).contains(&num_letters));
        let n_max = rack_size + num_letters;

        let pascal = crate::prob::Pascal::with_rows(n_max + 1);

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

    #[inline(always)]
    pub fn full_rack_start(&self) -> usize {
        self.size_offset[self.rack_size] as usize
    }

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
                within += self.c((rem - j) + parts - 1, parts - 1);
            }
            rem -= ct;
        }
        (self.size_offset[s] + within) as u32
    }

    #[inline(always)]
    pub fn rank_sparse(&self, s: usize, items: &[(u8, u8)]) -> u32 {
        self.rank_sparse_iter(s, items.iter().copied())
    }

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

    #[inline]
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

    #[inline]
    pub fn tally(&self, idx: usize) -> Vec<u8> {
        let mut out = vec![0u8; self.num_letters];
        self.unrank_into(idx, &mut out);
        out
    }
}

#[inline]
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

#[inline]
pub fn best_equity_table(lat: &MultisetLattice, sheet: &[i32], leave: &[i32], out: &mut [i32]) {
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];
    // the nonzero letters are added high-to-low, so each letter's suffix size is
    // fixed the moment it is added and its rank contribution is known then.
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

            let (t, cnt) = self.nz[i - 1];
            let parts = self.n - 1 - t;
            for cp in 0..=cnt {
                let ck = cnt - cp;

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

#[inline]
pub fn apportion_table(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    num: &mut [f64],
    den: &mut [f64],
) {
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];

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

// add(idx, t) is the rank of multiset(idx) with one more t.
pub struct AddTable {
    num_letters: usize,
    add: Vec<u32>,
}

impl AddTable {
    #[inline(always)]
    pub fn new(lat: &MultisetLattice) -> Self {
        Self::new_with_threads(lat, num_cpus::get())
    }

    #[inline(always)]
    pub fn new_with_threads(lat: &MultisetLattice, num_threads: usize) -> Self {
        let n = lat.num_letters();
        let rows = lat.full_rack_start();
        let mut add = vec![0u32; rows * n];

        let num_threads = num_threads.max(1).min(rows.max(1));
        let chunk_rows = rows.div_ceil(num_threads);
        std::thread::scope(|s| {
            for (ci, add_chunk) in add.chunks_mut(chunk_rows * n).enumerate() {
                let base = ci * chunk_rows;
                s.spawn(move || {
                    let mut tally = vec![0u8; n];
                    for (j, row) in add_chunk.chunks_exact_mut(n).enumerate() {
                        let idx = base + j;
                        lat.unrank_into(idx, &mut tally);
                        for (t, slot) in row.iter_mut().enumerate() {
                            tally[t] += 1;
                            *slot = lat.rank(&tally);
                            tally[t] -= 1;
                        }
                    }
                });
            }
        });
        Self {
            num_letters: n,
            add,
        }
    }

    #[inline(always)]
    pub fn add(&self, idx: usize, t: usize) -> usize {
        // SAFETY: add holds full_rack_start()*num_letters u32s; the caller's contract is
        // idx < full_rack_start() and t < num_letters, so idx*num_letters+t is
        // < add.len().
        unsafe { *self.add.get_unchecked(idx * self.num_letters + t) as usize }
    }
}

#[inline]
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

#[inline]
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

            self.rec_k(t + 1, remaining, k_idx, r_idx);

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

    for pj in 1..lat.len() {
        // SAFETY: pj is the loop index over 1..lat.len(), so pj < lat.len() = sheet.len().
        let sp_val = unsafe { *sheet.get_unchecked(pj) };
        if sp_val <= 0 {
            continue;
        }
        lat.unrank_into(pj, &mut p_tally[..n]);
        let psize: usize = p_tally[..n].iter().map(|&c| c as usize).sum();
        let krem = rack_size - psize;

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

#[derive(Clone)]
pub struct ApportionBoard<'a> {
    pub sheet: &'a [i32],
    pub leave: &'a [i32],
    pub unseen: &'a [u8],
}

pub struct ApportionOut<'a> {
    pub num: &'a mut [f64],
    pub den: &'a mut [f64],
}

#[derive(Clone)]
pub struct ApportionMode {
    pub zeta: bool,
    pub null_leave: bool,
    pub scatter: bool,
}

#[derive(Clone)]
pub struct OppDenialParams<'a> {
    pub oppdenial_rack: f64,
    pub marginal: &'a [f64],
    pub oppdenial_exact: f64,
    pub oppdenial_exact_term: &'a [f64],
}

#[inline]
pub fn apportion_fused(
    lat: &MultisetLattice,
    add: &AddTable,
    board: &ApportionBoard,
    out: ApportionOut,
    maxsheet: &mut [i32],
    mode: ApportionMode,
    opp_denial: &OppDenialParams,
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
    let OppDenialParams {
        oppdenial_rack,
        marginal,
        oppdenial_exact,
        oppdenial_exact_term,
    } = *opp_denial;
    let n = lat.num_letters();

    let subset_max = null_leave && zeta;
    let scatter_active = scatter && !null_leave && zeta;
    if subset_max {
        subset_max_transform(lat, add, sheet, maxsheet);
    } else if scatter_active {
        subset_max_transform(lat, add, leave, maxsheet);
        scatter_words(lat, add, sheet, leave, unseen, maxsheet);
    }
    let best_from_maxsheet = subset_max || scatter_active;

    let rack_size = lat.rack_size();

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
        oppdenial_rack: f64,
        marginal: &'a [f64],
        oppdenial_exact: f64,
        oppdenial_exact_term: &'a [f64],
        nz: &'a mut [(usize, u8)],
    }
    impl Ctx<'_> {
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

        fn enum_drawable(&mut self, t: usize, remaining: usize, w: f64, idx: usize, m: usize) {
            if remaining == 0 {
                let best = if self.best_from_maxsheet {
                    // SAFETY: idx is the full-rack lattice index built up from 0 via the add
                    // table, so < lat.len(); maxsheet is lat.len()-sized (filled above).
                    unsafe { *self.maxsheet.get_unchecked(idx) }
                } else {
                    let mut b = UNPLAYABLE;
                    self.rec_max(m, 0, 0, &mut b);
                    b
                };

                let mut best_f = best as f64;
                if self.oppdenial_rack != 0.0 {
                    let mut opp = 0.0f64;
                    for &(t, c) in &self.nz[..m] {
                        // SAFETY: t is a rack letter from nz (t <
                        // num_letters); reached only when oppdenial_rack !=
                        // 0.0, where the caller passes the num_letters-length
                        // denial marginals (&[] is passed only when
                        // oppdenial_rack == 0, which skips this branch).
                        opp += c as f64 * unsafe { *self.marginal.get_unchecked(t) };
                    }
                    best_f += self.oppdenial_rack * opp;
                }

                if self.oppdenial_exact != 0.0 {
                    // SAFETY: idx is the full-rack lattice index (<
                    // lat.len()); reached only when oppdenial_exact != 0.0,
                    // where the caller passes the lat.len() per-rack term (&[]
                    // is passed only when oppdenial_exact == 0, which skips
                    // this branch).
                    best_f -= self.oppdenial_exact
                        * unsafe { *self.oppdenial_exact_term.get_unchecked(idx) };
                }
                if self.zeta {
                    // seed the superset-sum source on the full-rack index `idx`.
                    // SAFETY: idx is the full-rack lattice index built up from 0 via the add
                    // table, so < lat.len(); num and den are lat.len()-sized.
                    unsafe {
                        *self.num.get_unchecked_mut(idx) = w * best_f;
                        *self.den.get_unchecked_mut(idx) = w;
                    }
                } else {
                    self.apportion_rec(m, 0, w, w * best_f);
                }
                return;
            }
            if t == self.n || (self.suffix_cap[t] as usize) < remaining {
                return;
            }

            self.enum_drawable(t + 1, remaining, w, idx, m);

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

        #[inline]
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
        oppdenial_rack,
        marginal,
        oppdenial_exact,
        oppdenial_exact_term,
        nz: &mut nz,
    };
    ctx.enum_drawable(0, rack_size, 1.0, 0, 0);
    if zeta {
        ctx.fold_zeta(lat.full_rack_start());
    }
}

#[inline]
pub fn opp_denial_marginals(
    lat: &MultisetLattice,
    add: &AddTable,
    best: &[i32],
    unseen: &[u8],
    marginal: &mut [f64],
) {
    let n = lat.num_letters();
    let rack_size = lat.rack_size();
    let mut suffix_cap = [0u32; MAX_LETTERS + 1];
    for t in (0..n).rev() {
        suffix_cap[t] = suffix_cap[t + 1] + (unseen[t] as u32).min(rack_size as u32);
    }
    let mut num_u = 0f64;
    let mut den_u = 0f64;
    let mut swb = [0f64; MAX_LETTERS]; // swb[t] = sum_R w*best*R[t]
    let mut tw = [0f64; MAX_LETTERS]; // tw[t] = sum_R w*R[t]

    struct Ctx<'a> {
        n: usize,
        unseen: &'a [u8],
        suffix_cap: &'a [u32],
        add: &'a AddTable,
        best: &'a [i32],
        num_u: &'a mut f64,
        den_u: &'a mut f64,
        swb: &'a mut [f64],
        tw: &'a mut [f64],
        nz: &'a mut [(usize, u8)],
    }
    impl Ctx<'_> {
        fn rec(&mut self, t: usize, remaining: usize, w: f64, idx: usize, m: usize) {
            if remaining == 0 {
                // SAFETY: idx is a full-rack lattice index built up via the add table (size ==
                // rack_size at remaining == 0), < lat.len(); best is the lat.len()
                // best_equity buffer.
                let b = unsafe { *self.best.get_unchecked(idx) } as f64;
                let wb = w * b;
                *self.num_u += wb;
                *self.den_u += w;
                for &(letter, cnt) in &self.nz[..m] {
                    let c = cnt as f64;
                    // SAFETY: letter is a rack letter from nz (letter <
                    // num_letters <= MAX_LETTERS); swb and tw are
                    // MAX_LETTERS-length stack arrays.
                    unsafe {
                        *self.swb.get_unchecked_mut(letter) += wb * c;
                        *self.tw.get_unchecked_mut(letter) += w * c;
                    }
                }
                return;
            }
            if t == self.n || (self.suffix_cap[t] as usize) < remaining {
                return;
            }
            self.rec(t + 1, remaining, w, idx, m);
            let nt = self.unseen[t] as usize;
            let cap = nt.min(remaining);
            let mut binom = 1.0f64;
            let mut idx_c = idx;
            for c in 1..=cap {
                binom = binom * (nt - c + 1) as f64 / c as f64;
                idx_c = self.add.add(idx_c, t);
                self.nz[m] = (t, c as u8);
                self.rec(t + 1, remaining - c, w * binom, idx_c, m + 1);
            }
        }
    }
    let mut nz = [(0usize, 0u8); MAX_LETTERS];
    Ctx {
        n,
        unseen,
        suffix_cap: &suffix_cap,
        add,
        best,
        num_u: &mut num_u,
        den_u: &mut den_u,
        swb: &mut swb,
        tw: &mut tw,
        nz: &mut nz,
    }
    .rec(0, rack_size, 1.0, 0, 0);
    let baseline = if den_u > 0.0 { num_u / den_u } else { 0.0 };
    for t in 0..n {
        let u = unseen[t] as f64;
        let opp_t = if u > 0.0 {
            let nt = num_u - swb[t] / u;
            let dt = den_u - tw[t] / u;
            if dt > 0.0 { nt / dt } else { baseline }
        } else {
            baseline
        };
        marginal[t] = baseline - opp_t;
    }
}

struct DrawCtx<'a> {
    n: usize,
    pool: &'a [u8],
    suffix_cap: &'a [u32],
    add: &'a AddTable,
    best: &'a [i32],
}
impl DrawCtx<'_> {
    fn aggregate(
        &self,
        t: usize,
        remaining: usize,
        w: f64,
        ridx: usize,
        num: &mut f64,
        den: &mut f64,
    ) {
        if remaining == 0 {
            // SAFETY: at remaining == 0, ridx is a complete full-rack lattice index (the
            // caller's start advanced one tile at a time via the add table), <
            // lat.len(); best is the lat.len() best_equity buffer.
            let b = unsafe { *self.best.get_unchecked(ridx) } as f64;
            *num += w * b;
            *den += w;
            return;
        }
        if t == self.n || (self.suffix_cap[t] as usize) < remaining {
            return;
        }
        self.aggregate(t + 1, remaining, w, ridx, num, den);
        let nt = self.pool[t] as usize;
        let cap = nt.min(remaining);
        let mut binom = 1.0f64;
        let mut idx = ridx;
        for c in 1..=cap {
            binom = binom * (nt - c + 1) as f64 / c as f64;
            idx = self.add.add(idx, t);
            self.aggregate(t + 1, remaining - c, w * binom, idx, num, den);
        }
    }
}

#[inline]
pub fn opp_value_per_rack(
    lat: &MultisetLattice,
    add: &AddTable,
    best: &[i32],
    unseen: &[u8],
    out: &mut [f64],
) {
    let n = lat.num_letters();
    let rack_size = lat.rack_size();

    struct Ctx<'a> {
        n: usize,
        unseen: &'a [u8],
        pool: &'a mut [u8],
        outer_cap: &'a [u32],
        add: &'a AddTable,
        best: &'a [i32],
        rack_size: usize,
        out: &'a mut [f64],
    }
    impl Ctx<'_> {
        fn outer(&mut self, t: usize, remaining: usize, r_idx: usize) {
            if remaining == 0 {
                let mut suffix = [0u32; MAX_LETTERS + 1];
                for tt in (0..self.n).rev() {
                    suffix[tt] = suffix[tt + 1] + (self.pool[tt] as u32).min(self.rack_size as u32);
                }
                let (mut num, mut den) = (0.0f64, 0.0f64);
                DrawCtx {
                    n: self.n,
                    pool: &*self.pool,
                    suffix_cap: &suffix,
                    add: self.add,
                    best: self.best,
                }
                .aggregate(0, self.rack_size, 1.0, 0, &mut num, &mut den);
                self.out[r_idx] = if den > 0.0 { num / den } else { 0.0 };
                return;
            }
            if t == self.n || (self.outer_cap[t] as usize) < remaining {
                return;
            }

            self.outer(t + 1, remaining, r_idx);
            let nt = self.unseen[t] as usize;
            let cap = nt.min(remaining);
            let mut idx = r_idx;
            for c in 1..=cap {
                idx = self.add.add(idx, t);
                self.pool[t] -= 1;
                self.outer(t + 1, remaining - c, idx);
            }
            self.pool[t] += cap as u8; // restore the c tiles removed in the loop
        }
    }
    let mut outer_cap = [0u32; MAX_LETTERS + 1];
    for t in (0..n).rev() {
        outer_cap[t] = outer_cap[t + 1] + (unseen[t] as u32).min(rack_size as u32);
    }
    let mut pool = [0u8; MAX_LETTERS];
    pool[..n].copy_from_slice(&unseen[..n]);
    Ctx {
        n,
        unseen,
        pool: &mut pool,
        outer_cap: &outer_cap,
        add,
        best,
        rack_size,
        out,
    }
    .outer(0, rack_size, 0);
}

#[inline]
pub fn best_equity_argmax_table(
    lat: &MultisetLattice,
    sheet: &[i32],
    leave: &[i32],
    out_best: &mut [i32],
    out_kept_idx: &mut [u32],
    out_kept_size: &mut [u8],
) {
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];

    struct Ctx<'a> {
        nz: &'a [(usize, u8)],
        n: usize,
        lat: &'a MultisetLattice,
        sheet: &'a [i32],
        leave: &'a [i32],
        best: &'a mut i32,
        best_kr: &'a mut usize,
        best_ks: &'a mut usize,
    }
    impl Ctx<'_> {
        fn rec(&mut self, i: usize, s_p: usize, within_p: u64, s_k: usize, within_k: u64) {
            if i == 0 {
                let pr = (self.lat.size_offset[s_p] + within_p) as usize;
                let kr = (self.lat.size_offset[s_k] + within_k) as usize;
                // SAFETY: pr = size_offset[s_p]+within_p and kr = size_offset[s_k]+within_k are
                // the ranks of the played P and kept K subracks (s_p, s_k <= rack_size),
                // so each is < lat.len(); sheet and leave are lat.len()-sized.
                let v = unsafe { *self.sheet.get_unchecked(pr) }
                    + unsafe { *self.leave.get_unchecked(kr) };
                if v > *self.best {
                    *self.best = v;
                    *self.best_kr = kr;
                    *self.best_ks = s_k;
                }
                return;
            }
            let (t, cnt) = self.nz[i - 1];
            let parts = self.n - 1 - t;
            for cp in 0..=cnt {
                let ck = cnt - cp;
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
    for ridx in lo..out_best.len() {
        lat.unrank_into(ridx, &mut r[..n]);
        let mut m = 0;
        for (t, &c) in r[..n].iter().enumerate() {
            if c > 0 {
                nz[m] = (t, c);
                m += 1;
            }
        }
        let mut best = UNPLAYABLE;
        let mut best_kr = 0usize;
        let mut best_ks = 0usize;
        Ctx {
            nz: &nz[..m],
            n,
            lat,
            sheet,
            leave,
            best: &mut best,
            best_kr: &mut best_kr,
            best_ks: &mut best_ks,
        }
        .rec(m, 0, 0, 0, 0);
        out_best[ridx] = best;
        out_kept_idx[ridx] = best_kr as u32;
        out_kept_size[ridx] = best_ks as u8;
    }
}

#[derive(Clone)]
pub struct KeptArgmax<'a> {
    pub idx: &'a [u32],
    pub size: &'a [u8],
}

#[inline]
pub fn opp_me2_per_rack(
    lat: &MultisetLattice,
    add: &AddTable,
    best: &[i32],
    kept: &KeptArgmax,
    unseen: &[u8],
    me2_scale: f64,
    out_diff: &mut [f64],
) {
    let KeptArgmax {
        idx: kept_idx,
        size: kept_size,
    } = *kept;
    let n = lat.num_letters();
    let rack_size = lat.rack_size();

    struct Ctx<'a> {
        n: usize,
        unseen: &'a [u8],
        pool: &'a mut [u8],
        outer_cap: &'a [u32],
        add: &'a AddTable,
        best: &'a [i32],
        kept_idx: &'a [u32],
        kept_size: &'a [u8],
        rack_size: usize,
        me2_scale: f64,
        out_diff: &'a mut [f64],
    }
    impl Ctx<'_> {
        fn outer(&mut self, t: usize, remaining: usize, r_idx: usize) {
            if remaining == 0 {
                let mut suffix = [0u32; MAX_LETTERS + 1];
                for tt in (0..self.n).rev() {
                    suffix[tt] = suffix[tt + 1] + (self.pool[tt] as u32).min(self.rack_size as u32);
                }
                let draw = DrawCtx {
                    n: self.n,
                    pool: &*self.pool,
                    suffix_cap: &suffix,
                    add: self.add,
                    best: self.best,
                };

                let (mut on, mut od) = (0.0f64, 0.0f64);
                draw.aggregate(0, self.rack_size, 1.0, 0, &mut on, &mut od);
                let opp1 = if od > 0.0 { on / od } else { 0.0 };

                let ks = self.kept_size[r_idx] as usize;
                let (mut mn, mut md) = (0.0f64, 0.0f64);
                draw.aggregate(
                    0,
                    self.rack_size - ks,
                    1.0,
                    self.kept_idx[r_idx] as usize,
                    &mut mn,
                    &mut md,
                );
                let me2 = if md > 0.0 { mn / md } else { 0.0 };
                self.out_diff[r_idx] = opp1 - self.me2_scale * me2;
                return;
            }
            if t == self.n || (self.outer_cap[t] as usize) < remaining {
                return;
            }

            self.outer(t + 1, remaining, r_idx);
            let nt = self.unseen[t] as usize;
            let cap = nt.min(remaining);
            let mut idx = r_idx;
            for c in 1..=cap {
                idx = self.add.add(idx, t);
                self.pool[t] -= 1;
                self.outer(t + 1, remaining - c, idx);
            }
            self.pool[t] += cap as u8; // restore the c tiles removed in the loop
        }
    }
    let mut outer_cap = [0u32; MAX_LETTERS + 1];
    for t in (0..n).rev() {
        outer_cap[t] = outer_cap[t + 1] + (unseen[t] as u32).min(rack_size as u32);
    }
    let mut pool = [0u8; MAX_LETTERS];
    pool[..n].copy_from_slice(&unseen[..n]);
    Ctx {
        n,
        unseen,
        pool: &mut pool,
        outer_cap: &outer_cap,
        add,
        best,
        kept_idx,
        kept_size,
        rack_size,
        me2_scale,
        out_diff,
    }
    .outer(0, rack_size, 0);
}

#[inline]
pub fn entering_fused(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    num: &mut [i128],
    den: &mut [i128],
) {
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];

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

        if b == UNPLAYABLE {
            continue;
        }
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

#[inline]
pub fn entering_leave_ci_fused(
    lat: &MultisetLattice,
    varr: &[f64],
    unseen: &[u8],
    den: &mut [f64],
    w2v: &mut [f64],
) {
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];

    struct Ctx<'a> {
        n: usize,
        lat: &'a MultisetLattice,
        unseen: &'a [u8],
        den: &'a mut [f64],
        w2v: &'a mut [f64],
    }
    impl Ctx<'_> {
        fn rec(
            &mut self,
            i: usize,
            nz: &[(usize, u8)],
            s_s: usize,
            within_s: u64,
            w: f64,
            varr_r: f64,
        ) {
            if i == 0 {
                let sr = (self.lat.size_offset[s_s] + within_s) as usize;
                // SAFETY: sr = size_offset[s_s]+within_s is the rank of a subrack S of a
                // size<=rack_size rack, < lat.len(); den and w2v are lat.len()-sized.
                unsafe {
                    *self.den.get_unchecked_mut(sr) += w;
                    *self.w2v.get_unchecked_mut(sr) += w * w * varr_r;
                }
                return;
            }
            let (t, cnt) = nz[i - 1];
            let parts = self.n - 1 - t;
            for cs in 0..=cnt {
                if cs > self.unseen[t] {
                    continue;
                }
                let cw = n_choose_k((self.unseen[t] - cs) as u64, (cnt - cs) as u64) as f64;
                if cw == 0.0 {
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
                self.rec(i - 1, nz, s_s + cs as usize, within_s + dw, w * cw, varr_r);
            }
        }
    }
    let mut ctx = Ctx {
        n,
        lat,
        unseen,
        den,
        w2v,
    };
    let lo = lat.full_rack_start();
    let mut nz: [(usize, u8); MAX_LETTERS] = [(0, 0); MAX_LETTERS];
    for ridx in lo..lat.len() {
        // SAFETY: ridx ranges over lo..lat.len() (lo = full_rack_start()), so ridx <
        // lat.len(); varr is the lat.len() per-rack variance buffer.
        let v = unsafe { *varr.get_unchecked(ridx) };

        if v < 0.0 {
            continue;
        }
        lat.unrank_into(ridx, &mut r[..n]);
        let mut m = 0;
        for (t, &c) in r[..n].iter().enumerate() {
            if c > 0 {
                nz[m] = (t, c);
                m += 1;
            }
        }
        ctx.rec(m, &nz[..m], 0, 0, 1.0, v);
    }
}

#[inline]
pub fn generate_fused(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    num: &mut [f64],
    den: &mut [f64],
) {
    struct Ctx<'a> {
        lat: &'a MultisetLattice,
        unseen: &'a [u8],
        n: usize,
        num: &'a mut [f64],
        den: &'a mut [f64],
    }
    impl Ctx<'_> {
        fn rec(
            &mut self,
            i: usize,
            nz: &[(usize, u8)],
            s_s: usize,
            within_s: u64,
            w: f64,
            vr: f64,
        ) {
            if i == 0 {
                let sr = (self.lat.size_offset[s_s] + within_s) as usize;
                // SAFETY: sr is the rank of a sub-multiset of a size<=rack_size rack,
                // so it is a valid lattice index (< num.len() == den.len() == lat.len()).
                // This is the innermost accumulation (one write per (rack, subrack)),
                // so eliding the bounds check is the perf-relevant unsafe here.
                unsafe {
                    *self.num.get_unchecked_mut(sr) += w * vr;
                    *self.den.get_unchecked_mut(sr) += w;
                }
                return;
            }
            let (t, cnt) = nz[i - 1];
            let parts = self.n - 1 - t;
            for cs in 0..=cnt {
                if cs > self.unseen[t] {
                    continue;
                }
                let cw = n_choose_k((self.unseen[t] - cs) as u64, (cnt - cs) as u64) as f64;
                if cw == 0.0 {
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
                self.rec(i - 1, nz, s_s + cs as usize, within_s + dw, w * cw, vr);
            }
        }
    }
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];
    let mut ctx = Ctx {
        lat,
        unseen,
        n,
        num,
        den,
    };
    let lo = lat.full_rack_start();
    let mut nz: [(usize, u8); MAX_LETTERS] = [(0, 0); MAX_LETTERS];

    for (ridx, &b) in best.iter().enumerate().skip(lo) {
        if b == UNPLAYABLE {
            continue;
        }
        ctx.lat.unrank_into(ridx, &mut r[..n]);
        let mut m = 0;
        for (t, &c) in r[..n].iter().enumerate() {
            if c > 0 {
                nz[m] = (t, c);
                m += 1;
            }
        }
        ctx.rec(m, &nz[..m], 0, 0, 1.0, b as f64);
    }
}

#[inline]
pub fn mark_drawable_best(
    lat: &MultisetLattice,
    add: &AddTable,
    best: &[i32],
    unseen: &[u8],
    out: &mut [i32],
) {
    let n = lat.num_letters();
    let rack_size = lat.rack_size();
    let mut suffix_cap = [0u32; MAX_LETTERS + 1];
    for t in (0..n).rev() {
        suffix_cap[t] = suffix_cap[t + 1] + (unseen[t] as u32).min(rack_size as u32);
    }

    struct Ctx<'a> {
        n: usize,
        unseen: &'a [u8],
        suffix_cap: &'a [u32],
        add: &'a AddTable,
        best: &'a [i32],
        out: &'a mut [i32],
    }
    impl Ctx<'_> {
        fn rec(&mut self, t: usize, remaining: usize, idx: usize) {
            if remaining == 0 {
                // SAFETY: at remaining == 0, idx is a full-rack lattice index (built up one tile
                // at a time via the add table), < lat.len(); best and out are both
                // lat.len()-sized.
                unsafe {
                    *self.out.get_unchecked_mut(idx) = *self.best.get_unchecked(idx);
                }
                return;
            }
            if t == self.n || (self.suffix_cap[t] as usize) < remaining {
                return;
            }

            self.rec(t + 1, remaining, idx);

            let cap = (self.unseen[t] as usize).min(remaining);
            let mut idx_c = idx;
            for c in 1..=cap {
                idx_c = self.add.add(idx_c, t);
                self.rec(t + 1, remaining - c, idx_c);
            }
        }
    }
    Ctx {
        n,
        unseen,
        suffix_cap: &suffix_cap,
        add,
        best,
        out,
    }
    .rec(0, rack_size, 0);
}

#[inline]
pub fn leave_value_by_draw(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    s_tally: &[u8],
) -> i32 {
    let n = lat.num_letters();

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
pub fn dynamic_leave_value(
    lat: &MultisetLattice,
    add: &AddTable,
    full_v: &[i32],
    pool: &[u8],
    s_ridx: usize,
    draw: usize,
) -> i32 {
    let n = lat.num_letters();
    let rack_size = lat.rack_size();
    let mut suffix_cap = [0u32; MAX_LETTERS + 1];
    for t in (0..n).rev() {
        suffix_cap[t] = suffix_cap[t + 1] + (pool[t] as u32).min(rack_size as u32);
    }
    let (mut num, mut den) = (0.0f64, 0.0f64);
    DrawCtx {
        n,
        pool,
        suffix_cap: &suffix_cap,
        add,
        best: full_v,
    }
    .aggregate(0, draw, 1.0, s_ridx, &mut num, &mut den);
    if den > 0.0 {
        (num / den) as i32
    } else {
        UNPLAYABLE
    }
}

#[inline]
pub fn fill_lattice_leaves(
    lat: &MultisetLattice,
    out: &mut [i32],
    value_of: impl Fn(&[u8]) -> i32,
) {
    let mut tally = vec![0u8; lat.num_letters()];
    for (idx, slot) in out.iter_mut().enumerate() {
        lat.unrank_into(idx, &mut tally);
        *slot = value_of(&tally);
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

#[inline]
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

    placed.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

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
        return;
    }
    let leftover = num_blanks_eff - total_forced;
    let mut tally = [0u8; MAX_LETTERS];

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

            let mut drop_run = 0i32;
            for e in &self.placed[start..start + forced] {
                drop_run += e.1;
            }
            for extra in 0..=max_extra {
                if extra > 0 {
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
    #[inline]
    fn lattice_roundtrips_and_counts() {
        let lat = MultisetLattice::new(3, 2);

        assert_eq!(lat.len(), 10);
        for idx in 0..lat.len() {
            let tally = lat.tally(idx);
            assert_eq!(lat.rank(&tally), idx as u32);
            assert!(tally.iter().map(|&c| c as usize).sum::<usize>() <= 2);
        }
    }

    #[test]
    #[inline]
    fn lattice_roundtrips_english_sized() {
        let lat = MultisetLattice::new(27, 7);
        assert_eq!(lat.len(), 5_379_616);
        let mut buf = vec![0u8; 27];
        for idx in (0..lat.len()).step_by(997) {
            lat.unrank_into(idx, &mut buf);
            assert_eq!(lat.rank(&buf), idx as u32, "roundtrip idx {idx}");
        }
    }

    #[test]
    #[inline]
    fn rank_sparse_matches_rank() {
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

        assert_eq!(lat.rank_sparse(0, &[]), lat.rank(&[0u8; 27]));
    }

    #[test]
    #[inline]
    fn naive_best_equity_matches_hand_calc() {
        let lat = MultisetLattice::new(2, 2);

        let mut sheet = vec![0i32; lat.len()];
        sheet[lat.rank(&[1, 0]) as usize] = 5_000;
        sheet[lat.rank(&[0, 1]) as usize] = 3_000;
        let mut leave = vec![0i32; lat.len()];
        leave[lat.rank(&[1, 0]) as usize] = 4_000;
        leave[lat.rank(&[0, 1]) as usize] = 1_000;

        let (eq, kept) = naive_best_equity(&lat, &sheet, &leave, &[1, 1]);
        assert_eq!(eq, 7_000);
        assert_eq!(kept, vec![0u8]);
    }

    #[test]
    #[inline]
    fn fast_conv_matches_naive() {
        let lat = MultisetLattice::new(4, 4);

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

        for (idx, &b) in best.iter().enumerate().skip(lat.full_rack_start()) {
            let tally = lat.tally(idx);
            assert_eq!(tally.iter().map(|&c| c as usize).sum::<usize>(), 4);
            let (naive, _) = naive_best_equity(&lat, &sheet, &leave, &tally);
            assert_eq!(b, naive, "mismatch at idx {idx} tally {tally:?}");
        }
    }

    #[test]
    #[inline]
    fn draw_average_weights_and_full_leave() {
        let lat = MultisetLattice::new(2, 2);
        let unseen = [1u8, 1u8];
        let mut best = vec![0i32; lat.len()];
        best[lat.rank(&[2, 0]) as usize] = 10_000;
        best[lat.rank(&[1, 1]) as usize] = 6_000;
        best[lat.rank(&[0, 2]) as usize] = 2_000;
        best[lat.rank(&[1, 0]) as usize] = 100;
        best[lat.rank(&[0, 1]) as usize] = 200;

        let e = leave_value_by_draw(&lat, &best, &unseen, &[0u8, 0u8]);
        assert_eq!(e, 6_000);

        let f = leave_value_by_draw(&lat, &best, &unseen, &[1u8, 1u8]);
        assert_eq!(f, 6_000);
    }

    #[test]
    #[inline]
    fn entering_fused_matches_draw() {
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
    #[inline]
    fn dynamic_leave_matches_draw_with_s_added() {
        let lat = MultisetLattice::new(3, 3);
        let add = AddTable::new(&lat);
        let pool = [3u8, 2u8, 2u8];
        let mut full_v = vec![UNPLAYABLE; lat.len()];
        for (idx, slot) in full_v.iter_mut().enumerate() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            *slot = h.rem_euclid(20_000) - 5_000;
        }
        for s_idx in 0..lat.len() {
            let s = lat.tally(s_idx);
            let s_size: usize = s.iter().map(|&c| c as usize).sum();
            let draw = lat.rack_size() - s_size;
            let mut unseen = [0u8; 3];
            for t in 0..3 {
                unseen[t] = pool[t] + s[t];
            }
            let dyn_v = dynamic_leave_value(&lat, &add, &full_v, &pool, s_idx, draw);
            let ref_v = leave_value_by_draw(&lat, &full_v, &unseen, &s);
            assert_eq!(dyn_v, ref_v, "S={s:?} (idx {s_idx})");
        }
    }

    #[test]
    #[inline]
    fn entering_leave_ci_matches_brute() {
        let lat = MultisetLattice::new(3, 3);
        let unseen = [4u8, 3u8, 2u8];
        let n = lat.num_letters();
        let mut varr = vec![0f64; lat.len()];
        for (idx, slot) in varr.iter_mut().enumerate().skip(lat.full_rack_start()) {
            let h = (idx as u32).wrapping_mul(2654435761u32);
            *slot = (h % 9000) as f64; // non-negative per-rack variance
        }
        let mut den = vec![0f64; lat.len()];
        let mut w2v = vec![0f64; lat.len()];
        entering_leave_ci_fused(&lat, &varr, &unseen, &mut den, &mut w2v);
        let mut den_b = vec![0f64; lat.len()];
        let mut w2v_b = vec![0f64; lat.len()];
        for (ridx, &vr) in varr.iter().enumerate().skip(lat.full_rack_start()) {
            let rk = lat.tally(ridx);
            let mut s = vec![0u8; n];
            for a in 0..=rk[0] {
                for b in 0..=rk[1] {
                    for c in 0..=rk[2] {
                        s[0] = a;
                        s[1] = b;
                        s[2] = c;
                        let mut cw = 1.0f64;
                        for t in 0..n {
                            if s[t] > unseen[t] {
                                cw = 0.0;
                                break;
                            }
                            cw *=
                                n_choose_k((unseen[t] - s[t]) as u64, (rk[t] - s[t]) as u64) as f64;
                        }
                        if cw == 0.0 {
                            continue;
                        }
                        let sr = lat.rank(&s) as usize;
                        den_b[sr] += cw;
                        w2v_b[sr] += cw * cw * vr;
                    }
                }
            }
        }
        for idx in 0..lat.len() {
            assert!(
                (den[idx] - den_b[idx]).abs() <= 1e-6 * den_b[idx].abs().max(1.0),
                "den {idx}: {} vs brute {}",
                den[idx],
                den_b[idx],
            );
            assert!(
                (w2v[idx] - w2v_b[idx]).abs() <= 1e-3 * w2v_b[idx].abs().max(1.0),
                "w2v {idx}: {} vs brute {}",
                w2v[idx],
                w2v_b[idx],
            );
        }
    }

    #[test]
    #[inline]
    fn generate_fused_matches_brute() {
        let lat = MultisetLattice::new(3, 3);
        let unseen = [4u8, 3u8, 2u8];
        let n = lat.num_letters();
        let mut best = vec![UNPLAYABLE; lat.len()];
        for (idx, slot) in best.iter_mut().enumerate().skip(lat.full_rack_start()) {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);

            if (h & 7) != 0 {
                *slot = h.rem_euclid(80_000) - 40_000;
            }
        }
        let mut num = vec![0f64; lat.len()];
        let mut den = vec![0f64; lat.len()];
        generate_fused(&lat, &best, &unseen, &mut num, &mut den);
        let mut num_b = vec![0f64; lat.len()];
        let mut den_b = vec![0f64; lat.len()];
        for (ridx, &bval) in best.iter().enumerate().skip(lat.full_rack_start()) {
            if bval == UNPLAYABLE {
                continue;
            }
            let vr = bval as f64;
            let rk = lat.tally(ridx);
            let mut s = vec![0u8; n];
            for a in 0..=rk[0] {
                for b in 0..=rk[1] {
                    for c in 0..=rk[2] {
                        s[0] = a;
                        s[1] = b;
                        s[2] = c;
                        let mut cw = 1.0f64;
                        for t in 0..n {
                            if s[t] > unseen[t] {
                                cw = 0.0;
                                break;
                            }
                            cw *=
                                n_choose_k((unseen[t] - s[t]) as u64, (rk[t] - s[t]) as u64) as f64;
                        }
                        if cw == 0.0 {
                            continue;
                        }
                        let sr = lat.rank(&s) as usize;
                        num_b[sr] += cw * vr;
                        den_b[sr] += cw;
                    }
                }
            }
        }
        for idx in 0..lat.len() {
            assert!(
                (den[idx] - den_b[idx]).abs() <= 1e-6 * den_b[idx].abs().max(1.0),
                "den {idx}: {} vs brute {}",
                den[idx],
                den_b[idx],
            );
            assert!(
                (num[idx] - num_b[idx]).abs() <= 1e-3 * num_b[idx].abs().max(1.0),
                "num {idx}: {} vs brute {}",
                num[idx],
                num_b[idx],
            );
        }
    }

    #[test]
    #[inline]
    fn apportion_matches_naive() {
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
    #[inline]
    fn mark_drawable_best_copies_drawable() {
        let lat = MultisetLattice::new(3, 3);
        let add = AddTable::new(&lat);
        let unseen = [4u8, 1u8, 2u8];
        let mut best = vec![UNPLAYABLE; lat.len()];
        for (idx, slot) in best.iter_mut().enumerate().skip(lat.full_rack_start()) {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            *slot = h.rem_euclid(20_000) - 5_000;
        }
        let mut out = vec![UNPLAYABLE; lat.len()];
        mark_drawable_best(&lat, &add, &best, &unseen, &mut out);
        let n = lat.num_letters();
        for idx in 0..lat.len() {
            let rk = lat.tally(idx);
            let size: usize = rk.iter().map(|&c| c as usize).sum();
            let drawable = size == lat.rack_size() && (0..n).all(|t| rk[t] <= unseen[t]);
            let want = if drawable { best[idx] } else { UNPLAYABLE };
            assert_eq!(out[idx], want, "mismatch at {idx}");
        }
    }

    #[test]
    #[inline]
    fn apportion_fused_matches_split() {
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
                    &OppDenialParams {
                        oppdenial_rack: 0.0,
                        marginal: &[],
                        oppdenial_exact: 0.0,
                        oppdenial_exact_term: &[],
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
    #[inline]
    fn apportion_fused_null_leave_matches() {
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
                    &OppDenialParams {
                        oppdenial_rack: 0.0,
                        marginal: &[],
                        oppdenial_exact: 0.0,
                        oppdenial_exact_term: &[],
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
    #[inline]
    fn apportion_fused_oppdenial_rack_matches_brute() {
        let lat = MultisetLattice::new(4, 4);
        let unseen = [3u8, 2u8, 4u8, 1u8];
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
        let n = lat.num_letters();
        let marginal = [1.5f64, -0.5, 3.0, 2.0];
        let oppdenial_rack = 0.75f64;

        let mut num_ref = vec![0f64; lat.len()];
        let mut den_ref = vec![0f64; lat.len()];
        let mut r = [0u8; MAX_LETTERS];
        let mut s = [0u8; MAX_LETTERS];
        for (ridx, &brack) in best.iter().enumerate().skip(lat.full_rack_start()) {
            lat.unrank_into(ridx, &mut r[..n]);
            let mut w = 1.0f64;
            let mut drawable = true;
            let mut opp = 0.0f64;
            for t in 0..n {
                if r[t] > unseen[t] {
                    drawable = false;
                    break;
                }
                w *= lat.c(unseen[t] as usize, r[t] as usize) as f64;
                opp += r[t] as f64 * marginal[t];
            }
            if !drawable {
                continue;
            }
            let val = brack as f64 + oppdenial_rack * opp;
            for sidx in 0..lat.len() {
                lat.unrank_into(sidx, &mut s[..n]);
                if (0..n).all(|t| s[t] <= r[t]) {
                    num_ref[sidx] += w * val;
                    den_ref[sidx] += w;
                }
            }
        }
        let add = AddTable::new(&lat);
        let mut maxsheet = vec![0i32; lat.len()];
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
                    &OppDenialParams {
                        oppdenial_rack,
                        marginal: &marginal,
                        oppdenial_exact: 0.0,
                        oppdenial_exact_term: &[],
                    },
                );
                for idx in 0..lat.len() {
                    assert!(
                        (num_b[idx] - num_ref[idx]).abs() <= 1e-6 * (1.0 + num_ref[idx].abs()),
                        "num at {idx} (zeta={zeta}, scatter={scatter}): {} vs {}",
                        num_b[idx],
                        num_ref[idx],
                    );
                    assert!(
                        (den_b[idx] - den_ref[idx]).abs() <= 1e-6 * (1.0 + den_ref[idx].abs()),
                        "den at {idx} (zeta={zeta}, scatter={scatter}): {} vs {}",
                        den_b[idx],
                        den_ref[idx],
                    );
                }
            }
        }
    }

    #[test]
    #[inline]
    fn opp_value_per_rack_matches_brute() {
        let lat = MultisetLattice::new(4, 3);
        let unseen = [3u8, 2u8, 4u8, 1u8];
        let n = lat.num_letters();
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
        let add = AddTable::new(&lat);
        let mut out = vec![0f64; lat.len()];
        opp_value_per_rack(&lat, &add, &best, &unseen, &mut out);
        let mut r = [0u8; MAX_LETTERS];
        let mut s = [0u8; MAX_LETTERS];
        for (ridx, &produced) in out.iter().enumerate().skip(lat.full_rack_start()) {
            lat.unrank_into(ridx, &mut r[..n]);
            if (0..n).any(|t| r[t] > unseen[t]) {
                continue; // R not drawable from unseen
            }
            let mut pool = [0u8; MAX_LETTERS];
            for t in 0..n {
                pool[t] = unseen[t] - r[t];
            }
            let (mut num, mut den) = (0f64, 0f64);
            for (r2, &b2) in best.iter().enumerate().skip(lat.full_rack_start()) {
                lat.unrank_into(r2, &mut s[..n]);
                if (0..n).any(|t| s[t] > pool[t]) {
                    continue;
                }
                let mut w = 1.0f64;
                for t in 0..n {
                    w *= lat.c(pool[t] as usize, s[t] as usize) as f64;
                }
                num += w * b2 as f64;
                den += w;
            }
            let expect = if den > 0.0 { num / den } else { 0.0 };
            assert!(
                (produced - expect).abs() <= 1e-6 * (1.0 + expect.abs()),
                "opp_value(U-R) at {ridx}: {} vs brute {}",
                produced,
                expect,
            );
        }
    }

    #[test]
    #[inline]
    fn opp_me2_per_rack_me2_scale_zero_is_opp_value() {
        let lat = MultisetLattice::new(4, 3);
        let unseen = [3u8, 2u8, 4u8, 1u8];
        let mut sheet = vec![0i32; lat.len()];
        let mut leave = vec![0i32; lat.len()];
        for idx in 0..lat.len() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            if (h & 3) != 0 {
                sheet[idx] = h.rem_euclid(20_000);
            }
            leave[idx] = h.rem_euclid(8_000) - 4_000;
        }
        let add = AddTable::new(&lat);
        let mut best = vec![UNPLAYABLE; lat.len()];
        let mut kept_idx = vec![0u32; lat.len()];
        let mut kept_size = vec![0u8; lat.len()];
        best_equity_argmax_table(
            &lat,
            &sheet,
            &leave,
            &mut best,
            &mut kept_idx,
            &mut kept_size,
        );
        let mut diff0 = vec![0f64; lat.len()];
        opp_me2_per_rack(
            &lat,
            &add,
            &best,
            &KeptArgmax {
                idx: &kept_idx,
                size: &kept_size,
            },
            &unseen,
            0.0,
            &mut diff0,
        );
        let mut opp1 = vec![0f64; lat.len()];
        opp_value_per_rack(&lat, &add, &best, &unseen, &mut opp1);
        for ridx in lat.full_rack_start()..lat.len() {
            assert!(
                (diff0[ridx] - opp1[ridx]).abs() <= 1e-6 * (1.0 + opp1[ridx].abs()),
                "me2_scale=0 at {ridx}: opp_me2 {} vs opp_value {}",
                diff0[ridx],
                opp1[ridx],
            );
        }
    }

    #[test]
    #[inline]
    fn apportion_fused_oppdenial_exact_matches_brute() {
        let lat = MultisetLattice::new(4, 4);
        let unseen = [3u8, 2u8, 4u8, 1u8];
        let n = lat.num_letters();
        let mut sheet = vec![0i32; lat.len()];
        let mut leave = vec![0i32; lat.len()];
        for idx in 0..lat.len() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            if (h & 3) != 0 {
                sheet[idx] = h.rem_euclid(20_000);
            }
            leave[idx] = h.rem_euclid(8_000) - 4_000;
        }
        let add = AddTable::new(&lat);
        let mut best = vec![UNPLAYABLE; lat.len()];
        let mut kept_idx = vec![0u32; lat.len()];
        let mut kept_size = vec![0u8; lat.len()];
        best_equity_argmax_table(
            &lat,
            &sheet,
            &leave,
            &mut best,
            &mut kept_idx,
            &mut kept_size,
        );
        let mut term = vec![0f64; lat.len()];
        opp_me2_per_rack(
            &lat,
            &add,
            &best,
            &KeptArgmax {
                idx: &kept_idx,
                size: &kept_size,
            },
            &unseen,
            1.0,
            &mut term,
        );
        let oppdenial_exact = 0.5f64;
        let mut num_ref = vec![0f64; lat.len()];
        let mut den_ref = vec![0f64; lat.len()];
        let mut r = [0u8; MAX_LETTERS];
        let mut s = [0u8; MAX_LETTERS];
        for ridx in lat.full_rack_start()..lat.len() {
            lat.unrank_into(ridx, &mut r[..n]);
            let mut w = 1.0f64;
            let mut drawable = true;
            for t in 0..n {
                if r[t] > unseen[t] {
                    drawable = false;
                    break;
                }
                w *= lat.c(unseen[t] as usize, r[t] as usize) as f64;
            }
            if !drawable {
                continue;
            }
            let val = best[ridx] as f64 - oppdenial_exact * term[ridx];
            for sidx in 0..lat.len() {
                lat.unrank_into(sidx, &mut s[..n]);
                if (0..n).all(|t| s[t] <= r[t]) {
                    num_ref[sidx] += w * val;
                    den_ref[sidx] += w;
                }
            }
        }
        let mut maxsheet = vec![0i32; lat.len()];
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
                    &OppDenialParams {
                        oppdenial_rack: 0.0,
                        marginal: &[],
                        oppdenial_exact,
                        oppdenial_exact_term: &term,
                    },
                );
                for idx in 0..lat.len() {
                    assert!(
                        (num_b[idx] - num_ref[idx]).abs() <= 1e-6 * (1.0 + num_ref[idx].abs()),
                        "num at {idx} (zeta={zeta}, scatter={scatter}): {} vs {}",
                        num_b[idx],
                        num_ref[idx],
                    );
                    assert!(
                        (den_b[idx] - den_ref[idx]).abs() <= 1e-6 * (1.0 + den_ref[idx].abs()),
                        "den at {idx} (zeta={zeta}, scatter={scatter}): {} vs {}",
                        den_b[idx],
                        den_ref[idx],
                    );
                }
            }
        }
    }

    #[test]
    #[inline]
    fn opp_me2_per_rack_matches_brute() {
        let lat = MultisetLattice::new(4, 3);
        let unseen = [3u8, 2u8, 4u8, 1u8];
        let n = lat.num_letters();
        let mut sheet = vec![0i32; lat.len()];
        let mut leave = vec![0i32; lat.len()];
        for idx in 0..lat.len() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            if (h & 3) != 0 {
                sheet[idx] = h.rem_euclid(20_000);
            }
            leave[idx] = h.rem_euclid(8_000) - 4_000;
        }
        let add = AddTable::new(&lat);
        let mut best = vec![UNPLAYABLE; lat.len()];
        let mut kept_idx = vec![0u32; lat.len()];
        let mut kept_size = vec![0u8; lat.len()];
        best_equity_argmax_table(
            &lat,
            &sheet,
            &leave,
            &mut best,
            &mut kept_idx,
            &mut kept_size,
        );

        let mut best_ref = vec![UNPLAYABLE; lat.len()];
        best_equity_table(&lat, &sheet, &leave, &mut best_ref);
        for idx in lat.full_rack_start()..lat.len() {
            assert_eq!(best[idx], best_ref[idx], "best mismatch at {idx}");
        }
        let mut out = vec![0f64; lat.len()];
        opp_me2_per_rack(
            &lat,
            &add,
            &best,
            &KeptArgmax {
                idx: &kept_idx,
                size: &kept_size,
            },
            &unseen,
            1.0,
            &mut out,
        );
        let mut r = [0u8; MAX_LETTERS];
        let mut s = [0u8; MAX_LETTERS];
        for ridx in lat.full_rack_start()..lat.len() {
            lat.unrank_into(ridx, &mut r[..n]);
            if (0..n).any(|t| r[t] > unseen[t]) {
                continue; // R not drawable from unseen
            }
            let mut pool = [0u8; MAX_LETTERS];
            for t in 0..n {
                pool[t] = unseen[t] - r[t];
            }

            let kr = kept_idx[ridx] as usize;
            let ks = kept_size[ridx] as usize;
            let mut kstar = [0u8; MAX_LETTERS];
            lat.unrank_into(kr, &mut kstar[..n]);
            assert_eq!(
                kstar[..n].iter().map(|&c| c as usize).sum::<usize>(),
                ks,
                "kept size at {ridx}"
            );
            assert!((0..n).all(|t| kstar[t] <= r[t]), "K* not <= R at {ridx}");
            let mut played = [0u8; MAX_LETTERS];
            for t in 0..n {
                played[t] = r[t] - kstar[t];
            }
            let pr = lat.rank(&played[..n]) as usize;
            assert_eq!(sheet[pr] + leave[kr], best[ridx], "K* not argmax at {ridx}");

            let (mut on, mut od) = (0f64, 0f64);
            let (mut mn, mut md) = (0f64, 0f64);
            for (r2, &b2) in best.iter().enumerate().skip(lat.full_rack_start()) {
                lat.unrank_into(r2, &mut s[..n]);
                if (0..n).all(|t| s[t] <= pool[t]) {
                    let mut w = 1.0f64;
                    for t in 0..n {
                        w *= lat.c(pool[t] as usize, s[t] as usize) as f64;
                    }
                    on += w * b2 as f64;
                    od += w;
                }
                if (0..n).all(|t| s[t] >= kstar[t] && s[t] - kstar[t] <= pool[t]) {
                    let mut w = 1.0f64;
                    for t in 0..n {
                        w *= lat.c(pool[t] as usize, (s[t] - kstar[t]) as usize) as f64;
                    }
                    mn += w * b2 as f64;
                    md += w;
                }
            }
            let opp1 = if od > 0.0 { on / od } else { 0.0 };
            let me2 = if md > 0.0 { mn / md } else { 0.0 };
            let expect = opp1 - me2;
            assert!(
                (out[ridx] - expect).abs() <= 1e-6 * (1.0 + expect.abs()),
                "opp-me2 at {ridx}: {} vs brute {} (opp1 {} me2 {})",
                out[ridx],
                expect,
                opp1,
                me2,
            );
        }
    }

    #[test]
    #[inline]
    fn opp_denial_marginals_matches_brute() {
        let lat = MultisetLattice::new(4, 4);
        let add = AddTable::new(&lat);
        let unseen = [5u8, 4u8, 6u8, 3u8];
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
        let mut marginal = vec![0f64; 4];
        opp_denial_marginals(&lat, &add, &best, &unseen, &mut marginal);
        #[inline]
        fn binom(n: u64, k: u64) -> f64 {
            if k > n {
                return 0.0;
            }
            let mut r = 1.0;
            for i in 0..k {
                r = r * (n - i) as f64 / (i + 1) as f64;
            }
            r
        }
        let opp_value = |pool: &[u8; 4]| -> f64 {
            let mut num = 0.0;
            let mut den = 0.0;
            for (idx, &b) in best.iter().enumerate().skip(lat.full_rack_start()) {
                let tally = lat.tally(idx);
                let mut w = 1.0;
                let mut ok = true;
                for t in 0..4 {
                    if tally[t] > pool[t] {
                        ok = false;
                        break;
                    }
                    w *= binom(pool[t] as u64, tally[t] as u64);
                }
                if !ok || w == 0.0 {
                    continue;
                }
                num += w * b as f64;
                den += w;
            }
            if den > 0.0 { num / den } else { 0.0 }
        };
        let base = opp_value(&unseen);
        for t in 0..4 {
            let mut pool = unseen;
            pool[t] -= 1;
            let expect = base - opp_value(&pool);
            assert!(
                (marginal[t] - expect).abs() < 1e-6,
                "marginal[{t}] = {} expected {}",
                marginal[t],
                expect
            );
        }
    }

    #[test]
    #[inline]
    fn record_blank_variants_enumerates_designations() {
        let lat = MultisetLattice::new(4, 4);
        let key = |tally: &[u8]| lat.rank(tally) as usize;
        let run = |placed: &mut [(u8, i32)], unseen: &[u8], blanks: usize, real_score: i32| {
            let mut sheet = vec![0i32; lat.len()];
            record_blank_variants(&lat, &mut sheet, real_score, placed, unseen, blanks);
            sheet
        };

        let sheet = run(&mut [(1, 10), (1, 4)], &[0, 2, 0, 0], 1, 100);
        assert_eq!(sheet[key(&[0, 2, 0, 0])], 100); // {A,A}
        assert_eq!(sheet[key(&[1, 1, 0, 0])], 96); // {blank,A}, dropped 4
        assert_eq!(sheet[key(&[2, 0, 0, 0])], 0); // {blank,blank}: only 1 blank, unreached

        let sheet = run(&mut [(1, 10), (1, 4)], &[0, 1, 0, 0], 2, 100);
        assert_eq!(sheet[key(&[0, 2, 0, 0])], 0); // {A,A} infeasible
        assert_eq!(sheet[key(&[1, 1, 0, 0])], 96); // {blank,A}: drop the cheaper (4)
        assert_eq!(sheet[key(&[2, 0, 0, 0])], 86); // {blank,blank}: drop both (4+10)

        let sheet = run(&mut [(1, 8), (2, 5), (3, 3)], &[0, 0, 0, 0], 2, 70);
        assert!(sheet.iter().all(|&v| v == 0));

        let sheet = run(&mut [(1, 8), (2, 6)], &[0, 1, 1, 0], 1, 50);
        assert_eq!(sheet[key(&[0, 1, 1, 0])], 50); // {A,B} all real
        assert_eq!(sheet[key(&[1, 0, 1, 0])], 42); // {blank,B}: A is the blank, drop 8
        assert_eq!(sheet[key(&[1, 1, 0, 0])], 44); // {blank,A}: B is the blank, drop 6
        assert_eq!(sheet[key(&[2, 0, 0, 0])], 0); // two blanks unreached (1 blank)

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
