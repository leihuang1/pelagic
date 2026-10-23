#!/usr/bin/env python3
"""Check that every relative link in tracked Markdown resolves, including #anchors.

Run from anywhere: python3 tools/check-docs.py. External URLs are not fetched, except
that links to this repository's main branch (used by the PyPI-rendered README) are
checked as repository paths.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"\]\(([^)\s]+)\)")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
FENCE = re.compile(r"^\s*(```|~~~)")
REPOSITORY = r"https://github\.com/leihuang1/pelagic/(blob|tree)/main/"


def anchors(path, cache={}):
    """GitHub-style heading slugs for one Markdown file."""
    if path not in cache:
        seen, slugs, fenced = {}, set(), False
        for line in path.read_text(encoding="utf-8").splitlines():
            if FENCE.match(line):
                fenced = not fenced
                continue
            match = None if fenced else HEADING.match(line)
            if not match:
                continue
            text = re.sub(r"`|\*\*|__|\[([^\]]*)\]\([^)]*\)", r"\1", match.group(2))
            slug = re.sub(r"[^\w\- ]", "", text.lower()).replace(" ", "-")
            count = seen.get(slug, 0)
            seen[slug] = count + 1
            slugs.add(slug if count == 0 else f"{slug}-{count}")
        cache[path] = slugs
    return cache[path]


def links(path):
    fenced = False
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if FENCE.match(line):
            fenced = not fenced
        elif not fenced:
            for match in LINK.finditer(re.sub(r"`[^`]*`", "", line)):
                yield number, match.group(1)


def main():
    files = subprocess.run(["git", "-C", str(ROOT), "ls-files", "*.md"],
                           capture_output=True, text=True, check=True).stdout.split()
    errors = []
    for name in files:
        source = ROOT / name
        for number, target in links(source):
            repository = re.match(REPOSITORY, target)
            if repository:
                target = target[repository.end():]
            elif re.match(r"[a-z][a-z0-9+.-]*:", target):
                continue
            path, _, anchor = target.partition("#")
            base = ROOT if repository else source.parent
            resolved = (base / path).resolve() if path else source
            if not resolved.exists():
                errors.append(f"{name}:{number}: missing target {target}")
            elif anchor and resolved.suffix == ".md" and anchor not in anchors(resolved):
                errors.append(f"{name}:{number}: missing anchor {target}")
    for error in errors:
        print(error, file=sys.stderr)
    print(f"checked {len(files)} Markdown files: {len(errors)} broken link(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
