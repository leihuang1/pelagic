#!/usr/bin/env python3
"""Reject incomplete, duplicate, or differing test collections across shards."""
import argparse
from collections import Counter
import json
from pathlib import Path


def check(manifests):
    assert manifests, "no shard manifests"
    count = manifests[0]["count"]
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
    parser.add_argument("--platforms", nargs="+", required=True)
    args = parser.parse_args()
    for platform in args.platforms:
        manifests = [json.loads(path.read_text()) for path in sorted(args.directory.glob(f"shards-{platform}-*/*.json"))]
        print(f"{platform}: {check(manifests)} tests assigned exactly once")
