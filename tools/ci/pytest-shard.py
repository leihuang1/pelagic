#!/usr/bin/env python3
"""Run one deterministic, disjoint shard of the complete pytest collection.

No test names/markers are excluded. The manifests allow the final CI gate to
prove that all collected node IDs were assigned exactly once on each platform.
"""
import argparse
import hashlib
import json
from pathlib import Path


def shard_for(nodeid, count):
    return int.from_bytes(hashlib.sha256(nodeid.encode()).digest()[:8], "big") % count


class Shard:
    def __init__(self, index, count, manifest):
        self.index, self.count, self.manifest = index, count, manifest

    def pytest_collection_modifyitems(self, session, config, items):
        all_ids = [item.nodeid for item in items]
        selected = [item for item in items if shard_for(item.nodeid, self.count) == self.index]
        deselected = [item for item in items if shard_for(item.nodeid, self.count) != self.index]
        self.manifest.write_text(json.dumps({"index": self.index, "count": self.count,
            "all": all_ids, "selected": [item.nodeid for item in selected]}, indent=2))
        config.hook.pytest_deselected(items=deselected)
        items[:] = selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--index", type=int, required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args, pytest_args = parser.parse_known_args()
    if not 0 <= args.index < args.count:
        parser.error("require 0 <= index < count")
    import pytest
    return pytest.main(pytest_args, plugins=[Shard(args.index, args.count, args.manifest)])


if __name__ == "__main__":
    raise SystemExit(main())
