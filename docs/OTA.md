# M61 网页 OTA 发布与传输规范

本项目的网页 OTA 只更新 **Ai-M61-32S-Kit / USB Full-Speed** 的应用固件。第一次部署仍必须通过串口完整刷写受支持的 `boot2`、`partition.bin` 和带 OTA 功能的应用；完成这次基线刷写后，网页才可以更新单个 RAW 应用镜像。

## 用户快速升级

1. 使用 Chrome 或 Edge 打开部署在 HTTPS/localhost 的 `web/` 页面。
2. 通过原生 USB 飞线连接 DS5Dongle，点击“连接设备”。板载 CH340 Type-C 只负责供电、串口日志和线刷，不能替代原生 USB 数据线。
3. 在 OTA 页面确认设备报告 `aim61`、`fs`、RAW OTA、A/B 分区、强制签名和已配置发布公钥。
4. 加载默认在线稳定版，网页会先验证清单、目标、版本、完整容器 SHA-256、RAW body SHA-256 和 P-256 签名。
5. 确认后开始升级。不要断电、拔线、关闭页面或让电脑休眠。
6. 设备写入非活动槽并重启；重新连接后确认版本。新固件未通过健康确认时，Boot2 应回滚旧槽。

默认稳定版清单来自当前仓库 latest Release。没有生产公钥、签名资产或正确 OTA 基线的设备必须拒绝升级；这种情况下请使用 Windows 刷写器执行完整 UART 恢复，而不是绕过校验。

## 绝对不能通过网页写入的内容

- `boot2_bl616_*.bin`
- `partition.bin`（包括两份分区表）
- 完整线刷 ZIP 或其中的任意地址配置
- High-Speed、其他开发板或超过 M61 备用 FW 槽的镜像

网页只接受后缀为 `.bin.ota` 的 Bouffalo RAW OTA 容器。容器由 512 字节 `BL60X_OTA` 头和应用 body 组成；M61 备用 FW 槽最多容纳 `0x168000`（1,474,560）字节的 RAW body。

## 发布清单

稳定版固定发布两个资产：

- `DS5Dongle-aim61-fs-v<version>.bin.ota`
- `DS5Dongle-aim61-fs-stable.ota.json`

Release 工作流只接受 `v?major.minor.patch` 标签：每部分为 0..254，且去掉可选 `v` 后的规范版本字符串最长 8 个 ASCII 字符。Bouffalo SDK 的 `ver_software[16]` 必须容纳 `EVENT_V<version>` 和终止 NUL；因此 `254.5.0` 可用，而 `254.254.254` 必须在构建前拒绝。工作流把同一个规范版本同时注入 `FIRMWARE_VERSION` 与 `PROJECT_SDK_VERSION`，SDK 生成的 OTA 头必须精确等于 `EVENT_Vmajor.minor.patch`，并与 manifest `version` 一致；发布工具在生成和验证阶段都会检查这条版本链。非 Release CI 构建使用 `3.5.0`。

网页可通过 GitHub 的 latest 别名读取清单：

```text
https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases/latest/download/DS5Dongle-aim61-fs-stable.ota.json
```

清单 schema 1 的字段集合是固定的，不允许缺失或附加字段：

```json
{
  "board": "aim61",
  "body_sha256": "<RAW body 的 64 位小写 SHA-256>",
  "body_size": 863216,
  "channel": "stable",
  "schema": 1,
  "sha256": "<整个 .bin.ota 的 64 位小写 SHA-256>",
  "signature": {
    "algorithm": "ECDSA-P256-SHA256",
    "key_id": "<发布公钥标识>",
    "scope": "DS5DONGLE-OTA-V1",
    "value": "<64 字节 r||s 的标准 Base64>"
  },
  "size": 863728,
  "url": "https://github.com/.../DS5Dongle-aim61-fs-v1.2.3.bin.ota",
  "usb_speed": "fs",
  "version": "1.2.3"
}
```

`version` 在清单中规范化为不带 `v` 的 `major.minor.patch`，每部分必须为 0..254，整体最多 8 个字符。只有 `dev` 清单可以把 `signature` 设为 `null`，且生成、验证和网页端都必须显式开启开发模式；`beta` 和 `stable` 必须签名。工具、网页和设备都必须 fail closed，不能在公钥缺失、`key_id` 不匹配或签名失败时继续更新。

仓库默认固件和网页 UI 都不接受未签名升级。只有隔离测试固件显式设置构建环境变量 `DS5_OTA_ALLOW_UNSIGNED_DEV=1`，并且调用网页协议层时显式传入 `allowUnsignedDev`，才可配合 `--allow-unsigned-dev` 清单；该组合会打印不安全警告，绝不能作为串口基线或发布资产。

### 签名字节

签名不是对 JSON 排版结果签名，而是先对下列恰好 57 字节的 canonical 做 SHA-256，再用 ECDSA secp256r1（P-256）签名。这样网页和 BL618 固件无需实现相同的 JSON canonicalization：

```text
ASCII "DS5DONGLE-OTA-V1"（恰好 16 字节，无 NUL）
board_id:u8（aim61 = 1）
usb_speed:u8（FS = 0，HS = 1）
semver_major:u8 || semver_minor:u8 || semver_patch:u8
body_size:u32 little-endian
body_sha256:32 raw bytes
```

字段顺序固定，无换行、无 NUL、无终止换行。签名在 manifest 中编码为固定 64 字节 `r || s`，其中 r、s 各为 32 字节大端整数，并使用 low-S 形式。`size`/`sha256` 用于浏览器校验整个下载文件，设备实际执行所需的 board、speed、版本、body 长度和 body SHA 则由签名绑定；设备还必须核对 RAW 外头与这些签名元数据一致。

网页可使用部署变量注入 65 字节 SEC1 uncompressed P-256 公钥（`04 || X || Y`，Base64）和允许的 `key_id`；固件中也必须固化同一个生产公钥。仓库不提供、也不伪造生产密钥。PEM 公钥的部署形态如下，尖括号内容必须替换为真实公钥编码：

```pem
-----BEGIN PUBLIC KEY-----
<BASE64_ENCODED_P256_SUBJECT_PUBLIC_KEY_INFO>
-----END PUBLIC KEY-----
```

生成真实密钥时让私钥保持离线，绝不能提交到仓库：

```bash
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out ota-p256-private.pem
openssl pkey -in ota-p256-private.pem -pubout -out ota-p256-public.pem
```

将 PEM 公钥严格验证并导出为固件/网页使用的 65 字节 SEC1 `04||X||Y`（工具会解析 SPKI 的 `id-ecPublicKey` 与 `prime256v1` OID，不会盲目截取文件尾部）：

```powershell
python tools\ota_release.py export-public-key `
  --public-key ota-p256-public.pem `
  --output ota-p256-public-65b.bin
```

`ota-p256-public-65b.bin` 必须 Base64 后作为同一份信任根提供给网页和固件；私钥不能执行这一部署步骤，更不能复制到源码树或网页资产中。GitHub 仓库需要配置：

- 加密 Secret `OTA_P256_PRIVATE_KEY_B64`：发布私钥 PEM 的 Base64，仅发布任务可读。
- Repository variable `OTA_P256_PUBLIC_KEY_SEC1_B64`：上述 65 字节 `04||X||Y` 文件的 Base64。Release 固件矩阵会严格解码、检查长度与 `0x04` 前缀，再通过 `DS5_OTA_PUBLIC_KEY_FILE` 注入；缺失时 Release 构建直接失败。
- Repository variable `OTA_P256_KEY_ID`：1..64 个 `[0-9A-Za-z._-]` 字符的公钥标识。
- 网页部署变量 `NEXT_PUBLIC_OTA_P256_PUBLIC_KEY` 与 `NEXT_PUBLIC_OTA_KEY_ID`：分别使用相同的 SEC1 Base64 与相同 `key_id`。

在 PowerShell 中生成两项 Base64 配置值时不要经文本编码转换：

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("ota-p256-private.pem"))
[Convert]::ToBase64String([IO.File]::ReadAllBytes("ota-p256-public-65b.bin"))
```

发布任务还会从私钥重新导出 SEC1 公钥，并与 `OTA_P256_PUBLIC_KEY_SEC1_B64` 做逐字节比较后才签清单；因此“固件内公钥、网页公钥、清单 `key_id`、签名私钥”不能静默错配。runner 只在临时目录中使用私钥，发布资产不包含它。普通 PR/push 构建不要求生产变量，生成的固件保持 `KEY_CONFIGURED=0` 并对签名 OTA fail closed，不能拿来作为可 OTA 的正式基线。

## 发布工具

检查 SDK 生成的 RAW 容器（类型字段必须为四字节 `RAW `，末尾是空格）：

```powershell
python tools\ota_release.py inspect `
  --image build\build_out\ds5dongle_bl618_bl616.bin.ota
```

工具会校验 512 字节头、头内 body 长度、头内 body SHA-256、M61 备用槽上限，并另外计算整个 `.bin.ota` 的发布 SHA-256。

生成可复现的未签名开发清单：

```powershell
python tools\ota_release.py generate `
  --image build\build_out\ds5dongle_bl618_bl616.bin.ota `
  --manifest dist\DS5Dongle-aim61-fs-dev.ota.json `
  --channel dev --version v1.2.3 --allow-unsigned-dev `
  --url https://downloads.example.test/DS5Dongle-aim61-fs-v1.2.3.bin.ota
```

生成稳定版必须提供真实私钥和 `key_id`：

```powershell
python tools\ota_release.py generate `
  --image dist\DS5Dongle-aim61-fs-v1.2.3.bin.ota `
  --manifest dist\DS5Dongle-aim61-fs-stable.ota.json `
  --channel stable --version v1.2.3 `
  --url https://github.com/example/project/releases/download/v1.2.3/DS5Dongle-aim61-fs-v1.2.3.bin.ota `
  --private-key C:\secure\ota-p256-private.pem --key-id release-2026
```

验证清单、镜像和签名：

```powershell
python tools\ota_release.py verify `
  --image dist\DS5Dongle-aim61-fs-v1.2.3.bin.ota `
  --manifest dist\DS5Dongle-aim61-fs-stable.ota.json `
  --public-key ota-p256-public.pem --expected-key-id release-2026
```

签名和验证调用 OpenSSL 的 ECDSA P-256 实现，并要求 OpenSSL 3.5 或更新版本的 provider 支持 RFC 6979 deterministic nonce（`nonce-type:1`）；相同镜像、版本、URL、密钥和 `key_id` 会生成逐字节相同的清单。工具负责 DER 与固定 64 字节 raw `r||s` 的严格转换；找不到或版本过旧的 OpenSSL 时工具直接失败，也可以通过 `--openssl <路径>` 指定可执行文件。Release 发布任务固定使用带 OpenSSL 3.5+ 的 Windows Runner，不能改回仍只有 OpenSSL 3.0 的 `ubuntu-24.04`。

## WebHID 帧协议 v1

WebHID API 的 report ID 与 63 字节 payload 分开传递：

- `0xFA` 是 Output DATA report，网页使用 `sendReport(0xFA, payload)`。
- `0xFC` 是 Feature CONTROL/STATUS report，网页使用 `sendFeatureReport` 和 `receiveFeatureReport`。

两种 payload 都是固定 63 字节：

| 字节 | 长度 | 含义 |
|---|---:|---|
| 0..1 | 2 | ASCII `OT` |
| 2 | 1 | 协议版本 `1` |
| 3 | 1 | DATA type 或 CONTROL opcode |
| 4..7 | 4 | session，uint32 little-endian |
| 8..11 | 4 | DATA offset 或控制参数，uint32 little-endian |
| 12 | 1 | data length，最大 46 |
| 13..58 | 46 | data，未使用部分必须补零 |
| 59..62 | 4 | CRC-32/ISO-HDLC little-endian，覆盖字节 0..58 |

DATA type 固定为 `0x10`。CONTROL opcode 是 `BEGIN=0x01`、`AUTH=0x02`、`COMMIT=0x03`、`ABORT=0x04`、`STATUS=0x05`、`ACK=0x80`、`ERROR=0xFF`。

`BEGIN` 的 argument 是完整 `.bin.ota` 大小，data length 固定为 42：

| BEGIN data 偏移 | 长度 | 含义 |
|---|---:|---|
| 0 | 1 | board ID，M61=`1` |
| 1 | 1 | USB speed，FS=`0` |
| 2..4 | 3 | semver major/minor/patch |
| 5 | 1 | flags；stable signed=`bit0` |
| 6..9 | 4 | RAW body size，uint32 LE |
| 10..41 | 32 | RAW body SHA-256 |

`AUTH` 只传固定 64 字节 P-256 raw `r||s`：第一帧 argument=`0`、len=`46`，第二帧 argument=`46`、len=`18`。设备用 BEGIN 元数据自行重建 57 字节 canonical，核对 RAW 头后使用固化的生产公钥验签；网页不能用自报 `key_id` 改变设备信任根。

网页使用 `receiveFeatureReport(0xFC)` 直接读取设备已发布的状态快照，不需要在每次轮询前再发送 `STATUS`，否则会无意义地占用控制队列。空闲快照的 session=`0`，可用于能力查询；传输期间快照带当前 session。设备返回 `ACK=0x80` 或 `ERROR=0xFF`，argument 是设备已接受的连续传输 offset，data length 固定为 44。`STATUS=0x05` 无 data 的控制帧仅保留作显式诊断/兼容请求，不是正常轮询的前置步骤：

| STATUS data 偏移 | 长度 | 含义 |
|---|---:|---|
| 0 | 1 | update state |
| 1 | 1 | error code |
| 2 | 1 | last request opcode |
| 3 | 1 | status flags |
| 4..7 | 4 | committed offset，uint32 LE |
| 8..11 | 4 | complete OTA size |
| 12..15 | 4 | inactive slot 最大完整 OTA size |
| 16..19 | 4 | capability bits |
| 20..27 | 8 | board、speed、format、max data、window、active slot、trial retry、reboot delay |
| 28..43 | 16 | 当前固件版本，ASCII/NUL padded |

`capability bits` 当前定义为：bit0 A/B 槽、bit1 SHA-256、bit3 trial boot、bit4 maintenance、bit5 RAW OTA、bit6 帧 CRC32、bit7 延迟重启、bit8 强制签名、bit9 已配置发布公钥。bit2 `LIVE_RESUME` 保留但当前必须为 0；v1 的 DATA offset 只用于当前 session 内的顺序/落盘确认，不承诺页面重载、USB 断线或跨 session 续传。

错误码 0..19 依次为：OK、BAD_MAGIC、BAD_VERSION、BAD_OPCODE、BAD_STATE、BAD_SESSION、BAD_OFFSET、BAD_LENGTH、BAD_CRC、BAD_TARGET、TOO_LARGE、QUEUE_FULL、FLASH、HASH、HEADER、INTERNAL、AUTH_REQUIRED、AUTH_FAILED、KEY_MISSING、TIMEOUT。正在进行的 session 连续 60 秒无活动时，设备以 `TIMEOUT=19` 中止并退出维护态；网页必须重新查询状态并用新 session 从 `BEGIN` 开始，不能沿用旧 offset。

协议固定向量（每行均为不含 report ID 的 63 字节 payload）如下；固件、Python 和网页测试必须逐字节一致：

```text
BEGIN  = 4f54010178563412f02b0d002a010001020301f0290d00b0d51c58c8b9c1f458fadf16c7d375630ef51da4df81915893b05c0fa4ed8bc600000000f9c2342b
AUTH0  = 4f54010278563412000000002e000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2dea3513bc
AUTH46 = 4f540102785634122e000000122e2f303132333435363738393a3b3c3d3e3f00000000000000000000000000000000000000000000000000000000ae93fa7f
ACK    = 4f54018078563412401000002c0300020100100000f02b0d0000821600fb0300000100012e1000f20f424c3631382d44533520332e350000000000f75742f8
ERROR  = 4f5401ff78563412001000002c0611030100100000f02b0d0000821600fb0300000100012e1000f20f424c3631382d44533520332e350000000000ef54ddf6
```

推荐流程：

1. 读取 `STATUS`，确认设备是 `aim61/fs`、协议版本匹配且当前不在升级。
2. `BEGIN` 发送总长度、目标、semver、签名 flag、RAW body 长度和 body SHA。
3. stable 更新用两个 `AUTH` 帧发送 64 字节 P-256 raw `r||s`；设备重建 canonical 并验签成功后才进入接收状态。
4. 用 `0xFA` 发送最多 46 字节的数据块。offset 是幂等序号；设备只确认连续写入的下一个 offset，重试同一 offset 不得重复推进。
5. 周期性 GET `ACK/ERROR` 状态快照做背压和当前 session 的落盘进度确认，不为每次轮询额外排队 `STATUS` 命令。浏览器不得假定 USB 发送成功等于 Flash 已落盘；USB 断线、页面重载或 60 秒设备超时后必须用新 session 从 `BEGIN` 重启传输。
6. `COMMIT` 后设备依次校验 BL60X RAW 头、已签名的 body 长度/body SHA 元数据和 stable P-256 签名；全部通过后才能切换启动分区。整个 `.bin.ota` 的 size/SHA 是浏览器下载完整性检查，不冒充设备侧签名校验。
7. 重启并重新连接，读取新版本；新固件健康确认前保留旧槽。

参考编码器与跨语言固定向量位于 `tools/ota_protocol.py` 和 `tools/test_ota_protocol.py`。

## 实机发布门槛

首次完整线刷后，至少完成以下 M61 断电/回滚测试才可开启 stable：

- 在擦除前、写入约 1%、25%、50%、99% 和验证后切换前分别断电，旧固件仍能启动。
- 对 DATA 帧做丢包、重复、乱序和 CRC 错误注入，设备只能确认连续且已写入的 offset。
- 写入错误 body、错误 header SHA、错误 P-256 key/签名和降级版本，设备必须拒绝切换；错误完整文件 SHA 或 `key_id` 必须先被网页拒绝。
- 新槽启动后主动触发崩溃/看门狗，Boot2 必须回滚旧槽；健康运行后再提交确认，之后不能误回滚。
- 更新期间断开手柄并暂停 USB/蓝牙音频，确认 Flash 擦写不会破坏 DualSense 实时链路或 USB 控制传输。
- OTA 不能擦除 PSM/KEY/DATA，蓝牙配对和网页配置在成功升级及回滚后都应保留。

如果两槽或 Boot2/分区表已经损坏，网页 OTA 不能救援，必须回到串口完整刷写。
