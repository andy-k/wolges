#[macro_use]
mod error;

mod alphabet;
mod bites;
mod board_layout;
mod build;
mod display;
mod game_config;
mod klv;
mod kwg;
mod main_auto;
mod main_build;
mod main_lex;
mod matrix;
mod movegen;

fn main() -> error::Returns<()> {
    if false {
        main_build::main()?;
    }

    if false {
        main_lex::main()?;
        return Ok(());
    }

    if true {
        main_auto::main()?;
    }

    println!("Hello, world!");
    Ok(())
}
