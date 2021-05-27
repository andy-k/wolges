// Copyright (C) 2020-2021 Andy Kurnia.

use wolges::{alphabet, error, game_config, kwg};

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

pub fn main() -> error::Returns<()> {
    if false {
        let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/OSPS42.kwg")?);
        print_dawg(&alphabet::make_polish_alphabet(), &kwg);
        return Ok(());
    }
    let kwg = kwg::Kwg::from_bytes_alloc(&std::fs::read("lexbin/CSW19.kwg")?);
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
