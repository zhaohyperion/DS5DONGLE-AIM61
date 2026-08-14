# Bouffalo SDK patch set

Firmware v3.5.2 builds from the official `bouffalolab/bouffalo_sdk` commit
`09abb06993d7aaa594648ecb6a3212c53a44f3da` (SDK 2.3.31).  The patch files in
this directory contain only the DS5Dongle-relevant changes audited from
`sqlCRT/bouffalo_sdk` commit
`42c20811c613a3c1575cbe3492e045eed5eefe7c`.

Apply in lexical order with `git apply --check --recount`, then
`git apply --recount`. CI refuses to build if the SDK commit differs or if any
patch no longer applies cleanly. `--recount` is required because this audited
multi-file patch is intentionally kept compact without mail-format metadata.

Included:

- verified flash-clock fallback needed by BL618/AIM61 PSRAM initialization;
- CherryUSB audio control compatibility and endpoint-close/VDMA recovery;
- dedicated HID buffer pools and BR/EDR L2CAP completion callback;
- deterministic CMake cache reset used by profile/speed matrix builds.

Excluded:

- documentation and GitHub workflow changes from the fork;
- proprietary LP firmware archives;
- BL618DG-only ROM/PDS changes and unrelated examples.

The fork patch remains copyright its contributors and is used under the
upstream Bouffalo SDK Apache-2.0 license.  See `THIRD_PARTY_NOTICES.md`.
