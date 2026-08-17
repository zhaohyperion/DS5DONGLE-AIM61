import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile


class PackageFirmwareTests(unittest.TestCase):
    def assert_package(self, board: str, usb_speed: str, profile: str = "standard") -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, output = root / "firmware", root / "dist"
            source.mkdir()
            (source / "boot2_bl616_test.bin").write_bytes(b"b" * 52_576)
            (source / "partition.bin").write_bytes(b"p" * 308)
            suffix = "-hs" if usb_speed == "hs" else ""
            profile_suffix = "-diag" if profile == "diagnostic" else ""
            firmware_name = f"ds5dongle-{board}{suffix}{profile_suffix}.bin"
            (source / firmware_name).write_bytes(b"f" * 65_536)
            script = Path(__file__).with_name("package_firmware.py")
            subprocess.run(
                [
                    sys.executable,
                    str(script),
                    "--board",
                    board,
                    "--usb-speed",
                    usb_speed,
                    "--version",
                    "v1.2.3",
                    "--profile",
                    profile,
                    "--firmware-dir",
                    str(source),
                    "--output-dir",
                    str(output),
                ],
                check=True,
            )
            package = next(output.glob("*.zip"))
            with zipfile.ZipFile(package) as archive:
                manifest = json.loads(archive.read("firmware.json"))
                self.assertEqual(manifest["board"], board)
                self.assertEqual(manifest["usb_speed"], usb_speed)
                self.assertEqual(manifest["profile"], profile)
                self.assertIn(firmware_name, archive.namelist())
                self.assertIn("firmware.json", archive.read("SHA256SUMS.txt").decode())

    def test_aim61_default_package_contains_manifest_and_checksums(self):
        self.assert_package("aim61", "fs")

    def test_lctech_compatibility_package_is_retained(self):
        self.assert_package("lctech616", "fs")

    def test_aim61_diagnostic_high_speed_name_and_manifest(self):
        self.assert_package("aim61", "hs", "diagnostic")


if __name__ == "__main__":
    unittest.main()
