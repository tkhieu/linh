#!/usr/bin/env python3
"""Require all release version sources (and an optional v-prefixed tag) to agree."""

import argparse
import json
import pathlib
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", help="tag to validate, e.g. v0.2.0")
    args = parser.parse_args()

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
    if args.tag:
        if not args.tag.startswith("v"):
            parser.error(f"release tag must start with v: {args.tag}")
        versions["release tag"] = args.tag[1:]

    for source, version in versions.items():
        print(f"{source}: {version}")
    if len(set(versions.values())) != 1:
        print("ERROR: release versions differ; update all three files before tagging", file=sys.stderr)
        return 1
    version = next(iter(versions.values()))
    if len(version.split(".")) != 3 or not all(
        part.isdecimal() and int(part) <= 65535 for part in version.split(".")
    ):
        print("ERROR: version must have three numeric MSIX-compatible parts", file=sys.stderr)
        return 1
    print(f"VERSION={version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
