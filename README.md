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
| 固件开发版本 | `3.5.0`；正式 Release 版本由标签注入 |
| 音频编解码 | Opus 1.5.2，固定点，E907 DSP 快速路径 |
| Web OTA | 仅 Ai-M61 Full-Speed，P-256 签名、A/B 分区、试运行回滚 |
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
| `FIRMWARE_VERSION` | `X.Y.Z`，每段 0..254 | `3.5.0` |

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

生成供刷写器使用的本地完整包：

```powershell
python tools/package_firmware.py `
  --board aim61 `
  --usb-speed fs `
  --version local `
  --firmware-dir firmware/aim61 `
  --output-dir dist
```

Ai-M61 进入 UART ISP：

1. 使用板载 Type-C 连接电脑。
2. 按住 **BOOT**。
3. 按一下 RESET，或在按住 BOOT 时重新插线。
4. 松开 BOOT，在刷写器中选择 CH340 对应的 COM 口。
5. 选择生成的 `DS5Dongle-aim61-fs-vlocal.zip` 并开始刷写。

刷写器源码与构建说明见 [`tools/ds5dongle-flasher/`](tools/ds5dongle-flasher/) 和 [`docs/FLASHER.md`](docs/FLASHER.md)。

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
- Ai-M61 Full-Speed 签名 OTA

生产 OTA 使用 fail-closed 信任模型：

- 网页和设备端分别验证 P-256 ECDSA 签名。
- 同时校验容器和固件体 SHA-256。
- 只写非活动分区，完成校验后才切换启动槽。
- 新固件必须通过试运行确认，否则 Boot2 回滚。
- 普通本地构建不包含默认发布密钥，因此不会意外接受生产 OTA。

完整协议和发布流程见 [`docs/OTA.md`](docs/OTA.md)。生产私钥不得写入源码、网页环境变量、构建日志或 GitHub 仓库。

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
- Web OTA 当前只接受 Ai-M61 Full-Speed RAW OTA 镜像。
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
lib/opus/                    Opus 1.5.2 上游源码
firmware/                    板型刷写配置与本地输出目录
tools/ds5dongle-flasher/     Windows GUI/CLI 刷写器
tools/ota_*.py               OTA 协议、签名和发布工具
tools/m61-diagnostics.ps1    运行期诊断采集工具
web/                         WebHID 配置与 OTA 页面
docs/                        OTA、诊断和刷写器详细文档
.github/workflows/           构建与发布自动化
```

## 来源与致谢

- [awalol/DS5Dongle](https://github.com/awalol/DS5Dongle)：Raspberry Pi Pico 2W 原始实现和核心协议参考
- [sqlCRT/bouffalo_sdk](https://github.com/sqlCRT/bouffalo_sdk)：BL616/BL618 SDK 与本项目所需适配
- [CherryUSB](https://github.com/cherry-embedded/CherryUSB)：USB Device Stack
- [xiph/opus](https://github.com/xiph/opus)：Opus 音频编解码器

本移植将 BTstack/TinyUSB 架构迁移到 Bouffalo SDK Bluetooth Stack 与 CherryUSB，并针对 E907、Ai-M61 PSRAM、实时音频和 USB HID 调度进行了适配。

## 许可证

本项目采用 [GNU General Public License v3.0](LICENSE)。第三方代码的版权和许可证信息见 [NOTICE](NOTICE)；`lib/opus` 的许可证见 [`lib/opus/LICENSE_PLEASE_READ.txt`](lib/opus/LICENSE_PLEASE_READ.txt)。

---

**English summary:** DS5DONGLE-AIM61 bridges a DualSense or DualSense Edge controller over Bluetooth Classic HID to a PC as a wired USB HID/UAC1 device. Ai-M61-32S-Kit is the default target; Full-Speed USB is recommended. The firmware includes bidirectional Opus audio, adaptive triggers, haptics, diagnostics, and signed A/B OTA. See the build and wiring sections above before flashing.
