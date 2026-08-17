#!/usr/bin/env python3
"""Add the signed RAW OTA pair to an existing deterministic flash ZIP."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import zipfile


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--ota-image", type=Path, required=True)
    parser.add_argument("--ota-manifest", type=Path, required=True)
    args = parser.parse_args()

    with zipfile.ZipFile(args.archive, "r") as source:
        payloads = {
            info.filename: source.read(info)
            for info in source.infolist()
            if info.filename not in {"firmware.json", "SHA256SUMS.txt"}
            and not info.is_dir()
        }
        firmware_manifest = json.loads(source.read("firmware.json"))

    ota_image = args.ota_image.read_bytes()
    ota_manifest = args.ota_manifest.read_bytes()
    ota_metadata = json.loads(ota_manifest)
    if ota_metadata.get("profile") != firmware_manifest.get("profile"):
        raise SystemExit("OTA and flash package profiles differ")
    if ota_metadata.get("usb_speed") != firmware_manifest.get("usb_speed"):
        raise SystemExit("OTA and flash package USB speeds differ")
    if ota_metadata.get("sha256") != digest(ota_image):
        raise SystemExit("OTA manifest hash does not match the supplied image")

    payloads[args.ota_image.name] = ota_image
    payloads[args.ota_manifest.name] = ota_manifest
    firmware_manifest["ota_image"] = args.ota_image.name
    firmware_manifest["ota_manifest"] = args.ota_manifest.name
    payloads["firmware.json"] = (
        json.dumps(firmware_manifest, ensure_ascii=False, indent=2) + "\n"
    ).encode("utf-8")
    payloads["SHA256SUMS.txt"] = "".join(
        f"{digest(data)}  {name}\n" for name, data in sorted(payloads.items())
    ).encode("ascii")

    timestamp = (2026, 1, 1, 0, 0, 0)
    with zipfile.ZipFile(args.archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as output:
        for name, data in sorted(payloads.items()):
            info = zipfile.ZipInfo(name, timestamp)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            output.writestr(info, data)
    print(f"{args.archive}  sha256={digest(args.archive.read_bytes())}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
