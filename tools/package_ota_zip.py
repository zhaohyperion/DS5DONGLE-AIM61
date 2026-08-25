#!/usr/bin/env python3
"""Create a deterministic signed-OTA-only DS5Dongle ZIP."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from urllib.parse import urlparse
import zipfile


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()

    image = args.image.read_bytes()
    manifest_bytes = args.manifest.read_bytes()
    try:
        manifest = json.loads(manifest_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid OTA manifest JSON: {error}") from error

    board = manifest.get("board")
    usb_speed = manifest.get("usb_speed")
    profile = manifest.get("profile")
    version = str(manifest.get("version", "")).removeprefix("v")
    if board != "aim61" or usb_speed != "hs":
        raise SystemExit("OTA ZIP target must be AIM61 High-Speed")
    if profile not in {"standard", "diagnostic"}:
        raise SystemExit("OTA ZIP profile must be standard or diagnostic")
    if not version or any(part == "" for part in version.split(".")):
        raise SystemExit("OTA manifest version is invalid")
    if manifest.get("sha256") != digest(image):
        raise SystemExit("OTA manifest hash does not match the supplied image")
    if not isinstance(manifest.get("signature"), dict):
        raise SystemExit("OTA ZIP requires a signed manifest")

    expected_name = (
        f"DS5Dongle-{board}-{usb_speed}-{profile}-ota-v{version}.zip"
    )
    if args.archive.name != expected_name:
        raise SystemExit(
            f"OTA archive must be named {expected_name}, got {args.archive.name}"
        )
    url_name = Path(urlparse(str(manifest.get("url", ""))).path).name
    if url_name != expected_name:
        raise SystemExit(
            f"OTA manifest URL must end with {expected_name}, got {url_name or 'none'}"
        )
    if not args.image.name.endswith(".bin.ota"):
        raise SystemExit("OTA image filename must end with .bin.ota")
    if not args.manifest.name.endswith(".ota.json"):
        raise SystemExit("OTA manifest filename must end with .ota.json")

    payloads = {
        args.image.name: image,
        args.manifest.name: manifest_bytes,
    }
    payloads["SHA256SUMS.txt"] = "".join(
        f"{digest(data)}  {name}\n" for name, data in sorted(payloads.items())
    ).encode("ascii")

    args.archive.parent.mkdir(parents=True, exist_ok=True)
    timestamp = (2026, 1, 1, 0, 0, 0)
    with zipfile.ZipFile(
        args.archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9
    ) as output:
        for name, data in sorted(payloads.items()):
            info = zipfile.ZipInfo(name, timestamp)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            output.writestr(info, data)
    print(f"{args.archive}  sha256={digest(args.archive.read_bytes())}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
