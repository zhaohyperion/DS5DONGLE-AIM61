# DS5Dongle AIM61 Tool Center

Windows-native GUI/CLI for DS5DONGLE-AIM61. Version 1.3.1 provides a white high-contrast interface with three tabs:

1. Test center for live DualSense input, lights, bounded rumble, adaptive triggers, controller audio and real UAC microphone recording/playback.
2. Firmware flasher for verified GitHub Release ZIPs, local packages, CH340-only serial selection and 460800/115200 baud recovery.
3. Device debug for guided diagnostics, 5-minute default stress testing, Windows HID timing, Diagnostic-only M61 bridge telemetry, JSON export and signed Standard/Diagnostic OTA switching.

The default online list shows the newest AIM61 High-Speed Standard firmware only. Diagnostic firmware is exposed through the advanced control. The tool validates GitHub asset digests, package checksums and target metadata; the device independently validates OTA target, body hash and P-256 signature.

Public configuration-web code and web OTA are intentionally not included. Diagnostics run locally and are not uploaded.

See [`../../docs/FLASHER.md`](../../docs/FLASHER.md), [`../../docs/DIAGNOSTICS.md`](../../docs/DIAGNOSTICS.md), and [`../../docs/OTA.md`](../../docs/OTA.md).

```powershell
$env:M61_BLFLASHCOMMAND = "C:\path\to\bouffalo_sdk\tools\bflb_tools\bouffalo_flash_cube\BLFlashCommand.exe"
cargo fmt --manifest-path tools\ds5dongle-flasher\Cargo.toml -- --check
cargo test --locked --manifest-path tools\ds5dongle-flasher\Cargo.toml
cargo build --locked --release --manifest-path tools\ds5dongle-flasher\Cargo.toml
```
