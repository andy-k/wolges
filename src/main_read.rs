// Copyright (C) 2020-2023 Andy Kurnia.

use wolges::{alphabet, error};

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
}

struct WolgesAlphabetLabel<'a> {
    alphabet: &'a alphabet::Alphabet<'a>,
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

fn iter_dawg<F: FnMut(&str) -> error::Returns<()>, A: AlphabetLabel, R: WgReader>(
    a: &A,
    r: &R,
    b: &[u8],
    initial_idx: usize,
    blank_str: Option<&str>,
    accepts: &mut F,
) -> error::Returns<()> {
    struct Env<'a, F: FnMut(&str) -> error::Returns<()>, A: AlphabetLabel, R: WgReader> {
        a: &'a A,
        r: &'a R,
        b: &'a [u8],
        s: &'a mut String,
        blank_str: Option<&'a str>,
        accepts: &'a mut F,
    }
    fn iter<F: FnMut(&str) -> error::Returns<()>, A: AlphabetLabel, R: WgReader>(
        env: &mut Env<'_, F, A, R>,
        mut p: usize,
    ) -> error::Returns<()> {
        let l = env.s.len();
        loop {
            if p >= env.r.len(env.b) {
                return Err("out of bounds".into());
            }
            let t = env.r.tile(env.b, p);
            if t == 0 {
                env.s.push_str(env.blank_str.ok_or("invalid tile")?);
            } else if t & 0x80 == 0 {
                env.a.label(env.s, t)?;
            } else {
                return Err("invalid tile".into());
            }
            if env.r.accepts(env.b, p) {
                (env.accepts)(env.s)?;
            }
            if env.r.arc_index(env.b, p) != 0 {
                iter(env, env.r.arc_index(env.b, p))?;
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
        },
        initial_idx,
    )
}

fn do_lang<'a, AlphabetMaker: Fn() -> alphabet::Alphabet<'a>>(
    args: &[String],
    language_name: &str,
    make_alphabet: AlphabetMaker,
) -> error::Returns<bool> {
    match args[1].strip_prefix(language_name) {
        Some(args1_suffix) => match args1_suffix {
            "-klv" => {
                let alphabet = make_alphabet();
                let reader = &KwgReader {};
                let klv_bytes = &std::fs::read(&args[2])?;
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
                let mut csv_out = csv::Writer::from_path(&args[3])?;
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
                )?;
                if r != klv_bytes.len() {
                    return Err("too many leaves".into());
                }
                Ok(true)
            }
            "-kwg" => {
                let alphabet = make_alphabet();
                let reader = &KwgReader {};
                let kwg_bytes = &std::fs::read(&args[2])?;
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
                )?;
                std::fs::write(&args[3], ret)?;
                Ok(true)
            }
            "-kwg-gaddag" => {
                let alphabet = make_alphabet();
                let reader = &KwgReader {};
                let kwg_bytes = &std::fs::read(&args[2])?;
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
                )?;
                std::fs::write(&args[3], ret)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        None => Ok(false),
    }
}

fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 {
        println!(
            "args:
  english-klv english.klv english.csv
  english-klv english.klv2 english.csv
    read klv/klv2 file
  english-kwg CSW21.kwg CSW21.txt
  english-kwg CSW21.kad CSW21.txt
    read kwg/kad file (dawg)
  english-kwg-gaddag CSW21.kwg CSW21.txt
    read gaddawg kwg file (gaddag)
  (english can also be catalan, french, german, norwegian, polish, spanish)
  quackle-make-superleaves english.klv superleaves
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
    read lexpert dawg"
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
            || do_lang(&args, "spanish", alphabet::make_spanish_alphabet)?
        {
        } else if args[1] == "quackle-make-superleaves" {
            let reader = &KwgReader {};
            let klv_bytes = &std::fs::read(&args[2])?;
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
            )?;
            if r != klv_bytes.len() {
                return Err("too many leaves".into());
            }
            std::fs::write(&args[3], ret)?;
        } else if args[1] == "quackle-superleaves" {
            let bytes = &std::fs::read(&args[2])?;
            let mut csv_out = csv::Writer::from_path(&args[3])?;
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
            let quackle_bytes = &std::fs::read(&args[2])?;
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
            )?;
            std::fs::write(&args[3], ret)?;
        } else if args[1] == "quackle-small" {
            let quackle_bytes = &std::fs::read(&args[2])?;
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
            )?;
            std::fs::write(&args[3], ret)?;
        } else if args[1] == "zyzzyva" {
            let reader = &ZyzzyvaReader {};
            let zyzzyva_bytes = &std::fs::read(&args[2])?;
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
            )?;
            std::fs::write(&args[3], ret)?;
        } else if args[1] == "lexpert" {
            let reader = &LexpertReader {};
            let lexpert_bytes = &std::fs::read(&args[2])?;
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
            )?;
            std::fs::write(&args[3], ret)?;
        } else {
            return Err("invalid argument".into());
        }
        println!("time taken: {:?}", t0.elapsed());
        Ok(())
    }
}
