// Copyright (C) 2020-2024 Andy Kurnia.

use wolges::error;
mod rlhelper;

use std::io::BufRead;

fn main() -> error::Returns<()> {
    let mut rl = rlhelper::new_rl_editor()?;
    let mut cmd_stack = Vec::<(String, Option<(String, usize)>)>::new();
    loop {
        if let Some((line, source)) = cmd_stack.pop() {
            if let Some((filename, line_num)) = source {
                println!("{filename}:{line_num}> {line}");
            }
            match shell_words::split(&line) {
                Ok(strings) => {
                    if !strings.is_empty() {
                        match strings[0].as_str() {
                            "help" => {
                                println!("no help, try reading the source");
                            }
                            "exit" => {
                                break;
                            }
                            "source" => {
                                if strings.len() > 1 {
                                    if let Err(err) = (|| -> error::Returns<()> {
                                        let v = cmd_stack.len();
                                        for (line_num, line) in std::io::BufReader::new(
                                            std::fs::File::open(&strings[1])?,
                                        )
                                        .lines()
                                        .enumerate()
                                        {
                                            cmd_stack.push((
                                                line?.to_string(),
                                                Some((strings[1].clone(), line_num + 1)),
                                            ));
                                        }
                                        cmd_stack[v..].reverse();
                                        Ok(())
                                    })() {
                                        println!("cannot read file: {err:?}");
                                    }
                                } else {
                                    println!("need another arg");
                                }
                            }
                            _ => {
                                println!("invalid input, help for help");
                            }
                        }
                    }
                }
                Err(err) => {
                    println!("Bad quoting: {err:?}");
                }
            }
        } else {
            match rl.readline(">> ") {
                Ok(line) => {
                    rl.add_history_entry(line.as_str())?;
                    cmd_stack.push((line, None));
                }
                Err(rustyline::error::ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
                }
                Err(rustyline::error::ReadlineError::Eof) => {
                    println!("CTRL-D");
                    break;
                }
                Err(err) => {
                    println!("Error: {err:?}");
                    break;
                }
            }
        }
    }

    Ok(())
}
