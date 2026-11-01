#!/usr/bin/env python3
"""Validate always-running gates and conditionally skipped expensive jobs."""
import json
import os


def check(needs):
    gates = {"plan", "windows-check"}
    assert gates <= needs.keys(), "missing mandatory gate"
    full = needs["plan"]["outputs"]["full"]
    assert full in {"true", "false"}, "invalid change plan"
    for name, job in needs.items():
        expected = "success" if full == "true" or name in gates else "skipped"
        assert job["result"] == expected, (name, job, expected)


if __name__ == "__main__":
    check(json.loads(os.environ["NEEDS"]))
