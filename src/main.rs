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
            env.s.push_str(env.a.from_board(env.g[p].tile()).unwrap());
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
    println!("Hello, world!");
    Ok(())
}
