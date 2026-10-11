#!/usr/bin/env python3
"""Verify recorded hashes and require a manifest entry for every numerical golden."""
import hashlib
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]


def verify(base, records, artifacts):
    expected = {}
    for digest, name in records:
        if name in expected and expected[name] != digest:
            raise ValueError(f"conflicting hashes: {name}")
        expected[name] = digest
        if hashlib.sha256((base / name).read_bytes()).hexdigest() != digest:
            raise ValueError(f"SHA-256 mismatch: {base / name}")
    missing = {str(path.relative_to(base)) for path in artifacts} - expected.keys()
    if missing:
        raise ValueError(f"unmanifested goldens: {sorted(missing)}")
    print(f"{base}: {len(expected)} verified records, {len(artifacts)} numerical artifacts")


def records(text):
    return re.findall(r"^([0-9a-f]{64})  (.+)$", text, re.MULTILINE)


def main():
    base = ROOT / "crates/kraken/tests/fixtures"
    verify(base, [entry for manifest in sorted((base / "golden").glob("*.sha256"))
                  for entry in records(manifest.read_text())],
           [p for p in (base / "golden").iterdir() if p.suffix in {".mod", ".shd", ".prt"}])
    base = ROOT / "crates/bellhop/tests/fixtures/golden"
    verify(base, records((base / "README.md").read_text()),
           [p for p in base.iterdir() if p.suffix in {".ray", ".arr", ".csv", ".brc"}])


if __name__ == "__main__":
    main()
