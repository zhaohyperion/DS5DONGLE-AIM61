import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

const geistSans = Geist({ variable: "--font-geist-sans", subsets: ["latin"] });
const geistMono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] });

export const metadata: Metadata = {
  title: "DS5Dongle · Ai-M61 设备工作台",
  description: "Ai-M61 DS5Dongle 的 WebHID 配置、按键映射、实时遥测与签名 A/B OTA 工具。",
  manifest: "/manifest.webmanifest",
  icons: { icon: "/favicon.svg", shortcut: "/favicon.svg" },
  openGraph: {
    type: "website",
    title: "DS5Dongle · Ai-M61 设备工作台",
    description: "本地配置、按键映射与实时遥测，并通过签名 A/B 分区安全升级 Ai-M61 固件。",
    images: [{ url: "/og-v2.png", width: 1536, height: 1024, alt: "DS5Dongle Ai-M61 Device Studio" }],
  },
  twitter: {
    card: "summary_large_image",
    title: "DS5Dongle · Ai-M61 设备工作台",
    description: "Ai-M61 DS5Dongle 的 WebHID 配置、按键映射、遥测与安全 OTA 工具。",
    images: ["/og-v2.png"],
  },
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  themeColor: "#0b1018",
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="zh-CN">
      <body className={`${geistSans.variable} ${geistMono.variable}`}>{children}</body>
    </html>
  );
}
