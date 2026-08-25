# DS5Dongle AIM61 Tool Center

Windows-native GUI/CLI for DS5DONGLE-AIM61. Version 1.4.2 provides a white high-contrast interface with four tabs:

1. Test center for live DualSense input, lights, bounded rumble, independently adjustable L2/R2 adaptive-trigger force, controller audio and real UAC microphone recording/playback.
2. Button mapping for v3 19-control one-to-many mappings, synchronous combinations, macro timelines and standalone recordings.
3. Device debug for guided diagnostics, 5-minute default stress testing, Windows HID timing, Diagnostic-only M61 bridge telemetry and unified JSON export.
4. Firmware flasher for explicitly named `uart-full` GitHub/local packages and CH340 recovery; signed `ota` packages are handled only by the Standard/Diagnostic OTA controls.

The default online UART list shows the newest AIM61 High-Speed Standard `uart-full` firmware only. Diagnostic UART firmware is exposed through the advanced control. `uart-full` and `ota` ZIPs are content-disjoint and are never offered in the other workflow. The tool validates GitHub asset digests, package checksums and target metadata. For OTA, the host verifies the exact canonical P-256 signature before transfer, the device independently verifies it again, and the tool waits for USB re-enumeration and matching `0xF8` identity before reporting success. Diagnostic OTA additionally requires a validated runtime snapshot through `0xFD` or the one-shot `0xF8` fallback.

Snapshot capture rejects all-zero, boot-placeholder and incomplete data. Unified report export first checks which test sections actually contain samples and records the result in `dataSelfCheck`; structurally inconsistent sampling, microphone or firmware-snapshot data blocks export instead of creating a misleading report.

Microphone capture uses WASAPI to select an M61/DualSense input endpoint instead of depending on the Windows default input. Capture metrics and WAV data survive a playback-device failure, and the latest recording can be saved from the Test Center.

Public configuration-web code and web OTA are intentionally not included. Diagnostics run locally and are not uploaded.

The live controller diagram directly uses a localized SVG-path adaptation of the fixed `daidr/dualsense-tester` DualSense model together with its upstream 1117x892 physical-control coordinates, stick normalization and two-point touch mapping. The SVG is compiled into the native executable; the Vue application is not embedded or loaded. Attribution and the retained MIT notice are in [`../../THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md).

See [`../../docs/FLASHER.md`](../../docs/FLASHER.md), [`../../docs/DIAGNOSTICS.md`](../../docs/DIAGNOSTICS.md), and [`../../docs/OTA.md`](../../docs/OTA.md).

```powershell
$env:M61_BLFLASHCOMMAND = "C:\path\to\bouffalo_sdk\tools\bflb_tools\bouffalo_flash_cube\BLFlashCommand.exe"
cargo fmt --manifest-path tools\ds5dongle-flasher\Cargo.toml -- --check
cargo test --locked --manifest-path tools\ds5dongle-flasher\Cargo.toml
cargo build --locked --release --manifest-path tools\ds5dongle-flasher\Cargo.toml
```
