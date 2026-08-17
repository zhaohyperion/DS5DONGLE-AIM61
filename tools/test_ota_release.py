from __future__ import annotations

import base64
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

from tools import ota_release


OPENSSL = shutil.which("openssl")
if OPENSSL is None:
    git_openssl = Path(r"C:\Program Files\Git\usr\bin\openssl.exe")
    if git_openssl.is_file():
        OPENSSL = str(git_openssl)


def make_raw_ota(body: bytes = b"DS5Dongle OTA body" * 64) -> bytes:
    header = bytearray(b"\xFF" * ota_release.OTA_HEADER_SIZE)
    header[:16] = ota_release.OTA_MAGIC
    header[16:20] = ota_release.OTA_RAW_TYPE
    struct.pack_into("<I", header, 20, len(body))
    header[24:32] = bytes(range(1, 9))
    header[32:48] = b"BFL_Module_v1.1\x00"
    header[48:64] = b"EVENT_V1.2.3\x00\x00\x00\x00"
    header[64:96] = hashlib.sha256(body).digest()
    return bytes(header) + body


class OtaReleaseTests(unittest.TestCase):
    def test_semver_components_are_limited_to_sdk_range(self) -> None:
        self.assertEqual(ota_release._validate_version("v254.5.0"), "254.5.0")
        for version in ("255.0.0", "0.255.0", "0.0.255"):
            with self.subTest(version=version):
                with self.assertRaisesRegex(ota_release.OtaReleaseError, "0..254"):
                    ota_release._validate_version(version)
        for version in ("254.0.254", "254.254.254"):
            with self.subTest(version=version):
                with self.assertRaisesRegex(ota_release.OtaReleaseError, "at most 8"):
                    ota_release._validate_version(version)

    def test_release_workflow_builds_and_signs_two_m61_hs_profiles(self) -> None:
        workflow = (Path(__file__).resolve().parents[1] / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn('FIRMWARE_VERSION: 3.5.2', workflow)
        self.assertIn('PROJECT_SDK_VERSION: 3.5.2', workflow)
        self.assertIn('profile: [standard, diagnostic]', workflow)
        self.assertIn('DS5Dongle-aim61-hs$suffix-v3.5.2.bin.ota', workflow)
        self.assertIn("foreach ($profile in @('standard', 'diagnostic'))", workflow)
        self.assertIn('--profile $profile', workflow)
        self.assertIn("secrets.OTA_P256_PRIVATE_KEY_B64", workflow)
        self.assertIn("vars.OTA_P256_KEY_ID", workflow)
        self.assertGreaterEqual(
            workflow.count("vars.OTA_P256_PUBLIC_KEY_SEC1_B64"), 2
        )
        self.assertIn("$env:DS5_OTA_PUBLIC_KEY_FILE = $keyPath", workflow)
        cmake = (Path(__file__).resolve().parents[1] / "CMakeLists.txt").read_text(
            encoding="utf-8"
        )
        self.assertIn("DS5_OTA_RELEASE_PUBLIC_KEY_CONFIGURED 1", cmake)
        self.assertIn("tools\\ota_release.py export-public-key", workflow)
        self.assertIn("$configuredHash = (Get-FileHash", workflow)
        self.assertIn("if ($configuredHash -ne $derivedHash)", workflow)
        self.assertIn("path: dist/*", workflow)
        self.assertIn("runs-on: windows-2025", workflow)
        self.assertIn("tools\\repack_firmware_zip.py", workflow)
        self.assertNotIn("ED25519", workflow.upper())

    def test_parses_raw_header_and_checks_both_hashes(self) -> None:
        image = make_raw_ota()
        info = ota_release.parse_ota_bytes(image)
        self.assertEqual(info.size, len(image))
        self.assertEqual(info.body_size, len(image) - ota_release.OTA_HEADER_SIZE)
        self.assertEqual(info.body_sha256, hashlib.sha256(image[512:]).hexdigest())
        self.assertEqual(info.sha256, hashlib.sha256(image).hexdigest())
        self.assertEqual(info.hardware_version, "BFL_Module_v1.1")
        self.assertEqual(info.software_version, "EVENT_V1.2.3")

    def test_rejects_non_raw_body_length_and_body_hash(self) -> None:
        image = bytearray(make_raw_ota())
        image[16:20] = b"XZ  "
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "type must be RAW"):
            ota_release.parse_ota_bytes(bytes(image))

        image = bytearray(make_raw_ota())
        struct.pack_into("<I", image, 20, len(image) - 511)
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "length mismatch"):
            ota_release.parse_ota_bytes(bytes(image))

        image = bytearray(make_raw_ota())
        image[-1] ^= 0x01
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "SHA-256 mismatch"):
            ota_release.parse_ota_bytes(bytes(image))

    def test_manifest_is_reproducible_and_unsigned_dev_verifies(self) -> None:
        info = ota_release.parse_ota_bytes(make_raw_ota())
        arguments = {
            "channel": "dev",
            "version": "v1.2.3",
            "url": "https://downloads.example.test/DS5Dongle-aim61-fs.bin.ota",
            "allow_unsigned_dev": True,
        }
        first = ota_release.build_manifest(info, **arguments)
        second = ota_release.build_manifest(info, **arguments)
        self.assertEqual(ota_release.manifest_bytes(first), ota_release.manifest_bytes(second))
        self.assertIsNone(first["signature"])
        self.assertEqual(first["board"], "aim61")
        self.assertEqual(first["usb_speed"], "fs")
        self.assertEqual(first["version"], "1.2.3")
        self.assertEqual(first["body_size"], info.body_size)
        self.assertEqual(first["body_sha256"], info.body_sha256)
        ota_release.verify_manifest(first, info, allow_unsigned_dev=True)

    def test_high_speed_manifest_binds_high_speed_target(self) -> None:
        info = ota_release.parse_ota_bytes(make_raw_ota())
        manifest = ota_release.build_manifest(
            info,
            channel="dev",
            version="1.2.3",
            url="https://downloads.example.test/DS5Dongle-aim61-hs.bin.ota",
            usb_speed="hs",
            allow_unsigned_dev=True,
        )
        self.assertEqual(manifest["usb_speed"], "hs")
        canonical = ota_release.signature_payload(manifest, info)
        self.assertEqual(canonical[16], ota_release.MANIFEST_BOARD_ID)
        self.assertEqual(canonical[17], ota_release.MANIFEST_USB_SPEED_IDS["hs"])
        ota_release.verify_manifest(manifest, info, allow_unsigned_dev=True)

    def test_stable_manifest_cannot_be_unsigned(self) -> None:
        info = ota_release.parse_ota_bytes(make_raw_ota())
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "unsigned manifests"):
            ota_release.build_manifest(
                info,
                channel="stable",
                version="v1.2.3",
                url="https://downloads.example.test/firmware.bin.ota",
            )

        manifest = ota_release.build_manifest(
            info,
            channel="dev",
            version="v1.2.3",
            url="https://downloads.example.test/firmware.bin.ota",
            allow_unsigned_dev=True,
        )
        manifest["channel"] = "stable"
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "unsigned manifest"):
            ota_release.verify_manifest(manifest, info, allow_unsigned_dev=True)

    def test_ota_header_version_must_match_manifest_version(self) -> None:
        info = ota_release.parse_ota_bytes(make_raw_ota())
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "software version mismatch"):
            ota_release.build_manifest(
                info,
                channel="dev",
                version="1.2.4",
                url="https://downloads.example.test/firmware.bin.ota",
                allow_unsigned_dev=True,
            )

    def test_signed_manifest_is_fail_closed_without_public_key(self) -> None:
        info = ota_release.parse_ota_bytes(make_raw_ota())
        manifest = ota_release.build_manifest(
            info,
            channel="dev",
            version="v1.2.3",
            url="https://downloads.example.test/firmware.bin.ota",
            allow_unsigned_dev=True,
        )
        manifest["signature"] = {
            "algorithm": "ECDSA-P256-SHA256",
            "key_id": "release-test",
            "scope": ota_release.SIGNATURE_SCOPE,
            "value": base64.b64encode((bytes(31) + b"\x01") * 2).decode("ascii"),
        }
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "requires --public-key"):
            ota_release.verify_manifest(manifest, info)

    def test_signature_authorization_payload_is_stable(self) -> None:
        info = ota_release.OtaImageInfo(
            path=Path("vector.bin.ota"),
            size=0x010203040506,
            sha256="11" * 32,
            body_size=0x01020304,
            body_sha256="00" * 31 + "ff",
            hardware_version="BFL_Module_v1.1",
            software_version="EVENT_V1.2.3",
        )
        manifest = {
            "schema": 1,
            "channel": "stable",
            "board": "aim61",
            "usb_speed": "fs",
            "profile": "diagnostic",
            "version": "1.2.3",
            "size": info.size,
            "sha256": info.sha256,
            "body_size": info.body_size,
            "body_sha256": info.body_sha256,
            "url": "https://mirror.example.test/image.bin.ota",
            "signature": None,
        }
        expected_hex = (
            "445335444f4e474c452d4f54412d5632"
            "010001010203"
            "04030201"
            + "00" * 31
            + "ff"
        )
        canonical = ota_release.signature_payload(manifest, info)
        self.assertEqual(len(canonical), 58)
        self.assertEqual(canonical.hex(), expected_hex)

    def test_ecdsa_der_raw_conversion_uses_fixed_width_low_s(self) -> None:
        raw = (bytes(31) + b"\x01") + (bytes(31) + b"\x02")
        self.assertEqual(ota_release.ecdsa_der_to_raw(ota_release.ecdsa_raw_to_der(raw)), raw)
        high_s = (bytes(31) + b"\x01") + (
            ota_release.P256_ORDER - 1
        ).to_bytes(32, "big")
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "high-S"):
            ota_release.ecdsa_raw_to_der(high_s)

    def test_strict_p256_spki_parser_exports_sec1_point(self) -> None:
        point = bytes.fromhex(
            "04"
            "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296"
            "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"
        )
        algorithm = bytes.fromhex("301306072a8648ce3d020106082a8648ce3d030107")
        spki = b"\x30\x59" + algorithm + b"\x03\x42\x00" + point
        self.assertEqual(ota_release.parse_p256_spki(spki), point)
        wrong_curve = bytearray(spki)
        wrong_curve[22] = 0x08
        with self.assertRaisesRegex(ota_release.OtaReleaseError, "prime256v1"):
            ota_release.parse_p256_spki(bytes(wrong_curve))

    def test_wrong_target_version_and_body_metadata_fail_closed(self) -> None:
        info = ota_release.parse_ota_bytes(make_raw_ota())
        original = ota_release.build_manifest(
            info,
            channel="dev",
            version="v1.2.3",
            url="https://downloads.example.test/firmware.bin.ota",
            allow_unsigned_dev=True,
        )
        mutations = {
            "board": "lctech616",
            "usb_speed": "ss",
            "version": "1.2.255",
            "body_size": info.body_size + 1,
            "body_sha256": "00" * 32,
        }
        for field, value in mutations.items():
            with self.subTest(field=field):
                manifest = dict(original)
                manifest[field] = value
                with self.assertRaises(ota_release.OtaReleaseError):
                    ota_release.verify_manifest(
                        manifest, info, allow_unsigned_dev=True
                    )

    @unittest.skipUnless(OPENSSL, "OpenSSL is unavailable")
    def test_stable_p256_manifest_is_reproducible_and_verifies(self) -> None:
        openssl = OPENSSL
        assert openssl is not None
        info = ota_release.parse_ota_bytes(make_raw_ota())
        with tempfile.TemporaryDirectory() as temporary:
            private_key = Path(temporary) / "private.pem"
            public_key = Path(temporary) / "public.pem"
            subprocess.run(
                [
                    openssl,
                    "genpkey",
                    "-algorithm",
                    "EC",
                    "-pkeyopt",
                    "ec_paramgen_curve:P-256",
                    "-out",
                    str(private_key),
                ],
                check=True,
                capture_output=True,
            )
            subprocess.run(
                [openssl, "pkey", "-in", str(private_key), "-pubout", "-out", str(public_key)],
                check=True,
                capture_output=True,
            )
            raw_public_key = ota_release.export_p256_public_key(public_key, openssl)
            self.assertEqual(len(raw_public_key), 65)
            self.assertEqual(raw_public_key[0], 0x04)
            arguments = {
                "channel": "stable",
                "version": "v1.2.3",
                "url": "https://downloads.example.test/firmware.bin.ota",
                "private_key": private_key,
                "key_id": "test-p256",
                "openssl": openssl,
            }
            first = ota_release.build_manifest(info, **arguments)
            second = ota_release.build_manifest(info, **arguments)
            self.assertEqual(ota_release.manifest_bytes(first), ota_release.manifest_bytes(second))
            ota_release.verify_manifest(
                first,
                info,
                public_key=public_key,
                expected_key_id="test-p256",
                openssl=openssl,
            )
            corrupted = json.loads(ota_release.manifest_bytes(first))
            raw_signature = bytearray(base64.b64decode(corrupted["signature"]["value"]))
            raw_signature[31] ^= 1
            corrupted["signature"]["value"] = base64.b64encode(raw_signature).decode("ascii")
            with self.assertRaisesRegex(ota_release.OtaReleaseError, "verification failed"):
                ota_release.verify_manifest(
                    corrupted,
                    info,
                    public_key=public_key,
                    expected_key_id="test-p256",
                    openssl=openssl,
                )
            wrong_speed = json.loads(ota_release.manifest_bytes(first))
            wrong_speed["usb_speed"] = "hs"
            with self.assertRaisesRegex(ota_release.OtaReleaseError, "verification failed"):
                ota_release.verify_manifest(
                    wrong_speed,
                    info,
                    public_key=public_key,
                    expected_key_id="test-p256",
                    openssl=openssl,
                )

    def test_manifest_loader_rejects_unknown_fields(self) -> None:
        info = ota_release.parse_ota_bytes(make_raw_ota())
        manifest = ota_release.build_manifest(
            info,
            channel="dev",
            version="v1.2.3",
            url="https://downloads.example.test/firmware.bin.ota",
            allow_unsigned_dev=True,
        )
        manifest["boot2"] = "forbidden.bin"
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "ota.json"
            path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(ota_release.OtaReleaseError, "fields differ"):
                ota_release._load_manifest(path)


if __name__ == "__main__":
    unittest.main()
