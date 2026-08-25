# M61 签名 A/B OTA 规范（协议 v2）

本规范只适用于 **Ai-M61-32S-Kit / BL618 / USB High-Speed** 的 v3.6.2 常用版和诊断版。刷写器 v1.4.2 已实现原生 OTA 客户端，可在“固件刷写”页执行 `standard ↔ diagnostic` 双向切换。公开配置网站和网页 OTA 已下线。

第一次部署、Boot2/分区表损坏或两槽均不可启动时，必须使用 CH340 UART 完整刷写。OTA 只更新应用 RAW 镜像，绝不能写入 Boot2、分区表、整片 Flash、PSM/KEY/DATA 或其他板卡的镜像。

## 存储与性能边界

A/B 槽位位于板载 SPI Flash，不在 PSRAM。备用 FW 槽允许的 RAW body 最大为 `0x168000`（1,474,560）字节。PSRAM 断电即失且不是可信启动介质，不能保存可启动 OTA 槽。

空闲时 OTA 任务阻塞等待队列，不产生持续 Flash 写入。传输时进入维护态，按 4 KiB SDK slice 写备用槽、计算 SHA-256、验证签名和头部；完成后由 Boot2 试运行/回滚机制接管。配对、映射和设置分区不随配置切换清除。

## Release 资产

v3.6.2 Release 只公开：

- `DS5Dongle-aim61-hs-standard-uart-full-v3.6.2.zip`；
- `DS5Dongle-aim61-hs-standard-ota-v3.6.2.zip`；
- `DS5Dongle-aim61-hs-diagnostic-uart-full-v3.6.2.zip`；
- `DS5Dongle-aim61-hs-diagnostic-ota-v3.6.2.zip`；
- `DS5Dongle-Flasher-Windows-v1.4.2.exe`；
- `SHA256SUMS.txt`。

`uart-full` 与 `ota` 是用途互斥的纯包。UART 包只包含 Boot2、分区表、应用 `.bin`、`firmware.json` 和校验表；OTA 包只包含一个 `.bin.ota`、对应 `.ota.json` 和校验表。刷写器拒绝把 OTA 文件混入完整刷写包，也拒绝 OTA 包携带 Boot2、分区表或普通应用 `.bin`。裸 OTA 文件不单独公开。

“固件刷写”页也允许手动选择本地签名 OTA ZIP。本地文件执行与在线包相同的结构、目标、hash 和电脑端签名校验；用户确认执行前会重新读取并校验归档 SHA-256，防止选择后文件被替换。未签名包、错误密钥、非 AIM61 HS、多个 OTA 清单、清单/正文不一致或混入 UART 文件均在传输前拒绝。

## RAW OTA 容器

Bouffalo SDK 输出：

```text
0..15   "BL60X_OTA_Ver1.0"
16..19  "RAW "
20..23  body_size, uint32 LE
32..47  hardware version
48..63  "EVENT_V3.6.2\0"
64..95  SHA-256(body)
512..   application body
```

工具拒绝非 RAW、长度不一致、正文为空、超过备用槽、头部 hash 错误或版本链不一致的输入。

## OTA 清单

```json
{
  "schema": 1,
  "channel": "beta",
  "board": "aim61",
  "usb_speed": "hs",
  "profile": "standard",
  "version": "3.6.2",
  "size": 906832,
  "sha256": "<完整 .bin.ota SHA-256>",
  "body_size": 906320,
  "body_sha256": "<正文 SHA-256>",
  "url": "https://github.com/.../DS5Dongle-aim61-hs-standard-ota-v3.6.2.zip",
  "signature": {
    "algorithm": "ECDSA-P256-SHA256",
    "key_id": "local-m61-2026",
    "scope": "DS5DONGLE-OTA-V2",
    "value": "<Base64 raw r||s, 64 bytes>"
  }
}
```

`channel` 只能是 `dev/beta/stable`。正式和预发布资产都必须签名；未签名只允许隔离开发固件在 `channel=dev` 且显式启用不安全构建开关时使用，不能作为公开基线。

## P-256 签名 canonical

签名输入固定为 58 字节：

```text
"DS5DONGLE-OTA-V2"             16 bytes, no NUL
board_id                         1 byte, AIM61=1
usb_speed                        1 byte, FS=0 / HS=1
profile                          1 byte, Standard=0 / Diagnostic=1
semver                           3 bytes
body_size                        4 bytes LE
SHA-256(body)                   32 bytes
```

签名是 ECDSA P-256/SHA-256 的 raw `r||s`，要求 canonical low-S。配置进入 canonical，因此不能把 Standard 的签名套到 Diagnostic，也不能签名后修改板型、速度、版本、长度或正文 hash。Windows 工具内置发布公钥并使用完全相同的 58 字节 canonical 在传输前验签，固件端仍独立验签，任一侧失败都会中止。

发布配置：

- Secret `OTA_P256_PRIVATE_KEY_B64`：私钥 PEM 的 Base64；
- Variable `OTA_P256_PUBLIC_KEY_SEC1_B64`：65 字节未压缩 `04||X||Y` 公钥的 Base64；
- Variable `OTA_P256_KEY_ID`：1..64 个 `[0-9A-Za-z._-]` 字符。

Release 工作流从私钥重新导出公钥并与配置公钥逐字节比较，再为两种配置分别签名。Release 构建缺少公钥时直接失败；普通 PR 构建不持有生产私钥。

## HID 传输

报告 ID 不计入以下 63 字节 payload：

- Output Report `0xFA`：DATA；
- Feature Report `0xFC` SET：BEGIN/AUTH/COMMIT/ABORT；
- Feature Report `0xFC` GET：ACK/ERROR 状态快照。

通用帧：

| payload offset | size | 内容 |
|---:|---:|---|
| 0..1 | 2 | `OT` |
| 2 | 1 | protocol=`2` |
| 3 | 1 | opcode |
| 4..7 | 4 | session, uint32 LE |
| 8..11 | 4 | 参数或 DATA offset |
| 12 | 1 | data length，0..46 |
| 13..58 | 46 | data，余下清零 |
| 59..62 | 4 | CRC32/IEEE(payload 0..58), LE |

操作码：`BEGIN=0x01`、`AUTH=0x02`、`COMMIT=0x03`、`ABORT=0x04`、`STATUS=0x05`、`DATA=0x10`、`ACK=0x80`、`ERROR=0xFF`。

BEGIN data 固定 43 字节：

| offset | size | 内容 |
|---:|---:|---|
| 0 | 1 | board=1 |
| 1 | 1 | speed=1（HS） |
| 2..4 | 3 | semver |
| 5 | 1 | flags，bit0=SIGNED |
| 6..9 | 4 | body size, LE |
| 10..41 | 32 | body SHA-256 |
| 42 | 1 | target profile，0/1 |

AUTH 把 64 字节 raw 签名拆成 `offset=0,len=46` 和 `offset=46,len=18` 两帧。

GET `0xFC` 返回 `len=44` 状态数据，frame argument 是 `accepted_offset`：

| data offset | size | 内容 |
|---:|---:|---|
| 0 | 1 | state |
| 1 | 1 | error |
| 2 | 1 | last request |
| 3 | 1 | flags |
| 4..7 | 4 | committed offset |
| 8..11 | 4 | total OTA size |
| 12..15 | 4 | max file size |
| 16..19 | 4 | capabilities |
| 20 | 1 | board |
| 21 | 1 | speed |
| 22 | 1 | format |
| 23 | 1 | max DATA bytes |
| 24 | 1 | window frames |
| 25 | 1 | active slot |
| 26 | 1 | trial retry |
| 27 | 1 | reboot delay（100 ms） |
| 28..43 | 16 | current version |

状态：`IDLE=0`、`AUTHORIZING=1`、`PREPARING=2`、`RECEIVING=3`、`VERIFYING=4`、`READY_REBOOT=5`、`ERROR=6`、`ABORTED=7`。

能力 bit：0 A/B、1 SHA-256、3 trial boot、4 maintenance、5 RAW、6 frame CRC32、7 delayed reboot、8 signature required、9 key configured。bit2 live resume 预留且当前为 0；断线后必须新建 session 从 BEGIN 开始。

错误码 0..19：OK、BAD_MAGIC、BAD_VERSION、BAD_OPCODE、BAD_STATE、BAD_SESSION、BAD_OFFSET、BAD_LENGTH、BAD_CRC、BAD_TARGET、TOO_LARGE、QUEUE_FULL、FLASH、HASH、HEADER、INTERNAL、AUTH_REQUIRED、AUTH_FAILED、KEY_MISSING、TIMEOUT。

## 主机状态机

1. GET STATUS，确认 IDLE、AIM61、HS、协议 v2、最大文件长度、强制签名且已配置公钥。
2. 生成非零 session，发送 BEGIN；等待 AUTHORIZING。
3. 发送两帧 AUTH；等待 RECEIVING。设备在任务上下文验证 P-256 并准备备用槽。
4. 按设备窗口发送最多 46 字节 DATA，批次后轮询 `accepted_offset`，不让 USB 回调队列溢出。
5. 全部接受后发送 COMMIT。设备提交末尾 slice、核对完整/正文 hash、RAW 头和版本，标记试运行槽。
6. 等待 READY_REBOOT 或计划内 USB 断开；重启后重新读取 `0xF8` 的 `版本|配置`。
7. 任一步骤失败时尽力发送 ABORT；原活动槽不被替换。60 秒无活动自动 TIMEOUT。

同版本只允许配置不同的显式切换；同版本同配置被拒绝，版本降级被拒绝。v3.6.2 回退 v3.5.x 必须走 UART 完整刷写。v3.6.2 由签名 HID OTA 状态机统一执行版本/配置策略，不再同时启用 SDK 的纯语义版本检查；后者会误拒绝合法的同版本 `standard ↔ diagnostic` 切换。

## 固定协议向量

以下 payload 均为 63 字节（不含 HID report ID）：

```text
BEGIN  = 4f54020178563412f02b0d002b010101020301f0290d00b0d51c58c8b9c1f458fadf16c7d375630ef51da4df81915893b05c0fa4ed8bc6000000007c9724c8
AUTH0  = 4f54020278563412000000002e000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d4643ec95
AUTH46 = 4f540202785634122e000000122e2f303132333435363738393a3b3c3d3e3f0000000000000000000000000000000000000000000000000000000002e50556
ACK    = 4f54028078563412401000002c0300020100100000f02b0d0000821600fb0300000100012e1000f20f424c3631382d44533520332e3500000000005b21bdd1
ERROR  = 4f5402ff78563412001000002c0611030100100000f02b0d0000821600fb0300000100012e1000f20f424c3631382d44533520332e350000000000432222df
```

这些向量由 `tools/test_ota_protocol.py` 锁定。

## 发布与验证

```powershell
python tools\ota_release.py inspect --image build\build_out\ds5dongle_bl618_bl616.bin.ota
python tools\ota_release.py generate --image <image.bin.ota> --manifest <profile.ota.json> `
  --channel beta --version 3.6.2 --usb-speed hs --profile standard `
  --url https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases/download/v3.6.2/DS5Dongle-aim61-hs-standard-ota-v3.6.2.zip `
  --private-key <private.pem> --key-id <key-id>
python tools\ota_release.py verify --image <image.bin.ota> --manifest <profile.ota.json> `
  --public-key <public.pem> --expected-key-id <key-id>
python tools\package_ota_zip.py --image <image.bin.ota> --manifest <profile.ota.json> `
  --archive DS5Dongle-aim61-hs-standard-ota-v3.6.2.zip
```

发布前应验证：正常升级、Standard/Diagnostic 双向切换、传输中掉电、签名/hash/目标错误拒绝、试运行失败回滚、配对、映射、宏与设置保留、UART 救援。v3.6.2 预发布按用户要求不等待真机测试，但不能把“CI 通过”描述成“真机已验证”。
