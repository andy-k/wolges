#!/usr/bin/env bash
# ./build-pgo-endgame.sh CSW24.kwg BATCH
#
# A profile-guided build of endgame for this machine, by hand only (CI and the
# release stay plain): an instrumented build, a training run solving every
# position in BATCH (an endgame --batch file; one thread), the profile merged
# with rustup's llvm-profdata (rustup component add llvm-tools), then the
# final build. It writes only under target/ and stops if target/pgo-eg-raw is
# already there.

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: ./build-pgo-endgame.sh KWG BATCH" >&2
  exit 2
fi
kwg="$1"
batch="$2"

host="$(rustc -vV | sed -n 's/^host: //p')"
profdata="$(rustc --print sysroot)/lib/rustlib/${host}/bin/llvm-profdata"
if [ ! -x "$profdata" ]; then
  echo "no llvm-profdata at ${profdata}: rustup component add llvm-tools" >&2
  exit 2
fi
raw="${PWD}/target/pgo-eg-raw"
if [ -e "$raw" ]; then
  echo "${raw} is already there: remove it first" >&2
  exit 2
fi
mkdir -p "$raw"

# RUSTFLAGS takes the place of .cargo/config.toml's rustflags, so
# target-cpu=native is written in beside the profile flags.
RUSTFLAGS="-C target-cpu=native -Cprofile-generate=${raw}" \
  cargo build --release --target-dir target/pgo-eg-gen --bin endgame
target/pgo-eg-gen/release/endgame "$kwg" --batch "$batch" --quiet >/dev/null
"$profdata" merge -o "${raw}/merged.profdata" "$raw"
RUSTFLAGS="-C target-cpu=native -Cprofile-use=${raw}/merged.profdata" \
  cargo build --release --target-dir target/pgo-eg --bin endgame
echo "built target/pgo-eg/release/endgame"
