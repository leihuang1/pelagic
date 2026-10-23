#!/usr/bin/env python3
"""Write every example case's JSON from its legacy files, using `pelagic ... export`.

    python3 tools/export-examples.py          # rewrite the JSON files
    python3 tools/export-examples.py --check  # fail if any JSON is out of date

The case list is examples/cases/catalog.json. The CLI is $PELAGIC_CLI, or
target/release/pelagic (build it with `cargo build --release -p cli`). Output is
indented for reading, with arrays of numbers kept on one line so that large
range-dependent cases stay well under the 1 MiB JSON input limit.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CASES = ROOT / "examples" / "cases"


def dumps(value, indent=""):
    """Indented JSON, with arrays of scalars on one line."""
    inner = indent + "  "
    if isinstance(value, dict) and value:
        items = (f"{inner}{json.dumps(key)}: {dumps(item, inner)}" for key, item in value.items())
        return "{\n" + ",\n".join(items) + "\n" + indent + "}"
    if isinstance(value, list) and any(isinstance(item, (dict, list)) for item in value):
        return "[\n" + ",\n".join(inner + dumps(item, inner) for item in value) + "\n" + indent + "]"
    return json.dumps(value, ensure_ascii=False)


def export(cli, case):
    command = [cli, case["model"], "export", str(CASES / case["env"])]
    if case["model"] == "kraken":
        command += ["--flp", str(CASES / case["flp"]), "--solver", case["solver"]]
    output = subprocess.run(command, check=True, capture_output=True).stdout
    return (dumps(json.loads(output)) + "\n").encode()


def main():
    check = sys.argv[1:] == ["--check"]
    if sys.argv[1:] not in ([], ["--check"]):
        sys.exit(__doc__)
    cli = os.environ.get("PELAGIC_CLI") or str(ROOT / "target" / "release" / "pelagic")
    stale = []
    for case in json.loads((CASES / "catalog.json").read_text())["cases"]:
        target = CASES / case["json"]
        document = export(cli, case)
        if target.exists() and target.read_bytes() == document:
            continue
        stale.append(case["json"])
        if not check:
            target.write_bytes(document)
    verb = "out of date" if check else "written"
    print(f"{len(stale)} example JSON file(s) {verb}" + "".join(f"\n  {name}" for name in stale))
    return 1 if check and stale else 0


if __name__ == "__main__":
    sys.exit(main())
