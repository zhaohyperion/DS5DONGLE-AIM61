# DS5Dongle AIM61

面向 Ai-M61-32S-Kit（BL618）的 DualSense / DualSense Edge 蓝牙转 USB 适配器固件，以及配套的 Windows 原生测试、诊断和刷写工具。

[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
![Firmware](https://img.shields.io/badge/firmware-v3.5.2-orange)
![Flasher](https://img.shields.io/badge/flasher-v1.3.1-0067b8)
![USB](https://img.shields.io/badge/USB-High--Speed-success)

> 本项目与 Sony Interactive Entertainment 无关。“DualSense”“DualSense Edge”和“PlayStation”是其各自权利人的商标。固件模拟 Sony USB VID/PID，请自行评估使用环境和风险。

## v3.5.2 发布组成

v3.5.2 只提供 Ai-M61 High-Speed 两种配置，USB 描述符、产品名、VID/PID 和核心转发行为保持一致：

| 固件 | 文件 | 用途 |
|---|---|---|
| 常用版 HS | `DS5Dongle-aim61-hs-v3.5.2.zip` | 日常使用；性能优先，不枚举 `0xFD`，不创建诊断任务 |
| 诊断版 HS | `DS5Dongle-aim61-hs-diag-v3.5.2.zip` | 引导测试、内部 RX/TX/音频/队列/内存诊断和持续负载 |

v3.5.2 首先作为 **Pre-release** 发布，不删除或替换 v3.5.1。刷写器默认只显示最新常用版并把 v3.5.2 常用版作为 OTA 目标；诊断版位于“高级固件”，也可在“设备调试”页一键进入。回到常用版同样使用签名 OTA。

Windows 工具发布文件：`DS5Dongle-Flasher-Windows-v1.3.1.exe`。GitHub Release 只公开两份固件 ZIP、一个 EXE 和总 `SHA256SUMS.txt`；裸 `.bin` 仅保留在 CI 构建产物中。

## 主要功能

- Bluetooth Classic BR/EDR HID Host，支持配对、重连和最多 8 个绑定记录。
- DualSense / DualSense Edge 输入：按键、摇杆、扳机、触摸板、陀螺仪、加速度计和电量。
- 输出：灯条、玩家灯、静音灯、左右震动、自适应扳机和手柄声音。
- USB Audio Class 1.0 双向音频；Opus 1.5.2 固定点低延迟编解码及 E907 位精确优化。
- Ai-M61 4 MiB PSRAM 只用于显式冷数据；实时队列、Opus 状态和 USB/BT 热数据保留在内部 SRAM/TCM。
- P-256 签名、A/B 分区、掉电保护、试运行确认和失败回滚的 OTA 基础设施。
- Windows 原生“DS5Dongle AIM61 工具中心”，中文默认、可切换英文、单一白色高对比主题。

## 工具中心

工具中心包含三个职责明确的页签：

1. **测试中心**：实时输入、灯效、限强度震动、自适应扳机、手柄扬声器/耳机和 M61 USB 麦克风测试。
2. **固件刷写**：读取设备版本/构建配置，校验完整 ZIP，使用 CH340 UART ISP 刷写。
3. **设备调试**：常用/诊断配置双向 OTA、引导式诊断、M61 内部快照、Windows HID 间隔、10/20/50 Hz 持续负载和 JSON 导出。

引导诊断不限时长，可重复按键多次，采样充足后由用户手动进入下一项。固定顺序覆盖端点预检、全部输入、触摸、六轴、灯效、左右震动、左右扳机、声音、麦克风和最终复位。输出强度默认受限；按 `Esc` 可立即停止声音、震动并复位扳机。

声音、震动、灯效和自适应扳机使用冻结在本地的 `ds.evua.cc` 兼容 HID `0x02` / Feature `0x80` 测试向量，不在运行时访问该网站。该网站公开脚本的“麦克风”操作只控制静音灯，因此本工具在保持该行为的同时，额外通过 Windows 的 M61 UAC 输入端点完成实际录音/回放测试。

诊断 JSON 使用 `ds5dongle-flasher-diagnostics/v2`，区分 RX、TX、音频输入和音频输出；包含引导阶段、用户确认、原始快照和 Pass/Warning/Fail 结论。旧 v1 快照仍可读取，缺失字段按“不支持”处理而不是填零。默认不导出蓝牙地址、序列号和设备路径。

## Ai-M61 接线

板载 Type-C 连接 CH340，仅用于供电、UART 日志和 ISP 刷写。原生 USB 数据需从 GPIO37/38 引出：

```text
Ai-M61 GPIO37 / USB_DM  ─── USB D-
Ai-M61 GPIO38 / USB_DP  ─── USB D+
Ai-M61 GND              ─── USB GND
```

High-Speed 对线材、接头和飞线长度更敏感。D+/D- 应短、成对、阻抗尽量连续；除非确认供电拓扑安全，不要连接第二根 USB 线的 5 V。

## 构建

固件固定使用官方 Bouffalo SDK 2.3.31：

```powershell
git clone https://github.com/bouffalolab/bouffalo_sdk.git ..\bouffalo_sdk
git -C ..\bouffalo_sdk checkout 09abb06993d7aaa594648ecb6a3212c53a44f3da
Get-ChildItem sdk-patches\*.patch | Sort-Object Name | ForEach-Object {
  git -C ..\bouffalo_sdk apply --check --recount $_.FullName
  git -C ..\bouffalo_sdk apply --recount $_.FullName
}
```

补丁来源和取舍见 [`sdk-patches/README.md`](sdk-patches/README.md)。Windows 还需 T-Head `riscv64-unknown-elf` 工具链。

```powershell
$env:BL_SDK_BASE = "C:\path\to\bouffalo_sdk"
$env:TOOLCHAIN_PATH = "C:\path\to\toolchain_gcc_t-head_windows"
$env:BOARD_TYPE = "aim61"
$env:USB_SPEED = "hs"
$env:FIRMWARE_VERSION = "3.5.2"

$env:DS5_BUILD_PROFILE = "standard"
$env:DS5_LOG_LEVEL = "0"
.\build_windows.bat rebuild

$env:DS5_BUILD_PROFILE = "diagnostic"
$env:DS5_LOG_LEVEL = "1"
.\build_windows.bat rebuild
```

打包：

```powershell
python tools\package_firmware.py --board aim61 --usb-speed hs --profile standard `
  --version 3.5.2 --firmware-dir firmware\aim61 --output-dir dist
python tools\package_firmware.py --board aim61 --usb-speed hs --profile diagnostic `
  --version 3.5.2 --firmware-dir firmware\aim61 --output-dir dist
```

## 刷写与配对

1. 从 [Releases](https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases) 下载 v1.3.1 工具并核对 `SHA256SUMS.txt`。
2. 按住 Ai-M61 的 **BOOT**，短按 **RESET** 后松开 BOOT，使其进入 UART ISP。
3. 在“固件刷写”页选择 CH340 端口和完整 ZIP；工具会检查板型、HS 配置、文件大小和包内 SHA-256。
4. 正常复位。手柄关机时长按 **PS + Create** 进入蓝牙配对。

不要把 OTA `.bin.ota` 当作完整 UART 刷写包，也不要把其他开发板的 Boot2/分区表写入 Ai-M61。

## OTA 与存储

OTA A/B 槽位位于板载 SPI Flash，不在 PSRAM。PSRAM 断电即失且不是可信启动介质，只能作为下载/诊断期间的临时缓冲。A/B 固件交换不会持续占用 CPU；只有下载、校验和写 Flash 阶段会产生短时负载。

同版本 `standard ↔ diagnostic` 允许显式切换，但自动更新只跟随相同配置。v3.5.2 不通过 OTA 自动降级到 v3.5.1；需要回退时使用 UART 完整刷写。配对、映射和设置分区在配置切换时保留。协议细节见 [`docs/OTA.md`](docs/OTA.md)。

## 测试

```powershell
python -m unittest discover -s tools -p "test_*.py"
cargo fmt --manifest-path tools\ds5dongle-flasher\Cargo.toml -- --check
cargo test --locked --manifest-path tools\ds5dongle-flasher\Cargo.toml
git diff --check
```

本次版本不等待真机才生成代码和预发行包，但 **Pre-release 不等于真机验证完成**。稳定推广前仍应在 Ai-M61 上验证配对/重连、DS/Edge、HS 信号完整性、扬声器/麦克风、5 分钟持续负载、掉电 OTA 与回滚。

## 来源与致谢

本项目沿用了多层上游成果，不能只归因于当前仓库：

| 来源 | 与本项目的关系 |
|---|---|
| [awalol/DS5Dongle](https://github.com/awalol/DS5Dongle) | 最初的 DualSense 蓝牙 HID 到 USB 项目方向和核心设计，MIT |
| [ccc007ccc/DS5Dongle](https://github.com/ccc007ccc/DS5Dongle) | BL616/BL618/Ai-M61 移植、实时音频、E907 优化和工程演进；原提交历史保留，MIT |
| [sqlCRT/ds5dongle-bl618-opensource](https://github.com/sqlCRT/ds5dongle-bl618-opensource) | 本 BL618 固件主线的重要开源基准和后续功能来源，GPL-3.0 |
| [bouffalolab/bouffalo_sdk](https://github.com/bouffalolab/bouffalo_sdk) | 官方 SDK 2.3.31 固定基线，Apache-2.0 |
| [sqlCRT/bouffalo_sdk](https://github.com/sqlCRT/bouffalo_sdk) | AIM61/USB Audio/BR-EDR/PSRAM 修复的补丁来源；已审计为本仓库透明补丁，Apache-2.0 |
| [xiph/opus](https://github.com/xiph/opus) | Opus 1.5.2 固定点编解码器，BSD-3-Clause |
| [CherryUSB](https://github.com/cherry-embedded/CherryUSB) | Bouffalo SDK 内使用的 USB 设备栈，Apache-2.0 |
| [ds.evua.cc](https://ds.evua.cc/) | 手柄输出测试行为的公开参考；兼容测试向量冻结在本地，不构成运行时依赖 |

完整许可证、版本、采用范围和未采用范围见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。如发现遗漏，请提交 Issue 补充，不应删除上游版权或许可证声明。

## 许可证

当前仓库整体按 [GPL-3.0](LICENSE) 发布。第三方组件继续受其各自许可证约束。
