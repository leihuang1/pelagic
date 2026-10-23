#!/usr/bin/env python3
"""Verify build artifacts before bundling matching CLIs; never downloads or executes them."""
import argparse
import hashlib
from pathlib import Path
import shutil

PLATFORMS = ("linux-x64", "linux-arm64", "macos-arm64", "macos-x64", "windows-x64")


def stage(source, destination):
    verified = []
    for platform in PLATFORMS:
        root = source / ("cli-" + platform)
        hashes = {}
        for line in (root / "SHA256SUMS").read_text().splitlines():
            digest, name = line.split("  ", 1)
            if name in hashes or Path(name).name != name:
                raise ValueError("invalid checksum manifest name")
            hashes[name] = digest
        executable = "pelagic.exe" if platform == "windows-x64" else "pelagic"
        for name in (executable, "LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.json"):
            path = root / name
            if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != hashes.get(name):
                raise ValueError(f"checksum mismatch: {path}")
            verified.append((platform, path, name == executable))
    for platform, path, executable in verified:
        target = destination / platform / path.name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        if executable:
            target.chmod(0o755)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    stage(args.source, args.destination)
