// Copyright (C) 2020-2026 Andy Kurnia.

pub const UNPLAYABLE: i32 = i32::MIN / 2;

pub const MAX_LETTERS: usize = 64;

pub struct MultisetLattice {
    num_letters: usize,
    rack_size: usize,
    pascal: crate::prob::Pascal,
    size_offset: Vec<u64>,
    steps: RankSteps,
}

#[inline]
pub fn check_letters(num_letters: usize) -> crate::error::Returns<()> {
    if !(1..=MAX_LETTERS).contains(&num_letters) {
        return Err(
            format!("a census counts 1 to {MAX_LETTERS} letters, not {num_letters}").into(),
        );
    }
    Ok(())
}

impl MultisetLattice {
    #[inline(always)]
    pub fn new(num_letters: usize, rack_size: usize) -> crate::error::Returns<Self> {
        check_letters(num_letters)?;
        let n_max = rack_size + num_letters;

        let pascal = crate::prob::Pascal::with_rows(n_max + 1);

        let mut size_offset = vec![0u64; rack_size + 2];
        let mut count = 1u64;
        for (s, slot) in size_offset.iter_mut().enumerate().skip(1) {
            *slot = count;
            if s <= rack_size {
                count = u64::try_from(count as u128 * (s + num_letters) as u128 / s as u128)
                    .ok()
                    .filter(|&n| usize::try_from(n).is_ok())
                    .ok_or_else(|| {
                        format!(
                            "a census of {num_letters} letters and racks of {rack_size} has more multisets than can be counted"
                        )
                    })?;
            }
        }
        let steps = RankSteps::new(&pascal, num_letters, rack_size);
        Ok(Self {
            num_letters,
            rack_size,
            pascal,
            size_offset,
            steps,
        })
    }

    #[inline(always)]
    fn c(&self, n: usize, k: usize) -> u64 {
        self.pascal.binom(n, k)
    }

    #[inline(always)]
    fn steps(&self) -> &RankSteps {
        &self.steps
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.size_offset[self.rack_size + 1] as usize
    }
    #[inline]
    pub fn dense<T: Clone>(&self, value: T) -> crate::error::Returns<Vec<T>> {
        let mut table = Vec::new();
        table.try_reserve_exact(self.len())?;
        table.resize(self.len(), value);
        Ok(table)
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

    // the multisets of s tiles are the indices from size_start(s) up to
    // size_start(s + 1), for s up to rack_size
    #[inline(always)]
    pub fn size_start(&self, s: usize) -> usize {
        self.size_offset[s] as usize
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

// the nonzero letters are added high-to-low, so each letter's suffix size is
// fixed the moment it is added and its rank contribution is known then.
struct SplitCtx<'a> {
    nz: &'a [(usize, u8)],
    n: usize,
    lat: &'a MultisetLattice,
    sheet: &'a [i32],
    leave: &'a [i32],
    best: &'a mut i32,
}

impl SplitCtx<'_> {
    fn rec(&mut self, i: usize, s_p: usize, within_p: u64, s_k: usize, within_k: u64) {
        if i == 0 {
            // disposing the played tiles P is worth max(best word score, 0):
            // you can always EXCHANGE them for 0 (pre-endgame, bag non-empty).
            // That floor is baked into the sheet at build time (init 0; a word
            // only RAISES an entry), so an unreached or negative-scoring P
            // reads as 0 + leave(K) = the exchange-keep-K value.
            let pr = (self.lat.size_offset[s_p] + within_p) as usize;
            let kr = (self.lat.size_offset[s_k] + within_k) as usize;
            // Safety: pr and kr rank P and K, the parts of a rack of at most
            // rack_size tiles the lattice holds (unrank_into's, or a tally
            // best_equity_of checked), so each is below lat.len(), the length
            // of sheet and of leave (both callers check it).
            let v =
                unsafe { *self.sheet.get_unchecked(pr) } + unsafe { *self.leave.get_unchecked(kr) };
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

// the best split of the rack whose nonzero letters are nz, (letter, count) with
// the letters ascending and the counts summing to at most rack_size
#[inline(always)]
fn best_split_of(lat: &MultisetLattice, sheet: &[i32], leave: &[i32], nz: &[(usize, u8)]) -> i32 {
    let mut best = UNPLAYABLE;
    SplitCtx {
        nz,
        n: lat.num_letters(),
        lat,
        sheet,
        leave,
        best: &mut best,
    }
    .rec(nz.len(), 0, 0, 0, 0);
    best
}

// the best leave among the sub-multisets of keep tiles of the rack given as its
// tally: its value and its rank, the smallest rank among equal values (the
// first in the order each_tally_of_size walks them); the unplayable value and
// no rank for a rack holding fewer than keep tiles, a tally the lattice does
// not hold, or a leave table not the lattice's size
#[inline]
pub fn best_keep_of(lat: &MultisetLattice, leave: &[i32], tally: &[u8], keep: usize) -> (i32, u32) {
    if leave.len() != lat.len()
        || tally.len() != lat.num_letters()
        || tally.iter().map(|&c| c as usize).sum::<usize>() > lat.rack_size()
    {
        return (UNPLAYABLE, !0);
    }
    let mut nz: [(usize, u8); MAX_LETTERS] = [(0, 0); MAX_LETTERS];
    // how many tiles the nonzero letters before each hold
    let mut below = [0usize; MAX_LETTERS + 1];
    let mut m = 0;
    for (t, &c) in tally.iter().enumerate() {
        if c > 0 {
            nz[m] = (t, c);
            below[m + 1] = below[m] + c as usize;
            m += 1;
        }
    }
    let mut best = (UNPLAYABLE, !0);
    if keep <= below[m] {
        KeepCtx {
            nz: &nz[..m],
            below: &below[..=m],
            n: lat.num_letters(),
            lat,
            leave,
            best: &mut best,
        }
        .rec(m, 0, 0, keep);
    }
    best
}

// as SplitCtx, with kept tiles only: the letters are added high-to-low, and of
// each from the fewest kept that the letters below can still make up for to the
// most it holds, so every walk ends with exactly the tiles asked for kept
struct KeepCtx<'a> {
    nz: &'a [(usize, u8)],
    below: &'a [usize],
    n: usize,
    lat: &'a MultisetLattice,
    leave: &'a [i32],
    best: &'a mut (i32, u32),
}

impl KeepCtx<'_> {
    fn rec(&mut self, i: usize, s_k: usize, within_k: u64, left: usize) {
        if i == 0 {
            let kr = (self.lat.size_offset[s_k] + within_k) as u32;
            let v = self.leave[kr as usize];
            if v > self.best.0 || (v == self.best.0 && kr < self.best.1) {
                *self.best = (v, kr);
            }
            return;
        }
        let (t, cnt) = self.nz[i - 1];
        let parts = self.n - 1 - t;
        for ck in left.saturating_sub(self.below[i - 1])..=(cnt as usize).min(left) {
            let mut dwk = 0u64;
            if t + 1 < self.n {
                let mut b = s_k + ck;
                for _ in 0..ck {
                    dwk += self.lat.c(b + parts - 1, parts - 1);
                    b -= 1;
                }
            }
            self.rec(i - 1, s_k + ck, within_k + dwk, left - ck);
        }
    }
}

#[inline]
pub fn best_equity_table(lat: &MultisetLattice, sheet: &[i32], leave: &[i32], out: &mut [i32]) {
    // the walk's reads are unchecked: every table is the lattice's size
    assert!(sheet.len() == lat.len() && leave.len() == lat.len() && out.len() == lat.len());
    let n = lat.num_letters();
    let mut r = [0u8; MAX_LETTERS];
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
        *slot = best_split_of(lat, sheet, leave, &nz[..m]);
    }
}

// best_equity_table's answer for one rack of any size up to rack_size, given as
// its tally; UNPLAYABLE for a tally the lattice does not hold, or for a sheet or
// a leave table not the lattice's size (its reads are unchecked)
#[inline]
pub fn best_equity_of(lat: &MultisetLattice, sheet: &[i32], leave: &[i32], tally: &[u8]) -> i32 {
    if sheet.len() != lat.len()
        || leave.len() != lat.len()
        || tally.len() != lat.num_letters()
        || tally.iter().map(|&c| c as usize).sum::<usize>() > lat.rack_size()
    {
        return UNPLAYABLE;
    }
    let mut nz: [(usize, u8); MAX_LETTERS] = [(0, 0); MAX_LETTERS];
    let mut m = 0;
    for (t, &c) in tally.iter().enumerate() {
        if c > 0 {
            nz[m] = (t, c);
            m += 1;
        }
    }
    best_split_of(lat, sheet, leave, &nz[..m])
}

#[inline]
pub fn apportion_table(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    num: &mut [f64],
    den: &mut [f64],
) {
    // the walk's reads and writes are unchecked: every table is the lattice's
    // size
    assert!(best.len() == lat.len() && num.len() == lat.len() && den.len() == lat.len());
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
                // Safety: sr ranks a sub-multiset of a full rack the lattice
                // holds, so it is below lat.len(), the length of num and of den
                // (apportion_table checks it).
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
        // Safety: ridx runs over lo..lat.len(), and best's length is lat.len()
        // (checked above).
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

// add(idx, t) is the rank of multiset(idx) with one more t, kept index by
// index; by_letter keeps the same ranks letter by letter, for a pass of one
// letter over the lattice, and is empty unless asked for (with_letter_passes)
pub struct AddTable {
    num_letters: usize,
    add: Vec<u32>,
    by_letter: Vec<u32>,
}

impl AddTable {
    #[inline(always)]
    pub fn new(lat: &MultisetLattice) -> Self {
        Self::new_with_threads(lat, crate::parallelism::available())
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
            by_letter: Vec::new(),
        }
    }

    // the table with its ranks also kept letter by letter, on as many threads
    // (a letter each at most), so that a pass of one letter reads its ranks in
    // order where by index each would be a row apart
    pub fn with_letter_passes(mut self, num_threads: usize) -> Self {
        let n = self.num_letters;
        let rows = self.add.len() / n;
        let mut by_letter = vec![0u32; rows * n];
        let per_thread = n.div_ceil(num_threads.max(1));
        std::thread::scope(|s| {
            let add = &self.add;
            for (ci, letters) in by_letter.chunks_mut((per_thread * rows).max(1)).enumerate() {
                s.spawn(move || {
                    for (j, letter) in letters.chunks_exact_mut(rows).enumerate() {
                        let t = ci * per_thread + j;
                        for (idx, slot) in letter.iter_mut().enumerate() {
                            *slot = add[idx * n + t];
                        }
                    }
                });
            }
        });
        self.by_letter = by_letter;
        self
    }

    // for idx below the lattice's full_rack_start() and t below its num_letters
    // (it panics past the table); the rank it returns is always the lattice's,
    // below its len()
    #[inline(always)]
    pub fn add(&self, idx: usize, t: usize) -> usize {
        self.add[idx * self.num_letters + t] as usize
    }

    // whether it was made for lat: a lattice's letters and its full racks'
    // start fix its rack size, so its every rank
    #[inline(always)]
    fn made_for(&self, lat: &MultisetLattice) -> bool {
        self.num_letters == lat.num_letters()
            && self.add.len() == lat.full_rack_start() * lat.num_letters()
    }

    // add(idx, t) for every idx below the full racks, in order of idx, when the
    // table was made with_letter_passes
    #[inline(always)]
    pub fn letter(&self, t: usize) -> Option<&[u32]> {
        let rows = self.add.len() / self.num_letters;
        (!self.by_letter.is_empty()).then(|| &self.by_letter[t * rows..(t + 1) * rows])
    }
}

#[inline]
pub fn subset_max_transform(lat: &MultisetLattice, add: &AddTable, src: &[i32], dst: &mut [i32]) {
    // lift's reads and writes are unchecked: dst is the lattice's size, and
    // add's ranks are the lattice's
    assert!(dst.len() == lat.len() && add.made_for(lat));
    let n = lat.num_letters();
    let lo = lat.full_rack_start();
    dst.copy_from_slice(src);
    // a superset's value at least its subset's
    #[inline(always)]
    fn lift(dst: &mut [i32], idx: usize, sp: usize) {
        // Safety: idx runs below full_rack_start(), so below lat.len(), dst's
        // length (subset_max_transform checks it).
        let v = unsafe { *dst.get_unchecked(idx) };
        // Safety: sp is one of add's ranks (by letter, or add's own), and add
        // was made for this lattice, so sp is below lat.len(), dst's length
        // (both checked by subset_max_transform).
        let cur = unsafe { dst.get_unchecked_mut(sp) };
        if v > *cur {
            *cur = v;
        }
    }
    for t in 0..n {
        match add.letter(t) {
            Some(to) => {
                for (idx, &sp) in to[..lo].iter().enumerate() {
                    lift(dst, idx, sp as usize);
                }
            }
            None => {
                for idx in 0..lo {
                    lift(dst, idx, add.add(idx, t));
                }
            }
        }
    }
}

// what a letter's count adds to a multiset's rank (MultisetLattice::rank),
// given the letter, the tiles left from it on, and how many of those are it: a
// walk adding a letter at a time ranks as it goes, with no table of ranks to
// read. The last letter adds nothing, its count implied
struct RankSteps {
    span: usize, // rack_size + 1, the tiles left or counted
    steps: Vec<usize>,
}

impl RankSteps {
    fn new(pascal: &crate::prob::Pascal, n: usize, rack_size: usize) -> Self {
        let span = rack_size + 1;
        let mut steps = vec![0usize; n * span * span];
        for t in 0..n - 1 {
            let parts = n - 1 - t;
            for rem in 0..span {
                let mut within = 0usize;
                for ct in 1..=rem {
                    within += pascal.binom((rem - (ct - 1)) + parts - 1, parts - 1) as usize;
                    steps[(t * span + rem) * span + ct] = within;
                }
            }
        }
        Self { span, steps }
    }

    // the rank letter t adds with rem tiles left for it on, ct of them t
    #[inline(always)]
    fn of(&self, t: usize, rem: usize, ct: usize) -> usize {
        self.steps[(t * self.span + rem) * self.span + ct]
    }
}

#[inline]
fn scatter_words(
    lat: &MultisetLattice,
    sheet: &[i32],
    leave: &[i32],
    unseen: &[u8],
    best: &mut [i32],
) {
    let n = lat.num_letters();
    let rack_size = lat.rack_size();
    let steps = lat.steps();

    // a rack R is a played P with a kept K. The walk picks K a letter at a time
    // and ranks K and R as it goes. At a letter, R's count is P's plus K's, and
    // the tiles left for R from it on are P's from it on (p_rest) plus K's not
    // yet picked; once K is whole, P's later letters add p_tail to R's rank
    struct Ctx<'a> {
        sp_val: i32,
        n: usize,
        steps: &'a RankSteps,
        p_tally: &'a [u8],
        p_rest: &'a [usize],
        p_tail: &'a [usize],
        leave: &'a [i32],
        avail: &'a [u8],
        suffix_cap: &'a [u32],
        k_one: &'a [usize],
        one: &'a [usize],
        c0: &'a [usize],
        letters: &'a [u8],
        from: &'a [u8],
        k_two: &'a [(usize, usize)],
        two: &'a [(usize, usize)],
        c0_two: &'a [usize],
        best: &'a mut [i32],
    }
    impl Ctx<'_> {
        // the rest of K from letter t on, with remaining tiles (at least one)
        // still to pick. At each letter K takes none and the loop moves on, or
        // it takes some: enough to make it whole (its rack is offered), or
        // fewer (the walk goes on at the next letter)
        fn rec_k(&mut self, mut t: usize, remaining: usize, k_idx: usize, mut r_idx: usize) {
            while t < self.n && (self.suffix_cap[t] as usize) >= remaining {
                let rem_r = self.p_rest[t] + remaining;
                let pt = self.p_tally[t] as usize;
                let cap = (self.avail[t] as usize).min(remaining);
                for c in 1..=cap {
                    let k = k_idx + self.steps.of(t, remaining, c);
                    let r = r_idx + self.steps.of(t, rem_r, pt + c);
                    if c == remaining {
                        self.offer(k, r + self.p_tail[t + 1]);
                    } else if c + 1 == remaining {
                        self.last_one(t + 1, k, r);
                    } else if c + 2 == remaining {
                        self.last_two(t + 1, k, r);
                    } else {
                        self.rec_k(t + 1, remaining - c, k, r);
                    }
                }
                r_idx += self.steps.of(t, rem_r, pt);
                t += 1;
            }
        }

        // K one tile short from letter t on: each letter u from t on that the
        // pool still holds completes it. R's rank gains c0[u] - c0[t] for the
        // letters before u, which K takes none of, and one[u] from u on
        #[inline(always)]
        fn last_one(&mut self, t: usize, k_idx: usize, r_idx: usize) {
            let (letters, k_one, one) = (self.letters, self.k_one, self.one);
            let base = r_idx.wrapping_sub(self.c0[t]);
            for &u in &letters[self.from[t] as usize..] {
                let u = u as usize;
                self.offer(k_idx + k_one[u], base.wrapping_add(one[u]));
            }
        }

        // K two tiles short from letter t on: each letter u from t on that the
        // pool still holds takes one of them, the rest of K then one short, or
        // both when the pool holds two
        #[inline(always)]
        fn last_two(&mut self, t: usize, k_idx: usize, r_idx: usize) {
            let (letters, k_two, two) = (self.letters, self.k_two, self.two);
            let base = r_idx.wrapping_sub(self.c0_two[t]);
            for &u in &letters[self.from[t] as usize..] {
                let u = u as usize;
                let (k_once, k_twice) = k_two[u];
                let (r_once, r_twice) = two[u];
                self.last_one(u + 1, k_idx + k_once, base.wrapping_add(r_once));
                if self.avail[u] >= 2 {
                    self.offer(k_idx + k_twice, base.wrapping_add(r_twice));
                }
            }
        }

        // the rack P + K at r is worth at least P's play with K kept
        #[inline(always)]
        fn offer(&mut self, k_idx: usize, r: usize) {
            let cand = self.sp_val + self.leave[k_idx];
            let cur = &mut self.best[r];
            if cand > *cur {
                *cur = cand;
            }
        }
    }
    let mut p_tally = [0u8; MAX_LETTERS];
    let mut avail = [0u8; MAX_LETTERS];
    let mut suffix_cap = [0u32; MAX_LETTERS + 1];
    let mut p_rest = [0usize; MAX_LETTERS + 1];
    let mut p_tail = [0usize; MAX_LETTERS + 1];
    let k_one = (0..n).map(|u| steps.of(u, 1, 1)).collect::<Vec<_>>();
    let k_two = (0..n)
        .map(|u| (steps.of(u, 2, 1), steps.of(u, 2, 2)))
        .collect::<Vec<_>>();
    let mut two = [(0usize, 0usize); MAX_LETTERS];
    let mut c0_two = [0usize; MAX_LETTERS + 1];
    let mut one = [0usize; MAX_LETTERS];
    let mut c0 = [0usize; MAX_LETTERS + 1];
    let mut letters = [0u8; MAX_LETTERS];
    let mut from = [0u8; MAX_LETTERS + 1];

    for pj in 1..lat.len() {
        // Safety: pj runs below lat.len(), sheet's length (apportion_fused, the
        // only caller, checks it).
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
        p_rest[n] = 0;
        p_tail[n] = 0;
        for t in (0..n).rev() {
            suffix_cap[t] = suffix_cap[t + 1] + (avail[t] as u32).min(krem as u32);
            p_rest[t] = p_rest[t + 1] + p_tally[t] as usize;
            p_tail[t] = p_tail[t + 1] + steps.of(t, p_rest[t], p_tally[t] as usize);
        }
        // for K one tile short (last_one): which letters the pool still holds,
        // and what R's rank gains by each
        let mut held = 0;
        if krem > 0 {
            let mut taken_none = 0;
            for t in 0..n {
                from[t] = held as u8;
                c0[t] = taken_none;
                let (rem_r, pt) = (p_rest[t] + 1, p_tally[t] as usize);
                if avail[t] > 0 {
                    one[t] = taken_none + steps.of(t, rem_r, pt + 1) + p_tail[t + 1];
                    letters[held] = t as u8;
                    held += 1;
                }
                taken_none += steps.of(t, rem_r, pt);
            }
            from[n] = held as u8;
            c0[n] = taken_none;
        }
        // for K two tiles short (last_two): what R's rank gains by one or two
        // of each letter the pool still holds
        if krem > 1 {
            let mut taken_none = 0;
            for t in 0..n {
                c0_two[t] = taken_none;
                let (rem_r, pt) = (p_rest[t] + 2, p_tally[t] as usize);
                if avail[t] > 0 {
                    two[t] = (
                        taken_none + steps.of(t, rem_r, pt + 1),
                        if avail[t] > 1 {
                            taken_none + steps.of(t, rem_r, pt + 2) + p_tail[t + 1]
                        } else {
                            0
                        },
                    );
                }
                taken_none += steps.of(t, rem_r, pt);
            }
            c0_two[n] = taken_none;
        }
        let mut ctx = Ctx {
            sp_val,
            n,
            steps,
            p_tally: &p_tally,
            p_rest: &p_rest,
            p_tail: &p_tail,
            leave,
            avail: &avail,
            suffix_cap: &suffix_cap,
            k_one: &k_one,
            one: &one,
            c0: &c0,
            letters: &letters[..held],
            from: &from,
            k_two: &k_two,
            two: &two,
            c0_two: &c0_two,
            best: &mut *best,
        };
        let r_idx = lat.size_start(rack_size);
        if krem == 0 {
            ctx.offer(lat.size_start(0), r_idx + p_tail[0]);
        } else if krem == 1 {
            ctx.last_one(0, lat.size_start(1), r_idx);
        } else if krem == 2 {
            ctx.last_two(0, lat.size_start(2), r_idx);
        } else {
            ctx.rec_k(0, krem, lat.size_start(krem), r_idx);
        }
    }
}

pub struct ApportionBoard<'a> {
    pub sheet: &'a [i32],
    pub leave: &'a [i32],
    pub leave_max: &'a [i32], // subset_max_transform of leave
    pub unseen: &'a [u8],
}

// where a board's apportioning goes: each multiset's sums of its full racks'
// bests and deals, below the full racks (num, den: full_rack_start() long);
// each full rack's deals (ways: the rest of the lattice long), its best then in
// maxsheet, so a full rack's mean is its best
pub struct ApportionOut<'a> {
    pub num: &'a mut [f64],
    pub den: &'a mut [f64],
    pub ways: &'a mut [f64],
}

pub struct ApportionMode {
    pub zeta: bool,
    pub null_leave: bool,
    pub scatter: bool,
}

#[inline]
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
        leave_max,
        unseen,
    } = *board;
    let ApportionMode {
        zeta,
        null_leave,
        scatter,
    } = mode;
    // the walks' reads are unchecked: every table is the lattice's size, and
    // add's ranks are the lattice's
    assert!(
        sheet.len() == lat.len()
            && leave.len() == lat.len()
            && maxsheet.len() == lat.len()
            && add.made_for(lat)
    );
    let n = lat.num_letters();

    let subset_max = null_leave && zeta;
    let scatter_active = scatter && !null_leave && zeta;
    if subset_max {
        subset_max_transform(lat, add, sheet, maxsheet);
    } else if scatter_active {
        maxsheet.copy_from_slice(leave_max);
        scatter_words(lat, sheet, leave, unseen, maxsheet);
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
        ways: &'a mut [f64],
        lo: usize,
        zeta: bool,
        best_from_maxsheet: bool,
        maxsheet: &'a mut [i32],
        nz: &'a mut [(usize, u8)],
    }
    impl Ctx<'_> {
        fn rec_max(&self, i: usize, p_idx: usize, k_idx: usize, best: &mut i32) {
            if i == 0 {
                // Safety: p_idx and k_idx rank the played P and the kept K of a
                // full rack R, built up from 0 by add, whose ranks are the
                // lattice's; so each is below lat.len(), the length of sheet
                // and of leave (apportion_fused checks both).
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
                // the full rack itself keeps its share as its deals (ways)
                if s_idx < self.lo {
                    self.num[s_idx] += we;
                    self.den[s_idx] += w;
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
                    // Safety: idx ranks a full rack, built up from 0 by add,
                    // whose ranks are the lattice's; so it is below lat.len(),
                    // maxsheet's length (apportion_fused checks it).
                    unsafe { *self.maxsheet.get_unchecked(idx) }
                } else {
                    let mut b = UNPLAYABLE;
                    self.rec_max(m, 0, 0, &mut b);
                    self.maxsheet[idx] = b;
                    b
                };
                // the full rack's deals beside its best, as the sums below it
                // read it (fold_full)
                self.ways[idx - self.lo] = w;
                if !self.zeta {
                    self.apportion_rec(m, 0, w, w * best as f64);
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

        // each multiset's sums over its supersets, a letter at a time: those
        // one tile short of a full rack (short on) from the full racks' deals
        // and bests, the rest from the sums. A letter the pool lacks is in no
        // rack drawn from it: its pass would add nothing, so it is skipped
        #[inline]
        fn fold_zeta(&mut self, short: usize) {
            let (add, lo) = (self.add, self.lo);
            for t in 0..self.n {
                if self.unseen[t] == 0 {
                    continue;
                }
                match add.letter(t) {
                    Some(to) => {
                        for (idx, &sp) in (short..lo).zip(&to[short..lo]).rev() {
                            self.fold_full(idx, sp as usize);
                        }
                        for (idx, &sp) in to[..short].iter().enumerate().rev() {
                            self.fold_one(idx, sp as usize);
                        }
                    }
                    None => {
                        for idx in (short..lo).rev() {
                            self.fold_full(idx, add.add(idx, t));
                        }
                        for idx in (0..short).rev() {
                            self.fold_one(idx, add.add(idx, t));
                        }
                    }
                }
            }
        }

        // a multiset's sums gain those of its superset sp, a full rack: its
        // best for each of its deals
        #[inline(always)]
        fn fold_full(&mut self, idx: usize, sp: usize) {
            let w = self.ways[sp - self.lo];
            self.num[idx] += w * self.maxsheet[sp] as f64;
            self.den[idx] += w;
        }

        // a multiset's sums gain those of its superset sp, short of a full rack
        #[inline(always)]
        fn fold_one(&mut self, idx: usize, sp: usize) {
            let (sn, sd) = (self.num[sp], self.den[sp]);
            self.num[idx] += sn;
            self.den[idx] += sd;
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
        ways: out.ways,
        lo: lat.full_rack_start(),
        zeta,
        best_from_maxsheet,
        maxsheet,
        nz: &mut nz,
    };
    ctx.enum_drawable(0, rack_size, 1.0, 0, 0);
    if zeta {
        ctx.fold_zeta(lat.size_start(rack_size.saturating_sub(1)));
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
            // Safety: ridx is the start dynamic_leave_value was given (below
            // lat.len(), it checks) or one of add's ranks, the lattice's; so it
            // is below lat.len(), best's length (it checks that too).
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

#[cfg(test)]
#[inline]
pub fn leave_value_by_draw(
    lat: &MultisetLattice,
    best: &[i32],
    unseen: &[u8],
    s_tally: &[u8],
) -> i32 {
    // the walk's reads of best are unchecked: it is the lattice's size
    assert_eq!(best.len(), lat.len());
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
                // Safety: rank gives !0 for a tally the lattice does not hold,
                // and otherwise its rank, below lat.len(), best's length
                // (checked above).
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
    // the walk's reads of full_v are unchecked: it is the lattice's size, and
    // the walk starts at one of the lattice's ranks and adds by add's
    assert!(full_v.len() == lat.len() && s_ridx < lat.len() && add.made_for(lat));
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

// fill_lattice_leaves with klv.leave_value_from_tally, from one walk of the
// leaves' graph in place of one lookup a tally: every tally the graph lacks is
// 0
#[inline]
pub fn fill_lattice_klv_leaves<L: crate::kwg::Node>(
    lat: &MultisetLattice,
    out: &mut [i32],
    klv: &crate::klv::Klv<L>,
) {
    assert!(out.len() == lat.len());
    out.fill(0);
    klv.for_each_tally_leave(lat.num_letters(), |tally, value| {
        let ri = lat.rank(tally);
        if ri != !0 {
            out[ri as usize] = value;
        }
    });
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

    // the runs taken from the last letter back: a letter adds to the rank by
    // how many real tiles there are from it on, which the runs after it fix
    struct Ctx<'a> {
        runs: &'a [(u8, u8, u8, u8)],
        placed: &'a [(u8, i32)],
        steps: &'a RankSteps,
        size: usize,
        size_start: usize,
        sheet: &'a mut [i32],
        real_score: i32,
    }
    impl Ctx<'_> {
        // runs[..ri] still to choose, with the blanks chosen so far, the real
        // tiles from runs[ri] on, and what their letters add to the rank
        fn rec(
            &mut self,
            ri: usize,
            leftover: usize,
            blanks_total: usize,
            real_after: usize,
            rank: usize,
            drop_acc: i32,
        ) {
            if ri == 0 {
                let key = self.size_start + self.steps.of(0, self.size, blanks_total) + rank;
                let slot = &mut self.sheet[key];
                let val = self.real_score - drop_acc;
                if val > *slot {
                    *slot = val;
                }
                return;
            }
            let (letter, start, count, forced) = self.runs[ri - 1];
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
                let real = count - b;
                let from_here = real_after + real;
                self.rec(
                    ri - 1,
                    leftover - extra,
                    blanks_total + b,
                    from_here,
                    rank + self.steps.of(letter, from_here, real),
                    drop_acc + drop_run,
                );
            }
        }
    }

    Ctx {
        runs: &runs[..num_runs],
        placed,
        steps: lat.steps(),
        size: n,
        size_start: lat.size_start(n),
        sheet,
        real_score,
    }
    .rec(num_runs, leftover, 0, 0, 0, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[inline]
    fn a_lattice_ranks_every_multiset_back_to_itself() {
        let lat = MultisetLattice::new(3, 2).unwrap();

        assert_eq!(lat.len(), 10);
        for idx in 0..lat.len() {
            let tally = lat.tally(idx);
            assert_eq!(lat.rank(&tally), idx as u32);
            assert!(tally.iter().map(|&c| c as usize).sum::<usize>() <= 2);
        }
    }

    #[test]
    #[inline]
    fn a_lattice_counts_one_to_sixty_four_letters() {
        assert!(MultisetLattice::new(0, 7).is_err());
        assert!(MultisetLattice::new(1, 7).is_ok());
        assert!(MultisetLattice::new(64, 1).is_ok());
        assert!(MultisetLattice::new(65, 1).is_err());
        assert!(MultisetLattice::new(64, 255).is_err());
    }

    #[test]
    #[inline]
    fn an_english_sized_lattice_ranks_back_at_every_sample() {
        let lat = MultisetLattice::new(27, 7).unwrap();
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
        let lat = MultisetLattice::new(27, 7).unwrap();
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
        let lat = MultisetLattice::new(2, 2).unwrap();

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
    fn best_equity_table_matches_naive() {
        let lat = MultisetLattice::new(4, 4).unwrap();

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
    fn best_equity_of_matches_naive_at_every_size() {
        let lat = MultisetLattice::new(4, 4).unwrap();

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

        for (idx, &b) in best.iter().enumerate() {
            let tally = lat.tally(idx);
            let (naive, _) = naive_best_equity(&lat, &sheet, &leave, &tally);
            let one = best_equity_of(&lat, &sheet, &leave, &tally);
            assert_eq!(one, naive, "mismatch at idx {idx} tally {tally:?}");
            if idx >= lat.full_rack_start() {
                assert_eq!(one, b, "table differs at idx {idx} tally {tally:?}");
            }
        }
        assert_eq!(
            best_equity_of(&lat, &sheet, &leave, &[2, 2, 1, 0]),
            UNPLAYABLE
        );
        assert_eq!(best_equity_of(&lat, &sheet, &leave, &[1, 1, 1]), UNPLAYABLE);
        assert_eq!(
            best_equity_of(&lat, &sheet[1..], &leave, &[1, 1, 1, 1]),
            UNPLAYABLE
        );
        assert_eq!(
            best_equity_of(&lat, &sheet, &leave[1..], &[1, 1, 1, 1]),
            UNPLAYABLE
        );
    }

    // every sub-multiset of left tiles of rack, in the order each_tally_of_size
    // walks them: the first letter's count rising slowest
    fn each_sub(rack: &[u8], t: usize, left: usize, cur: &mut [u8], f: &mut impl FnMut(&[u8])) {
        if t == rack.len() {
            if left == 0 {
                f(cur);
            }
            return;
        }
        for c in 0..=(rack[t] as usize).min(left) {
            cur[t] = c as u8;
            each_sub(rack, t + 1, left - c, cur, f);
        }
        cur[t] = 0;
    }

    #[test]
    #[inline]
    fn best_keep_of_is_the_first_best_leave_in_tally_order() {
        let lat = MultisetLattice::new(4, 4).unwrap();
        let mut leave = vec![0i32; lat.len()];
        for (idx, slot) in leave.iter_mut().enumerate() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            // few values, so that ties are common
            *slot = h.rem_euclid(5) - 2;
        }
        let mut checked = 0;
        for idx in 0..lat.len() {
            let tally = lat.tally(idx);
            let size = tally.iter().map(|&c| c as usize).sum::<usize>();
            for keep in 0..=5 {
                let mut want = (UNPLAYABLE, !0u32);
                let mut cur = vec![0u8; 4];
                each_sub(&tally, 0, keep, &mut cur, &mut |kept| {
                    let kr = lat.rank(kept);
                    if leave[kr as usize] > want.0 {
                        want = (leave[kr as usize], kr);
                    }
                });
                assert_eq!(
                    best_keep_of(&lat, &leave, &tally, keep),
                    want,
                    "tally {tally:?} keep {keep}"
                );
                checked += (keep <= size) as usize;
            }
        }
        assert!(checked > 0);
        assert_eq!(best_keep_of(&lat, &leave, &[1, 1, 1], 1), (UNPLAYABLE, !0));
        assert_eq!(
            best_keep_of(&lat, &leave[1..], &[1, 1, 1, 1], 1),
            (UNPLAYABLE, !0)
        );
    }

    #[test]
    #[inline]
    fn the_walks_fit_a_small_stack() {
        // the walks recurse a frame a letter, over an alphabet the size of
        // English's, so they run on a thread with far less stack than a main
        // thread has
        let lat = MultisetLattice::new(27, 3).unwrap();
        let mut unseen = vec![0u8; 27];
        for (t, slot) in unseen.iter_mut().enumerate() {
            *slot = (t % 4) as u8;
        }
        let mut sheet = vec![0i32; lat.len()];
        let mut leave = vec![0i32; lat.len()];
        for idx in 0..lat.len() {
            let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
            sheet[idx] = h.rem_euclid(20_000);
            leave[idx] = h.rem_euclid(8_000) - 4_000;
        }
        let (full, value) = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(256 << 10)
                .spawn_scoped(s, || {
                    let mut best = vec![UNPLAYABLE; lat.len()];
                    best_equity_table(&lat, &sheet, &leave, &mut best);
                    let mut num = vec![0f64; lat.len()];
                    let mut den = vec![0f64; lat.len()];
                    apportion_table(&lat, &best, &unseen, &mut num, &mut den);
                    let tally = vec![0u8; 27];
                    let value = leave_value_by_draw(&lat, &best, &unseen, &tally);
                    let mut placed = vec![(1u8, 5i32), (2, 7), (3, 9)];
                    record_blank_variants(&lat, &mut sheet, 40_000, &mut placed, &unseen, 1);
                    (best[lat.full_rack_start()], value)
                })
                .unwrap()
                .join()
                .unwrap()
        });
        assert_ne!(full, UNPLAYABLE);
        assert_ne!(value, UNPLAYABLE);
    }

    #[test]
    #[inline]
    fn a_leave_is_worth_the_average_of_the_racks_it_draws_into() {
        let lat = MultisetLattice::new(2, 2).unwrap();
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
    fn dynamic_leave_matches_draw_with_s_added() {
        let lat = MultisetLattice::new(3, 3).unwrap();
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
    fn apportion_table_matches_naive() {
        let lat = MultisetLattice::new(3, 3).unwrap();
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
    fn apportion_fused_matches_split() {
        // each way apportion_fused can go against apportion_table. The add
        // table is by index alone or by letter too. The pool holds every letter
        // or lacks one (whose superset pass is then skipped), and may hold a
        // full rack of the last letter (each size's first multiset). Plays
        // score up to 20,000 or 1,000 against leaves of -4,000 to 4,000, so a
        // rack's best is a play, or often an exchange
        let lat = MultisetLattice::new(4, 4).unwrap();
        for top in [20_000, 1_000] {
            let mut sheet = vec![0i32; lat.len()]; // >= 0, like a built sheet
            let mut leave = vec![0i32; lat.len()];
            for idx in 0..lat.len() {
                let h = (idx as i32).wrapping_mul(2654435761u32 as i32);
                if (h & 3) != 0 {
                    sheet[idx] = h.rem_euclid(top);
                }
                leave[idx] = h.rem_euclid(8_000) - 4_000;
            }
            let mut best = vec![UNPLAYABLE; lat.len()];
            best_equity_table(&lat, &sheet, &leave, &mut best);
            let tables = [
                AddTable::new(&lat),
                AddTable::new(&lat).with_letter_passes(3),
            ];
            let mut leave_max = vec![0i32; lat.len()];
            subset_max_transform(&lat, &tables[0], &leave, &mut leave_max);
            let mut maxsheet = vec![0i32; lat.len()];

            for unseen in [[3u8, 2, 4, 1], [3, 0, 4, 1], [2, 1, 0, 4]] {
                let mut num_a = vec![0f64; lat.len()];
                let mut den_a = vec![0f64; lat.len()];
                apportion_table(&lat, &best, &unseen, &mut num_a, &mut den_a);
                for (by_letter, add) in tables.iter().enumerate() {
                    for scatter in [false, true] {
                        for zeta in [false, true] {
                            let lo = lat.full_rack_start();
                            let mut num_b = vec![0f64; lo];
                            let mut den_b = vec![0f64; lo];
                            let mut ways_b = vec![0f64; lat.len() - lo];
                            // what a board before leaves there, never read back
                            maxsheet.fill(UNPLAYABLE);
                            apportion_fused(
                                &lat,
                                add,
                                &ApportionBoard {
                                    sheet: &sheet,
                                    leave: &leave,
                                    leave_max: &leave_max,
                                    unseen: &unseen,
                                },
                                ApportionOut {
                                    num: &mut num_b,
                                    den: &mut den_b,
                                    ways: &mut ways_b,
                                },
                                &mut maxsheet,
                                ApportionMode {
                                    zeta,
                                    null_leave: false,
                                    scatter,
                                },
                            );
                            let case = (top, unseen, by_letter, zeta, scatter);
                            for idx in 0..lo {
                                assert_eq!(num_a[idx], num_b[idx], "num at {idx} {case:?}");
                                assert_eq!(den_a[idx], den_b[idx], "den at {idx} {case:?}");
                            }
                            // a full rack: its deals, and its best for each
                            for (idx, &ways) in (lo..).zip(&ways_b) {
                                assert_eq!(den_a[idx], ways, "ways at {idx} {case:?}");
                                let num = if ways == 0.0 {
                                    0.0
                                } else {
                                    ways * maxsheet[idx] as f64
                                };
                                assert_eq!(num_a[idx], num, "best at {idx} {case:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[inline]
    fn apportion_fused_null_leave_matches() {
        // as apportion_fused_matches_split, with every leave worth nothing
        let lat = MultisetLattice::new(4, 4).unwrap();
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
        let tables = [
            AddTable::new(&lat),
            AddTable::new(&lat).with_letter_passes(3),
        ];
        let mut leave_max = vec![0i32; lat.len()];
        subset_max_transform(&lat, &tables[0], &leave, &mut leave_max);
        let mut maxsheet = vec![0i32; lat.len()];

        for unseen in [[3u8, 2, 4, 1], [3, 0, 4, 1], [2, 1, 0, 4]] {
            let mut num_a = vec![0f64; lat.len()];
            let mut den_a = vec![0f64; lat.len()];
            apportion_table(&lat, &best, &unseen, &mut num_a, &mut den_a);
            for (by_letter, add) in tables.iter().enumerate() {
                for null_leave in [false, true] {
                    for zeta in [false, true] {
                        let lo = lat.full_rack_start();
                        let mut num_b = vec![0f64; lo];
                        let mut den_b = vec![0f64; lo];
                        let mut ways_b = vec![0f64; lat.len() - lo];
                        // what a board before leaves there, never read back
                        maxsheet.fill(UNPLAYABLE);
                        apportion_fused(
                            &lat,
                            add,
                            &ApportionBoard {
                                sheet: &sheet,
                                leave: &leave,
                                leave_max: &leave_max,
                                unseen: &unseen,
                            },
                            ApportionOut {
                                num: &mut num_b,
                                den: &mut den_b,
                                ways: &mut ways_b,
                            },
                            &mut maxsheet,
                            ApportionMode {
                                zeta,
                                null_leave,
                                scatter: false,
                            },
                        );
                        let case = (unseen, by_letter, zeta, null_leave);
                        for idx in 0..lo {
                            assert_eq!(num_a[idx], num_b[idx], "num at {idx} {case:?}");
                            assert_eq!(den_a[idx], den_b[idx], "den at {idx} {case:?}");
                        }
                        // a full rack: its deals, and its best for each
                        for (idx, &ways) in (lo..).zip(&ways_b) {
                            assert_eq!(den_a[idx], ways, "ways at {idx} {case:?}");
                            let num = if ways == 0.0 {
                                0.0
                            } else {
                                ways * maxsheet[idx] as f64
                            };
                            assert_eq!(num_a[idx], num, "best at {idx} {case:?}");
                        }
                    }
                }
            }
        }
    }

    // record_blank_variants as plainly as it can be put: each count of blanks a
    // letter's placed tiles can take, its cheapest tiles blanked first, the
    // whole tally ranked
    fn record_blank_variants_by_rank(
        lat: &MultisetLattice,
        sheet: &mut [i32],
        real_score: i32,
        placed: &[(u8, i32)],
        unseen: &[u8],
        num_blanks_eff: usize,
    ) {
        let rack_size = lat.rack_size();
        if placed.is_empty() || placed.len() > rack_size {
            return;
        }
        let mut sorted = placed.to_vec();
        sorted.sort_unstable();
        let runs = sorted
            .chunk_by(|a, b| a.0 == b.0)
            .map(|run| {
                let real = (unseen[run[0].0 as usize] as usize).min(rack_size);
                (run[0].0, run.len().saturating_sub(real), run)
            })
            .collect::<Vec<_>>();
        let mut blanks = vec![0usize; runs.len()];
        loop {
            if (0..runs.len()).all(|i| blanks[i] >= runs[i].1)
                && blanks.iter().sum::<usize>() <= num_blanks_eff
            {
                let mut tally = vec![0u8; lat.num_letters()];
                tally[0] = blanks.iter().sum::<usize>() as u8;
                let mut val = real_score;
                for (i, &(letter, _, run)) in runs.iter().enumerate() {
                    tally[letter as usize] = (run.len() - blanks[i]) as u8;
                    val -= run[..blanks[i]].iter().map(|e| e.1).sum::<i32>();
                }
                let slot = &mut sheet[lat.rank(&tally) as usize];
                *slot = (*slot).max(val);
            }
            // the next count of blanks, as an odometer over the runs
            let mut i = 0;
            while i < runs.len() && blanks[i] == runs[i].2.len() {
                blanks[i] = 0;
                i += 1;
            }
            if i == runs.len() {
                return;
            }
            blanks[i] += 1;
        }
    }

    #[test]
    #[inline]
    fn record_blank_variants_matches_ranking_each_tally() {
        use rand::prelude::*;
        let lat = MultisetLattice::new(6, 7).unwrap();
        let mut fast = vec![0i32; lat.len()];
        let mut plain = vec![0i32; lat.len()];
        let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0);
        for _ in 0..3000 {
            let unseen = (0..6).map(|_| rng.random_range(0..5u8)).collect::<Vec<_>>();
            let num_blanks_eff = rng.random_range(0..4usize);
            let real_score = rng.random_range(0..200i32);
            let mut placed = (0..rng.random_range(1..=7usize))
                .map(|_| (rng.random_range(1..=5u8), rng.random_range(0..30i32)))
                .collect::<Vec<_>>();
            record_blank_variants_by_rank(
                &lat,
                &mut plain,
                real_score,
                &placed,
                &unseen,
                num_blanks_eff,
            );
            record_blank_variants(
                &lat,
                &mut fast,
                real_score,
                &mut placed,
                &unseen,
                num_blanks_eff,
            );
        }
        let valued = plain.iter().filter(|&&v| v != 0).count();
        assert!(valued > lat.len() / 2, "{valued} of {}", lat.len());
        assert_eq!(fast, plain);
    }

    #[test]
    #[inline]
    fn record_blank_variants_enumerates_designations() {
        let lat = MultisetLattice::new(4, 4).unwrap();
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

    #[test]
    fn a_lattice_too_large_to_count_or_hold_is_refused() {
        let mut rack_size = 1;
        let lat = loop {
            match MultisetLattice::new(64, rack_size) {
                Ok(lat) if lat.len() > isize::MAX as usize / std::mem::size_of::<u64>() => {
                    break lat;
                }
                Ok(_) => rack_size += 1,
                Err(e) => panic!("refused at rack {rack_size} before too large to hold: {e}"),
            }
        };
        assert!(lat.dense(0u64).is_err());
        assert!(MultisetLattice::new(64, rack_size + 64).is_err());
        let small = MultisetLattice::new(27, 7).unwrap();
        assert_eq!(small.dense(0i32).unwrap().len(), small.len());
    }
}
