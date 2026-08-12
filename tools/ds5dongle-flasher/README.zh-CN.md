# DS5Dongle Windows 一键刷写器

DS5DONGLE-AIM61 的单文件 Windows GUI/CLI 刷写器与原生测试中心。默认首选稳定版 AI-M61 Full-Speed 固件，并兼容仓库生成的其他完整固件包；支持通过 USB HID 读取当前设备固件版本及七页 `0xFD` 运行态诊断（兼容旧六页固件）、M61 内部桥接延迟与 Windows HID 抖动分析、保存本地 JSON 诊断包、内置按键/摇杆/触摸/传感器/灯效/震动/自适应扳机/USB 音频测试、仅允许 CH340 进入刷写候选，以及 GitHub Release 在线下载、本地 ZIP/目录、SHA256 校验、BOOT+RESET 引导和 460800/115200 波特率失败重试。

固件包格式、构建和发布方法见 [`../../docs/FLASHER.md`](../../docs/FLASHER.md)。
