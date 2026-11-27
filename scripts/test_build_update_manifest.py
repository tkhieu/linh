"""Tests for build-update-manifest.py using small synthetic release assets."""

import base64
import importlib.util
import pathlib
import tempfile
import unittest


_SPEC = importlib.util.spec_from_file_location(
    "build_update_manifest",
    pathlib.Path(__file__).resolve().parent / "build-update-manifest.py",
)
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)

NAMES = (
    "linh-0.3.2-1.x86_64.rpm",
    "linh_0.3.2_amd64.deb",
    "linh_0.3.2_x64-setup.exe",
    "linh.app.tar.gz",
)


def signature(name, version="0.3.2"):
    contents = f"untrusted comment: synthetic\nAA==\ntrusted comment: timestamp:1\tfile:{name}\tversion:{version}\nAA==\n"
    return base64.b64encode(contents.encode()).decode()


class BuildUpdateManifestTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.assets = pathlib.Path(self.temp.name)
        for name in NAMES:
            (self.assets / name).write_bytes(b"artifact")
            (self.assets / (name + ".sig")).write_text(signature(name))
        self.release = {
            "tag_name": "v0.3.2",
            "draft": False,
            "prerelease": False,
            "published_at": "2026-11-27T10:00:00Z",
            "body": "Release notes",
        }

    def build(self, tag="v0.3.2"):
        return _MODULE.build_manifest(tag, "tkhieu/linh", self.assets, self.release)

    def test_static_manifest_and_windows_fallback(self):
        manifest = self.build()
        self.assertEqual(manifest["version"], "0.3.2")
        self.assertEqual(manifest["pub_date"], "2026-11-27T10:00:00+00:00")
        self.assertEqual(manifest["notes"], "Release notes")
        platforms = manifest["platforms"]
        self.assertEqual(set(platforms), {
            "linux-x86_64-rpm", "linux-x86_64-deb", "windows-x86_64-nsis",
            "windows-x86_64", "darwin-aarch64", "darwin-x86_64",
        })
        self.assertEqual(platforms["windows-x86_64"], platforms["windows-x86_64-nsis"])
        self.assertEqual(platforms["darwin-aarch64"], platforms["darwin-x86_64"])
        self.assertEqual(platforms["linux-x86_64-rpm"]["signature"], signature(NAMES[0]))
        self.assertEqual(platforms["linux-x86_64-rpm"]["url"],
                         "https://github.com/tkhieu/linh/releases/download/v0.3.2/" + NAMES[0])

    def test_missing_or_empty_assets_fail(self):
        for name in NAMES:
            for suffix in ("", ".sig"):
                with self.subTest(name=name, suffix=suffix):
                    path = self.assets / (name + suffix)
                    content = path.read_bytes()
                    path.unlink()
                    with self.assertRaisesRegex(ValueError, "artifact|signature"):
                        self.build()
                    path.write_bytes(b"")
                    with self.assertRaisesRegex(ValueError, "artifact|signature"):
                        self.build()
                    path.write_bytes(content)

    def test_duplicate_artifact_or_signature_fails(self):
        nested = self.assets / "nested"
        nested.mkdir()
        for suffix in ("", ".sig"):
            name = NAMES[0] + suffix
            copy = nested / name
            copy.write_bytes((self.assets / name).read_bytes())
            with self.assertRaisesRegex(ValueError, "duplicate"):
                self.build()
            copy.unlink()

    def test_wrong_filename_version_fails(self):
        original = self.assets / NAMES[1]
        renamed = self.assets / NAMES[1].replace("0.3.2", "0.3.3")
        original.rename(renamed)
        (self.assets / (NAMES[1] + ".sig")).rename(self.assets / (renamed.name + ".sig"))
        with self.assertRaisesRegex(ValueError, "does not match tag"):
            self.build()

    def test_wrong_signature_version_or_file_fails(self):
        path = self.assets / (NAMES[3] + ".sig")
        path.write_text(signature(NAMES[3], "0.3.3"))
        with self.assertRaisesRegex(ValueError, "signature metadata"):
            self.build()
        path.write_text(signature("other.app.tar.gz"))
        with self.assertRaisesRegex(ValueError, "signature metadata"):
            self.build()

    def test_invalid_signature_encoding_fails(self):
        (self.assets / (NAMES[0] + ".sig")).write_text("not-base64")
        with self.assertRaisesRegex(ValueError, "signature encoding"):
            self.build()

    def test_release_metadata_and_tag_are_validated(self):
        with self.assertRaisesRegex(ValueError, "invalid release tag"):
            self.build("v0.3.2-beta")
        self.release["draft"] = True
        with self.assertRaisesRegex(ValueError, "draft/prerelease"):
            self.build()
        self.release["draft"] = False
        self.release["published_at"] = "2026-11-27T10:00:00"
        with self.assertRaisesRegex(ValueError, "timezone"):
            self.build()

    def test_long_notes_truncate(self):
        self.release["body"] = "word " * 1000
        notes = self.build()["notes"]
        self.assertLessEqual(len(notes), 4000)
        self.assertTrue(notes.endswith("..."))


if __name__ == "__main__":
    unittest.main()
