#!/usr/bin/env bash
# ./build-liwords-kwg.sh english something.txt CSW19X

set -euo pipefail

lexlang="$1"
infile="$2"
lexname="$3"

liwordsdir="../liwords"
dawgpath="${liwordsdir}/data/lexica/dawg/${lexname}.dawg"
gaddagpath="${liwordsdir}/liwords-ui/public/wasm/${lexname}.gaddag"
kwgpath="${liwordsdir}/liwords-ui/public/wasm/${lexname}.kwg"

cargo run --release --bin buildlex -- "${lexlang}-kwg" "$infile" "$kwgpath"
cargo run --release --bin buildlex -- "${lexlang}-macondo" "$kwgpath" "$lexname" "$dawgpath" "$gaddagpath"
