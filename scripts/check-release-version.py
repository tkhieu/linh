#!/usr/bin/env python3
"""Require all release version sources (and an optional v-prefixed tag) to agree."""

import argparse
import json
import pathlib
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[1]


def is_msix_compatible(version: str) -> bool:
    """MSIX package versions must be a plain numeric Major.Minor.Patch."""
    parts = version.split(".")
    return len(parts) == 3 and all(
        part.isdecimal() and int(part) <= 65535 for part in parts
    )


def msix_error(version: str, tag: str | None) -> str:
    given = f" (from tag {tag})" if tag else ""
    return (
        f"ERROR: version '{version}'{given} is not MSIX-compatible: MSIX packages "
        "require a numeric Major.Minor.Patch version, so pre-release and build "
        "suffixes (e.g. '-beta', '+build') are not supported."
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", help="tag to validate, e.g. v0.2.0")
    args = parser.parse_args()

    # Validate the tag's own format before comparing it with the files:
    # a pre-release/build tag must be rejected with the MSIX-specific
    # message even when the files happen to disagree with it too.
    tag_version = None
    if args.tag:
        if not args.tag.startswith("v"):
            parser.error(f"release tag must start with v: {args.tag}")
        tag_version = args.tag[1:]
        if not is_msix_compatible(tag_version):
            print(msix_error(tag_version, args.tag), file=sys.stderr)
            return 1

    with (ROOT / "Cargo.toml").open("rb") as cargo_file:
        cargo = tomllib.load(cargo_file)
    versions = {
        "Cargo.toml [workspace.package]": cargo["workspace"]["package"]["version"],
        "src-tauri/tauri.conf.json": json.loads(
            (ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8")
        )["version"],
        "ui/package.json": json.loads(
            (ROOT / "ui/package.json").read_text(encoding="utf-8")
        )["version"],
    }
    if tag_version is not None:
        versions["release tag"] = tag_version

    for source, version in versions.items():
        print(f"{source}: {version}")
    if len(set(versions.values())) != 1:
        print("ERROR: release versions differ; update all three files before tagging", file=sys.stderr)
        return 1
    version = next(iter(versions.values()))
    if not is_msix_compatible(version):
        print(msix_error(version, args.tag), file=sys.stderr)
        return 1
    print(f"VERSION={version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
