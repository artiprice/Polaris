"""Build the standalone Windows x64 release and verify its PE imports.

Rust/MinGW support libraries are static. Windows system DLLs, including the OS
UCRT, remain dependencies. This is not a freestanding or DLL-free executable.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-gnu"
SYSTEM_DLLS = frozenset({
    "advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "crypt32.dll",
    "dbghelp.dll", "gdi32.dll", "imm32.dll", "kernel32.dll", "msvcrt.dll",
    "ntdll.dll", "ole32.dll", "oleaut32.dll", "psapi.dll", "runtimeobject.dll",
    "secur32.dll", "shell32.dll", "shlwapi.dll", "ucrtbase.dll", "user32.dll",
    "userenv.dll", "version.dll", "winmm.dll", "ws2_32.dll",
})


def pe_imports(path: Path) -> list[str]:
    data = path.read_bytes()

    def u16(offset: int) -> int:
        return struct.unpack_from("<H", data, offset)[0]

    def u32(offset: int) -> int:
        return struct.unpack_from("<I", data, offset)[0]

    if data[:2] != b"MZ":
        raise ValueError("Not a Windows PE executable")
    pe = u32(0x3C)
    if data[pe:pe + 4] != b"PE\0\0" or u16(pe + 4) != 0x8664:
        raise ValueError("Expected a Windows x64 executable")
    optional = pe + 24
    if u16(optional) != 0x20B:
        raise ValueError("Expected a PE32+ optional header")
    directories = optional + 112
    if u32(optional + 108) < 14:
        raise ValueError("Missing PE import directories")
    sections = []
    for index in range(u16(pe + 6)):
        pos = optional + u16(pe + 20) + 40 * index
        sections.append((u32(pos + 12), max(u32(pos + 8), u32(pos + 16)), u32(pos + 20)))

    def offset(rva: int) -> int:
        for address, size, raw in sections:
            if address <= rva < address + size:
                result = raw + rva - address
                if result >= len(data):
                    break
                return result
        raise ValueError(f"Invalid PE RVA: {rva:#x}")

    # There are no delay imports in this application. Fail closed if that changes,
    # so a newly introduced runtime cannot bypass this direct-import check.
    if u32(directories + 13 * 8):
        raise ValueError("Unexpected delay imports; inspect them before distributing")
    imports = []
    import_rva = u32(directories + 8)
    if not import_rva:
        raise ValueError("Expected Windows system imports")
    pos = offset(import_rva)
    for _ in range(4096):
        if data[pos:pos + 20] == bytes(20):
            return sorted(set(imports), key=str.lower)
        name = offset(u32(pos + 12))
        end = data.index(0, name)
        imports.append(data[name:end].decode("ascii"))
        pos += 20
    raise ValueError("Unterminated PE import table")


def verify_imports(path: Path) -> list[str]:
    imports = pe_imports(path)
    external = [
        name for name in imports
        if name.lower() not in SYSTEM_DLLS
        and not name.lower().startswith(("api-ms-win-", "ext-ms-win-"))
    ]
    if external:
        raise ValueError("Non-system DLL dependencies: " + ", ".join(external))
    return imports


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--test", action="store_true", help="Run the release suite before packaging its exact executable")
    parser.add_argument("--verify-only", type=Path)
    args = parser.parse_args()
    if args.verify_only:
        print(json.dumps({"imports": verify_imports(args.verify_only)}, indent=2))
        return
    command = ["cargo", "static"]
    if args.offline:
        command.append("--offline")
    subprocess.run(command, cwd=ROOT, check=True)
    if args.test:
        test = ["cargo", "test", "--release", "--locked", "--target", TARGET, "--target-dir", "target/static"]
        if args.offline:
            test.append("--offline")
        subprocess.run(test, cwd=ROOT, check=True)
    source = ROOT / "target" / "static" / TARGET / "release" / "polaris.exe"
    imports = verify_imports(source)
    output = ROOT / "dist"
    output.mkdir(exist_ok=True)
    destination = output / "polaris.exe"
    shutil.copyfile(source, destination)
    if source.read_bytes() != destination.read_bytes():
        raise ValueError("Packaged executable differs from verified build")
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    (output / "SHA256SUMS.txt").write_text(f"{digest}  polaris.exe\n", encoding="ascii")
    report = {
        "target": TARGET, "bytes": destination.stat().st_size, "sha256": digest,
        "imports": imports,
        "runtime": "Static Rust/MinGW support; Windows system DLLs including OS UCRT required",
    }
    (output / "build-info.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"binary": str(destination), **report}, indent=2))


if __name__ == "__main__":
    main()
