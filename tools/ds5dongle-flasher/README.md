# DS5Dongle Windows Flasher

Single-file Windows GUI and CLI flasher and native test center for DS5DONGLE-AIM61. It prefers stable AI-M61 Full-Speed firmware while remaining compatible with other complete packages produced by this repository. It reads the running firmware version and the seven-page `0xFD` runtime diagnostic snapshot over USB HID, while retaining compatibility with legacy six-page firmware, and correlates M61 internal bridge latency with Windows HID jitter. It also includes native tests for inputs, touch, motion sensors, lights, rumble, adaptive triggers and USB audio, plus local JSON diagnostic export, CH340-only COM-port selection, GitHub Release downloads, local ZIPs/directories, SHA256 verification, BOOT+RESET guidance, and 460800/115200 baud retry.

See [`../../docs/FLASHER.md`](../../docs/FLASHER.md) for package, build, and release instructions.
