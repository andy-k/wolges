#!/usr/bin/env bash
# ./build-pgo-simmer.sh KWG KLV
#
# A profile-guided build of simmer for this machine, by hand only (CI and the
# release stay plain): an instrumented build, a training run simulating the
# built-in position (seeds 1 and 2; seed 1 again on the lean arm; one thread
# each), the profiles merged with rustup's llvm-profdata (rustup component
# add llvm-tools), then the final build. It writes only under target/ and
# stops if target/pgo-sim-raw is already there.

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: ./build-pgo-simmer.sh KWG KLV" >&2
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
raw="${PWD}/target/pgo-sim-raw"
if [ -e "$raw" ]; then
  echo "${raw} is already there: remove it first" >&2
  exit 2
fi
mkdir -p "$raw"

# RUSTFLAGS takes the place of .cargo/config.toml's rustflags, so
# target-cpu=native is written in beside the profile flags.
RUSTFLAGS="-C target-cpu=native -Cprofile-generate=${raw}" \
  cargo build --release --target-dir target/pgo-sim-gen --bin simmer
target/pgo-sim-gen/release/simmer "$kwg" "$klv" 1 >/dev/null
target/pgo-sim-gen/release/simmer "$kwg" "$klv" 2 >/dev/null
target/pgo-sim-gen/release/simmer --lean "$kwg" "$klv" 1 >/dev/null
"$profdata" merge -o "${raw}/merged.profdata" "$raw"
RUSTFLAGS="-C target-cpu=native -Cprofile-use=${raw}/merged.profdata" \
  cargo build --release --target-dir target/pgo-sim --bin simmer
echo "built target/pgo-sim/release/simmer"
