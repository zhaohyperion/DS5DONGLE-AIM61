"""Static contract checks for the BL618 DualSense firmware.

These tests intentionally validate the hardware/protocol constants that must
not move while the runtime pipeline is optimized.  They use only the Python
standard library so they can run on Windows without the Bouffalo toolchain.
"""

from __future__ import annotations

import re
import struct
import unittest
import zlib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def read(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def integer_define(relative: str, name: str) -> int:
    text = read(relative)
    match = re.search(
        rf"^\s*#define\s+{re.escape(name)}\s+"
        rf"(0x[0-9a-fA-F]+|[0-9]+)(?:[uUlL]+)?\b",
        text,
        re.MULTILINE,
    )
    if not match:
        raise AssertionError(f"missing integer define {name} in {relative}")
    return int(match.group(1), 0)


class FirmwareContractTests(unittest.TestCase):
    def test_release_endpoints_follow_the_current_repository(self) -> None:
        repository = "zhaohyperion/DS5DONGLE-AIM61"
        legacy_repositories = (
            "ccc007ccc/DS5Dongle/releases",
            "sqlCRT/ds5dongle-bl618-opensource/releases/latest",
        )

        flasher = read("tools/ds5dongle-flasher/src/main.rs")
        web = read("web/app/DeviceConsole.tsx")
        ota_docs = read("docs/OTA.md")
        for text in (flasher, web, ota_docs):
            self.assertIn(repository, text)
            for legacy in legacy_repositories:
                self.assertNotIn(legacy, text)

    def test_opus_e907_bit_exact_fast_paths(self) -> None:
        cmake = read("CMakeLists.txt")
        ecintrin = read("lib/opus/celt/ecintrin.h")
        fixed = read("lib/opus/celt/fixed_generic.h")
        entcode = read("lib/opus/celt/entcode.h")
        vq = read("lib/opus/celt/vq.c")
        bands = read("lib/opus/celt/bands.c")

        self.assertIn("option(DS5_OPUS_E907_OPTIMIZATIONS", cmake)
        for define in (
            "M61_OPUS_E907_CLZ32=1",
            "M61_OPUS_E907_Q15_KMMWB2=1",
            "M61_OPUS_E907_Q16_SMMWB=1",
            "M61_OPUS_E907_POW2_DIV=1",
        ):
            self.assertIn(define, cmake)
        self.assertIn('__asm__("clz32 %0, %1"', ecintrin)
        self.assertIn("value == 0 ? 32 : __builtin_clz(value)", ecintrin)
        self.assertIn('__asm__("kmmwb2 %0, %1, %2"', fixed)
        self.assertIn('__asm__("smmwb %0, %1, %2"', fixed)
        self.assertIn("static OPUS_INLINE opus_uint32 celt_udiv_pow2", entcode)
        self.assertIn("celt_assert((d&(d-1))==0)", entcode)
        self.assertEqual(vq.count("celt_udiv_pow2("), 2)
        self.assertEqual(bands.count("celt_udiv_pow2("), 1)

    def test_runtime_diagnostics_feature_contract(self) -> None:
        header = "src/runtime_diag.h"
        self.assertEqual(integer_define(header, "RUNTIME_DIAG_REPORT_ID"), 0xFD)
        self.assertEqual(integer_define(header, "RUNTIME_DIAG_REPORT_SIZE"), 63)
        self.assertEqual(integer_define(header, "RUNTIME_DIAG_CRC_OFFSET"), 59)
        self.assertEqual(integer_define(header, "RUNTIME_DIAG_PROTOCOL_VERSION"), 1)
        self.assertEqual(integer_define(header, "RUNTIME_DIAG_HEADER_SIZE"), 16)
        self.assertEqual(integer_define(header, "RUNTIME_DIAG_DATA_MAX"), 43)
        self.assertEqual(integer_define(header, "RUNTIME_DIAG_PAGE_COUNT"), 6)
        self.assertEqual(integer_define(header, "RUNTIME_DIAG_SELECT_PAGE"), 1)

        ota_header = "src/ota_update.h"
        ota_data_id = integer_define(ota_header, "OTA_DATA_REPORT_ID")
        ota_control_id = integer_define(ota_header, "OTA_CONTROL_REPORT_ID")
        self.assertEqual(ota_data_id, 0xFA)
        self.assertEqual(ota_control_id, 0xFC)
        self.assertEqual(len({ota_data_id, ota_control_id, 0xFD}), 3)

        usb = read("src/usb_gamepad.c")
        self.assertEqual(integer_define("src/usb_gamepad.c", "HID_REPORT_DESC_SIZE_DS"), 353)
        self.assertEqual(integer_define("src/usb_gamepad.c", "HID_REPORT_DESC_SIZE_DSE"), 469)
        self.assertEqual(usb.count("0x85, RUNTIME_DIAG_REPORT_ID"), 2)
        self.assertEqual(usb.count("0x09, 0x3F"), 2)
        self.assertIn("report_id == RUNTIME_DIAG_REPORT_ID", usb)
        self.assertIn("runtime_diag_get_selected_report(feature_resp_buf + 1)", usb)
        self.assertIn("*len = 1 + RUNTIME_DIAG_REPORT_SIZE", usb)
        self.assertIn("runtime_diag_select_page_from_isr(payload, payload_len)", usb)

        ep0_region = usb[
            usb.index("void usbd_hid_get_report") :
            usb.index("void usb_soft_disconnect")
        ]
        for forbidden in (
            "usb_gamepad_get_runtime_stats",
            "usb_audio_get_stats",
            "audio_get_runtime_stats",
            "bt_hid_host_get_tx_stats",
            "xPortGetFreeHeapSize",
            "xPortGetMinimumEverFreeHeapSize",
            "uxTaskGetStackHighWaterMark",
        ):
            self.assertNotIn(forbidden, ep0_region)

        diag = read("src/runtime_diag.c")
        self.assertIn(
            "diag_reports[2][RUNTIME_DIAG_PAGE_COUNT]",
            diag,
        )
        self.assertIn('asm volatile("fence rw, rw"', diag)
        self.assertIn("diag_report_index = next", diag)
        self.assertIn("diag_selected_page = RUNTIME_DIAG_PAGE_IDENTITY", diag)
        self.assertIn("payload[2] >= RUNTIME_DIAG_PAGE_COUNT", diag)
        self.assertIn("memcpy(out, diag_reports[index][page]", diag)
        self.assertIn("if (!bt_connected || rssi >= 0)", diag)
        self.assertIn("rssi = INT8_MAX", diag)
        self.assertIn("kfree_size(0)", diag)
        self.assertIn("diag_min_free_bytes", diag)
        self.assertIn("free_bytes < diag_min_free_bytes", diag)
        self.assertNotIn("kmin_free_size(", diag)
        self.assertIn("audio_get_runtime_diag_stats(&audio_stats)", diag)
        self.assertNotIn("audio_get_runtime_stats(&audio_stats)", diag)
        self.assertNotIn("xPortGetFreeHeapSize", diag)
        self.assertNotIn("xPortGetMinimumEverFreeHeapSize", diag)
        self.assertIn(
            "data[16] != 0 ? bt_hid_host_get_active_idx() : 0xFFu",
            diag,
        )
        self.assertIsNone(
            re.search(r"(?:LOG_(?:INF|ERR|WRN|DBG)|printf)\s*\(", diag)
        )

        get_region = diag[diag.index("void runtime_diag_get_selected_report") :]
        for forbidden in (
            "usb_gamepad_get_runtime_stats",
            "usb_audio_get_stats",
            "audio_get_runtime_stats",
            "bt_hid_host_get_tx_stats",
            "xPortGetFreeHeapSize",
            "ota_update_crc32",
        ):
            self.assertNotIn(forbidden, get_region)

        main = read("src/main.c")
        diag_task = main[
            main.index("static void diagnostics_task") :
            main.index("static void usb_task")
        ]
        usb_task = main[
            main.index("static void usb_task") :
            main.index("static void led_task")
        ]
        self.assertEqual(integer_define("src/main.c", "DIAG_TASK_STACK_DEPTH"), 1024)
        self.assertEqual(integer_define("src/main.c", "DIAG_TASK_PRIORITY"), 1)
        self.assertIn("#if DIAG_TASK_PRIORITY >= LED_TASK_PRIORITY", main)
        self.assertIn("runtime_diag_task_update(now_us)", diag_task)
        self.assertIn("RUNTIME_DIAG_PUBLISH_INTERVAL_MS", diag_task)
        self.assertIn("diagnostics_log_runtime()", diag_task)
        self.assertNotIn("runtime_diag_task_update", usb_task)
        self.assertNotIn("[USB-PERF]", usb_task)
        self.assertNotIn("ota_update_crc32", usb_task)
        self.assertIn(
            'xTaskCreate(diagnostics_task, "diag", DIAG_TASK_STACK_DEPTH', main
        )
        self.assertLess(
            main.index('xTaskCreate(ota_update_task, "ota"'),
            main.index('xTaskCreate(diagnostics_task, "diag"'),
        )
        self.assertIn("diagnostics task alloc FAIL; bridge continues", main)

        audio_header = read("src/audio.h")
        self.assertIn("audio_runtime_diag_stats_t", audio_header)
        self.assertIn("audio_get_runtime_diag_stats", audio_header)
        audio = read("src/audio.c")
        compact = audio[audio.index("void audio_get_runtime_diag_stats") :]
        critical = compact[
            compact.index("taskENTER_CRITICAL()") :
            compact.index("taskEXIT_CRITICAL()")
        ]
        self.assertNotIn("snapshot = runtime_stats", critical)
        self.assertNotIn("histogram", critical)
        self.assertIn("runtime_stats.block.samples", critical)
        self.assertIn("audio_timing_p99_samples", compact)
        full = audio[
            audio.index("void audio_get_runtime_stats") :
            audio.index("void audio_get_runtime_diag_stats")
        ]
        self.assertIn("snapshot = runtime_stats", full)

        # Golden DG/v1 page validates the documented offsets and CRC domain.
        frame = bytearray(63)
        frame[0:2] = b"DG"
        frame[2:8] = bytes((1, 16, 0, 6, 37, 3))
        struct.pack_into("<II", frame, 8, 1, 1000)
        struct.pack_into("<I", frame, 16, 1000)
        frame[20:25] = bytes((7, 3, (-42) & 0xFF, 80, 1))
        crc = zlib.crc32(frame[:59]) & 0xFFFFFFFF
        self.assertEqual(crc, 0xACC89D44)
        struct.pack_into("<I", frame, 59, crc)
        self.assertEqual(struct.unpack_from("<I", frame, 59)[0], crc)

        docs = read("docs/DIAGNOSTICS.md")
        for fragment in (
            "Feature Report `0xFD`",
            "CRC-32/IEEE",
            "Page 0: identity and health",
            "Page 5: OTA and memory",
            "[0x01, 0x01, page]",
            "priority 1",
            "4 KiB",
        ):
            self.assertIn(fragment, docs)

    def test_custom_log_level_build_contract(self) -> None:
        cmake = read("CMakeLists.txt")
        self.assertIn('set(DS5_LOG_LEVEL "2" CACHE STRING', cmake)
        self.assertIn(
            'set_property(CACHE DS5_LOG_LEVEL PROPERTY STRINGS 0 1 2 3)',
            cmake,
        )
        self.assertIn('MATCHES "^[0-3]$"', cmake)
        self.assertIn("DS5_LOG_LEVEL must be exactly one of 0, 1, 2, or 3", cmake)
        self.assertIn(
            "target_compile_definitions(app PRIVATE LOG_LEVEL=${DS5_LOG_LEVEL})",
            cmake,
        )

        windows = read("build_windows.bat")
        self.assertIn('if "%DS5_LOG_LEVEL%"=="" set "DS5_LOG_LEVEL=2"', windows)
        self.assertIn("-%USB_SPEED%-log%DS5_LOG_LEVEL%", windows)
        self.assertIn("USB: %USB_SPEED%  LOG: %DS5_LOG_LEVEL%", windows)
        self.assertIn("Invalid DS5_LOG_LEVEL", windows)

        self.assertIn("DS5_LOG_LEVEL=3", read("README.md"))

    def test_cherryusb_init_events_are_deferred_not_unknown(self) -> None:
        usb = read("src/usb_gamepad.c")
        event_region = usb[
            usb.index("static void usbd_event_handler") :
            usb.index("void usb_gamepad_set_dse_mode")
        ]
        self.assertIn("case USBD_EVENT_INIT:", event_region)
        self.assertIn("usb_event_logs_pending |= USB_LOG_INIT", event_region)
        self.assertIn("case USBD_EVENT_DEINIT:", event_region)
        self.assertIn("usb_event_logs_pending |= USB_LOG_DEINIT", event_region)
        self.assertLess(
            event_region.index("case USBD_EVENT_INIT:"),
            event_region.index("default:"),
        )
        self.assertIsNone(
            re.search(r"LOG_(?:INF|ERR|WRN|DBG)\s*\(", event_region)
        )
        self.assertNotIn("printf(", event_region)

        deferred = usb[
            usb.index("void usb_gamepad_process_deferred") :
            usb.index("int usb_gamepad_init")
        ]
        self.assertIn("[USB-EVT] INIT - device controller ready", deferred)
        self.assertIn("[USB-EVT] DEINIT - device controller stopped", deferred)

    def test_bluetooth_classic_hid_contract(self) -> None:
        self.assertEqual(integer_define("src/bt_hid_host.h", "HID_PSM_CONTROL"), 0x11)
        self.assertEqual(integer_define("src/bt_hid_host.h", "HID_PSM_INTERRUPT"), 0x13)
        self.assertEqual(integer_define("src/bt_hid_host.c", "L2CAP_BR_MTU"), 672)

        project = read("proj.conf")
        self.assertIn("set(CONFIG_BT_BREDR y)", project)
        self.assertIn("set(CONFIG_BT_L2CAP_DYNAMIC_CHANNEL y)", project)
        self.assertIn("set(CONFIG_BT_L2CAP_BR_SERVER y)", project)

        config = read("defconfig")
        self.assertIn("CONFIG_BTBLECONTROLLER_LIB =ble1m2s1bredr1", config)
        self.assertIn("CONFIG_EM_SIZE =32", config)

        board = read("src/board_config.h")
        self.assertIn('#define BOARD_NAME          "Ai-M61"', board)
        self.assertIn("#define LED_WHITE_PIN       GPIO_PIN_29", board)
        self.assertIn("#define LED_RED_PIN         GPIO_PIN_12", board)
        self.assertIn("#define LED_GREEN_PIN       GPIO_PIN_14", board)
        self.assertIn("#define LED_BLUE_PIN        GPIO_PIN_15", board)

    def test_dualsense_report_sizes_and_ids(self) -> None:
        header = "src/ds5_protocol.h"
        self.assertEqual(integer_define(header, "DS5_BT_OUTPUT_REPORT_ID"), 0x31)
        self.assertEqual(integer_define(header, "DS5_BT_OUTPUT_REPORT_ID_EXT"), 0x32)
        self.assertEqual(integer_define(header, "DS5_BT_AUDIO_REPORT_ID"), 0x39)
        self.assertEqual(integer_define(header, "DS5_BT_OUTPUT_REPORT_SIZE"), 78)
        self.assertEqual(integer_define(header, "DS5_BT_OUTPUT_EXT_SIZE"), 142)
        self.assertEqual(integer_define(header, "DS5_BT_AUDIO_REPORT_SIZE"), 547)
        self.assertEqual(integer_define(header, "DS5_BT_OUTPUT_CRC_SEED"), 0xA2)

        # HIDP DATA|OUTPUT adds one byte in front of the report.  The largest
        # DS5 packet must remain inside one L2CAP SDU configured by the host.
        l2cap_mtu = integer_define("src/bt_hid_host.c", "L2CAP_BR_MTU")
        audio_sdu = integer_define(header, "DS5_BT_AUDIO_REPORT_SIZE") + 1
        self.assertLessEqual(audio_sdu, l2cap_mtu)

    def test_usb_audio_wire_format(self) -> None:
        header = "src/ds5_usb_audio.h"
        self.assertEqual(integer_define(header, "USB_AUDIO_SAMPLE_RATE"), 48000)
        self.assertEqual(integer_define(header, "USB_AUDIO_CHANNELS"), 4)
        self.assertEqual(integer_define(header, "USB_AUDIO_BITS"), 16)
        self.assertEqual(integer_define(header, "USB_AUDIO_MIC_CHANNELS"), 2)
        self.assertEqual(integer_define(header, "USB_AUDIO_BLOCK_SAMPLES"), 512)
        self.assertEqual(integer_define(header, "USB_AUDIO_PCM_BLOCK_COUNT"), 4)

        source = read("src/ds5_usb_audio.c")
        interval = re.search(
            r"#ifdef\s+FORCE_FS_MODE\s*"
            r"#define\s+USB_AUDIO_ISO_INTERVAL\s+0x01\s*"
            r"#else\s*"
            r"#define\s+USB_AUDIO_ISO_INTERVAL\s+0x04\s*"
            r"#endif",
            source,
        )
        self.assertIsNotNone(interval, "FS and HS must both describe 1 ms ISO")
        self.assertEqual(source.count("USB_AUDIO_ISO_INTERVAL,"), 2)
        irq_region = source[
            source.index("audio_ep_out_handler") :
            source.index("void usb_audio_process_deferred")
        ]
        self.assertNotIn("LOG_", irq_region)
        self.assertIn(
            "static __attribute__((noinline)) void audio_mic_ep_in_handler",
            source,
        )

    def test_audio_packet_layout_is_still_double_frame(self) -> None:
        audio = read("src/audio.c")
        required_fragments = (
            "pkt[0] = DS5_BT_AUDIO_REPORT_ID",
            "pkt[10] = DS5_AUDIO_TAG_HAPTICS",
            "pkt[140]",
            "opus_slots[0]",
            "opus_slots[1]",
            "DS5_BT_AUDIO_REPORT_SIZE - 4",
        )
        for fragment in required_fragments:
            self.assertIn(fragment, audio)
        self.assertIn("encoded != OPUS_OUT_SIZE", audio)
        self.assertIn("deadline_misses", audio)
        self.assertIn("static __attribute__((noinline)) void resample_512_480", audio)

    def test_bounded_realtime_schedulers(self) -> None:
        bt = read("src/bt_hid_host.c")
        self.assertEqual(integer_define("src/bt_hid_host.c", "APP_AUDIO_DEPTH"), 2)
        self.assertEqual(integer_define("src/bt_hid_host.c", "APP_CONTROL_DEPTH"), 4)
        self.assertEqual(integer_define("src/bt_hid_host.c", "APP_MAX_AUDIO_BURST"), 2)
        self.assertIn("bt_l2cap_send_cb", bt)
        self.assertNotIn("bt_l2cap_br_chan_send(", bt)
        self.assertNotIn("bt_l2cap_br_chan_send_cb(", bt)
        self.assertIn("bt_hid_host_drop_audio_pending", bt)
        self.assertIn("audio_dropped_stale", bt)
        self.assertIn("feature_cache[slot].len = 0", bt)
        self.assertIn("feature_cache_barrier", bt)
        self.assertIn("app_game_merge_gates_locked", bt)
        self.assertIn("APP_GAME_MISC_FLAGS_OFF", bt)
        self.assertIn("app_game_rechecksum_locked", bt)
        rssi_region = bt[
            bt.index("int bt_hid_host_read_rssi") :
            bt.index("int8_t bt_hid_host_get_cached_rssi")
        ]
        self.assertIn("bt_conn_ref", rssi_region)
        self.assertIn("bt_conn_unref", rssi_region)

        # Known DualSense output IDs have fixed wire sizes; malformed known
        # reports must not silently fall into another scheduler class.
        self.assertIn("len != DS5_BT_OUTPUT_REPORT_SIZE", bt)
        self.assertIn("len != DS5_BT_OUTPUT_EXT_SIZE", bt)
        self.assertIn("len != DS5_BT_AUDIO_REPORT_SIZE", bt)

    def test_usb_input_epoch_and_build_optimization_contract(self) -> None:
        main = read("src/main.c")
        usb = read("src/usb_gamepad.c")
        cmake = read("CMakeLists.txt")

        self.assertIn("connection_epoch", main)
        self.assertIn("output_queue_item_t", main)
        self.assertIn("queued_output.connection_epoch", main)
        self.assertIn("input_epoch_is_current", main)
        self.assertIn("usb_gamepad_stage_raw_input", usb)
        self.assertIn("usb_gamepad_commit_raw_input", usb)
        self.assertIn("usb_was_configured_before_suspend", usb)
        self.assertIn("usb_audio_suspend();", usb)
        self.assertIn("usb_audio_resume(busid);", usb)
        self.assertIn("request_led_primer", main)
        self.assertIn("retry_led_primer", main)
        self.assertIn("pending_mute_light", main)
        self.assertIn("usb_audio_mic_flush", read("src/audio.c"))
        audio_usb = read("src/ds5_usb_audio.c")
        self.assertIn("stream_requested", audio_usb)
        self.assertIn("mic_requested", audio_usb)
        self.assertIn(
            "audio_set_mic_active(usb_audio_mic_is_active())", main
        )

        state = read("src/state_mgr.c")
        self.assertIn("state_mgr_snapshot", state)
        self.assertIn("state_mgr_ack", state)
        self.assertIn("state_revision", state)

        self.assertIn('src/audio.c PROPERTIES COMPILE_OPTIONS "-O3"', cmake)
        self.assertIn('${OPUS_SOURCES} PROPERTIES COMPILE_OPTIONS "-O2"', cmake)
        self.assertNotIn("<COMPILE_LANGUAGE:C>:-flto", cmake)

    def test_usb_callbacks_are_bounded_and_speed_correct(self) -> None:
        usb = read("src/usb_gamepad.c")
        debug = read("src/debug_log.h")

        event_region = usb[
            usb.index("static void usbd_event_handler") :
            usb.index("void usb_gamepad_set_dse_mode")
        ]
        ep0_region = usb[
            usb.index("void usbd_hid_get_report") :
            usb.index("void usb_soft_disconnect")
        ]
        active_log = re.compile(r"LOG_(?:INF|ERR|WRN|DBG)\s*\(")
        self.assertIsNone(active_log.search(event_region))
        self.assertIsNone(active_log.search(ep0_region))
        self.assertNotIn("printf(", event_region)
        self.assertNotIn("printf(", ep0_region)
        self.assertRegex(
            debug,
            r"#define\s+LOG_ISR\(fmt, \.\.\.\)\s+\(\(void\)0\)",
        )

        # Runtime speed matters: an HS-capable BL618 can enumerate at FS when
        # attached through a full-speed hub.  Current and other-speed
        # descriptors must therefore be derived from CherryUSB's speed value,
        # not only from the build-time FORCE_FS_MODE switch.
        self.assertIn("speed == USB_SPEED_HIGH", usb)
        self.assertIn("speed != USB_SPEED_HIGH", usb)
        self.assertIn("patch_descriptor_intervals(config_desc", usb)
        self.assertIn("high_speed ? 4 : 1", usb)
        self.assertIn("USB_KBD_INTERVAL_HS  7", usb)
        self.assertIn("usb_config_apply_pending", usb)
        self.assertNotIn("config_set(payload", ep0_region)
        self.assertNotIn("remap_set(payload", ep0_region)
        self.assertNotIn("dse_on_profile_write", ep0_region)

        wake = read("src/usb_wake.c")
        dse = read("src/dse.c")
        self.assertNotRegex(wake, r"volatile\s+uint64_t")
        self.assertNotIn("static uint64_t", dse)
        self.assertIn("suspend_at_us == suspended_at", wake)

    def test_psram_is_explicit_and_kept_off_realtime_paths(self) -> None:
        cmake = read("CMakeLists.txt")
        layout = read("src/memory_layout.h")
        bt = read("src/bt_hid_host.c")

        # Only Ai-M61 is guaranteed to carry the module's 4 MiB x8 PSRAM.
        self.assertIn("NOT DEFINED ENV{BOARD_LCTECH_616}", cmake)
        self.assertIn("NOT DEFINED ENV{BOARD_M0S_DOCK}", cmake)
        self.assertIn("set(CONFIG_PSRAM y)", cmake)
        self.assertIn("set(CONFIG_PSRAM_LENGTH 4194304)", cmake)
        self.assertIn("set(CONFIG_PSRAM_SKIP_REGISTER_HEAP y)", cmake)
        self.assertIn("set(CONFIG_PSRAM_COPY_CODE n)", cmake)

        # The no-init section is safe only because both cold objects have an
        # explicit clear before their first use.
        self.assertIn("ATTR_NOINIT_PSRAM_SECTION", layout)
        self.assertRegex(
            bt,
            r"feature_cache_payload\[FEATURE_CACHE_SLOTS\]"
            r"\[FEATURE_DATA_MAX\]\s*DS5_PSRAM_COLD_DATA",
        )
        metadata_decl = bt[bt.index("static struct {") : bt.index("/* Keep the publication")]
        self.assertNotIn("DS5_PSRAM_COLD_DATA", metadata_decl)
        self.assertIn("*data = feature_cache_payload[i];", bt)
        self.assertRegex(
            bt,
            r"discovery_results\[BT_MAX_DISCOVERED\]\s*"
            r"DS5_PSRAM_COLD_DATA",
        )
        self.assertIn("feature_cache_clear();", bt)
        self.assertIn(
            "memset(discovery_results, 0, sizeof(discovery_results));", bt
        )

        # Hard-realtime storage must remain in internal SRAM/TCM.
        for source in (read("src/audio.c"), read("src/ds5_usb_audio.c")):
            self.assertNotIn("DS5_PSRAM_COLD_DATA", source)

    def test_aim61_is_the_safe_default_build_target(self) -> None:
        windows = read("build_windows.bat")
        posix = read("build_macos.sh")

        self.assertIn('if "%BOARD_TYPE%"=="" set "BOARD_TYPE=aim61"', windows)
        self.assertIn('BOARD_TYPE="${BOARD_TYPE:-aim61}"', posix)
        self.assertNotIn('set "BOARD_TYPE=lctech616"', windows)
        self.assertNotIn('BOARD_TYPE="${BOARD_TYPE:-lctech616}"', posix)

        # A typo must fail instead of silently producing firmware with the
        # wrong GPIO and PSRAM policy for the connected board.
        self.assertIn("Unknown BOARD_TYPE", windows)
        self.assertIn("unknown BOARD_TYPE", posix)
        self.assertIn("Unknown USB_SPEED", windows)
        self.assertIn("unknown USB_SPEED", posix)

    def test_ota_ab_rollback_and_status_contract(self) -> None:
        ota = read("src/ota_update.c")

        # M61 slot A is larger than slot B.  The device must permanently cap
        # accepted images to the smaller slot so every accepted release can
        # continue alternating between A and B.
        self.assertIn("uint32_t body_max = entry.max_len[0]", ota)
        self.assertIn("entry.max_len[1] < body_max", ota)
        self.assertNotIn(
            "ota_ctx.max_image_size = ota_ctx.sdk->part_size +", ota
        )
        capabilities = ota[ota.index("static uint32_t current_capabilities") :
                           ota.index("static int load_release_public_key")]
        self.assertNotIn("OTA_CAP_LIVE_RESUME", capabilities)

        # Boot2 decrements F2->F1 and F1->F0 before launching the two trial
        # attempts.  The application must confirm both F1 and F0 boots.
        self.assertIn(
            "trial_boot = ota_ctx.trial_retry_byte >= 0xF0u", ota
        )

        # A STATUS probe must not clear a pending asynchronous error before
        # the browser's following GET_REPORT can observe it.
        status = ota[ota.index("static void process_status") :
                     ota.index("static void process_control")]
        self.assertNotIn("ota_ctx.error = OTA_ERROR_OK", status)
        self.assertIn("OTA_SESSION_IDLE_TIMEOUT_MS", ota)
        self.assertIn("fail_transfer(OTA_ERROR_TIMEOUT)", ota)

        begin = ota[ota.index("static void process_begin") :
                    ota.index("static void process_auth")]
        self.assertIn("OTA_BEGIN_SEMVER_OFFSET] == 0xFFu", begin)
        self.assertIn("OTA_BEGIN_SEMVER_OFFSET + 1] == 0xFFu", begin)
        self.assertIn("OTA_BEGIN_SEMVER_OFFSET + 2] == 0xFFu", begin)
        self.assertIn("!semver_fits_ota_header", begin)

        cmake = read("CMakeLists.txt")
        self.assertIn(
            "target_compile_definitions(app PRIVATE CONFIG_ENABLE_IMG_HASH)",
            cmake,
        )
        self.assertIn(
            "target_compile_definitions(libfota PRIVATE CONFIG_OTA_VERSION_CHECK)",
            cmake,
        )
        self.assertIn("DS5_SELECTED_VERSION_LENGTH GREATER 8", cmake)


if __name__ == "__main__":
    unittest.main()
