#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<"EOF"
usage 1:
  mkdir t
  cd t
  cp -ip ../.../CSW24.kwg .
  ../genleaves.sh [options] --preset super-english 2000000 2000000 2000000
usage 2:
  mkdir t
  cd t
  cp -ip ../.../NSF20.kwg .
  ../genleaves.sh [options] --preset norwegian 2000000 2000000 2000000
usage 3: (same as usage 2 but sample different number of games)
  ../genleaves.sh [options] --preset norwegian 1000000 2000000 3000000
usage 4: (number of number of games does not have to be 3, minimum is 1)
  ../genleaves.sh [options] --preset norwegian 100 300 600 1000
usage 2b: (same as usage 2, just use a .kbwg file instead of .kwg)
  mkdir t
  cd t
  cp -ip ../.../DSW25.kbwg .
  ../genleaves.sh [options] --preset dutch 2000000 2000000 2000000
usage 2c:
  ../genleaves.sh [options] --preset norwegian 2000000:100 2000000:500 2000000:1000
bash allows this syntax:
  ../genleaves.sh [options] --preset norwegian 2000000{,,}
  ../genleaves.sh [options] --preset norwegian {1..3}000000
  ../genleaves.sh [options] --preset norwegian {1,3,6,10}00
  ../genleaves.sh [options] --preset norwegian 2000000:{100,500,1000}
options:
  --full        generate full-rack leaves
  --klv N       the leave table buildlex writes: 2 (klv2, default), 1 (klv,
                not recommended) or 16 (klv16)
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
  the game, as leave takes it (see leave --help): --preset NAME|FILE (default
  english), --tiles NAME|FILE, --board NAME|FILE, --rack-size N, --jumbled,
  --players N, --bingo-bonus TILES:POINTS,..., --zeros-to-end N,
  --passes-to-end N, --exchange-limit N, --exchanges N|unlimited. buildlex
  reads the leaves with --tiles, else the preset's tiles.
EOF
}

full_mode=""
klv_format="2"
logs_mode=""
gilles_mode=""
no_forcing_mode=""
threads=""
preset="english"
tiles=""
jumbled=""
game_options=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --)
      shift
      break
      ;;
    --full)
      full_mode=1
      shift
      ;;
    --klv)
      klv_format="${2:?--klv needs 1, 2 or 16}"
      shift 2
      ;;
    --logs)
      logs_mode=1
      shift
      ;;
    --gilles)
      gilles_mode=1
      shift
      ;;
    --no-forcing)
      no_forcing_mode=1
      shift
      ;;
    --threads)
      threads="${2:?--threads needs a number}"
      shift 2
      ;;
    --preset)
      preset="${2:?--preset needs a name or a file}"
      shift 2
      ;;
    --tiles)
      tiles="${2:?--tiles needs a name or a file}"
      game_options+=(--tiles "$tiles")
      shift 2
      ;;
    --jumbled)
      jumbled=1
      game_options+=(--jumbled)
      shift
      ;;
    --board | --rack-size | --players | --bingo-bonus | --zeros-to-end | --passes-to-end | --exchange-limit | --exchanges)
      game_options+=("$1" "${2:?$1 needs a value}")
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*)
      echo "unknown option $1" >&2
      exit 2
      ;;
    *)
      break
      ;;
  esac
done
leave_options=(--preset "$preset" "${game_options[@]}")

case "$klv_format" in
  1) klv_ext="klv" ;;
  2) klv_ext="klv2" ;;
  16) klv_ext="klv16" ;;
  *)
    echo "--klv takes 1, 2 or 16, not $klv_format" >&2
    exit 2
    ;;
esac

forcing_on=""
if [ -z "$gilles_mode" ] && [ -z "$no_forcing_mode" ]; then
  forcing_on=1
fi

if [ "$#" -lt 1 ]; then
  usage
  exit 2
fi

# buildlex reads the leaves with the game's tiles: --tiles, else the preset's
# (a bundled preset's tiles have its name; a preset file names its own)
buildlex_tiles="$tiles"
if [ ! "$buildlex_tiles" ]; then
  if [ -f "$preset" ]; then
    buildlex_tiles="$(sed -n 's/^tiles //p' "$preset" | head -1)"
    if [ ! "$buildlex_tiles" ]; then
      echo "$preset names no tiles: give --tiles" >&2
      exit 1
    fi
    if [ -f "$(dirname "$preset")/$buildlex_tiles" ]; then
      buildlex_tiles="$(dirname "$preset")/$buildlex_tiles"
    fi
  else
    buildlex_tiles="$preset"
  fi
fi

kwg=""
for x in *.kwg *.kbwg *.kad; do
  if [ ! -f "$x" ]; then
    :
  elif [ ! "$kwg" ]; then
    kwg="$x"
  else
    echo "there must be exactly 1 kwg, kbwg or kad here (found multiple)" >&2
    exit 1
  fi
done
if [ ! "$kwg" ]; then
  echo "there must be exactly 1 kwg, kbwg or kad here (found none)" >&2
  exit 1
fi
if [ "$kwg" != "${kwg%.kad}" ] && [ ! "$jumbled" ]; then
  echo "$kwg is for jumbled rules: add --jumbled" >&2
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
done

autoplay_subcommand="autoplay-summarize"
gilles_subcommand="gilles"
generate_subcommand="generate"
if [ ! "$logs_mode" ]; then
  autoplay_subcommand="${autoplay_subcommand}-only"
fi
if [ "$full_mode" ]; then
  generate_subcommand="${generate_subcommand}-full"
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

for full_arg in "$@"; do
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
  time cargo run --release --bin buildlex -- --tiles "$buildlex_tiles" klv --klv "$klv_format" "$last_leave".{csv,"$klv_ext"}
  zip -9v result.zip "summary${num_processed}.csv" "$last_leave".{csv,"$klv_ext"}
  last_leave="${last_leave}.${klv_ext}"
  let num_processed=num_processed+1
done
