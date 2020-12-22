use super::{bites, build, error, kwg};

fn read_english_machine_words(giant_string: &str) -> error::Returns<Box<[bites::Bites]>> {
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
            } else if c == '?' {
                v.push(0); // temp hack
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
        &read_english_machine_words(&leave_values.iter().fold(
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
    std::fs::write(
        "csw19.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("csw19.txt")?)?,
        )?,
    )?;
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
    std::fs::write(
        "twl14.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_english_machine_words(&std::fs::read_to_string("twl14.txt")?)?,
        )?,
    )?;

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
        println!("num dedup: {}", v.len());
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
