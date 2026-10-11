#!/usr/bin/env bash
# Original inputs + generated reference resources; not itself Rust acceptance.
set -euo pipefail
if [[ $# -gt 1 ]]; then
  echo "usage: $0 [OUTPUT_DIRECTORY]" >&2
  exit 2
fi
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
output=${1:-${PELAGIC_REFERENCE_DIR:-$root/target/reference}/TabRefCoef}
mkdir -p "$output/inputs"
output=$(cd "$output" && pwd)
image=${BELLHOP_REFERENCE_IMAGE:-pelagic-reference:v2023.5-amd64}
if ! docker image inspect "$image" >/dev/null 2>&1; then
  "$root/tools/reference/build-image.sh"
fi

docker run --rm --platform linux/amd64 --user "$(id -u):$(id -g)" \
  --volume "$output:/out" --entrypoint /bin/sh "$image" -c '
    set -eu
    test -x /usr/local/bin/bounce-fortran || {
      echo "rebuild the reference image: source-built bounce-fortran is required" >&2; exit 1;
    }
    cp /opt/reference-build.txt /out/
    cp /opt/acoustics-toolbox/tests/TabRefCoef/neggradB.env /out/inputs/
    for case in neggradC_geo neggradC_brc neggradC_irc; do
      cp /opt/acoustics-toolbox/tests/TabRefCoef/$case.env /opt/acoustics-toolbox/tests/TabRefCoef/$case.flp /out/inputs/
    done
    cd /out
    sha256sum inputs/* > inputs.sha256
  '

for iteration in 1 2 3; do
  mkdir -p "$output/run$iteration"
  cp "$output/inputs/"* "$output/run$iteration/"
  docker run --rm --platform linux/amd64 --user "$(id -u):$(id -g)" \
    --volume "$output:/out" --entrypoint /bin/sh "$image" -c '
      set -eu
      cd "/out/run$1"
      /usr/local/bin/bounce-fortran neggradB > bounce.stdout
      cp neggradB.brc neggradC_brc.brc
      cp neggradB.irc neggradC_irc.irc
      for case in neggradC_geo neggradC_brc neggradC_irc; do
        /usr/local/bin/krakenc-fortran "$case" > "$case.stdout"
        /usr/local/bin/field-fortran "$case" > "$case.field.stdout"
        mv field.prt "$case.field.prt"
        test -s "$case.mod"
        test -s "$case.shd"
      done
      sha256sum *.env *.flp *.brc *.irc *.mod *.shd > outputs.sha256
    ' tabref "$iteration"
done
# CPU times in print files are intentionally not required to match.
for iteration in 2 3; do
  for file in neggradB.brc neggradB.irc neggradC_geo.mod neggradC_geo.shd \
    neggradC_brc.mod neggradC_brc.shd neggradC_irc.mod neggradC_irc.shd; do
    cmp "$output/run1/$file" "$output/run$iteration/$file"
  done
done
printf '%s\n' "$output"
