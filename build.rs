// Copyright (C) 2020-2026 Andy Kurnia.

// stamps every build with the commit it was built from, as the release names
// its files after it: <the commit's time in UTC>-<its short sha>, with "-dirty"
// when what builds it has uncommitted changes, then the target. Where git
// cannot say, as in a copy of the source without its repository, the commit
// reads "unknown". It never stops a build.

use std::path::Path;
use std::process::Command;

// what a build is made of: an uncommitted change to any of these makes it dirty
const BUILT_FROM: [&str; 5] = [".cargo", "Cargo.lock", "Cargo.toml", "build.rs", "src"];

// git's output, trimmed, when it succeeds; git takes no optional lock, so git
// status leaves the index as it was
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .env("TZ", "UTC0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_owned())
}

// a path to watch, only if it exists: cargo reruns a build script every build
// for one that does not
fn rerun_if_changed(path: &Path) {
    if path.exists() {
        println!("cargo::rerun-if-changed={}", path.display());
    }
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // only this package's own repository says what it was built from, never one
    // that holds it further up
    let own_repo = std::env::var("CARGO_MANIFEST_DIR")
        .ok()
        .and_then(|dir| Path::new(&dir).canonicalize().ok())
        .is_some_and(|dir| {
            git(&["rev-parse", "--show-toplevel"])
                .and_then(|top| Path::new(&top).canonicalize().ok())
                .is_some_and(|top| top == dir)
        });
    let commit = if own_repo {
        // a commit, a checkout, or a reset moves HEAD: git rewrites HEAD or
        // adds to logs/HEAD, or, where refs live in reftable/, rewrites
        // tables.list there. Not the index: git status and staging rewrite it
        // without changing what builds, and each rerun rebuilds the library and
        // every program
        if let Some(git_dir) = git(&["rev-parse", "--git-dir"]) {
            for file in ["HEAD", "logs/HEAD", "reftable/tables.list"] {
                rerun_if_changed(&Path::new(&git_dir).join(file));
            }
        }
        for path in BUILT_FROM {
            rerun_if_changed(Path::new(path));
        }
        (|| {
            let time = git(&[
                "log",
                "-1",
                "--format=%cd",
                "--date=format-local:%Y%m%dT%H%M%SZ",
            ])?;
            let sha = git(&["rev-parse", "--short", "HEAD"])?;
            let mut status = vec!["status", "--porcelain", "--untracked-files=no", "--"];
            status.extend(BUILT_FROM);
            let dirty = if git(&status)?.is_empty() {
                ""
            } else {
                "-dirty"
            };
            Some(format!("{time}-{sha}{dirty}"))
        })()
    } else {
        None
    };
    println!(
        "cargo::rustc-env=WOLGES_VERSION={} {}",
        commit.as_deref().unwrap_or("unknown"),
        std::env::var("TARGET").unwrap_or_default(),
    );
}
