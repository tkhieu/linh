#!/usr/bin/env python3
"""Flatten build artifacts and make the exact set used by a draft release."""

import argparse
import hashlib
import pathlib
import shutil
import sys


REQUIRED_SUFFIXES = {".rpm", ".deb", ".exe", ".msix", ".dmg"}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=pathlib.Path)
    parser.add_argument("destination", type=pathlib.Path)
    args = parser.parse_args()
    files = sorted(path for path in args.source.rglob("*") if path.is_file())
    suffixes = [path.suffix for path in files]
    if set(suffixes) != REQUIRED_SUFFIXES or len(suffixes) != len(REQUIRED_SUFFIXES):
        print(
            f"ERROR: expected exactly one of {sorted(REQUIRED_SUFFIXES)}, got {suffixes}",
            file=sys.stderr,
        )
        return 1
    names = [path.name for path in files]
    if len(set(names)) != len(names):
        print("ERROR: duplicate installer filenames", file=sys.stderr)
        return 1

    args.destination.mkdir(parents=True, exist_ok=True)
    sums = []
    for source in files:
        dest = args.destination / source.name
        shutil.copyfile(source, dest)
        with dest.open("rb") as artifact:
            checksum = hashlib.file_digest(artifact, "sha256").hexdigest()
        sums.append(f"{checksum}  {dest.name}")
    (args.destination / "SHA256SUMS").write_text(
        "\n".join(sums) + "\n", encoding="ascii"
    )
    print("\n".join(sums))
    return 0


if __name__ == "__main__":
    sys.exit(main())
