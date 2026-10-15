#!/usr/bin/env bash
# Rust and the unchanged Fortran oracle run natively in one Linux amd64 container.
set -euo pipefail
if [[ $# -lt 1 ]]; then
  echo "usage: $0 LINUX_RUST_1.99_TOOLCHAIN_DIRECTORY [bellhop-business.py arguments...]" >&2
  exit 2
fi
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
toolchain=$(cd "$1" && pwd)
shift
[[ -x "$toolchain/bin/rustc" && -x "$toolchain/bin/cargo" ]]
docker build --platform linux/amd64 -t pelagic-business-linux:local \
  -f "$root/tools/reference/Dockerfile.business" "$root/tools/reference"
docker run --rm --platform linux/amd64 \
  --volume "$root:/repo" --volume "$toolchain:/opt/rust:ro" \
  --volume pelagic-business-cargo:/usr/local/cargo \
  --env CARGO_HOME=/usr/local/cargo --env CARGO_TARGET_DIR=/repo/target/linux-business-build \
  --workdir /repo pelagic-business-linux:local \
  /bin/bash -c 'set -euo pipefail
    [[ $(rustc --version) == "rustc 1.99.0 "* ]]
    [[ $(uname -m) == x86_64 ]]
    git config --global --add safe.directory /repo
    exec python3 tools/reference/bellhop-business.py --native-reference "$@"' bash "$@"
