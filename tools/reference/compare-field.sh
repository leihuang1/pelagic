#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 CASE.env" >&2
  exit 2
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
case_path=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
stem=$(basename "$case_path" .env)
output="${PELAGIC_REFERENCE_DIR:-$root/target/reference}/$stem"
image=${BELLHOP_REFERENCE_IMAGE:-pelagic-reference:v2023.5-amd64}

if ! docker image inspect "$image" >/dev/null 2>&1; then
  "$root/tools/reference/build-image.sh"
fi
"$root/tools/reference/run-case.sh" "$case_path" "$output"

BELLHOP_DIFFERENTIAL_ENV="$case_path" \
BELLHOP_DIFFERENTIAL_SHD="$output/$stem.shd" \
"$root/tools/reference/test-one.sh" \
  --release \
  --manifest-path "$root/Cargo.toml" \
  --package bellhop \
  --test differential_reference \
  pressure_fields_match_pinned_linux_reference \
  -- --ignored --exact --nocapture --test-threads=1