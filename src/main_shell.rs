// Copyright (C) 2020-2026 Andy Kurnia.

use wolges::error;
mod endgame_watch;
mod rlhelper;
mod shell_session;

use std::io::BufRead;
use std::io::Write;

// --help says what the shell takes, --version which commit it was built from
#[derive(clap::Parser)]
#[command(
    about = "an interactive shell that keeps a game its commands build (help lists them)",
    version = wolges::VERSION
)]
struct Cli {
    #[arg(long, help = "print no progress")]
    quiet: bool,
    #[arg(
        long,
        value_name = "GB",
        default_value_t = endgame_watch::DEFAULT_MAX_MEMORY_GB,
        help = "stop an endgame solve once the program uses GB gigabytes (10^9 bytes each; 0: \
                never)"
    )]
    max_memory: f64,
}

fn main() -> error::Returns<()> {
    let cli: Cli = clap::Parser::parse();
    let mut session =
        shell_session::Session::new(cli.quiet, endgame_watch::memory_limit(cli.max_memory)?)?;
    let mut rl = rlhelper::new_rl_editor()?;
    let mut cmd_stack = Vec::<(String, Option<(String, usize)>)>::new();
    loop {
        if let Some((line, source)) = cmd_stack.pop() {
            if let Some((filename, line_num)) = &source {
                writeln!(std::io::stdout(), "{filename}:{line_num}> {line}")?;
            }
            let split = if cfg!(windows) {
                split_keeping_backslashes(&line)
            } else {
                shell_words::split(&line)
            };
            match split {
                Ok(strings) => {
                    if !strings.is_empty() {
                        match strings[0].as_str() {
                            "help" => {
                                let width = shell_session::COMMANDS
                                    .iter()
                                    .map(|(command, _)| command.len())
                                    .max()
                                    .unwrap_or(0);
                                for (command, what) in shell_session::COMMANDS {
                                    writeln!(std::io::stdout(), "{command:<width$}  {what}")?;
                                }
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
                                        writeln!(std::io::stdout(), "cannot read file: {err:?}")?;
                                    }
                                } else {
                                    writeln!(std::io::stdout(), "need another arg")?;
                                }
                            }
                            _ => {
                                // a line from a file replays what was logged,
                                // including the computer's moves
                                let typed = source.is_none();
                                if let Err(err) = session.run(&line, &strings, typed) {
                                    writeln!(std::io::stdout(), "error: {err}")?;
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    writeln!(std::io::stdout(), "Bad quoting: {err:?}")?;
                }
            }
        } else {
            match rl.readline(">> ") {
                Ok(line) => {
                    rl.add_history_entry(line.as_str())?;
                    cmd_stack.push((line, None));
                }
                Err(rustyline::error::ReadlineError::Interrupted) => {
                    writeln!(std::io::stdout(), "CTRL-C")?;
                    break;
                }
                Err(rustyline::error::ReadlineError::Eof) => {
                    writeln!(std::io::stdout(), "CTRL-D")?;
                    break;
                }
                Err(err) => {
                    writeln!(std::io::stdout(), "Error: {err:?}")?;
                    break;
                }
            }
        }
    }

    Ok(())
}

// shell_words::split, but with a backslash an ordinary character, as it is in a
// Windows path (C:\a\b.txt): '...' and "..." group a word, with no escape in
// either, and a # that begins a word begins a comment
fn split_keeping_backslashes(line: &str) -> Result<Vec<String>, shell_words::ParseError> {
    enum At {
        Gap,
        Word,
        Quoted(char),
        Comment,
    }
    let mut words = Vec::new();
    let mut word = String::new();
    let mut at = At::Gap;
    for c in line.chars() {
        at = match at {
            At::Gap => match c {
                ' ' | '\t' | '\n' => At::Gap,
                '#' => At::Comment,
                '\'' | '"' => At::Quoted(c),
                _ => {
                    word.push(c);
                    At::Word
                }
            },
            At::Word => match c {
                ' ' | '\t' | '\n' => {
                    words.push(std::mem::take(&mut word));
                    At::Gap
                }
                '\'' | '"' => At::Quoted(c),
                _ => {
                    word.push(c);
                    At::Word
                }
            },
            At::Quoted(q) => {
                if c == q {
                    At::Word
                } else {
                    word.push(c);
                    At::Quoted(q)
                }
            }
            At::Comment => {
                if c == '\n' {
                    At::Gap
                } else {
                    At::Comment
                }
            }
        };
    }
    match at {
        At::Quoted(_) => Err(shell_words::ParseError),
        At::Word => {
            words.push(word);
            Ok(words)
        }
        At::Gap | At::Comment => Ok(words),
    }
}

#[cfg(test)]
mod tests {
    use super::split_keeping_backslashes;

    #[test]
    #[inline]
    fn a_windows_path_keeps_its_backslashes() {
        // each of these, shell_words splits with a backslash dropped
        for (line, words) in [
            (r"source C:\a\b.txt", [r"source", r"C:\a\b.txt"]),
            (r"source \\host\c\b.txt", [r"source", r"\\host\c\b.txt"]),
            (r#"source "\\host\c\b.txt""#, [r"source", r"\\host\c\b.txt"]),
            (r"source C:\b.txt # its words", [r"source", r"C:\b.txt"]),
            (r"source C:\a\", [r"source", r"C:\a\"]),
        ] {
            assert_eq!(split_keeping_backslashes(line).unwrap(), words, "{line}");
            assert_ne!(shell_words::split(line).unwrap(), words, "{line}");
        }
        // and these it splits the same
        for (line, words) in [
            (r#"source "C:\a b\c.txt""#, [r"source", r"C:\a b\c.txt"]),
            (r"source 'C:\a b\c.txt'", [r"source", r"C:\a b\c.txt"]),
        ] {
            assert_eq!(split_keeping_backslashes(line).unwrap(), words, "{line}");
            assert_eq!(shell_words::split(line).unwrap(), words, "{line}");
        }
        assert!(split_keeping_backslashes(r#"source "C:\x.txt"#).is_err());
    }

    #[test]
    #[inline]
    fn a_line_with_no_backslash_splits_as_shell_words_splits_it() {
        for line in [
            "",
            "help",
            "  source   a.txt  ",
            "source 'a b.txt'",
            r#"x "a'b" 'c"d' e"f g"h"#,
            "a#b # c",
            "'' \"\" x''y",
            "a\tb\nc",
            "# all of it",
        ] {
            assert_eq!(
                split_keeping_backslashes(line).unwrap(),
                shell_words::split(line).unwrap(),
                "{line:?}"
            );
        }
        for line in ["'a", "\"a", "x 'y z"] {
            assert!(split_keeping_backslashes(line).is_err());
            assert!(shell_words::split(line).is_err());
        }
    }
}
