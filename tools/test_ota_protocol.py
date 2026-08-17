from __future__ import annotations

import hashlib
from pathlib import Path
import re
import struct
import unittest

from tools import ota_protocol


BEGIN_VECTOR = (
    "4f54020178563412f02b0d002b"
    "010101020301f0290d00"
    "b0d51c58c8b9c1f458fadf16c7d375630ef51da4df81915893b05c0fa4ed8bc6"
    "000000007c9724c8"
)
DATA_VECTOR = (
    "4f540210785634122e00000019"
    "445335446f6e676c65204f5441207465737420766563746f72"
    "000000000000000000000000000000000000000000"
    "28b4c6a1"
)
AUTH0_VECTOR = (
    "4f54020278563412000000002e"
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
    "202122232425262728292a2b2c2d"
    "4643ec95"
)
AUTH46_VECTOR = (
    "4f540202785634122e00000012"
    "2e2f303132333435363738393a3b3c3d3e3f"
    "00000000000000000000000000000000000000000000000000000000"
    "02e50556"
)
ACK_VECTOR = (
    "4f54028078563412401000002c"
    "0300020100100000f02b0d0000821600fb0300000100012e1000f20f"
    "424c3631382d44533520332e35000000"
    "00005b21bdd1"
)
ERROR_VECTOR = (
    "4f5402ff78563412001000002c"
    "0611030100100000f02b0d0000821600fb0300000100012e1000f20f"
    "424c3631382d44533520332e35000000"
    "0000432222df"
)


ROOT = Path(__file__).resolve().parents[1]


def header_integer(name: str) -> int:
    text = (ROOT / "src" / "ota_update.h").read_text(encoding="utf-8")
    match = re.search(
        rf"\b{re.escape(name)}\s*(?:=|\s)\s*(0x[0-9A-Fa-f]+|[0-9]+)u?\b", text
    )
    if not match:
        raise AssertionError(f"missing firmware OTA constant {name}")
    return int(match.group(1), 0)


class OtaProtocolTests(unittest.TestCase):
    def test_python_and_documented_vectors_are_identical(self) -> None:
        vectors = {
            "BEGIN_VECTOR": ("BEGIN", BEGIN_VECTOR),
            "AUTH0_VECTOR": ("AUTH0", AUTH0_VECTOR),
            "AUTH46_VECTOR": ("AUTH46", AUTH46_VECTOR),
            "ACK_VECTOR": ("ACK", ACK_VECTOR),
            "ERROR_VECTOR": ("ERROR", ERROR_VECTOR),
        }
        documentation = (ROOT / "docs" / "OTA.md").read_text(encoding="utf-8")
        for _name, (label, expected) in vectors.items():
            with self.subTest(vector=label):
                self.assertRegex(
                    documentation,
                    rf"(?m)^{label}\s*=\s*{re.escape(expected)}$",
                )

    def test_report_ids_and_frame_contract(self) -> None:
        self.assertEqual(ota_protocol.OTA_DATA_REPORT_ID, 0xFA)
        self.assertEqual(ota_protocol.OTA_CONTROL_REPORT_ID, 0xFC)
        self.assertEqual(ota_protocol.OTA_PAYLOAD_SIZE, 63)
        self.assertEqual(ota_protocol.OTA_DATA_CAPACITY, 46)
        self.assertEqual(ota_protocol.OTA_CRC_OFFSET, 59)
        self.assertEqual(header_integer("OTA_DATA_REPORT_ID"), ota_protocol.OTA_DATA_REPORT_ID)
        self.assertEqual(
            header_integer("OTA_CONTROL_REPORT_ID"), ota_protocol.OTA_CONTROL_REPORT_ID
        )
        self.assertEqual(header_integer("OTA_REPORT_PAYLOAD_SIZE"), 63)
        self.assertEqual(header_integer("OTA_REPORT_CRC_OFFSET"), 59)
        self.assertEqual(header_integer("OTA_DATA_BYTES_MAX"), 46)
        self.assertEqual(header_integer("OTA_AUTH_SIGNATURE_SIZE"), 64)
        self.assertEqual(header_integer("OTA_SIGN_CANONICAL_SIZE"), 58)
        self.assertEqual(header_integer("OTA_BEGIN_DATA_SIZE"), 43)
        self.assertEqual(header_integer("OTA_STATUS_DATA_SIZE"), 44)
        self.assertEqual(header_integer("OTA_CTRL_BEGIN"), ota_protocol.OTA_CONTROL_BEGIN)
        self.assertEqual(header_integer("OTA_CTRL_AUTH"), ota_protocol.OTA_CONTROL_AUTH)
        self.assertEqual(header_integer("OTA_CTRL_COMMIT"), ota_protocol.OTA_CONTROL_COMMIT)
        self.assertEqual(header_integer("OTA_CTRL_ABORT"), ota_protocol.OTA_CONTROL_ABORT)
        self.assertEqual(header_integer("OTA_CTRL_STATUS"), ota_protocol.OTA_CONTROL_STATUS)
        self.assertEqual(header_integer("OTA_CTRL_ACK"), ota_protocol.OTA_CONTROL_ACK)
        self.assertEqual(header_integer("OTA_CTRL_ERROR"), ota_protocol.OTA_CONTROL_ERROR)
        self.assertEqual(
            header_integer("OTA_ERROR_TIMEOUT"), ota_protocol.OTA_ERROR_TIMEOUT
        )

        source = (ROOT / "src" / "ota_update.c").read_text(encoding="utf-8")
        capability_builder = re.search(
            r"static uint32_t current_capabilities\(void\)\s*\{(.*?)\n\}",
            source,
            re.DOTALL,
        )
        self.assertIsNotNone(capability_builder)
        self.assertNotIn("OTA_CAP_LIVE_RESUME", capability_builder.group(1))
        self.assertIn("OTA_SESSION_IDLE_TIMEOUT_MS 60000u", source)

    def test_begin_control_vector(self) -> None:
        body_size = 0x000D29F0
        body_sha = hashlib.sha256(b"vector").digest()
        payload = ota_protocol.encode_begin(
            session=0x12345678,
            total_size=body_size + 512,
            version=(1, 2, 3),
            body_size=body_size,
            body_sha256=body_sha,
        )
        self.assertEqual(payload.hex(), BEGIN_VECTOR)
        self.assertEqual(
            ota_protocol.encode_wire_frame(0xFC, payload).hex(), "fc" + BEGIN_VECTOR
        )
        decoded = ota_protocol.decode_payload(0xFC, payload)
        self.assertEqual(decoded.session, 0x12345678)
        self.assertEqual(decoded.argument, 0x000D2BF0)
        self.assertEqual(decoded.data[0:6], bytes((1, 1, 1, 2, 3, 1)))
        self.assertEqual(struct.unpack_from("<I", decoded.data, 6)[0], body_size)
        self.assertEqual(decoded.data[10:42], body_sha)
        self.assertEqual(decoded.data[42], ota_protocol.OTA_PROFILE_STANDARD)

    def test_begin_rejects_versions_outside_sdk_header_limits(self) -> None:
        for version in ((255, 0, 0), (0, 255, 0), (0, 0, 255)):
            with self.subTest(version=version):
                with self.assertRaisesRegex(ota_protocol.OtaProtocolError, "0..254"):
                    ota_protocol.encode_begin(
                        1,
                        513,
                        version,
                        1,
                        hashlib.sha256(b"x").digest(),
                    )
        ota_protocol.encode_begin(
            1, 513, (254, 5, 0), 1, hashlib.sha256(b"x").digest()
        )
        with self.assertRaisesRegex(ota_protocol.OtaProtocolError, "16-byte"):
            ota_protocol.encode_begin(
                1, 513, (254, 254, 254), 1, hashlib.sha256(b"x").digest()
            )

    def test_auth_signature_split_vectors(self) -> None:
        first, second = ota_protocol.encode_auth_frames(0x12345678, bytes(range(64)))
        self.assertEqual(first.hex(), AUTH0_VECTOR)
        self.assertEqual(second.hex(), AUTH46_VECTOR)
        decoded_first = ota_protocol.decode_payload(0xFC, first)
        decoded_second = ota_protocol.decode_payload(0xFC, second)
        self.assertEqual(decoded_first.argument, 0)
        self.assertEqual(decoded_second.argument, 46)
        self.assertEqual(decoded_first.data + decoded_second.data, bytes(range(64)))

    def test_ack_and_error_status_snapshot_vectors(self) -> None:
        ack = bytes.fromhex(ACK_VECTOR)
        error = bytes.fromhex(ERROR_VECTOR)
        ack_status = ota_protocol.decode_status(ack)
        error_status = ota_protocol.decode_status(error)
        self.assertEqual(ack_status.opcode, ota_protocol.OTA_CONTROL_ACK)
        self.assertEqual(ack_status.accepted_offset, 0x1040)
        self.assertEqual(ack_status.committed_offset, 0x1000)
        self.assertEqual(ack_status.capabilities, 0x3FB)
        self.assertEqual(
            ack_status.capabilities & ota_protocol.OTA_CAP_LIVE_RESUME, 0
        )
        self.assertEqual(ack_status.board, 1)
        self.assertEqual(ack_status.usb_speed, 0)
        self.assertEqual(ack_status.max_data, 46)
        self.assertEqual(ack_status.window, 16)
        self.assertEqual(ack_status.version, "BL618-DS5 3.5")
        self.assertEqual(error_status.opcode, ota_protocol.OTA_CONTROL_ERROR)
        self.assertEqual(error_status.error, 17)
        self.assertEqual(error_status.accepted_offset, 0x1000)

    def test_output_data_vector(self) -> None:
        data = b"DS5Dongle OTA test vector"
        payload = ota_protocol.encode_data(0x12345678, 0x2E, data)
        self.assertEqual(payload.hex(), DATA_VECTOR)
        self.assertEqual(
            ota_protocol.encode_wire_frame(0xFA, payload).hex(), "fa" + DATA_VECTOR
        )
        decoded = ota_protocol.decode_payload(0xFA, payload)
        self.assertEqual(decoded.opcode, ota_protocol.OTA_DATA_TYPE)
        self.assertEqual(decoded.session, 0x12345678)
        self.assertEqual(decoded.argument, 0x2E)
        self.assertEqual(decoded.data, data)

    def test_crc_covers_bytes_zero_through_58(self) -> None:
        payload = ota_protocol.encode_data(1, 0, b"abc")
        expected = struct.unpack_from("<I", payload, 59)[0]
        self.assertEqual(expected, ota_protocol.crc32_ieee(payload[:59]))
        corrupted = bytearray(payload)
        corrupted[13] ^= 0x80
        with self.assertRaisesRegex(ota_protocol.OtaProtocolError, "CRC mismatch"):
            ota_protocol.decode_payload(0xFA, bytes(corrupted))

    def test_decoder_rejects_nonzero_padding_even_with_valid_crc(self) -> None:
        payload = bytearray(ota_protocol.encode_data(1, 0, b"abc"))
        payload[20] = 1
        struct.pack_into("<I", payload, 59, ota_protocol.crc32_ieee(payload[:59]))
        with self.assertRaisesRegex(ota_protocol.OtaProtocolError, "non-zero padding"):
            ota_protocol.decode_payload(0xFA, bytes(payload))

    def test_data_is_bounded_to_46_bytes(self) -> None:
        self.assertEqual(len(ota_protocol.encode_data(1, 0, bytes(46))), 63)
        with self.assertRaisesRegex(ota_protocol.OtaProtocolError, "at least one"):
            ota_protocol.encode_data(1, 0, b"")
        with self.assertRaisesRegex(ota_protocol.OtaProtocolError, "exceeds 46"):
            ota_protocol.encode_data(1, 0, bytes(47))


if __name__ == "__main__":
    unittest.main()
