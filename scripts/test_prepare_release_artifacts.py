#!/usr/bin/env python3
"""Unit tests for prepare-release-artifacts.py. Run with: python3 -m unittest discover -s scripts -p 'test_*.py'"""

import importlib.util
import pathlib
import shutil
import tempfile
import unittest


_SPEC = importlib.util.spec_from_file_location(
    "prepare_release_artifacts",
    pathlib.Path(__file__).resolve().parent / "prepare-release-artifacts.py",
)
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)
build_release_assets = _MODULE.build_release_assets


INSTALLERS = [
    "linh-0.3.2-1.x86_64.rpm",
    "linh_0.3.2_amd64.deb",
    "Linh_0.3.2_x64-setup.exe",
    "Linh_0.3.2_x64.msix",
    "linh.app.tar.gz",
    "linh_0.3.2_universal.dmg",
]

SIGS = [
    "linh-0.3.2-1.x86_64.rpm.sig",
    "linh_0.3.2_amd64.deb.sig",
    "Linh_0.3.2_x64-setup.exe.sig",
    "linh.app.tar.gz.sig",
]

COMPLETE_SET = INSTALLERS + SIGS


class BuildReleaseAssetsTest(unittest.TestCase):
    def _write(self, source: pathlib.Path, names: list[str]) -> None:
        for i, name in enumerate(names):
            (source / name).write_bytes(f"content-{i}".encode())

    def _run(self, names: list[str], *, unsigned: bool = False):
        tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, tmp, ignore_errors=True)
        source = pathlib.Path(tmp) / "dist"
        destination = pathlib.Path(tmp) / "release-assets"
        source.mkdir()
        self._write(source, names)
        code = build_release_assets(source, destination, require_signatures=not unsigned)
        return code, destination

    def test_complete_set_succeeds_and_writes_checksums(self) -> None:
        code, destination = self._run(COMPLETE_SET)

        self.assertEqual(code, 0)
        for name in COMPLETE_SET:
            self.assertTrue((destination / name).is_file())
        sums_text = (destination / "SHA256SUMS").read_text()
        lines = sums_text.strip().splitlines()
        self.assertEqual(len(lines), len(COMPLETE_SET))
        for name in COMPLETE_SET:
            self.assertTrue(any(line.endswith(f"  {name}") for line in lines))

    def test_unsigned_dry_run_with_no_sigs_succeeds(self) -> None:
        code, destination = self._run(INSTALLERS, unsigned=True)

        self.assertEqual(code, 0)
        for name in INSTALLERS:
            self.assertTrue((destination / name).is_file())

    def test_unsigned_mode_rejects_a_present_sig(self) -> None:
        # The signing step either runs for every signable artifact or not at
        # all; a lone .sig alongside an otherwise-unsigned build means the
        # signing policy and the build disagree, so it must fail loudly.
        code, _ = self._run(INSTALLERS + ["linh.app.tar.gz.sig"], unsigned=True)

        self.assertEqual(code, 1)

    def test_signed_mode_rejects_missing_sigs(self) -> None:
        code, _ = self._run(INSTALLERS, unsigned=False)

        self.assertEqual(code, 1)

    def test_missing_sig_fails(self) -> None:
        names = [n for n in COMPLETE_SET if n != "linh.app.tar.gz.sig"]
        code, _ = self._run(names)

        self.assertEqual(code, 1)

    def test_missing_rpm_fails(self) -> None:
        names = [n for n in COMPLETE_SET if not n.endswith(".rpm")]
        code, _ = self._run(names)

        self.assertEqual(code, 1)

    def test_extra_unexpected_file_fails(self) -> None:
        code, _ = self._run(COMPLETE_SET + ["stray.txt"])

        self.assertEqual(code, 1)

    def test_replaced_signature_fails(self) -> None:
        # The dmg's signature in place of the app archive's: same count of
        # .sig files, but a counting-only check would wrongly accept this.
        names = [n for n in COMPLETE_SET if n != "linh.app.tar.gz.sig"]
        names.append("linh_0.3.2_universal.dmg.sig")

        code, _ = self._run(names)

        self.assertEqual(code, 1)

    def test_stray_signature_fails(self) -> None:
        # A .sig whose base name matches no installer at all.
        code, _ = self._run(COMPLETE_SET + ["unexpected-file.sig"])

        self.assertEqual(code, 1)

    def test_signature_for_msix_fails(self) -> None:
        # msix is never signed for the updater; a .sig next to it is a defect.
        code, _ = self._run(COMPLETE_SET + ["Linh_0.3.2_x64.msix.sig"])

        self.assertEqual(code, 1)

    def test_signature_for_dmg_fails(self) -> None:
        # dmg is never signed for the updater either.
        code, _ = self._run(COMPLETE_SET + ["linh_0.3.2_universal.dmg.sig"])

        self.assertEqual(code, 1)

    def test_duplicate_filenames_in_different_dirs_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "dist"
            destination = pathlib.Path(tmp) / "release-assets"
            (source / "a").mkdir(parents=True)
            (source / "b").mkdir(parents=True)
            self._write(source / "a", COMPLETE_SET)
            (source / "b" / "linh.app.tar.gz.sig").write_bytes(b"dup")

            code = build_release_assets(source, destination, require_signatures=True)

            self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main()
