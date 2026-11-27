#!/usr/bin/env python3
"""Flatten build artifacts and make the exact set used by a draft release."""

import argparse
import collections
import hashlib
import pathlib
import shutil
import sys


# Each installer ships exactly once; updater `.sig` files come from four
# signed artifacts (rpm, deb, NSIS exe, macOS `.app.tar.gz`), and the macOS
# updater archive itself is the only `.tar.gz`. Suffixes are checked longest
# first so "linh.app.tar.gz" is classified as ".tar.gz", not ".gz".
REQUIRED_SUFFIXES = {
    ".rpm": 1,
    ".deb": 1,
    ".exe": 1,
    ".msix": 1,
    ".dmg": 1,
    ".tar.gz": 1,
    ".sig": 4,
}


def classify(name: str) -> str:
    for suffix in sorted(REQUIRED_SUFFIXES, key=len, reverse=True):
        if name.endswith(suffix):
            return suffix
    return ""


def build_release_assets(source: pathlib.Path, destination: pathlib.Path) -> int:
    files = sorted(path for path in source.rglob("*") if path.is_file())
    names = [path.name for path in files]
    if len(set(names)) != len(names):
        print("ERROR: duplicate installer filenames", file=sys.stderr)
        return 1

    counts = collections.Counter(classify(name) for name in names)
    if counts != collections.Counter(REQUIRED_SUFFIXES):
        print(
            f"ERROR: expected {dict(REQUIRED_SUFFIXES)}, got {dict(counts)}",
            file=sys.stderr,
        )
        return 1

    destination.mkdir(parents=True, exist_ok=True)
    sums = []
    for src in files:
        dest = destination / src.name
        shutil.copyfile(src, dest)
        with dest.open("rb") as artifact:
            checksum = hashlib.file_digest(artifact, "sha256").hexdigest()
        sums.append(f"{checksum}  {dest.name}")
    (destination / "SHA256SUMS").write_text("\n".join(sums) + "\n", encoding="ascii")
    print("\n".join(sums))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=pathlib.Path)
    parser.add_argument("destination", type=pathlib.Path)
    args = parser.parse_args()
    return build_release_assets(args.source, args.destination)


if __name__ == "__main__":
    sys.exit(main())
