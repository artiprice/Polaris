import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("build-context.py")
SPEC = importlib.util.spec_from_file_location("sandbox_build_context", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load sandbox build-context script")
BUILD_CONTEXT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILD_CONTEXT)


class CopyWorkspacePathPackagesTests(unittest.TestCase):
    def test_copies_only_workspace_referenced_vendor_roots_with_their_sources(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "workspace"
            output = Path(temporary) / "context"
            for relative in (
                "vendor/biscuit-auth/src/lib.rs",
                "vendor/biscuit-auth/build.rs",
                "vendor/docx-rs/src/lib.rs",
                "vendor/unreferenced/src/lib.rs",
            ):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture", encoding="utf-8")
            for package in ("biscuit-auth", "docx-rs", "unreferenced"):
                (root / "vendor" / package / "Cargo.toml").write_text(
                    f'[package]\nname = "{package}"\nversion = "1.0.0"\n',
                    encoding="utf-8",
                )
            output.mkdir()
            workspace = {
                "workspace": {
                    "dependencies": {
                        "biscuit-auth": {"version": "6.0.0", "path": "vendor/biscuit-auth"},
                        "docx-rs": {"version": "0.4.22", "path": "vendor/docx-rs"},
                    }
                }
            }

            copied = BUILD_CONTEXT.copy_workspace_vendor_packages(root, workspace, output)

            self.assertEqual(
                copied,
                {Path("vendor/biscuit-auth"), Path("vendor/docx-rs")},
            )
            self.assertEqual(
                (output / "vendor/biscuit-auth/src/lib.rs").read_text(encoding="utf-8"),
                "fixture",
            )
            self.assertEqual(
                (output / "vendor/docx-rs/src/lib.rs").read_text(encoding="utf-8"),
                "fixture",
            )
            self.assertFalse((output / "vendor/unreferenced").exists())

    def test_rejects_vendor_manifest_name_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "workspace"
            output = Path(temporary) / "context"
            package = root / "vendor" / "actual-name"
            package.mkdir(parents=True)
            (package / "Cargo.toml").write_text(
                '[package]\nname = "different-name"\nversion = "1.0.0"\n',
                encoding="utf-8",
            )
            output.mkdir()
            workspace = {
                "workspace": {
                    "dependencies": {
                        "expected-name": {"path": "vendor/actual-name"},
                    }
                }
            }

            with self.assertRaisesRegex(RuntimeError, "does not match workspace dependency"):
                BUILD_CONTEXT.copy_workspace_vendor_packages(root, workspace, output)

    def test_copies_declared_guest_build_script_support_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "workspace"
            output = Path(temporary) / "context"
            package = root / "crates/polaris-proto"
            package.mkdir(parents=True)
            (package / "build_support.rs").write_text("pub fn fixture() {}", encoding="utf-8")
            (root / "crates/other").mkdir(parents=True)
            (root / "crates/other/build_support.rs").write_text("unused", encoding="utf-8")
            output.mkdir()

            BUILD_CONTEXT.copy_guest_build_support(
                root,
                {Path("crates/polaris-proto")},
                output,
            )

            self.assertEqual(
                (output / "crates/polaris-proto/build_support.rs").read_text(encoding="utf-8"),
                "pub fn fixture() {}",
            )
            self.assertFalse((output / "crates/other/build_support.rs").exists())


if __name__ == "__main__":
    unittest.main()
