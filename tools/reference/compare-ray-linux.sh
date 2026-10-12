#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 CASE.env" >&2
  exit 2
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
case_path=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
case_dir=$(dirname "$case_path")
case_name=$(basename "$case_path")
stem=${case_name%.env}
output="${PELAGIC_REFERENCE_DIR:-$root/target/reference}/$stem"
rust_image='rust:1.88.0-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0'
reference_image=${BELLHOP_REFERENCE_IMAGE:-pelagic-reference:v2023.5-amd64}

if ! docker image inspect "$reference_image" >/dev/null 2>&1; then
  "$root/tools/reference/build-image.sh"
fi
"$root/tools/reference/run-case.sh" "$case_path" "$output"

docker run --rm \
  --platform linux/amd64 \
  --env CARGO_TARGET_DIR=/target \
  --env RUSTUP_TOOLCHAIN=1.88.0 \
  --env BELLHOP_DIFFERENTIAL_ENV="/case/$case_name" \
  --env BELLHOP_DIFFERENTIAL_RAY="/reference/$stem.ray" \
  --env BELLHOP_DIFFERENTIAL_POSITION_TOLERANCE_M="${BELLHOP_DIFFERENTIAL_POSITION_TOLERANCE_M:-1e-5}" \
  --env BELLHOP_DIFFERENTIAL_MINIMUM_STEP_FACTOR="${BELLHOP_DIFFERENTIAL_MINIMUM_STEP_FACTOR:-4.1}" \
  --volume "$root:/repo:ro" \
  --volume "$case_dir:/case:ro" \
  --volume "$output:/reference:ro" \
  --volume pelagic-cargo-registry:/usr/local/cargo/registry \
  --volume pelagic-linux-target:/target \
  --workdir /repo \
  "$rust_image" \
  tools/reference/test-one.sh --package bellhop --test differential_reference \
    ray_trajectories_match_pinned_linux_reference \
    -- --ignored --exact --nocapture --test-threads=1
