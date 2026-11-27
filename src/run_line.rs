// Copyright (C) 2020-2026 Andy Kurnia.

use std::fmt::Write;

// each input a run has read: its path, its bytes, and its hash.
static INPUT_DIGESTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[inline]
pub fn note_input(
    path: &(impl std::fmt::Debug + ?Sized),
    len: usize,
    hasher: &wolges::fash::MyHasher,
) {
    let digest = format!(
        "{path:?}[{len} bytes {:016x}]",
        std::hash::Hasher::finish(hasher)
    );
    if let Ok(mut digests) = INPUT_DIGESTS.lock() {
        digests.push(digest);
    }
}

#[inline]
pub fn read_input<P: AsRef<std::path::Path>>(path: P) -> Result<Vec<u8>, std::io::Error> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let mut hasher = wolges::fash::MyHasher::default();
    std::hash::Hasher::write(&mut hasher, &bytes);
    note_input(path, bytes.len(), &hasher);
    Ok(bytes)
}

pub fn run_line(args: &[std::ffi::OsString]) -> String {
    let mut line = format!("run: {}", wolges::VERSION);
    for arg in args {
        write!(line, " {arg:?}").ok();
    }
    if let Ok(digests) = INPUT_DIGESTS.lock() {
        for digest in digests.iter() {
            write!(line, " {digest}").ok();
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_line_names_the_build_and_each_input_read() {
        let line = run_line(&[]);
        assert!(line.starts_with(&format!("run: {}", wolges::VERSION)));
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        let bytes = read_input(path).unwrap();
        let line = run_line(&["x".into()]);
        assert!(line.starts_with(&format!("run: {} \"x\"", wolges::VERSION)));
        assert!(line.contains(&format!("{path:?}[{} bytes ", bytes.len())));
    }
}
