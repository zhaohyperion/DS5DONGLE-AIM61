# DS5Dongle Windows Flasher

The flasher supports AI-M61-32S-KIT, LCTech BL616 and Sipeed M0S Dock firmware in both Full-Speed and High-Speed USB variants. Online releases and local files use the same signed-by-hash package format.

## Package format

Every `DS5Dongle-<board>-<fs|hs>-v<version>.zip` contains `firmware.json`, `SHA256SUMS.txt`, one BL616 boot2 image, `partition.bin`, and exactly one board-specific application image. The flasher rejects path traversal, duplicate filenames, invalid sizes, checksum mismatches, and board/speed filename mismatches.

Create a package after building:

```powershell
python tools\package_firmware.py --board aim61 --usb-speed fs --version v3.15 --firmware-dir firmware\aim61 --output-dir dist
```

## Device selection

The Windows GUI enumerates every healthy COM device rather than assuming CH340. AI-M61 normally appears as CH340; other boards or external USB-TTL adapters may use different identities. When more than one port exists, select the port connected to the board. The WCH driver installer is only intended for an AI-M61/CH340 device.

## Safety

Select the package matching the physical board. The GUI and non-interactive CLI prefer the latest stable AI-M61 Full-Speed package by default; explicit selections remain available for every board. Full-Speed is recommended. High-Speed can provide a higher polling ceiling but is more sensitive to cabling. Put the board into UART ISP mode with BOOT+RESET before flashing. The default baud rate is 460800; retry at 115200 if the first attempt fails.

## Build the one-file GUI

The verified build uses sqlCRT `bouffalo_sdk` commit `cf6adf74b374a0e485defa9c89610f3e7ffcc3ec`. Set `M61_BLFLASHCOMMAND` to its `BLFlashCommand.exe`, then run:

```powershell
cargo test --manifest-path tools\ds5dongle-flasher\Cargo.toml
cargo build --release --manifest-path tools\ds5dongle-flasher\Cargo.toml
```

The executable is `tools/ds5dongle-flasher/target/release/ds5dongle-flasher.exe`.

The repository configures static CRT/unwind linkage for the `x86_64-pc-windows-gnullvm` target so locally distributed builds do not require `libunwind.dll` beside the EXE.
