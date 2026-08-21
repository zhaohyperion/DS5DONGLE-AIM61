# DS5Dongle AIM61

面向 Ai-M61-32S-Kit（BL618）的 DualSense / DualSense Edge 蓝牙转 USB 适配器固件，以及配套的 Windows 原生测试、诊断和刷写工具。

[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
![Firmware](https://img.shields.io/badge/firmware-v3.6.0-orange)
![Flasher](https://img.shields.io/badge/flasher-v1.4.0-0067b8)
![USB](https://img.shields.io/badge/USB-High--Speed-success)

> 本项目与 Sony Interactive Entertainment 无关。“DualSense”“DualSense Edge”和“PlayStation”是其各自权利人的商标。固件模拟 Sony USB VID/PID，请自行评估使用环境和风险。

## v3.6.0 发布组成

v3.6.0 只提供 Ai-M61 High-Speed 两种配置，USB 描述符、产品名、VID/PID 和核心转发行为保持一致：

| 固件 | 文件 | 用途 |
|---|---|---|
| 常用版 HS | `DS5Dongle-aim61-hs-v3.6.0.zip` | 日常使用；性能优先，不创建诊断任务，也不生成 `0xFD` 快照 |
| 诊断版 HS | `DS5Dongle-aim61-hs-diag-v3.6.0.zip` | 引导测试、内部 RX/TX/音频/队列/内存诊断和持续负载 |

v3.6.0 首先作为 **Pre-release** 发布，不删除或替换 v3.5.1/v3.5.2。刷写器默认只显示最新常用版并把 v3.6.0 常用版作为 OTA 目标；诊断版位于“高级固件”。回到常用版同样使用签名 OTA。

Windows 工具发布文件：`DS5Dongle-Flasher-Windows-v1.4.0.exe`。GitHub Release 只公开两份固件 ZIP、一个 EXE 和总 `SHA256SUMS.txt`；裸 `.bin` 仅保留在 CI 构建产物中。

四个页签均支持整页鼠标滚轮和滚动条浏览，在较小窗口或 Windows 高 DPI 缩放下仍可访问页面底部操作。

## 主要功能

- Bluetooth Classic BR/EDR HID Host，支持配对、重连和最多 8 个绑定记录。
- DualSense / DualSense Edge 输入：按键、摇杆、扳机、触摸板、陀螺仪、加速度计和电量。
- 输出：灯条、玩家灯、静音灯、左右震动、自适应扳机和手柄声音。
- USB Audio Class 1.0 双向音频；Opus 1.5.2 固定点低延迟编解码及 E907 位精确优化。
- High-Speed 首次启动默认实时档（约 750 Hz）；Q15 音频重采样减少热路径浮点运算，USB/BT 输入任务继续优先于音频任务。
- 快速重连保留 Windows HID/UAC 会话 10 秒；休眠后关闭无用蓝牙扫描，恢复时自动重新启用。
- Ai-M61 4 MiB PSRAM 只用于显式冷数据；实时队列、Opus 状态和 USB/BT 热数据保留在内部 SRAM/TCM。
- P-256 签名、A/B 分区、掉电保护、试运行确认和失败回滚的 OTA 基础设施。
- Windows 原生“DS5Dongle AIM61 工具中心”，中文默认、可切换英文、单一白色高对比主题。

## 工具中心

工具中心包含四个职责明确的页签，顺序固定为：

1. **测试中心**：实时输入、灯效、限强度震动、L2/R2 独立力度可调的自适应扳机、手柄扬声器/耳机、M61 USB 麦克风测试，以及双摇杆中心偏移、范围覆盖和圆度误差分析。
2. **按键映射**：19 个物理控件的一对多同步映射，以及时序、连发、重复、循环和独立录制宏。
3. **设备调试**：250/500/实时轮询档位、引导式诊断、M61 内部快照、Windows HID 间隔、10/20/50 Hz 持续负载和 JSON 导出。
4. **固件刷写**：读取设备版本/构建配置，校验完整 ZIP，使用 CH340 UART ISP 刷写，并支持签名在线/本地 OTA。

轮询档位通过固件现有的 `0xF7` 配置读取、`0xF6` 配置写入。工具会保留整份设备配置，只修改轮询字段，然后保存并重启 M61，使 Windows 重新读取 USB `bInterval`。已保存的旧配置不会因为升级固件被强制覆盖；需要最高性能时可在“设备调试”页明确选择实时档。

“按键映射”页提供完整的 19 控件映射：方块、叉、圆、三角、L1/R1、L2/R2、创建、选项、L3/R3、PS、触摸板键、静音键以及方向键上/右/下/左。`0xFB` v3 为每个物理输入保存 19 位输出掩码：零目标表示禁用，一个目标表示普通映射，多个目标会在同一 HID 帧同步输出组合键；写入后立即回读校验。旧 v2 的 19 键单目标表可读取并转换成草稿，旧 15 键表不再兼容。

宏协议使用独立的 `0xFE` Feature Report，不占用 `0xFA/0xFC` OTA、`0xFB` 映射和诊断版 `0xFD`。支持 4 个设备档位、按下/释放/长按/双击/按住触发、单次/按住/定次重复/按住循环/开关循环/1–50 Hz 连发，以及按键、摇杆、L2/R2、双触摸点和随机附加时长步骤。设备只运行一个宏，新触发会打断旧宏；物理 PS 长按 2 秒始终急停。完整宏数据和上传缓冲放在 PSRAM，实时执行状态留在片内 SRAM；PSM 中以 A/B 文件、代数和 CRC 事务保存，工具写入后回读验证并防止覆盖外部修改。

无需打开工具也可独立录制：Create + Options 长按 2 秒后选择物理触发键，黄灯倒计时 3 秒，绿灯常亮时录制映射后的按键/摇杆/扳机/触摸与时序；同一组合键结束，80%/满容量使用红灯提示，保存成功绿灯闪三次，PS 长按 2 秒取消。Mute + Options 长按 2 秒进入完全隔离的管理模式，可切换总开关、时序/重复类别、4 个档位和单个触发键宏。

引导诊断不限时长，输入覆盖从开始到结束全程累计，不会因为提前操作而丢失。面键、功能键、方向键八方向、肩键、双摇杆八方向与回中、L2/R2 起始/中段/满量程/释放、双触摸点、左右区域、四向滑动、触摸板点击以及陀螺仪/加速度计六轴响应均由程序显示完成比例、缺失动作并自动判定；覆盖完成后自动进入下一项。灯效、左右震动、自适应扳机阻力、声音和回听失真等无法仅凭 HID 数据判断的主观输出继续由用户确认。输出强度默认受限；按 `Esc` 可立即停止声音、震动并复位扳机。

声音、震动、灯效和自适应扳机使用冻结在本地的 `ds.evua.cc` 兼容 HID `0x02` / Feature `0x80` 测试向量，不在运行时访问该网站。该网站公开脚本的“麦克风”操作只控制静音灯，因此本工具在保持该行为的同时，额外通过 Windows WASAPI 自动寻找 M61 / DualSense UAC 输入端点完成实际录音，计算 RMS、峰值、有效声音窗口、静音底噪和信噪比，并允许保存 WAV。录音和默认输出设备回放分别判定，回放失败不会丢失录音指标；回听仍用于确认失真、爆音等纯数值难以判断的问题。

摇杆分析原生移植了 `dualshock-tools` 的 48 方向采样和 RMS 圆度误差定义，并结合 `dualsense-tester` 的 DualSense 输入、触摸、六轴与输出测试流程。引导输出采用左右依次震动、RGB/玩家灯动画、三轮扳机阻力/复位以及扬声器/耳机交替测试；普通 DualSense 与 Edge 的永久校准分别遵循上游单阶段和双阶段 `0x82/0x83` 状态序列。工具依据项目经验阈值给出“正常 / 建议复测 / 建议校准”，同时保留中心偏移、X/Y 范围、方向覆盖和圆度误差原值；这不是 Sony 官方检测标准。当前复测阈值为中心偏移超过 3%、圆度误差低于 5% 或超过 12%、任一轴范围不足 95%；其中低于 5% 仅按 `dualshock-tools` 的操控手感提示建议复测，不会建议永久校准。严重阈值分别为中心 5%、圆度 20% 和范围 85%。

只读分析不会改写手柄；只有左右摇杆都完成松手中心采样且方向覆盖达到 75% 后，才可显式确认并分步执行高级永久中心/范围校准。检测结果无需通过即可解锁，避免故障摇杆陷入死锁。固件只对白名单内且长度、命令结构正确的 `0x82` 校准报告开放转发，仍会拦截可能关机或重新配对的其他 Feature SET_REPORT。旧固件可使用全部只读测试，但不支持经 M61 写入永久校准。

“导出完整测试报告 JSON”会把手柄实时输入、自动输入覆盖率与缺失动作、主观输出确认、双摇杆分析、校准步骤及校准前后复测、声音与麦克风确认、Windows HID 延迟/抖动、压力负载和 M61 运行快照集中到一个文件。自动结果与人工确认分别标记 `resultSource`，不会把程序判定伪装成用户确认。报告使用 `ds5dongle-flasher-diagnostics/v2`，顶部 `summaryZhCn` 提供可直接阅读的中文总体结论、异常项、未测试项、关键指标和处理建议，同时保留机器分析所需的原始字段；未执行或只完成一部分的项目不会被误判为通过。旧 v1 快照仍可读取，缺失字段按“不支持”处理而不是填零。默认不导出蓝牙地址、序列号和设备路径。

引导式设备诊断卡片内也提供直接导出按钮，无需先运行压力测试。测试进行中可以导出阶段性报告，完成后可导出最终报告；两者都沿用同一 `v2` 统一格式，避免形成互不兼容的独立报告。开始下一轮引导或压力测试前，建议先保存需要保留的上一轮结果。

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
$env:FIRMWARE_VERSION = "3.6.0"

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
  --version 3.6.0 --firmware-dir firmware\aim61 --output-dir dist
python tools\package_firmware.py --board aim61 --usb-speed hs --profile diagnostic `
  --version 3.6.0 --firmware-dir firmware\aim61 --output-dir dist
```

## 刷写与配对

1. 从 [Releases](https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases) 下载 v1.4.0 工具并核对 `SHA256SUMS.txt`。
2. 按住 Ai-M61 的 **BOOT**，短按 **RESET** 后松开 BOOT，使其进入 UART ISP。
3. 在“固件刷写”页选择 CH340 端口和完整 ZIP；工具会检查板型、HS 配置、文件大小和包内 SHA-256。
4. 正常复位。手柄关机时长按 **PS + Create** 进入蓝牙配对。

不要把 OTA `.bin.ota` 当作完整 UART 刷写包，也不要把其他开发板的 Boot2/分区表写入 Ai-M61。

## OTA 与存储

OTA A/B 槽位位于板载 SPI Flash，不在 PSRAM。PSRAM 断电即失且不是可信启动介质，只能作为下载/诊断期间的临时缓冲。A/B 固件交换不会持续占用 CPU；只有下载、校验和写 Flash 阶段会产生短时负载。

同版本 `standard ↔ diagnostic` 允许显式切换，但自动更新只跟随相同配置。v3.6.0 不自动降级；需要回退时使用高级 OTA 明确确认或 UART 完整刷写。配对、映射、宏和设置在配置切换时保留。协议细节见 [`docs/OTA.md`](docs/OTA.md)。

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
| [bibuq0/DSdongle-bl616](https://github.com/bibuq0/DSdongle-bl616) | v3.17 的 Q15 重采样、快速重连 USB 宽限期、Primer 音量重发和休眠 radio 静默作为本次选择性移植参考；未采用 Electron 应用、`silk_stubs` 或 CELT-only 源码裁剪，GPL-3.0 |
| [bouffalolab/bouffalo_sdk](https://github.com/bouffalolab/bouffalo_sdk) | 官方 SDK 2.3.31 固定基线，Apache-2.0 |
| [sqlCRT/bouffalo_sdk](https://github.com/sqlCRT/bouffalo_sdk) | AIM61/USB Audio/BR-EDR/PSRAM 修复的补丁来源；已审计为本仓库透明补丁，Apache-2.0 |
| [xiph/opus](https://github.com/xiph/opus) | Opus 1.5.2 固定点编解码器，BSD-3-Clause |
| [CherryUSB](https://github.com/cherry-embedded/CherryUSB) | Bouffalo SDK 内使用的 USB 设备栈，Apache-2.0 |
| [ds.evua.cc](https://ds.evua.cc/) | 手柄输出测试行为的公开参考；兼容测试向量冻结在本地，不构成运行时依赖 |
| [daidr/dualsense-tester](https://github.com/daidr/dualsense-tester) | DualSense 输入、触摸、六轴、输出与音频测试交互参考；测试中心直接采用并本地化其固定提交的 DS5 SVG 路径、模型坐标、控件布局、摇杆归一化和双触摸点映射，不嵌入网页，MIT |
| [dualshock-tools/dualshock-tools.github.io](https://github.com/dualshock-tools/dualshock-tools.github.io) | 48 方向摇杆轨迹、范围覆盖、RMS 圆度误差和 DS5 `0x82/0x83` 分步校准协议参考，MIT |

完整许可证、版本、采用范围和未采用范围见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。如发现遗漏，请提交 Issue 补充，不应删除上游版权或许可证声明。

## 许可证

当前仓库整体按 [GPL-3.0](LICENSE) 发布。第三方组件继续受其各自许可证约束。
