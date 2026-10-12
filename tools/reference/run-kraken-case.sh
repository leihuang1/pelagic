#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "usage: $0 {kraken|krakenc} CASE.env [OUTPUT_DIRECTORY]" >&2
  exit 2
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
engine=$1
case "$engine" in
  kraken|krakenc) program="/usr/local/bin/${engine}-fortran" ;;
  *) echo "engine must be kraken or krakenc: $engine" >&2; exit 2 ;;
esac
case_path=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
if [[ ${case_path##*.} != env || ! -f $case_path ]]; then
  echo "case must be an existing .env file: $case_path" >&2
  exit 2
fi
case_dir=$(dirname "$case_path")
stem=$(basename "$case_path" .env)
has_field=0
if [[ -f $case_dir/$stem.flp ]]; then
  has_field=1
fi
output=${3:-${PELAGIC_REFERENCE_DIR:-$root/target/reference}/$stem-$engine}
mkdir -p "$output"
output=$(cd "$output" && pwd)
image=${BELLHOP_REFERENCE_IMAGE:-pelagic-reference:v2023.5-amd64}

if ! docker image inspect "$image" >/dev/null 2>&1; then
  "$root/tools/reference/build-image.sh"
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/kraken-reference.XXXXXX")
trap 'code=$?; if (( code != 0 )); then cp -R "$work" "$output/failed-work-$$"; fi; rm -rf "$work"; exit "$code"' EXIT
for input in "$case_dir/$stem".*; do
  case ${input##*.} in
    env|flp|ssp|ati|bty|brc|trc|sbp|irc) cp "$input" "$work/" ;;
  esac
done

docker run --rm \
  --platform linux/amd64 \
  --user "$(id -u):$(id -g)" \
  --volume "$work:/work" \
  --workdir /work \
  --entrypoint /bin/sh \
  "$image" \
  -c 'set -eu; "$1" "$2"; if [ "$3" = 1 ]; then /usr/local/bin/field-fortran "$2"; fi' \
  run-reference "$program" "/work/$stem" "$has_field"

test -s "$work/$stem.mod" || {
  echo "reference run did not produce $stem.mod" >&2
  exit 1
}
if [[ $has_field == 1 ]]; then
  test -s "$work/$stem.shd" || {
    echo "FIELD did not produce $stem.shd" >&2
    exit 1
  }
fi

for result in "$work/$stem.mod" "$work/$stem.evm" "$work/$stem.shd" "$work/$stem.prt" "$work/field.prt"; do
  if [[ -f $result ]]; then
    cp "$result" "$output/"
  fi
done
printf '%s\n' "$output"
