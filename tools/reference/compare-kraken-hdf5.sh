#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 {kraken|krakenc} CASE.env" >&2
  exit 2
fi
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
engine=$1
case_path=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
stem=$(basename "$case_path" .env)
reference="${PELAGIC_REFERENCE_DIR:-$root/target/reference}/$stem-$engine/$stem"
"$root/tools/reference/run-kraken-case.sh" "$engine" "$case_path"

compare() {
  KRAKEN_FREQUENCY_SOLVER="$engine" KRAKEN_DIFFERENTIAL_ENV="$case_path" KRAKEN_DIFFERENTIAL_ROOT="$reference" \
    "$root/tools/reference/test-one.sh" --release --manifest-path "$root/Cargo.toml" -p kraken --test differential_reference \
      multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
}
compare
directory="$(dirname "$reference")/pelagic"
"$root/tools/reference/run-cli-kraken.sh" "$case_path" "$directory" --solver "$engine"
KRAKEN_HDF5_RESULT="$directory/$stem.h5" compare
