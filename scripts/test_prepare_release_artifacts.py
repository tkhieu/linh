#!/usr/bin/env python3
"""Unit tests for prepare-release-artifacts.py. Run with: python3 -m unittest discover -s scripts -p 'test_*.py'"""

import importlib.util
import pathlib
import tempfile
import unittest


_SPEC = importlib.util.spec_from_file_location(
    "prepare_release_artifacts",
    pathlib.Path(__file__).resolve().parent / "prepare-release-artifacts.py",
)
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)
build_release_assets = _MODULE.build_release_assets


COMPLETE_SET = [
    "linh-0.3.2-1.x86_64.rpm",
    "linh-0.3.2-1.x86_64.rpm.sig",
    "linh_0.3.2_amd64.deb",
    "linh_0.3.2_amd64.deb.sig",
    "Linh_0.3.2_x64-setup.exe",
    "Linh_0.3.2_x64-setup.exe.sig",
    "Linh_0.3.2_x64.msix",
    "linh.app.tar.gz",
    "linh.app.tar.gz.sig",
    "linh_0.3.2_universal.dmg",
]


class BuildReleaseAssetsTest(unittest.TestCase):
    def _write(self, source: pathlib.Path, names: list[str]) -> None:
        for i, name in enumerate(names):
            (source / name).write_bytes(f"content-{i}".encode())

    def test_complete_set_succeeds_and_writes_checksums(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "dist"
            destination = pathlib.Path(tmp) / "release-assets"
            source.mkdir()
            self._write(source, COMPLETE_SET)

            code = build_release_assets(source, destination)

            self.assertEqual(code, 0)
            for name in COMPLETE_SET:
                self.assertTrue((destination / name).is_file())
            sums_text = (destination / "SHA256SUMS").read_text()
            lines = sums_text.strip().splitlines()
            self.assertEqual(len(lines), len(COMPLETE_SET))
            for name in COMPLETE_SET:
                self.assertTrue(any(line.endswith(f"  {name}") for line in lines))

    def test_missing_sig_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "dist"
            destination = pathlib.Path(tmp) / "release-assets"
            source.mkdir()
            names = [n for n in COMPLETE_SET if n != "linh.app.tar.gz.sig"]
            self._write(source, names)

            code = build_release_assets(source, destination)

            self.assertEqual(code, 1)

    def test_missing_rpm_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "dist"
            destination = pathlib.Path(tmp) / "release-assets"
            source.mkdir()
            names = [n for n in COMPLETE_SET if not n.endswith(".rpm")]
            self._write(source, names)

            code = build_release_assets(source, destination)

            self.assertEqual(code, 1)

    def test_extra_unexpected_file_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "dist"
            destination = pathlib.Path(tmp) / "release-assets"
            source.mkdir()
            self._write(source, COMPLETE_SET + ["stray.txt"])

            code = build_release_assets(source, destination)

            self.assertEqual(code, 1)

    def test_duplicate_filenames_in_different_dirs_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "dist"
            destination = pathlib.Path(tmp) / "release-assets"
            (source / "a").mkdir(parents=True)
            (source / "b").mkdir(parents=True)
            self._write(source / "a", COMPLETE_SET)
            (source / "b" / "linh.app.tar.gz.sig").write_bytes(b"dup")

            code = build_release_assets(source, destination)

            self.assertEqual(code, 1)

    def test_wrong_sig_count_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "dist"
            destination = pathlib.Path(tmp) / "release-assets"
            source.mkdir()
            # A fifth .sig (the dmg is never signed for the updater) must
            # be rejected even though every other required file is present.
            self._write(source, COMPLETE_SET + ["linh_0.3.2_universal.dmg.sig"])

            code = build_release_assets(source, destination)

            self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main()
