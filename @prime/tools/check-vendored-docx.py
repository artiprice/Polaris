#!/usr/bin/env python3
"""Verify the narrowly patched docx-rs 0.4.22 source tree."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_VENDOR = ROOT / "vendor" / "docx-rs"
MANIFEST_SHA256 = "b6a3135b182ea9ee7bdbfb93a208e450c253229fc9ffd7e82a2619f3bccb9b38"
PATCH_NOTES_SHA256 = "ba9965a751319efb929ce752ab2e0a88f60be6bbac4632f1ac86935beedcb1cf"
LICENSE_SHA256 = "718ca36cb18f8ee3ddf9fdffc5f12fc17fad6328d40d8f13bee0a6daa7e1416e"
ALLOWED_EXTRA_FILES = {"LICENSE", "UPSTREAM-CHECKSUMS.json", "POLARIS-PATCH.md"}
PATCHES = {
    "Cargo.toml": (b'version = "9.0.4"', b'version = "8.6.0"'),
    "Cargo.toml.orig": (b'version = "9.0.4"', b'version = "8.6.0"'),
    "src/documents/mod.rs": (b"FileOptions<'_, '_, ()>", b"FileOptions<'_, ()>"),
}


class VerificationError(Exception):
    """The vendored source no longer matches its pinned provenance."""


def fail(message: str) -> None:
    raise VerificationError(message)


def transformed_baseline(path: str, data: bytes) -> bytes:
    if path == "Cargo.toml":
        text = data.decode("utf-8")
        section_pattern = re.compile(
            r"(?ms)(^\[dependencies\.zip\]\s*\n)(.*?)(?=^\[|\Z)"
        )
        sections = list(section_pattern.finditer(text))
        if len(sections) != 1:
            fail("normalized Cargo.toml must contain one [dependencies.zip] section")
        section = sections[0]
        body = section.group(2)
        if body.count('version = "9.0.4"') != 1:
            fail("normalized ZIP requirement differs from the authorized change")
        body = body.replace('version = "9.0.4"', 'version = "8.6.0"', 1)
        text = text[: section.start(2)] + body + text[section.end(2) :]
        return text.encode("utf-8")

    new, old = PATCHES[path]
    if data.count(new) != 1:
        fail(f"{path} does not contain exactly one authorized patched form")
    return data.replace(new, old, 1)


def verify(vendor: Path) -> None:
    manifest_path = vendor / "UPSTREAM-CHECKSUMS.json"
    try:
        manifest_bytes = manifest_path.read_bytes()
    except OSError as error:
        fail(f"cannot read pinned manifest: {error}")
    if hashlib.sha256(manifest_bytes).hexdigest() != MANIFEST_SHA256:
        fail("UPSTREAM-CHECKSUMS.json differs from the checker-pinned manifest")
    try:
        manifest = json.loads(manifest_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid pinned manifest JSON: {error}")
    if (manifest.get("crate"), manifest.get("version")) != ("docx-rs", "0.4.22"):
        fail("manifest does not identify docx-rs 0.4.22")
    if manifest.get("archive_sha256") != "7fdf00e8af6d0b3e92d4bbf9b76f773d8b84ea80f310324ad16cbdc2e653e02c":
        fail("published archive hash differs from the approved crate archive")
    if manifest.get("upstream_revision") != "f04cf8b47a925b2b6fff850dcb75069aae89dab4":
        fail("published source revision differs from the approved revision")
    if manifest.get("security_fix_commit") != "ad7fede6a7f3b61a097c28d0c39dd04626e44701":
        fail("security backport commit differs from the approved upstream commit")
    files = manifest.get("files")
    if not isinstance(files, dict) or not files:
        fail("manifest source file map is missing")
    expected_paths = set(files) | ALLOWED_EXTRA_FILES
    observed_paths: set[str] = set()
    for path in vendor.rglob("*"):
        relative = path.relative_to(vendor).as_posix()
        if path.is_symlink():
            fail(f"symlinks are not allowed: {relative}")
        if path.is_file():
            observed_paths.add(relative)
    if observed_paths != expected_paths:
        missing = sorted(expected_paths - observed_paths)
        extra = sorted(observed_paths - expected_paths)
        fail(f"vendored file set differs (missing={missing}, extra={extra})")
    notes_hash = hashlib.sha256((vendor / "POLARIS-PATCH.md").read_bytes()).hexdigest()
    if notes_hash != PATCH_NOTES_SHA256:
        fail("POLARIS-PATCH.md differs from the approved patch record")
    license_hash = hashlib.sha256((vendor / "LICENSE").read_bytes()).hexdigest()
    if license_hash != LICENSE_SHA256:
        fail("LICENSE differs from the approved upstream revision")

    for relative, expected_hash in files.items():
        if not isinstance(relative, str) or not isinstance(expected_hash, str):
            fail("manifest contains a malformed path or digest")
        path = vendor.joinpath(*relative.split("/"))
        try:
            data = path.read_bytes()
        except OSError as error:
            fail(f"cannot read upstream file {relative}: {error}")
        if relative in PATCHES:
            data = transformed_baseline(relative, data)
        if hashlib.sha256(data).hexdigest() != expected_hash:
            fail(f"upstream file differs beyond authorized patch: {relative}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--vendor-root", type=Path, default=DEFAULT_VENDOR)
    args = parser.parse_args()
    try:
        verify(args.vendor_root)
    except VerificationError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print("PASS: docx-rs 0.4.22 source provenance and ZIP 9 backport verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
