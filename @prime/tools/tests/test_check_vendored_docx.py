from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
VENDOR = ROOT / "vendor" / "docx-rs"
CHECKER = ROOT / "@prime" / "tools" / "check-vendored-docx.py"


class VendoredDocxIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.vendor = Path(self.temp.name) / "docx-rs"
        shutil.copytree(VENDOR, self.vendor)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_checker(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                "-I",
                str(CHECKER),
                "--vendor-root",
                str(self.vendor),
            ],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_published_archive_hash_and_file_set_are_pinned(self) -> None:
        result = self.run_checker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS:", result.stdout)

    def test_only_authorized_zip_backport_changes_are_accepted(self) -> None:
        result = self.run_checker()
        self.assertEqual(result.returncode, 0, result.stderr)
        source = self.vendor / "src" / "documents" / "mod.rs"
        source.write_bytes(source.read_bytes() + b"\n// unauthorized\n")
        self.assertNotEqual(self.run_checker().returncode, 0)

    def test_tampered_upstream_source_is_rejected(self) -> None:
        source = self.vendor / "src" / "lib.rs"
        source.write_bytes(source.read_bytes() + b"\n// tampered\n")
        result = self.run_checker()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("src/lib.rs", result.stderr)

    def test_unexpected_source_file_or_manifest_is_rejected(self) -> None:
        (self.vendor / "unexpected.rs").write_text("unexpected", encoding="utf-8")
        result = self.run_checker()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("file set differs", result.stderr)

        (self.vendor / "unexpected.rs").unlink()
        (self.vendor / "src" / "reader" / "read_zip.rs").unlink()
        self.assertNotEqual(self.run_checker().returncode, 0)
        shutil.copyfile(
            VENDOR / "src" / "reader" / "read_zip.rs",
            self.vendor / "src" / "reader" / "read_zip.rs",
        )

        license_file = self.vendor / "LICENSE"
        license_file.write_bytes(license_file.read_bytes() + b"tampered")
        self.assertNotEqual(self.run_checker().returncode, 0)

        shutil.copyfile(VENDOR / "LICENSE", license_file)
        manifest = self.vendor / "UPSTREAM-CHECKSUMS.json"
        manifest.write_bytes(manifest.read_bytes() + b" ")
        self.assertNotEqual(self.run_checker().returncode, 0)


if __name__ == "__main__":
    unittest.main()
