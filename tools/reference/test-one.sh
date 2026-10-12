#!/usr/bin/env bash
# Focused reference checks must execute exactly one test, including after renames.
set -euo pipefail
log=$(mktemp "${TMPDIR:-/tmp}/pelagic-test-one.XXXXXX")
trap 'rm -f "$log"' EXIT
cargo test --locked "$@" 2>&1 | tee "$log"
if [[ $(grep -c '^test result:' "$log") != 1 ]] ||
   ! grep -q '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$log"; then
  echo 'Expected exactly one passing test; refusing an empty or ambiguous selection.' >&2
  exit 1
fi
