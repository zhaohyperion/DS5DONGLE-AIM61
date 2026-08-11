#!/usr/bin/env python3
"""Create a deterministic, self-describing DS5Dongle flash ZIP."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import zipfile

BOARDS = {"aim61": 4, "lctech616": 4, "m0sdock": 4}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--board", choices=BOARDS, required=True)
    parser.add_argument("--usb-speed", choices=("fs", "hs"), required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--firmware-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, default=Path("dist"))
    args = parser.parse_args()

    source = args.firmware_dir.resolve()
    boot2 = sorted(source.glob("boot2_bl616_*.bin"))
    if len(boot2) != 1:
        raise SystemExit(f"expected one boot2_bl616_*.bin in {source}, found {len(boot2)}")
    partition = source / "partition.bin"
    suffix = "-hs" if args.usb_speed == "hs" else ""
    firmware = source / f"ds5dongle-{args.board}{suffix}.bin"
    for path in (partition, firmware):
        if not path.is_file():
            raise SystemExit(f"missing required file: {path}")
    if partition.stat().st_size != 308:
        raise SystemExit(f"partition.bin must be 308 bytes, got {partition.stat().st_size}")
    if not 64 * 1024 <= firmware.stat().st_size <= 8 * 1024 * 1024:
        raise SystemExit(f"application firmware size is unsafe: {firmware.stat().st_size}")

    manifest = {
        "schema": 1, "project": "DS5Dongle", "version": args.version,
        "board": args.board, "usb_speed": args.usb_speed, "chip": "bl616",
        "flash_size": BOARDS[args.board], "boot2": boot2[0].name,
        "partition": partition.name, "firmware": firmware.name,
    }
    payloads = {
        boot2[0].name: boot2[0].read_bytes(),
        partition.name: partition.read_bytes(),
        firmware.name: firmware.read_bytes(),
        "firmware.json": (json.dumps(manifest, ensure_ascii=False, indent=2) + "\n").encode(),
    }
    checksums = "".join(f"{digest(data)}  {name}\n" for name, data in sorted(payloads.items()))
    payloads["SHA256SUMS.txt"] = checksums.encode()

    args.output_dir.mkdir(parents=True, exist_ok=True)
    version = args.version.removeprefix("v")
    output = args.output_dir / f"DS5Dongle-{args.board}-{args.usb_speed}-v{version}.zip"
    timestamp = (2026, 1, 1, 0, 0, 0)
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name, data in sorted(payloads.items()):
            info = zipfile.ZipInfo(name, timestamp)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, data)
    print(f"{output}  sha256={digest(output.read_bytes())}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
