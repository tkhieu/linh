#!/usr/bin/env python3
"""Flatten build artifacts and make the exact set used by a draft release."""

import argparse
import collections
import hashlib
import pathlib
import shutil
import sys


# Each installer ships exactly once. The macOS updater archive is the only
# `.tar.gz`. Suffixes are checked longest first so "linh.app.tar.gz" is
# classified as ".tar.gz", not ".gz".
REQUIRED_SUFFIXES = (".rpm", ".deb", ".exe", ".msix", ".dmg", ".tar.gz")

# Only these installers get an updater signature; rpm/deb/exe/tar.gz are the
# files `tauri-plugin-updater` installs or replaces. msix and dmg are never
# signed for the updater (the Store and Gatekeeper cover them instead), so a
# `.sig` next to either of those is a defect, not a valid signature.
SIGNABLE_SUFFIXES = (".rpm", ".deb", ".exe", ".tar.gz")


def classify(name: str) -> str:
    for suffix in sorted(REQUIRED_SUFFIXES, key=len, reverse=True):
        if name.endswith(suffix):
            return suffix
    return ""


def build_release_assets(
    source: pathlib.Path, destination: pathlib.Path, *, require_signatures: bool
) -> int:
    files = sorted(path for path in source.rglob("*") if path.is_file())
    names = [path.name for path in files]
    if len(set(names)) != len(names):
        print("ERROR: duplicate installer filenames", file=sys.stderr)
        return 1

    installers: dict[str, str] = {}
    sigs: set[str] = set()
    for name in names:
        if name.endswith(".sig"):
            sigs.add(name)
            continue
        suffix = classify(name)
        if not suffix:
            print(f"ERROR: unexpected file {name!r}", file=sys.stderr)
            return 1
        installers[name] = suffix

    counts = collections.Counter(installers.values())
    if counts != collections.Counter(REQUIRED_SUFFIXES):
        print(
            f"ERROR: expected one each of {sorted(REQUIRED_SUFFIXES)}, got {dict(counts)}",
            file=sys.stderr,
        )
        return 1

    # Signatures must pair by exact name with the installer they sign: a
    # `.sig` only ever belongs to the file named by stripping it, so a stray
    # or swapped signature (e.g. the dmg's name with the app archive's
    # signature) cannot slip past a suffix-only count.
    expected_sigs = (
        {name + ".sig" for name, suffix in installers.items() if suffix in SIGNABLE_SUFFIXES}
        if require_signatures
        else set()
    )
    if sigs != expected_sigs:
        missing = sorted(expected_sigs - sigs)
        unexpected = sorted(sigs - expected_sigs)
        print(
            f"ERROR: signature mismatch; missing {missing}, unexpected {unexpected}",
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
    parser.add_argument(
        "--unsigned",
        action="store_true",
        help="accept a build with no updater .sig files (an unsigned dry run)",
    )
    args = parser.parse_args()
    return build_release_assets(
        args.source, args.destination, require_signatures=not args.unsigned
    )


if __name__ == "__main__":
    sys.exit(main())
