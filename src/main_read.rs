// Copyright (C) 2020-2024 Andy Kurnia.

use wolges::{alphabet, error, prob};

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
                    | (klv_bytes[r + 1] as u32) << 8
                    | (klv_bytes[r + 2] as u32) << 16
                    | (klv_bytes[r + 3] as u32) << 24)
                    as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = (klv_bytes[r] as u32
                    | (klv_bytes[r + 1] as u32) << 8
                    | (klv_bytes[r + 2] as u32) << 16
                    | (klv_bytes[r + 3] as u32) << 24) as usize;
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
                                        | (klv_bytes[r - 3] as u32) << 8
                                        | (klv_bytes[r - 2] as u32) << 16
                                        | (klv_bytes[r - 1] as u32) << 24,
                                )
                            } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                                r += 2;
                                ((klv_bytes[r - 2] as u16 | (klv_bytes[r - 1] as u16) << 8) as i16)
                                    as f32
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
                    a.0 .1.cmp(&b.0 .1).then_with(|| {
                        b.0 .2
                            .cmp(&a.0 .2)
                            .then_with(|| a.1 .0.cmp(&b.1 .0).then_with(|| a.1 .1.cmp(&b.1 .1)))
                    })
                });
                let mut csv_out = csv::Writer::from_writer(make_writer(&args[3])?);
                let mut last_anagram_key = Vec::new();
                let mut num_sets = 0;
                let mut num_in_set = 0;
                for elt in vec_out {
                    if last_anagram_key != elt.1 .0 {
                        if last_anagram_key.len() != elt.1 .0.len() {
                            num_sets = 1;
                        } else {
                            num_sets += 1;
                        }
                        num_in_set = 0;
                        last_anagram_key.clone_from(&elt.1 .0);
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
                    | (klv_bytes[r + 1] as u32) << 8
                    | (klv_bytes[r + 2] as u32) << 16
                    | (klv_bytes[r + 3] as u32) << 24)
                    as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = (klv_bytes[r] as u32
                    | (klv_bytes[r + 1] as u32) << 8
                    | (klv_bytes[r + 2] as u32) << 16
                    | (klv_bytes[r + 3] as u32) << 24) as usize;
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
                                    | (klv_bytes[r - 3] as u32) << 8
                                    | (klv_bytes[r - 2] as u32) << 16
                                    | (klv_bytes[r - 1] as u32) << 24,
                            )
                        } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                            r += 2;
                            ((klv_bytes[r - 2] as u16 | (klv_bytes[r - 1] as u16) << 8) as i16)
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
                    | (klv_bytes[r + 1] as u32) << 8
                    | (klv_bytes[r + 2] as u32) << 16
                    | (klv_bytes[r + 3] as u32) << 24)
                    as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = (klv_bytes[r] as u32
                    | (klv_bytes[r + 1] as u32) << 8
                    | (klv_bytes[r + 2] as u32) << 16
                    | (klv_bytes[r + 3] as u32) << 24) as usize;
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
                                    | (klv_bytes[r - 3] as u32) << 8
                                    | (klv_bytes[r - 2] as u32) << 16
                                    | (klv_bytes[r - 1] as u32) << 24,
                            )
                        } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                            r += 2;
                            ((klv_bytes[r - 2] as u16 | (klv_bytes[r - 1] as u16) << 8) as i16)
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
                    | (klv_bytes[r + 1] as u32) << 8
                    | (klv_bytes[r + 2] as u32) << 16
                    | (klv_bytes[r + 3] as u32) << 24)
                    as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = (klv_bytes[r] as u32
                    | (klv_bytes[r + 1] as u32) << 8
                    | (klv_bytes[r + 2] as u32) << 16
                    | (klv_bytes[r + 3] as u32) << 24) as usize;
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
                                    | (klv_bytes[r - 3] as u32) << 8
                                    | (klv_bytes[r - 2] as u32) << 16
                                    | (klv_bytes[r - 1] as u32) << 24,
                            )
                        } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                            r += 2;
                            ((klv_bytes[r - 2] as u16 | (klv_bytes[r - 1] as u16) << 8) as i16)
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
                    | (ort_bytes[r + 1] as u32) << 8
                    | (ort_bytes[r + 2] as u32) << 16
                    | (ort_bytes[r + 3] as u32) << 24;
                r += 4;
                let ort_num_values = ort_bytes[r] as u32
                    | (ort_bytes[r + 1] as u32) << 8
                    | (ort_bytes[r + 2] as u32) << 16
                    | (ort_bytes[r + 3] as u32) << 24;
                r += 4;
                if ort_bytes.len() < r + ((ort_num_buckets + 1 + ort_num_values) * 4) as usize {
                    return Err("out of bounds".into());
                }
                let mut ort_buckets = Vec::with_capacity(ort_num_buckets as usize + 1);
                for _ in 0..=ort_num_buckets {
                    ort_buckets.push(
                        ort_bytes[r] as u32
                            | (ort_bytes[r + 1] as u32) << 8
                            | (ort_bytes[r + 2] as u32) << 16
                            | (ort_bytes[r + 3] as u32) << 24,
                    );
                    r += 4;
                }
                let mut ort_values = Vec::with_capacity(ort_num_values as usize);
                for _ in 0..ort_num_values {
                    ort_values.push(
                        ort_bytes[r] as u32
                            | (ort_bytes[r + 1] as u32) << 8
                            | (ort_bytes[r + 2] as u32) << 16
                            | (ort_bytes[r + 3] as u32) << 24,
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
            "-wmp-words" => {
                let alphabet = make_alphabet();
                let alphabet_label = &WolgesAlphabetLabel {
                    alphabet: &alphabet,
                };

                // wmp: olaugh's wordmap from jvc56/MAGPIE.
                let wmp_bytes = &read_to_end(&mut make_reader(&args[2])?)?;
                if wmp_bytes.len() < 12 {
                    return Err("out of bounds".into());
                }

                let mut ret = String::new();

                let min_len = wmp_bytes[2];
                let max_len = wmp_bytes[3];
                let mut r = 12;
                for len in min_len..=max_len {
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_word_buckets = wmp_bytes[r] as u32
                        | (wmp_bytes[r + 1] as u32) << 8
                        | (wmp_bytes[r + 2] as u32) << 16
                        | (wmp_bytes[r + 3] as u32) << 24;
                    r += 4 * (2 + wmp_bylen_num_word_buckets as usize);
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_word_entries = wmp_bytes[r] as u32
                        | (wmp_bytes[r + 1] as u32) << 8
                        | (wmp_bytes[r + 2] as u32) << 16
                        | (wmp_bytes[r + 3] as u32) << 24;
                    r += 4 + 28 * wmp_bylen_num_word_entries as usize;
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_words = wmp_bytes[r] as u32
                        | (wmp_bytes[r + 1] as u32) << 8
                        | (wmp_bytes[r + 2] as u32) << 16
                        | (wmp_bytes[r + 3] as u32) << 24;
                    r += 4;
                    for _ in 0..wmp_bylen_num_words {
                        if wmp_bytes.len() < r + len as usize {
                            return Err("out of bounds".into());
                        }
                        for _ in 0..len {
                            alphabet_label.label(&mut ret, wmp_bytes[r])?;
                            r += 1;
                        }
                        ret.push('\n');
                    }

                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_blank_buckets = wmp_bytes[r] as u32
                        | (wmp_bytes[r + 1] as u32) << 8
                        | (wmp_bytes[r + 2] as u32) << 16
                        | (wmp_bytes[r + 3] as u32) << 24;
                    r += 4 * (2 + wmp_bylen_num_blank_buckets as usize);
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_blank_entries = wmp_bytes[r] as u32
                        | (wmp_bytes[r + 1] as u32) << 8
                        | (wmp_bytes[r + 2] as u32) << 16
                        | (wmp_bytes[r + 3] as u32) << 24;
                    r += 4 + 28 * wmp_bylen_num_blank_entries as usize;

                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_double_blank_buckets = wmp_bytes[r] as u32
                        | (wmp_bytes[r + 1] as u32) << 8
                        | (wmp_bytes[r + 2] as u32) << 16
                        | (wmp_bytes[r + 3] as u32) << 24;
                    r += 4 * (2 + wmp_bylen_num_double_blank_buckets as usize);
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_double_blank_entries = wmp_bytes[r] as u32
                        | (wmp_bytes[r + 1] as u32) << 8
                        | (wmp_bytes[r + 2] as u32) << 16
                        | (wmp_bytes[r + 3] as u32) << 24;
                    r += 4 + 28 * wmp_bylen_num_double_blank_entries as usize;
                    if wmp_bytes.len() < r + 4 {
                        return Err("out of bounds".into());
                    }
                    let wmp_bylen_num_blank_pairs = wmp_bytes[r] as u32
                        | (wmp_bytes[r + 1] as u32) << 8
                        | (wmp_bytes[r + 2] as u32) << 16
                        | (wmp_bytes[r + 3] as u32) << 24;
                    r += 4 + 2 * wmp_bylen_num_blank_pairs as usize;
                }

                if wmp_bytes.len() < r {
                    return Err("out of bounds".into());
                }
                if wmp_bytes.len() != r {
                    return Err("incorrect file size".into());
                }

                make_writer(&args[3])?.write_all(ret.as_bytes())?;
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
            if cache_set.iter().any(|&x| x == prev_cache_line_idx) {
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

fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 {
        println!(
            "args:
  english-klv CSW21.klv CSW21.csv
  english-klv CSW21.klv2 CSW21.csv
    read klv/klv2 file
  english-kwg CSW21.kwg CSW21.txt
  english-kwg CSW21.kad CSW21.txt
    read kwg/kad file (dawg) (use kwg0 to allow 0, such as for klv-kwg-extract)
  english-kwg-gaddag CSW21.kwg CSW21.txt
    read gaddawg kwg file (gaddag)
  english-kwg-nodes CSW21.kwg CSW21.kwg.raw
    read kwg file for human inspection
  english-kwg-prob CSW21.kwg -
    read kwg file (dawg) by probability (output format subject to changes)
  english-prob word [word...]
    show raw probability
  english-klv-anagram- CSW21.klv2 - A?AC
  english-klv-anagram CSW21.klv2 - A?AC
  english-klv-anagram+ CSW21.klv2 - A?AC
    list all leaves with subanagram, anagram, or superanagram
  english-kwg-anagram- CSW21.kwg - A?AC
  english-kwg-anagram CSW21.kwg - A?AC
  english-kwg-anagram+ CSW21.kwg - A?AC
    list all words with subanagram, anagram, or superanagram (using dawg)
  english-kwg-check CSW21.kwg word [word...]
    checks if all words are valid (using dawg)
  english-q2-ort something.ort something.csv
    read .ort (format subject to change)
  english-make-q2-ort something.csv something.ort num_buckets
    generate .ort with the given num_buckets (ideally prime eg 5297687)
  english-wmp-words something.wmp something.txt
    read .wmp words (format subject to change)
  (english can also be catalan, french, german, norwegian, polish, slovene,
    spanish, decimal)
  klv-kwg-extract CSW21.klv2 racks.kwg
    just copy out the kwg for further analysis.
  kwg-hitcheck CSW21.kwg cls csa ncs outfile
    check dawg cache hit rate.
    cls = cache line size, 64 is typical.
    csa = cache set associativity, e.g. 8 for 8-way set associativity.
    ncs = number of cache sets = L1D cache size / cores / csa / cls.
    example for i7-8700B (192 kB, 6 cores, 8-way set assoc): 64 8 64.
  kwg-hitcheck-gaddag CSW21.kwg cls csa ncs outfile
    ditto for gaddag.
  quackle-make-superleaves CSW21.klv superleaves
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
                | (klv_bytes[r + 1] as u32) << 8
                | (klv_bytes[r + 2] as u32) << 16
                | (klv_bytes[r + 3] as u32) << 24) as usize)
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
                | (klv_bytes[r + 1] as u32) << 8
                | (klv_bytes[r + 2] as u32) << 16
                | (klv_bytes[r + 3] as u32) << 24) as usize)
                * 4;
            r += 4;
            if klv_bytes.len() < r + kwg_bytes_len + 4 {
                return Err("out of bounds".into());
            }
            let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
            r += kwg_bytes_len;
            let lv_len = (klv_bytes[r] as u32
                | (klv_bytes[r + 1] as u32) << 8
                | (klv_bytes[r + 2] as u32) << 16
                | (klv_bytes[r + 3] as u32) << 24) as usize;
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
                                | (klv_bytes[r - 3] as u32) << 8
                                | (klv_bytes[r - 2] as u32) << 16
                                | (klv_bytes[r - 1] as u32) << 24,
                        )
                    } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                        r += 2;
                        ((klv_bytes[r - 2] as u16 | (klv_bytes[r - 1] as u16) << 8) as i16) as f32
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
                    (bytes[i - 2] as u16 | (bytes[i - 1] as u16) << 8) as f32 * (1.0 / 256.0)
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
        } else {
            return Err("invalid argument".into());
        }
        writeln!(boxed_stdout_or_stderr(), "time taken: {:?}", t0.elapsed())?;
        Ok(())
    }
}
