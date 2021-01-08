// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

use super::{alphabet, bites, build, error, kwg, lexport};

struct AlphabetReader<'a> {
    supported_tiles: Box<[(u8, &'a str)]>,
}

// This is slow, but supports multi-codepoint tiles with greedy matching.
// For example, a CH/LL/RR tile will parse correctly.
impl<'a> AlphabetReader<'a> {
    // Usually min_index=1. Use min_index=0 to allow blanks.
    fn new(alphabet: &alphabet::Alphabet<'a>, min_index: u8) -> Self {
        // non-blank tiles by first byte (asc), length (desc), and tile (asc).
        let mut supported_tiles = (min_index..alphabet.len())
            .map(|tile| (tile, alphabet.from_rack(tile).unwrap()))
            .collect::<Box<_>>();
        supported_tiles.sort_unstable_by(|(a_tile, a_label), (b_tile, b_label)| {
            a_label.as_bytes()[0]
                .cmp(&b_label.as_bytes()[0])
                .then_with(|| {
                    b_label
                        .len()
                        .cmp(&a_label.len())
                        .then_with(|| a_tile.cmp(b_tile))
                })
        });
        Self { supported_tiles }
    }

    fn read_machine_words(&self, giant_string: &str) -> error::Returns<Box<[bites::Bites]>> {
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
                let seek = sb[ix];
                let first_possible_index = self
                    .supported_tiles
                    .binary_search_by(|(_, probe_label)| {
                        probe_label.as_bytes()[0]
                            .cmp(&seek)
                            .then(std::cmp::Ordering::Greater)
                    })
                    .unwrap_err();
                let mut found = false;
                for (tile, label) in
                    &self.supported_tiles[first_possible_index..self.supported_tiles.len()]
                {
                    if label.as_bytes()[0] != seek {
                        // tiles with the same first byte are clustered together
                        break;
                    }
                    if ix + label.len() <= sb.len() && sb[ix..ix + label.len()] == *label.as_bytes()
                    {
                        found = true;
                        ix += label.len();
                        v.push(*tile);
                        break;
                    }
                }
                if !found {
                    return_error!(format!("invalid tile after {:?} in {:?}", v, s));
                }
            }
            machine_words.push(v[..].into());
        }
        machine_words.sort_unstable();
        machine_words.dedup();
        Ok(machine_words.into_boxed_slice())
    }
}

// This is rarely used, so it allocates a single-use AlphabetReader.
fn read_polish_machine_words(giant_string: &str) -> error::Returns<Box<[bites::Bites]>> {
    AlphabetReader::new(&alphabet::make_polish_alphabet(), 1).read_machine_words(giant_string)
}

// This is a much faster replacement of
// AlphabetReader::new(&alphabet::make_english_alphabet(), 0).read_machine_words(giant_string)
// and requires a clean, pre-sorted input.
fn read_english_machine_words_or_leaves(
    blank: char,
    giant_string: &str,
) -> error::Returns<Box<[bites::Bites]>> {
    // Memory wastage notes:
    // - Vec of 270k words have size 512k because vec grows by doubling.
    // - Size of vec is 24 bytes. Size of slice would have been 16 bytes.
    // - Each vec is individually allocated. We could instead join them all.
    // - We do not do this, because that O(n) already gives build().

    let mut machine_words = Vec::<bites::Bites>::new();
    for s in giant_string.lines() {
        let mut v = Vec::with_capacity(s.len());
        // This is English-only, and will need adjustment for multibyte.
        // The output must be 1-based because 0 has special meaning.
        // It should also not be too high to fit in a u64 cross-set.
        for c in s.chars() {
            if c >= 'A' && c <= 'Z' {
                v.push((c as u8) & 0x3f);
            } else if c == blank {
                // Test this after the letters. Pass a letter to disable.
                v.push(0);
            } else {
                return_error!(format!("invalid tile after {:?} in {:?}", v, s));
            }
        }
        // Performance notes:
        // - .last() is slow.
        // - But the borrow checker does not like raw pointer.
        match machine_words.last() {
            Some(previous_v) => {
                if v[..] <= previous_v[..] {
                    return_error!(format!(
                        "input is not sorted, {:?} cannot come after {:?}",
                        v, previous_v
                    ));
                }
            }
            None => {
                if v.is_empty() {
                    return_error!("first line is blank".into());
                }
            }
        };
        machine_words.push(v[..].into());
    }
    Ok(machine_words.into_boxed_slice())
}

#[inline(always)]
fn read_english_leaves_machine_words(giant_string: &str) -> error::Returns<Box<[bites::Bites]>> {
    read_english_machine_words_or_leaves('?', giant_string)
}

#[inline(always)]
fn read_english_machine_words(giant_string: &str) -> error::Returns<Box<[bites::Bites]>> {
    read_english_machine_words_or_leaves('A', giant_string)
}

use std::str::FromStr;

pub fn main() -> error::Returns<()> {
    let f = std::fs::File::open("leaves.csv")?;
    let mut leave_values = Vec::new();
    // extern crate csv;
    let mut csv_reader = csv::ReaderBuilder::new().has_headers(false).from_reader(f);
    for result in csv_reader.records() {
        let record = result?;
        let rounded_leave = (f32::from_str(&record[1])? * 256.0).round();
        let int_leave = rounded_leave as i16;
        assert!((int_leave as f32 - rounded_leave).abs() == 0.0);
        leave_values.push((String::from(&record[0]), int_leave));
    }
    leave_values.sort_unstable_by(|(s1, _), (s2, _)| s1.cmp(s2));
    let leaves_kwg = build::build(
        build::BuildFormat::DawgOnly,
        &read_english_leaves_machine_words(&leave_values.iter().fold(
            String::new(),
            |mut acc, (s, _)| {
                acc.push_str(s);
                acc.push('\n');
                acc
            },
        ))?,
    )?;
    let mut bin = vec![0; 2 * 4 + leaves_kwg.len() + leave_values.len() * 2];
    let mut w = 0;
    bin[w..w + 4].copy_from_slice(&((leaves_kwg.len() / 4) as u32).to_le_bytes());
    w += 4;
    bin[w..w + leaves_kwg.len()].copy_from_slice(&leaves_kwg);
    w += leaves_kwg.len();
    bin[w..w + 4].copy_from_slice(&(leave_values.len() as u32).to_le_bytes());
    w += 4;
    for (_, v) in leave_values {
        bin[w..w + 2].copy_from_slice(&v.to_le_bytes());
        w += 2;
    }
    assert_eq!(w, bin.len());
    std::fs::write("leaves.klv", bin)?;
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "csw19.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words(&std::fs::read_to_string("csw19.txt")?)?,
            )?,
        )?;
        println!("{:?} for reading+building+writing csw19 kwg", t0.elapsed());
    }
    std::fs::write(
        "ecwl.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("ecwl.txt")?)?,
        )?,
    )?;
    std::fs::write(
        "nwl18.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("nwl18.txt")?)?,
        )?,
    )?;
    std::fs::write(
        "nwl20.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("nwl20.txt")?)?,
        )?,
    )?;
    if true {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "osps42-dawg.kwg",
            build::build(
                build::BuildFormat::DawgOnly,
                &read_polish_machine_words(&std::fs::read_to_string("osps42.txt")?)?,
            )?,
        )?;
        println!(
            "{:?} for reading+building+writing polish dawgonly",
            t0.elapsed()
        );
    }
    if true {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "osps42.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_polish_machine_words(&std::fs::read_to_string("osps42.txt")?)?,
            )?,
        )?;
        println!(
            "{:?} for reading+building+writing polish gaddawg",
            t0.elapsed()
        );
    }
    std::fs::write(
        "twl14.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("twl14.txt")?)?,
        )?,
    )?;

    if true {
        let english_alphabet = alphabet::make_english_alphabet();
        let polish_alphabet = alphabet::make_polish_alphabet();
        let t0 = std::time::Instant::now();
        {
            let t0 = std::time::Instant::now();
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("csw19.kwg")?);
            println!("{:?} for rereading csw19.kwg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "CSW19.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "CSW19",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            println!("{:?} for exporting csw19 dawg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "CSW19.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "CSW19",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
            println!("{:?} for exporting csw19 gaddag", t0.elapsed());
        }
        {
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("nwl18.kwg")?);
            std::fs::write(
                "NWL18.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL18",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            std::fs::write(
                "NWL18.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL18",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
        }
        {
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("nwl20.kwg")?);
            std::fs::write(
                "NWL20.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL20",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            std::fs::write(
                "NWL20.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL20",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
        }
        {
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("ecwl.kwg")?);
            std::fs::write(
                "ECWL.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "ECWL",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            std::fs::write(
                "ECWL.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "ECWL",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
        }
        println!("{:?} for exporting many files", t0.elapsed());
        if true {
            let t0 = std::time::Instant::now();
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("osps42.kwg")?);
            println!("{:?} for rereading osps42.kwg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "OSPS42.dawg",
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    "OSPS42",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            println!("{:?} for exporting osps42 dawg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "OSPS42.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    "OSPS42",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
            println!("{:?} for exporting osps42 gaddag", t0.elapsed());
        }
    }

    if true {
        // this reads the files again, but this code is temporary
        let v_csw19 = read_english_machine_words(&std::fs::read_to_string("csw19.txt")?)?;
        let v_ecwl = read_english_machine_words(&std::fs::read_to_string("ecwl.txt")?)?;
        let v_nwl18 = read_english_machine_words(&std::fs::read_to_string("nwl18.txt")?)?;
        let v_nwl20 = read_english_machine_words(&std::fs::read_to_string("nwl20.txt")?)?;
        let v_twl14 = read_english_machine_words(&std::fs::read_to_string("twl14.txt")?)?;
        let mut v = Vec::<bites::Bites>::new();
        v.extend_from_slice(&v_csw19);
        v.extend_from_slice(&v_ecwl);
        v.extend_from_slice(&v_nwl18);
        v.extend_from_slice(&v_nwl20);
        v.extend_from_slice(&v_twl14);
        v.sort_unstable();
        v.dedup();
        let v = v.into_boxed_slice();
        let v_bits_bytes = (v.len() + 7) / 8;
        let mut v_csw19_bits = vec![0u8; v_bits_bytes];
        let mut v_ecwl_bits = vec![0u8; v_bits_bytes];
        let mut v_nwl18_bits = vec![0u8; v_bits_bytes];
        let mut v_nwl20_bits = vec![0u8; v_bits_bytes];
        let mut v_twl14_bits = vec![0u8; v_bits_bytes];
        let mut p_csw19 = v_csw19.len();
        let mut p_ecwl = v_ecwl.len();
        let mut p_nwl18 = v_nwl18.len();
        let mut p_nwl20 = v_nwl20.len();
        let mut p_twl14 = v_twl14.len();
        for i in (0..v.len()).rev() {
            if p_csw19 > 0 && v[i] == v_csw19[p_csw19 - 1] {
                v_csw19_bits[i / 8] |= 1 << (i % 8);
                p_csw19 -= 1;
            }
            if p_ecwl > 0 && v[i] == v_ecwl[p_ecwl - 1] {
                v_ecwl_bits[i / 8] |= 1 << (i % 8);
                p_ecwl -= 1;
            }
            if p_nwl18 > 0 && v[i] == v_nwl18[p_nwl18 - 1] {
                v_nwl18_bits[i / 8] |= 1 << (i % 8);
                p_nwl18 -= 1;
            }
            if p_nwl20 > 0 && v[i] == v_nwl20[p_nwl20 - 1] {
                v_nwl20_bits[i / 8] |= 1 << (i % 8);
                p_nwl20 -= 1;
            }
            if p_twl14 > 0 && v[i] == v_twl14[p_twl14 - 1] {
                v_twl14_bits[i / 8] |= 1 << (i % 8);
                p_twl14 -= 1;
            }
        }
        std::fs::write("allgdw.kwg", build::build(build::BuildFormat::Gaddawg, &v)?)?;
        std::fs::write("all-csw19.kwi", v_csw19_bits)?;
        std::fs::write("all-ecwl.kwi", v_ecwl_bits)?;
        std::fs::write("all-nwl18.kwi", v_nwl18_bits)?;
        std::fs::write("all-nwl20.kwi", v_nwl20_bits)?;
        std::fs::write("all-twl14.kwi", v_twl14_bits)?;
    }

    if false {
        // proof-of-concept
        let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("allgdw.kwg")?);
        let word_counts = kwg.count_dawg_words_alloc();
        // because dawg do not need gaddag nodes
        println!("only counting {} nodes", word_counts.len());
        let v_csw19_bits = std::fs::read("all-csw19.kwi")?;
        let v_ecwl_bits = std::fs::read("all-ecwl.kwi")?;
        let v_nwl18_bits = std::fs::read("all-nwl18.kwi")?;
        let v_nwl20_bits = std::fs::read("all-nwl20.kwi")?;
        let v_twl14_bits = std::fs::read("all-twl14.kwi")?;
        let mut out_vec = Vec::new();
        let dawg_root = kwg[0].arc_index();
        for i in 0..word_counts[dawg_root as usize] {
            out_vec.clear();
            kwg.get_word_by_index(&word_counts, dawg_root, i, |v| {
                out_vec.push(v);
            });
            let j = kwg.get_word_index(&word_counts, dawg_root, &out_vec);
            print!("{} {} {:?}", i, j, out_vec);
            let byte_index = j as usize / 8;
            let bit = 1 << (j as usize % 8);
            if v_csw19_bits[byte_index] & bit != 0 {
                print!(" csw19");
            }
            if v_ecwl_bits[byte_index] & bit != 0 {
                print!(" ecwl");
            }
            if v_nwl18_bits[byte_index] & bit != 0 {
                print!(" nwl18");
            }
            if v_nwl20_bits[byte_index] & bit != 0 {
                print!(" nwl20");
            }
            if v_twl14_bits[byte_index] & bit != 0 {
                print!(" twl14");
            }
            println!();
            assert_eq!(i, j);
        }
    }

    std::fs::write(
        "volost.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words("VOLOST\nVOLOSTS")?,
        )?,
    )?;
    std::fs::write(
        "empty.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words("")?,
        )?,
    )?;

    Ok(())
}
