// Copyright (C) 2020-2021 Andy Kurnia. All rights reserved.

#[macro_use]
mod error;

mod alphabet;
mod bag;
mod bites;
mod board_layout;
mod build;
mod display;
mod game_config;
mod game_state;
mod klv;
mod kwg;
mod lexport;
mod main_auto;
mod main_build;
mod main_json;
mod main_lex;
mod matrix;
mod movegen;
mod prob;
mod stats;

fn main() -> error::Returns<()> {
    if false {
        main_build::main()?;
        return Ok(());
    }

    if false {
        main_lex::main()?;
        return Ok(());
    }

    if false {
        main_json::main()?;
        return Ok(());
    }

    if true {
        main_auto::main()?;
    }

    println!("Hello, world!");
    Ok(())
}
