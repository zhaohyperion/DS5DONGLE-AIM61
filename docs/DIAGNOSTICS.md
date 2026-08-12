# AI-M61 runtime diagnostics

The supported diagnostics path is deliberately split into two low-overhead channels:

- the on-board CH340 Type-C port carries boot and event logs at 115200 8-N-1;
- native USB carries versioned vendor-HID state and diagnostic snapshots.

Do not print every USB, Bluetooth, controller, audio, or OTA packet. At 115200 baud the UART carries roughly 11.5 KB/s, far below the raw DS5 and audio data rate. Packet-by-packet logging changes the timing being measured and can itself create USB deadline misses, Bluetooth backpressure, and audio underruns.

## Receive-only Windows collector

Close the flasher and other serial terminals, then run:

```powershell
powershell -ExecutionPolicy Bypass -File tools\m61-diagnostics.ps1 -Port COM8
```

The port can normally be detected automatically, so `-Port COM8` is optional. Start the collector before pressing RESET, and do not hold BOOT during the reset. Stop with Ctrl+C, or request a bounded capture:

```powershell
powershell -ExecutionPolicy Bypass -File tools\m61-diagnostics.ps1 -DurationSeconds 120
```

The collector never sends serial data, changes DTR/RTS, resets the board, erases flash, or starts a firmware update. Every session contains:

- `uart.raw.log`: exact local log, potentially containing device identifiers;
- `uart.redacted.log`: Bluetooth address, USB serial, SSP passkey, and selected pointers removed;
- `events.redacted.jsonl`: timestamped, categorized events;
- `metadata.json`: capture and host metadata;
- `summary.json`: versions, readiness flags, final state, and redline matches.

A `*-safe.zip` archive is created beside the session directory. It excludes `uart.raw.log` and is the preferred artifact to share.

List ports or test the collector without opening a device:

```powershell
powershell -ExecutionPolicy Bypass -File tools\m61-diagnostics.ps1 -ListPorts
powershell -ExecutionPolicy Bypass -File tools\m61-diagnostics.ps1 -SelfTest
```

## Firmware logging policy

Production firmware stays at custom `LOG_LEVEL=2`: errors, warnings, and lifecycle events. A diagnostic build may use level 3 to emit fixed-size aggregate USB, audio, queue, and Bluetooth counters every ten seconds.

The following paths must never print or hex-dump payloads:

- USB endpoint, EP0, ISO, bus ISR, and event callbacks;
- Bluetooth HCI/L2CAP per-packet callbacks;
- DS5 250/500/1000 Hz input handling;
- 1 ms audio ISO, Opus, resampling, and microphone-ring paths;
- OTA DATA frames, flash slices, and critical sections.

Hot paths only update preallocated counters, timestamps, maxima, and high-water marks. A low-priority task publishes an atomic diagnostic snapshot and performs any periodic logging.

## Native diagnostics

Native USB and the CH340 Type-C port are independent and can be used at the same time. The Windows test center and device-debug page read structured USB HID snapshots at a low rate while the CH340 remains available for serial recovery. Collection is suspended during OTA transfer and verification. Exported JSON is local-only and redacts identifiers by default.

For AI-M61, native USB still requires the USB_DM/USB_DP/GND wiring described in the main README. When the board is already powered through its Type-C port, do not connect the cut USB cable's red 5 V conductor.

## Vendor HID Feature Report `0xFD`

Runtime diagnostics use local vendor Feature Report `0xFD`. It is deliberately separate from configuration reports `0xF6`-`0xF9`, remapping `0xFB`, and OTA reports `0xFA`/`0xFC`; it is never forwarded to the controller. Both standard DualSense and DualSense Edge descriptors declare 63 payload bytes for this report.

The layouts below describe the 63-byte payload of Feature Report `0xFD`. CherryUSB internally prefixes that payload with the report ID, so the firmware class callback returns 64 bytes. Native host APIs may expose that prefix differently; it is never part of the CRC.

### Page selection

Page zero is selected after boot. Select another page with:

```text
sendFeatureReport(0xFD, [0x01, 0x01, page])
                         |     |     +-- page 0..6
                         |     +-------- protocol version 1
                         +-------------- SELECT_PAGE opcode
```

The firmware accepts a payload of at least three bytes because HID stacks may pad the report; trailing bytes are reserved and ignored. An invalid opcode, version, length, or page leaves the previous selection unchanged. SET performs only bounded byte validation and one atomic page store. It does not capture data, compute a CRC, allocate memory, log, or contact Bluetooth.

The selector is device-global, so two host readers can race. A reader must validate the returned page index as well as the sequence. GET performs one fixed 63-byte copy from the current published bank.

### Common response header

Each second, the dedicated low-priority `diag` task builds all seven pages in an inactive bank and atomically publishes the complete bank. Every page in one snapshot has the same sequence and monotonic time. The task runs at FreeRTOS priority 1 with 1024 RV32 stack words (4 KiB), below LED, microphone, audio, Bluetooth, OTA, USB, and timer work.

| Payload bytes | Type | Meaning |
| --- | --- | --- |
| `0..1` | ASCII | Magic `DG` |
| `2` | `u8` | Protocol version, currently `1` |
| `3` | `u8` | Header length, `16` |
| `4` | `u8` | Page index, `0..6` |
| `5` | `u8` | Page count, `7` |
| `6` | `u8` | Used page-data bytes, at most `43` |
| `7` | bitset | Snapshot flags |
| `8..11` | `u32 LE` | Snapshot sequence |
| `12..15` | `u32 LE` | Monotonic milliseconds |
| `16..58` | bytes | Page data, zero after `data_len` |
| `59..62` | `u32 LE` | CRC-32/IEEE of payload bytes `0..58` |

CRC uses reflected polynomial `0xEDB88320`, initial value `0xFFFFFFFF`, and final XOR `0xFFFFFFFF`. Header flag bits are: bit 0 valid, bit 1 coherent multi-page snapshot, bit 2 PSRAM present, and bit 3 OTA maintenance active.

The sequence, monotonic time, and all cumulative `u32` counters wrap naturally modulo `2^32`. A multi-page reader should retry the complete group if any page has a different page index, sequence, or monotonic time. Reading does not clear counters.

All offsets below are relative to payload byte 16, the start of page data.

### Page 0: identity and health (`data_len=37`)

| Offset | Type | Meaning |
| --- | --- | --- |
| `0` | `u32` | Uptime in milliseconds, equal to the common monotonic value |
| `4` | `u8` | `enum bt_hid_host_state` |
| `5` | bitset | Health flags |
| `6` | `i8` | Cached RSSI in dBm; `127` means unavailable |
| `7` | `u8` | Battery percent; `0xFF` means unavailable |
| `8` | `u8` | Raw DualSense battery-state nibble |
| `9` | `u8` | Board ID: 1 AI-M61, 2 LCTech BL616, 3 M0S Dock |
| `10` | `u8` | Runtime CherryUSB speed: 0 unknown, 2 full speed, 3 high speed |
| `11` | `u8` | Firmware speed policy: 0 FS variant, 1 HS variant |
| `12` | `u8` | Compiled custom `LOG_LEVEL` |
| `13..15` | `u8[3]` | Firmware semantic version |
| `16` | `u8` | Bonded-controller count |
| `17` | `u8` | Active bonded-controller index; `0xFF` when no bond exists |
| `18` | bitset | USB lifecycle flags |
| `19` | `u8` | Controller-switch operation active |
| `20` | bitset | Diagnostic capabilities |
| `21` | `u32` | GET request count |
| `25` | `u32` | SELECT request count |
| `29` | `u32` | Invalid SELECT count |
| `33` | `u32` | Publish interval in milliseconds |

Health bits are USB configured, BT connected, Edge controller, speaker active, microphone active, OTA maintenance, PSRAM present, and USB suspended in bits 0 through 7. USB lifecycle bits are configured, suspended, maintenance, Edge descriptor mode, HID IN busy, and keyboard interface registered in bits 0 through 5. Capability bits are paging, CRC32, coherent snapshots, monotonic time, read-only telemetry, SELECT_PAGE, and bridge-latency timing in bits 0 through 6.

### Page 1: USB and Bluetooth traffic (`data_len=43`)

| Offset | Type | Meaning |
| --- | --- | --- |
| `0` | `u32` | Completed USB HID IN transfers |
| `4` | `u32` | Bluetooth HID input reports received |
| `8` | `u32` | Completed BT game + audio + control outputs |
| `12` | `u32` | USB input-state updates |
| `16` | `u32` | USB latest-state coalesces |
| `20` | `u32` | USB HID IN transfers started |
| `24` | `u32` | USB start errors |
| `28` | `u32` | BT game reports enqueued |
| `32` | `u32` | BT game reports coalesced |
| `36` | `u32` | BT game reports completed |
| `40` | bitset | Bit 0 game pending, bit 1 BT output in flight |
| `41` | `u8` | Current BT audio queue depth |
| `42` | `u8` | Current BT control queue depth |

### Page 2: loss and backpressure (`data_len=43`)

| Offset | Type | Meaning |
| --- | --- | --- |
| `0` | `u32` | Aggregate pressure events from the individual loss/failure counters |
| `4` | `u32` | USB start errors |
| `8` | `u32` | BT audio backpressure |
| `12` | `u32` | Stale BT audio reports dropped |
| `16` | `u32` | BT control backpressure |
| `20` | `u32` | BT allocation failures |
| `24` | `u32` | BT send failures |
| `28` | `u32` | Stale BT completions |
| `32` | `u32` | USB PCM blocks dropped |
| `36` | `u32` | USB PCM pool starvations |
| `40` | `u8` | BT audio queue high-water mark |
| `41` | `u8` | BT control queue high-water mark |
| `42` | bitset | Bit 0 indicates at least one Opus encode error |

The aggregate at offset 0 is a trend indicator, not a packet count: it sums USB errors, BT pressure/failures, PCM loss/starvation/overrun, and microphone under/overrun counters modulo `2^32`.

### Page 3: audio processing timing (`data_len=41`)

| Offset | Type | Meaning |
| --- | --- | --- |
| `0` | `u32` | Audio frames processed |
| `4` | `u32` | Audio pairs submitted to BT |
| `8` | `u32` | Audio pairs rejected |
| `12` | `u32` | Input discontinuities |
| `16` | `u32` | Opus encode errors |
| `20` | `u32` | Pipeline resets |
| `24` | `u32` | Processing deadline misses |
| `28,30` | `u16,u16` | Block p99 and maximum microseconds |
| `32,34` | `u16,u16` | Resampler p99 and maximum microseconds |
| `36,38` | `u16,u16` | Opus p99 and maximum microseconds |
| `40` | bitset | Saturation flags for block, resampler, and Opus timing |

Timing values saturate at `65535` microseconds rather than wrapping. Saturation bits 0, 1, and 2 correspond to the three timing families. The diagnostic getter masks interrupts only while copying thirteen aligned counters/sample/max scalars. Its bucketed p99 estimates scan the single-writer cumulative histograms afterwards in low-priority context; they can include one newer audio sample without delaying the realtime producer. The existing full statistics API remains available to non-realtime callers for a strictly consistent export.

### Page 4: audio pipeline (`data_len=41`)

| Offset | Type | Meaning |
| --- | --- | --- |
| `0` | `u32` | Speaker PCM blocks queued from USB |
| `4` | `u32` | Encoded audio pairs submitted to BT |
| `8` | `u32` | Microphone ring underruns |
| `12` | `u32` | Microphone ring overruns |
| `16` | `u32` | PCM blocks dropped |
| `20` | `u32` | PCM pool starvations |
| `24` | `u32` | PCM queue overruns |
| `28` | `u32` | Stale PCM blocks discarded |
| `32` | `u32` | PCM ready-queue high-water mark |
| `36` | `u32` | Microphone endpoint write errors |
| `40` | bitset | Bit 0 speaker active, bit 1 microphone active |

### Page 5: OTA and memory (`data_len=43`)

| Offset | Type | Meaning |
| --- | --- | --- |
| `0` | `u32` | Current free bytes reported by the internal Bouffalo SDK memory manager |
| `4` | `u32` | Minimum free bytes observed by the 1 Hz diagnostic snapshots since boot |
| `8` | `u8` | OTA state, or `0xFF` if the OTA snapshot failed validation |
| `9` | `u8` | OTA error, or `0xFF` if invalid |
| `10` | `u8` | Last OTA request |
| `11` | bitset | OTA status flags |
| `12` | `u32` | OTA session |
| `16` | `u32` | Accepted file offset |
| `20` | `u32` | Committed flash offset |
| `24` | `u32` | Total update size |
| `28` | `u32` | Maximum accepted update size |
| `32` | `u32` | OTA capability bits |
| `36` | `u32` | Installed PSRAM bytes, zero on boards without PSRAM |
| `40` | `u8` | Active OTA slot |
| `41` | `u8` | Boot2 trial-retry byte |
| `42` | bitset | Memory/status validity flags |

Memory/status bit 0 means the internal SDK memory-manager values are valid, bit 1 means PSRAM is present, bit 2 means PSRAM is intentionally excluded from the realtime allocator, and bit 3 means the copied OTA status passed its own CRC and protocol checks. The current free-byte value aggregates the active internal heaps selected by `kfree_size(0)`; the minimum is the lowest of those 1 Hz task-context samples since diagnostics initialization, not an allocator-event minimum. Neither value includes the reserved PSRAM diagnostic buffers. OTA states, errors, status flags, and capabilities retain the values documented in [OTA.md](OTA.md).

### Page 6: M61 bridge latency (`data_len=24`)

| Offset | Type | Meaning |
| --- | --- | --- |
| `0` | `u32` | Completed timed input transfers in the most recent diagnostic window |
| `4,6` | `u16,u16` | Bluetooth HID callback entry to USB IN submission, average and maximum µs |
| `8,10` | `u16,u16` | USB IN submission to transfer-complete callback, average and maximum µs |
| `12` | `u16` | Bluetooth callback entry to USB completion, average µs |
| `14,16` | `u16,u16` | Total bridge-latency P95 and P99 upper bounds in µs |
| `18` | `u16` | Total bridge-latency maximum in µs |
| `20` | `u16` | Diagnostic publish/window interval in milliseconds |
| `22` | bitset | Saturation: bit 0 receive/submit, bit 1 USB transfer, bit 2 total |
| `23` | `u8` | Bridge-latency schema version, currently `1` |

This page measures only controller input reports that complete a USB IN transfer. The start timestamp is taken at the firmware's Bluetooth HID input callback, after lower radio/L2CAP processing; the endpoint timestamp is the M61 USB transfer-complete callback, before Windows application scheduling. Reports replaced by the depth-one latest-state queues are visible through the traffic/coalescing counters on page 1 but do not become latency samples. P95/P99 are realtime-safe histogram upper bounds, and all microsecond fields saturate at `65535` with the corresponding flag set.

## Runtime cost and safety

USB, Bluetooth, audio, and OTA hot paths only capture monotonic timestamps and update fixed-size aligned counters/histograms. They never format pages, calculate diagnostic CRCs, or print periodic diagnostics. Once per second, the priority-1 `diag` task copies the counters, samples heap and link state, formats seven pages, computes seven CRCs, and publishes one bank index after a RISC-V memory fence. Level-3 aggregate logs also run from this task every ten seconds. The next update writes the other bank.

`runtime_diag_init()` prepares CRC-valid, zero-length sequence-0 pages before task startup. The optional task is allocated last, after bridge, audio, indicator, and OTA workers. If its 4 KiB stack cannot be allocated, boot emits one bounded error and continues the controller bridge; hosts keep receiving those sequence-0 pages instead of triggering work in EP0. `0xFD` remains available during OTA maintenance because its callbacks do not touch flash, hashes, queues, or the controller.
