#!/usr/bin/env bash
# Validate and run every example case with the command-line program.
#
#   examples/cli/run-all.sh [output-directory]      (default: results/cli)
#
# Uses `pelagic` on PATH, or the program named by $PELAGIC_CLI. Each case is
# run from its self-contained JSON, writing Toolbox-format files and HDF5.
set -euo pipefail
pelagic=${PELAGIC_CLI:-pelagic}
cases=$(cd "$(dirname "$0")/../cases" && pwd)
out=${1:-results/cli}

"$pelagic" version
for json in "$cases"/*/*/*.json; do
    model=$(basename "$(dirname "$(dirname "$json")")")   # bellhop or kraken
    name=$(basename "$json" .json)
    "$pelagic" "$model" validate "$json" > /dev/null
    "$pelagic" "$model" run "$json" --format both --output "$out/$model/$name" 2> /dev/null
done

# The same cases also run straight from their Acoustics Toolbox files:
"$pelagic" bellhop run "$cases/bellhop/gulf/gulf.env" --output "$out/legacy/gulf"
"$pelagic" kraken run "$cases/kraken/gulf/gulf.env" --flp "$cases/kraken/gulf/gulf-coupled.flp" \
    --output "$out/legacy/gulf-coupled"
"$pelagic" kraken run "$cases/kraken/leaky-modes/leaky-modes.env" --solver krakenc \
    --output "$out/legacy/leaky-modes"
