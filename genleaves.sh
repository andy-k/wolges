#!/usr/bin/env bash

set -euo pipefail

full_mode=""
klv1_mode=""
logs_mode=""
smooth_mode=""
no_smooth_mode=""
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
  if [ "${1:-}" = "--smooth" ]; then
    smooth_mode=1
    shift
    continue
  fi
  if [ "${1:-}" = "--no-smooth" ]; then
    no_smooth_mode=1
    shift
    continue
  fi
  break
done

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
  --smooth      enable smoothing (default without :min_samples_per_rack or with :0)
  --no-smooth   disable smoothing (default with :min_samples_per_rack)
EOF
  exit 2
fi

leave_param="$1"
buildlex_param="$2"

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

kbwg_modifier=""
if [ "$kwg" != "${kwg%.kbwg}" ]; then
  kbwg_modifier="-big"
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
    after_colon="${full_arg#*:}"
    if [ "${after_colon}" != "$[${after_colon} + 0]" ]; then
      echo "invalid number: ${after_colon}" >&2
      exit 1
    fi
  fi
  let i=i+1
done

autoplay_subcommand="${leave_param}${kbwg_modifier}-autoplay-summarize"
generate_subcommand="${leave_param}-generate"
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
  if [ -e "leaves-smooth$[num_processed + 1].${klv_ext}" ]; then
    last_leave="leaves-smooth$[num_processed + 1].${klv_ext}"
    let num_processed=num_processed+1
  elif [ -e "leaves$[num_processed + 1].${klv_ext}" ]; then
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
    # default
    after_colon="0"
  fi

  should_smooth=0
  if [ "$after_colon" = "0" ]; then
    should_smooth=1
  fi
  if [ "$smooth_mode" ]; then
    should_smooth=1
  fi
  if [ "$no_smooth_mode" ]; then
    should_smooth=0
  fi

  effective_generate_subcommand="${generate_subcommand}"
  leave_name="leaves-smooth"
  if [ "$should_smooth" = "0" ]; then
    # this must come after full_mode
    effective_generate_subcommand="${generate_subcommand}-no-smooth"
    leave_name="leaves"
  fi

  time cargo run --release --bin leave -- "$autoplay_subcommand" "$kwg" "$last_leave"{,} "$before_colon" "$after_colon"
  log_file="$(ls -1td games-log-* | head -1 | cut -f2- -d-)"
  echo "$log_file"
  mv -fv "summary-${log_file}" "summary${num_processed}.csv"
  last_leave="${leave_name}$[num_processed + 1]"
  time cargo run --release --bin leave -- "$effective_generate_subcommand" "summary${num_processed}.csv" "${last_leave}.csv"
  time cargo run --release --bin buildlex -- "$buildlex_subcommand" "$last_leave".{csv,"$klv_ext"}
  zip -9v result.zip "summary${num_processed}.csv" "$last_leave".{csv,"$klv_ext"}
  last_leave="${last_leave}.${klv_ext}"
  let num_processed=num_processed+1

  let i=i+1
done
