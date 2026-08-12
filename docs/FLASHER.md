# DS5Dongle Windows 一键刷写器

`DS5Dongle-Flasher-Windows.exe` 是 DS5DONGLE-AIM61 的单文件 GUI/CLI 刷写器。它可以读取本仓库 GitHub Release，也可以刷写本地完整固件 ZIP 或解包目录。

项目原有的公开配置网站已经下线。设备测试、运行诊断和固件管理均在本机刷写器内完成；诊断 JSON 不会自动上传，当前版本也不通过网页或原生界面执行应用级 OTA。

## 普通用户：在线选择固件并刷写

1. 从 [DS5DONGLE-AIM61 Releases](https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases/latest) 下载 `DS5Dongle-Flasher-Windows.exe`。
2. 连接开发板的 UART/下载串口。Ai-M61 使用板载 Type-C/CH340；另外两种板卡可使用支持 3.3 V 电平的 USB-TTL。
3. 按住 BOOT，短按 RESET 后松开 BOOT，使 BL616/BL618 进入 UART ISP。
4. 启动刷写器，选择物理板型、USB Full-Speed/High-Speed 和正确 COM 口。
5. 在“在线 Release”中选择版本；默认推荐 Ai-M61 Full-Speed 稳定版。
6. 开始刷写。工具会下载并验证完整包，然后写入 Boot2、分区表和应用固件。

刷写器启动后还会通过运行态 USB HID `0xF8` Feature Report 读取当前设备的固件版本。该读取不会进入 ISP、不会访问 Flash 写入路径；设备必须已正常启动并以 USB 游戏控制器连接。也可以点击“重新读取”，或使用 CLI：

```powershell
.\DS5Dongle-Flasher-Windows.exe --device-info
```

### 一键诊断

刷写器 1.2.0 起提供原生运行态诊断。点击“运行一键诊断”会同时执行：

- 检查 Windows 串口、CH340 PnP 状态和驱动可用性；只有 `VID_1A86/PID_7523` 的 CH340 会进入刷写候选，主板串口 `COM1` 等普通端口只显示为“已忽略”。
- 通过 USB HID `0xFD` 读取七页运行态快照（兼容旧六页固件），逐页校验 DG/v1 头、CRC32、页号、快照序号和单调时间一致性。
- 显示运行时间、空闲堆、RSSI、电量、USB/蓝牙计数、压力/丢失、麦克风欠载及 OTA 状态。
- 将串口环境与原始 `0xFD` 页面保存为本地 JSON 诊断包；数据不会上传服务器，导出内容不包含 PnP 实例序列号或操作日志中的本机路径。

设备必须正常启动且运行支持 `0xFD` 的本项目固件。上游基线固件或较旧固件可以继续线刷，但会在诊断窗口明确显示“不支持 0xFD”。CLI 可直接输出相同 JSON：

```powershell
.\DS5Dongle-Flasher-Windows.exe --diagnostics
```

### 手柄功能测试中心

刷写器 1.3.0 起内置本机测试中心。主窗口顶部采用“测试中心 / 固件刷写 / 设备调试”三个页签，测试中心排在第一位并作为默认页；三个页面可以随时来回切换。正常启动固件并连接手柄后可以：

- 实时查看全部常用按键、方向键、摇杆、L2/R2 模拟量、双指触摸坐标、陀螺仪、加速度计和实际 HID 输入报告频率；
- 控制左右震动电机、灯条 RGB、五个玩家灯、静音灯；
- 向左右自适应扳机发送限强度的阻力或脉冲预设；
- 播放左、右或双声道 440 Hz 测试音；
- 使用系统默认麦克风录制 5 秒并自动回放，临时 WAV 文件在测试后删除。

输入和 HID 输出直接使用本项目固件提供的 DS5 游戏控制器接口。音频使用 Windows 当前默认播放/录音设备，因此开始前需要在 Windows“声音设置”中把 `DualSense Wireless Controller` 分别设为默认输出和默认输入。关闭 Steam、DS4Windows 和浏览器手柄测试页，避免 HID 接口或输出状态被其他程序占用。关闭刷写器会自动发送全复位报告，停止震动、灯效和自适应扳机。

### 设备调试与性能分析

第三个“设备调试”页签用于定位延迟、调度抖动和运行性能问题：

- 可运行 5、15、30 或 60 分钟自动压力测试，统计 HID 报告频率、平均/最小间隔、P95、P99、最大间隔和标准差抖动；默认 15 分钟；
- 可选输出负载默认 20 Hz，适合常规稳定性长测；另有明确标注的 50 Hz 极限链路模式。执行器按 15 秒工作、5 秒完全释放循环，停止、异常或退出时自动发送全复位报告；
- 使用固定大小直方图和在线统计累计长测数据，不会因运行时间增长而持续占用内存或反复排序全部样本；
- 统计超过 5 ms 与 10 ms 的长间隔次数，并按样本自动给出稳定、警告或异常结论；
- 开始、结束及测试期间每 5 秒自动采集固件 `0xFD` 快照；Windows 检测到新的至少 20 ms HID 停顿时会追加采集，并限制触发频率，便于关联 M61 内部延迟、USB、蓝牙和内存状态；
- 新版固件额外给出蓝牙 HID 输入回调进入 M61 到 USB IN 提交、USB 提交到完成，以及 M61 内部总转发延迟的平均值、P95、P99 和最大值；
- 同时检查 RSSI、空闲堆、丢失压力、音频提交量以及麦克风欠载/过载；
- 可导出 `ds5dongle-performance/v1` JSON，包含 HID 指标、基准前后快照及明确的测量边界说明。

M61 内部计时从固件蓝牙 HID 回调入口开始，到 USB 传输完成回调结束；Windows“间隔/抖动”则是 HID 报告到达系统后的调度指标。两组数据可帮助区分固件桥接延迟和 PC 调度停顿，但都不包含无线电/L2CAP 之前的时间，也不是从物理按键动作到游戏画面响应的绝对端到端延迟。要测完整端到端延迟仍需高速摄像机、光电传感器或外部逻辑分析设备。

长时间执行器测试会增加手柄耗电、马达温升和自适应扳机机械负担。本工具无法读取执行器温度，因此 20 Hz 是默认档；50 Hz 最长限制为 15 分钟，只建议在通风条件良好时用于短时极限链路测试。5 秒快照会增加少量 EP0 控制传输，但远低于持续 HID 数据流负载，并避免 60 秒采样遗漏大多数一秒延迟窗口。

7. 成功后按 RESET 正常启动。460800 失败时可在 GUI 中用 115200 重试。

本仓库首次发布带完整固件 ZIP 的 GitHub Release 前，在线列表为空属于正常现象；可以使用下文的本地完整包流程。

只有安装 WCH 驱动会触发管理员权限。驱动安装器从 WCH 官方地址下载，启动前会检查固定 SHA-256 和 Authenticode 发布者；普通刷写过程不需要把驱动或固件上传到第三方服务。

## 本地完整包

GUI 可选择 `DS5Dongle-<board>-<fs|hs>-v<version>.zip`，也可选择已解包目录。每个包必须包含：

- `firmware.json`
- `SHA256SUMS.txt`
- 一个 `boot2_bl616_*.bin`
- `partition.bin`
- 恰好一个与板型、USB 速度一致的应用 `.bin`

刷写器拒绝路径穿越、重复文件、异常尺寸、SHA-256 不匹配、清单字段错误及板型/速度与文件名不一致。完整线刷包与 OTA `.bin.ota` 不可互换。

本地构建后生成包：

```powershell
python tools\package_firmware.py `
  --board aim61 `
  --usb-speed fs `
  --version v3.5.1 `
  --firmware-dir firmware\aim61 `
  --output-dir dist
```

## CLI

```text
DS5Dongle-Flasher-Windows.exe [options]

--board aim61|lctech616|m0sdock
--usb-speed fs|hs
--port COM5
--baud 460800|115200
--list
--device-info
--diagnostics
--list-releases
--release <tag>
--verify-release
--tool-preflight
--assets-info
--install-driver
--dry-run
--yes
```

示例：

```powershell
# 检测串口并列出本仓库可用完整包
.\DS5Dongle-Flasher-Windows.exe --list
.\DS5Dongle-Flasher-Windows.exe --device-info
.\DS5Dongle-Flasher-Windows.exe --diagnostics
.\DS5Dongle-Flasher-Windows.exe --list-releases --board aim61 --usb-speed fs

# 只下载并验证，不接触设备
.\DS5Dongle-Flasher-Windows.exe --verify-release --release v3.5.1 --board aim61 --usb-speed fs

# 验证设备、工具和命令，但不执行写入
.\DS5Dongle-Flasher-Windows.exe --release v3.5.1 --board aim61 --usb-speed fs --port COM5 --dry-run

# 受控自动化刷写；务必先确认 COM 口和物理板型
.\DS5Dongle-Flasher-Windows.exe --release v3.5.1 --board aim61 --usb-speed fs --port COM5 --yes
```

## 自行构建刷写器

构建脚本从固定的 [`sqlCRT/bouffalo_sdk`](https://github.com/sqlCRT/bouffalo_sdk) 提交 `cf6adf74b374a0e485defa9c89610f3e7ffcc3ec` 嵌入 `BLFlashCommand.exe`、eflash loader 配置和 Flash 参数。设置 `M61_BLFLASHCOMMAND` 后执行：

```powershell
cargo fmt --manifest-path tools\ds5dongle-flasher\Cargo.toml -- --check
cargo test --locked --manifest-path tools\ds5dongle-flasher\Cargo.toml
cargo build --locked --release --manifest-path tools\ds5dongle-flasher\Cargo.toml
```

输出位于 `tools/ds5dongle-flasher/target/release/ds5dongle-flasher.exe`。仓库对 `x86_64-pc-windows-gnullvm` 配置静态 CRT/unwind 链接，正式 EXE 不应依赖旁置的 `libunwind.dll`。

## 安全边界与排障

- 选择错误板型或分区表可能导致设备无法正常启动；开始前再次核对板型、USB 速度和 COM 口。
- 自动检测和 GUI 只允许 M61 的 CH340；普通 `COM1`、蓝牙串口及其他虚拟 COM 口不能用于刷写。
- Full-Speed 是 Ai-M61 飞线场景的默认选择；High-Speed 只适合已经验证的短差分布线。
- 刷写器只显示带 GitHub SHA-256 digest、且包含完整线刷资产的 Release。
- 日志与临时运行目录会保留到刷写完成或失败，便于定位问题；分享前请检查其中是否含本机路径或串口信息。
- 两槽 OTA、Boot2 或分区表损坏时，必须使用这里的 UART 完整刷写恢复，应用级 OTA 无法救援。
