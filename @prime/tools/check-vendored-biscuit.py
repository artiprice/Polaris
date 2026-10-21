#!/usr/bin/env python3
"""Verify published Biscuit 6.0.0 bytes modulo exactly seven missing feature guards."""
import hashlib
import json
from pathlib import Path
import re
import sys

MANIFEST_SHA256 = "9f429106af54670b0d5bf5e32992546818aa338c44558a28ec5ae5c5f894a4ca"
ARCHIVE_SHA256 = "d5884fc86b3e21f5649ef4326e17ef729b3096e6502deaf13db7b7fb05bb992b"
PATCHED = {f"src/token/builder/{name}.rs" for name in ("check", "fact", "policy", "rule", "term")}
GUARD = re.compile(rb'#\[cfg\(feature = "datalog-macro"\)\]\nuse super::ToAnyParam;\n(use super::\{[^\n]*)(\};\n)')
OTHER_GUARDS = {
    "src/token/builder.rs": (b'#[cfg(any(feature = "datalog-macro", test))]\nuse crate::crypto::PublicKey;', b'use crate::crypto::PublicKey;'),
    "src/crypto/mod.rs": (b'#[cfg(feature = "pem")]\nfn parse_any_algorithm', b'fn parse_any_algorithm'),
}


def verify(root: Path) -> None:
    root = root.resolve(strict=True)
    encoded = (root / "UPSTREAM-CHECKSUMS.json").read_bytes()
    if hashlib.sha256(encoded).hexdigest() != MANIFEST_SHA256:
        raise ValueError("published checksum manifest changed")
    manifest = json.loads(encoded)
    if manifest["archive_sha256"] != ARCHIVE_SHA256:
        raise ValueError("unexpected upstream archive")
    expected = set(manifest["files"]) | {"UPSTREAM-CHECKSUMS.json", "POLARIS-PATCH.md"}
    actual = set()
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"symlink in vendor tree: {path}")
        if path.is_file():
            actual.add(path.relative_to(root).as_posix())
    if actual != expected:
        raise ValueError(f"vendor file-set mismatch: extra={sorted(actual-expected)} missing={sorted(expected-actual)}")
    for relative, digest in manifest["files"].items():
        path = root / relative
        if not path.resolve().is_relative_to(root):
            raise ValueError("upstream path escapes vendor root")
        data = path.read_bytes()
        if relative in PATCHED:
            data, count = GUARD.subn(lambda match: match.group(1) + b", ToAnyParam" + match.group(2), data)
            if count != 1:
                raise ValueError(f"missing/extra authorized import guard: {relative}")
        if relative in OTHER_GUARDS:
            patched, original = OTHER_GUARDS[relative]
            if data.count(patched) != 1:
                raise ValueError(f"missing/extra authorized feature guard: {relative}")
            data = data.replace(patched, original)
        if relative == "src/format/schema.rs":
            # Preserve every generated token/tag/string; reverse only rustfmt's spaces around =
            # inside prost attributes. This lets the original `cargo fmt --all --check` stay intact.
            data = re.sub(rb'#\[prost\([^\n]*\)\]', lambda match: match.group().replace(b' = ', b'='), data)
            data = data.replace(b'pub struct Empty {}\n', b'pub struct Empty {\n}\n')
        if hashlib.sha256(data).hexdigest() != digest:
            raise ValueError(f"unexpected upstream change: {relative}")
    print(f"check-vendored-biscuit: {len(manifest['files'])} published files; 7 feature guards + prost attribute formatting -> OK")


if __name__ == "__main__":
    try:
        verify(Path(sys.argv[1]) if len(sys.argv) == 2 else Path(__file__).resolve().parents[2] / "vendor" / "biscuit-auth")
    except (OSError, ValueError, KeyError) as error:
        print(f"check-vendored-biscuit: FAILED: {error}")
        sys.exit(1)
