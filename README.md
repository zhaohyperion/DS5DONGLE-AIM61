# DS5DONGLE-AIM61

基于 Bouffalo BL616/BL618 的 DualSense / DualSense Edge 无线转 USB 适配器。

它通过 Bluetooth Classic HID 连接手柄，再向电脑呈现为标准有线 DualSense USB 设备；支持按键、摇杆、触摸板、陀螺仪、自适应扳机、灯光、震动以及双向 USB Audio。

[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
![Target](https://img.shields.io/badge/default-Ai--M61--32S--Kit-orange)
![Opus](https://img.shields.io/badge/Opus-1.5.2-green)
![USB](https://img.shields.io/badge/USB-Full--Speed%20recommended-7952b3)

> [!IMPORTANT]
> 这是非官方开源项目，与 Sony Interactive Entertainment 无关。“DualSense”“DualSense Edge”和“PlayStation”均为其商标。项目模拟 Sony USB VID/PID，使主机将设备识别为有线手柄，请自行评估使用风险。

> [!WARNING]
> Ai-M61-32S-Kit 板载 Type-C 连接的是 CH340 串口，只用于供电、日志和烧录。手柄 USB 数据必须从 GPIO37/38 引出，不能只插板载 Type-C。

## 项目状态

| 项目 | 当前状态 |
|---|---|
| 默认硬件 | Ai-M61-32S-Kit / BL618 |
| 默认 USB 模式 | Full-Speed 12 Mbps，兼容性优先 |
| 可选 USB 模式 | High-Speed 480 Mbps |
| 固件开发版本 | `3.5.1`；正式 Release 版本由标签注入 |
| 音频编解码 | Opus 1.5.2，固定点，E907 DSP 快速路径 |
| Web OTA | Ai-M61 Full-Speed / High-Speed，P-256 签名、同速校验、A/B 分区、试运行回滚 |
| 本地普通构建 | 可串口刷写；未注入发布公钥时，生产 OTA 会安全拒绝 |

当前仓库以 Ai-M61 为主要目标，同时保留 LCTech BL616 与 Sipeed M0S Dock 的构建支持。

## 功能

- DualSense 与 DualSense Edge 自动识别
- Bluetooth Classic BR/EDR HID Host：扫描、配对、重连、SDP、L2CAP
- 完整输入透传：按键、摇杆、扳机、触摸板、IMU、电量
- 完整输出透传：震动、灯条、玩家灯、自适应扳机
- USB Audio Class 1.0 双向音频
  - 48 kHz USB 扬声器音频编码为 Opus 后发送到手柄
  - 手柄麦克风 Opus 解码后输出为 USB 麦克风
  - 音频通道驱动 HD Haptics
- 最多记忆 8 个已配对手柄，支持按键快速切换
- 配置、按键映射、遥测和运行期诊断
- 签名 A/B OTA、反降级、掉电保护和启动回滚
- Windows GUI/CLI 一键刷写器
- Ai-M61 4 MiB PSRAM 冷数据隔离，实时数据留在内部 SRAM/TCM

## 工作方式

```mermaid
flowchart LR
    DS5["DualSense / Edge"]
    BT["Bluetooth Classic HID"]
    M61["Ai-M61 / BL618"]
    USB["USB HID + UAC1"]
    HOST["Windows / Linux / macOS"]

    DS5 <-->|"Input · Output · Opus Audio"| BT
    BT <--> M61
    M61 <-->|"Gamepad · Speaker · Microphone"| USB
    USB <--> HOST
```

固件的实时路径采用有界队列和静态缓冲：USB 输入使用 latest-state 双缓冲，蓝牙输出按 game/audio/control 分类调度，音频缓冲使用代际编号避免旧数据跨连接或跨音频会话传播。

## 支持的开发板

| 开发板 | 芯片 | 原生 USB | 指示灯 | 默认支持 |
|---|---|---|---|---|
| **Ai-M61-32S-Kit** | BL618 QFN56 | GPIO37/38 飞线 | RGB + 白色 LED | 是 |
| LCTech BL616 | BL616 QFN32 | 板载 Type-C | 单蓝灯 | 是 |
| Sipeed M0S Dock | BL616 QFN32 | 板载 Type-C | 双红灯 | 是 |

不同开发板的 USB、LED、BOOT 和 PSRAM 配置在编译期由 [`src/board_config.h`](src/board_config.h) 选择。不要把其他板型的完整固件包刷入 Ai-M61。

## Ai-M61 接线

Ai-M61 板载 Type-C 继续用于供电、串口日志和 UART ISP 烧录；另外准备一条 USB 数据线，将数据线接到排针：

```text
Ai-M61 GPIO37 / USB_DM  ─── USB D-（通常为白线）
Ai-M61 GPIO38 / USB_DP  ─── USB D+（通常为绿线）
Ai-M61 GND              ─── USB GND（通常为黑线）
```

注意事项：

- 不要连接 USB 数据线的 5V 红线，除非已经确认供电拓扑不会形成反灌。
- D+、D- 尽量短并成对走线；High-Speed 对线材和接线质量要求明显更高。
- 第一次验证建议使用 Full-Speed 固件。

## 快速开始

### 1. 准备 SDK 和工具链

项目依赖带有 DS5Dongle 蓝牙/USB 修改的 Bouffalo SDK：

```bash
git clone https://github.com/sqlCRT/bouffalo_sdk.git ../bouffalo_sdk
git -C ../bouffalo_sdk checkout cf6adf74b374a0e485defa9c89610f3e7ffcc3ec
```

BL616/BL618 使用 T-Head 扩展指令集，需要 `riscv64-unknown-elf` T-Head 工具链。Windows 可使用：

```powershell
git clone https://gitee.com/bouffalolab/toolchain_gcc_t-head_windows.git "$env:USERPROFILE\Desktop\toolchain_gcc_t-head_windows"
```

也可以通过环境变量指定自定义路径：

```powershell
$env:BL_SDK_BASE = "C:\path\to\bouffalo_sdk"
$env:TOOLCHAIN_PATH = "C:\path\to\toolchain_gcc_t-head_windows"
```

### 2. 编译 Ai-M61 固件

Windows：

```bat
build_windows.bat rebuild   rem Ai-M61 Full-Speed，推荐
build_windows.bat both      rem 同时构建 Full-Speed 与 High-Speed
build_windows.bat build     rem 增量构建
build_windows.bat clean     rem 清理构建目录
```

需要采集详细但低扰动的诊断计数时，可临时构建日志级别 3；正式发布仍建议默认级别 2：

```bat
set DS5_LOG_LEVEL=3
build_windows.bat rebuild
```

macOS / Linux：

```bash
bash build_macos.sh rebuild
USB_SPEED=hs bash build_macos.sh rebuild
```

其他开发板：

```bash
BOARD_TYPE=lctech616 bash build_macos.sh rebuild
BOARD_TYPE=m0sdock bash build_macos.sh rebuild
```

构建参数：

| 环境变量 | 可选值 | 默认值 |
|---|---|---|
| `BOARD_TYPE` | `aim61` / `lctech616` / `m0sdock` | `aim61` |
| `USB_SPEED` | `fs` / `hs` | `fs` |
| `DS5_LOG_LEVEL` | `0`..`3` | `2` |
| `FIRMWARE_VERSION` | `X.Y.Z`，每段 0..254 | `3.5.1` |

主要输出位于：

```text
firmware/aim61/ds5dongle-aim61.bin
firmware/aim61/ds5dongle-aim61-hs.bin
firmware/aim61/boot2_bl616_*.bin
firmware/aim61/partition.bin
```

二进制和本地构建目录默认被 Git 忽略，应通过 GitHub Release 分发，不要直接提交到源码仓库。

### 3. 首次完整烧录

OTA 只更新应用分区。第一次安装必须通过 UART ISP 完整写入 Boot2、分区表和应用固件。

#### 使用 Windows 一键刷写器

1. 从 [Releases](https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases/latest) 下载 `DS5Dongle-Flasher-Windows.exe`。
2. Ai-M61 用板载 Type-C 连接电脑；如果没有 COM 口，可在刷写器内安装经过 SHA-256 和数字签名校验的 WCH CH340/CH341 官方驱动。
3. 按住 **BOOT**，短按 **RESET**（也可按住 BOOT 重新插线），然后松开 BOOT。
4. 打开刷写器，选择 `Ai-M61-32S-Kit`、`Full-Speed` 和对应 COM 口。
5. 选择在线 Release，或加载本地完整固件 ZIP/目录；确认目标板型和 USB 速度后开始刷写。
6. 成功后按 RESET 正常启动。若 460800 波特率失败，可在界面中改用 115200 重试。

刷写器会先验证包结构、文件名、尺寸和 SHA-256，再调用内置并校验过的 Bouffalo `BLFlashCommand`。不要给 Ai-M61 刷入 LCTech/M0S 包，也不要把 OTA `.bin.ota` 当作完整线刷包。

在本仓库发布第一个包含完整固件资产的 GitHub Release 之前，在线列表会为空；此时请按下文生成本地 ZIP，不要回退到旧仓库下载源。

常用 CLI 检查命令：

```powershell
.\DS5Dongle-Flasher-Windows.exe --list
.\DS5Dongle-Flasher-Windows.exe --list-releases --board aim61 --usb-speed fs
.\DS5Dongle-Flasher-Windows.exe --verify-release --release v3.5.1 --board aim61 --usb-speed fs
.\DS5Dongle-Flasher-Windows.exe --release v3.5.1 --board aim61 --usb-speed fs --port COM5 --dry-run
```

#### 从本地构建生成完整包

生成供刷写器使用的本地完整包：

```powershell
python tools/package_firmware.py `
  --board aim61 `
  --usb-speed fs `
  --version local `
  --firmware-dir firmware/aim61 `
  --output-dir dist
```

刷写器源码、完整 GUI/CLI 参数、包格式及自行构建方法见 [`tools/ds5dongle-flasher/`](tools/ds5dongle-flasher/) 和 [`docs/FLASHER.md`](docs/FLASHER.md)。

### 4. 配对手柄

1. 给开发板供电并连接 Ai-M61 原生 USB 飞线。
2. 手柄关机状态下，同时长按 **PS + Create**，直到灯条快速闪烁。
3. 固件会扫描、配对并自动保存链路密钥。
4. 主机应识别出有线 DualSense 或 DualSense Edge。

常用 BOOT 手势：

| 操作 | 功能 |
|---|---|
| 单击 | 切换到下一个已保存手柄 |
| 双击 | 断开当前手柄并扫描新手柄 |
| 长按约 3 秒 | 清空配对记录并重新扫描 |

## USB 模式与轮询率

| 固件模式 | USB 链路 | 特点 | 建议 |
|---|---:|---|---|
| Full-Speed | 12 Mbps | 线材和飞线兼容性最好 | 默认使用 |
| High-Speed | 480 Mbps | 更高 USB 轮询上限，对布线更敏感 | 硬件验证后使用 |

配置中的轮询模式对应约 250 Hz、500 Hz 和实时档；实际输入频率还受蓝牙链路、主机调度和手柄报告率限制。High-Speed 不会降低蓝牙本身的空口延迟。

## Web 配置与 OTA

[`web/`](web/) 提供基于 WebHID 的设备配置页面，支持 Chrome / Edge，必须运行在 HTTPS 或 `localhost` 安全上下文。

```bash
cd web
npm install
npm run dev
npm test
```

Web 功能包括：

- 设备配置读取、临时应用和 Flash 保存
- 灯光、音量、轮询率、休眠及音频选项
- 手柄按键映射
- 电量、RSSI、USB/蓝牙/音频运行期诊断
- Ai-M61 Full-Speed / High-Speed 签名 OTA

### 在线 OTA 使用条件

- 支持 **Ai-M61-32S-Kit + USB Full-Speed/High-Speed + RAW `.bin.ota`**；只允许 FS→FS、HS→HS，同一设备不能跨USB档位OTA。
- 设备必须先完整线刷本项目的 OTA 分区表、Boot2 和带相同发布公钥的基础固件。
- 原生 USB 的 D+/D-/GND 飞线必须稳定；升级期间不要断电、拔线或关闭页面。
- 网页根据设备能力从 latest Release 读取 `DS5Dongle-aim61-fs-stable.ota.json` 或 `DS5Dongle-aim61-hs-stable.ota.json`；也可手动填写其他受信任清单 URL。

用户升级流程：

1. 在 HTTPS 页面中点击连接设备，并选择当前 DS5Dongle。
2. 打开 OTA 页，读取设备版本和能力；目标必须显示与当前固件一致的 `Ai-M61 · FS/HS · RAW`。
3. 加载在线稳定版清单，等待网页完成清单、目标、容器哈希、固件体哈希和 P-256 签名校验。
4. 勾选升级确认后开始传输。固件写入非活动槽，升级期间会暂停实时蓝牙/音频业务。
5. 设备验证完成后重启；重新连接并确认新版本。试运行未确认或启动失败时，由 Boot2 回滚到旧槽。

在线稳定版不会因为“网页能下载文件”就自动可信。维护者必须在仓库配置 `OTA_P256_PRIVATE_KEY_B64` Secret，以及 `OTA_P256_PUBLIC_KEY_SEC1_B64`、`OTA_P256_KEY_ID` Repository variables；网页部署还必须注入相同信任根的 `NEXT_PUBLIC_OTA_P256_PUBLIC_KEY` 和 `NEXT_PUBLIC_OTA_KEY_ID`。普通本地构建没有生产密钥，会按 fail-closed 原则拒绝稳定版 OTA。

生产 OTA 使用 fail-closed 信任模型：

- 网页和设备端分别验证 P-256 ECDSA 签名。
- 同时校验容器和固件体 SHA-256。
- 只写非活动分区，完成校验后才切换启动槽。
- 新固件必须通过试运行确认，否则 Boot2 回滚。
- 普通本地构建不包含默认发布密钥，因此不会意外接受生产 OTA。

完整协议、密钥生成、发布命令、断电测试和回滚门槛见 [`docs/OTA.md`](docs/OTA.md)。生产私钥不得写入源码、网页公开环境变量、构建日志或 GitHub 仓库。

## 性能设计

- USB 任务保持最高应用优先级，端点回调只做有界复制与通知。
- 输入使用深度 1 latest-state 队列，拥塞时覆盖旧状态而不是累计延迟。
- 蓝牙输出采用有界分类调度：游戏状态 latest-wins，音频和控制报告单独限深。
- Opus 使用固定点 `RESTRICTED_LOWDELAY`、10 ms 帧、复杂度 0。
- E907 优化包含 CLZ、Q15/Q16 DSP 乘法和 2 的幂除法快速路径。
- USB DMA、PCM、麦克风环形缓冲、Opus 状态和 FreeRTOS 栈保留在内部 SRAM/TCM。
- Ai-M61 PSRAM 只存放蓝牙扫描结果和 Feature Report 等冷数据，不进入通用堆。

固件内置运行期统计，可通过 Web 或 [`tools/m61-diagnostics.ps1`](tools/m61-diagnostics.ps1) 导出。性能结论应以真实硬件抓取的 p99、丢包和队列高水位为准，而不是只看理论轮询率。

## 已知限制

- 一次只桥接一个活动手柄。
- Web OTA 只接受与当前设备USB档位一致的 Ai-M61 RAW OTA 镜像，不支持 FS/HS 交叉升级。
- Ai-M61 High-Speed 对 USB 飞线、接头和主机控制器更敏感。
- 键盘映射类型属于预留协议；当前固件只保证手柄到手柄的按键映射。
- 首次安装和 Boot2/分区表变更必须使用串口完整刷写。
- 本项目不承诺在 PlayStation 主机上工作，主要目标是 PC、Steam、SDL 和远程串流环境。

## 测试

Python 工具和协议测试：

```bash
python -m unittest discover -s tools -p "test_*.py"
```

Web：

```bash
cd web
npm run lint
npm test
```

Windows 刷写器：

```powershell
cargo fmt --manifest-path tools/ds5dongle-flasher/Cargo.toml -- --check
cargo test --manifest-path tools/ds5dongle-flasher/Cargo.toml
```

正式发布还应完成各板型 FS/HS 构建矩阵，以及真实硬件上的配对、重连、音频、待机恢复、掉电 OTA 和回滚测试。

## 项目结构

```text
src/                         BL616/BL618 固件
lib/opus/                    Opus 1.5.2 固件所需源码子集
firmware/                    板型刷写配置与本地输出目录
tools/ds5dongle-flasher/     Windows GUI/CLI 刷写器
tools/ota_*.py               OTA 协议、签名和发布工具
tools/m61-diagnostics.ps1    运行期诊断采集工具
web/                         WebHID 配置与 OTA 页面
docs/                        OTA、诊断和刷写器详细文档
.github/workflows/           构建与发布自动化
```

## 来源、演进与致谢

本项目不是从零开始的独立实现。为避免把“历史参考”“直接代码来源”和“当前外部依赖”混为一谈，完整沿革如下。

### 项目沿革与贡献

| 来源 / 贡献者 | 与本项目的关系 |
|---|---|
| [awalol/DS5Dongle](https://github.com/awalol/DS5Dongle) | 最初的 Raspberry Pi Pico 2W 实现；提供 DualSense 蓝牙 HID 到 USB 的核心架构、协议处理和产品方向。上游为 MIT 许可。 |
| [ccc007ccc/DS5Dongle](https://github.com/ccc007ccc/DS5Dongle) | 本仓库直接继承的 BL616/BL618/Ai-M61 开发主线，包含板卡移植、实时音频、E907 优化、刷写器与大量稳定性工程；原提交历史已完整保留。该历史版本为 MIT 许可，Copyright (c) 2026 awalol and contributors / ccc007ccc。 |
| [sqlCRT/ds5dongle-bl618-opensource](https://github.com/sqlCRT/ds5dongle-bl618-opensource) | BL618 开源发布、三板型支持和后续功能基线；当前代码在此类 BL618 实现上继续整合 OTA、诊断、Web 配置和工程化改进。上游项目采用 GPL-3.0。 |
| [sqlCRT/bouffalo_sdk](https://github.com/sqlCRT/bouffalo_sdk) | 当前构建所需的 Bouffalo SDK 分支，基于 BouffaloSDK v2.3.28，补充了 DS5Dongle 所需的 Bluetooth HID `net_buf`、USB Audio 与刷写工具适配；本仓库固定使用提交 `cf6adf74b374a0e485defa9c89610f3e7ffcc3ec`。 |
| [zhaohyperion/DS5DONGLE-AIM61](https://github.com/zhaohyperion/DS5DONGLE-AIM61) | 当前维护仓库：以 Ai-M61 为默认目标，承接固件、Windows 刷写器、签名 A/B OTA、WebHID 配置、诊断与发布流程。 |

当前 Opus 基线是官方 [xiph/opus 1.5.2](https://github.com/xiph/opus/tree/v1.5.2)，不是第三方 Opus fork。E907 的 CLZ、Q15/Q16 DSP 乘法、2 的幂除法和 D4 音频快速路径来自本项目旧版 Opus 1.2.1 [M61 优化补丁组](https://github.com/zhaohyperion/DS5DONGLE-AIM61/tree/0968b719a38b5bfea951c8e87f3250ecac4e8fa8/m61/dualsense_hidp_probe/patches)，由 `ccc007ccc` 在真实 E907 热点分析和位精确测试基础上开发，之后适配并重新验证到 1.5.2。

### 固件及协议栈依赖

| 项目 | 用途 | 许可 / 备注 |
|---|---|---|
| [BouffaloSDK](https://github.com/bouffalolab/bouffalo_sdk) | BL616/BL618 HAL、启动、Flash、Bluetooth 与构建系统 | Apache-2.0；实际构建使用上表 `sqlCRT` 分支 |
| [Zephyr Project](https://github.com/zephyrproject-rtos/zephyr) | Bouffalo Bluetooth Host 中采用的 API 与代码基础 | Apache-2.0，经 SDK 引入 |
| [CherryUSB](https://github.com/cherry-embedded/CherryUSB) | USB HID/UAC1 Device Stack；SDK 固定版本为 v1.5.3 | Apache-2.0，经 SDK 引入 |
| [FreeRTOS Kernel](https://github.com/FreeRTOS/FreeRTOS-Kernel) | 固件任务、队列、同步和定时 | MIT，经 SDK 引入；`src/FreeRTOSConfig.h` 保留原版权声明 |
| [Mbed TLS](https://github.com/Mbed-TLS/mbedtls) | OTA SHA-256 与 ECDSA P-256 设备端验签 | Apache-2.0 或 GPL-2.0-or-later 双许可，本项目按 Apache-2.0 使用，经 SDK 引入 |
| [littlefs](https://github.com/littlefs-project/littlefs) | 配置与配对数据持久化，SDK 通过 EasyFlash 兼容接口暴露 | BSD-3-Clause，经 SDK 引入 |
| [xiph/opus](https://github.com/xiph/opus) | DualSense 双向语音音频编解码，固定点模式 | BSD-3-Clause；仓库只保留固件编译所需的 1.5.2 源码、头文件和许可证 |

[BTstack](https://github.com/bluekitchen/btstack) 与 [TinyUSB](https://github.com/hathach/tinyusb) 属于 `awalol/DS5Dongle` 的历史架构来源；当前 BL618 固件没有链接它们。Linux 内核 [`hid-playstation.c`](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-playstation.c) 仅用于 DualSense 报告布局和偏移交叉验证，仓库未复制 Linux 内核代码。

原始 `awalol/DS5Dongle` 还致谢了 [rafaelvaloto/Pico_W-Dualsense](https://github.com/rafaelvaloto/Pico_W-Dualsense)（项目灵感）、[egormanga/SAxense](https://github.com/egormanga/SAxense)（蓝牙触觉概念验证）、[Controllers Wiki 的 DualSense 报告结构资料](https://controllers.fandom.com/wiki/Sony_DualSense) 与 [Paliverse/DualSenseX](https://github.com/Paliverse/DualSenseX)（扬声器报告包参考）。这些间接历史参考一并保留致谢，但不是当前仓库的直接构建依赖。

### 工具、网页与开发协助

- Windows 刷写器由 Rust 构建，直接依赖 `anyhow`、`base64`、`eframe/egui`、`reqwest/rustls`、`rfd`、`serde`、`sha2`、`windows-sys` 和 `zip` 等项目；精确版本与完整传递依赖见 [`Cargo.toml`](tools/ds5dongle-flasher/Cargo.toml) 和 [`Cargo.lock`](tools/ds5dongle-flasher/Cargo.lock)。刷写后端来自 Bouffalo Lab `BLFlashCommand`；可选串口驱动由 [WCH](https://www.wch-ic.com/) 提供，驱动不会提交进仓库。
- Web 配置器使用 [React](https://github.com/facebook/react)、[Vinext](https://github.com/cloudflare/vinext)、[Vite](https://github.com/vitejs/vite)、[Tailwind CSS](https://github.com/tailwindlabs/tailwindcss) 和 Cloudflare 工具链；精确版本见 [`package.json`](web/package.json) 与 [`package-lock.json`](web/package-lock.json)。
- BL618 移植阶段使用 Cursor 与 Claude Opus 4.6 辅助开发；当前代码审计、OTA/刷写流程和文档整理使用 OpenAI Codex 辅助。所有合入结果仍由仓库维护者负责审查、测试与发布。

感谢上述作者、维护者和社区贡献者。更严格的版权归属、许可证范围及“仅参考而未包含代码”的界线见 [`NOTICE`](NOTICE)；Git 提交历史是个人代码贡献的最终记录。

## 许可证

本项目采用 [GNU General Public License v3.0](LICENSE)。第三方代码的版权和许可证信息见 [NOTICE](NOTICE)；`lib/opus` 的许可证见 [`lib/opus/LICENSE_PLEASE_READ.txt`](lib/opus/LICENSE_PLEASE_READ.txt)。

---

**English summary:** DS5DONGLE-AIM61 bridges a DualSense or DualSense Edge controller over Bluetooth Classic HID to a PC as a wired USB HID/UAC1 device. Ai-M61-32S-Kit is the default target; Full-Speed USB is recommended. The firmware includes bidirectional Opus audio, adaptive triggers, haptics, diagnostics, and signed A/B OTA. See the build and wiring sections above before flashing.
