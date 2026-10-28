"""Negative and positive checks for the standalone release import gate."""
import struct
import tempfile
import unittest
from pathlib import Path
from build_static import verify_imports


def fixture(dll: str, *, machine: int = 0x8664, delay: bool = False) -> bytes:
    data = bytearray(1536)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<HH", data, 132, machine, 1)
    struct.pack_into("<H", data, 148, 240)
    struct.pack_into("<H", data, 152, 0x20B)
    struct.pack_into("<I", data, 260, 16)
    struct.pack_into("<I", data, 272, 0x1000)
    if delay:
        struct.pack_into("<I", data, 264 + 13 * 8, 0x1080)
    struct.pack_into("<IIII", data, 400, 1024, 0x1000, 1024, 512)
    struct.pack_into("<I", data, 524, 0x1040)
    encoded = dll.encode("ascii") + b"\0"
    data[576:576 + len(encoded)] = encoded
    return bytes(data)


class ImportGateTests(unittest.TestCase):
    def verify(self, data: bytes):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "fixture.exe"
            binary.write_bytes(data)
            return verify_imports(binary)

    def test_system_import_is_allowed(self):
        self.assertEqual(self.verify(fixture("KERNEL32.dll")), ["KERNEL32.dll"])

    def test_os_ucrt_contract_is_allowed(self):
        self.assertEqual(self.verify(fixture("api-ms-win-crt-runtime-l1-1-0.dll")),
                         ["api-ms-win-crt-runtime-l1-1-0.dll"])

    def test_external_runtimes_and_unknown_dlls_are_rejected(self):
        for name in ["VCRUNTIME140.dll", "MSVCP140.dll", "libgcc_s_seh-1.dll",
                     "libwinpthread-1.dll", "custom-runtime.dll"]:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "Non-system"):
                self.verify(fixture(name))

    def test_delay_imports_require_review(self):
        with self.assertRaisesRegex(ValueError, "delay imports"):
            self.verify(fixture("KERNEL32.dll", delay=True))

    def test_wrong_architecture_and_malformed_input_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "x64"):
            self.verify(fixture("KERNEL32.dll", machine=0x14C))
        with self.assertRaisesRegex(ValueError, "PE executable"):
            self.verify(b"not an executable")


if __name__ == "__main__":
    unittest.main()
