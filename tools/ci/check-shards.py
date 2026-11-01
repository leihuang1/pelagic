#!/usr/bin/env python3
"""Reject incomplete, duplicate, or differing test collections across shards."""
import argparse
from collections import Counter
import json
from pathlib import Path


def check(manifests, planned_count=None):
    assert manifests, "no shard manifests"
    count = manifests[0]["count"]
    if planned_count is not None:
        assert count == planned_count, "shard count differs from plan"
    assert sorted(m["index"] for m in manifests) == list(range(count)), "missing/duplicate shard"
    expected = manifests[0]["all"]
    assert expected and len(set(expected)) == len(expected), "empty/duplicate collection"
    for manifest in manifests:
        assert manifest["count"] == count and manifest["all"] == expected, "collections differ"
    actual = Counter(node for m in manifests for node in m["selected"])
    assert actual == Counter(expected), "missing or duplicate test assignment"
    return len(expected)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--planned", type=json.loads, required=True)
    args = parser.parse_args()
    assert args.planned, "empty platform plan"
    for platform, count in args.planned.items():
        assert isinstance(count, int) and count >= 1, "invalid planned count"
        manifests = [json.loads(path.read_text()) for path in sorted(args.directory.glob(f"shards-{platform}-*/*.json"))]
        print(f"{platform}: {check(manifests, count)} tests assigned exactly once")
