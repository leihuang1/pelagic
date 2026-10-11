#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 CASE.env [OUTPUT_DIRECTORY]" >&2
  exit 2
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
case_path=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
if [[ ${case_path##*.} != env || ! -f $case_path ]]; then
  echo "case must be an existing .env file: $case_path" >&2
  exit 2
fi
case_dir=$(dirname "$case_path")
stem=$(basename "$case_path" .env)
output=${2:-${PELAGIC_REFERENCE_DIR:-$root/target/reference}/$stem}
mkdir -p "$output"
output=$(cd "$output" && pwd)
image=${BELLHOP_REFERENCE_IMAGE:-pelagic-reference:v2023.5-amd64}

work=$(mktemp -d "${TMPDIR:-/tmp}/bellhop-reference.XXXXXX")
trap 'code=$?; if (( code != 0 )); then cp -R "$work" "$output/failed-work-$$"; fi; rm -rf "$work"; exit "$code"' EXIT
for input in "$case_dir/$stem".*; do
  case ${input##*.} in
    env|ssp|ati|bty|brc|trc|sbp|irc) cp "$input" "$work/" ;;
  esac
done

docker run --rm \
  --platform linux/amd64 \
  --user "$(id -u):$(id -g)" \
  --volume "$work:/work" \
  "$image" "/work/$stem"

found=0
for extension in prt ray arr shd; do
  result="$work/$stem.$extension"
  if [[ -f $result ]]; then
    cp "$result" "$output/"
    found=1
  fi
done
if [[ $found == 0 ]]; then
  echo "reference run produced no recognized output" >&2
  exit 1
fi
printf '%s\n' "$output"
