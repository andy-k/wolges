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
                let kwg_bytes_len = (u32::from_le(
                    klv_bytes[r] as u32
                        | (klv_bytes[r + 1] as u32) << 8
                        | (klv_bytes[r + 2] as u32) << 16
                        | (klv_bytes[r + 3] as u32) << 24,
                ) as usize)
                    * 4;
                r += 4;
                if klv_bytes.len() < r + kwg_bytes_len + 4 {
                    return Err("out of bounds".into());
                }
                let kwg_bytes = &klv_bytes[r..r + kwg_bytes_len];
                r += kwg_bytes_len;
                let lv_len = u32::from_le(
                    klv_bytes[r] as u32
                        | (klv_bytes[r + 1] as u32) << 8
                        | (klv_bytes[r + 2] as u32) << 16
                        | (klv_bytes[r + 3] as u32) << 24,
                ) as usize;
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
                                f32::from_bits(u32::from_le(
                                    klv_bytes[r - 4] as u32
                                        | (klv_bytes[r - 3] as u32) << 8
                                        | (klv_bytes[r - 2] as u32) << 16
                                        | (klv_bytes[r - 1] as u32) << 24,
                                ))
                            } else if !is_klv2 && klv_bytes.len() >= r + 2 {
                                r += 2;
                                i16::from_le(
                                    (klv_bytes[r - 2] as u16 | (klv_bytes[r - 1] as u16) << 8)
                                        as i16,
                                ) as f32
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
  (english can also be catalan, french, german, norwegian, polish, spanish)"
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
        } else {
            return Err("invalid argument".into());
        }
        println!("time taken: {:?}", t0.elapsed());
        Ok(())
    }
}
