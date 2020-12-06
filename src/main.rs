mod alphabet;
mod board_layout;
mod game_config;
mod gdw;
mod matrix;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gdw = gdw::from_bytes(&std::fs::read("csw19.gdw")?);
    println!("{}", gdw.0.len());
    println!("{}", std::mem::size_of_val(&game_config::COMMON_ENGLISH_GAME_CONFIG));
    println!("Hello, world!");
    Ok(())
}
