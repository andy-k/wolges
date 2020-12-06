mod alphabet;
mod board_layout;
mod display;
mod game_config;
mod gdw;
mod matrix;

fn print_dawg<'a>(a: &alphabet::Alphabet<'a>, g: &gdw::Gdw) {
    struct Env<'a> {
        a: &'a alphabet::Alphabet<'a>,
        g: &'a gdw::Gdw,
        s: &'a mut String,
    }
    fn iter(env: &mut Env, mut p: u32) {
        let l = env.s.len();
        loop {
            let t=env.g[p].tile();
            env.s.push_str(
              if t == 0 { "@" }
              else if t & 0x80 == 0 { env.a.from_board(t).unwrap() }
              else { panic!() }
            );
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
        g[0u32].arc_index(),
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gdw = gdw::from_bytes(&std::fs::read("csw19.gdw")?);
    let gc = &game_config::COMMON_ENGLISH_GAME_CONFIG;
    print_dawg(gc.alphabet(), &gdw);
    println!("{}", gdw.0.len());
    let bl = gc.board_layout();
    let dim = bl.dim();
    for r in 0..dim.rows {
        for c in 0..dim.cols {
            print!("{}", display::empty_label(bl, r, c));
        }
        println!(" = {}", r)
    }

    let board =
        b"\
\x0f\x04\x00\x00\x00\x00\x08\x01\x12\x0c\x0f\x14\x13\x00\x00\
\x06\x09\x0e\x00\x00\x00\x00\x00\x00\x00\x00\x17\x00\x00\x00\
\x00\x14\x05\x05\x00\x07\x00\x00\x00\x00\x00\x09\x00\x00\x00\
\x00\x1a\x01\x18\x00\x12\x00\x00\x00\x00\x00\x03\x00\x00\x00\
\x00\x00\x14\x0f\x12\x09\x00\x00\x03\x00\x04\x05\x00\x00\x00\
\x00\x00\x0c\x00\x01\x0d\x00\x14\x15\x0e\x01\x00\x00\x00\x00\
\x00\x00\x19\x05\x0e\x00\x11\x09\x0e\x00\x08\x00\x00\x00\x00\
\x00\x00\x00\x16\x09\x02\x09\x13\x14\x00\x0c\x00\x00\x00\x00\
\x00\x00\x00\x05\x00\x00\x00\x00\x00\x00\x09\x00\x00\x00\x00\
\x00\x00\x00\x0a\x01\x19\x00\x00\x00\x0e\x01\x00\x00\x00\x00\
\x00\x00\x00\x01\x0d\x05\x00\x00\x06\x01\x13\x00\x00\x00\x00\
\x00\x00\x00\x12\x05\x10\x00\x12\x15\x0e\x00\x00\x00\x00\x00\
\x00\x00\x0f\x00\x02\x00\x00\x00\x07\x00\x00\x00\x00\x00\x00\
\x00\x00\x17\x12\x01\x10\x00\x00\x00\x00\x00\x00\x00\x00\x00\
\x0f\x0b\x05\x00\x05\x09\x04\x05\x00\x00\x00\x00\x00\x00\x00\
";
    {
      let al = gc.alphabet();
      let bl = gc.board_layout();
      println!("---dim---");
      let dim = bl.dim();
      for r in 0..dim.rows {
        for c in 0..dim.cols {
          print!("{}", al.from_board(board[dim.at_row_col(r,c)]).unwrap_or_else(|| display::empty_label(bl, r, c)));
        }
        println!(" = {}", r)
      }
      println!("---transposable---");
      let dim = bl.dim().transposable();
      for r in 0..dim.pris {
        for c in 0..dim.secs {
          print!("{}", al.from_board(board[dim.at_pri_sec(r,c)]).unwrap_or_else(|| display::empty_label(bl, r, c)));
        }
        println!(" = {}", r)
      }
      println!("---transposable'---");
      let dim = dim.transpose();
      for r in 0..dim.pris {
        for c in 0..dim.secs {
          print!("{}", al.from_board(board[dim.at_pri_sec(r,c)]).unwrap_or_else(|| display::empty_label(bl, r, c)));
        }
        println!(" = {}", r)
      }
      println!("---transposable''---");
      let dim = dim.transpose();
      for r in 0..dim.pris {
        for c in 0..dim.secs {
          print!("{}", al.from_board(board[dim.at_pri_sec(r,c)]).unwrap_or_else(|| display::empty_label(bl, r, c)));
        }
        println!(" = {}", r)
      }
    }

    println!("Hello, world!");
    Ok(())
}
