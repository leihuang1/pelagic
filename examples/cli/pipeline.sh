#!/usr/bin/env bash
# Use the command-line program from scripts and other languages.
#
#   examples/cli/pipeline.sh [output-directory]      (default: results/pipeline)
#
# Shows JSON on stdin, machine-readable diagnostics and version checks. Needs
# python3 for the JSON edits; any language that reads JSON works the same way.
set -euo pipefail
pelagic=${PELAGIC_CLI:-pelagic}
cases=$(cd "$(dirname "$0")/../cases" && pwd)
out=${1:-results/pipeline}

# 1. Versions and schemas, as JSON: check these before relying on a format.
"$pelagic" version --json
echo

# 2. Convert a legacy case to JSON, change it, and run it from stdin.
"$pelagic" bellhop export "$cases/bellhop/munk-tl/munk-tl.env" \
    | python3 -c 'import json, sys; case = json.load(sys.stdin); case["frequency_hz"] = 100.0; json.dump(case, sys.stdout)' \
    | "$pelagic" --diagnostics json bellhop run - --format hdf5 --output "$out/munk-100hz"

# 3. Errors as JSON lines on stderr, with a stable code and the field's JSON path.
set +e
"$pelagic" bellhop export "$cases/bellhop/munk-tl/munk-tl.json" \
    | python3 -c 'import json, sys; case = json.load(sys.stdin); case["frequency_hz"] = -50; json.dump(case, sys.stdout)' \
    | "$pelagic" --diagnostics json bellhop validate - 2> "$out/errors.ndjson"
status=$?
set -e
echo "exit code $status (2 = invalid input); diagnostics:"
python3 -c 'import json, sys
for line in open(sys.argv[1]):
    for d in json.loads(line)["diagnostics"]:
        print("  {severity} {code} at {field}: {message}".format(**d))' "$out/errors.ndjson"
