# Wolges

## About

Wolges, a library of Andy Kurnia's experiments to understand how to better play
the Orthogonal Morphemes Game of Words, also known as OMGWords.

Backronym idea: Weaving Out Letters Gathering Expected Scores.

It is named after Woogles.io, the best site to play the game on.

## License

Copyright (C) 2020-2026 Andy Kurnia.\
Released under the MIT license.

Bugs included.

## Initial Setup

```
brew install rustup-init
rustup-init
(accept defaults)
(restart shell to activate .profile)

(cd src for convenience)
(put some word list .txt files and leaves.csv files in current directory)
#cargo run --release --bin buildlex -- klv --klv 1 leaves.csv leaves.klv
cargo run --release --bin buildlex -- klv leaves.csv leaves.klv2
cargo run --release --bin buildlex -- kwg CSW19.txt CSW19.kwg
cargo run --release --bin buildlex -- macondo CSW19.kwg CSW19 CSW19.dawg CSW19.gaddag
cargo run --release --bin buildlex -- kwg --words alphagrams CSW19.txt CSW19.kad
```

## Running

```
cargo run --release
```

## Quick start

From src, with CSW24.kwg there (see Initial Setup).

The strongest leaves measured, the census and self-play mixed (hours):

```
mkdir t && cd t && cp -ip ../CSW24.kwg . && ../../genblend.sh --preset english
```

Good leaves in minutes, the census alone:

```
cargo run --release --bin leave -- census CSW24.kwg - - 3x256,2048
```

Two leave tables played against each other:

```
cargo run --release --bin leave -- compare CSW24.kwg a.klv2 b.klv2
```

Every binary and every task takes --help; lab/*.txt explains the methods.

## Details

Details are not in this short readme.

## GitHub Badge

- [![Actions](https://github.com/andy-k/wolges/actions/workflows/actions.yml/badge.svg)](https://github.com/andy-k/wolges/actions/workflows/actions.yml)
