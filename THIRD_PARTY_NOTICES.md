# Third-party notices

This file records provenance; it does not replace the license text shipped by
each component.

## DS5Dongle lineage

- `awalol/DS5Dongle` — original project direction and DualSense bridge design;
  MIT License; copyright awalol and contributors.
- `ccc007ccc/DS5Dongle` — BL616/BL618/Ai-M61 port, audio and performance
  engineering inherited through repository history; MIT License in that
  history; copyright its contributors.
- `sqlCRT/ds5dongle-bl618-opensource` — principal open BL618 comparison
  baseline and source of later integration work; GNU GPL v3; copyright sqlCRT
  and contributors.
- `bibuq0/DSdongle-bl616` — GPL-3.0 comparison source for the v3.17 Q15
  resampler, delayed USB disconnect, reconnect primer volume flags and
  suspend-time BR/EDR scan silencing.  This project adapted those ideas to its
  existing asynchronous ownership and RV32 race-safety model.  The Electron
  companion, `silk_stubs.c`, CELT-only source pruning and whole-task TCM
  placement were deliberately not copied.

## Bouffalo SDK

- Official base: `bouffalolab/bouffalo_sdk`, SDK 2.3.31, commit
  `09abb06993d7aaa594648ecb6a3212c53a44f3da`, Apache License 2.0.
- Audited patch source: `sqlCRT/bouffalo_sdk`, commit
  `42c20811c613a3c1575cbe3492e045eed5eefe7c`, based on SDK 2.3.28,
  Apache License 2.0.
- Adopted patches: AIM61 flash-clock fallback; CherryUSB audio compatibility;
  USB-v2 endpoint close and bounded VDMA recovery; HID `net_buf` pools;
  BR/EDR L2CAP send-completion callback; deterministic CMake cache reset.
- Not adopted: fork documentation/workflows, proprietary LP firmware archive,
  BL618DG-only ROM/PDS changes, unrelated examples.

The exact maintained patch set is in `sdk-patches/`.

## Opus

Opus 1.5.2 source subset from Xiph.Org, BSD 3-Clause License. Local E907
fixed-point fast paths are required to remain bit-exact; fast-math is not used.

## CherryUSB and embedded tools

CherryUSB is consumed through Bouffalo SDK under Apache-2.0. The Windows
flasher embeds Bouffalo Lab's `BLFlashCommand` from the pinned SDK; its digest
is checked at build and runtime. WCH CH340/CH341 driver downloads remain WCH
software and are verified by pinned SHA-256 and Windows signer identity before
installation.

## ds.evua.cc reference behavior

The public `ds.evua.cc` controller tester was inspected as a behavioral
reference for DualSense output reports. This project independently stores only
small interoperability facts and test vectors required to reproduce behavior:

- USB HID output report `0x02` field layout for rumble, lights and adaptive
  triggers;
- Feature report `0x80` wave-output start/stop sequences for speaker/headset;
- safe volume transitions and output reset order.

No site HTML, branding or bundled script is redistributed, and the application
does not contact the site at runtime. The site's public implementation only
toggles the microphone indicator; this tool additionally tests actual audio
capture through the M61 Windows UAC endpoint and clearly identifies that as an
extension.

## DualSense tester and calibration references

- `daidr/dualsense-tester`, commit
  `d85bbaf2cf6ade22aae3983f22c99a176e50c827` — DualSense input, touch,
  motion, output and audio-test interaction reference. MIT License; copyright
  (c) 2023 Xuezhou Dai (daidr).
- `dualshock-tools/dualshock-tools.github.io`, commit
  `af58465fac3447eafa7949b65f0ae650df74a554` — 48-direction stick sampling,
  RMS circularity-error definition and DS5 `0x82`/`0x83` center/range
  calibration protocol reference. MIT License; copyright (c) 2024 the_al.

The Windows application is a native Rust implementation and does not embed,
load or redistribute either web application. Small protocol facts, algorithms
and interaction patterns were adapted under their MIT terms. The required MIT
copyright and permission notices are retained below:

> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions: The above copyright
> notice and this permission notice shall be included in all copies or
> substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS",
> WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
> TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
> NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
> FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
> TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR
> THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Rust and Python dependencies

The native tool uses Rust crates listed in `tools/ds5dongle-flasher/Cargo.lock`;
their upstream license metadata governs those copies. Python packaging and OTA
utilities use Python's standard library plus OpenSSL invoked by CI. GitHub
Actions used in CI retain their respective licenses.
