#!/usr/bin/env python3
"""Build and verify DS5Dongle M61 FS/HS OTA release manifests.

The Bouffalo SDK emits a 512-byte ``BL60X_OTA`` header followed by the
application body.  This tool deliberately accepts RAW application images only;
boot2, partition tables and full-flash packages are never valid OTA inputs.
"""
from __future__ import annotations

import argparse
import base64
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
from typing import Any
from urllib.parse import urlparse


OTA_HEADER_SIZE = 512
OTA_MAGIC = b"BL60X_OTA_Ver1.0"
# Bouffalo's image builder uses a space-padded four-byte type field.
OTA_RAW_TYPE = b"RAW "
M61_BACKUP_SLOT_SIZE = 0x168000
MANIFEST_SCHEMA = 1
MANIFEST_BOARD = "aim61"
MANIFEST_BOARD_ID = 1
MANIFEST_USB_SPEED_IDS = {"fs": 0, "hs": 1}
SIGNATURE_ALGORITHM = "ECDSA-P256-SHA256"
SIGNATURE_SCOPE = "DS5DONGLE-OTA-V1"
SIGNATURE_CANONICAL_MAGIC = SIGNATURE_SCOPE.encode("ascii")
P256_ORDER = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
CHANNELS = ("dev", "beta", "stable")
VERSION_RE = re.compile(r"v?([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\Z")
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")


class OtaReleaseError(ValueError):
    """An OTA image or release manifest failed validation."""


@dataclass(frozen=True)
class OtaImageInfo:
    path: Path
    size: int
    sha256: str
    body_size: int
    body_sha256: str
    hardware_version: str
    software_version: str


def _decode_header_text(raw: bytes, field: str) -> str:
    value, separator, padding = raw.partition(b"\x00")
    if separator and any(padding):
        raise OtaReleaseError(f"OTA {field} contains non-zero bytes after NUL")
    try:
        text = value.decode("ascii")
    except UnicodeDecodeError as exc:
        raise OtaReleaseError(f"OTA {field} is not ASCII") from exc
    if any(ord(char) < 0x20 or ord(char) > 0x7E for char in text):
        raise OtaReleaseError(f"OTA {field} contains non-printable characters")
    return text


def parse_ota_bytes(data: bytes, path: Path = Path("<memory>.bin.ota")) -> OtaImageInfo:
    """Validate a Bouffalo 512-byte RAW OTA container."""
    if len(data) < OTA_HEADER_SIZE:
        raise OtaReleaseError(
            f"OTA image is shorter than the {OTA_HEADER_SIZE}-byte header"
        )
    if data[:16] != OTA_MAGIC:
        raise OtaReleaseError("OTA magic must be exactly BL60X_OTA_Ver1.0")
    if data[16:20] != OTA_RAW_TYPE:
        observed = _decode_header_text(data[16:20], "type")
        raise OtaReleaseError(f"OTA type must be RAW, got {observed!r}")

    (declared_body_size,) = struct.unpack_from("<I", data, 20)
    body = data[OTA_HEADER_SIZE:]
    if declared_body_size != len(body):
        raise OtaReleaseError(
            "OTA body length mismatch: "
            f"header={declared_body_size}, actual={len(body)}"
        )
    if declared_body_size == 0:
        raise OtaReleaseError("OTA body is empty")
    if declared_body_size > M61_BACKUP_SLOT_SIZE:
        raise OtaReleaseError(
            f"OTA body ({declared_body_size} bytes) exceeds the M61 backup FW "
            f"slot ({M61_BACKUP_SLOT_SIZE} bytes)"
        )

    expected_body_sha = data[64:96]
    actual_body_sha = hashlib.sha256(body).digest()
    if expected_body_sha != actual_body_sha:
        raise OtaReleaseError(
            "OTA body SHA-256 mismatch: "
            f"header={expected_body_sha.hex()}, actual={actual_body_sha.hex()}"
        )

    hardware_version = _decode_header_text(data[32:48], "hardware version")
    software_version = _decode_header_text(data[48:64], "software version")
    return OtaImageInfo(
        path=path,
        size=len(data),
        sha256=hashlib.sha256(data).hexdigest(),
        body_size=declared_body_size,
        body_sha256=actual_body_sha.hex(),
        hardware_version=hardware_version,
        software_version=software_version,
    )


def inspect_ota(path: Path) -> OtaImageInfo:
    if not path.is_file():
        raise OtaReleaseError(f"missing OTA image: {path}")
    if not path.name.endswith(".bin.ota"):
        raise OtaReleaseError("OTA release input must end in .bin.ota (RAW only)")
    return parse_ota_bytes(path.read_bytes(), path.resolve())


def _validate_version(version: Any) -> str:
    match = VERSION_RE.fullmatch(version) if isinstance(version, str) else None
    if not match or any(int(component) >= 255 for component in match.groups()):
        raise OtaReleaseError("version must be v?MAJOR.MINOR.PATCH with each part 0..254")
    normalized = ".".join(str(int(component)) for component in match.groups())
    # The SDK stores ``EVENT_V${version}\0`` in ver_software[16].
    if len(normalized) > 8:
        raise OtaReleaseError(
            "normalized version must be at most 8 ASCII characters so "
            "EVENT_V<version> fits the 16-byte OTA header field with a NUL"
        )
    return normalized


def _version_triplet(version: str) -> tuple[int, int, int]:
    normalized = _validate_version(version)
    match = VERSION_RE.fullmatch(normalized)
    assert match is not None
    return tuple(int(component) for component in match.groups())  # type: ignore[return-value]


def _validate_url(url: Any) -> str:
    if not isinstance(url, str):
        raise OtaReleaseError("url must be a string")
    parsed = urlparse(url)
    if parsed.scheme != "https" or not parsed.netloc or parsed.username or parsed.password:
        raise OtaReleaseError("url must be an HTTPS URL without embedded credentials")
    if parsed.fragment:
        raise OtaReleaseError("url must not contain a fragment")
    return url


def _validate_usb_speed(usb_speed: Any) -> str:
    if not isinstance(usb_speed, str) or usb_speed not in MANIFEST_USB_SPEED_IDS:
        raise OtaReleaseError("OTA manifest usb_speed must be fs or hs")
    return usb_speed


def signature_payload(manifest: dict[str, Any], info: OtaImageInfo) -> bytes:
    """Return the exact 57-byte P-256 authorization canonical."""
    if type(manifest.get("schema")) is not int or manifest.get("schema") != MANIFEST_SCHEMA:
        raise OtaReleaseError("unsupported manifest schema")
    channel = manifest.get("channel")
    if not isinstance(channel, str) or channel not in CHANNELS:
        raise OtaReleaseError(f"unsupported channel: {channel!r}")
    if manifest.get("board") != MANIFEST_BOARD:
        raise OtaReleaseError("OTA manifest board must be aim61")
    usb_speed = _validate_usb_speed(manifest.get("usb_speed"))
    raw_version = manifest.get("version")
    version = _validate_version(raw_version)
    if raw_version != version:
        raise OtaReleaseError("manifest version must be normalized without v or leading zeros")
    expected_software_version = f"EVENT_V{version}"
    if info.software_version != expected_software_version:
        raise OtaReleaseError(
            "OTA header software version mismatch: "
            f"header={info.software_version!r}, manifest={version!r}"
        )
    size = manifest.get("size")
    if not isinstance(size, int) or isinstance(size, bool) or size <= OTA_HEADER_SIZE:
        raise OtaReleaseError("manifest size must be an integer larger than 512")
    sha256 = manifest.get("sha256")
    if not isinstance(sha256, str) or not SHA256_RE.fullmatch(sha256):
        raise OtaReleaseError("manifest sha256 must be 64 lowercase hex characters")
    if size != info.size or sha256 != info.sha256:
        raise OtaReleaseError("manifest does not describe the supplied OTA image")
    body_size = manifest.get("body_size")
    if type(body_size) is not int or body_size != info.body_size:
        raise OtaReleaseError("manifest body_size does not match the supplied OTA image")
    body_sha256 = manifest.get("body_sha256")
    if (
        not isinstance(body_sha256, str)
        or not SHA256_RE.fullmatch(body_sha256)
        or body_sha256 != info.body_sha256
    ):
        raise OtaReleaseError("manifest body_sha256 does not match the supplied OTA image")

    canonical = b"".join(
        (
            SIGNATURE_CANONICAL_MAGIC,
            bytes((MANIFEST_BOARD_ID, MANIFEST_USB_SPEED_IDS[usb_speed])),
            bytes(_version_triplet(version)),
            struct.pack("<I", info.body_size),
            bytes.fromhex(info.body_sha256),
        )
    )
    if len(canonical) != 57:
        raise AssertionError(f"P-256 OTA canonical must be 57 bytes, got {len(canonical)}")
    return canonical


def _openssl_binary(explicit: str | None) -> str:
    executable = explicit or shutil.which("openssl")
    if not executable:
        raise OtaReleaseError(
            "OpenSSL is required for ECDSA P-256 signing/verification; install "
            "OpenSSL or pass --openssl with its executable path"
        )
    return executable


def _run_openssl(arguments: list[str], error_message: str) -> None:
    result = subprocess.run(arguments, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        if len(detail) > 500:
            detail = detail[:500] + "..."
        raise OtaReleaseError(f"{error_message}: {detail or 'OpenSSL failed'}")


def _der_read_length(data: bytes, offset: int) -> tuple[int, int]:
    if offset >= len(data):
        raise OtaReleaseError("truncated ECDSA DER signature")
    first = data[offset]
    offset += 1
    if first < 0x80:
        return first, offset
    count = first & 0x7F
    if count == 0 or count > 2 or offset + count > len(data):
        raise OtaReleaseError("invalid ECDSA DER length")
    length = int.from_bytes(data[offset : offset + count], "big")
    if length < 0x80:
        raise OtaReleaseError("non-minimal ECDSA DER length")
    return length, offset + count


def _der_decode_integer(data: bytes, offset: int) -> tuple[int, int]:
    if offset >= len(data) or data[offset] != 0x02:
        raise OtaReleaseError("ECDSA DER signature is missing an INTEGER")
    length, start = _der_read_length(data, offset + 1)
    end = start + length
    if length == 0 or end > len(data):
        raise OtaReleaseError("truncated ECDSA DER INTEGER")
    encoded = data[start:end]
    if encoded[0] & 0x80:
        raise OtaReleaseError("ECDSA DER INTEGER is negative")
    if len(encoded) > 1 and encoded[0] == 0 and not (encoded[1] & 0x80):
        raise OtaReleaseError("ECDSA DER INTEGER is not minimally encoded")
    return int.from_bytes(encoded, "big"), end


def _der_read_tlv(data: bytes, offset: int, expected_tag: int) -> tuple[bytes, int]:
    if offset >= len(data) or data[offset] != expected_tag:
        raise OtaReleaseError(f"SPKI expected DER tag 0x{expected_tag:02X}")
    length, start = _der_read_length(data, offset + 1)
    end = start + length
    if end > len(data):
        raise OtaReleaseError("truncated P-256 SubjectPublicKeyInfo")
    return data[start:end], end


def parse_p256_spki(der: bytes) -> bytes:
    """Strictly extract a 65-byte uncompressed SEC1 point from P-256 SPKI."""
    outer, end = _der_read_tlv(der, 0, 0x30)
    if end != len(der):
        raise OtaReleaseError("P-256 SubjectPublicKeyInfo has trailing bytes")
    algorithm, offset = _der_read_tlv(outer, 0, 0x30)
    bit_string, offset = _der_read_tlv(outer, offset, 0x03)
    if offset != len(outer):
        raise OtaReleaseError("P-256 SubjectPublicKeyInfo has extra fields")

    ec_public_key_oid = bytes.fromhex("2a8648ce3d0201")
    prime256v1_oid = bytes.fromhex("2a8648ce3d030107")
    oid, algorithm_offset = _der_read_tlv(algorithm, 0, 0x06)
    curve_oid, algorithm_offset = _der_read_tlv(algorithm, algorithm_offset, 0x06)
    if algorithm_offset != len(algorithm):
        raise OtaReleaseError("P-256 AlgorithmIdentifier has extra fields")
    if oid != ec_public_key_oid or curve_oid != prime256v1_oid:
        raise OtaReleaseError("public key SPKI is not id-ecPublicKey/prime256v1")
    if len(bit_string) != 66 or bit_string[0] != 0:
        raise OtaReleaseError("P-256 public key BIT STRING must contain 0 unused bits and 65 bytes")
    point = bit_string[1:]
    if point[0] != 0x04:
        raise OtaReleaseError("P-256 public key must use uncompressed SEC1 form")
    return point


def ecdsa_der_to_raw(signature: bytes) -> bytes:
    if not signature or signature[0] != 0x30:
        raise OtaReleaseError("ECDSA signature is not a DER SEQUENCE")
    sequence_length, offset = _der_read_length(signature, 1)
    if offset + sequence_length != len(signature):
        raise OtaReleaseError("ECDSA DER SEQUENCE length mismatch")
    r, offset = _der_decode_integer(signature, offset)
    s, offset = _der_decode_integer(signature, offset)
    if offset != len(signature):
        raise OtaReleaseError("ECDSA DER signature has trailing bytes")
    if not 1 <= r < P256_ORDER or not 1 <= s < P256_ORDER:
        raise OtaReleaseError("ECDSA P-256 r/s is outside the curve order")
    # Canonical low-S form prevents a second encoding of the same signature.
    s = min(s, P256_ORDER - s)
    return r.to_bytes(32, "big") + s.to_bytes(32, "big")


def _der_length(length: int) -> bytes:
    if length < 0x80:
        return bytes((length,))
    encoded = length.to_bytes((length.bit_length() + 7) // 8, "big")
    return bytes((0x80 | len(encoded),)) + encoded


def _der_integer(value: int) -> bytes:
    encoded = value.to_bytes(max(1, (value.bit_length() + 7) // 8), "big")
    if encoded[0] & 0x80:
        encoded = b"\x00" + encoded
    return b"\x02" + _der_length(len(encoded)) + encoded


def ecdsa_raw_to_der(signature: bytes) -> bytes:
    if len(signature) != 64:
        raise OtaReleaseError(f"ECDSA P-256 signature must be 64 bytes, got {len(signature)}")
    r = int.from_bytes(signature[:32], "big")
    s = int.from_bytes(signature[32:], "big")
    if not 1 <= r < P256_ORDER or not 1 <= s < P256_ORDER:
        raise OtaReleaseError("ECDSA P-256 r/s is outside the curve order")
    if s > P256_ORDER // 2:
        raise OtaReleaseError("ECDSA P-256 signature is non-canonical high-S")
    body = _der_integer(r) + _der_integer(s)
    return b"\x30" + _der_length(len(body)) + body


def _validate_p256_public_key(executable: str, public_key: Path) -> None:
    result = subprocess.run(
        [executable, "pkey", "-pubin", "-in", str(public_key), "-text_pub", "-noout"],
        capture_output=True,
        text=True,
        check=False,
    )
    description = (result.stdout + "\n" + result.stderr)
    if result.returncode != 0:
        raise OtaReleaseError("cannot parse ECDSA public key")
    if "prime256v1" not in description and "P-256" not in description:
        raise OtaReleaseError("OTA signing key must use ECDSA P-256 (prime256v1)")


def export_p256_public_key(public_key: Path, openssl: str | None = None) -> bytes:
    """Validate a PEM public key and return its 65-byte SEC1 representation."""
    if not public_key.is_file():
        raise OtaReleaseError(f"missing ECDSA P-256 public key: {public_key}")
    executable = _openssl_binary(openssl)
    _validate_p256_public_key(executable, public_key)
    with tempfile.TemporaryDirectory(prefix="ds5-ota-public-") as temp_dir:
        der_path = Path(temp_dir) / "public.der"
        _run_openssl(
            [
                executable,
                "pkey",
                "-pubin",
                "-in",
                str(public_key),
                "-outform",
                "DER",
                "-out",
                str(der_path),
            ],
            "cannot export ECDSA P-256 public key",
        )
        return parse_p256_spki(der_path.read_bytes())


def ecdsa_p256_sign(payload: bytes, private_key: Path, openssl: str | None = None) -> bytes:
    if not private_key.is_file():
        raise OtaReleaseError(f"missing ECDSA P-256 private key: {private_key}")
    executable = _openssl_binary(openssl)
    with tempfile.TemporaryDirectory(prefix="ds5-ota-sign-") as temp_dir:
        payload_path = Path(temp_dir) / "payload.bin"
        signature_path = Path(temp_dir) / "signature.der"
        public_key_path = Path(temp_dir) / "public.pem"
        payload_path.write_bytes(payload)
        _run_openssl(
            [
                executable,
                "pkey",
                "-in",
                str(private_key),
                "-pubout",
                "-out",
                str(public_key_path),
            ],
            "cannot derive ECDSA public key",
        )
        _validate_p256_public_key(executable, public_key_path)
        _run_openssl(
            [
                executable,
                "dgst",
                "-sha256",
                "-sign",
                str(private_key),
                "-sigopt",
                "nonce-type:1",
                "-out",
                str(signature_path),
                str(payload_path),
            ],
            "ECDSA P-256 signing failed",
        )
        return ecdsa_der_to_raw(signature_path.read_bytes())


def ecdsa_p256_verify(
    payload: bytes,
    signature: bytes,
    public_key: Path,
    openssl: str | None = None,
) -> None:
    der_signature = ecdsa_raw_to_der(signature)
    if not public_key.is_file():
        raise OtaReleaseError(f"missing ECDSA P-256 public key: {public_key}")
    executable = _openssl_binary(openssl)
    _validate_p256_public_key(executable, public_key)
    with tempfile.TemporaryDirectory(prefix="ds5-ota-verify-") as temp_dir:
        payload_path = Path(temp_dir) / "payload.bin"
        signature_path = Path(temp_dir) / "signature.der"
        payload_path.write_bytes(payload)
        signature_path.write_bytes(der_signature)
        _run_openssl(
            [
                executable,
                "dgst",
                "-sha256",
                "-verify",
                str(public_key),
                "-signature",
                str(signature_path),
                str(payload_path),
            ],
            "ECDSA P-256 signature verification failed",
        )


def build_manifest(
    info: OtaImageInfo,
    *,
    channel: str,
    version: str,
    url: str,
    usb_speed: str = "fs",
    private_key: Path | None = None,
    key_id: str | None = None,
    openssl: str | None = None,
    allow_unsigned_dev: bool = False,
) -> dict[str, Any]:
    if channel not in CHANNELS:
        raise OtaReleaseError(f"unsupported channel: {channel!r}")
    version = _validate_version(version)
    if info.software_version != f"EVENT_V{version}":
        raise OtaReleaseError(
            "OTA header software version mismatch: "
            f"header={info.software_version!r}, requested={version!r}"
        )
    url = _validate_url(url)
    usb_speed = _validate_usb_speed(usb_speed)
    manifest: dict[str, Any] = {
        "schema": MANIFEST_SCHEMA,
        "channel": channel,
        "board": MANIFEST_BOARD,
        "usb_speed": usb_speed,
        "version": version,
        "size": info.size,
        "sha256": info.sha256,
        "body_size": info.body_size,
        "body_sha256": info.body_sha256,
        "url": url,
        "signature": None,
    }

    if private_key is None:
        if channel != "dev" or not allow_unsigned_dev:
            raise OtaReleaseError(
                "unsigned manifests are allowed only for channel=dev with "
                "--allow-unsigned-dev"
            )
        if key_id is not None:
            raise OtaReleaseError("--key-id requires --private-key")
        return manifest

    if not key_id or not re.fullmatch(r"[0-9A-Za-z._-]{1,64}", key_id):
        raise OtaReleaseError(
            "signed manifests require a 1-64 character --key-id"
        )
    signature = ecdsa_p256_sign(signature_payload(manifest, info), private_key, openssl)
    manifest["signature"] = {
        "algorithm": SIGNATURE_ALGORITHM,
        "key_id": key_id,
        "scope": SIGNATURE_SCOPE,
        "value": base64.b64encode(signature).decode("ascii"),
    }
    return manifest


def manifest_bytes(manifest: dict[str, Any]) -> bytes:
    """Serialize a manifest reproducibly."""
    return (json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )


def _load_manifest(path: Path) -> dict[str, Any]:
    try:
        parsed = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise OtaReleaseError(f"cannot read manifest {path}: {exc}") from exc
    if not isinstance(parsed, dict):
        raise OtaReleaseError("manifest root must be a JSON object")
    expected = {
        "schema",
        "channel",
        "board",
        "usb_speed",
        "version",
        "size",
        "sha256",
        "body_size",
        "body_sha256",
        "url",
        "signature",
    }
    if set(parsed) != expected:
        missing = sorted(expected - set(parsed))
        extra = sorted(set(parsed) - expected)
        raise OtaReleaseError(f"manifest fields differ: missing={missing}, extra={extra}")
    return parsed


def verify_manifest(
    manifest: dict[str, Any],
    info: OtaImageInfo,
    *,
    public_key: Path | None = None,
    expected_key_id: str | None = None,
    openssl: str | None = None,
    allow_unsigned_dev: bool = False,
) -> None:
    # This validates all signed fields even for an explicitly unsigned dev manifest.
    _validate_url(manifest.get("url"))
    if manifest["size"] != info.size:
        raise OtaReleaseError(
            f"manifest size mismatch: manifest={manifest['size']}, image={info.size}"
        )
    if manifest["sha256"] != info.sha256:
        raise OtaReleaseError(
            f"manifest SHA-256 mismatch: manifest={manifest['sha256']}, image={info.sha256}"
        )
    if manifest["body_size"] != info.body_size:
        raise OtaReleaseError(
            f"manifest body_size mismatch: manifest={manifest['body_size']}, "
            f"image={info.body_size}"
        )
    if manifest["body_sha256"] != info.body_sha256:
        raise OtaReleaseError("manifest body_sha256 does not match the RAW OTA body")
    payload = signature_payload(manifest, info)

    signature = manifest.get("signature")
    if signature is None:
        if manifest["channel"] != "dev" or not allow_unsigned_dev:
            raise OtaReleaseError(
                "unsigned manifest requires channel=dev and --allow-unsigned-dev"
            )
        if public_key is not None or expected_key_id is not None:
            raise OtaReleaseError("manifest is unsigned but signature verification was requested")
        return
    if not isinstance(signature, dict) or set(signature) != {
        "algorithm",
        "key_id",
        "scope",
        "value",
    }:
        raise OtaReleaseError("signature must contain algorithm, key_id, scope, and value")
    if signature["algorithm"] != SIGNATURE_ALGORITHM:
        raise OtaReleaseError("signature algorithm must be ECDSA-P256-SHA256")
    if signature["scope"] != SIGNATURE_SCOPE:
        raise OtaReleaseError(f"signature scope must be {SIGNATURE_SCOPE}")
    if not isinstance(signature["key_id"], str) or not re.fullmatch(
        r"[0-9A-Za-z._-]{1,64}", signature["key_id"]
    ):
        raise OtaReleaseError("signature key_id is invalid")
    if expected_key_id is not None and signature["key_id"] != expected_key_id:
        raise OtaReleaseError(
            f"signature key_id mismatch: expected {expected_key_id!r}, "
            f"got {signature['key_id']!r}"
        )
    try:
        raw_signature = base64.b64decode(signature["value"], validate=True)
    except (TypeError, ValueError) as exc:
        raise OtaReleaseError("signature value is not valid base64") from exc
    if len(raw_signature) != 64:
        raise OtaReleaseError(
            f"ECDSA P-256 raw r||s signature must decode to 64 bytes, got {len(raw_signature)}"
        )
    if public_key is None:
        raise OtaReleaseError("signed manifest verification requires --public-key")
    ecdsa_p256_verify(payload, raw_signature, public_key, openssl)


def _print_info(info: OtaImageInfo) -> None:
    print(f"image={info.path}")
    print(f"size={info.size}")
    print(f"sha256={info.sha256}")
    print(f"body_size={info.body_size}")
    print(f"body_sha256={info.body_sha256}")
    print(f"hardware_version={info.hardware_version}")
    print(f"software_version={info.software_version}")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    inspect_parser = subparsers.add_parser("inspect", help="validate and inspect a RAW OTA")
    inspect_parser.add_argument("--image", type=Path, required=True)

    generate_parser = subparsers.add_parser("generate", help="generate a release manifest")
    generate_parser.add_argument("--image", type=Path, required=True)
    generate_parser.add_argument("--manifest", type=Path, required=True)
    generate_parser.add_argument("--channel", choices=CHANNELS, required=True)
    generate_parser.add_argument("--version", required=True)
    generate_parser.add_argument("--url", required=True)
    generate_parser.add_argument("--usb-speed", choices=tuple(MANIFEST_USB_SPEED_IDS), required=True)
    generate_parser.add_argument("--private-key", type=Path)
    generate_parser.add_argument("--key-id")
    generate_parser.add_argument("--openssl")
    generate_parser.add_argument(
        "--allow-unsigned-dev",
        action="store_true",
        help="explicitly allow signature=null for channel=dev only",
    )

    verify_parser = subparsers.add_parser("verify", help="verify an image and manifest")
    verify_parser.add_argument("--image", type=Path, required=True)
    verify_parser.add_argument("--manifest", type=Path, required=True)
    verify_parser.add_argument("--public-key", type=Path)
    verify_parser.add_argument("--expected-key-id")
    verify_parser.add_argument("--openssl")
    verify_parser.add_argument(
        "--allow-unsigned-dev",
        action="store_true",
        help="explicitly accept signature=null for channel=dev only",
    )

    export_parser = subparsers.add_parser(
        "export-public-key",
        help="export a validated P-256 PEM public key as SEC1 04||X||Y",
    )
    export_parser.add_argument("--public-key", type=Path, required=True)
    export_parser.add_argument("--output", type=Path, required=True)
    export_parser.add_argument("--openssl")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "export-public-key":
            point = export_p256_public_key(args.public_key, args.openssl)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_bytes(point)
            print(f"public_key={args.output.resolve()}")
            print(f"size={len(point)}")
            print(f"sha256={hashlib.sha256(point).hexdigest()}")
            return 0
        info = inspect_ota(args.image)
        if args.command == "inspect":
            _print_info(info)
            return 0
        if args.command == "generate":
            manifest = build_manifest(
                info,
                channel=args.channel,
                version=args.version,
                url=args.url,
                usb_speed=args.usb_speed,
                private_key=args.private_key,
                key_id=args.key_id,
                openssl=args.openssl,
                allow_unsigned_dev=args.allow_unsigned_dev,
            )
            args.manifest.parent.mkdir(parents=True, exist_ok=True)
            args.manifest.write_bytes(manifest_bytes(manifest))
            print(f"manifest={args.manifest.resolve()}")
            print(f"manifest_sha256={hashlib.sha256(manifest_bytes(manifest)).hexdigest()}")
            return 0
        if args.command == "verify":
            manifest = _load_manifest(args.manifest)
            verify_manifest(
                manifest,
                info,
                public_key=args.public_key,
                expected_key_id=args.expected_key_id,
                openssl=args.openssl,
                allow_unsigned_dev=args.allow_unsigned_dev,
            )
            _print_info(info)
            print(f"manifest={args.manifest.resolve()}")
            print("verified=true")
            return 0
    except OtaReleaseError as exc:
        raise SystemExit(f"error: {exc}") from exc
    raise AssertionError("unreachable")


if __name__ == "__main__":
    raise SystemExit(main())
