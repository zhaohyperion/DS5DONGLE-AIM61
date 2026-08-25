from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile


class PackageOtaTests(unittest.TestCase):
    def test_creates_ota_only_archive_with_explicit_name(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            image = root / "DS5Dongle-aim61-hs-standard-v3.6.0.bin.ota"
            manifest = root / "DS5Dongle-aim61-hs-standard-v3.6.0.ota.json"
            archive = root / "DS5Dongle-aim61-hs-standard-ota-v3.6.0.zip"
            image_bytes = b"signed raw OTA test image"
            image.write_bytes(image_bytes)
            manifest.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "board": "aim61",
                        "usb_speed": "hs",
                        "profile": "standard",
                        "version": "3.6.0",
                        "sha256": hashlib.sha256(image_bytes).hexdigest(),
                        "url": f"https://example.test/{archive.name}",
                        "signature": {"algorithm": "test"},
                    }
                ),
                encoding="utf-8",
            )
            script = Path(__file__).with_name("package_ota_zip.py")
            subprocess.run(
                [
                    sys.executable,
                    str(script),
                    "--image",
                    str(image),
                    "--manifest",
                    str(manifest),
                    "--archive",
                    str(archive),
                ],
                check=True,
            )

            with zipfile.ZipFile(archive) as package:
                self.assertEqual(
                    sorted(package.namelist()),
                    sorted([image.name, manifest.name, "SHA256SUMS.txt"]),
                )
                checksums = package.read("SHA256SUMS.txt").decode("ascii")
                self.assertIn(image.name, checksums)
                self.assertIn(manifest.name, checksums)
                self.assertNotIn("partition.bin", package.namelist())
                self.assertNotIn("firmware.json", package.namelist())

    def test_rejects_ambiguous_archive_name(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            image = root / "image.bin.ota"
            manifest = root / "image.ota.json"
            image.write_bytes(b"image")
            manifest.write_text(
                json.dumps(
                    {
                        "board": "aim61",
                        "usb_speed": "hs",
                        "profile": "diagnostic",
                        "version": "3.6.0",
                        "sha256": hashlib.sha256(b"image").hexdigest(),
                        "url": "https://example.test/ambiguous.zip",
                        "signature": {"algorithm": "test"},
                    }
                ),
                encoding="utf-8",
            )
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("package_ota_zip.py")),
                    "--image",
                    str(image),
                    "--manifest",
                    str(manifest),
                    "--archive",
                    str(root / "ambiguous.zip"),
                ],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("must be named", result.stderr)


if __name__ == "__main__":
    unittest.main()
