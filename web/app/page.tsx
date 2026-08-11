import type { Metadata } from "next";
import { DeviceConsole } from "./DeviceConsole";

export const metadata: Metadata = {
  title: "DS5Dongle · Ai-M61 设备工作台",
  description: "通过 WebHID 配置 Ai-M61 DS5Dongle，并安全检查和安装 OTA 固件。",
};

export default function Home() {
  return <DeviceConsole />;
}
