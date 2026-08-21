# DS5Dongle AIM61 工具中心

`DS5Dongle-Flasher-Windows-v1.4.0.exe` 是 DS5DONGLE-AIM61 的单文件 Windows GUI/CLI 工具。四个页签依次为测试中心、按键映射、设备调试和固件刷写；默认只显示最新 Ai-M61 High-Speed 常用版，诊断版位于高级区域。

工具支持 19 控件一对多映射和宏、DS5 图形实时输入、灯效/限强度震动/自适应扳机/手柄声音测试、WASAPI 自动选择 M61/DualSense 麦克风并保存 WAV、五分钟压力测试、统一 JSON 报告，以及诊断版七页 `0xFD` 快照和内部桥接延迟。常用版会被识别为无快照配置，不再误报 0xFD 读取故障。

刷写页只允许可用 CH340 进入 UART 候选；在线和本地完整包都验证目标、结构和 `SHA256SUMS.txt`。本地签名 OTA ZIP 会先在电脑端使用固定 P-256 公钥验签，设备端再独立验签；重启后必须重新枚举并匹配固件版本/配置，诊断版还要成功读取快照，工具才报告升级完成。

固件包格式、构建和发布方法见 [`../../docs/FLASHER.md`](../../docs/FLASHER.md)。
