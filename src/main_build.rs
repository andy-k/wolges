// Copyright (C) 2020-2021 Andy Kurnia.

use wolges::{alphabet, bites, build, error, kwg, lexport, prob};

fn read_machine_words(
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
            } else {
                wolges::return_error!(format!("invalid tile after {:?} in {:?}", v, s));
            }
        }
        machine_words.push(v[..].into());
    }
    machine_words.sort_unstable();
    machine_words.dedup();
    Ok(machine_words.into_boxed_slice())
}

// This is rarely used, so it allocates a single-use AlphabetReader.
fn read_polish_machine_words(giant_string: &str) -> error::Returns<Box<[bites::Bites]>> {
    read_machine_words(
        &alphabet::AlphabetReader::new_for_words(&alphabet::make_polish_alphabet()),
        giant_string,
    )
}

// This is a much faster replacement of
// read_machine_words(
//     &alphabet::AlphabetReader::new_for_racks(&alphabet::make_english_alphabet()),
//     giant_string,
// )
// and requires a clean, pre-sorted input.
fn read_english_machine_words_or_leaves(
    blank: char,
    giant_string: &str,
) -> error::Returns<Box<[bites::Bites]>> {
    let mut machine_words = Vec::<bites::Bites>::new();
    let mut v = Vec::new();
    for s in giant_string.lines() {
        v.clear();
        v.reserve(s.len());
        // This is English-only, and will need adjustment for multibyte.
        // The output must be 1-based because 0 has special meaning.
        // It should also not be too high to fit in a u64 cross-set.
        for c in s.chars() {
            if ('A'..='Z').contains(&c) {
                v.push((c as u8) & 0x3f);
            } else if c == blank {
                // Test this after the letters. Pass a letter to disable.
                v.push(0);
            } else {
                wolges::return_error!(format!("invalid tile after {:?} in {:?}", v, s));
            }
        }
        // Performance notes:
        // - .last() is slow.
        // - But the borrow checker does not like raw pointer.
        match machine_words.last() {
            Some(previous_v) => {
                if v[..] <= previous_v[..] {
                    wolges::return_error!(format!(
                        "input is not sorted, {:?} cannot come after {:?}",
                        v, previous_v
                    ));
                }
            }
            None => {
                if v.is_empty() {
                    wolges::return_error!("first line is blank".into());
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

use std::convert::TryInto;
use std::str::FromStr;

fn build_english_leaves(f: Box<dyn std::io::Read>) -> error::Returns<Vec<u8>> {
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
    Ok(bin)
}

pub fn main() -> error::Returns<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 {
        old_main()
    } else {
        let t0 = std::time::Instant::now();
        if args[1] == "english-klv" {
            std::fs::write(
                &args[3],
                build_english_leaves(Box::new(std::fs::File::open(&args[2])?))?,
            )?;
        } else if args[1] == "english-kwg" {
            std::fs::write(
                &args[3],
                build::build(
                    build::BuildFormat::Gaddawg,
                    &read_english_machine_words(&std::fs::read_to_string(&args[2])?)?,
                )?,
            )?;
        } else if args[1] == "english-kwg-dawg" {
            std::fs::write(
                &args[3],
                build::build(
                    build::BuildFormat::DawgOnly,
                    &read_english_machine_words(&std::fs::read_to_string(&args[2])?)?,
                )?,
            )?;
        } else if args[1] == "english-kwg-alpha" {
            std::fs::write(
                &args[3],
                build::build(
                    build::BuildFormat::AlphaDawg,
                    &read_english_machine_words(&std::fs::read_to_string(&args[2])?)?,
                )?,
            )?;
        } else if args[1] == "english-macondo" {
            let english_alphabet = alphabet::make_english_alphabet();
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read(&args[2])?);
            std::fs::write(
                &args[4],
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    &args[3],
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            std::fs::write(
                &args[5],
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    &args[3],
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
        } else if args[1] == "polish-kwg" {
            std::fs::write(
                &args[3],
                build::build(
                    build::BuildFormat::Gaddawg,
                    &read_polish_machine_words(&std::fs::read_to_string(&args[2])?)?,
                )?,
            )?;
        } else if args[1] == "polish-kwg-dawg" {
            std::fs::write(
                &args[3],
                build::build(
                    build::BuildFormat::DawgOnly,
                    &read_polish_machine_words(&std::fs::read_to_string(&args[2])?)?,
                )?,
            )?;
        } else if args[1] == "polish-kwg-alpha" {
            std::fs::write(
                &args[3],
                build::build(
                    build::BuildFormat::AlphaDawg,
                    &read_polish_machine_words(&std::fs::read_to_string(&args[2])?)?,
                )?,
            )?;
        } else if args[1] == "polish-macondo" {
            let polish_alphabet = alphabet::make_polish_alphabet();
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read(&args[2])?);
            std::fs::write(
                &args[4],
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    &args[3],
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            std::fs::write(
                &args[5],
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    &args[3],
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
        } else {
            return Err("invalid argument".into());
        }
        println!("time taken: {:?}", t0.elapsed());
        Ok(())
    }
}

fn old_main() -> error::Returns<()> {
    std::fs::write(
        "lexbin/leaves.klv",
        build_english_leaves(Box::new(std::fs::File::open("lexsrc/leaves.csv")?))?,
    )?;
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/CSW19.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_english_machine_words(&std::fs::read_to_string("lexsrc/CSW19.txt")?)?,
            )?,
        )?;
        println!("{:?} for reading+building+writing CSW19 kwg", t0.elapsed());
    }
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/CSW19.kad",
            build::build(
                build::BuildFormat::AlphaDawg,
                &read_english_machine_words(&std::fs::read_to_string("lexsrc/CSW19.txt")?)?,
            )?,
        )?;
        println!(
            "{:?} for reading+building+writing CSW19 alpha dawg",
            t0.elapsed()
        );
    }
    std::fs::write(
        "lexbin/ECWL.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("lexsrc/ECWL.txt")?)?,
        )?,
    )?;
    std::fs::write(
        "lexbin/NWL18.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("lexsrc/NWL18.txt")?)?,
        )?,
    )?;
    std::fs::write(
        "lexbin/NWL20.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("lexsrc/NWL20.txt")?)?,
        )?,
    )?;
    if true {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/OSPS42-dawg.kwg",
            build::build(
                build::BuildFormat::DawgOnly,
                &read_polish_machine_words(&std::fs::read_to_string("lexsrc/OSPS42.txt")?)?,
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
            "lexbin/OSPS42.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_polish_machine_words(&std::fs::read_to_string("lexsrc/OSPS42.txt")?)?,
            )?,
        )?;
        println!(
            "{:?} for reading+building+writing polish gaddawg",
            t0.elapsed()
        );
    }
    std::fs::write(
        "lexbin/TWL14.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("lexsrc/TWL14.txt")?)?,
        )?,
    )?;

    if true {
        let english_alphabet = alphabet::make_english_alphabet();
        let polish_alphabet = alphabet::make_polish_alphabet();
        let t0 = std::time::Instant::now();
        {
            let t0 = std::time::Instant::now();
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW19.kwg")?);
            println!("{:?} for rereading CSW19.kwg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/CSW19.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "CSW19",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            println!("{:?} for exporting CSW19 dawg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/CSW19.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "CSW19",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
            println!("{:?} for exporting CSW19 gaddag", t0.elapsed());
        }
        {
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL18.kwg")?);
            std::fs::write(
                "lexbin/NWL18.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL18",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            std::fs::write(
                "lexbin/NWL18.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL18",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
        }
        {
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL20.kwg")?);
            std::fs::write(
                "lexbin/NWL20.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL20",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            std::fs::write(
                "lexbin/NWL20.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL20",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
        }
        {
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/ECWL.kwg")?);
            std::fs::write(
                "lexbin/ECWL.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "ECWL",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            std::fs::write(
                "lexbin/ECWL.gaddag",
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
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS42.kwg")?);
            println!("{:?} for rereading OSPS42.kwg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/OSPS42.dawg",
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    "OSPS42",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            println!("{:?} for exporting OSPS42 dawg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/OSPS42.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    "OSPS42",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
            println!("{:?} for exporting OSPS42 gaddag", t0.elapsed());
        }
    }

    if true {
        // this reads the files again, but this code is temporary
        let v_csw19 = read_english_machine_words(&std::fs::read_to_string("lexsrc/CSW19.txt")?)?;
        let v_ecwl = read_english_machine_words(&std::fs::read_to_string("lexsrc/ECWL.txt")?)?;
        let v_nwl18 = read_english_machine_words(&std::fs::read_to_string("lexsrc/NWL18.txt")?)?;
        let v_nwl20 = read_english_machine_words(&std::fs::read_to_string("lexsrc/NWL20.txt")?)?;
        let v_twl14 = read_english_machine_words(&std::fs::read_to_string("lexsrc/TWL14.txt")?)?;
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
        std::fs::write(
            "lexbin/allgdw.kwg",
            build::build(build::BuildFormat::Gaddawg, &v)?,
        )?;
        std::fs::write("lexbin/all-CSW19.kwi", v_csw19_bits)?;
        std::fs::write("lexbin/all-ECWL.kwi", v_ecwl_bits)?;
        std::fs::write("lexbin/all-NWL18.kwi", v_nwl18_bits)?;
        std::fs::write("lexbin/all-NWL20.kwi", v_nwl20_bits)?;
        std::fs::write("lexbin/all-TWL14.kwi", v_twl14_bits)?;

        let english_alphabet = alphabet::make_english_alphabet();
        let mut word_prob = prob::WordProbability::new(&english_alphabet);
        let mut max_len = 0;
        let mut tmp_vec = Vec::new();
        let for_sorting = v
            .iter()
            .map(|word| {
                tmp_vec.clear();
                tmp_vec.extend_from_slice(word);
                tmp_vec.sort_unstable();
                let alphagram: bites::Bites = tmp_vec[..].into();
                max_len = std::cmp::max(max_len, v.len());
                (alphagram, word_prob.count_ways(word))
            })
            .collect::<Box<_>>();
        let mut iter_indexes = (0u32..v.len() as u32).collect::<Box<_>>();
        // sort by probability descending, then by alphagram ascending,
        // then by raw index (v is already sorted)
        iter_indexes.sort_unstable_by(|&a_idx, &b_idx| {
            for_sorting[b_idx as usize]
                .1
                .cmp(&for_sorting[a_idx as usize].1)
                .then_with(|| {
                    for_sorting[a_idx as usize]
                        .0
                        .cmp(&for_sorting[b_idx as usize].0)
                        .then_with(|| a_idx.cmp(&b_idx))
                })
        });
        // assign probability indexes by length
        // 32-bit may be overkill, no length has more than 64k words yet
        let mut assigned_indexes = vec![0u32; max_len + 1];
        let mut output_probability_indexes = vec![0u32; v.len()];
        for &idx in iter_indexes.iter() {
            let len = v[idx as usize].len();
            assigned_indexes[len] += 1;
            output_probability_indexes[idx as usize] = assigned_indexes[len];
            //println!(
            //    "[{}] {:?} (len={} alpha={:?} wp={}) index={}",
            //    idx,
            //    v[idx as usize],
            //    len,
            //    for_sorting[idx as usize].0,
            //    for_sorting[idx as usize].1,
            //    output_probability_indexes[idx as usize]
            //);
        }
        let mut v_probability_indexes = vec![0u8; output_probability_indexes.len() * 4];
        let mut w = 0;
        for val in output_probability_indexes {
            v_probability_indexes[w..w + 4].copy_from_slice(&val.to_le_bytes());
            w += 4;
        }
        std::fs::write("lexbin/all-probidx.kwp", v_probability_indexes)?;
    }

    if true {
        // proof-of-concept
        let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/allgdw.kwg")?);
        let word_counts = kwg.count_dawg_words_alloc();
        // because dawg do not need gaddag nodes
        println!("only counting {} nodes", word_counts.len());
        let v_csw19_bits = std::fs::read("lexbin/all-CSW19.kwi")?;
        let v_ecwl_bits = std::fs::read("lexbin/all-ECWL.kwi")?;
        let v_nwl18_bits = std::fs::read("lexbin/all-NWL18.kwi")?;
        let v_nwl20_bits = std::fs::read("lexbin/all-NWL20.kwi")?;
        let v_twl14_bits = std::fs::read("lexbin/all-TWL14.kwi")?;
        let v_probability_indexes = std::fs::read("lexbin/all-probidx.kwp")?;
        let mut out_vec = Vec::new();
        let dawg_root = kwg[0].arc_index();
        let english_alphabet = alphabet::make_english_alphabet();
        let mut word_prob = prob::WordProbability::new(&english_alphabet);
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
                print!(" CSW19");
            }
            if v_ecwl_bits[byte_index] & bit != 0 {
                print!(" ECWL");
            }
            if v_nwl18_bits[byte_index] & bit != 0 {
                print!(" NWL18");
            }
            if v_nwl20_bits[byte_index] & bit != 0 {
                print!(" NWL20");
            }
            if v_twl14_bits[byte_index] & bit != 0 {
                print!(" TWL14");
            }
            print!(" wp={}", word_prob.count_ways(&out_vec));
            print!(
                " pi={}",
                u32::from_le_bytes(
                    v_probability_indexes[i as usize * 4..i as usize * 4 + 4]
                        .try_into()
                        .unwrap()
                )
            );
            println!();
            assert_eq!(i, j);
        }
    }

    std::fs::write(
        "lexbin/volost.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words("VOLOST\nVOLOSTS")?,
        )?,
    )?;
    std::fs::write(
        "lexbin/empty.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words("")?,
        )?,
    )?;

    Ok(())
}
