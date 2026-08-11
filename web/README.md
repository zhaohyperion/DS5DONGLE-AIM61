# DS5Dongle Ai-M61 Device Studio

基于 WebHID 的 Ai-M61-32S-Kit 配置与安全 OTA 网页。页面只支持 Sony VID `054C`、PID `0CE6/0DF2`，并把 OTA 目标锁定为 Ai-M61 / 当前设备 USB Full-Speed 或 High-Speed / RAW `.bin.ota`；只允许同速升级。

## 本地运行

```text
npm install
npm run dev
npm test
```

WebHID 需要 Chrome 或 Edge，并且页面必须运行在 HTTPS 或 `localhost` 安全上下文中。

## 生产 OTA 信任配置

稳定版和测试版升级采用 fail-closed 策略。部署时必须注入：

- `NEXT_PUBLIC_OTA_KEY_ID`：受信任的发布公钥标识。
- `NEXT_PUBLIC_OTA_P256_PUBLIC_KEY`：65 字节 SEC1 uncompressed P-256 公钥（`04 || X || Y`）的标准 Base64。

网页校验 manifest 严格字段、目标、版本、完整容器 size/SHA-256、RAW body size/SHA-256 和 low-S ECDSA P-256 签名；设备使用同一信任根再次验签。生产私钥不得进入本目录、网页构建产物或仓库。

`public/og-v2.png` 是本项目的 Open Graph/Twitter 社交分享图。

项目保留 Sites 的 `sites()` Vite 插件和 `.openai/hosting.json`，但本次实现不执行部署。
