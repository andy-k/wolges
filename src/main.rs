mod alphabet;
mod board_layout;
mod display;
mod game_config;
mod gdw;
mod matrix;

fn prt(z: matrix::Strider) {
    for c in 0..z.len() {
        println!("{} = {}", c, z.at(c))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    prt(matrix::Strider {
        base: 5,
        step: 2,
        len: 15,
    });
    // row 4. base=row*cols, len=cols, step=1
    prt(matrix::Strider {
        base: 4 * 15,
        step: 1,
        len: 15,
    });
    // col 4. base=col, len=rows, step=cols
    prt(matrix::Strider {
        base: 4,
        step: 15,
        len: 15,
    });
    prt(matrix::Dim { rows: 15, cols: 15 }.across(3));
    prt(matrix::Dim { rows: 15, cols: 15 }.down(7));
    let gdw = gdw::from_bytes(&std::fs::read("csw19.gdw")?);
    println!("{}", gdw.0.len());
    let gc = &game_config::COMMON_ENGLISH_GAME_CONFIG;
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
