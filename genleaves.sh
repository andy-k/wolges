#!/usr/bin/env bash

set -euo pipefail

if [ "$#" -ne 3 ]; then
  cat <<"EOF"
usage 1:
  mkdir t
  cd t
  cp -ip ../.../CSW21.kwg .
  ../genleaves.sh super-english english 2000000
usage 2:
  mkdir t
  cd t
  cp -ip ../.../NSF20.kwg .
  ../genleaves.sh norwegian norwegian 2000000
EOF
  exit 2
fi

leave_param="$1"
buildlex_param="$2"
num="$3"
echo "$num"

kwg=""
for x in *.kwg; do
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

echo "$kwg"

time cargo run --release --bin leave -- "$leave_param"-autoplay "$kwg" -{,} "$num"
log_file="$(ls -1td log-* | head -1)"
echo "$log_file"
time cargo run --release --bin leave -- "$leave_param"-summarize "$log_file" summary0.csv
time cargo run --release --bin leave -- "$leave_param"-generate summary0.csv leaves-smooth1.csv
time cargo run --release --bin buildlex -- "$buildlex_param"-klv2 leaves-smooth1.{csv,klv2}

time cargo run --release --bin leave -- "$leave_param"-autoplay "$kwg" leaves-smooth1.klv2{,} "$num"
log_file="$(ls -1td log-* | head -1)"
echo "$log_file"
time cargo run --release --bin leave -- "$leave_param"-summarize "$log_file" summary1.csv
time cargo run --release --bin leave -- "$leave_param"-generate summary1.csv leaves-smooth2.csv
time cargo run --release --bin buildlex -- "$buildlex_param"-klv2 leaves-smooth2.{csv,klv2}

time cargo run --release --bin leave -- "$leave_param"-autoplay "$kwg" leaves-smooth2.klv2{,} "$num"
log_file="$(ls -1td log-* | head -1)"
echo "$log_file"
time cargo run --release --bin leave -- "$leave_param"-summarize "$log_file" summary2.csv
time cargo run --release --bin leave -- "$leave_param"-generate summary2.csv leaves-smooth3.csv
time cargo run --release --bin buildlex -- "$buildlex_param"-klv2 leaves-smooth3.{csv,klv2}

zip -9v result.zip summary0.csv leaves-smooth1.{csv,klv2} summary1.csv leaves-smooth2.{csv,klv2} summary2.csv leaves-smooth3.{csv,klv2}
