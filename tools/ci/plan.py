#!/usr/bin/env python3
"""Plan baseline wheel/CLI pairs and the separate v3 CLI validation."""
import json
import os


def plan(event):
    wheels = [
        {"os": "ubuntu-22.04", "target": "x86_64-unknown-linux-gnu", "platform": "linux-x64"},
        {"os": "ubuntu-24.04-arm", "target": "aarch64-unknown-linux-gnu", "platform": "linux-arm64"},
        {"os": "macos-15", "target": "aarch64-apple-darwin", "platform": "macos-arm64"},
        {"os": "macos-15-intel", "target": "x86_64-apple-darwin", "platform": "macos-x64"},
        {"os": "windows-2022", "target": "x86_64-pc-windows-msvc", "platform": "windows-x64", "interpreter": "python"},
    ]
    if event == "pull_request":
        wheels = [wheel for wheel in wheels if wheel["platform"] != "macos-x64"]
    shards = {w["platform"]: (2 if w["platform"] in ("windows-x64", "macos-x64") else 1) for w in wheels}
    others = [w for w in wheels if w["platform"] not in ("linux-x64", "windows-x64")]
    return {
        "shards": shards,
        "wheels": {"include": wheels},
        "others": {"include": others},
        "tests": {"include": [dict(w, shard=shard, count=shards[w["platform"]])
                              for w in others for shard in range(shards[w["platform"]])]},
        "platforms": " ".join(wheel["platform"] for wheel in wheels),
        # Keep the optional ISA out of wheels, wheel tests and MATLAB's bundled CLIs.
        "v3": {"include": [dict(w, platform="linux-x64-v3") for w in wheels if w["platform"] == "linux-x64"]},
    }


if __name__ == "__main__":
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        for key, value in plan(os.environ["EVENT"]).items():
            print(f"{key}={value if isinstance(value, str) else json.dumps(value)}", file=output)
