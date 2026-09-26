#!/usr/bin/env bash

set -euo pipefail

full_mode=""
klv1_mode=""
logs_mode=""
gilles_mode=""
no_forcing_mode=""
threads=""
while :; do
  if [ "${1:-}" = "--" ]; then
    shift
    break
  fi
  if [ "${1:-}" = "--full" ]; then
    full_mode=1
    shift
    continue
  fi
  if [ "${1:-}" = "--klv1" ]; then
    klv1_mode=1
    shift
    continue
  fi
  if [ "${1:-}" = "--logs" ]; then
    logs_mode=1
    shift
    continue
  fi
  if [ "${1:-}" = "--gilles" ]; then
    gilles_mode=1
    shift
    continue
  fi
  if [ "${1:-}" = "--no-forcing" ]; then
    no_forcing_mode=1
    shift
    continue
  fi
  if [ "${1:-}" = "--threads" ]; then
    threads="${2:?--threads needs a number}"
    shift 2
    continue
  fi
  break
done

forcing_on=""
if [ -z "$gilles_mode" ] && [ -z "$no_forcing_mode" ]; then
  forcing_on=1
  export WOLGES_IMPOSSIBLE_OK=1
fi

if [ "$#" -lt 3 ]; then
  cat <<"EOF"
usage 1:
  mkdir t
  cd t
  cp -ip ../.../CSW24.kwg .
  ../genleaves.sh [options] super-english english 2000000 2000000 2000000
usage 2:
  mkdir t
  cd t
  cp -ip ../.../NSF20.kwg .
  ../genleaves.sh [options] norwegian norwegian 2000000 2000000 2000000
usage 3: (same as usage 2 but sample different number of games)
  ../genleaves.sh [options] norwegian norwegian 1000000 2000000 3000000
usage 4: (number of number of games does not have to be 3, minimum is 1)
  ../genleaves.sh [options] norwegian norwegian 100 300 600 1000
usage 2b: (same as usage 2, just use a .kbwg file instead of .kwg)
  mkdir t
  cd t
  cp -ip ../.../DSW25.kbwg .
  ../genleaves.sh [options] dutch dutch 2000000 2000000 2000000
usage 2c:
  ../genleaves.sh [options] norwegian norwegian 2000000:100 2000000:500 2000000:1000
bash allows this syntax:
  ../genleaves.sh [options] {super-,}english 2000000{,,}
  ../genleaves.sh [options] {,}norwegian 2000000{,,}
  ../genleaves.sh [options] {,}norwegian {1..3}000000
  ../genleaves.sh [options] {,}norwegian {1,3,6,10}00
  ../genleaves.sh [options] {,}dutch 2000000{,,}
  ../genleaves.sh [options] {,}norwegian 2000000:{100,500,1000}
options:
  --full        generate full-rack leaves
  --klv1        use klv1 instead of klv2 (not recommended)
  --logs        log complete games (not recommended if not needed)
  --no-forcing  disable full-rack forcing (on by default for autoplay). by
                default the autoplay path forces each undersampled full rack to
                the per-gen :min (defaulting :min to 1 = cover each
                globally-possible rack once) with impossible-tolerant placement.
                --no-forcing reverts to plain natural-rack sampling. ignored
                under --gilles.
  --threads N   worker threads for every leave run (default: every core)
  --gilles      collect samples via gillesb board-sampling instead
                of autoplay. counts take no :min_samples_per_rack.
EOF
  exit 2
fi

leave_param="$1"
buildlex_param="$2"

# a config word as leave's options: the word is a preset, and a jumbled-
# prefix adds --jumbled
leave_options=(--preset "${leave_param#jumbled-}")
if [ "${leave_param#jumbled-}" != "$leave_param" ]; then
  leave_options+=(--jumbled)
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

let i=3
while [ "${!i:-}" != "" ]; do
  full_arg="${!i}"
  before_colon="${full_arg%%:*}"
  if [ "${before_colon}" != "$[${before_colon} + 0]" ]; then
    echo "invalid number: ${before_colon}" >&2
    exit 1
  fi
  if [ "${full_arg}" != "${before_colon}" ]; then
    if [ "$gilles_mode" ]; then
      echo "--gilles takes a plain game count, not ${full_arg}" >&2
      exit 1
    fi
    after_colon="${full_arg#*:}"
    if [ "${after_colon}" != "$[${after_colon} + 0]" ]; then
      echo "invalid number: ${after_colon}" >&2
      exit 1
    fi
  fi
  let i=i+1
done

autoplay_subcommand="autoplay-summarize"
gilles_subcommand="gilles"
generate_subcommand="generate"
buildlex_subcommand="${buildlex_param}-klv2"
klv_ext="klv2"
if [ ! "$logs_mode" ]; then
  autoplay_subcommand="${autoplay_subcommand}-only"
fi
if [ "$full_mode" ]; then
  generate_subcommand="${generate_subcommand}-full"
fi
if [ "$klv1_mode" ]; then
  buildlex_subcommand="${buildlex_param}-klv"
  klv_ext="klv"
fi

num_processed=0
last_leave="-"

# continue from previous run if found
while :; do
  if [ -e "leaves$[num_processed + 1].${klv_ext}" ]; then
    last_leave="leaves$[num_processed + 1].${klv_ext}"
    let num_processed=num_processed+1
  else
    break
  fi
done

let i=3
while [ "${!i:-}" != "" ]; do
  full_arg="${!i}"
  before_colon="${full_arg%%:*}"
  if [ "${full_arg}" != "${before_colon}" ]; then
    after_colon="${full_arg#*:}"
  else
    if [ "$forcing_on" ]; then
      after_colon="1"
    else
      after_colon="0"
    fi
  fi

  effective_generate_subcommand="${generate_subcommand}"
  leave_name="leaves"

  if [ "$gilles_mode" ]; then
    time cargo run --release --bin leave -- "${leave_options[@]}" ${kbwg_flag:+"$kbwg_flag"} ${threads:+--threads "$threads"} "$gilles_subcommand" "$kwg" "$last_leave"{,} "$before_colon"
    summary_file="$(ls -1td gilles-summary-* | head -1)"
    echo "$summary_file"
    mv -fv "$summary_file" "summary${num_processed}.csv"
  else
    time cargo run --release --bin leave -- "${leave_options[@]}" ${kbwg_flag:+"$kbwg_flag"} ${threads:+--threads "$threads"} "$autoplay_subcommand" "$kwg" "$last_leave"{,} "$before_colon" "$after_colon"
    log_file="$(ls -1td games-log-* | head -1 | cut -f2- -d-)"
    echo "$log_file"
    mv -fv "summary-${log_file}" "summary${num_processed}.csv"
  fi
  last_leave="${leave_name}$[num_processed + 1]"
  time cargo run --release --bin leave -- "${leave_options[@]}" ${threads:+--threads "$threads"} "$effective_generate_subcommand" "summary${num_processed}.csv" "${last_leave}.csv"
  time cargo run --release --bin buildlex -- "$buildlex_subcommand" "$last_leave".{csv,"$klv_ext"}
  zip -9v result.zip "summary${num_processed}.csv" "$last_leave".{csv,"$klv_ext"}
  last_leave="${last_leave}.${klv_ext}"
  let num_processed=num_processed+1

  let i=i+1
done
