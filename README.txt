ABOUT

Wolges, a library of Andy Kurnia's experiments to understand how to better play
the Orthogonal Morphemes Game of Words, also known as OMGWords.

It is named after Woogles.io, the best site to play the game on.


LICENSE

Copyright (C) 2020-2021 Andy Kurnia.
Released under the MIT license.

Bugs included.


INITIAL SETUP

brew install rustup-init
rustup-init
(accept defaults)
(restart shell to activate .profile)

(cd src for convenience)
(put some word list .txt files and leaves.csv files in current directory)
cargo run --release --bin buildlex -- english-klv leaves.csv leaves.klv
cargo run --release --bin buildlex -- english-kwg csw19.txt csw19.kwg
cargo run --release --bin buildlex -- english-macondo csw19.kwg CSW19 CSW19.dawg CSW19.gaddag


RUNNING

cargo run --release


DETAILS

Details are not in this short readme.
