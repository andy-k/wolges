// Copyright (C) 2020-2026 Andy Kurnia.

use std::io::Write;
use std::process::{Command, Stdio};

fn piped(command: &mut Command, input: &str) -> String {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn polish_tiles_pass_through_the_pipes() {
    let words = "PŁÓD\nŻÓŁW\n";
    let kwg = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("polish.kwg");
    piped(
        Command::new(env!("CARGO_BIN_EXE_buildlex"))
            .args(["--tiles", "polish", "kwg", "-"])
            .arg(&kwg),
        &words.replace('\n', "\r\n"),
    );
    let read = piped(
        Command::new(env!("CARGO_BIN_EXE_read"))
            .args(["--tiles", "polish", "kwg"])
            .arg(&kwg)
            .arg("-"),
        "",
    );
    assert_eq!(read.replace("\r\n", "\n"), words);
    let lexicon = format!("lexicon \"{}\"", kwg.display());
    let session = [
        "rules polish",
        &lexicon,
        "leaves -",
        "new",
        "rack PŁÓDŻW",
        "gen",
        "play 8F PŁÓD",
        "exit",
        "",
    ]
    .join("\r\n");
    let shown = piped(
        Command::new(env!("CARGO_BIN_EXE_shell")).arg("--quiet"),
        &session,
    );
    assert!(shown.lines().any(|line| line.ends_with(" 8F PŁÓD 24")));
    assert!(shown.contains("P Ł Ó D"));
}
