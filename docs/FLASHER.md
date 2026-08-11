# DS5Dongle Windows 一键刷写器

`DS5Dongle-Flasher-Windows.exe` 是面向 Ai-M61-32S-Kit、LCTech BL616 和 Sipeed M0S Dock 的单文件 GUI/CLI 刷写器。它可以读取本仓库 GitHub Release，也可以刷写本地完整固件 ZIP 或解包目录。

## 普通用户：在线选择固件并刷写

1. 从 [DS5DONGLE-AIM61 Releases](https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases/latest) 下载 `DS5Dongle-Flasher-Windows.exe`。
2. 连接开发板的 UART/下载串口。Ai-M61 使用板载 Type-C/CH340；另外两种板卡可使用支持 3.3 V 电平的 USB-TTL。
3. 按住 BOOT，短按 RESET 后松开 BOOT，使 BL616/BL618 进入 UART ISP。
4. 启动刷写器，选择物理板型、USB Full-Speed/High-Speed 和正确 COM 口。
5. 在“在线 Release”中选择版本；默认推荐 Ai-M61 Full-Speed 稳定版。
6. 开始刷写。工具会下载并验证完整包，然后写入 Boot2、分区表和应用固件。
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

刷写器拒绝路径穿越、重复文件、异常尺寸、SHA-256 不匹配、清单字段错误及板型/速度与文件名不一致。完整线刷包与网页 OTA `.bin.ota` 不可互换。

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
- Full-Speed 是 Ai-M61 飞线场景的默认选择；High-Speed 只适合已经验证的短差分布线。
- 刷写器只显示带 GitHub SHA-256 digest、且包含完整线刷资产的 Release。
- 日志与临时运行目录会保留到刷写完成或失败，便于定位问题；分享前请检查其中是否含本机路径或串口信息。
- 两槽 OTA、Boot2 或分区表损坏时，必须使用这里的 UART 完整刷写恢复，网页 OTA 无法救援。
