// Copyright (C) 2020-2024 Andy Kurnia.

use wolges::{alphabet, bites, build, error, fash, kwg, lexport, prob};

fn read_machine_words(
    alphabet_reader: &alphabet::AlphabetReader<'_>,
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
    machine_words.sort_unstable();
    machine_words.dedup();
    Ok(machine_words.into_boxed_slice())
}

use std::str::FromStr;

fn build_leaves<Readable: std::io::Read>(
    f: Readable,
    alph: alphabet::Alphabet,
) -> error::Returns<Vec<u8>> {
    let alphabet_reader = alphabet::AlphabetReader::new_for_racks(&alph);
    let mut leaves_map: fash::MyHashMap<bites::Bites, _> = fash::MyHashMap::default();
    let mut csv_reader = csv::ReaderBuilder::new().has_headers(false).from_reader(f);
    let mut v = Vec::new();
    for result in csv_reader.records() {
        let record = result?;
        alphabet_reader.set_word(&record[0], &mut v)?;
        v.sort_unstable();
        let rounded_leave = (f32::from_str(&record[1])? * 256.0).round();
        let int_leave = rounded_leave as i16;
        assert!(
            (int_leave as f32 - rounded_leave).abs() == 0.0,
            "for {}: {} (f32) {} (*256) {} (round) {} (int) {} (float) {} (-) {} (abs) {}",
            &record[0],
            &record[1],
            f32::from_str(&record[1])?,
            f32::from_str(&record[1])? * 256.0,
            rounded_leave,
            int_leave,
            int_leave as f32,
            int_leave as f32 - rounded_leave,
            (int_leave as f32 - rounded_leave).abs(),
        );
        if leaves_map.insert(v[..].into(), int_leave).is_some() {
            wolges::return_error!(format!("duplicate record {}", &record[0]));
        }
    }
    let mut sorted_machine_words = leaves_map.keys().cloned().collect::<Box<_>>();
    sorted_machine_words.sort_unstable();
    let leaves_kwg = build::build(build::BuildFormat::DawgOnly, &sorted_machine_words)?;
    let leave_values = sorted_machine_words
        .iter()
        .map(|s| leaves_map[s])
        .collect::<Box<_>>();
    drop(sorted_machine_words);
    drop(leaves_map);
    let mut bin = vec![0; 2 * 4 + leaves_kwg.len() + leave_values.len() * 2];
    let mut w = 0;
    bin[w..w + 4].copy_from_slice(&((leaves_kwg.len() / 4) as u32).to_le_bytes());
    w += 4;
    bin[w..w + leaves_kwg.len()].copy_from_slice(&leaves_kwg);
    w += leaves_kwg.len();
    bin[w..w + 4].copy_from_slice(&(leave_values.len() as u32).to_le_bytes());
    w += 4;
    for v in &leave_values[..] {
        bin[w..w + 2].copy_from_slice(&v.to_le_bytes());
        w += 2;
    }
    assert_eq!(w, bin.len());
    Ok(bin)
}

fn build_leaves_f32<Readable: std::io::Read>(
    f: Readable,
    alph: alphabet::Alphabet,
) -> error::Returns<Vec<u8>> {
    let alphabet_reader = alphabet::AlphabetReader::new_for_racks(&alph);
    let mut leaves_map = fash::MyHashMap::default();
    let mut csv_reader = csv::ReaderBuilder::new().has_headers(false).from_reader(f);
    let mut v = Vec::new();
    for result in csv_reader.records() {
        let record = result?;
        alphabet_reader.set_word(&record[0], &mut v)?;
        v.sort_unstable();
        let float_leave = f32::from_str(&record[1])?;
        if leaves_map.insert(v[..].into(), float_leave).is_some() {
            wolges::return_error!(format!("duplicate record {}", &record[0]));
        }
    }
    let mut sorted_machine_words = leaves_map.keys().cloned().collect::<Box<_>>();
    sorted_machine_words.sort_unstable();
    let leaves_kwg = build::build(build::BuildFormat::DawgOnly, &sorted_machine_words)?;
    let leave_values = sorted_machine_words
        .iter()
        .map(|s| leaves_map[s])
        .collect::<Box<_>>();
    drop(sorted_machine_words);
    drop(leaves_map);
    let mut bin = vec![0; 2 * 4 + leaves_kwg.len() + leave_values.len() * 4];
    let mut w = 0;
    bin[w..w + 4].copy_from_slice(&((leaves_kwg.len() / 4) as u32).to_le_bytes());
    w += 4;
    bin[w..w + leaves_kwg.len()].copy_from_slice(&leaves_kwg);
    w += leaves_kwg.len();
    bin[w..w + 4].copy_from_slice(&(leave_values.len() as u32).to_le_bytes());
    w += 4;
    for v in &leave_values[..] {
        bin[w..w + 4].copy_from_slice(&v.to_le_bytes());
        w += 4;
    }
    assert_eq!(w, bin.len());
    Ok(bin)
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

// slower than std::fs::read_to_string because it cannot preallocate the correct size.
fn read_to_string(reader: &mut Box<dyn std::io::Read>) -> Result<String, std::io::Error> {
    let mut s = String::new();
    reader.read_to_string(&mut s)?;
    Ok(s)
}

fn do_lang<AlphabetMaker: Fn() -> alphabet::Alphabet>(
    args: &[String],
    language_name: &str,
    make_alphabet: AlphabetMaker,
) -> error::Returns<bool> {
    match args[1].strip_prefix(language_name) {
        Some(args1_suffix) => match args1_suffix {
            "-klv" => {
                make_writer(&args[3])?
                    .write_all(&build_leaves(&mut make_reader(&args[2])?, make_alphabet())?)?;
                Ok(true)
            }
            "-klv2" => {
                make_writer(&args[3])?.write_all(&build_leaves_f32(
                    &mut make_reader(&args[2])?,
                    make_alphabet(),
                )?)?;
                Ok(true)
            }
            "-kwg" => {
                make_writer(&args[3])?.write_all(&build::build(
                    build::BuildFormat::Gaddawg,
                    &read_machine_words(
                        &alphabet::AlphabetReader::new_for_words(&make_alphabet()),
                        &read_to_string(&mut make_reader(&args[2])?)?,
                    )?,
                )?)?;
                Ok(true)
            }
            "-kwg-dawg" => {
                make_writer(&args[3])?.write_all(&build::build(
                    build::BuildFormat::DawgOnly,
                    &read_machine_words(
                        &alphabet::AlphabetReader::new_for_words(&make_alphabet()),
                        &read_to_string(&mut make_reader(&args[2])?)?,
                    )?,
                )?)?;
                Ok(true)
            }
            "-kwg-alpha" => {
                make_writer(&args[3])?.write_all(&build::build(
                    build::BuildFormat::DawgOnly,
                    &build::make_alphagrams(&read_machine_words(
                        &alphabet::AlphabetReader::new_for_words(&make_alphabet()),
                        &read_to_string(&mut make_reader(&args[2])?)?,
                    )?),
                )?)?;
                Ok(true)
            }
            "-kwg-score" => {
                make_writer(&args[3])?.write_all(&build::build(
                    build::BuildFormat::Gaddawg,
                    &read_machine_words(
                        &alphabet::AlphabetReader::new_for_word_scores(&make_alphabet()),
                        &read_to_string(&mut make_reader(&args[2])?)?,
                    )?,
                )?)?;
                Ok(true)
            }
            "-kwg-score-dawg" => {
                make_writer(&args[3])?.write_all(&build::build(
                    build::BuildFormat::DawgOnly,
                    &read_machine_words(
                        &alphabet::AlphabetReader::new_for_word_scores(&make_alphabet()),
                        &read_to_string(&mut make_reader(&args[2])?)?,
                    )?,
                )?)?;
                Ok(true)
            }
            "-kwg-score-alpha" => {
                make_writer(&args[3])?.write_all(&build::build(
                    build::BuildFormat::DawgOnly,
                    &build::make_alphagrams(&read_machine_words(
                        &alphabet::AlphabetReader::new_for_word_scores(&make_alphabet()),
                        &read_to_string(&mut make_reader(&args[2])?)?,
                    )?),
                )?)?;
                Ok(true)
            }
            "-macondo" => {
                let alphabet = make_alphabet();
                let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read(&args[2])?);
                make_writer(&args[4])?.write_all(&lexport::to_macondo(
                    &kwg,
                    &alphabet,
                    &args[3],
                    lexport::MacondoFormat::Dawg,
                ))?;
                make_writer(&args[5])?.write_all(&lexport::to_macondo(
                    &kwg,
                    &alphabet,
                    &args[3],
                    lexport::MacondoFormat::Gaddag,
                ))?;
                Ok(true)
            }
            "-lxd" => {
                let alphabet = make_alphabet();
                let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read(&args[2])?);
                make_writer(&args[5])?
                    .write_all(&lexport::to_lxd(&kwg, &alphabet, &args[3], &args[4])?)?;
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
  auto
    just to test
  english-klv english.csv english.klv
    generate klv file (deprecated?)
  english-klv2 english.csv english.klv2
    generate klv2 file (preferred)
  english-kwg CSW21.txt CSW21.kwg
    generate kwg file containing gaddawg
  english-macondo CSW21.kwg CSW21 CSW21.dawg CSW21.gaddag
    read kwg file, with lexicon name save macondo dawg/gaddag
  english-lxd CSW21.kwg \"CSW21 something\" \"17 June 2021\" UKNA.lxd
    read kwg file, with title and date, save lxd
  english-kwg-alpha CSW21.txt CSW21.kad
    generate kad file containing alpha dawg
  english-kwg-dawg CSW21.txt outfile.dwg
    generate dawg-only file
  english-kwg-score CSW21.txt CSW21.kwg
  english-kwg-score-alpha CSW21.txt CSW21.kad
  english-kwg-score-dawg CSW21.txt outfile.dwg
    same as above but with representative same-score tiles
  (english can also be catalan, french, german, norwegian, polish, slovene,
    spanish, yupik)
input/output files can be \"-\" (not advisable for binary files)"
        );
        Ok(())
    } else if args[1] == "auto" {
        old_main()?;
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
            || do_lang(&args, "yupik", alphabet::make_yupik_alphabet)?
        {
        } else {
            return Err("invalid argument".into());
        }
        writeln!(boxed_stdout_or_stderr(), "time taken: {:?}", t0.elapsed())?;
        Ok(())
    }
}

fn old_main() -> error::Returns<()> {
    std::fs::write(
        "lexbin/english.klv2",
        build_leaves(
            Box::new(std::fs::File::open("lexsrc/english.csv")?),
            alphabet::make_english_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/CSW21.klv2",
        build_leaves(
            Box::new(std::fs::File::open("lexsrc/CSW21.csv")?),
            alphabet::make_english_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/french.klv2",
        build_leaves(
            Box::new(std::fs::File::open("lexsrc/french.csv")?),
            alphabet::make_french_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/german.klv2",
        build_leaves(
            Box::new(std::fs::File::open("lexsrc/german.csv")?),
            alphabet::make_german_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/norwegian.klv2",
        build_leaves(
            Box::new(std::fs::File::open("lexsrc/norwegian.csv")?),
            alphabet::make_norwegian_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/english.klv2",
        build_leaves_f32(
            Box::new(std::fs::File::open("lexsrc/english.csv")?),
            alphabet::make_english_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/CSW21.klv2",
        build_leaves_f32(
            Box::new(std::fs::File::open("lexsrc/CSW21.csv")?),
            alphabet::make_english_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/french.klv2",
        build_leaves_f32(
            Box::new(std::fs::File::open("lexsrc/french.csv")?),
            alphabet::make_french_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/german.klv2",
        build_leaves_f32(
            Box::new(std::fs::File::open("lexsrc/german.csv")?),
            alphabet::make_german_alphabet(),
        )?,
    )?;
    std::fs::write(
        "lexbin/norwegian.klv2",
        build_leaves_f32(
            Box::new(std::fs::File::open("lexsrc/norwegian.csv")?),
            alphabet::make_norwegian_alphabet(),
        )?,
    )?;
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/CSW21.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                    &std::fs::read_to_string("lexsrc/CSW21.txt")?,
                )?,
            )?,
        )?;
        println!("{:?} for reading+building+writing CSW21 kwg", t0.elapsed());
    }
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/CSW21.kad",
            build::build(
                build::BuildFormat::DawgOnly,
                &build::make_alphagrams(&read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                    &std::fs::read_to_string("lexsrc/CSW21.txt")?,
                )?),
            )?,
        )?;
        println!(
            "{:?} for reading+building+writing CSW21 alpha dawg",
            t0.elapsed()
        );
    }
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/CSW19.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                    &std::fs::read_to_string("lexsrc/CSW19.txt")?,
                )?,
            )?,
        )?;
        println!("{:?} for reading+building+writing CSW19 kwg", t0.elapsed());
    }
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/CSW19.kad",
            build::build(
                build::BuildFormat::DawgOnly,
                &build::make_alphagrams(&read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                    &std::fs::read_to_string("lexsrc/CSW19.txt")?,
                )?),
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
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                &std::fs::read_to_string("lexsrc/ECWL.txt")?,
            )?,
        )?,
    )?;
    std::fs::write(
        "lexbin/NWL18.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                &std::fs::read_to_string("lexsrc/NWL18.txt")?,
            )?,
        )?,
    )?;
    std::fs::write(
        "lexbin/NWL20.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                &std::fs::read_to_string("lexsrc/NWL20.txt")?,
            )?,
        )?,
    )?;
    if true {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/OSPS42-dawg.kwg",
            build::build(
                build::BuildFormat::DawgOnly,
                &read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_polish_alphabet()),
                    &std::fs::read_to_string("lexsrc/OSPS42.txt")?,
                )?,
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
                &read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_polish_alphabet()),
                    &std::fs::read_to_string("lexsrc/OSPS42.txt")?,
                )?,
            )?,
        )?;
        println!(
            "{:?} for reading+building+writing polish gaddawg",
            t0.elapsed()
        );
    }
    if true {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/OSPS44-dawg.kwg",
            build::build(
                build::BuildFormat::DawgOnly,
                &read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_polish_alphabet()),
                    &std::fs::read_to_string("lexsrc/OSPS44.txt")?,
                )?,
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
            "lexbin/OSPS44.kwg",
            build::build(
                build::BuildFormat::Gaddawg,
                &read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_polish_alphabet()),
                    &std::fs::read_to_string("lexsrc/OSPS44.txt")?,
                )?,
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
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                &std::fs::read_to_string("lexsrc/TWL14.txt")?,
            )?,
        )?,
    )?;

    if true {
        let english_alphabet = alphabet::make_english_alphabet();
        let polish_alphabet = alphabet::make_polish_alphabet();
        let t0 = std::time::Instant::now();
        {
            let t0 = std::time::Instant::now();
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW21.kwg")?);
            println!("{:?} for rereading CSW21.kwg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/CSW21.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "CSW21",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            println!("{:?} for exporting CSW21 dawg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/CSW21.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "CSW21",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
            println!("{:?} for exporting CSW21 gaddag", t0.elapsed());
        }
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
        if true {
            let t0 = std::time::Instant::now();
            let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS44.kwg")?);
            println!("{:?} for rereading OSPS44.kwg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/OSPS44.dawg",
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    "OSPS44",
                    lexport::MacondoFormat::Dawg,
                ),
            )?;
            println!("{:?} for exporting OSPS44 dawg", t0.elapsed());
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/OSPS44.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    "OSPS44",
                    lexport::MacondoFormat::Gaddag,
                ),
            )?;
            println!("{:?} for exporting OSPS44 gaddag", t0.elapsed());
        }
    }

    if true {
        // this reads the files again, but this code is temporary
        let v_csw21 = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/CSW21.txt")?,
        )?;
        let v_csw19 = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/CSW19.txt")?,
        )?;
        let v_ecwl = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/ECWL.txt")?,
        )?;
        let v_nwl18 = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/NWL18.txt")?,
        )?;
        let v_nwl20 = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/NWL20.txt")?,
        )?;
        let v_twl14 = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/TWL14.txt")?,
        )?;
        let mut v = Vec::<bites::Bites>::new();
        v.extend_from_slice(&v_csw21);
        v.extend_from_slice(&v_csw19);
        v.extend_from_slice(&v_ecwl);
        v.extend_from_slice(&v_nwl18);
        v.extend_from_slice(&v_nwl20);
        v.extend_from_slice(&v_twl14);
        v.sort_unstable();
        v.dedup();
        let v = v.into_boxed_slice();
        let v_bits_bytes = (v.len() + 7) / 8;
        let mut v_csw21_bits = vec![0u8; v_bits_bytes];
        let mut v_csw19_bits = vec![0u8; v_bits_bytes];
        let mut v_ecwl_bits = vec![0u8; v_bits_bytes];
        let mut v_nwl18_bits = vec![0u8; v_bits_bytes];
        let mut v_nwl20_bits = vec![0u8; v_bits_bytes];
        let mut v_twl14_bits = vec![0u8; v_bits_bytes];
        let mut p_csw21 = v_csw21.len();
        let mut p_csw19 = v_csw19.len();
        let mut p_ecwl = v_ecwl.len();
        let mut p_nwl18 = v_nwl18.len();
        let mut p_nwl20 = v_nwl20.len();
        let mut p_twl14 = v_twl14.len();
        for i in (0..v.len()).rev() {
            if p_csw21 > 0 && v[i] == v_csw21[p_csw21 - 1] {
                v_csw21_bits[i / 8] |= 1 << (i % 8);
                p_csw21 -= 1;
            }
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
        std::fs::write("lexbin/all-CSW21.kwi", v_csw21_bits)?;
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
                max_len = max_len.max(v.len());
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
        let v_csw21_bits = std::fs::read("lexbin/all-CSW21.kwi")?;
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
            print!("{i} {j} {out_vec:?}");
            let byte_index = j as usize / 8;
            let bit = 1 << (j as usize % 8);
            if v_csw21_bits[byte_index] & bit != 0 {
                print!(" CSW21");
            }
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
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                "VOLOST\nVOLOSTS",
            )?,
        )?,
    )?;
    std::fs::write(
        "lexbin/empty.kwg",
        build::build(
            build::BuildFormat::Gaddawg,
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                "",
            )?,
        )?,
    )?;

    Ok(())
}
