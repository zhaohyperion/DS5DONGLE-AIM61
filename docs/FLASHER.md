# DS5Dongle Windows 工具中心 v1.4.2

`DS5Dongle-Flasher-Windows-v1.4.2.exe` 是 DS5DONGLE-AIM61 的单文件 Windows 原生工具。公开配置网站已下线，仓库不再包含配置网页代码；测试、映射与宏、诊断、OTA 配置切换和 UART 恢复都在本机完成，诊断数据不会自动上传。

## 四个页签

### 1. 测试中心

默认首页，直接操作当前 USB 游戏控制器接口：

- 实时显示按键、方向键、摇杆、L2/R2、双点触摸、六轴、电量和输入报告率；
- 生成灯条/玩家灯/静音灯、限强度震动和自适应扳机测试信号；L2/R2 可分别选择模式并在 `0–255` 范围内调节力度，超过 200 只建议短时验证；
- 使用与 `ds.evua.cc` 公开实现一致的 DS5 HID Output/Feature 报告测试声音、震动和扳机；
- 通过 WASAPI 自动选择 M61 / DualSense UAC 输入端点录音，验证真实输入链路，而不只是切换静音灯；录音可另存为 WAV，回放失败不会抹掉采集指标；
- 退出、断开或按 `Esc` 时停止声音、震动并复位扳机。

测试前关闭 Steam、DS4Windows、浏览器手柄页及其他会占用 HID/音频端点的软件。声音测试时应在 Windows 声音设置中确认 `DualSense Wireless Controller` 的扬声器、耳机和麦克风端点可用。

### 2. 按键映射

此页使用 DS5 图形呈现 19 个物理控件。每个来源可映射为零个、一个或多个同步目标；工具写入 `0xFB` v3 后会回读代数、CRC 和完整掩码。固件不读取旧 15 键配置，首次升级默认为一对一。

同页的宏编辑器支持 4 个设备档位、按下/释放/长按/双击/按住触发，单次、按住、定次重复、按住循环、开关循环和 1–50 Hz 连发。宏步骤可包含按键、摇杆、扳机、双触摸点和随机附加时长，不记录灯效、震动、自适应扳机、声音、六轴或条件。设备只运行一个宏，新宏打断旧宏；物理 PS 长按 2 秒始终急停。

不开工具也可用 Create+Options 长按 2 秒进入独立录制：选择触发键后黄灯倒计时 3 秒，绿灯亮起开始录制；再次按组合键结束。Mute+Options 长按 2 秒进入设备宏管理。录制内容存入事务式 A/B PSM 文件，并可在工具中读取修改。

### 3. 设备调试

此页包含引导诊断和默认 5 分钟的性能/压力测试。Diagnostic 固件按需生成 `0xFD` 快照和 M61 蓝牙回调→USB 完成内部计时；Standard 保留相同 HID 描述符布局以避免 Windows 缓存冲突，但不创建诊断任务，也不响应或生成快照。统一 JSON 报告包含输入覆盖、人工输出确认、Windows HID 统计、全部已采集快照、映射协议，以及宏配置元数据和编译数据 SHA-256（默认不包含完整宏定义）。

引导式诊断不限时长，可重复操作多次后由用户决定通过、未生效、重测或跳过。流程覆盖端点预检、全部按键/方向、摇杆/扳机输入、触摸、六轴、灯效、震动、自适应扳机、声音、麦克风和最终复位。诊断 JSON 使用 `ds5dongle-flasher-diagnostics/v2`，区分 RX、TX、音频输入和音频输出，并保留用户确认和原始快照。

压力测试默认 5 分钟，可选 15/30/60 分钟：

- 输出负载默认 20 Hz，可选 10 Hz 或 50 Hz；50 Hz 只用于短时极限链路检查；
- 执行器按工作/释放周期运行，震动与扳机强度受限；
- 使用固定直方图和在线统计计算报告率、平均/最小间隔、P95、P99、最大间隔和抖动，内存不会随时长线性增长；
- Diagnostic 模式定期采集 M61 内部蓝牙回调→USB 提交、USB 提交→完成及总桥接延迟，并关联堆、RSSI、USB/BT/音频/麦克风计数；
- Windows HID 间隔不是物理按键到画面的绝对端到端延迟。完整端到端测量仍需高速摄像、光电或逻辑分析设备。

长测会增加耗电、马达温升和扳机机械负担。工具无法读取执行器温度；发现异响、过热或机械异常时应立即按 `Esc`。

### 4. 固件刷写

默认在线 UART 列表只显示最新 **AIM61 High-Speed Standard（常用版）`uart-full`**。勾选“高级固件（显示诊断版）”后才显示 Diagnostic `uart-full` ZIP。也可选择本地完整 ZIP 或解包目录；工具会显示实际来源路径，且两者同样强制要求 `SHA256SUMS.txt`。带 `ota` 标识的资产不会进入 UART 列表。

完整 UART 包必须包含：

- `firmware.json`；
- `SHA256SUMS.txt`；
- `boot2_bl616_*.bin`；
- `partition.bin`；
- 与清单的板型、速度和配置一致的应用 `.bin`。

工具拒绝路径穿越、重复文件、异常尺寸、SHA-256 不匹配、清单/文件名不一致、混入 `.bin.ota`/`.ota.json` 以及不完整 Release。在线资产还必须带 GitHub 提供的 SHA-256 digest。

UART 恢复步骤：

1. 连接 Ai-M61 板载 Type-C/CH340；原生 USB D+/D- 可保持连接。
2. 按住 **BOOT**，短按 **RESET**，再松开 BOOT，进入 BL616/BL618 ISP。
3. 选择检测到的 `USB-SERIAL CH340 (COMx)`；普通 `COM1` 不会进入候选。
4. 优先使用 460800 baud；握手失败可按提示以 115200 baud 重试。
5. 完成后按 RESET 正常启动。

只有安装/修复 WCH 驱动会请求管理员权限。驱动安装器从 WCH 官方地址下载，并在启动前验证固定 SHA-256 与 Authenticode 发布者。

#### 签名 OTA

固件刷写页同时提供签名 OTA 配置切换：

配置切换：

- Standard 与 Diagnostic 使用相同 HID 描述符长度，防止 Windows 在同 VID/PID 下缓存旧的 Feature Report 能力；Standard 不创建诊断任务、不响应 `0xFD`，也不采集内部桥接计时，适合日常最低开销运行；
- Diagnostic 仅在主机显式开启会话时每秒生成 `0xFD` 快照；
- “进入诊断模式（OTA）”和“恢复常用版（OTA）”只使用 Release 中带 `ota` 标识的纯 OTA ZIP；切换前自动停止所有测试输出；
- “选择本地签名 OTA ZIP”支持手动选择官方/CI 签名包；选择时即在电脑端校验 ZIP 结构、目标、RAW 头、正文 SHA-256 和固定 P-256 公钥签名，确认执行前再校验一次以防文件被替换；
- 同版本 `standard ↔ diagnostic` 允许切换。设备再次校验板型、HS、版本策略、正文 SHA-256 和 P-256 签名后才切换 A/B 槽；重启后工具等待 USB 重新枚举并核对版本/配置，诊断版还必须成功读取 `0xFD` 才报告完成。

## 运行态信息与诊断

`0xF8` Feature Report 通常返回 `版本|配置`，例如 `3.6.2|standard`。Standard 与 Diagnostic 都支持读取身份；Diagnostic 响应 `0xFD` v2 快照。v3.6.2 在 `0xFD` 返回全零时，通过当前真机已确认可读的 `0xF8` 通道发送选择命令；紧随其后的一次 `0xF8` GET 返回快照页，然后立即恢复固件身份语义。旧固件返回单独版本或 v1 快照时，工具按兼容模式显示，缺失字段标记为“不支持”，不会伪造为零。

性能压力测试的 Windows HID 间隔采样与固件快照完全解耦。引导式诊断未手动结束时，启动性能测试会安全结束引导输出、保留已经取得的结果并立即开始计时；快照失败只影响 M61 内部计数，不会阻止 HID 性能样本。若 3 秒内没有可计时报告，v1.4.2 会停止测试并给出明确错误。快照页和报告导出都会先做数据自检；启动占位页、页数/标志不完整、零时长却存在样本等矛盾数据会被拒绝，部分报告会列出实际存在和缺失的数据区段。

CLI：

```powershell
.\DS5Dongle-Flasher-Windows-v1.4.2.exe --device-info
.\DS5Dongle-Flasher-Windows-v1.4.2.exe --diagnostics
.\DS5Dongle-Flasher-Windows-v1.4.2.exe --list
.\DS5Dongle-Flasher-Windows-v1.4.2.exe --list-releases --board aim61 --usb-speed hs
.\DS5Dongle-Flasher-Windows-v1.4.2.exe --verify-release --release v3.6.2 --board aim61 --usb-speed hs
.\DS5Dongle-Flasher-Windows-v1.4.2.exe --release v3.6.2 --board aim61 --usb-speed hs --port COM3 --dry-run
```

CLI 的自动选择同样优先最新 AIM61 HS Standard。诊断版的显式选择和应用级 OTA 主要通过 GUI 完成。

## 本地打包

```powershell
python tools\package_firmware.py --board aim61 --usb-speed hs `
  --profile standard --version 3.6.2 `
  --firmware-dir firmware\aim61 --output-dir dist

python tools\package_firmware.py --board aim61 --usb-speed hs `
  --profile diagnostic --version 3.6.2 `
  --firmware-dir firmware\aim61 --output-dir dist
```

完整 `uart-full` ZIP 与签名 `ota` ZIP 不可互换。Boot2、分区表或两槽均损坏时，应用级 OTA 无法救援，必须使用 UART 完整刷写。

## 构建刷写器

`build.rs` 从固定的官方 Bouffalo SDK 2.3.31 工具路径嵌入 `BLFlashCommand.exe`、eflash loader 配置和 Flash 参数：

```powershell
$env:M61_BLFLASHCOMMAND = "C:\path\to\bouffalo_sdk\tools\bflb_tools\bouffalo_flash_cube\BLFlashCommand.exe"
cargo fmt --manifest-path tools\ds5dongle-flasher\Cargo.toml -- --check
cargo test --locked --manifest-path tools\ds5dongle-flasher\Cargo.toml
cargo build --locked --release --manifest-path tools\ds5dongle-flasher\Cargo.toml
```

正式文件为 `tools/ds5dongle-flasher/target/release/ds5dongle-flasher.exe`。发布版使用静态 CRT/unwind，不依赖旁置 `libunwind.dll`。
