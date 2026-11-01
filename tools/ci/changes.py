#!/usr/bin/env python3
"""Conservatively identify prose-only diffs without suppressing required checks.

Unknown paths, deleted/nonexistent base commits, manual runs, and releases run
the full validation. Markdown embedded in executable examples is NOT prose-only.
"""
import json
import os
from pathlib import PurePosixPath, Path
import subprocess


def is_documentation(path):
    p = PurePosixPath(path)
    return (
        path in {"README.md", "README.zh-CN.md", "CHANGELOG.md"}
        or (p.parts[0] == "docs" and p.suffix.lower() in {".md", ".png", ".jpg", ".jpeg", ".svg"})
    )


def needs_full_validation(paths):
    return not paths or any(not is_documentation(path) for path in paths)


def main():
    event_name = os.environ.get("GITHUB_EVENT_NAME", "workflow_dispatch")
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    full = True
    paths = []
    if event_name in {"pull_request", "push"}:
        base = event["pull_request"]["base"]["sha"] if event_name == "pull_request" else event.get("before", "")
        head = os.environ["GITHUB_SHA"]
        if base and set(base) != {"0"}:
            try:
                # PR checkouts are merge commits. Diff against their exact base,
                # not the last push (which could hide earlier code changes).
                subprocess.run(["git", "cat-file", "-e", base], check=True, capture_output=True)
                # Include both sides of a rename: moving code into docs must
                # not hide the deletion of the original executable input.
                raw = subprocess.check_output(["git", "diff", "--no-renames", "--name-only", "-z", base, head])
                paths = [p for p in raw.decode().split("\0") if p]
                full = needs_full_validation(paths)
            except (subprocess.CalledProcessError, UnicodeDecodeError):
                print("Base unavailable: keeping full validation.")
    print(json.dumps({"full": full, "changed_files": paths}, indent=2))
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        print(f"full={str(full).lower()}", file=output)


if __name__ == "__main__":
    main()
