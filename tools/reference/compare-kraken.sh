#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 CASE.env (requires same-stem .flp; supported single-fluid slice only)" >&2
  exit 2
fi
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
case_path=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
stem=$(basename "$case_path" .env)
if [[ ! -f ${case_path%.env}.flp ]]; then
  echo "FIELD comparison requires ${case_path%.env}.flp" >&2
  exit 2
fi
output="${PELAGIC_REFERENCE_DIR:-$root/target/reference}/$stem-kraken"
"$root/tools/reference/run-kraken-case.sh" kraken "$case_path" "$output"

KRAKEN_DIFFERENTIAL_ENV="$case_path" \
KRAKEN_DIFFERENTIAL_ROOT="$output/$stem" \
"$root/tools/reference/test-one.sh" --release --manifest-path "$root/Cargo.toml" --package kraken \
  --test differential_reference fluid_matches_fresh_reference \
  -- --ignored --exact --nocapture
