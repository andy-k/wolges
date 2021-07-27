// Copyright (C) 2020-2021 Andy Kurnia.

use wolges::{alphabet, bites, error, fash, game_config, kwg};

fn print_dawg<'a>(a: &alphabet::Alphabet<'a>, g: &kwg::Kwg) {
    struct Env<'a> {
        a: &'a alphabet::Alphabet<'a>,
        g: &'a kwg::Kwg,
        s: &'a mut String,
    }
    fn iter(env: &mut Env, mut p: i32) {
        let l = env.s.len();
        loop {
            let t = env.g[p].tile();
            env.s.push_str(if t == 0 {
                "@"
            } else if t & 0x80 == 0 {
                env.a.from_board(t).unwrap()
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
            a: &a,
            g: &g,
            s: &mut String::new(),
        },
        g[0].arc_index(),
    );
}

// parses '#' as 0
fn parse_embedded_words_board(
    alphabet_reader: &alphabet::AlphabetReader,
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
                wolges::return_error!(format!("invalid tile after {:?} in {:?}", v, s));
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

    fn iter_embedded_words<F: FnMut(&[u8])>(
        &mut self,
        board: &[u8],
        kwg: &kwg::Kwg,
        r: usize,
        c: usize,
        mut p: i32,
        f: &mut F,
    ) {
        if r >= self.rows || c >= self.cols {
            return;
        }
        let idx = r * self.cols + c;
        if self.ubuf[idx] {
            return;
        }
        let tile = board[idx];
        if tile == 0 {
            return;
        }
        p = kwg.seek(p, tile);
        if p <= 0 {
            return;
        }
        let orig_len = self.wbuf.len();
        self.ubuf[idx] = true;
        self.wbuf.push(tile);
        let node = kwg[p];
        if node.accepts() {
            f(&self.wbuf);
        }
        if node.arc_index() as i32 != 0 {
            for dr in -1..=1 {
                for dc in -1..=1 {
                    self.iter_embedded_words(
                        board,
                        kwg,
                        (r as isize + dr) as usize,
                        (c as isize + dc) as usize,
                        p,
                        f,
                    );
                }
            }
        }
        if matches!(self.q_tile, Some(q_tile) if q_tile == tile) {
            if let Some(u_tile) = self.u_tile {
                p = kwg.seek(p, u_tile);
                if p > 0 {
                    self.wbuf.push(u_tile);
                    let node = kwg[p];
                    if node.accepts() {
                        f(&self.wbuf);
                    }
                    if node.arc_index() as i32 != 0 {
                        for dr in -1..=1 {
                            for dc in -1..=1 {
                                self.iter_embedded_words(
                                    board,
                                    kwg,
                                    (r as isize + dr) as usize,
                                    (c as isize + dc) as usize,
                                    p,
                                    f,
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

    fn find_embedded_words<F: FnMut(&[u8])>(&mut self, board: &[u8], kwg: &kwg::Kwg, f: &mut F) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                self.iter_embedded_words(board, kwg, r, c, 0, f);
            }
        }
    }
}

fn test_find_embedded_words(alphabet: &alphabet::Alphabet, kwg: &kwg::Kwg) -> error::Returns<()> {
    let mut board = Vec::new();
    let alphabet_reader = alphabet::AlphabetReader::new_for_words(&alphabet);
    let q_tile = alphabet_reader.next_tile(b"Q", 0).map(|x| x.0);
    let u_tile = alphabet_reader.next_tile(b"U", 0).map(|x| x.0);
    let mut ewf = EmbeddedWordsFinder::new(q_tile, u_tile);
    for board_str in "
LIASERTAIDKEMAIR
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
    .split_whitespace()
    {
        parse_embedded_words_board(&alphabet_reader, &board_str, &mut board)?;
        let board_len = board.len();
        let board_dim = isqrt(board_len);
        if board_dim * board_dim != board_len {
            wolges::return_error!(format!(
                "{} length {} is not a square",
                board_str, board_len
            ));
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
                        alphabet.from_rack(tile).unwrap()
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
        let mut ans_set = fash::MyHashSet::default();
        let t0 = std::time::Instant::now();
        ewf.find_embedded_words(&board, &kwg, &mut |word| {
            ans_set.insert(word.into());
        });
        println!("Found {} words in {:?}", ans_set.len(), t0.elapsed());
        let mut ans: Box<[bites::Bites]> = ans_set.into_iter().collect();
        ans.sort_unstable_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(&b)));
        let mut pt = 0;
        while pt < ans.len() {
            let cur_len = ans[pt].len();
            let pt2 = pt + ans[pt..].partition_point(|x| x.len() == cur_len);
            println!("{} words of length {}:", pt2 - pt, cur_len);
            print!(" ");
            for word in &ans[pt..pt2] {
                print!(" ");
                for &tile in &word[..] {
                    print!("{}", alphabet.from_rack(tile).unwrap());
                }
            }
            println!();
            pt = pt2;
        }
        println!();
    }
    Ok(())
}

pub fn main() -> error::Returns<()> {
    if false {
        let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS42.kwg")?);
        print_dawg(&alphabet::make_polish_alphabet(), &kwg);
        return Ok(());
    }
    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW19.kwg")?);
    if true {
        let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/TWL14.kwg")?);
        let alphabet = alphabet::make_english_alphabet();
        return test_find_embedded_words(&alphabet, &kwg);
    }
    let game_config = &game_config::make_common_english_game_config();

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
        println!("{} {} {:?}", i, j, out_vec);
        assert_eq!(i, j);
    }
    assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[5, 3, 1]), !0);
    assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[]), !0);
    assert_eq!(kwg.get_word_index(&word_counts, dawg_root, &[1, 3]), !0);

    Ok(())
}
