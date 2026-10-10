#!/usr/bin/env bash
# ./build-pgo.sh CSW24.kwg CSW24.klv2
#
# A profile-guided build of leave for this machine, by hand only (CI and the
# release stay plain): an instrumented build, a training run on both arms
# (1000 pairs each, one thread), the profiles merged with rustup's
# llvm-profdata (rustup component add llvm-tools), then the final build.
# It writes only under target/ and stops if target/pgo-raw is already there.

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: ./build-pgo.sh KWG KLV" >&2
  exit 2
fi
kwg="$1"
klv="$2"

host="$(rustc -vV | sed -n 's/^host: //p')"
profdata="$(rustc --print sysroot)/lib/rustlib/${host}/bin/llvm-profdata"
if [ ! -x "$profdata" ]; then
  echo "no llvm-profdata at ${profdata}: rustup component add llvm-tools" >&2
  exit 2
fi
raw="${PWD}/target/pgo-raw"
if [ -e "$raw" ]; then
  echo "${raw} is already there: remove it first" >&2
  exit 2
fi
mkdir -p "$raw"

# RUSTFLAGS takes the place of .cargo/config.toml's rustflags, so
# target-cpu=native is written in beside the profile flags.
RUSTFLAGS="-C target-cpu=native -Cprofile-generate=${raw}" \
  cargo build --release --target-dir target/pgo-gen --bin leave
target/pgo-gen/release/leave --threads 1 compare "$kwg" "$klv" "$klv" 1000 1
target/pgo-gen/release/leave --threads 1 compare --lean "$kwg" "$klv" "$klv" 1000 1
"$profdata" merge -o "${raw}/merged.profdata" "$raw"
RUSTFLAGS="-C target-cpu=native -Cprofile-use=${raw}/merged.profdata" \
  cargo build --release --target-dir target/pgo --bin leave
echo "built target/pgo/release/leave"
