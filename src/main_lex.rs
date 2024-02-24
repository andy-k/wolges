// Copyright (C) 2020-2024 Andy Kurnia.

use wolges::{alphabet, bites, error, fash, game_config, kwg};

fn print_dawg(a: &alphabet::Alphabet, g: &kwg::Kwg) {
    struct Env<'a> {
        a: &'a alphabet::Alphabet,
        g: &'a kwg::Kwg,
        s: &'a mut String,
    }
    fn iter(env: &mut Env<'_>, mut p: i32) {
        let l = env.s.len();
        loop {
            let t = env.g[p].tile();
            env.s.push_str(if t == 0 {
                "@"
            } else if t & 0x80 == 0 {
                env.a.of_board(t).unwrap()
            } else {
                panic!()
            });
            if env.g[p].accepts() {
                println!("{}", env.s);
            }
            if env.g[p].arc_index() != 0 {
                iter(env, env.g[p].arc_index());
            }
            env.s.truncate(l);
            if env.g[p].is_end() {
                break;
            }
            p += 1;
        }
    }
    iter(
        &mut Env {
            a,
            g,
            s: &mut String::new(),
        },
        g[0].arc_index(),
    );
}

// parses '#' as 0
fn parse_embedded_words_board(
    alphabet_reader: &alphabet::AlphabetReader<'_>,
    s: &str,
    v: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    v.clear();
    if !s.is_empty() {
        v.reserve(s.len());
        let sb = s.as_bytes();
        let mut ix = 0;
        while ix < sb.len() {
            if let Some((tile, end_ix)) = alphabet_reader.next_tile(sb, ix) {
                v.push(tile);
                ix = end_ix;
            } else if sb[ix] == b'#' {
                v.push(0);
                ix += 1;
            } else {
                wolges::return_error!(format!("invalid tile after {v:?} in {s:?}"));
            }
        }
    }
    Ok(())
}

// x.sqrt().floor(), works with usize::MAX edge case too
fn isqrt(x: usize) -> usize {
    let lz = usize::leading_zeros(x);
    let mut bit = 1 << ((usize::BITS - lz - (lz == 0) as u32) >> 1);
    let mut ret = bit;
    loop {
        if ret * ret > x {
            ret ^= bit;
        }
        bit >>= 1;
        if bit == 0 {
            break;
        }
        ret |= bit;
    }
    ret
}

struct EmbeddedWordsFinder {
    q_tile: Option<u8>,
    u_tile: Option<u8>,
    rows: usize,
    cols: usize,
    ubuf: Vec<bool>,
    wbuf: Vec<u8>,
}

struct FindEmbeddedWordParams<'a, M: Fn(usize) -> i8, F: FnMut(&[u8], i8)> {
    board: &'a [u8],
    kwg: &'a kwg::Kwg,
    get_multiplier_at: &'a M,
    record_finding: &'a mut F,
}

impl EmbeddedWordsFinder {
    fn new(q_tile: Option<u8>, u_tile: Option<u8>) -> Self {
        Self {
            q_tile,
            u_tile,
            rows: 0,
            cols: 0,
            ubuf: Vec::new(),
            wbuf: Vec::new(),
        }
    }

    fn resize(&mut self, rows: usize, cols: usize) {
        self.rows = rows;
        self.cols = cols;
        self.ubuf.clear();
        self.wbuf.clear();
        self.ubuf.resize(rows * cols, false);
    }

    fn iter_embedded_words<M: Fn(usize) -> i8, F: FnMut(&[u8], i8)>(
        &mut self,
        params: &mut FindEmbeddedWordParams<'_, M, F>,
        row: usize,
        col: usize,
        mut p: i32,
        mut multiplier: i8,
    ) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let idx = row * self.cols + col;
        if self.ubuf[idx] {
            return;
        }
        let tile = params.board[idx];
        if tile == 0 {
            return;
        }
        p = params.kwg.seek(p, tile);
        if p <= 0 {
            return;
        }
        let orig_len = self.wbuf.len();
        self.ubuf[idx] = true;
        self.wbuf.push(tile);
        multiplier *= (params.get_multiplier_at)(idx);
        let node = params.kwg[p];
        if node.accepts() {
            (params.record_finding)(&self.wbuf, multiplier);
        }
        if node.arc_index() != 0 {
            for dr in -1..=1 {
                for dc in -1..=1 {
                    self.iter_embedded_words(
                        params,
                        (row as isize + dr) as usize,
                        (col as isize + dc) as usize,
                        p,
                        multiplier,
                    );
                }
            }
        }
        if matches!(self.q_tile, Some(q_tile) if q_tile == tile) {
            if let Some(u_tile) = self.u_tile {
                p = params.kwg.seek(p, u_tile);
                if p > 0 {
                    self.wbuf.push(u_tile);
                    let node = params.kwg[p];
                    if node.accepts() {
                        (params.record_finding)(&self.wbuf, multiplier);
                    }
                    if node.arc_index() != 0 {
                        for dr in -1..=1 {
                            for dc in -1..=1 {
                                self.iter_embedded_words(
                                    params,
                                    (row as isize + dr) as usize,
                                    (col as isize + dc) as usize,
                                    p,
                                    multiplier,
                                );
                            }
                        }
                    }
                }
            }
        }
        self.wbuf.truncate(orig_len);
        self.ubuf[idx] = false;
    }

    fn find_embedded_words<M: Fn(usize) -> i8, F: FnMut(&[u8], i8)>(
        &mut self,
        params: &mut FindEmbeddedWordParams<'_, M, F>,
    ) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                self.iter_embedded_words(params, r, c, 0, 1);
            }
        }
    }
}

fn test_find_embedded_words<'a>(
    alphabet: &alphabet::Alphabet,
    kwg: &kwg::Kwg,
    board_strs: impl IntoIterator<Item = &'a str>,
    board_muls: Option<&[i8]>,
) -> error::Returns<()> {
    let mut board = Vec::new();
    let alphabet_reader = alphabet::AlphabetReader::new_for_words(alphabet);
    let q_tile = alphabet_reader.next_tile(b"Q", 0).map(|x| x.0);
    let u_tile = alphabet_reader.next_tile(b"U", 0).map(|x| x.0);
    let mut ewf = EmbeddedWordsFinder::new(q_tile, u_tile);
    for board_str in board_strs {
        parse_embedded_words_board(&alphabet_reader, board_str, &mut board)?;
        let board_len = board.len();
        let effective_board_muls = match board_muls {
            Some(muls) if board_len == muls.len() => board_muls,
            _ => None,
        };
        let board_dim = isqrt(board_len);
        if board_dim * board_dim != board_len {
            wolges::return_error!(format!("{board_str} length {board_len} is not a square"));
        }
        let rows = board_dim;
        let cols = board_dim;
        println!("Board:");
        for r in 0..rows {
            print!("  ");
            for c in 0..cols {
                let tile = board[r * cols + c];
                print!(
                    "{}{}",
                    if tile == 0 {
                        "#"
                    } else {
                        alphabet.of_rack(tile).unwrap()
                    },
                    if matches!(q_tile, Some(q_tile) if q_tile == tile) {
                        'u'
                    } else {
                        ' '
                    }
                );
            }
            println!();
        }
        println!();
        ewf.resize(rows, cols);
        let mut ans_map = fash::MyHashMap::default();
        let t0 = std::time::Instant::now();
        ewf.find_embedded_words(&mut FindEmbeddedWordParams {
            board: &board,
            kwg,
            get_multiplier_at: &|idx| match effective_board_muls {
                Some(v) => v[idx],
                None => 1,
            },
            record_finding: &mut |word, mul| {
                let v = mul as i32
                    * match word.len() {
                        3 => 1,
                        4 => 2,
                        5 => 4,
                        x if x >= 6 => 7 + (x - 6) * 4,
                        _ => 0,
                    } as i32;
                ans_map
                    .entry(word.into())
                    .and_modify(|e| {
                        if *e < v {
                            *e = v
                        }
                    })
                    .or_insert(v);
            },
        });
        println!("Found {} words in {:?}", ans_map.len(), t0.elapsed());
        let mut ans: Box<[(bites::Bites, _)]> = ans_map.into_iter().collect();
        ans.sort_unstable_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        let mut pt = 0;
        while pt < ans.len() {
            let cur_len = ans[pt].0.len();
            let pt2 = pt + ans[pt..].partition_point(|x| x.0.len() == cur_len);
            println!("{} words of length {}:", pt2 - pt, cur_len);
            print!(" ");
            for (word, score) in &ans[pt..pt2] {
                print!(" ");
                for &tile in &word[..] {
                    print!("{}", alphabet.of_rack(tile).unwrap());
                }
                print!(" ({score})");
            }
            println!();
            pt = pt2;
        }
        println!();
    }
    Ok(())
}

fn main() -> error::Returns<()> {
    if false {
        let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS44.kwg")?);
        print_dawg(&alphabet::make_polish_alphabet(), &kwg);
        return Ok(());
    }
    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW21.kwg")?);
    if true {
        let alphabet = alphabet::make_english_alphabet();
        let nwl18_kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/NWL18.kwg")?);
        let twl14_kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/TWL14.kwg")?);
        let known_boards = [
            &[
                1, 1, 1, 1, //
                1, 1, 1, 1, //
                1, 1, 1, 1, //
                1, 1, 1, 1, //
            ][..],
            &[
                0, 0, 2, 1, 0, 0, //
                0, 1, 1, 1, 1, 0, //
                1, 1, 1, 1, 1, 1, //
                1, 1, 1, 1, 1, 1, //
                0, 1, 1, 1, 1, 0, //
                0, 0, 1, 2, 0, 0, //
            ][..],
            &[
                3, 1, 1, 1, 0, 0, //
                1, 1, 1, 1, 0, 0, //
                1, 1, 1, 1, 1, 1, //
                1, 1, 1, 1, 1, 1, //
                0, 0, 1, 1, 1, 1, //
                0, 0, 1, 1, 1, 3, //
            ][..],
            &[
                2, 1, 1, 1, 1, 3, //
                1, 1, 1, 1, 1, 1, //
                1, 1, 0, 0, 1, 1, //
                1, 1, 0, 0, 1, 1, //
                1, 1, 1, 1, 1, 1, //
                3, 1, 1, 1, 1, 2, //
            ][..],
        ];
        test_find_embedded_words(
            &alphabet,
            &nwl18_kwg,
            ["LIASERTAPGADID##KEMA##IRAIVZQAEFEGSY"],
            Some(known_boards[3]),
        )?;
        test_find_embedded_words(&alphabet, &twl14_kwg, ["LIASERTAIDKEMAIR"], None)?;
        test_find_embedded_words(
            &alphabet,
            &kwg,
            "
NQALBRYUDAMEWPAI
YELWRNOILEDAUMRRSOFOJEOAW
JAICOHDOOEGATBITETYREBRXALKNQURODNEE
WFRWFTOEYTAUELLDOANDLOMVTNIADEIDAONZ
MLEGTEBRWWAUIRFPODKXLIVIO
AFEEIUTSELRIANSAAUIEIVQDLOTCEFVEIIHN
ETARBAISAIHHOAEM
BNLWRQTTWEGIORUE
AAVOYREMSELEZLQIHEERICDICGWKWRNLEBPD
SPXYIERKBATDBJWR
AFSOGXNLESGTIRDD
SMIAGIDTGEREDNES
XNMTEAMGJCYPTDSUAVOLWARISQEOHUDIEFILRNRLEOWGECIODAHASGRIBNFETTRD
OEKOAUEJDIAQINBE
GOAEOURDOBALIEOH
TNXNOSDDDQIRAEIT
LWAURIHTSAALOITO
BSROBPDQUZATASOT
HIOUAIBAEUUIIOAE
ZCLONAUGEIRIINNE
IAGTENTARNAUNAER
CRIGIYSTVAEUEEPLLUIAOEDKLPJAOLEIAINEWRNUORNHOETRHEDGFAEIODAIUXEZ
RCNVFODIAYSNSTSY
RGIANBDYRMROCVQR
AEAREVWRSOEOOASR
LSEOBASRUAYEYEIO
POACNFETUIAERNLOOWORTNHIYUFCIGSDXVEALEGALDSHOISKURDGEELOAYMDEPMI
#CSRGGPUUETIDIN#
A#GHT#NLVFUAA#O#
IU#RO##ZDGAWO#II
VORU#ITOES#TOTEA
FEM#ISMEPNJRIEL#OVTYWEOEAWVEHNDSIZKGASIHT#OLOD#RGNCOC##ENL#AAPR#
TABCEEAWNREEOSRLLDGN####F#UN#OIAESTYNKACEPIHUEIST#AZRET#IDTYMUOE
        "
            .split_whitespace(),
            None,
        )?;
        return Ok(());
    }
    let game_config = &game_config::make_english_game_config();

    print_dawg(game_config.alphabet(), &kwg);
    let t0 = std::time::Instant::now();
    let word_counts = kwg.count_words_alloc();
    println!("took {} ms", t0.elapsed().as_millis());
    println!("{:?}", &word_counts[0..100]);
    let mut out_vec = Vec::new();
    let dawg_root = kwg[0].arc_index();
    for i in 0..word_counts[dawg_root as usize] {
        out_vec.clear();
        kwg.get_word_by_index(&word_counts, dawg_root, i, |v| {
            out_vec.push(v);
        });
        let j = kwg.get_word_index(&word_counts, dawg_root, &out_vec);
        println!("{i} {j} {out_vec:?}");
        assert_eq!(i, j);
    }
    assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[5, 3, 1]), !0);
    assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[]), !0);
    assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[1, 3]), !0);

    Ok(())
}
