#!/usr/bin/env python3
"""Reference encoder/decoder for the DS5Dongle USB HID OTA transport."""
from __future__ import annotations

from dataclasses import dataclass
import struct
import zlib


OTA_DATA_REPORT_ID = 0xFA
OTA_CONTROL_REPORT_ID = 0xFC
OTA_PAYLOAD_SIZE = 63
OTA_CRC_OFFSET = 59
OTA_MAGIC = b"OT"
OTA_PROTOCOL_VERSION = 2
OTA_DATA_TYPE = 0x10
OTA_DATA_CAPACITY = 46

OTA_CONTROL_BEGIN = 0x01
OTA_CONTROL_AUTH = 0x02
OTA_CONTROL_COMMIT = 0x03
OTA_CONTROL_ABORT = 0x04
OTA_CONTROL_STATUS = 0x05
OTA_CONTROL_ACK = 0x80
OTA_CONTROL_ERROR = 0xFF

OTA_BOARD_AIM61 = 1
OTA_USB_FULL_SPEED = 0
OTA_USB_HIGH_SPEED = 1
OTA_PROFILE_STANDARD = 0
OTA_PROFILE_DIAGNOSTIC = 1
OTA_FORMAT_RAW = 1
OTA_BEGIN_FLAG_SIGNED = 1 << 0
OTA_BEGIN_DATA_SIZE = 43
OTA_AUTH_SIGNATURE_SIZE = 64
OTA_STATUS_DATA_SIZE = 44
OTA_CAP_LIVE_RESUME = 1 << 2  # Reserved; current firmware does not advertise it.
OTA_ERROR_TIMEOUT = 19

CONTROL_OPCODES = {
    OTA_CONTROL_BEGIN,
    OTA_CONTROL_AUTH,
    OTA_CONTROL_COMMIT,
    OTA_CONTROL_ABORT,
    OTA_CONTROL_STATUS,
    OTA_CONTROL_ACK,
    OTA_CONTROL_ERROR,
}


class OtaProtocolError(ValueError):
    """A host/device OTA frame is invalid."""


@dataclass(frozen=True)
class OtaFrame:
    report_id: int
    opcode: int
    session: int
    argument: int
    data: bytes


@dataclass(frozen=True)
class OtaStatus:
    opcode: int
    session: int
    accepted_offset: int
    state: int
    error: int
    last_request: int
    flags: int
    committed_offset: int
    total_size: int
    max_file_size: int
    capabilities: int
    board: int
    usb_speed: int
    image_format: int
    max_data: int
    window: int
    active_slot: int
    trial_retry: int
    reboot_delay_100ms: int
    version: str


def crc32_ieee(data: bytes) -> int:
    """CRC-32/ISO-HDLC (the standard zlib/Ethernet CRC-32)."""
    return zlib.crc32(data) & 0xFFFFFFFF


def _encode(report_id: int, opcode: int, session: int, argument: int, data: bytes) -> bytes:
    if report_id not in (OTA_DATA_REPORT_ID, OTA_CONTROL_REPORT_ID):
        raise OtaProtocolError(f"unsupported OTA report ID 0x{report_id:02X}")
    if not 0 <= opcode <= 0xFF:
        raise OtaProtocolError("opcode must fit in one byte")
    if not 0 <= session <= 0xFFFFFFFF:
        raise OtaProtocolError("session must fit in uint32")
    if not 0 <= argument <= 0xFFFFFFFF:
        raise OtaProtocolError("argument/offset must fit in uint32")
    if len(data) > OTA_DATA_CAPACITY:
        raise OtaProtocolError(f"frame data exceeds {OTA_DATA_CAPACITY} bytes")
    if report_id == OTA_DATA_REPORT_ID and opcode != OTA_DATA_TYPE:
        raise OtaProtocolError("report 0xFA accepts DATA type 0x10 only")
    if report_id == OTA_CONTROL_REPORT_ID and opcode not in CONTROL_OPCODES:
        raise OtaProtocolError(f"unsupported control opcode 0x{opcode:02X}")

    payload = bytearray(OTA_PAYLOAD_SIZE)
    payload[0:2] = OTA_MAGIC
    payload[2] = OTA_PROTOCOL_VERSION
    payload[3] = opcode
    struct.pack_into("<I", payload, 4, session)
    struct.pack_into("<I", payload, 8, argument)
    payload[12] = len(data)
    payload[13 : 13 + len(data)] = data
    struct.pack_into("<I", payload, OTA_CRC_OFFSET, crc32_ieee(payload[:OTA_CRC_OFFSET]))
    return bytes(payload)


def encode_data(session: int, offset: int, data: bytes) -> bytes:
    """Encode the 63-byte payload carried by Output Report 0xFA."""
    if not data:
        raise OtaProtocolError("DATA frame must carry at least one byte")
    return _encode(OTA_DATA_REPORT_ID, OTA_DATA_TYPE, session, offset, data)


def encode_control(opcode: int, session: int, argument: int = 0, data: bytes = b"") -> bytes:
    """Encode the 63-byte payload used by Feature Report 0xFC.

    Bytes 8..11 are opcode-specific ``argument`` (for example image size,
    signature offset or confirmed offset).  Byte 12 is the data length and
    bytes 13..58 contain up to 46 data bytes.
    """
    return _encode(OTA_CONTROL_REPORT_ID, opcode, session, argument, data)


def encode_begin(
    session: int,
    total_size: int,
    version: tuple[int, int, int],
    body_size: int,
    body_sha256: bytes,
    *,
    signed: bool = True,
    usb_speed: int = OTA_USB_HIGH_SPEED,
    profile: int = OTA_PROFILE_STANDARD,
) -> bytes:
    if len(version) != 3 or any(not 0 <= component < 255 for component in version):
        raise OtaProtocolError("semantic version must contain three values in 0..254")
    if len(".".join(str(component) for component in version)) > 8:
        raise OtaProtocolError(
            "semantic version must fit EVENT_V<version> plus NUL in the 16-byte OTA header"
        )
    if not 0 < body_size <= 0xFFFFFFFF:
        raise OtaProtocolError("body size must fit in a non-zero uint32")
    if total_size != body_size + 512:
        raise OtaProtocolError("complete OTA size must equal RAW body size + 512")
    if len(body_sha256) != 32:
        raise OtaProtocolError("RAW body SHA-256 must be exactly 32 bytes")
    if usb_speed not in (OTA_USB_FULL_SPEED, OTA_USB_HIGH_SPEED):
        raise OtaProtocolError("USB speed must be full-speed or high-speed")
    if profile not in (OTA_PROFILE_STANDARD, OTA_PROFILE_DIAGNOSTIC):
        raise OtaProtocolError("build profile must be standard or diagnostic")
    data = bytearray(OTA_BEGIN_DATA_SIZE)
    data[0] = OTA_BOARD_AIM61
    data[1] = usb_speed
    data[2:5] = bytes(version)
    data[5] = OTA_BEGIN_FLAG_SIGNED if signed else 0
    struct.pack_into("<I", data, 6, body_size)
    data[10:42] = body_sha256
    data[42] = profile
    return encode_control(OTA_CONTROL_BEGIN, session, total_size, bytes(data))


def encode_auth_frames(session: int, signature: bytes) -> tuple[bytes, bytes]:
    """Split a fixed-width P-256 r||s signature across two AUTH frames."""
    if len(signature) != OTA_AUTH_SIGNATURE_SIZE:
        raise OtaProtocolError("P-256 raw r||s signature must be exactly 64 bytes")
    first = encode_control(OTA_CONTROL_AUTH, session, 0, signature[:OTA_DATA_CAPACITY])
    second = encode_control(
        OTA_CONTROL_AUTH,
        session,
        OTA_DATA_CAPACITY,
        signature[OTA_DATA_CAPACITY:],
    )
    return first, second


def decode_status(payload: bytes) -> OtaStatus:
    frame = decode_payload(OTA_CONTROL_REPORT_ID, payload)
    if frame.opcode not in (OTA_CONTROL_ACK, OTA_CONTROL_ERROR):
        raise OtaProtocolError("status snapshot opcode must be ACK or ERROR")
    if len(frame.data) != OTA_STATUS_DATA_SIZE:
        raise OtaProtocolError(
            f"status snapshot must contain {OTA_STATUS_DATA_SIZE} data bytes"
        )
    data = frame.data
    raw_version = data[28:44]
    version_bytes, separator, padding = raw_version.partition(b"\x00")
    if separator and any(padding):
        raise OtaProtocolError("status version has non-zero bytes after NUL")
    try:
        version = version_bytes.decode("ascii")
    except UnicodeDecodeError as exc:
        raise OtaProtocolError("status version is not ASCII") from exc
    return OtaStatus(
        opcode=frame.opcode,
        session=frame.session,
        accepted_offset=frame.argument,
        state=data[0],
        error=data[1],
        last_request=data[2],
        flags=data[3],
        committed_offset=struct.unpack_from("<I", data, 4)[0],
        total_size=struct.unpack_from("<I", data, 8)[0],
        max_file_size=struct.unpack_from("<I", data, 12)[0],
        capabilities=struct.unpack_from("<I", data, 16)[0],
        board=data[20],
        usb_speed=data[21],
        image_format=data[22],
        max_data=data[23],
        window=data[24],
        active_slot=data[25],
        trial_retry=data[26],
        reboot_delay_100ms=data[27],
        version=version,
    )


def decode_payload(report_id: int, payload: bytes) -> OtaFrame:
    if len(payload) != OTA_PAYLOAD_SIZE:
        raise OtaProtocolError(
            f"OTA payload must be exactly {OTA_PAYLOAD_SIZE} bytes, got {len(payload)}"
        )
    if payload[:2] != OTA_MAGIC:
        raise OtaProtocolError("OTA frame magic must be 'OT'")
    if payload[2] != OTA_PROTOCOL_VERSION:
        raise OtaProtocolError(f"unsupported OTA protocol version {payload[2]}")
    expected_crc = struct.unpack_from("<I", payload, OTA_CRC_OFFSET)[0]
    actual_crc = crc32_ieee(payload[:OTA_CRC_OFFSET])
    if expected_crc != actual_crc:
        raise OtaProtocolError(
            f"OTA CRC mismatch: frame=0x{expected_crc:08X}, actual=0x{actual_crc:08X}"
        )

    opcode = payload[3]
    if report_id == OTA_DATA_REPORT_ID and opcode != OTA_DATA_TYPE:
        raise OtaProtocolError("report 0xFA must contain DATA type 0x10")
    if report_id == OTA_CONTROL_REPORT_ID and opcode not in CONTROL_OPCODES:
        raise OtaProtocolError(f"unsupported control opcode 0x{opcode:02X}")
    if report_id not in (OTA_DATA_REPORT_ID, OTA_CONTROL_REPORT_ID):
        raise OtaProtocolError(f"unsupported OTA report ID 0x{report_id:02X}")

    data_length = payload[12]
    if data_length > OTA_DATA_CAPACITY:
        raise OtaProtocolError(f"OTA data length exceeds {OTA_DATA_CAPACITY}")
    if report_id == OTA_DATA_REPORT_ID and data_length == 0:
        raise OtaProtocolError("DATA frame must carry at least one byte")
    if any(payload[13 + data_length : OTA_CRC_OFFSET]):
        raise OtaProtocolError("OTA frame has non-zero padding")
    return OtaFrame(
        report_id=report_id,
        opcode=opcode,
        session=struct.unpack_from("<I", payload, 4)[0],
        argument=struct.unpack_from("<I", payload, 8)[0],
        data=bytes(payload[13 : 13 + data_length]),
    )


def encode_wire_frame(report_id: int, payload: bytes) -> bytes:
    """Prefix a payload with its report ID for firmware-side 64-byte fixtures."""
    if len(payload) != OTA_PAYLOAD_SIZE:
        raise OtaProtocolError(f"payload must be {OTA_PAYLOAD_SIZE} bytes")
    if report_id not in (OTA_DATA_REPORT_ID, OTA_CONTROL_REPORT_ID):
        raise OtaProtocolError(f"unsupported OTA report ID 0x{report_id:02X}")
    return bytes((report_id,)) + payload
