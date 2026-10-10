// Copyright (C) 2020-2026 Andy Kurnia.

use wolges::kwg::Node;
use wolges::{alphabet, bites, build, equity, error, fash, klv, kwg, lexport, prob};

#[inline]
fn parse_machine_words(
    alphabet_reader: &alphabet::AlphabetReader,
    giant_string: &str,
) -> error::Returns<Vec<bites::Bites>> {
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
            } else if ix > 0 && sb[ix] <= b' ' {
                break;
            } else {
                wolges::return_error!(format!("invalid tile after {v:?} in {s:?}"));
            }
        }
        machine_words.push(v[..].into());
    }
    Ok(machine_words)
}

// the lines, as they came, whose word holds from min_len to max_len tiles: a
// word ends at its first blank, as in a word list; an empty line is no word
#[inline(always)]
fn trim_words(
    alphabet_reader: &alphabet::AlphabetReader,
    giant_string: &str,
    min_len: usize,
    max_len: usize,
) -> error::Returns<String> {
    let mut kept = String::new();
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
            } else if ix > 0 && sb[ix] <= b' ' {
                break;
            } else {
                wolges::return_error!(format!("invalid tile after {v:?} in {s:?}"));
            }
        }
        if (min_len..=max_len).contains(&v.len()) {
            kept.push_str(s);
            kept.push('\n');
        }
    }
    Ok(kept)
}

// the first header records of a csv as they are, then those whose field at
// column (0 the first) holds from min_len to max_len tiles, read and written by
// the csv crate, so a quoted field counts unquoted
#[inline(always)]
fn trim_csv<R: std::io::Read, W: std::io::Write>(
    alphabet_reader: &alphabet::AlphabetReader,
    input: R,
    output: W,
    column: usize,
    header: usize,
    min_len: usize,
    max_len: usize,
) -> error::Returns<()> {
    let mut csv_in = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(input);
    let mut csv_out = csv::WriterBuilder::new().flexible(true).from_writer(output);
    let mut v = Vec::new();
    for (i, result) in csv_in.records().enumerate() {
        let record = result?;
        if i >= header {
            let Some(field) = record.get(column) else {
                wolges::return_error!(format!("record {} has no field {}", i + 1, column + 1));
            };
            alphabet_reader.set_word(field, &mut v)?;
            if !(min_len..=max_len).contains(&v.len()) {
                continue;
            }
        }
        csv_out.write_record(&record)?;
    }
    csv_out.flush()?;
    Ok(())
}

#[inline]
fn read_machine_words(
    alphabet_reader: &alphabet::AlphabetReader,
    giant_string: &str,
) -> error::Returns<Box<[bites::Bites]>> {
    let mut machine_words = parse_machine_words(alphabet_reader, giant_string)?;
    machine_words.sort_unstable();
    machine_words.dedup();
    Ok(machine_words.into_boxed_slice())
}

#[inline]
fn read_machine_words_sorted_by_length(
    alphabet_reader: &alphabet::AlphabetReader,
    giant_string: &str,
) -> error::Returns<Box<[bites::Bites]>> {
    let mut machine_words = parse_machine_words(alphabet_reader, giant_string)?;
    machine_words.sort_unstable_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    machine_words.dedup();
    Ok(machine_words.into_boxed_slice())
}

use std::io::Write;
use std::str::FromStr;

// a load keeps round(v * SCALE) in an i32
#[inline(always)]
fn leave_value(leave: &str, value: &str) -> error::Returns<f32> {
    let v = f32::from_str(value)?;
    let milli = (v * equity::SCALE as f32).round() as f64;
    if (i32::MIN as f64..=i32::MAX as f64).contains(&milli) {
        Ok(v)
    } else {
        Err(format!("{leave:?} has the value {value:?}, which a leave table cannot hold").into())
    }
}

#[inline]
fn build_leaves_scaled_i16<Readable: std::io::Read>(
    f: Readable,
    alph: alphabet::Alphabet,
    build_layout: build::BuildLayout,
    scale: f32,
) -> error::Returns<Vec<u8>> {
    let alphabet_reader = alphabet::AlphabetReader::new_for_racks(&alph);
    let mut leaves_map: fash::MyHashMap<bites::Bites, _> = fash::MyHashMap::default();
    let mut csv_reader = csv::ReaderBuilder::new().has_headers(false).from_reader(f);
    let mut v = Vec::new();
    for result in csv_reader.records() {
        let record = result?;
        if record.len() < 2 {
            wolges::return_error!(format!(
                "{:?} is not a leave and a value",
                record.as_slice()
            ));
        }
        alphabet_reader.set_word(&record[0], &mut v)?;
        v.sort_unstable();
        let rounded_leave = (leave_value(&record[0], &record[1])? * scale).round();
        let int_leave = rounded_leave as i16;
        if (int_leave as f32 - rounded_leave).abs() != 0.0 {
            wolges::return_error!(format!(
                "for {}: {} (f32) {} (*{}) {} (round) {} (int) {} (float) {} (-) {} (abs) {}",
                &record[0],
                &record[1],
                f32::from_str(&record[1])?,
                scale,
                f32::from_str(&record[1])? * scale,
                rounded_leave,
                int_leave,
                int_leave as f32,
                int_leave as f32 - rounded_leave,
                (int_leave as f32 - rounded_leave).abs(),
            ));
        }
        if leaves_map.insert(v[..].into(), int_leave).is_some() {
            wolges::return_error!(format!("duplicate record {}", &record[0]));
        }
    }
    let mut sorted_machine_words = leaves_map.keys().cloned().collect::<Box<_>>();
    sorted_machine_words.sort_unstable();
    let leaves_kwg = build::build(
        build::BuildContent::DawgOnly,
        build_layout,
        build::BuildOrder::Sorted,
        &sorted_machine_words,
    )?;
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

#[inline]
fn write_leaves_f32(
    leaves_map: fash::MyHashMap<bites::Bites, f32>,
    build_layout: build::BuildLayout,
) -> error::Returns<Vec<u8>> {
    let mut sorted_machine_words = leaves_map.keys().cloned().collect::<Box<_>>();
    sorted_machine_words.sort_unstable();
    let leaves_kwg = build::build(
        build::BuildContent::DawgOnly,
        build_layout,
        build::BuildOrder::Sorted,
        &sorted_machine_words,
    )?;
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

#[inline]
fn read_leaves_f32<Readable: std::io::Read>(
    f: Readable,
    alph: &alphabet::Alphabet,
) -> error::Returns<fash::MyHashMap<bites::Bites, f32>> {
    let alphabet_reader = alphabet::AlphabetReader::new_for_racks(alph);
    let mut leaves_map = fash::MyHashMap::<bites::Bites, _>::default();
    let mut csv_reader = csv::ReaderBuilder::new().has_headers(false).from_reader(f);
    let mut v = Vec::new();
    for result in csv_reader.records() {
        let record = result?;
        if record.len() < 2 {
            wolges::return_error!(format!(
                "{:?} is not a leave and a value",
                record.as_slice()
            ));
        }
        alphabet_reader.set_word(&record[0], &mut v)?;
        v.sort_unstable();
        let float_leave = leave_value(&record[0], &record[1])?;
        if leaves_map.insert(v[..].into(), float_leave).is_some() {
            wolges::return_error!(format!("duplicate record {}", &record[0]));
        }
    }
    Ok(leaves_map)
}

#[inline]
fn leave_as_decimal(v: f32, scratch: &mut String) -> f64 {
    use std::fmt::Write as _;
    scratch.clear();
    let _ = write!(scratch, "{v}");
    scratch.parse::<f64>().unwrap_or(v as f64)
}

#[inline]
fn leave_at_millipoint(v: f64, scratch: &mut String) -> f32 {
    use std::fmt::Write as _;
    scratch.clear();
    let _ = write!(scratch, "{v:.3}");
    scratch.parse::<f32>().unwrap_or(v as f32)
}

#[inline]
fn refuse_a_wider_graph<N: kwg::Node>(
    kwg: &kwg::Kwg<N>,
    alphabet: &alphabet::Alphabet,
    kwg_path: &str,
) -> error::Returns<()> {
    let alphabet_len = alphabet.len();
    if kwg.fits_alphabet(alphabet_len) {
        return Ok(());
    }
    wolges::return_error!(format!(
        "{kwg_path} has tiles past these {alphabet_len} tiles",
    ));
}

static USED_STDOUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// support "-" to mean stdout.
#[inline]
fn make_writer(filename: &str) -> Result<Box<dyn std::io::Write>, std::io::Error> {
    Ok(if filename == "-" {
        USED_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(filename)?)
    })
}

// when using "-" as output filename, print things to stderr.
#[inline]
fn boxed_stdout_or_stderr() -> Box<dyn std::io::Write> {
    if USED_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
        Box::new(std::io::stderr()) as Box<dyn std::io::Write>
    } else {
        Box::new(std::io::stdout())
    }
}

// support "-" to mean stdin.
#[inline]
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

#[derive(Clone, clap::ValueEnum)]
enum Graph {
    Gaddawg,
    Dawg,
}

#[derive(Clone, clap::ValueEnum)]
enum Words {
    Plain,
    Alphagrams,
    Reversed,
    #[value(name = "plain+reversed")]
    PlainReversed,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Layout {
    Wolges,
    Magpie,
    Magpiemerged,
    Legacy,
    Experimental,
}

impl Layout {
    #[inline(always)]
    fn build_layout(self) -> build::BuildLayout {
        match self {
            Layout::Wolges => build::BuildLayout::Wolges,
            Layout::Magpie => build::BuildLayout::Magpie,
            Layout::Magpiemerged => build::BuildLayout::MagpieMerged,
            Layout::Legacy => build::BuildLayout::Legacy,
            Layout::Experimental => build::BuildLayout::Experimental,
        }
    }
}

#[derive(Clone, clap::ValueEnum)]
enum KlvFormat {
    #[value(name = "1")]
    One,
    #[value(name = "2")]
    Two,
    #[value(name = "16")]
    Sixteen,
}

#[derive(clap::Args)]
struct Kwg {
    #[arg(help = "the words, one a line")]
    word_list: String,
    #[arg(help = "the word graph to write")]
    out: String,
    #[arg(
        long,
        value_enum,
        help = "gaddawg (dawg and gaddag) or dawg alone [default: gaddawg; dawg for --words alphagrams or --blanks]"
    )]
    graph: Option<Graph>,
    #[arg(
        long,
        value_enum,
        default_value = "plain",
        help = "the words as listed, their alphagrams, the words written backward, or the words both ways"
    )]
    words: Words,
    #[arg(
        long,
        value_enum,
        default_value = "wolges",
        help = "where the nodes go: magpie is bigger and magpie-style, magpiemerged is magpie ordering with wolges merging, legacy is the former default"
    )]
    layout: Layout,
    #[arg(
        long,
        help = "a smaller graph: each node's children in whatever order shares the most nodes, instead of in tile order (not for alphagrams)"
    )]
    reordered: bool,
    #[arg(long, help = "one representative tile for each score")]
    score: bool,
    #[arg(
        long,
        help = "the wider format, kbwg (16M nodes instead of 4M; not widely read)"
    )]
    kbwg: bool,
    #[arg(long, help = "a word may hold ? (tile 0); the graph is then a dawg")]
    blanks: bool,
}

#[derive(clap::Args)]
struct Klv {
    #[arg(help = "the leaves and their values, leave,value a line")]
    leaves: String,
    #[arg(help = "the leave table to write")]
    out: String,
    #[arg(
        long,
        value_enum,
        default_value = "2",
        help = "2 (klv2, preferred), 1 (klv, deprecated?) or 16 (klv16, magpie-retro)"
    )]
    klv: KlvFormat,
    #[arg(long, value_enum, default_value = "wolges", help = "as for kwg")]
    layout: Layout,
}

#[derive(clap::Args)]
struct Blend {
    #[arg(help = "the first leave table")]
    a: String,
    #[arg(help = "the second leave table")]
    b: String,
    #[arg(
        allow_negative_numbers = true,
        help = "how much of the second table to take: 0 is the first table alone, 1 the second"
    )]
    weight: f64,
    #[arg(help = "the klv2 to write")]
    out: String,
    #[arg(long, value_enum, default_value = "wolges", help = "as for kwg")]
    layout: Layout,
}

#[derive(clap::Args)]
struct Macondo {
    #[arg(help = "the word graph")]
    kwg: String,
    #[arg(help = "the lexicon name to save")]
    name: String,
    #[arg(help = "the dawg to write")]
    dawg: String,
    #[arg(help = "the gaddag to write")]
    gaddag: String,
}

#[derive(clap::Args)]
struct Lxd {
    #[arg(help = "the word graph")]
    kwg: String,
    #[arg(help = "the title, such as \"CSW24 something\"")]
    title: String,
    #[arg(help = "the date, such as \"17 June 2021\"")]
    date: String,
    #[arg(help = "the lxd to write")]
    out: String,
}

#[derive(clap::Args)]
struct SortWords {
    #[arg(help = "the words, one a line")]
    word_list: String,
    #[arg(help = "the sorted words to write")]
    out: String,
    #[arg(long, help = "by length first")]
    len: bool,
}

#[derive(clap::Args)]
struct SortLeaves {
    #[arg(help = "the leaves and their values, leave,value a line")]
    leaves: String,
    #[arg(help = "the sorted leaves to write")]
    out: String,
    #[arg(long, help = "by length first")]
    len: bool,
    #[arg(long, help = "by value, highest first (after length, with --len)")]
    val: bool,
}

#[derive(clap::Args)]
struct Trim {
    #[arg(help = "the words, one a line; with --csv, a csv whose --column holds the tiles")]
    input: String,
    #[arg(help = "the words or records kept, in their order")]
    out: String,
    #[arg(long, help = "keep at least this many tiles")]
    min_len: Option<usize>,
    #[arg(long, help = "keep at most this many tiles")]
    max_len: Option<usize>,
    #[arg(
        long,
        help = "the input is a csv, read and written by the csv crate: leaves, summaries, playability or probability"
    )]
    csv: bool,
    #[arg(
        long,
        requires = "csv",
        default_value = "1",
        help = "with --csv, the field that holds the tiles, 1 the first"
    )]
    column: std::num::NonZeroUsize,
    #[arg(
        long,
        requires = "csv",
        default_value_t = 0,
        help = "with --csv, the first records kept as they are, such as a header"
    )]
    header: usize,
}

#[derive(clap::Subcommand)]
enum Task {
    #[command(about = "just to test")]
    Auto,
    #[command(about = "generate a word graph from a word list")]
    Kwg(Kwg),
    #[command(about = "generate a leave table from a csv")]
    Klv(Klv),
    #[command(
        about = "blend two leave tables into one klv2; a leave only one table lists keeps that table's value"
    )]
    Blend(Blend),
    #[command(about = "read a word graph, save macondo dawg and gaddag with the lexicon name")]
    Macondo(Macondo),
    #[command(about = "read a word graph, save lxd with the title and date")]
    Lxd(Lxd),
    #[command(about = "rewrite words, unique and sorted")]
    SortWords(SortWords),
    #[command(about = "rewrite leave,value lines, sorted")]
    SortLeaves(SortLeaves),
    #[command(
        about = "keep the words, or the csv records, whose tiles number from --min-len to --max-len",
        group(clap::ArgGroup::new("lengths").args(["min_len", "max_len"]).required(true).multiple(true))
    )]
    Trim(Trim),
}

#[derive(clap::Parser)]
#[command(
    about = "build word graphs and leave tables",
    arg_required_else_help = true,
    after_help = "input/output files can be \"-\" (not advisable for binary files)"
)]
struct Cli {
    #[arg(
        long,
        value_name = "NAME|FILE",
        default_value = "english",
        help = "bundled tiles or an alphabet file"
    )]
    tiles: String,
    #[command(subcommand)]
    task: Task,
}

#[inline]
fn make_kwg(args: &Kwg, alph: &alphabet::Alphabet) -> error::Returns<bites::Bites> {
    let dawg = match args.graph {
        Some(Graph::Dawg) => true,
        Some(Graph::Gaddawg) => {
            if args.blanks {
                return Err("--blanks makes a dawg: tile 0 is the gaddag's separator".into());
            }
            false
        }
        None => args.blanks || matches!(args.words, Words::Alphagrams),
    };
    if args.reordered && matches!(args.words, Words::Alphagrams) {
        return Err("alphagrams cannot be reordered: an alpha dawg is walked in tile order".into());
    }
    let alphabet_reader = if args.blanks {
        alphabet::AlphabetReader::new_for_racks(alph)
    } else if args.score {
        alphabet::AlphabetReader::new_for_word_scores(alph)
    } else {
        alphabet::AlphabetReader::new_for_words(alph)
    };
    let mut words = read_machine_words(
        &alphabet_reader,
        &read_to_string(&mut make_reader(&args.word_list)?)?,
    )?;
    if args.blanks && args.score {
        let mut scored = words
            .iter()
            .map(|word| {
                word.iter()
                    .map(|&tile| alph.representative_same_score_tile(tile))
                    .collect::<Vec<_>>()[..]
                    .into()
            })
            .collect::<Vec<bites::Bites>>();
        scored.sort_unstable();
        scored.dedup();
        words = scored.into_boxed_slice();
    }
    let words = match args.words {
        Words::Plain => words,
        Words::Alphagrams => build::make_alphagrams(&words),
        Words::Reversed | Words::PlainReversed => {
            let mut all = Vec::with_capacity(words.len() * 2);
            if matches!(args.words, Words::PlainReversed) {
                all.extend_from_slice(&words);
            }
            let mut v = Vec::new();
            for word in &words[..] {
                v.clear();
                v.extend_from_slice(word);
                v.reverse();
                all.push(v[..].into());
            }
            all.sort_unstable();
            all.dedup();
            all.into_boxed_slice()
        }
    };
    let build_content = if dawg {
        build::BuildContent::DawgOnly
    } else {
        build::BuildContent::Gaddawg
    };
    let build_order = if args.reordered {
        build::BuildOrder::Reordered
    } else {
        build::BuildOrder::Sorted
    };
    if args.kbwg {
        build::build_big(
            build_content,
            args.layout.build_layout(),
            build_order,
            &words,
        )
    } else {
        build::build(
            build_content,
            args.layout.build_layout(),
            build_order,
            &words,
        )
    }
}

#[inline]
fn do_task<AlphabetMaker: FnOnce() -> error::Returns<alphabet::Alphabet>>(
    task: &Task,
    make_alphabet: AlphabetMaker,
) -> error::Returns<()> {
    match task {
        Task::Auto => old_main(),
        Task::Kwg(args) => {
            let built = make_kwg(args, &make_alphabet()?)?;
            make_writer(&args.out)?.write_all(&built)?;
            Ok(())
        }
        Task::Klv(args) => {
            let alphabet = make_alphabet()?;
            let built = match args.klv {
                KlvFormat::Two => write_leaves_f32(
                    read_leaves_f32(&mut make_reader(&args.leaves)?, &alphabet)?,
                    args.layout.build_layout(),
                )?,
                KlvFormat::One | KlvFormat::Sixteen => build_leaves_scaled_i16(
                    &mut make_reader(&args.leaves)?,
                    alphabet,
                    args.layout.build_layout(),
                    if matches!(args.klv, KlvFormat::One) {
                        256.0
                    } else {
                        8.0
                    },
                )?,
            };
            make_writer(&args.out)?.write_all(&built)?;
            Ok(())
        }
        Task::Blend(args) => {
            let weight = args.weight;
            if !weight.is_finite() {
                return Err("blend needs a finite weight".into());
            }
            let leaves_a = klv::read_leaves_alloc::<kwg::Node22>(&std::fs::read(&args.a)?)?;
            let leaves_b = klv::read_leaves_alloc::<kwg::Node22>(&std::fs::read(&args.b)?)?;
            let num_a = leaves_a.len();
            let num_b = leaves_b.len();
            let mut scratch = String::new();
            let mut mixed = fash::MyHashMap::<bites::Bites, f64>::default();
            for (leave, value) in leaves_b {
                mixed.insert(leave, leave_as_decimal(value, &mut scratch));
            }
            let mut in_both = 0usize;
            for (leave, value) in leaves_a {
                let value_a = leave_as_decimal(value, &mut scratch);
                mixed
                    .entry(leave)
                    .and_modify(|value_b| {
                        in_both += 1;
                        *value_b = (1.0 - weight) * value_a + weight * *value_b;
                    })
                    .or_insert(value_a);
            }
            let mut leaves_map = fash::MyHashMap::<bites::Bites, f32>::default();
            leaves_map.reserve(mixed.len());
            for (leave, value) in mixed.drain() {
                leaves_map.insert(leave, leave_at_millipoint(value, &mut scratch));
            }
            drop(mixed);
            writeln!(
                boxed_stdout_or_stderr(),
                "blended {in_both} leaves at weight {weight}, kept {} found only in {} and {} found only in {}",
                num_a - in_both,
                args.a,
                num_b - in_both,
                args.b,
            )?;
            make_writer(&args.out)?
                .write_all(&write_leaves_f32(leaves_map, args.layout.build_layout())?)?;
            Ok(())
        }
        Task::Macondo(args) => {
            let alphabet = make_alphabet()?;
            let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read(&args.kwg)?)?;
            refuse_a_wider_graph(&kwg, &alphabet, &args.kwg)?;
            make_writer(&args.dawg)?.write_all(&lexport::to_macondo(
                &kwg,
                &alphabet,
                &args.name,
                lexport::MacondoFormat::Dawg,
            )?)?;
            make_writer(&args.gaddag)?.write_all(&lexport::to_macondo(
                &kwg,
                &alphabet,
                &args.name,
                lexport::MacondoFormat::Gaddag,
            )?)?;
            Ok(())
        }
        Task::Lxd(args) => {
            let alphabet = make_alphabet()?;
            let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read(&args.kwg)?)?;
            refuse_a_wider_graph(&kwg, &alphabet, &args.kwg)?;
            make_writer(&args.out)?.write_all(&lexport::to_lxd(
                &kwg,
                &alphabet,
                &args.title,
                &args.date,
            )?)?;
            Ok(())
        }
        Task::SortWords(args) => {
            let alphabet = make_alphabet()?;
            // allow "?" for more flexibility.
            let alphabet_reader = alphabet::AlphabetReader::new_for_racks(&alphabet);
            let giant_string = read_to_string(&mut make_reader(&args.word_list)?)?;
            let words = &if args.len {
                read_machine_words_sorted_by_length(&alphabet_reader, &giant_string)?
            } else {
                read_machine_words(&alphabet_reader, &giant_string)?
            };
            let mut ret = String::new();
            for word in words {
                for &tile in &word[..] {
                    ret.push_str(alphabet.of_rack(tile).unwrap());
                }
                ret.push('\n');
            }
            make_writer(&args.out)?.write_all(ret.as_bytes())?;
            Ok(())
        }
        Task::SortLeaves(args) => {
            let alphabet = make_alphabet()?;
            let mut leaves = read_leaves_f32(&mut make_reader(&args.leaves)?, &alphabet)?
                .drain()
                .collect::<Box<_>>();
            match (args.len, args.val) {
                (false, false) => {
                    leaves.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                }
                (true, false) => {
                    leaves.sort_unstable_by(|a, b| {
                        a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0))
                    });
                }
                (false, true) => {
                    leaves.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                }
                (true, true) => {
                    leaves.sort_unstable_by(|a, b| {
                        a.0.len()
                            .cmp(&b.0.len())
                            .then_with(|| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)))
                    });
                }
            }
            let mut csv_out = csv::Writer::from_writer(make_writer(&args.out)?);
            let mut s = String::new();
            for (k, v) in leaves {
                s.clear();
                for &tile in &k[..] {
                    s.push_str(alphabet.of_rack(tile).unwrap());
                }
                csv_out.serialize((&s, v))?;
            }
            Ok(())
        }
        Task::Trim(args) => {
            let min_len = args.min_len.unwrap_or(0);
            let max_len = args.max_len.unwrap_or(usize::MAX);
            if min_len > max_len {
                return Err("--min-len is more than --max-len".into());
            }
            let alphabet = make_alphabet()?;
            // a leave may hold ? (tile 0), as sort-words and sort-leaves allow
            let alphabet_reader = alphabet::AlphabetReader::new_for_racks(&alphabet);
            if args.csv {
                trim_csv(
                    &alphabet_reader,
                    make_reader(&args.input)?,
                    make_writer(&args.out)?,
                    args.column.get() - 1,
                    args.header,
                    min_len,
                    max_len,
                )
            } else {
                let kept = trim_words(
                    &alphabet_reader,
                    &read_to_string(&mut make_reader(&args.input)?)?,
                    min_len,
                    max_len,
                )?;
                make_writer(&args.out)?.write_all(kept.as_bytes())?;
                Ok(())
            }
        }
    }
}

fn main() -> error::Returns<()> {
    let cli: Cli = clap::Parser::parse();
    if let Task::Auto = cli.task {
        return old_main();
    }
    let t0 = std::time::Instant::now();
    do_task(&cli.task, || {
        alphabet::make_alphabet_from(&cli.tiles, &|path| Ok(std::fs::read_to_string(path)?))
    })?;
    writeln!(boxed_stdout_or_stderr(), "time taken: {:?}", t0.elapsed())?;
    Ok(())
}

#[inline]
fn old_main() -> error::Returns<()> {
    std::fs::write(
        "lexbin/CSW24.klv2",
        write_leaves_f32(
            read_leaves_f32(
                Box::new(std::fs::File::open("lexsrc/CSW24.csv")?),
                &alphabet::make_english_alphabet(),
            )?,
            build::BuildLayout::Wolges,
        )?,
    )?;
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/CSW24.kwg",
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                    &std::fs::read_to_string("lexsrc/CSW24.txt")?,
                )?,
            )?,
        )?;
        writeln!(
            std::io::stdout(),
            "{:?} for reading+building+writing CSW24 kwg",
            t0.elapsed()
        )?;
    }
    {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/CSW24.kad",
            build::build(
                build::BuildContent::DawgOnly,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &build::make_alphagrams(&read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                    &std::fs::read_to_string("lexsrc/CSW24.txt")?,
                )?),
            )?,
        )?;
        writeln!(
            std::io::stdout(),
            "{:?} for reading+building+writing CSW24 alpha dawg",
            t0.elapsed()
        )?;
    }
    std::fs::write(
        "lexbin/NWL23.kwg",
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                &std::fs::read_to_string("lexsrc/NWL23.txt")?,
            )?,
        )?,
    )?;
    std::fs::write(
        "lexbin/ECWL.kwg",
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                &std::fs::read_to_string("lexsrc/ECWL.txt")?,
            )?,
        )?,
    )?;
    if true {
        let t0 = std::time::Instant::now();
        std::fs::write(
            "lexbin/OSPS49.kwg",
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &read_machine_words(
                    &alphabet::AlphabetReader::new_for_words(&alphabet::make_polish_alphabet()),
                    &std::fs::read_to_string("lexsrc/OSPS49.txt")?,
                )?,
            )?,
        )?;
        writeln!(
            std::io::stdout(),
            "{:?} for reading+building+writing polish gaddawg",
            t0.elapsed()
        )?;
    }

    if true {
        let english_alphabet = alphabet::make_english_alphabet();
        let polish_alphabet = alphabet::make_polish_alphabet();
        let t0 = std::time::Instant::now();
        {
            let t0 = std::time::Instant::now();
            let kwg =
                kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read("lexbin/CSW24.kwg")?)?;
            writeln!(
                std::io::stdout(),
                "{:?} for rereading CSW24.kwg",
                t0.elapsed()
            )?;
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/CSW24.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "CSW24",
                    lexport::MacondoFormat::Dawg,
                )?,
            )?;
            writeln!(
                std::io::stdout(),
                "{:?} for exporting CSW24 dawg",
                t0.elapsed()
            )?;
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/CSW24.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "CSW24",
                    lexport::MacondoFormat::Gaddag,
                )?,
            )?;
            writeln!(
                std::io::stdout(),
                "{:?} for exporting CSW24 gaddag",
                t0.elapsed()
            )?;
        }
        {
            let kwg =
                kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read("lexbin/NWL23.kwg")?)?;
            std::fs::write(
                "lexbin/NWL23.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL23",
                    lexport::MacondoFormat::Dawg,
                )?,
            )?;
            std::fs::write(
                "lexbin/NWL23.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "NWL23",
                    lexport::MacondoFormat::Gaddag,
                )?,
            )?;
        }
        {
            let kwg =
                kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read("lexbin/ECWL.kwg")?)?;
            std::fs::write(
                "lexbin/ECWL.dawg",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "ECWL",
                    lexport::MacondoFormat::Dawg,
                )?,
            )?;
            std::fs::write(
                "lexbin/ECWL.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &english_alphabet,
                    "ECWL",
                    lexport::MacondoFormat::Gaddag,
                )?,
            )?;
        }
        writeln!(
            std::io::stdout(),
            "{:?} for exporting many files",
            t0.elapsed()
        )?;
        if true {
            let t0 = std::time::Instant::now();
            let kwg =
                kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read("lexbin/OSPS49.kwg")?)?;
            writeln!(
                std::io::stdout(),
                "{:?} for rereading OSPS49.kwg",
                t0.elapsed()
            )?;
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/OSPS49.dawg",
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    "OSPS49",
                    lexport::MacondoFormat::Dawg,
                )?,
            )?;
            writeln!(
                std::io::stdout(),
                "{:?} for exporting OSPS49 dawg",
                t0.elapsed()
            )?;
            let t0 = std::time::Instant::now();
            std::fs::write(
                "lexbin/OSPS49.gaddag",
                lexport::to_macondo(
                    &kwg,
                    &polish_alphabet,
                    "OSPS49",
                    lexport::MacondoFormat::Gaddag,
                )?,
            )?;
            writeln!(
                std::io::stdout(),
                "{:?} for exporting OSPS49 gaddag",
                t0.elapsed()
            )?;
        }
    }

    if true {
        // this reads the files again, but this code is temporary
        let v_csw24 = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/CSW24.txt")?,
        )?;
        let v_nwl23 = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/NWL23.txt")?,
        )?;
        let v_ecwl = read_machine_words(
            &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
            &std::fs::read_to_string("lexsrc/ECWL.txt")?,
        )?;
        let mut v = Vec::<bites::Bites>::new();
        v.extend_from_slice(&v_csw24);
        v.extend_from_slice(&v_nwl23);
        v.extend_from_slice(&v_ecwl);
        v.sort_unstable();
        v.dedup();
        let v = v.into_boxed_slice();
        let v_bits_bytes = v.len().div_ceil(8);
        let mut v_csw24_bits = vec![0u8; v_bits_bytes];
        let mut v_nwl23_bits = vec![0u8; v_bits_bytes];
        let mut v_ecwl_bits = vec![0u8; v_bits_bytes];
        let mut p_csw24 = v_csw24.len();
        let mut p_nwl23 = v_nwl23.len();
        let mut p_ecwl = v_ecwl.len();
        for i in (0..v.len()).rev() {
            if p_csw24 > 0 && v[i] == v_csw24[p_csw24 - 1] {
                v_csw24_bits[i / 8] |= 1 << (i % 8);
                p_csw24 -= 1;
            }
            if p_nwl23 > 0 && v[i] == v_nwl23[p_nwl23 - 1] {
                v_nwl23_bits[i / 8] |= 1 << (i % 8);
                p_nwl23 -= 1;
            }
            if p_ecwl > 0 && v[i] == v_ecwl[p_ecwl - 1] {
                v_ecwl_bits[i / 8] |= 1 << (i % 8);
                p_ecwl -= 1;
            }
        }
        std::fs::write(
            "lexbin/allgdw.kwg",
            build::build(
                build::BuildContent::Gaddawg,
                build::BuildLayout::Wolges,
                build::BuildOrder::Sorted,
                &v,
            )?,
        )?;
        std::fs::write("lexbin/all-CSW24.kwi", v_csw24_bits)?;
        std::fs::write("lexbin/all-NWL23.kwi", v_nwl23_bits)?;
        std::fs::write("lexbin/all-ECWL.kwi", v_ecwl_bits)?;

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
                max_len = max_len.max(word.len());
                (alphagram, word_prob.word_draw_ways(word))
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
        let kwg = kwg::Kwg::<kwg::Node22>::from_bytes_alloc(&std::fs::read("lexbin/allgdw.kwg")?)?;
        let word_counts = kwg.count_dawg_words_alloc()?;
        // because dawg do not need gaddag nodes
        writeln!(
            std::io::stdout(),
            "only counting {} nodes",
            word_counts.len()
        )?;
        let v_csw24_bits = std::fs::read("lexbin/all-CSW24.kwi")?;
        let v_nwl23_bits = std::fs::read("lexbin/all-NWL23.kwi")?;
        let v_ecwl_bits = std::fs::read("lexbin/all-ECWL.kwi")?;
        let v_probability_indexes = std::fs::read("lexbin/all-probidx.kwp")?;
        let mut out_vec = Vec::new();
        let dawg_root = kwg[0].arc_index();
        let english_alphabet = alphabet::make_english_alphabet();
        let mut word_prob = prob::WordProbability::new(&english_alphabet);
        for i in 0..word_counts[dawg_root as usize] {
            out_vec.clear();
            kwg.get_word_by_index(&word_counts, dawg_root, i, |v| {
                out_vec.push(v);
            })?;
            let j = kwg.get_word_index(&word_counts, dawg_root, &out_vec);
            write!(std::io::stdout(), "{i} {j} {out_vec:?}")?;
            let byte_index = j as usize / 8;
            let bit = 1 << (j as usize % 8);
            if v_csw24_bits[byte_index] & bit != 0 {
                write!(std::io::stdout(), " CSW24")?;
            }
            if v_nwl23_bits[byte_index] & bit != 0 {
                write!(std::io::stdout(), " NWL23")?;
            }
            if v_ecwl_bits[byte_index] & bit != 0 {
                write!(std::io::stdout(), " ECWL")?;
            }
            write!(
                std::io::stdout(),
                " wp={}",
                word_prob.word_draw_ways(&out_vec)
            )?;
            write!(
                std::io::stdout(),
                " pi={}",
                u32::from_le_bytes(
                    v_probability_indexes[i as usize * 4..i as usize * 4 + 4]
                        .try_into()
                        .unwrap()
                )
            )?;
            writeln!(std::io::stdout())?;
            assert_eq!(i, j);
        }
    }

    std::fs::write(
        "lexbin/volost.kwg",
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                "VOLOST\nVOLOSTS",
            )?,
        )?,
    )?;
    std::fs::write(
        "lexbin/empty.kwg",
        build::build(
            build::BuildContent::Gaddawg,
            build::BuildLayout::Wolges,
            build::BuildOrder::Sorted,
            &read_machine_words(
                &alphabet::AlphabetReader::new_for_words(&alphabet::make_english_alphabet()),
                "",
            )?,
        )?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[inline]
    fn trim_keeps_the_words_whose_tiles_fit() {
        let english = alphabet::make_english_alphabet();
        let reader = alphabet::AlphabetReader::new_for_racks(&english);
        let words = "QI\nQUA\nQUIZ definition\n\nQUIZZED\n";
        assert_eq!(
            trim_words(&reader, words, 3, 4).unwrap(),
            "QUA\nQUIZ definition\n"
        );
        assert_eq!(trim_words(&reader, words, 0, 2).unwrap(), "QI\n");
        assert!(trim_words(&reader, "A#\n", 0, 9).is_err());
        // a comma ends no word: a csv takes --csv
        assert!(trim_words(&reader, "QI,5\n", 0, 9).is_err());
        // tiles, not chars: [CH], also written 1, is one spanish tile
        let spanish = alphabet::make_spanish_alphabet();
        let reader = alphabet::AlphabetReader::new_for_racks(&spanish);
        assert_eq!(
            trim_words(&reader, "[CH]E\nCHE\n1E\n", 2, 2).unwrap(),
            "[CH]E\n1E\n"
        );
    }

    #[test]
    #[inline]
    fn trim_keeps_the_csv_records_whose_column_fits() {
        let english = alphabet::make_english_alphabet();
        let reader = alphabet::AlphabetReader::new_for_racks(&english);
        let trim = |input: &str, column, header, min_len, max_len| {
            let mut out = Vec::new();
            trim_csv(
                &reader,
                input.as_bytes(),
                &mut out,
                column,
                header,
                min_len,
                max_len,
            )
            .map(|()| String::from_utf8(out).unwrap())
        };
        // a summary's total line holds no tiles; a quoted leave counts unquoted
        let leaves = ",0.5,7\n?,25.1\n\"QU\",-3\nAEINST,30\n";
        assert_eq!(trim(leaves, 0, 0, 0, 2).unwrap(), ",0.5,7\n?,25.1\nQU,-3\n");
        assert_eq!(trim(leaves, 0, 0, 2, 2).unwrap(), "QU,-3\n");
        assert_eq!(trim(leaves, 0, 1, 2, 2).unwrap(), ",0.5,7\nQU,-3\n");
        // playability and probability records: the word is the first field
        let words = "QI,773.0,824\nQUA,1.5,2\nQI,2,9,1,1\n";
        assert_eq!(trim(words, 0, 0, 3, 3).unwrap(), "QUA,1.5,2\n");
        assert!(trim("A#,1\n", 0, 0, 0, 9).is_err());
        // the tiles in another field, after a header kept as it is
        let ranked = "rank,word\n1,QI\n2,QUA\n3,\"ZA\"\n";
        assert_eq!(trim(ranked, 1, 1, 2, 2).unwrap(), "rank,word\n1,QI\n3,ZA\n");
        // without the header kept, it is judged as tiles: WORD has four
        assert_eq!(trim(ranked, 1, 0, 4, 4).unwrap(), "rank,word\n");
        assert!(trim("QI\n", 1, 0, 0, 9).is_err());
    }

    #[test]
    #[inline]
    fn a_leave_value_is_a_number_a_leave_table_holds() {
        assert_eq!(leave_value("QU", "-3.5").unwrap(), -3.5);
        let held = |v: f32| {
            let milli = (v * equity::SCALE as f32).round();
            milli as i32 as i64 == milli as i64
        };
        for (value, ok) in [
            ("-2147483.648", true),
            ("2147483.5", true),
            ("2147483.648", false),
            ("-2147484", false),
            ("1e37", false),
        ] {
            assert_eq!(leave_value("QU", value).is_ok(), ok, "{value:?}");
            assert_eq!(held(f32::from_str(value).unwrap()), ok, "{value:?}");
        }
        let edge = 2147483648.0 / equity::SCALE as f32;
        for edge in [edge, -edge] {
            for step in -64..=64 {
                let v = f32::from_bits(edge.to_bits().wrapping_add_signed(step));
                assert_eq!(leave_value("QU", &v.to_string()).is_ok(), held(v), "{v}");
            }
        }
        for value in ["nan", "NaN", "inf", "-inf", "infinity", "", "x"] {
            assert!(leave_value("QU", value).is_err(), "{value:?}");
        }
    }
}
