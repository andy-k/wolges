#!/usr/bin/env bash

set -euo pipefail

recipe="3x256,2048"
weight="0.5"
census_seed=""
threads=""
while :; do
  if [ "${1:-}" = "--" ]; then
    shift
    break
  fi
  if [ "${1:-}" = "--recipe" ]; then
    recipe="${2:?--recipe needs a board count list}"
    shift 2
    continue
  fi
  if [ "${1:-}" = "--weight" ]; then
    weight="${2:?--weight needs a number}"
    shift 2
    continue
  fi
  if [ "${1:-}" = "--census-seed" ]; then
    census_seed="${2:?--census-seed needs a number}"
    shift 2
    continue
  fi
  if [ "${1:-}" = "--threads" ]; then
    threads="${2:?--threads needs a number}"
    shift 2
    continue
  fi
  break
done

if [ "$#" -lt 2 ]; then
  cat <<"EOF"
usage:
  mkdir t
  cd t
  cp -ip ../.../CSW24.kwg .
  ../genblend.sh english english
usage 2: (a language whose word graph is a .kbwg)
  cp -ip ../.../DSW25.kbwg .
  ../genblend.sh dutch dutch
usage 3: (spell out the self-play generations: games[:min_samples_per_rack])
  ../genblend.sh english english 100000000:500 100000000:1000
usage 4: (a small run of every stage)
  ../genblend.sh --recipe 64,256 english english 200000:100 200000:200
options:
  --recipe LIST     board counts for the census, comma separated, each N or KxN
                    (K generations of N boards). Default 3x256,2048.
  --weight W        how much of the self-play table to take in the blend: 0 is
                    the census alone, 1 self-play alone. Default 0.5.
  --census-seed S   fix the census's seed. Omitted means the census picks one
                    and prints it.
  --threads N       worker threads for every leave run. Default every core.
defaults, spelled out: the census recipe above, then two self-play generations
of 100 million games each, the first asking for 500 samples of every rack and
the second 1000, then a half-and-half blend.
EOF
  exit 2
fi

leave_param="$1"
buildlex_param="$2"
shift 2

# a config word as leave's options: the word is a preset, and a jumbled-
# prefix adds --jumbled
leave_options=(--preset "${leave_param#jumbled-}")
if [ "${leave_param#jumbled-}" != "$leave_param" ]; then
  leave_options+=(--jumbled)
fi

if [ "$#" -eq 0 ]; then
  set -- 100000000:500 100000000:1000
fi

kwg=""
for x in *.kwg *.kbwg; do
  if [ ! -f "$x" ]; then
    :
  elif [ ! "$kwg" ]; then
    kwg="$x"
  else
    echo "there must be exactly 1 kwg here (found multiple)" >&2
    exit 1
  fi
done
if [ ! "$kwg" ]; then
  echo "there must be exactly 1 kwg here (found none)" >&2
  exit 1
fi

kbwg_flag=""
if [ "$kwg" != "${kwg%.kbwg}" ]; then
  kbwg_flag="--kbwg"
fi

echo "$kwg"

for full_arg in "$@"; do
  before_colon="${full_arg%%:*}"
  if [ "${before_colon}" != "$[${before_colon} + 0]" ]; then
    echo "invalid number of games: ${before_colon}" >&2
    exit 1
  fi
  if [ "${full_arg}" != "${before_colon}" ]; then
    after_colon="${full_arg#*:}"
    if [ "${after_colon}" != "$[${after_colon} + 0]" ]; then
      echo "invalid number of samples: ${after_colon}" >&2
      exit 1
    fi
  fi
done

census_subcommand="census"
autoplay_subcommand="autoplay-summarize-only"
generate_subcommand="generate"
buildlex_subcommand="${buildlex_param}-klv2"
blend_subcommand="${buildlex_param}-blend"

# the census stamps its output with the run; copy it to a fixed name
if [ ! -e census.klv2 ]; then
  time cargo run --release --bin leave -- "${leave_options[@]}" ${kbwg_flag:+"$kbwg_flag"} ${threads:+--threads "$threads"} \
    "$census_subcommand" "$kwg" - - "$recipe" ${census_seed:+"$census_seed"}
  census_file="$(ls -1td census-leaves-*.klv2 2>/dev/null | head -1)"
  if [ ! "$census_file" ]; then
    echo "the census wrote no leave table" >&2
    exit 1
  fi
  echo "$census_file"
  cp -ipv "$census_file" census.klv2
fi

# self-play, each generation seeded by the table before it
last_leave="census.klv2"
num_processed=0
for full_arg in "$@"; do
  let num_processed=num_processed+1
  before_colon="${full_arg%%:*}"
  if [ "${full_arg}" != "${before_colon}" ]; then
    after_colon="${full_arg#*:}"
  else
    after_colon="0"
  fi

  # continue from previous run if found
  if [ -e "leaves${num_processed}.klv2" ]; then
    echo "leaves${num_processed}.klv2 exists, keeping it"
    last_leave="leaves${num_processed}.klv2"
    continue
  fi

  time cargo run --release --bin leave -- "${leave_options[@]}" ${kbwg_flag:+"$kbwg_flag"} ${threads:+--threads "$threads"} \
    "$autoplay_subcommand" "$kwg" "$last_leave"{,} "$before_colon" "$after_colon"
  log_file="$(ls -1td summary-log-* | head -1 | cut -f2- -d-)"
  echo "$log_file"
  mv -fv "summary-${log_file}" "summary${num_processed}.csv"
  if [ -f "games-${log_file}" ]; then
    rm -fv "games-${log_file}"
  fi
  time cargo run --release --bin leave -- "${leave_options[@]}" ${threads:+--threads "$threads"} \
    "$generate_subcommand" "summary${num_processed}.csv" "leaves${num_processed}.csv"
  time cargo run --release --bin buildlex -- "$buildlex_subcommand" \
    "leaves${num_processed}".{csv,klv2}
  last_leave="leaves${num_processed}.klv2"
done

# the blend
time cargo run --release --bin buildlex -- "$blend_subcommand" \
  census.klv2 "$last_leave" "$weight" blend.klv2
echo "blend.klv2 takes ${weight} of ${last_leave} and the rest of census.klv2"
ls -l census.klv2 "$last_leave" blend.klv2
