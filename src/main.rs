mod alphabet;
mod board_layout;
mod display;
mod game_config;
mod gdw;
mod matrix;

use game_config::GameConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gdw = gdw::from_bytes(&std::fs::read("csw19.gdw")?);
    println!("{}", gdw.0.len());
    //let gc  : &dyn game_config::GameConfig= &game_config::COMMON_ENGLISH_GAME_CONFIG;
    let gc  = &game_config::COMMON_ENGLISH_GAME_CONFIG;
    let bl = &gc.board_layout();
    let dim = bl.dim();
    for r in 0..dim.rows {
    for c in 0..dim.cols {
    print!("{}",display:: empty_label(&bl,r,c));
    }
    println!(" = {}", r)
    }
    println!("Hello, world!");
    Ok(())
}
