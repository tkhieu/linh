#!/usr/bin/env python3
"""Build Tauri updater v2's static latest.json from a published GitHub release."""

import argparse
import base64
import binascii
import datetime
import json
import pathlib
import re
import sys
from urllib.parse import quote


TAG = re.compile(r"v([0-9]+\.[0-9]+\.[0-9]+)\Z")
ARTIFACTS = {
    "linux-x86_64-rpm": re.compile(r"linh-(?P<version>[0-9]+\.[0-9]+\.[0-9]+)-[0-9]+\.x86_64\.rpm\Z"),
    "linux-x86_64-deb": re.compile(r"linh_(?P<version>[0-9]+\.[0-9]+\.[0-9]+)_amd64\.deb\Z"),
    "windows-x86_64-nsis": re.compile(r"linh_(?P<version>[0-9]+\.[0-9]+\.[0-9]+)_x64-setup\.exe\Z"),
    "darwin": re.compile(r"linh\.app\.tar\.gz\Z"),
}


def build_manifest(tag: str, repo: str, assets_dir: pathlib.Path, release: dict) -> dict:
    match = TAG.fullmatch(tag)
    if not match:
        raise ValueError(f"invalid release tag {tag!r}; expected vMAJOR.MINOR.PATCH")
    version = match.group(1)
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError(f"invalid repository {repo!r}")
    if release.get("tag_name") != tag or release.get("draft") or release.get("prerelease"):
        raise ValueError("release tag differs or release is draft/prerelease")
    published = release.get("published_at")
    if not isinstance(published, str):
        raise ValueError("release has no published_at timestamp")
    try:
        date = datetime.datetime.fromisoformat(published.replace("Z", "+00:00"))
    except ValueError as exc:
        raise ValueError("invalid published_at timestamp") from exc
    if date.tzinfo is None:
        raise ValueError("published_at must have a timezone")

    files = [path for path in assets_dir.rglob("*") if path.is_file()]
    by_name = {path.name: path for path in files}
    if len(by_name) != len(files):
        raise ValueError("duplicate asset filename")

    platforms = {}
    for target, pattern in ARTIFACTS.items():
        matches = [name for name in by_name if pattern.fullmatch(name)]
        if len(matches) != 1:
            raise ValueError(f"expected exactly one {target} artifact, found {len(matches)}")
        name = matches[0]
        artifact = by_name[name]
        signature = by_name.get(name + ".sig")
        if artifact.stat().st_size == 0 or signature is None or signature.stat().st_size == 0:
            raise ValueError(f"missing or empty artifact/signature for {target}: {name}")
        artifact_version = pattern.fullmatch(name).groupdict().get("version")
        if artifact_version is not None and artifact_version != version:
            raise ValueError(f"{name} does not match tag {tag}")
        signature_text = signature.read_text(encoding="utf-8").strip()
        if not signature_text:
            raise ValueError(f"empty signature for {name}")
        try:
            decoded = base64.b64decode(signature_text, validate=True).decode("utf-8")
        except (binascii.Error, UnicodeError) as exc:
            raise ValueError(f"invalid updater signature encoding for {name}") from exc
        comments = [line for line in decoded.splitlines() if line.startswith("trusted comment: ")]
        if len(comments) != 1 or f"file:{name}" not in comments[0].split("\t") or f"version:{version}" not in comments[0].split("\t"):
            raise ValueError(f"signature metadata does not match {name} and tag {tag}")
        platforms[target] = {
            "url": f"https://github.com/{repo}/releases/download/{quote(tag, safe='')}/{quote(name, safe='')}",
            "signature": signature_text,
        }

    # An NSIS install asks for the bundle-specific target first; the generic
    # Windows target is the documented fallback for installations without it.
    platforms["windows-x86_64"] = platforms["windows-x86_64-nsis"].copy()
    platforms["darwin-aarch64"] = platforms.pop("darwin")
    platforms["darwin-x86_64"] = platforms["darwin-aarch64"].copy()

    notes = release.get("body") or ""
    if not isinstance(notes, str):
        raise ValueError("release body must be text")
    if len(notes) > 4000:
        notes = notes[:3997].rsplit(" ", 1)[0].rstrip() + "..."
    return {
        "version": version,
        "notes": notes,
        "pub_date": date.isoformat(timespec="seconds"),
        "platforms": platforms,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--assets-dir", required=True, type=pathlib.Path)
    parser.add_argument("--release-json", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()
    try:
        release = json.loads(args.release_json.read_text(encoding="utf-8"))
        manifest = build_manifest(args.tag, args.repo, args.assets_dir, release)
        args.output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    except (OSError, ValueError, UnicodeError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
