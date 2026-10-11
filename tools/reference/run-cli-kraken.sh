#!/usr/bin/env bash
# One actual CLI solve writes both formats; independent readers check every native value.
set -euo pipefail
if [[ $# -lt 2 ]]; then
  echo "usage: $0 CASE.{env,json} RESULT_DIRECTORY [--solver kraken|krakenc]" >&2
  exit 2
fi
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
input=$1
output=$2
shift 2
base=$(basename "$input")
stem=${base%.*}
"$root/target/release/pelagic" kraken run "$input" --format both --output "$output" "$@"
KRAKEN_HDF5_RESULT="$output/$stem.h5" KRAKEN_DIFFERENTIAL_ENV="$input" \
  "$root/tools/reference/test-one.sh" --release --manifest-path "$root/Cargo.toml" -p cli --test native_output \
    actual_cli_native_layouts_match_all_hdf5_values -- --ignored --exact --nocapture
