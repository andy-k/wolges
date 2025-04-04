// Copyright (C) 2020-2025 Andy Kurnia.

use wolges::{alphabet, bites, error, fash, prob, stats};

use std::fmt::Write;
use std::str::FromStr;

trait WgReader {
    fn tile(&self, bytes: &[u8], idx: usize) -> u8;
    fn accepts(&self, bytes: &[u8], idx: usize) -> bool;
    fn is_end(&self, bytes: &[u8], idx: usize) -> bool;
    fn arc_index(&self, bytes: &[u8], idx: usize) -> usize;
    fn len(&self, bytes: &[u8]) -> usize;
}

struct KwgReader {}

impl WgReader for KwgReader {
    #[inline(always)]
    fn tile(&self, bytes: &[u8], idx: usize) -> u8 {
        bytes[(idx * 4) + 3]
    }

    #[inline(always)]
    fn accepts(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 4) + 2] & 0x80 != 0
    }

    #[inline(always)]
    fn is_end(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 4) + 2] & 0x40 != 0
    }

    #[inline(always)]
    fn arc_index(&self, bytes: &[u8], idx: usize) -> usize {
        (((bytes[(idx * 4) + 2] & 0x3f) as usize) << 16)
            | ((bytes[(idx * 4) + 1] as usize) << 8)
            | (bytes[idx * 4] as usize)
    }

    #[inline(always)]
    fn len(&self, bytes: &[u8]) -> usize {
        bytes.len() / 4
    }
}

struct LexpertReader {}

impl WgReader for LexpertReader {
    #[inline(always)]
    fn tile(&self, bytes: &[u8], idx: usize) -> u8 {
        (bytes[(idx * 4) + 0x43] << 2) | (bytes[(idx * 4) + 0x42] >> 6)
    }

    #[inline(always)]
    fn accepts(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 4) + 0x43] & 0x40 != 0
    }

    #[inline(always)]
    fn is_end(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 4) + 0x43] & 0x80 != 0
    }

    #[inline(always)]
    fn arc_index(&self, bytes: &[u8], idx: usize) -> usize {
        (((bytes[(idx * 4) + 0x42] & 0x3f) as usize) << 16)
            | ((bytes[(idx * 4) + 0x41] as usize) << 8)
            | (bytes[(idx * 4) + 0x40] as usize)
    }

    #[inline(always)]
    fn len(&self, bytes: &[u8]) -> usize {
        (bytes.len() - 0x40) / 4
    }
}

struct ZyzzyvaReader {}

impl WgReader for ZyzzyvaReader {
    #[inline(always)]
    fn tile(&self, bytes: &[u8], idx: usize) -> u8 {
        bytes[(idx * 4) + 3]
    }

    #[inline(always)]
    fn accepts(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 4) + 2] & 0x80 != 0
    }

    #[inline(always)]
    fn is_end(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 4) + 2] & 0x40 != 0
    }

    #[inline(always)]
    fn arc_index(&self, bytes: &[u8], idx: usize) -> usize {
        (((bytes[(idx * 4) + 2] & 0x3f) as usize) << 16)
            | ((bytes[(idx * 4) + 1] as usize) << 8)
            | (bytes[idx * 4] as usize)
    }

    #[inline(always)]
    fn len(&self, bytes: &[u8]) -> usize {
        bytes.len() / 4
    }
}

struct QuackleReader {
    offset: usize,
}

impl WgReader for QuackleReader {
    #[inline(always)]
    fn tile(&self, bytes: &[u8], idx: usize) -> u8 {
        (bytes[(idx * 7 + self.offset) + 3] & 0x3f) + 1
    }

    #[inline(always)]
    fn accepts(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 7 + self.offset) + 6] != 0
    }

    #[inline(always)]
    fn is_end(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 7 + self.offset) + 3] & 0x40 != 0
    }

    #[inline(always)]
    fn arc_index(&self, bytes: &[u8], idx: usize) -> usize {
        ((bytes[idx * 7 + self.offset] as usize) << 16)
            | ((bytes[(idx * 7 + self.offset) + 1] as usize) << 8)
            | (bytes[(idx * 7 + self.offset) + 2] as usize)
    }

    #[inline(always)]
    fn len(&self, bytes: &[u8]) -> usize {
        (bytes.len() - self.offset) / 7
    }
}

struct QuackleSmallReader {
    offset: usize,
}

impl WgReader for QuackleSmallReader {
    #[inline(always)]
    fn tile(&self, bytes: &[u8], idx: usize) -> u8 {
        (bytes[(idx * 7 + self.offset) + 3] & 0x3f) + 1
    }

    #[inline(always)]
    fn accepts(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 7 + self.offset) + 3] & 0x80 != 0
    }

    #[inline(always)]
    fn is_end(&self, bytes: &[u8], idx: usize) -> bool {
        bytes[(idx * 7 + self.offset) + 3] & 0x40 != 0
    }

    #[inline(always)]
    fn arc_index(&self, bytes: &[u8], idx: usize) -> usize {
        ((bytes[idx * 7 + self.offset] as usize) << 16)
            | ((bytes[(idx * 7 + self.offset) + 1] as usize) << 8)
            | (bytes[(idx * 7 + self.offset) + 2] as usize)
    }

    #[inline(always)]
    fn len(&self, bytes: &[u8]) -> usize {
        (bytes.len() - self.offset) / 7
    }
}

trait AlphabetLabel {
    fn label(&self, s: &mut String, tile: u8) -> error::Returns<()>;

    #[inline(always)]
    fn is_verbatim() -> bool {
        false
    }
}

struct WolgesAlphabetLabel<'a> {
    alphabet: &'a alphabet::Alphabet,
}

impl AlphabetLabel for WolgesAlphabetLabel<'_> {
    #[inline(always)]
    fn label(&self, s: &mut String, tile: u8) -> error::Returns<()> {
        s.push_str(self.alphabet.of_board(tile).ok_or("invalid tile")?);
        Ok(())
    }
}

struct WolgesAlphabetLabelAllowBlank<'a> {
    alphabet: &'a alphabet::Alphabet,
}

impl AlphabetLabel for WolgesAlphabetLabelAllowBlank<'_> {
    #[inline(always)]
    fn label(&self, s: &mut String, tile: u8) -> error::Returns<()> {
        s.push_str(self.alphabet.of_rack(tile).ok_or("invalid tile")?);
        Ok(())
    }
}

struct LexpertAlphabetLabel {}

impl AlphabetLabel for LexpertAlphabetLabel {
    #[inline(always)]
    fn label(&self, s: &mut String, tile: u8) -> error::Returns<()> {
        s.push(tile as char);
        Ok(())
    }

    #[inline(always)]
    fn is_verbatim() -> bool {
        true
    }
}

struct QuackleAlphabetLabel<'a> {
    alpha: &'a [&'a str],
}

impl AlphabetLabel for QuackleAlphabetLabel<'_> {
    #[inline(always)]
    fn label(&self, s: &mut String, tile: u8) -> error::Returns<()> {
        s.push_str(self.alpha.get((tile - 1) as usize).ok_or("invalid tile")?);
        Ok(())
    }
}

struct QuackleLeavesAlphabetLabel {}

impl AlphabetLabel for QuackleLeavesAlphabetLabel {
    #[inline(always)]
    fn label(&self, s: &mut String, tile: u8) -> error::Returns<()> {
        if tile == 0 {
            s.push(1 as char);
        } else if (1..=26).contains(&tile) {
            s.push((tile + 4) as char);
        } else {
            return Err("invalid tile".into());
        }
        Ok(())
    }
}

#[expect(clippy::too_many_arguments)]
fn iter_dawg<
    F: FnMut(&str) -> error::Returns<()>,
    In: FnMut(u8) -> error::Returns<Option<u8>>,
    Out: FnMut(u8) -> error::Returns<()>,
    A: AlphabetLabel,
    R: WgReader,
>(
    a: &A,
    r: &R,
    b: &[u8],
    initial_idx: usize,
    blank_str: Option<&str>,
    accepts: &mut F,
    on_in: &mut In,
    on_out: &mut Out,
) -> error::Returns<()> {
    struct Env<
        'a,
        F: FnMut(&str) -> error::Returns<()>,
        In: FnMut(u8) -> error::Returns<Option<u8>>,
        Out: FnMut(u8) -> error::Returns<()>,
        A: AlphabetLabel,
        R: WgReader,
    > {
        a: &'a A,
        r: &'a R,
        b: &'a [u8],
        s: &'a mut String,
        blank_str: Option<&'a str>,
        accepts: &'a mut F,
        on_in: &'a mut In,
        on_out: &'a mut Out,
    }
    fn iter<
        F: FnMut(&str) -> error::Returns<()>,
        In: FnMut(u8) -> error::Returns<Option<u8>>,
        Out: FnMut(u8) -> error::Returns<()>,
        A: AlphabetLabel,
        R: WgReader,
    >(
        env: &mut Env<'_, F, In, Out, A, R>,
        mut p: usize,
    ) -> error::Returns<()> {
        let l = env.s.len();
        loop {
            if p >= env.r.len(env.b) {
                return Err("out of bounds".into());
            }
            let t = env.r.tile(env.b, p);
            if A::is_verbatim() {
                env.a.label(env.s, t)?;
            } else if t == 0 {
                env.s.push_str(env.blank_str.ok_or("invalid tile")?);
            } else if t & 0x80 == 0 {
                env.a.label(env.s, t)?;
            } else {
                return Err("invalid tile".into());
            }
            if let Some(b) = (env.on_in)(t)? {
                if env.r.accepts(env.b, p) {
                    (env.accepts)(env.s)?;
                }
                if env.r.arc_index(env.b, p) != 0 {
                    iter(env, env.r.arc_index(env.b, p))?;
                }
                (env.on_out)(b)?;
            }
            env.s.truncate(l);
            if env.r.is_end(env.b, p) {
                break;
            }
            p += 1;
        }
        Ok(())
    }
    if initial_idx >= r.len(b) {
        return Err("out of bounds".into());
    }
    iter(
        &mut Env {
            a,
            r,
            b,
            s: &mut String::new(),
            blank_str,
            accepts,
            on_in,
            on_out,
        },
        initial_idx,
    )
}

static USED_STDOUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// support "-" to mean stdout.
fn use_writer(filename: &str) {
    if filename == "-" {
        USED_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

// support "-" to mean stdout.
fn make_writer(filename: &str) -> Result<Box<dyn std::io::Write>, std::io::Error> {
    Ok(if filename == "-" {
        USED_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(filename)?)
    })
}

// when using "-" as output filename, print things to stderr.
fn boxed_stdout_or_stderr() -> Box<dyn std::io::Write> {
    if USED_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
        Box::new(std::io::stderr()) as Box<dyn std::io::Write>
    } else {
        Box::new(std::io::stdout())
    }
}

// support "-" to mean stdin.
fn make_reader(filename: &str) -> Result<Box<dyn std::io::Read>, std::io::Error> {
    Ok(if filename == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(filename)?)
    })
}

// slower than std::fs::read because it cannot preallocate the correct size.
fn read_to_end(reader: &mut Box<dyn std::io::Read>) -> Result<Vec<u8>, std::io::Error> {
    let mut v = Vec::new();
    reader.read_to_end(&mut v)?;
    Ok(v)
}

// adjusted from main_build read_machine_words.
fn read_machine_words_sorted_by_length(
    alphabet_reader: &alphabet::AlphabetReader,
    giant_string: &str,
) -> error::Returns<Box<[bites::Bites]>> {
    let mut machine_words = Vec::<bites::Bites>::new();
    let mut v = Vec::new();
    for s in giant_string.lines() {
        if s.is_empty() {
            continue;
        }
        let sb = s.as_bytes();
        v.clear();
        let mut ix = 0;
        while ix < sb.len() {
            if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                v.push(tile);
                ix = end_ix;
            } else if ix > 0 && *unsafe { sb.get_unchecked(ix) } <= b' ' {
                // Safe because already checked length.
                break;
            } else {
                wolges::return_error!(format!("invalid tile after {v:?} in {s:?}"));
            }
        }
        machine_words.push(v[..].into());
    }
    machine_words.sort_unstable_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    machine_words.dedup();
    Ok(machine_words.into_boxed_slice())
}

// copied from main_build.
// slower than std::fs::read_to_string because it cannot preallocate the correct size.
fn read_to_string(reader: &mut Box<dyn std::io::Read>) -> Result<String, std::io::Error> {
    let mut s = String::new();
    reader.read_to_string(&mut s)?;
    Ok(s)
}

#[inline(always)]
fn default_in(_b: u8) -> error::Returns<Option<u8>> {
    Ok(Some(_b))
}

#[inline(always)]
fn default_out(_b: u8) -> error::Returns<()> {
    Ok(())
}

fn do_lang<AlphabetMaker: Fn() -> alphabet::Alphabet>(
    args: &[String],
    language_name: &str,
    make_alphabet: AlphabetMaker,
) -> error::Returns<bool> {
    match args[1].strip_prefix(language_name) {
        Some(args1_suffix) => match args1_suffix {
            "-klv" => {
                let alphabet = make_alphabet();
                let reader = &KwgReader {};
                let klv_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if klv_bytes.len() < 4 {
                    return Err("out of bounds".into());
                }
                let mut r = 0;
                let kwg_bytes_len = ((klv_bytes[r] as u32
                    | ((klv_bytes[r + 1] as u32) << 8)
                    | ((klv_bytes[r + 2] as u32) << 16)
                    | ((klv_bytes[r + 3] as u32) << 24))
                    as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = (klv_bytes[r] as u32
                    | ((klv_bytes[r + 1] as u32) << 8)
                    | ((klv_bytes[r + 2] as u32) << 16)
                    | ((klv_bytes[r + 3] as u32) << 24)) as usize;
                r += 4;
                let is_klv2 = klv_bytes.len() >= r + lv_len * 4;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut csv_out = csv::Writer::from_writer(make_writer(&args[3])?);
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    alphabet.of_rack(0),
                    &mut |s: &str| {
                        csv_out.serialize((
                            s,
                            if is_klv2 && klv_bytes.len() >= r + 4 {
                                r += 4;
                                f32::from_bits(
                                    klv_bytes[r - 4] as u32
                                        | ((klv_bytes[r - 3] as u32) << 8)
                                        | ((klv_bytes[r - 2] as u32) << 16)
                                        | ((klv_bytes[r - 1] as u32) << 24),
                                )
                            } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                                r += 2;
                                ((klv_bytes[r - 2] as u16 | ((klv_bytes[r - 1] as u16) << 8))
                                    as i16) as f32
                                    * (1.0 / 256.0)
                            } else {
                                return Err("missing leaves".into());
                            },
                        ))?;
                        Ok(())
                    },
                    &mut default_in,
                    &mut default_out,
                )?;
                if r != klv_bytes.len() {
                    return Err("too many leaves".into());
                }
                Ok(true)
            }
            "-kwg" => {
                let alphabet = make_alphabet();
                let reader = &KwgReader {};
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut ret = String::new();
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    None,
                    &mut |s: &str| {
                        ret.push_str(s);
                        ret.push('\n');
                        Ok(())
                    },
                    &mut default_in,
                    &mut default_out,
                )?;
                make_writer(&args[3])?.write_all(ret.as_bytes())?;
                Ok(true)
            }
            "-kwg0" => {
                let alphabet = make_alphabet();
                let reader = &KwgReader {};
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut ret = String::new();
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    alphabet.of_rack(0),
                    &mut |s: &str| {
                        ret.push_str(s);
                        ret.push('\n');
                        Ok(())
                    },
                    &mut default_in,
                    &mut default_out,
                )?;
                make_writer(&args[3])?.write_all(ret.as_bytes())?;
                Ok(true)
            }
            "-kwg-gaddag" => {
                let alphabet = make_alphabet();
                let reader = &KwgReader {};
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if 1 >= reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut ret = String::new();
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 1),
                    Some("@"),
                    &mut |s: &str| {
                        ret.push_str(s);
                        ret.push('\n');
                        Ok(())
                    },
                    &mut default_in,
                    &mut default_out,
                )?;
                make_writer(&args[3])?.write_all(ret.as_bytes())?;
                Ok(true)
            }
            "-kwg-nodes" => {
                // output format not guaranteed to be stable.
                let alphabet = make_alphabet();
                let alphabet_label = &WolgesAlphabetLabel {
                    alphabet: &alphabet,
                };
                let reader = &KwgReader {};
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                let kwg_len = reader.len(kwg_bytes);
                let kwg_len_width = format!("{}", kwg_len.saturating_sub(1)).len();
                let mut kwg_pointed_to = vec![false; kwg_len];
                for p in 0..kwg_len {
                    kwg_pointed_to[reader.arc_index(kwg_bytes, p)] = true;
                }
                let mut ret = String::new();
                for (p, &p_pointed_to) in kwg_pointed_to.iter().enumerate().take(kwg_len) {
                    if p_pointed_to {
                        write!(ret, "{p:kwg_len_width$}")?;
                    } else {
                        write!(ret, "{:kwg_len_width$}", "")?;
                    }
                    ret.push(' ');
                    let t = reader.tile(kwg_bytes, p);
                    if t == 0 {
                        ret.push('@');
                    } else {
                        alphabet_label.label(&mut ret, t)?;
                    }
                    if reader.accepts(kwg_bytes, p) {
                        ret.push('*');
                    }
                    let arc_index = reader.arc_index(kwg_bytes, p);
                    if arc_index != 0 {
                        write!(ret, " {arc_index}")?;
                    }
                    if reader.is_end(kwg_bytes, p) {
                        ret.push_str(" ends");
                    }
                    ret.push('\n');
                }
                make_writer(&args[3])?.write_all(ret.as_bytes())?;
                Ok(true)
            }
            "-kwg-prob" => {
                // output format not guaranteed to be stable.
                let alphabet = make_alphabet();
                let reader = &KwgReader {};
                let mut word_prob = prob::WordProbability::new(&alphabet);
                let word_cell = std::cell::RefCell::new(Vec::new());
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let vec_out_cell = std::cell::RefCell::new(Vec::new());
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    None,
                    &mut |s: &str| {
                        let word = word_cell.borrow();
                        let this_wp = word_prob.count_ways(&word);
                        let mut vec_out = vec_out_cell.borrow_mut();
                        let vec_len = vec_out.len();
                        let mut anagram_key = word.clone();
                        anagram_key.sort_unstable();
                        vec_out
                            .push(((s.to_string(), word.len(), this_wp), (anagram_key, vec_len)));
                        Ok(())
                    },
                    &mut |b: u8| {
                        word_cell.borrow_mut().push(b);
                        Ok(Some(b))
                    },
                    &mut |_b: u8| {
                        word_cell.borrow_mut().pop();
                        Ok(())
                    },
                )?;
                let mut vec_out = vec_out_cell.into_inner();
                vec_out.sort_unstable_by(|a, b| {
                    a.0.1.cmp(&b.0.1).then_with(|| {
                        b.0.2
                            .cmp(&a.0.2)
                            .then_with(|| a.1.0.cmp(&b.1.0).then_with(|| a.1.1.cmp(&b.1.1)))
                    })
                });
                let mut csv_out = csv::Writer::from_writer(make_writer(&args[3])?);
                let mut last_anagram_key = Vec::new();
                let mut num_sets = 0;
                let mut num_in_set = 0;
                for elt in vec_out {
                    if last_anagram_key != elt.1.0 {
                        if last_anagram_key.len() != elt.1.0.len() {
                            num_sets = 1;
                        } else {
                            num_sets += 1;
                        }
                        num_in_set = 0;
                        last_anagram_key.clone_from(&elt.1.0);
                    }
                    num_in_set += 1;
                    csv_out.serialize((elt.0, num_sets, num_in_set))?;
                }
                Ok(true)
            }
            "-prob" => {
                if args.len() < 3 {
                    return Err("need more argument".into());
                }
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_words(&alphabet);
                let mut word_prob = prob::WordProbability::new(&alphabet);
                let mut v = Vec::new();
                for word in &args[2..] {
                    v.clear();
                    let sb = &word.as_bytes();
                    let mut ix = 0;
                    while ix < sb.len() {
                        if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                            v.push(tile);
                            ix = end_ix;
                        } else {
                            return Err("invalid tile".into());
                        }
                    }
                    println!("{}", word_prob.count_ways(&v));
                }
                Ok(true)
            }
            "-klv-anagram-" => {
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_racks(&alphabet);
                let reader = &KwgReader {};
                let klv_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if klv_bytes.len() < 4 {
                    return Err("out of bounds".into());
                }
                let mut r = 0;
                let kwg_bytes_len = ((klv_bytes[r] as u32
                    | ((klv_bytes[r + 1] as u32) << 8)
                    | ((klv_bytes[r + 2] as u32) << 16)
                    | ((klv_bytes[r + 3] as u32) << 24))
                    as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = (klv_bytes[r] as u32
                    | ((klv_bytes[r + 1] as u32) << 8)
                    | ((klv_bytes[r + 2] as u32) << 16)
                    | ((klv_bytes[r + 3] as u32) << 24)) as usize;
                r += 4;
                let is_klv2 = klv_bytes.len() >= r + lv_len * 4;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut rack = vec![0; alphabet.len().into()];
                let sb = &args[4].as_bytes();
                let mut ix = 0;
                while ix < sb.len() {
                    if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                        rack[tile as usize] += 1;
                        ix = end_ix;
                    } else {
                        return Err("invalid tile".into());
                    }
                }
                let rack_cell = std::cell::RefCell::new(rack);
                let num_unspecified = std::sync::atomic::AtomicUsize::new(0);
                let mut csv_out = csv::Writer::from_writer(make_writer(&args[3])?);
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    alphabet.of_rack(0),
                    &mut |s: &str| {
                        let leave_value = if is_klv2 && klv_bytes.len() >= r + 4 {
                            r += 4;
                            f32::from_bits(
                                klv_bytes[r - 4] as u32
                                    | ((klv_bytes[r - 3] as u32) << 8)
                                    | ((klv_bytes[r - 2] as u32) << 16)
                                    | ((klv_bytes[r - 1] as u32) << 24),
                            )
                        } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                            r += 2;
                            ((klv_bytes[r - 2] as u16 | ((klv_bytes[r - 1] as u16) << 8)) as i16)
                                as f32
                                * (1.0 / 256.0)
                        } else {
                            return Err("missing leaves".into());
                        };
                        if num_unspecified.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                            csv_out.serialize((s, leave_value))?;
                        }
                        Ok(())
                    },
                    &mut |b: u8| {
                        let mut rack = rack_cell.borrow_mut();
                        if rack[b as usize] > 0 {
                            rack[b as usize] -= 1;
                            Ok(Some(b))
                        } else {
                            num_unspecified.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(0xff))
                        }
                    },
                    &mut |b: u8| {
                        if b != 0xff {
                            let mut rack = rack_cell.borrow_mut();
                            rack[b as usize] += 1;
                        } else {
                            num_unspecified.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Ok(())
                    },
                )?;
                if r != klv_bytes.len() {
                    return Err("too many leaves".into());
                }
                Ok(true)
            }
            "-klv-anagram" => {
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_racks(&alphabet);
                let reader = &KwgReader {};
                let klv_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if klv_bytes.len() < 4 {
                    return Err("out of bounds".into());
                }
                let mut r = 0;
                let kwg_bytes_len = ((klv_bytes[r] as u32
                    | ((klv_bytes[r + 1] as u32) << 8)
                    | ((klv_bytes[r + 2] as u32) << 16)
                    | ((klv_bytes[r + 3] as u32) << 24))
                    as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = (klv_bytes[r] as u32
                    | ((klv_bytes[r + 1] as u32) << 8)
                    | ((klv_bytes[r + 2] as u32) << 16)
                    | ((klv_bytes[r + 3] as u32) << 24)) as usize;
                r += 4;
                let is_klv2 = klv_bytes.len() >= r + lv_len * 4;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut rack = vec![0; alphabet.len().into()];
                let mut given_num_tiles = 0usize;
                let sb = &args[4].as_bytes();
                let mut ix = 0;
                while ix < sb.len() {
                    if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                        rack[tile as usize] += 1;
                        given_num_tiles += 1;
                        ix = end_ix;
                    } else {
                        return Err("invalid tile".into());
                    }
                }
                let rack_cell = std::cell::RefCell::new(rack);
                let num_tiles = std::sync::atomic::AtomicUsize::new(0);
                let num_unspecified = std::sync::atomic::AtomicUsize::new(0);
                let mut csv_out = csv::Writer::from_writer(make_writer(&args[3])?);
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    alphabet.of_rack(0),
                    &mut |s: &str| {
                        let leave_value = if is_klv2 && klv_bytes.len() >= r + 4 {
                            r += 4;
                            f32::from_bits(
                                klv_bytes[r - 4] as u32
                                    | ((klv_bytes[r - 3] as u32) << 8)
                                    | ((klv_bytes[r - 2] as u32) << 16)
                                    | ((klv_bytes[r - 1] as u32) << 24),
                            )
                        } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                            r += 2;
                            ((klv_bytes[r - 2] as u16 | ((klv_bytes[r - 1] as u16) << 8)) as i16)
                                as f32
                                * (1.0 / 256.0)
                        } else {
                            return Err("missing leaves".into());
                        };
                        if num_tiles.load(std::sync::atomic::Ordering::Relaxed) == given_num_tiles
                            && num_unspecified.load(std::sync::atomic::Ordering::Relaxed) == 0
                        {
                            csv_out.serialize((s, leave_value))?;
                        }
                        Ok(())
                    },
                    &mut |b: u8| {
                        let mut rack = rack_cell.borrow_mut();
                        if rack[b as usize] > 0 {
                            rack[b as usize] -= 1;
                            num_tiles.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(b))
                        } else {
                            num_unspecified.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(0xff))
                        }
                    },
                    &mut |b: u8| {
                        if b != 0xff {
                            let mut rack = rack_cell.borrow_mut();
                            rack[b as usize] += 1;
                            num_tiles.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        } else {
                            num_unspecified.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Ok(())
                    },
                )?;
                if r != klv_bytes.len() {
                    return Err("too many leaves".into());
                }
                Ok(true)
            }
            "-klv-anagram+" => {
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_racks(&alphabet);
                let reader = &KwgReader {};
                let klv_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if klv_bytes.len() < 4 {
                    return Err("out of bounds".into());
                }
                let mut r = 0;
                let kwg_bytes_len = ((klv_bytes[r] as u32
                    | ((klv_bytes[r + 1] as u32) << 8)
                    | ((klv_bytes[r + 2] as u32) << 16)
                    | ((klv_bytes[r + 3] as u32) << 24))
                    as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = (klv_bytes[r] as u32
                    | ((klv_bytes[r + 1] as u32) << 8)
                    | ((klv_bytes[r + 2] as u32) << 16)
                    | ((klv_bytes[r + 3] as u32) << 24)) as usize;
                r += 4;
                let is_klv2 = klv_bytes.len() >= r + lv_len * 4;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut rack = vec![0; alphabet.len().into()];
                let mut given_num_tiles = 0usize;
                let sb = &args[4].as_bytes();
                let mut ix = 0;
                while ix < sb.len() {
                    if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                        rack[tile as usize] += 1;
                        given_num_tiles += 1;
                        ix = end_ix;
                    } else {
                        return Err("invalid tile".into());
                    }
                }
                let rack_cell = std::cell::RefCell::new(rack);
                let num_tiles = std::sync::atomic::AtomicUsize::new(0);
                let mut csv_out = csv::Writer::from_writer(make_writer(&args[3])?);
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    alphabet.of_rack(0),
                    &mut |s: &str| {
                        let leave_value = if is_klv2 && klv_bytes.len() >= r + 4 {
                            r += 4;
                            f32::from_bits(
                                klv_bytes[r - 4] as u32
                                    | ((klv_bytes[r - 3] as u32) << 8)
                                    | ((klv_bytes[r - 2] as u32) << 16)
                                    | ((klv_bytes[r - 1] as u32) << 24),
                            )
                        } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                            r += 2;
                            ((klv_bytes[r - 2] as u16 | ((klv_bytes[r - 1] as u16) << 8)) as i16)
                                as f32
                                * (1.0 / 256.0)
                        } else {
                            return Err("missing leaves".into());
                        };
                        if num_tiles.load(std::sync::atomic::Ordering::Relaxed) == given_num_tiles {
                            csv_out.serialize((s, leave_value))?;
                        }
                        Ok(())
                    },
                    &mut |b: u8| {
                        let mut rack = rack_cell.borrow_mut();
                        if rack[b as usize] > 0 {
                            rack[b as usize] -= 1;
                            num_tiles.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(b))
                        } else {
                            Ok(Some(0xff))
                        }
                    },
                    &mut |b: u8| {
                        if b != 0xff {
                            let mut rack = rack_cell.borrow_mut();
                            rack[b as usize] += 1;
                            num_tiles.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Ok(())
                    },
                )?;
                if r != klv_bytes.len() {
                    return Err("too many leaves".into());
                }
                Ok(true)
            }
            "-kwg-anagram-" => {
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_racks(&alphabet);
                let reader = &KwgReader {};
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut rack = vec![0; alphabet.len().into()];
                let sb = &args[4].as_bytes();
                let mut ix = 0;
                while ix < sb.len() {
                    if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                        rack[tile as usize] += 1;
                        ix = end_ix;
                    } else {
                        return Err("invalid tile".into());
                    }
                }
                let rack_cell = std::cell::RefCell::new(rack);
                let mut ret = String::new();
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    None,
                    &mut |s: &str| {
                        ret.push_str(s);
                        ret.push('\n');
                        Ok(())
                    },
                    &mut |b: u8| {
                        let mut rack = rack_cell.borrow_mut();
                        if rack[b as usize] > 0 {
                            rack[b as usize] -= 1;
                            Ok(Some(b))
                        } else if rack[0] > 0 {
                            rack[0] -= 1;
                            Ok(Some(0))
                        } else {
                            Ok(None)
                        }
                    },
                    &mut |b: u8| {
                        let mut rack = rack_cell.borrow_mut();
                        rack[b as usize] += 1;
                        Ok(())
                    },
                )?;
                make_writer(&args[3])?.write_all(ret.as_bytes())?;
                Ok(true)
            }
            "-kwg-anagram" => {
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_racks(&alphabet);
                let reader = &KwgReader {};
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut rack = vec![0; alphabet.len().into()];
                let mut given_num_tiles = 0usize;
                let sb = &args[4].as_bytes();
                let mut ix = 0;
                while ix < sb.len() {
                    if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                        rack[tile as usize] += 1;
                        given_num_tiles += 1;
                        ix = end_ix;
                    } else {
                        return Err("invalid tile".into());
                    }
                }
                let rack_cell = std::cell::RefCell::new(rack);
                let num_tiles = std::sync::atomic::AtomicUsize::new(0);
                let mut ret = String::new();
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    None,
                    &mut |s: &str| {
                        if num_tiles.load(std::sync::atomic::Ordering::Relaxed) == given_num_tiles {
                            ret.push_str(s);
                            ret.push('\n');
                        }
                        Ok(())
                    },
                    &mut |b: u8| {
                        let mut rack = rack_cell.borrow_mut();
                        if rack[b as usize] > 0 {
                            rack[b as usize] -= 1;
                            num_tiles.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(b))
                        } else if rack[0] > 0 {
                            rack[0] -= 1;
                            num_tiles.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(0))
                        } else {
                            Ok(None)
                        }
                    },
                    &mut |b: u8| {
                        let mut rack = rack_cell.borrow_mut();
                        rack[b as usize] += 1;
                        num_tiles.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        Ok(())
                    },
                )?;
                make_writer(&args[3])?.write_all(ret.as_bytes())?;
                Ok(true)
            }
            "-kwg-anagram+" => {
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_racks(&alphabet);
                let reader = &KwgReader {};
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut rack = vec![0; alphabet.len().into()];
                let mut given_num_tiles = 0usize;
                let sb = &args[4].as_bytes();
                let mut ix = 0;
                while ix < sb.len() {
                    if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                        rack[tile as usize] += 1;
                        given_num_tiles += 1;
                        ix = end_ix;
                    } else {
                        return Err("invalid tile".into());
                    }
                }
                let rack_cell = std::cell::RefCell::new(rack);
                let num_tiles = std::sync::atomic::AtomicUsize::new(0);
                let mut ret = String::new();
                iter_dawg(
                    &WolgesAlphabetLabel {
                        alphabet: &alphabet,
                    },
                    reader,
                    kwg_bytes,
                    reader.arc_index(kwg_bytes, 0),
                    None,
                    &mut |s: &str| {
                        if num_tiles.load(std::sync::atomic::Ordering::Relaxed) == given_num_tiles {
                            ret.push_str(s);
                            ret.push('\n');
                        }
                        Ok(())
                    },
                    &mut |b: u8| {
                        let mut rack = rack_cell.borrow_mut();
                        if rack[b as usize] > 0 {
                            rack[b as usize] -= 1;
                            num_tiles.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(b))
                        } else if rack[0] > 0 {
                            rack[0] -= 1;
                            num_tiles.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(Some(0))
                        } else {
                            Ok(Some(0xff))
                        }
                    },
                    &mut |b: u8| {
                        if b != 0xff {
                            let mut rack = rack_cell.borrow_mut();
                            rack[b as usize] += 1;
                            num_tiles.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Ok(())
                    },
                )?;
                make_writer(&args[3])?.write_all(ret.as_bytes())?;
                Ok(true)
            }
            "-kwg-check" => {
                if args.len() < 4 {
                    return Err("need more argument".into());
                }
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_words(&alphabet);
                let reader = &KwgReader {};
                let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if 0 == reader.len(kwg_bytes) {
                    return Err("out of bounds".into());
                }
                let mut not_found = false;
                for word in &args[3..] {
                    let sb = &word.as_bytes();
                    let mut p = 0;
                    let mut ix = 0;
                    while ix < sb.len() {
                        if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                            if !not_found {
                                p = reader.arc_index(kwg_bytes, p);
                                if p > 0 {
                                    loop {
                                        if reader.tile(kwg_bytes, p) == tile {
                                            break;
                                        }
                                        if reader.is_end(kwg_bytes, p) {
                                            not_found = true;
                                            break;
                                        }
                                        p += 1;
                                    }
                                } else {
                                    not_found = true;
                                }
                            }
                            ix = end_ix;
                        } else {
                            return Err("invalid tile".into());
                        }
                    }
                    if !not_found && (ix == 0 || (p != 0 && !reader.accepts(kwg_bytes, p))) {
                        not_found = true;
                    }
                }
                println!(
                    "{}",
                    if not_found {
                        "Play is NOT acceptable"
                    } else {
                        "Play is Acceptable"
                    }
                );
                Ok(true)
            }
            "-q2-ort" => {
                let alphabet = make_alphabet();
                // ort: olaugh rack table.
                // the format was discussed in woogles discord.
                // https://discord.com/channels/741321677828522035/1157118170398724176/1164983643836530759
                let ort_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if ort_bytes.len() < 8 {
                    return Err("out of bounds".into());
                }
                let mut r = 0;
                let ort_num_buckets = ort_bytes[r] as u32
                    | ((ort_bytes[r + 1] as u32) << 8)
                    | ((ort_bytes[r + 2] as u32) << 16)
                    | ((ort_bytes[r + 3] as u32) << 24);
                r += 4;
                let ort_num_values = ort_bytes[r] as u32
                    | ((ort_bytes[r + 1] as u32) << 8)
                    | ((ort_bytes[r + 2] as u32) << 16)
                    | ((ort_bytes[r + 3] as u32) << 24);
                r += 4;
                if ort_bytes.len() < r + ((ort_num_buckets + 1 + ort_num_values) * 4) as usize {
                    return Err("out of bounds".into());
                }
                let mut ort_buckets = Vec::with_capacity(ort_num_buckets as usize + 1);
                for _ in 0..=ort_num_buckets {
                    ort_buckets.push(
                        ort_bytes[r] as u32
                            | ((ort_bytes[r + 1] as u32) << 8)
                            | ((ort_bytes[r + 2] as u32) << 16)
                            | ((ort_bytes[r + 3] as u32) << 24),
                    );
                    r += 4;
                }
                let mut ort_values = Vec::with_capacity(ort_num_values as usize);
                for _ in 0..ort_num_values {
                    ort_values.push(
                        ort_bytes[r] as u32
                            | ((ort_bytes[r + 1] as u32) << 8)
                            | ((ort_bytes[r + 2] as u32) << 16)
                            | ((ort_bytes[r + 3] as u32) << 24),
                    );
                    r += 4;
                }
                if r != ort_bytes.len() {
                    return Err("too many bytes".into());
                }
                if ort_buckets[0] != 0
                    || ort_buckets[ort_num_buckets as usize] != ort_num_values
                    || ort_buckets.windows(2).any(|x| x[0] > x[1])
                {
                    return Err("invalid buckets".into());
                }
                let mut csv_out = csv::Writer::from_writer(make_writer(&args[3])?);
                let mut rack_str = String::new();
                for bucket_num in 0..ort_num_buckets {
                    let mut next_allowed_quotient = 0;
                    for value in &ort_values[ort_buckets[bucket_num as usize] as usize
                        ..ort_buckets[bucket_num as usize + 1] as usize]
                    {
                        let quotient = value & 0x3fff; // 14 bits
                        if quotient < next_allowed_quotient {
                            return Err("quotients not sorted/unique".into());
                        }
                        next_allowed_quotient = quotient + 1;
                        // bucket_num is remainder, i.e. orig_hash % ort_num_buckets == bucket_num.
                        let orig_hash =
                            quotient as u64 * ort_num_buckets as u64 + bucket_num as u64;
                        rack_str.clear();
                        // recover rack_str from orig_hash, each tile uses 5 bits but can be shorter than 7 elements.
                        let mut last_seen_tile = 0;
                        if (orig_hash >> 35) != 0 {
                            return Err("too many tiles".into());
                        }
                        for shift in &[30, 25, 20, 15, 10, 5, 0] {
                            let tile = ((orig_hash >> shift) & 0x1f) as u8;
                            if tile < last_seen_tile {
                                return Err("tiles not sorted".into());
                            }
                            last_seen_tile = tile;
                            if tile != 0 {
                                rack_str.push_str(
                                    alphabet
                                        .of_rack(if tile == alphabet.len() { 0 } else { tile })
                                        .ok_or("invalid tile")?,
                                );
                            }
                        }
                        // last 6 numbers are f(0)..f(5) where f(b) = max r (0..7) such
                        // that there exists a word of length b+r with r tiles from
                        // rack and b additional tiles on board.
                        csv_out.serialize((
                            &rack_str,
                            (value >> 14) & 7,
                            (value >> 17) & 7,
                            (value >> 20) & 7,
                            (value >> 23) & 7,
                            (value >> 26) & 7,
                            (value >> 29) & 7,
                        ))?;
                    }
                }
                Ok(true)
            }
            "-make-q2-ort" => {
                // assume input is good (e.g. no duplicates)
                let num_buckets = u32::from_str(&args[4])?;
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_racks(&alphabet);
                let mut csv_reader = csv::ReaderBuilder::new()
                    .has_headers(false)
                    .from_reader(make_reader(&args[2])?);
                let mut values = Vec::new();
                let mut v = Vec::new();
                for result in csv_reader.records() {
                    let record = result?;
                    let sb = record[0].as_bytes();
                    v.clear();
                    let mut ix = 0;
                    while ix < sb.len() {
                        if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                            v.push(tile);
                            ix = end_ix;
                        } else {
                            return Err("invalid tile".into());
                        }
                    }
                    let val = ((u8::from_str(&record[6])? as u32) << 29)
                        | ((u8::from_str(&record[5])? as u32) << 26)
                        | ((u8::from_str(&record[4])? as u32) << 23)
                        | ((u8::from_str(&record[3])? as u32) << 20)
                        | ((u8::from_str(&record[2])? as u32) << 17)
                        | ((u8::from_str(&record[1])? as u32) << 14);
                    let mut orig_hash = 0u64;
                    v.sort_unstable();
                    for &i in &v {
                        if i != 0 {
                            orig_hash <<= 5;
                            orig_hash |= i as u64;
                        }
                    }
                    for &i in &v {
                        if i == 0 {
                            orig_hash <<= 5;
                            orig_hash |= alphabet.len() as u64;
                        }
                    }
                    let quotient = (orig_hash / (num_buckets as u64)) as u32;
                    if quotient & 0x3fff != quotient {
                        return Err("quotient does not fit in 14 bits".into());
                    }
                    let bucket_num = (orig_hash % (num_buckets as u64)) as u32;
                    values.push((bucket_num, quotient, val));
                }
                values.sort_unstable();
                let num_values = values.len() as u32;
                let mut ret =
                    Vec::with_capacity(4 * (3 + num_buckets as usize + num_values as usize));
                ret.extend(&num_buckets.to_le_bytes());
                ret.extend(&num_values.to_le_bytes());
                let mut bucket_start = 0u32;
                ret.extend(&bucket_start.to_le_bytes());
                let mut max_bucket_size = 0u32;
                for bucket_num in 0..num_buckets {
                    let this_bucket_start = bucket_start;
                    while bucket_start < num_values && values[bucket_start as usize].0 <= bucket_num
                    {
                        bucket_start += 1
                    }
                    ret.extend(&bucket_start.to_le_bytes());
                    max_bucket_size = max_bucket_size.max(bucket_start - this_bucket_start);
                }
                if bucket_start != num_values {
                    return Err("something wrong".into());
                }
                for (_, quotient, val) in values {
                    ret.extend(&(quotient | val).to_le_bytes());
                }
                // binary output
                make_writer(&args[3])?.write_all(&ret)?;
                writeln!(
                    boxed_stdout_or_stderr(),
                    "each bucket has at most {} values",
                    max_bucket_size
                )?;
                Ok(true)
            }
            "-wmp" | "-wmp-words" => {
                let words_only = args1_suffix == "-wmp-words";
                let alphabet = make_alphabet();
                let alphabet_label = &WolgesAlphabetLabel {
                    alphabet: &alphabet,
                };
                let alphabet_label_allow_blank = &WolgesAlphabetLabelAllowBlank {
                    alphabet: &alphabet,
                };

                // wmp: olaugh's wordmap from jvc56/MAGPIE.
                let wmp_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if wmp_bytes.len() < 6 {
                    return Err("out of bounds".into());
                }

                let mut ret = String::new();

                let wmp_ver = wmp_bytes[0];
                let max_len = wmp_bytes[1];
                let mut r = 2;
                let max_word_lookup_results = wmp_bytes[r] as u32
                    | ((wmp_bytes[r + 1] as u32) << 8)
                    | ((wmp_bytes[r + 2] as u32) << 16)
                    | ((wmp_bytes[r + 3] as u32) << 24);
                r += 4;
                if wmp_bytes.len() < 10 {
                    return Err("out of bounds".into());
                }
                let max_blank_pair_results;
                if wmp_ver < 2 {
                    max_blank_pair_results = wmp_bytes[r] as u32
                        | ((wmp_bytes[r + 1] as u32) << 8)
                        | ((wmp_bytes[r + 2] as u32) << 16)
                        | ((wmp_bytes[r + 3] as u32) << 24);
                    r += 4;
                } else {
                    max_blank_pair_results = 0;
                }
                if !words_only {
                    write!(
                        ret,
                        "wmp {} len 2..{} max_word_lookup_results={}",
                        wmp_ver, max_len, max_word_lookup_results
                    )?;
                    if wmp_ver < 2 {
                        write!(ret, " max_blank_pair_results={}", max_blank_pair_results)?;
                    }
                    ret.push('\n');
                }
                let mut expected_max_word_lookup_results = 0;
                let mut expected_max_blank_pair_results = 0;
                for len in 2..=max_len {
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_word_buckets = wmp_bytes[r] as u32
                        | ((wmp_bytes[r + 1] as u32) << 8)
                        | ((wmp_bytes[r + 2] as u32) << 16)
                        | ((wmp_bytes[r + 3] as u32) << 24);
                    r += 4;
                    let wmp_bylen_word_buckets_ofs = r;
                    r += 4 * (1 + wmp_bylen_num_word_buckets as usize);
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_word_entries = wmp_bytes[r] as u32
                        | ((wmp_bytes[r + 1] as u32) << 8)
                        | ((wmp_bytes[r + 2] as u32) << 16)
                        | ((wmp_bytes[r + 3] as u32) << 24);
                    r += 4;
                    let wmp_bylen_word_entries_ofs = r;
                    r += 28 * wmp_bylen_num_word_entries as usize;
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_words = wmp_bytes[r] as u32
                        | ((wmp_bytes[r + 1] as u32) << 8)
                        | ((wmp_bytes[r + 2] as u32) << 16)
                        | ((wmp_bytes[r + 3] as u32) << 24);
                    r += 4;
                    let wmp_bylen_words_ofs = r;
                    r += (len as u32 * wmp_bylen_num_words) as usize;

                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_blank_buckets = wmp_bytes[r] as u32
                        | ((wmp_bytes[r + 1] as u32) << 8)
                        | ((wmp_bytes[r + 2] as u32) << 16)
                        | ((wmp_bytes[r + 3] as u32) << 24);
                    r += 4;
                    let wmp_bylen_blank_buckets_ofs = r;
                    r += 4 * (1 + wmp_bylen_num_blank_buckets as usize);
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_blank_entries = wmp_bytes[r] as u32
                        | ((wmp_bytes[r + 1] as u32) << 8)
                        | ((wmp_bytes[r + 2] as u32) << 16)
                        | ((wmp_bytes[r + 3] as u32) << 24);
                    r += 4;
                    let wmp_bylen_blank_entries_ofs = r;
                    r += 28 * wmp_bylen_num_blank_entries as usize;

                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_double_blank_buckets = wmp_bytes[r] as u32
                        | ((wmp_bytes[r + 1] as u32) << 8)
                        | ((wmp_bytes[r + 2] as u32) << 16)
                        | ((wmp_bytes[r + 3] as u32) << 24);
                    r += 4;
                    let wmp_bylen_double_blank_buckets_ofs = r;
                    r += 4 * (1 + wmp_bylen_num_double_blank_buckets as usize);
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_double_blank_entries = wmp_bytes[r] as u32
                        | ((wmp_bytes[r + 1] as u32) << 8)
                        | ((wmp_bytes[r + 2] as u32) << 16)
                        | ((wmp_bytes[r + 3] as u32) << 24);
                    r += 4;
                    let wmp_bylen_double_blank_entries_ofs = r;
                    r += 28 * wmp_bylen_num_double_blank_entries as usize;
                    let wmp_bylen_blank_pairs_ofs;
                    if wmp_ver < 2 {
                        if wmp_bytes.len() < r + 4 {
                            return Err("out of bounds".into());
                        }
                        let wmp_bylen_num_blank_pairs = wmp_bytes[r] as u32
                            | ((wmp_bytes[r + 1] as u32) << 8)
                            | ((wmp_bytes[r + 2] as u32) << 16)
                            | ((wmp_bytes[r + 3] as u32) << 24);
                        r += 4;
                        wmp_bylen_blank_pairs_ofs = r;
                        r += 2 * wmp_bylen_num_blank_pairs as usize;
                    } else {
                        wmp_bylen_blank_pairs_ofs = 0;
                    }

                    if wmp_bytes.len() < r {
                        return Err("out of bounds".into());
                    }
                    if words_only {
                        let mut r = wmp_bylen_words_ofs;
                        for _ in 0..wmp_bylen_num_words {
                            for _ in 0..len {
                                alphabet_label.label(&mut ret, wmp_bytes[r])?;
                                r += 1;
                            }
                            ret.push('\n');
                        }
                    } else {
                        writeln!(ret, "\nlength: {len}")?;

                        // if there are n buckets, there are n+1 indexes:
                        // [0==s0, e0==s1, e1==s2, ..., en==num_entries].
                        writeln!(ret, "\nword buckets: {wmp_bylen_num_word_buckets}")?;
                        for bucket_idx in 0..wmp_bylen_num_word_buckets {
                            let mut p = wmp_bylen_word_buckets_ofs + bucket_idx as usize * 4;
                            let bucket_start_idx = wmp_bytes[p] as u32
                                | ((wmp_bytes[p + 1] as u32) << 8)
                                | ((wmp_bytes[p + 2] as u32) << 16)
                                | ((wmp_bytes[p + 3] as u32) << 24);
                            p += 4;
                            let bucket_end_idx = wmp_bytes[p] as u32
                                | ((wmp_bytes[p + 1] as u32) << 8)
                                | ((wmp_bytes[p + 2] as u32) << 16)
                                | ((wmp_bytes[p + 3] as u32) << 24);
                            if bucket_start_idx != bucket_end_idx {
                                writeln!(ret, "bucket {bucket_idx}/{wmp_bylen_num_word_buckets}:")?;
                                for entry_idx in bucket_start_idx..bucket_end_idx {
                                    p = wmp_bylen_word_entries_ofs + entry_idx as usize * 28 + 16;
                                    let quotient = (wmp_bytes[p] as u32
                                        | ((wmp_bytes[p + 1] as u32) << 8)
                                        | ((wmp_bytes[p + 2] as u32) << 16)
                                        | ((wmp_bytes[p + 3] as u32) << 24))
                                        as u128
                                        | (((wmp_bytes[p + 4] as u32
                                            | ((wmp_bytes[p + 5] as u32) << 8)
                                            | ((wmp_bytes[p + 6] as u32) << 16)
                                            | ((wmp_bytes[p + 7] as u32) << 24))
                                            as u128)
                                            << 32)
                                        | (((wmp_bytes[p + 8] as u32
                                            | ((wmp_bytes[p + 9] as u32) << 8)
                                            | ((wmp_bytes[p + 10] as u32) << 16)
                                            | ((wmp_bytes[p + 11] as u32) << 24))
                                            as u128)
                                            << 64);
                                    let bit_rack = quotient * wmp_bylen_num_word_buckets as u128
                                        + bucket_idx as u128;
                                    write!(ret, "  {bit_rack:032x} ")?;
                                    for i in 0..32 {
                                        for _ in 0..(bit_rack >> (4 * i)) as usize & 0xf {
                                            alphabet_label.label(&mut ret, i)?;
                                        }
                                    }
                                    ret.push_str(" =");
                                    p -= 16;
                                    let num_elts;
                                    if wmp_bytes[p] == 0 {
                                        // this is len * index, in bytes.
                                        p += 8;
                                        let initial_ofs = wmp_bytes[p] as u32
                                            | ((wmp_bytes[p + 1] as u32) << 8)
                                            | ((wmp_bytes[p + 2] as u32) << 16)
                                            | ((wmp_bytes[p + 3] as u32) << 24);
                                        p += 4;
                                        num_elts = wmp_bytes[p] as u32
                                            | ((wmp_bytes[p + 1] as u32) << 8)
                                            | ((wmp_bytes[p + 2] as u32) << 16)
                                            | ((wmp_bytes[p + 3] as u32) << 24);
                                        p = wmp_bylen_words_ofs + initial_ofs as usize;
                                    } else {
                                        num_elts = (wmp_bytes[p..p + 16]
                                            .iter()
                                            .position(|&b| b == 0)
                                            .unwrap_or(16)
                                            / len as usize)
                                            as u32;
                                    }
                                    for _ in 0..num_elts {
                                        ret.push(' ');
                                        for _ in 0..len {
                                            alphabet_label.label(&mut ret, wmp_bytes[p])?;
                                            p += 1;
                                        }
                                    }
                                    ret.push('\n');
                                }
                            }
                        }

                        writeln!(ret, "\nblank buckets: {wmp_bylen_num_blank_buckets}")?;
                        for bucket_idx in 0..wmp_bylen_num_blank_buckets {
                            let mut p = wmp_bylen_blank_buckets_ofs + bucket_idx as usize * 4;
                            let bucket_start_idx = wmp_bytes[p] as u32
                                | ((wmp_bytes[p + 1] as u32) << 8)
                                | ((wmp_bytes[p + 2] as u32) << 16)
                                | ((wmp_bytes[p + 3] as u32) << 24);
                            p += 4;
                            let bucket_end_idx = wmp_bytes[p] as u32
                                | ((wmp_bytes[p + 1] as u32) << 8)
                                | ((wmp_bytes[p + 2] as u32) << 16)
                                | ((wmp_bytes[p + 3] as u32) << 24);
                            if bucket_start_idx != bucket_end_idx {
                                writeln!(
                                    ret,
                                    "bucket {bucket_idx}/{wmp_bylen_num_blank_buckets}:"
                                )?;
                                for entry_idx in bucket_start_idx..bucket_end_idx {
                                    p = wmp_bylen_blank_entries_ofs + entry_idx as usize * 28 + 8;
                                    let bits = wmp_bytes[p] as u32
                                        | ((wmp_bytes[p + 1] as u32) << 8)
                                        | ((wmp_bytes[p + 2] as u32) << 16)
                                        | ((wmp_bytes[p + 3] as u32) << 24);
                                    p += 8;
                                    let quotient = (wmp_bytes[p] as u32
                                        | ((wmp_bytes[p + 1] as u32) << 8)
                                        | ((wmp_bytes[p + 2] as u32) << 16)
                                        | ((wmp_bytes[p + 3] as u32) << 24))
                                        as u128
                                        | (((wmp_bytes[p + 4] as u32
                                            | ((wmp_bytes[p + 5] as u32) << 8)
                                            | ((wmp_bytes[p + 6] as u32) << 16)
                                            | ((wmp_bytes[p + 7] as u32) << 24))
                                            as u128)
                                            << 32)
                                        | (((wmp_bytes[p + 8] as u32
                                            | ((wmp_bytes[p + 9] as u32) << 8)
                                            | ((wmp_bytes[p + 10] as u32) << 16)
                                            | ((wmp_bytes[p + 11] as u32) << 24))
                                            as u128)
                                            << 64);
                                    let bit_rack = quotient * wmp_bylen_num_blank_buckets as u128
                                        + bucket_idx as u128;
                                    write!(ret, "  {bit_rack:032x} ")?;
                                    for i in 0..32 {
                                        for _ in 0..(bit_rack >> (4 * i)) as usize & 0xf {
                                            alphabet_label_allow_blank.label(&mut ret, i)?;
                                        }
                                    }
                                    ret.push_str(" = ");
                                    for i in 0..32 {
                                        if bits & (1 << i) != 0 {
                                            alphabet_label.label(&mut ret, i)?;
                                        }
                                    }
                                    ret.push('\n');
                                }
                            }
                        }

                        writeln!(
                            ret,
                            "\ndouble blank buckets: {wmp_bylen_num_double_blank_buckets}"
                        )?;
                        if wmp_ver < 2 {
                            for bucket_idx in 0..wmp_bylen_num_double_blank_buckets {
                                let mut p =
                                    wmp_bylen_double_blank_buckets_ofs + bucket_idx as usize * 4;
                                let bucket_start_idx = wmp_bytes[p] as u32
                                    | ((wmp_bytes[p + 1] as u32) << 8)
                                    | ((wmp_bytes[p + 2] as u32) << 16)
                                    | ((wmp_bytes[p + 3] as u32) << 24);
                                p += 4;
                                let bucket_end_idx = wmp_bytes[p] as u32
                                    | ((wmp_bytes[p + 1] as u32) << 8)
                                    | ((wmp_bytes[p + 2] as u32) << 16)
                                    | ((wmp_bytes[p + 3] as u32) << 24);
                                if bucket_start_idx != bucket_end_idx {
                                    writeln!(
                                        ret,
                                        "bucket {bucket_idx}/{wmp_bylen_num_double_blank_buckets}:"
                                    )?;
                                    for entry_idx in bucket_start_idx..bucket_end_idx {
                                        p = wmp_bylen_double_blank_entries_ofs
                                            + entry_idx as usize * 28
                                            + 16;
                                        let quotient = (wmp_bytes[p] as u32
                                            | ((wmp_bytes[p + 1] as u32) << 8)
                                            | ((wmp_bytes[p + 2] as u32) << 16)
                                            | ((wmp_bytes[p + 3] as u32) << 24))
                                            as u128
                                            | (((wmp_bytes[p + 4] as u32
                                                | ((wmp_bytes[p + 5] as u32) << 8)
                                                | ((wmp_bytes[p + 6] as u32) << 16)
                                                | ((wmp_bytes[p + 7] as u32) << 24))
                                                as u128)
                                                << 32)
                                            | (((wmp_bytes[p + 8] as u32
                                                | ((wmp_bytes[p + 9] as u32) << 8)
                                                | ((wmp_bytes[p + 10] as u32) << 16)
                                                | ((wmp_bytes[p + 11] as u32) << 24))
                                                as u128)
                                                << 64);
                                        let bit_rack = quotient
                                            * wmp_bylen_num_double_blank_buckets as u128
                                            + bucket_idx as u128;
                                        write!(ret, "  {bit_rack:032x} ")?;
                                        for i in 0..32 {
                                            for _ in 0..(bit_rack >> (4 * i)) as usize & 0xf {
                                                alphabet_label_allow_blank.label(&mut ret, i)?;
                                            }
                                        }
                                        ret.push_str(" =");
                                        let mut this_word_lookup_results = 0u32;
                                        p -= 16;
                                        let num_elts;
                                        if wmp_bytes[p] == 0 {
                                            p += 8;
                                            let initial_ofs = wmp_bytes[p] as u32
                                                | ((wmp_bytes[p + 1] as u32) << 8)
                                                | ((wmp_bytes[p + 2] as u32) << 16)
                                                | ((wmp_bytes[p + 3] as u32) << 24);
                                            p += 4;
                                            num_elts = wmp_bytes[p] as u32
                                                | ((wmp_bytes[p + 1] as u32) << 8)
                                                | ((wmp_bytes[p + 2] as u32) << 16)
                                                | ((wmp_bytes[p + 3] as u32) << 24);
                                            p = wmp_bylen_blank_pairs_ofs + initial_ofs as usize;
                                        } else {
                                            num_elts = (wmp_bytes[p..p + 16]
                                                .iter()
                                                .position(|&b| b == 0)
                                                .unwrap_or(16)
                                                >> 1)
                                                as u32;
                                        }
                                        for _ in 0..num_elts {
                                            ret.push(' ');
                                            let mut unblanked_bit_rack = bit_rack & !0xf;
                                            for _ in 0..2 {
                                                let tile = wmp_bytes[p];
                                                unblanked_bit_rack += 1 << (4 * tile);
                                                alphabet_label.label(&mut ret, tile)?;
                                                p += 1;
                                            }

                                            {
                                                // shadow all the variables inside here.
                                                let mut sought_quotient = unblanked_bit_rack
                                                    / wmp_bylen_num_word_buckets as u128;
                                                let bucket_idx = (unblanked_bit_rack
                                                    % wmp_bylen_num_word_buckets as u128)
                                                    as u32;
                                                // write something if ZA/ZE/ZO overflow.
                                                if sought_quotient >> 96 != 0 {
                                                    sought_quotient &= (1u128 << 96) - 1;
                                                    write!(
                                                        ret,
                                                        " _OVERFLOW(0x{unblanked_bit_rack:032x}.divmod({wmp_bylen_num_word_buckets})=[0x{sought_quotient:024x},{bucket_idx}])"
                                                    )?;
                                                }
                                                let mut p = wmp_bylen_word_buckets_ofs
                                                    + bucket_idx as usize * 4;
                                                let bucket_start_idx = wmp_bytes[p] as u32
                                                    | ((wmp_bytes[p + 1] as u32) << 8)
                                                    | ((wmp_bytes[p + 2] as u32) << 16)
                                                    | ((wmp_bytes[p + 3] as u32) << 24);
                                                p += 4;
                                                let bucket_end_idx = wmp_bytes[p] as u32
                                                    | ((wmp_bytes[p + 1] as u32) << 8)
                                                    | ((wmp_bytes[p + 2] as u32) << 16)
                                                    | ((wmp_bytes[p + 3] as u32) << 24);
                                                let mut found = false;
                                                for entry_idx in bucket_start_idx..bucket_end_idx {
                                                    p = wmp_bylen_word_entries_ofs
                                                        + entry_idx as usize * 28
                                                        + 16;
                                                    let quotient = (wmp_bytes[p] as u32
                                                        | ((wmp_bytes[p + 1] as u32) << 8)
                                                        | ((wmp_bytes[p + 2] as u32) << 16)
                                                        | ((wmp_bytes[p + 3] as u32) << 24))
                                                        as u128
                                                        | (((wmp_bytes[p + 4] as u32
                                                            | ((wmp_bytes[p + 5] as u32) << 8)
                                                            | ((wmp_bytes[p + 6] as u32) << 16)
                                                            | ((wmp_bytes[p + 7] as u32) << 24))
                                                            as u128)
                                                            << 32)
                                                        | (((wmp_bytes[p + 8] as u32
                                                            | ((wmp_bytes[p + 9] as u32) << 8)
                                                            | ((wmp_bytes[p + 10] as u32) << 16)
                                                            | ((wmp_bytes[p + 11] as u32) << 24))
                                                            as u128)
                                                            << 64);
                                                    if quotient == sought_quotient {
                                                        p -= 16;
                                                        let num_elts = if wmp_bytes[p] == 0 {
                                                            p += 12;
                                                            wmp_bytes[p] as u32
                                                                | ((wmp_bytes[p + 1] as u32) << 8)
                                                                | ((wmp_bytes[p + 2] as u32) << 16)
                                                                | ((wmp_bytes[p + 3] as u32) << 24)
                                                        } else {
                                                            (wmp_bytes[p..p + 16]
                                                                .iter()
                                                                .position(|&b| b == 0)
                                                                .unwrap_or(16)
                                                                / len as usize)
                                                                as u32
                                                        };
                                                        this_word_lookup_results += num_elts;
                                                        found = true;
                                                        break;
                                                    }
                                                }
                                                if !found {
                                                    return Err("invalid double blank entry".into());
                                                }
                                            }
                                        }
                                        expected_max_word_lookup_results =
                                            expected_max_word_lookup_results
                                                .max(len as u32 * this_word_lookup_results);
                                        expected_max_blank_pair_results =
                                            expected_max_blank_pair_results.max(2 * num_elts);
                                        ret.push('\n');
                                    }
                                }
                            }
                        } else {
                            for bucket_idx in 0..wmp_bylen_num_double_blank_buckets {
                                let mut p =
                                    wmp_bylen_double_blank_buckets_ofs + bucket_idx as usize * 4;
                                let bucket_start_idx = wmp_bytes[p] as u32
                                    | ((wmp_bytes[p + 1] as u32) << 8)
                                    | ((wmp_bytes[p + 2] as u32) << 16)
                                    | ((wmp_bytes[p + 3] as u32) << 24);
                                p += 4;
                                let bucket_end_idx = wmp_bytes[p] as u32
                                    | ((wmp_bytes[p + 1] as u32) << 8)
                                    | ((wmp_bytes[p + 2] as u32) << 16)
                                    | ((wmp_bytes[p + 3] as u32) << 24);
                                if bucket_start_idx != bucket_end_idx {
                                    writeln!(
                                        ret,
                                        "bucket {bucket_idx}/{wmp_bylen_num_double_blank_buckets}:"
                                    )?;
                                    for entry_idx in bucket_start_idx..bucket_end_idx {
                                        p = wmp_bylen_double_blank_entries_ofs
                                            + entry_idx as usize * 28
                                            + 8;
                                        let bits = wmp_bytes[p] as u32
                                            | ((wmp_bytes[p + 1] as u32) << 8)
                                            | ((wmp_bytes[p + 2] as u32) << 16)
                                            | ((wmp_bytes[p + 3] as u32) << 24);
                                        p += 8;
                                        let quotient = (wmp_bytes[p] as u32
                                            | ((wmp_bytes[p + 1] as u32) << 8)
                                            | ((wmp_bytes[p + 2] as u32) << 16)
                                            | ((wmp_bytes[p + 3] as u32) << 24))
                                            as u128
                                            | (((wmp_bytes[p + 4] as u32
                                                | ((wmp_bytes[p + 5] as u32) << 8)
                                                | ((wmp_bytes[p + 6] as u32) << 16)
                                                | ((wmp_bytes[p + 7] as u32) << 24))
                                                as u128)
                                                << 32)
                                            | (((wmp_bytes[p + 8] as u32
                                                | ((wmp_bytes[p + 9] as u32) << 8)
                                                | ((wmp_bytes[p + 10] as u32) << 16)
                                                | ((wmp_bytes[p + 11] as u32) << 24))
                                                as u128)
                                                << 64);
                                        let bit_rack = quotient
                                            * wmp_bylen_num_double_blank_buckets as u128
                                            + bucket_idx as u128;
                                        write!(ret, "  {bit_rack:032x} ")?;
                                        for i in 0..32 {
                                            for _ in 0..(bit_rack >> (4 * i)) as usize & 0xf {
                                                alphabet_label_allow_blank.label(&mut ret, i)?;
                                            }
                                        }
                                        ret.push_str(" =");
                                        let mut this_word_lookup_results = 0u32;
                                        for first_i in 0..32 {
                                            if bits & (1 << first_i) != 0 {
                                                let b1_bit_rack =
                                                    (bit_rack - 1) + (1 << (4 * first_i));
                                                {
                                                    // shadow all the variables inside here.
                                                    let mut sought_quotient = b1_bit_rack
                                                        / wmp_bylen_num_blank_buckets as u128;
                                                    let bucket_idx = (b1_bit_rack
                                                        % wmp_bylen_num_blank_buckets as u128)
                                                        as u32;
                                                    // write something if ZA/ZE/ZO overflow.
                                                    if sought_quotient >> 96 != 0 {
                                                        sought_quotient &= (1u128 << 96) - 1;
                                                        write!(
                                                            ret,
                                                            " _OVERFLOW(0x{b1_bit_rack:032x}.divmod({wmp_bylen_num_blank_buckets})=[0x{sought_quotient:024x},{bucket_idx}])"
                                                        )?;
                                                    }
                                                    let mut p = wmp_bylen_blank_buckets_ofs
                                                        + bucket_idx as usize * 4;
                                                    let bucket_start_idx = wmp_bytes[p] as u32
                                                        | ((wmp_bytes[p + 1] as u32) << 8)
                                                        | ((wmp_bytes[p + 2] as u32) << 16)
                                                        | ((wmp_bytes[p + 3] as u32) << 24);
                                                    p += 4;
                                                    let bucket_end_idx = wmp_bytes[p] as u32
                                                        | ((wmp_bytes[p + 1] as u32) << 8)
                                                        | ((wmp_bytes[p + 2] as u32) << 16)
                                                        | ((wmp_bytes[p + 3] as u32) << 24);
                                                    let mut found = false;
                                                    for entry_idx in
                                                        bucket_start_idx..bucket_end_idx
                                                    {
                                                        p = wmp_bylen_blank_entries_ofs
                                                            + entry_idx as usize * 28
                                                            + 16;
                                                        let quotient = (wmp_bytes[p] as u32
                                                            | ((wmp_bytes[p + 1] as u32) << 8)
                                                            | ((wmp_bytes[p + 2] as u32) << 16)
                                                            | ((wmp_bytes[p + 3] as u32) << 24))
                                                            as u128
                                                            | (((wmp_bytes[p + 4] as u32
                                                                | ((wmp_bytes[p + 5] as u32) << 8)
                                                                | ((wmp_bytes[p + 6] as u32) << 16)
                                                                | ((wmp_bytes[p + 7] as u32) << 24))
                                                                as u128)
                                                                << 32)
                                                            | (((wmp_bytes[p + 8] as u32
                                                                | ((wmp_bytes[p + 9] as u32) << 8)
                                                                | ((wmp_bytes[p + 10] as u32)
                                                                    << 16)
                                                                | ((wmp_bytes[p + 11] as u32)
                                                                    << 24))
                                                                as u128)
                                                                << 64);
                                                        if quotient == sought_quotient {
                                                            p -= 8;
                                                            let b1_bits = wmp_bytes[p] as u32
                                                                | ((wmp_bytes[p + 1] as u32) << 8)
                                                                | ((wmp_bytes[p + 2] as u32) << 16)
                                                                | ((wmp_bytes[p + 3] as u32) << 24);
                                                            {
                                                                for i in first_i..32 {
                                                                    if b1_bits & (1 << i) != 0 {
                                                                        ret.push(' ');
                                                                        alphabet_label.label(
                                                                            &mut ret, first_i,
                                                                        )?;
                                                                        alphabet_label
                                                                            .label(&mut ret, i)?;

                                                                        let unblanked_bit_rack =
                                                                            (b1_bit_rack & !0xf)
                                                                                + (1 << (4 * i));
                                                                        {
                                                                            // shadow all the variables inside here.
                                                                            let mut sought_quotient =
                                                                                unblanked_bit_rack / wmp_bylen_num_word_buckets as u128;
                                                                            let bucket_idx = (unblanked_bit_rack
                                                                                % wmp_bylen_num_word_buckets as u128)
                                                                                as u32;
                                                                            // write something if ZA/ZE/ZO overflow.
                                                                            if sought_quotient >> 96
                                                                                != 0
                                                                            {
                                                                                sought_quotient &=
                                                                                    (1u128 << 96)
                                                                                        - 1;
                                                                                write!(
                                                                                    ret,
                                                                                    " _OVERFLOW(0x{unblanked_bit_rack:032x}.divmod({wmp_bylen_num_word_buckets})=[0x{sought_quotient:024x},{bucket_idx}])"
                                                                                )?;
                                                                            }
                                                                            let mut p = wmp_bylen_word_buckets_ofs + bucket_idx as usize * 4;
                                                                            let bucket_start_idx =
                                                                                wmp_bytes[p] as u32
                                                                                    | ((wmp_bytes
                                                                                        [p + 1]
                                                                                        as u32)
                                                                                        << 8)
                                                                                    | ((wmp_bytes
                                                                                        [p + 2]
                                                                                        as u32)
                                                                                        << 16)
                                                                                    | ((wmp_bytes
                                                                                        [p + 3]
                                                                                        as u32)
                                                                                        << 24);
                                                                            p += 4;
                                                                            let bucket_end_idx =
                                                                                wmp_bytes[p] as u32
                                                                                    | ((wmp_bytes
                                                                                        [p + 1]
                                                                                        as u32)
                                                                                        << 8)
                                                                                    | ((wmp_bytes
                                                                                        [p + 2]
                                                                                        as u32)
                                                                                        << 16)
                                                                                    | ((wmp_bytes
                                                                                        [p + 3]
                                                                                        as u32)
                                                                                        << 24);
                                                                            let mut found = false;
                                                                            for entry_idx in
                                                                                bucket_start_idx
                                                                                    ..bucket_end_idx
                                                                            {
                                                                                p = wmp_bylen_word_entries_ofs + entry_idx as usize * 28 + 16;
                                                                                let quotient = (wmp_bytes[p]
                                                                                    as u32
                                                                                    | ((wmp_bytes[p + 1]
                                                                                        as u32)
                                                                                        << 8)
                                                                                    | ((wmp_bytes[p + 2]
                                                                                        as u32)
                                                                                        << 16)
                                                                                    | ((wmp_bytes[p + 3]
                                                                                        as u32)
                                                                                        << 24))
                                                                                    as u128
                                                                                    | (((wmp_bytes[p + 4]
                                                                                        as u32
                                                                                        | ((wmp_bytes[p + 5]
                                                                                            as u32)
                                                                                            << 8)
                                                                                        | ((wmp_bytes[p + 6]
                                                                                            as u32)
                                                                                            << 16)
                                                                                        | ((wmp_bytes[p + 7]
                                                                                            as u32)
                                                                                            << 24))
                                                                                        as u128)
                                                                                        << 32)
                                                                                    | (((wmp_bytes[p + 8]
                                                                                        as u32
                                                                                        | ((wmp_bytes[p + 9]
                                                                                            as u32)
                                                                                            << 8)
                                                                                        | ((wmp_bytes
                                                                                            [p + 10]
                                                                                            as u32)
                                                                                            << 16)
                                                                                        | ((wmp_bytes
                                                                                            [p + 11]
                                                                                            as u32)
                                                                                            << 24))
                                                                                        as u128)
                                                                                        << 64);
                                                                                if quotient == sought_quotient {
                                                                                    p -= 16;
                                                                                    let num_elts = if wmp_bytes[p] == 0 {
                                                                                        p += 12;
                                                                                        wmp_bytes[p] as u32
                                                                                            | ((wmp_bytes
                                                                                                [p + 1]
                                                                                                as u32)
                                                                                                << 8)
                                                                                            | ((wmp_bytes
                                                                                                [p + 2]
                                                                                                as u32)
                                                                                                << 16)
                                                                                            | ((wmp_bytes
                                                                                                [p + 3]
                                                                                                as u32)
                                                                                                << 24)
                                                                                    } else {
                                                                                        (wmp_bytes[p..p + 16]
                                                                                            .iter()
                                                                                            .position(|&b| b == 0)
                                                                                            .unwrap_or(16)
                                                                                            / len as usize)
                                                                                            as u32
                                                                                    };
                                                                                    this_word_lookup_results += num_elts;
                                                                                    found = true;
                                                                                    break;
                                                                                }
                                                                            }
                                                                            if !found {
                                                                                return Err("invalid double blank entry".into());
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            found = true;
                                                            break;
                                                        }
                                                    }
                                                    if !found {
                                                        return Err(
                                                            "invalid double blank entry".into()
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        expected_max_word_lookup_results =
                                            expected_max_word_lookup_results
                                                .max(len as u32 * this_word_lookup_results);
                                        ret.push('\n');
                                    }
                                }
                            }
                        }
                    }
                }

                if !words_only {
                    ret.push('\n');
                    if expected_max_blank_pair_results != max_blank_pair_results {
                        writeln!(
                            ret,
                            "max_blank_pair_results != {max_blank_pair_results} (expected {expected_max_blank_pair_results})"
                        )?;
                    }
                    if expected_max_word_lookup_results != max_word_lookup_results {
                        writeln!(
                            ret,
                            "max_word_lookup_results != {max_word_lookup_results} (expected {expected_max_word_lookup_results})"
                        )?;
                    }
                }

                if wmp_bytes.len() != r {
                    return Err("incorrect file size".into());
                }

                make_writer(&args[3])?.write_all(ret.as_bytes())?;
                Ok(true)
            }
            "-make-wmp1" | "-make-wmp1-overflow" | "-make-wmp" | "-make-wmp-overflow" => {
                let allow_overflow =
                    args1_suffix == "-make-wmp-overflow" || args1_suffix == "-make-wmp1-overflow";
                let is_v1 = args1_suffix == "-make-wmp1" || args1_suffix == "-make-wmp1-overflow";
                let never_inline_b2 = allow_overflow;
                let alphabet = make_alphabet();
                let alphabet_reader = &alphabet::AlphabetReader::new_for_words(&alphabet);
                let all_words = read_machine_words_sorted_by_length(
                    alphabet_reader,
                    &read_to_string(&mut make_reader(&args[2])?)?,
                )?;
                use_writer(&args[3]); // redirect boxed_stdout_or_stderr() if appropriate.
                let mut ret = Vec::<u8>::new();
                let write_u32 = |ret: &mut Vec<u8>, x: u32| {
                    ret.extend(&x.to_le_bytes());
                };
                ret.push(if is_v1 { 1 } else { 2 }); // version
                let min_word_len = 2;
                let mut max_word_len = all_words.last().map_or(0, |x| x.len() as u8);
                if allow_overflow {
                    max_word_len = max_word_len.max(15);
                }
                let min_num_buckets = if allow_overflow {
                    let biggest =
                        std::iter::repeat_n(
                            alphabet.len().saturating_sub(1),
                            alphabet.freq(0) as usize,
                        )
                        .chain((1..alphabet.len().min(32)).rev().flat_map(|tile| {
                            std::iter::repeat_n(tile, alphabet.freq(tile) as usize)
                        }))
                        .take(max_word_len as usize)
                        .fold(0u128, |acc, tile| {
                            if (acc >> (tile << 2)) & 0xf < 0xf {
                                acc + (1u128 << (tile << 2))
                            } else {
                                acc
                            }
                        });
                    let target = 1u128 << 96;
                    next_prime(((biggest / target) as u32).saturating_add(1))
                } else {
                    0 // not used
                };
                ret.push(max_word_len);
                write_u32(&mut ret, 0); // placeholder for max_word_lookup_results
                if is_v1 {
                    write_u32(&mut ret, 0); // placeholder for max_blank_pair_results
                }
                let mut max_word_lookup_results = 0u32;
                let mut max_blank_pair_results = 0u32;
                // skip over any 1-letter words
                let mut min_idx = all_words.iter().position(|x| x.len() >= 2).unwrap_or(0);
                for this_len in min_word_len..=max_word_len {
                    // not using .chunk_by to allow non-consecutive word lengths.
                    let max_idx = min_idx + {
                        let this_len = this_len as usize;
                        all_words[min_idx..]
                            .iter()
                            .position(|x| x.len() != this_len)
                            .unwrap_or_else(|| all_words[min_idx..].len())
                    };
                    let words = &all_words[min_idx..max_idx];
                    min_idx = max_idx;

                    let mut bits = [0u8; 16];

                    let mut b0_vec = Vec::<([u8; 16], bites::Bites)>::with_capacity(words.len());
                    for this_word in words {
                        bits.iter_mut().for_each(|m| *m = 0);
                        for &t in this_word.iter() {
                            if t < 32 {
                                let bit_idx = (t >> 1) as usize;
                                if t & 1 == 0 {
                                    if bits[bit_idx] & 0xf == 0xf {
                                        return Err(format!(
                                            "word {:?} has too many tile {}",
                                            this_word, t
                                        )
                                        .into());
                                    }
                                    bits[bit_idx] += 1;
                                } else {
                                    if bits[bit_idx] & 0xf0 == 0xf0 {
                                        return Err(format!(
                                            "word {:?} has too many tile {}",
                                            this_word, t
                                        )
                                        .into());
                                    }
                                    bits[bit_idx] += 0x10;
                                }
                            } else {
                                return Err(format!("word {:?} has tile {}", this_word, t).into());
                            }
                        }
                        b0_vec.push((bits, this_word.clone()));
                    }
                    b0_vec.sort_unstable();

                    // value: (start, end) of same-alphagram words.
                    let mut b0_bits = fash::MyHashMap::<[u8; 16], (u32, u32)>::default();
                    for (i, (bits, _)) in (1..).zip(b0_vec.iter()) {
                        b0_bits
                            .entry(*bits)
                            .and_modify(|e| e.1 = i)
                            .or_insert_with(|| (i - 1, i));
                    }

                    // value: bits 1-31 = if adding that tile forms b0_bits.
                    // bit 0 should always be 0.
                    let mut b1_bits = fash::MyHashMap::<[u8; 16], u32>::default();
                    for key_bits in b0_bits.keys() {
                        bits.clone_from(key_bits);
                        bits[0] += 1;
                        for t2 in 0..16 {
                            let b = bits[t2] & if t2 == 0 { 0xf0 } else { 0xff };
                            if b != 0 {
                                let mut v = 1 << (t2 * 2);
                                if b & 0xf != 0 {
                                    bits[t2] -= 1;
                                    b1_bits.entry(bits).and_modify(|e| *e |= v).or_insert(v);
                                    bits[t2] += 1;
                                }
                                if b & 0xf0 != 0 {
                                    v <<= 1;
                                    bits[t2] -= 0x10;
                                    b1_bits.entry(bits).and_modify(|e| *e |= v).or_insert(v);
                                    bits[t2] += 0x10;
                                }
                            }
                        }
                        bits[0] -= 1;
                    }

                    // value: bits 1-31 = if adding that tile forms b1_bits.
                    // bit 0 should always be 0.
                    let mut b2_bits = fash::MyHashMap::<[u8; 16], u32>::default();
                    for (key_bits, val_bits) in b1_bits.iter() {
                        bits.clone_from(key_bits);
                        bits[0] += 1;
                        for t2 in 0..16 {
                            let b = bits[t2] & if t2 == 0 { 0xf0 } else { 0xff };
                            if b != 0 {
                                let mut v = 1 << (t2 * 2);
                                if b & 0xf != 0 && val_bits & !(v - 1) != 0 {
                                    bits[t2] -= 1;
                                    b2_bits.entry(bits).and_modify(|e| *e |= v).or_insert(v);
                                    bits[t2] += 1;
                                }
                                if b & 0xf0 != 0 {
                                    v <<= 1;
                                    if val_bits & !(v - 1) != 0 {
                                        bits[t2] -= 0x10;
                                        b2_bits.entry(bits).and_modify(|e| *e |= v).or_insert(v);
                                        bits[t2] += 0x10;
                                    }
                                }
                            }
                        }
                        bits[0] -= 1;
                    }

                    let num_word_buckets;
                    let num_blank_buckets;
                    let num_double_blank_buckets;
                    if allow_overflow {
                        // numbers of buckets in v1/v2.
                        // these are still not correct and will cause overflow.
                        // they are based on top 15 available non-letters
                        // (??ZYYXWW... with the ?s being valued as Z)
                        // even if there is a word like PIZZAZZ.
                        num_word_buckets = next_prime(b0_bits.len() as u32).max(min_num_buckets);
                        num_blank_buckets = next_prime(b1_bits.len() as u32).max(min_num_buckets);
                        num_double_blank_buckets =
                            next_prime(b2_bits.len() as u32).max(min_num_buckets);
                    } else {
                        let adjust_num_word_buckets = |len: u32, biggest: u128| {
                            let guess = next_prime(len);
                            let target = 1u128 << 96;
                            if biggest / (guess as u128) < target {
                                // intentional floor division.
                                guess
                            } else {
                                // intentional floor division.
                                // this can still overflow if given 15 of everything!
                                next_prime(((biggest / target) as u32).saturating_add(1))
                            }
                        };
                        num_word_buckets = adjust_num_word_buckets(
                            b0_bits.len() as u32,
                            b0_bits
                                .keys()
                                .fold(0u128, |acc, bits| acc.max(u128::from_le_bytes(*bits))),
                        );
                        num_blank_buckets = adjust_num_word_buckets(
                            b1_bits.len() as u32,
                            b1_bits
                                .keys()
                                .fold(0u128, |acc, bits| acc.max(u128::from_le_bytes(*bits))),
                        );
                        num_double_blank_buckets = adjust_num_word_buckets(
                            b2_bits.len() as u32,
                            b2_bits
                                .keys()
                                .fold(0u128, |acc, bits| acc.max(u128::from_le_bytes(*bits))),
                        );
                    }

                    // 0-blank section
                    {
                        let mut linearized_buckets = b0_bits
                            .keys()
                            .map(|bits| {
                                let bits_u128 = u128::from_le_bytes(*bits);
                                let mut quotient = bits_u128 / (num_word_buckets as u128);
                                let remainder = (bits_u128 % num_word_buckets as u128) as u32;
                                if quotient >> 96 != 0 {
                                    quotient &= (1u128 << 96) - 1;
                                    // hopefully this does not crash
                                    writeln!(
                                        boxed_stdout_or_stderr(),
                                        "OVERFLOW: 0x{:032x}.divmod({})=[0x{:024x},{}]",
                                        bits_u128,
                                        num_word_buckets,
                                        quotient,
                                        remainder
                                    )
                                    .unwrap();
                                }
                                (remainder, quotient, bits)
                            })
                            .collect::<Box<_>>();
                        linearized_buckets.sort_unstable();
                        write_u32(&mut ret, num_word_buckets);
                        write_u32(&mut ret, 0);
                        let mut bucket_min_idx = 0;
                        for this_rem in 0..num_word_buckets {
                            let bucket_max_idx = bucket_min_idx + {
                                linearized_buckets[bucket_min_idx..]
                                    .iter()
                                    .position(|x| x.0 != this_rem)
                                    .unwrap_or_else(|| linearized_buckets[bucket_min_idx..].len())
                            };
                            write_u32(&mut ret, bucket_max_idx as u32);
                            bucket_min_idx = bucket_max_idx;
                        }
                        write_u32(&mut ret, linearized_buckets.len() as u32);
                        // pretend it was already re-sorted according to remainder order.
                        let mut cum_start_idx = 0;
                        let inline_max = 16u32 / this_len as u32;
                        let mut uninlined_len = 0u32;
                        for (_remainder, quotient, bits) in linearized_buckets.iter() {
                            let (start_idx, end_idx) = b0_bits[*bits];
                            let num_elts = end_idx - start_idx;
                            if num_elts <= inline_max {
                                let new_size = ret.len() + 16;
                                for word_idx in start_idx..end_idx {
                                    ret.extend(&b0_vec[word_idx as usize].1[..]);
                                }
                                ret.resize(new_size, 0);
                            } else {
                                uninlined_len += num_elts;
                                write_u32(&mut ret, 0);
                                write_u32(&mut ret, 0);
                                write_u32(&mut ret, cum_start_idx * this_len as u32);
                                write_u32(&mut ret, num_elts);
                                cum_start_idx += num_elts;
                            }
                            write_u32(&mut ret, *quotient as u32);
                            write_u32(&mut ret, (quotient >> 32) as u32);
                            write_u32(&mut ret, (quotient >> 64) as u32);
                        }
                        write_u32(&mut ret, uninlined_len);
                        for (_remainder, _quotient, bits) in linearized_buckets.iter() {
                            let (start_idx, end_idx) = b0_bits[*bits];
                            if end_idx - start_idx > inline_max {
                                for word_idx in start_idx..end_idx {
                                    ret.extend(&b0_vec[word_idx as usize].1[..]);
                                }
                            }
                        }
                    }

                    // 1-blank section
                    {
                        let mut linearized_buckets = b1_bits
                            .keys()
                            .map(|bits| {
                                let bits_u128 = u128::from_le_bytes(*bits);
                                let mut quotient = bits_u128 / (num_blank_buckets as u128);
                                let remainder = (bits_u128 % num_blank_buckets as u128) as u32;
                                if quotient >> 96 != 0 {
                                    quotient &= (1u128 << 96) - 1;
                                    // hopefully this does not crash
                                    writeln!(
                                        boxed_stdout_or_stderr(),
                                        "OVERFLOW: 0x{:032x}.divmod({})=[0x{:024x},{}]",
                                        bits_u128,
                                        num_blank_buckets,
                                        quotient,
                                        remainder
                                    )
                                    .unwrap();
                                }
                                (remainder, quotient, bits)
                            })
                            .collect::<Box<_>>();
                        linearized_buckets.sort_unstable();
                        write_u32(&mut ret, num_blank_buckets);
                        write_u32(&mut ret, 0);
                        let mut bucket_min_idx = 0;
                        for this_rem in 0..num_blank_buckets {
                            let bucket_max_idx = bucket_min_idx + {
                                linearized_buckets[bucket_min_idx..]
                                    .iter()
                                    .position(|x| x.0 != this_rem)
                                    .unwrap_or_else(|| linearized_buckets[bucket_min_idx..].len())
                            };
                            write_u32(&mut ret, bucket_max_idx as u32);
                            bucket_min_idx = bucket_max_idx;
                        }
                        write_u32(&mut ret, linearized_buckets.len() as u32);
                        // pretend it was already re-sorted according to remainder order.
                        for (_remainder, quotient, bits) in linearized_buckets.iter() {
                            let blank_bits = b1_bits[*bits];
                            write_u32(&mut ret, 0);
                            write_u32(&mut ret, 0);
                            write_u32(&mut ret, blank_bits);
                            write_u32(&mut ret, 0);
                            write_u32(&mut ret, *quotient as u32);
                            write_u32(&mut ret, (quotient >> 32) as u32);
                            write_u32(&mut ret, (quotient >> 64) as u32);
                        }
                    }

                    // 2-blank section
                    {
                        let mut max_word_lookup_results_before_multiplying_len = 0u32;
                        if is_v1 {
                            let mut b2_chars = Vec::<u8>::new(); // even number of elements to be interpreted in pairs.
                            let mut linearized_buckets = b2_bits
                                .iter()
                                .map(|(bits_from_b2, b1)| {
                                    bits.clone_from(bits_from_b2);
                                    let bits_u128 = u128::from_le_bytes(*bits_from_b2);
                                    let mut quotient =
                                        bits_u128 / (num_double_blank_buckets as u128);
                                    let remainder =
                                        (bits_u128 % num_double_blank_buckets as u128) as u32;
                                    if quotient >> 96 != 0 {
                                        quotient &= (1u128 << 96) - 1;
                                        // hopefully this does not crash
                                        writeln!(
                                            boxed_stdout_or_stderr(),
                                            "OVERFLOW: 0x{:032x}.divmod({})=[0x{:024x},{}]",
                                            bits_u128,
                                            num_double_blank_buckets,
                                            quotient,
                                            remainder
                                        )
                                        .unwrap();
                                    }
                                    let start_idx = b2_chars.len() as u32;
                                    let mut num_words_here = 0u32;
                                    let mut b1 = *b1;
                                    bits[0] -= 1;
                                    while b1 != 0 {
                                        let t1 = b1.trailing_zeros() as u8;
                                        let bit_idx = (t1 >> 1) as usize;
                                        bits[bit_idx] += if t1 & 1 == 0 { 1 } else { 0x10 };
                                        let mut b0 = b1_bits[&bits] & !((1u32 << t1) - 1);
                                        bits[0] -= 1;
                                        while b0 != 0 {
                                            let t0 = b0.trailing_zeros() as u8;
                                            let bit_idx = (t0 >> 1) as usize;
                                            bits[bit_idx] += if t0 & 1 == 0 { 1 } else { 0x10 };
                                            let word_indexes = b0_bits[&bits];
                                            num_words_here += word_indexes.1 - word_indexes.0;
                                            b2_chars.push(t1);
                                            b2_chars.push(t0);
                                            bits[bit_idx] -= if t0 & 1 == 0 { 1 } else { 0x10 };
                                            b0 &= b0 - 1;
                                        }
                                        bits[0] += 1;
                                        bits[bit_idx] -= if t1 & 1 == 0 { 1 } else { 0x10 };
                                        b1 &= b1 - 1;
                                    }
                                    // no need to restore bits[0]
                                    max_word_lookup_results_before_multiplying_len =
                                        max_word_lookup_results_before_multiplying_len
                                            .max(num_words_here);
                                    let end_idx = b2_chars.len() as u32;
                                    (remainder, quotient, start_idx, end_idx)
                                })
                                .collect::<Box<_>>();
                            linearized_buckets.sort_unstable();
                            write_u32(&mut ret, num_double_blank_buckets);
                            write_u32(&mut ret, 0);
                            let mut bucket_min_idx = 0;
                            for this_rem in 0..num_double_blank_buckets {
                                let bucket_max_idx = bucket_min_idx + {
                                    linearized_buckets[bucket_min_idx..]
                                        .iter()
                                        .position(|x| x.0 != this_rem)
                                        .unwrap_or_else(|| {
                                            linearized_buckets[bucket_min_idx..].len()
                                        })
                                };
                                write_u32(&mut ret, bucket_max_idx as u32);
                                bucket_min_idx = bucket_max_idx;
                            }
                            write_u32(&mut ret, linearized_buckets.len() as u32);
                            // pretend it was already re-sorted according to remainder order.
                            let mut cum_start_idx = 0;
                            let mut uninlined_len = 0u32;
                            for (_remainder, quotient, start_idx, end_idx) in
                                linearized_buckets.iter()
                            {
                                let num_elts = end_idx - start_idx;
                                if !never_inline_b2 && num_elts <= 16 {
                                    let new_size = ret.len() + 16;
                                    ret.extend(&b2_chars[*start_idx as usize..*end_idx as usize]);
                                    ret.resize(new_size, 0);
                                } else {
                                    uninlined_len += num_elts;
                                    max_blank_pair_results = max_blank_pair_results.max(num_elts); // already * 2
                                    write_u32(&mut ret, 0);
                                    write_u32(&mut ret, 0);
                                    write_u32(&mut ret, cum_start_idx); // already * 2
                                    write_u32(&mut ret, num_elts >> 1); // here / 2
                                    cum_start_idx += num_elts;
                                }
                                write_u32(&mut ret, *quotient as u32);
                                write_u32(&mut ret, (quotient >> 32) as u32);
                                write_u32(&mut ret, (quotient >> 64) as u32);
                            }
                            write_u32(&mut ret, uninlined_len >> 1); // here / 2
                            for (_remainder, _quotient, start_idx, end_idx) in
                                linearized_buckets.iter()
                            {
                                if never_inline_b2 || end_idx - start_idx > 16 {
                                    ret.extend(&b2_chars[*start_idx as usize..*end_idx as usize]);
                                }
                            }
                        } else {
                            let mut linearized_buckets = b2_bits
                                .iter()
                                .map(|(key_bits, val_bits)| {
                                    let bits_u128 = u128::from_le_bytes(*key_bits);
                                    let mut quotient =
                                        bits_u128 / (num_double_blank_buckets as u128);
                                    let remainder =
                                        (bits_u128 % num_double_blank_buckets as u128) as u32;
                                    if quotient >> 96 != 0 {
                                        quotient &= (1u128 << 96) - 1;
                                        // hopefully this does not crash
                                        writeln!(
                                            boxed_stdout_or_stderr(),
                                            "OVERFLOW: 0x{:032x}.divmod({})=[0x{:024x},{}]",
                                            bits_u128,
                                            num_double_blank_buckets,
                                            quotient,
                                            remainder
                                        )
                                        .unwrap();
                                    }

                                    // this part is only to compute that max_word_lookup_results value.
                                    let mut num_words_here = 0u32;
                                    let mut b1 = *val_bits;
                                    bits.clone_from(key_bits);
                                    bits[0] -= 1;
                                    while b1 != 0 {
                                        let t1 = b1.trailing_zeros() as u8;
                                        let bit_idx = (t1 >> 1) as usize;
                                        bits[bit_idx] += if t1 & 1 == 0 { 1 } else { 0x10 };
                                        let mut b0 = b1_bits[&bits] & !((1u32 << t1) - 1);
                                        bits[0] -= 1;
                                        while b0 != 0 {
                                            let t0 = b0.trailing_zeros() as u8;
                                            let bit_idx = (t0 >> 1) as usize;
                                            bits[bit_idx] += if t0 & 1 == 0 { 1 } else { 0x10 };
                                            let word_indexes = b0_bits[&bits];
                                            num_words_here += word_indexes.1 - word_indexes.0;
                                            bits[bit_idx] -= if t0 & 1 == 0 { 1 } else { 0x10 };
                                            b0 &= b0 - 1;
                                        }
                                        bits[0] += 1;
                                        bits[bit_idx] -= if t1 & 1 == 0 { 1 } else { 0x10 };
                                        b1 &= b1 - 1;
                                    }
                                    // no need to restore bits[0]
                                    max_word_lookup_results_before_multiplying_len =
                                        max_word_lookup_results_before_multiplying_len
                                            .max(num_words_here);

                                    (remainder, quotient, key_bits)
                                })
                                .collect::<Box<_>>();
                            linearized_buckets.sort_unstable();
                            write_u32(&mut ret, num_double_blank_buckets);
                            write_u32(&mut ret, 0);
                            let mut bucket_min_idx = 0;
                            for this_rem in 0..num_double_blank_buckets {
                                let bucket_max_idx = bucket_min_idx + {
                                    linearized_buckets[bucket_min_idx..]
                                        .iter()
                                        .position(|x| x.0 != this_rem)
                                        .unwrap_or_else(|| {
                                            linearized_buckets[bucket_min_idx..].len()
                                        })
                                };
                                write_u32(&mut ret, bucket_max_idx as u32);
                                bucket_min_idx = bucket_max_idx;
                            }
                            write_u32(&mut ret, linearized_buckets.len() as u32);
                            // pretend it was already re-sorted according to remainder order.
                            for (_remainder, quotient, bits) in linearized_buckets.iter() {
                                let blank_bits = b2_bits[*bits];
                                write_u32(&mut ret, 0);
                                write_u32(&mut ret, 0);
                                write_u32(&mut ret, blank_bits);
                                write_u32(&mut ret, 0);
                                write_u32(&mut ret, *quotient as u32);
                                write_u32(&mut ret, (quotient >> 32) as u32);
                                write_u32(&mut ret, (quotient >> 64) as u32);
                            }
                        }
                        max_word_lookup_results = max_word_lookup_results
                            .max(max_word_lookup_results_before_multiplying_len * this_len as u32);
                    }
                }
                ret[2..6].copy_from_slice(&max_word_lookup_results.to_le_bytes());
                if is_v1 {
                    ret[6..10].copy_from_slice(&max_blank_pair_results.to_le_bytes());
                }
                // binary output
                make_writer(&args[3])?.write_all(&ret)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        None => Ok(false),
    }
}

// note: kwg in legacy layout is reported to be slower than the newer layouts.
// this cache simulation contradicts that observation. can someone explain?
fn kwg_hitcheck<R: WgReader>(
    ret: &mut String,
    r: &R,
    b: &[u8],
    initial_idx: usize,
    cache_line_size: u32,
    cache_set_associativity: u32,
    num_cache_sets: u32,
) -> error::Returns<()> {
    struct Cache {
        cache_line_size: u32,
        cache_set_associativity: u32,
        num_cache_sets: u32,
        misses: u64,
        prefetches: u64,
        cache_set_content: Vec<Vec<usize>>,
    }

    impl Cache {
        #[inline(always)]
        fn visit(&mut self, ret: &mut String, p: usize) -> error::Returns<()> {
            let byte_idx = p << 2; // because p is an idx to u32.
            let cache_line_idx = byte_idx / self.cache_line_size as usize; // these are cached together.
            let cache_set_idx = cache_line_idx % self.num_cache_sets as usize; // it can only go here.
            let mut cache_set = &mut self.cache_set_content[cache_set_idx];
            if let Some(pos) = cache_set.iter().rposition(|&x| x == cache_line_idx) {
                // to simulate FIFO instead of LRU, disable this by changing < to >.
                if pos < cache_set.len() - 1 {
                    cache_set.remove(pos);
                    cache_set.push(cache_line_idx);
                }
            } else {
                if cache_set.len() >= self.cache_set_associativity as usize {
                    cache_set.remove(0); // remove least recently used.
                }
                cache_set.push(cache_line_idx);
                self.misses += 1;
                writeln!(
                    ret,
                    "{:7} {:6x} {:5x} {:2x} {:x?}",
                    p, byte_idx, cache_line_idx, cache_set_idx, cache_set
                )?;
            }
            // if the previous cache set contains the previous cache line,
            // prefetch the following cache set.
            let prev_cache_line_idx = cache_line_idx.saturating_sub(1);
            let prev_cache_set_idx = prev_cache_line_idx % self.num_cache_sets as usize; // it can only go here.
            cache_set = &mut self.cache_set_content[prev_cache_set_idx];
            if cache_set.contains(&prev_cache_line_idx) {
                let next_cache_line_idx = cache_line_idx.saturating_add(1);
                let next_cache_set_idx = next_cache_line_idx % self.num_cache_sets as usize; // it can only go here.
                cache_set = &mut self.cache_set_content[next_cache_set_idx];
                if let Some(next_pos) = cache_set.iter().rposition(|&x| x == next_cache_line_idx) {
                    // to simulate FIFO instead of LRU, disable this by changing < to >.
                    if next_pos < cache_set.len() - 1 {
                        cache_set.remove(next_pos);
                        cache_set.push(next_cache_line_idx);
                    }
                } else {
                    if cache_set.len() >= self.cache_set_associativity as usize {
                        cache_set.remove(0); // remove least recently used.
                    }
                    cache_set.push(next_cache_line_idx);
                    self.prefetches += 1;
                    writeln!(
                        ret,
                        "{:7} {:6} {:5x} {:2x} {:x?}",
                        p, "pre", next_cache_line_idx, next_cache_set_idx, cache_set
                    )?;
                }
            }
            Ok(())
        }
    }

    let mut c = Cache {
        cache_line_size,
        cache_set_associativity,
        num_cache_sets,
        misses: 0,
        prefetches: 0,
        cache_set_content: Vec::with_capacity(num_cache_sets as usize),
    };
    for _ in 0..num_cache_sets {
        c.cache_set_content
            .push(Vec::with_capacity(cache_set_associativity as usize));
    }

    struct Env<'a, R: WgReader> {
        ret: &'a mut String,
        r: &'a R,
        b: &'a [u8],
        c: &'a mut Cache,
    }
    fn iter<R: WgReader>(env: &mut Env<'_, R>, mut p: usize) -> error::Returns<()> {
        loop {
            if p >= env.r.len(env.b) {
                return Err("out of bounds".into());
            }
            env.c.visit(env.ret, p)?;
            if env.r.arc_index(env.b, p) != 0 {
                iter(env, env.r.arc_index(env.b, p))?;
                env.c.visit(env.ret, p)?; // is_end is usually read after returning.
            }
            if env.r.is_end(env.b, p) {
                break;
            }
            p += 1;
        }
        Ok(())
    }
    if initial_idx >= r.len(b) {
        return Err("out of bounds".into());
    }
    iter(
        &mut Env {
            ret,
            r,
            b,
            c: &mut c,
        },
        initial_idx,
    )?;

    writeln!(
        ret,
        "cache: {} misses, {} prefetches",
        c.misses, c.prefetches
    )?;

    Ok(())
}

// naive algorithm.
fn next_prime(mut x: u32) -> u32 {
    if x <= 2 {
        return 2;
    }
    if x & 1 == 0 {
        x += 1
    }
    if x <= 7 {
        return x;
    }
    loop {
        let mut j = 3;
        while x % j != 0 {
            j += 2;
            if j * j > x {
                return x;
            }
        }
        x += 2;
    }
}

fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 {
        println!(
            "args:
  english-klv CSW24.klv CSW24.csv
  english-klv CSW24.klv2 CSW24.csv
    read klv/klv2 file
  english-kwg CSW24.kwg CSW24.txt
  english-kwg CSW24.kad CSW24.txt
    read kwg/kad file (dawg) (use kwg0 to allow 0, such as for klv-kwg-extract)
  english-kwg-gaddag CSW24.kwg CSW24.txt
    read gaddawg kwg file (gaddag)
  english-kwg-nodes CSW24.kwg CSW24.kwg.raw
    read kwg file for human inspection
  english-kwg-prob CSW24.kwg -
    read kwg file (dawg) by probability (output format subject to changes)
  english-prob word [word...]
    show raw probability
  english-klv-anagram- CSW24.klv2 - A?AC
  english-klv-anagram CSW24.klv2 - A?AC
  english-klv-anagram+ CSW24.klv2 - A?AC
    list all leaves with subanagram, anagram, or superanagram
  english-kwg-anagram- CSW24.kwg - A?AC
  english-kwg-anagram CSW24.kwg - A?AC
  english-kwg-anagram+ CSW24.kwg - A?AC
    list all words with subanagram, anagram, or superanagram (using dawg)
  english-kwg-check CSW24.kwg word [word...]
    checks if all words are valid (using dawg)
  english-q2-ort something.ort something.csv
    read .ort (format subject to change)
  english-make-q2-ort something.csv something.ort num_buckets
    generate .ort with the given num_buckets (ideally prime eg 5297687)
  english-wmp-words something.wmp something.txt
    read .wmp words (format subject to change)
  english-wmp something.wmp something.txt
    read .wmp (format subject to change)
  english-make-wmp1 something.txt something.wmp
  english-make-wmp1-overflow something.txt something.wmp
    generate .wmp v1 (-overflow = allow overflows, disable 2-blank inlining)
  english-make-wmp something.txt something.wmp
  english-make-wmp-overflow something.txt something.wmp
    generate .wmp v2 (-overflow = allow overflows)
  (english can also be catalan, french, german, norwegian, polish, slovene,
    spanish, decimal, hex)
  klv-kwg-extract CSW24.klv2 racks.kwg
    just copy out the kwg for further analysis.
  kwg-hitcheck CSW24.kwg cls csa ncs outfile
    check dawg cache hit rate.
    cls = cache line size, 64 is typical.
    csa = cache set associativity, e.g. 8 for 8-way set associativity.
    ncs = number of cache sets = L1D cache size / cores / csa / cls.
    example for i7-8700B (192 kB, 6 cores, 8-way set assoc): 64 8 64.
  kwg-hitcheck-gaddag CSW24.kwg cls csa ncs outfile
    ditto for gaddag.
  quackle-make-superleaves CSW24.klv superleaves
    read klv/klv2 file, save quackle superleaves (english/french)
  quackle-superleaves superleaves something.csv
    read quackle superleaves (english/french)
  quackle something.dawg something.txt
    read quackle dawg
  quackle-small something.dawg something.txt
    read quackle small dawg (for example, TWL-only words in older CSW files)
  zyzzyva something.dwg something.txt
    read zyzzyva dawg
  lexpert something.lxd something.txt
    read lexpert dawg
  stats-zt
    experimental statistics exploration showing the Z table
input/output files can be \"-\" (not advisable for binary files)"
        );
        Ok(())
    } else {
        let t0 = std::time::Instant::now();
        if do_lang(&args, "english", alphabet::make_english_alphabet)?
            || do_lang(&args, "catalan", alphabet::make_catalan_alphabet)?
            || do_lang(&args, "french", alphabet::make_french_alphabet)?
            || do_lang(&args, "german", alphabet::make_german_alphabet)?
            || do_lang(&args, "norwegian", alphabet::make_norwegian_alphabet)?
            || do_lang(&args, "polish", alphabet::make_polish_alphabet)?
            || do_lang(&args, "slovene", alphabet::make_slovene_alphabet)?
            || do_lang(&args, "spanish", alphabet::make_spanish_alphabet)?
            || do_lang(&args, "decimal", alphabet::make_decimal_alphabet)?
            || do_lang(&args, "hex", alphabet::make_hex_alphabet)?
            || do_lang(
                &args,
                "super-english",
                alphabet::make_super_english_alphabet,
            )?
            || do_lang(
                &args,
                "super-catalan",
                alphabet::make_super_catalan_alphabet,
            )?
            || do_lang(
                &args,
                "hong-kong-english",
                alphabet::make_hong_kong_english_alphabet,
            )?
        {
        } else if args[1] == "klv-kwg-extract" {
            let klv_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            if klv_bytes.len() < 4 {
                return Err("out of bounds".into());
            }
            let mut r = 0;
            let kwg_bytes_len = ((klv_bytes[r] as u32
                | ((klv_bytes[r + 1] as u32) << 8)
                | ((klv_bytes[r + 2] as u32) << 16)
                | ((klv_bytes[r + 3] as u32) << 24)) as usize)
                * 4;
            r += 4;
            if klv_bytes.len() < r + kwg_bytes_len + 4 {
                return Err("out of bounds".into());
            }
            let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
            // binary output
            make_writer(&args[3])?.write_all(kwg_bytes)?;
        } else if args[1] == "kwg-hitcheck" {
            let reader = &KwgReader {};
            let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            if 0 == reader.len(kwg_bytes) {
                return Err("out of bounds".into());
            }
            let mut ret = String::new();
            kwg_hitcheck(
                &mut ret,
                reader,
                kwg_bytes,
                0,
                u32::from_str(&args[3])?,
                u32::from_str(&args[4])?,
                u32::from_str(&args[5])?,
            )?;
            make_writer(&args[6])?.write_all(ret.as_bytes())?;
        } else if args[1] == "kwg-hitcheck-gaddag" {
            let reader = &KwgReader {};
            let kwg_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            if 1 >= reader.len(kwg_bytes) {
                return Err("out of bounds".into());
            }
            let mut ret = String::new();
            kwg_hitcheck(
                &mut ret,
                reader,
                kwg_bytes,
                1,
                u32::from_str(&args[3])?,
                u32::from_str(&args[4])?,
                u32::from_str(&args[5])?,
            )?;
            make_writer(&args[6])?.write_all(ret.as_bytes())?;
        } else if args[1] == "quackle-make-superleaves" {
            let reader = &KwgReader {};
            let klv_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            if klv_bytes.len() < 4 {
                return Err("out of bounds".into());
            }
            let mut r = 0;
            let kwg_bytes_len = ((klv_bytes[r] as u32
                | ((klv_bytes[r + 1] as u32) << 8)
                | ((klv_bytes[r + 2] as u32) << 16)
                | ((klv_bytes[r + 3] as u32) << 24)) as usize)
                * 4;
            r += 4;
            if klv_bytes.len() < r + kwg_bytes_len + 4 {
                return Err("out of bounds".into());
            }
            let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
            r += kwg_bytes_len;
            let lv_len = (klv_bytes[r] as u32
                | ((klv_bytes[r + 1] as u32) << 8)
                | ((klv_bytes[r + 2] as u32) << 16)
                | ((klv_bytes[r + 3] as u32) << 24)) as usize;
            r += 4;
            let is_klv2 = klv_bytes.len() >= r + lv_len * 4;
            if 0 == reader.len(kwg_bytes) {
                return Err("out of bounds".into());
            }
            let mut ret = Vec::new();
            iter_dawg(
                &QuackleLeavesAlphabetLabel {},
                reader,
                kwg_bytes,
                reader.arc_index(kwg_bytes, 0),
                Some("\x01"),
                &mut |s: &str| {
                    let float_leave = if is_klv2 && klv_bytes.len() >= r + 4 {
                        r += 4;
                        f32::from_bits(
                            klv_bytes[r - 4] as u32
                                | ((klv_bytes[r - 3] as u32) << 8)
                                | ((klv_bytes[r - 2] as u32) << 16)
                                | ((klv_bytes[r - 1] as u32) << 24),
                        )
                    } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                        r += 2;
                        ((klv_bytes[r - 2] as u16 | ((klv_bytes[r - 1] as u16) << 8)) as i16) as f32
                            * (1.0 / 256.0)
                    } else {
                        return Err("missing leaves".into());
                    };
                    let rounded_leave = (float_leave * 256.0).round();
                    let int_leave = (rounded_leave as i16) ^ 0x8000u16 as i16;
                    let slen = s.len();
                    ret.reserve(slen + 3);
                    ret.push(slen as u8);
                    ret.extend(s.bytes());
                    ret.extend(int_leave.to_le_bytes());
                    Ok(())
                },
                &mut default_in,
                &mut default_out,
            )?;
            if r != klv_bytes.len() {
                return Err("too many leaves".into());
            }
            // binary output
            make_writer(&args[3])?.write_all(&ret)?;
        } else if args[1] == "quackle-superleaves" {
            let bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            let mut csv_out = csv::Writer::from_writer(make_writer(&args[3])?);
            let mut i = 0;
            let mut s = String::new();
            while i < bytes.len() {
                let l = bytes[i] as usize;
                if i + l + 3 > bytes.len() {
                    return Err("out of bounds".into());
                }
                s.clear();
                for j in 1..=l {
                    let c = bytes[i + j];
                    if c == 1 {
                        s.push('?');
                    } else if (5..=30).contains(&c) {
                        s.push((c + (b'A' - 5)) as char);
                    } else {
                        return Err("invalid tile".into());
                    }
                }
                i += l + 3;
                csv_out.serialize((
                    &s,
                    (bytes[i - 2] as u16 | ((bytes[i - 1] as u16) << 8)) as f32 * (1.0 / 256.0)
                        - 128.0,
                ))?;
            }
        } else if args[1] == "quackle" {
            let quackle_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            if 20 > quackle_bytes.len() {
                return Err("out of bounds".into());
            }
            let alpha_size = quackle_bytes[20] as usize;
            let mut alpha = Vec::with_capacity(alpha_size);
            let mut p = 21;
            for _ in 0..alpha_size {
                let p0 = p;
                loop {
                    if p > quackle_bytes.len() {
                        return Err("out of bounds".into());
                    }
                    if quackle_bytes[p] == b' ' {
                        alpha.push(std::str::from_utf8(&quackle_bytes[p0..p])?);
                        p += 1;
                        break;
                    }
                    p += 1;
                }
            }
            let reader = &QuackleReader { offset: p };
            if 1 > reader.len(quackle_bytes) {
                return Err("out of bounds".into());
            }
            let mut ret = String::new();
            iter_dawg(
                &QuackleAlphabetLabel { alpha: &alpha },
                reader,
                quackle_bytes,
                1,
                None,
                &mut |s: &str| {
                    ret.push_str(s);
                    ret.push('\n');
                    Ok(())
                },
                &mut default_in,
                &mut default_out,
            )?;
            make_writer(&args[3])?.write_all(ret.as_bytes())?;
        } else if args[1] == "quackle-small" {
            let quackle_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            if 20 > quackle_bytes.len() {
                return Err("out of bounds".into());
            }
            let alpha_size = quackle_bytes[20] as usize;
            let mut alpha = Vec::with_capacity(alpha_size);
            let mut p = 21;
            for _ in 0..alpha_size {
                let p0 = p;
                loop {
                    if p > quackle_bytes.len() {
                        return Err("out of bounds".into());
                    }
                    if quackle_bytes[p] == b' ' {
                        alpha.push(std::str::from_utf8(&quackle_bytes[p0..p])?);
                        p += 1;
                        break;
                    }
                    p += 1;
                }
            }
            let reader = &QuackleSmallReader { offset: p };
            if 1 > reader.len(quackle_bytes) {
                return Err("out of bounds".into());
            }
            let mut ret = String::new();
            iter_dawg(
                &QuackleAlphabetLabel { alpha: &alpha },
                reader,
                quackle_bytes,
                1,
                None,
                &mut |s: &str| {
                    ret.push_str(s);
                    ret.push('\n');
                    Ok(())
                },
                &mut default_in,
                &mut default_out,
            )?;
            make_writer(&args[3])?.write_all(ret.as_bytes())?;
        } else if args[1] == "zyzzyva" {
            let reader = &ZyzzyvaReader {};
            let zyzzyva_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            if 0x8 > zyzzyva_bytes.len() {
                return Err("out of bounds".into());
            }
            let mut ret = String::new();
            iter_dawg(
                &LexpertAlphabetLabel {},
                reader,
                zyzzyva_bytes,
                1,
                None,
                &mut |s: &str| {
                    ret.push_str(s);
                    ret.push('\n');
                    Ok(())
                },
                &mut default_in,
                &mut default_out,
            )?;
            make_writer(&args[3])?.write_all(ret.as_bytes())?;
        } else if args[1] == "lexpert" {
            let reader = &LexpertReader {};
            let lexpert_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
            if 0x4c > lexpert_bytes.len() {
                return Err("out of bounds".into());
            }
            let mut ret = String::new();
            iter_dawg(
                &LexpertAlphabetLabel {},
                reader,
                lexpert_bytes,
                2,
                None,
                &mut |s: &str| {
                    ret.push_str(s);
                    ret.push('\n');
                    Ok(())
                },
                &mut default_in,
                &mut default_out,
            )?;
            make_writer(&args[3])?.write_all(ret.as_bytes())?;
        } else if args[1] == "stats-zt" {
            let mut ret = String::new();
            for ci in [0.8f64, 0.85, 0.9, 0.95, 0.99, 0.995, 0.999] {
                writeln!(
                    ret,
                    "{:4.1}% {}",
                    ci * 100.0,
                    stats::NormalDistribution::reverse_ci(ci)
                )?;
            }
            ret.push('\n');
            let mut cnd = stats::CumulativeNormalDensity::new();
            let mut cumulative_normal_density = |x: f64| cnd.get(x);
            for y in (35..=50).rev().step_by(5) {
                let v = y as f32 * -0.1;
                writeln!(ret, "{:4.1} {}", v, cumulative_normal_density(v.into()))?;
            }
            for y in (0..=34i32).rev() {
                write!(ret, "{:4.1}", y as f32 * -0.1)?;
                for x in 0..=9 {
                    let v = (y * 10 + x) as f32 * -0.01;
                    //write!(ret, " {:5.2}", v)?;
                    write!(ret, " {:6.4}", cumulative_normal_density(v.into()))?;
                }
                ret.push('\n');
            }
            ret.push('\n');
            for y in 0..=34i32 {
                write!(ret, "{:4.1}", y as f32 * 0.1)?;
                for x in 0..=9 {
                    let v = (y * 10 + x) as f32 * 0.01;
                    //write!(ret, " {:5.2}", v)?;
                    write!(ret, " {:6.4}", cumulative_normal_density(v.into()))?;
                }
                ret.push('\n');
            }
            for y in (35..=50).step_by(5) {
                let v = y as f32 * 0.1;
                writeln!(ret, "{:4.1} {}", v, cumulative_normal_density(v.into()))?;
            }
            print!("{}", ret);
        } else {
            return Err("invalid argument".into());
        }
        writeln!(boxed_stdout_or_stderr(), "time taken: {:?}", t0.elapsed())?;
        Ok(())
    }
}
