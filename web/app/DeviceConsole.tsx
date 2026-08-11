"use client";

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
} from "react";
import {
  DEFAULT_CONFIG,
  OtaState,
  compareVersions,
  identityRemap,
  inspectOtaImage,
  parseManifest,
  validateLocalOtaFilename,
  validateManifestImage,
  verifyManifestSignature,
  type DeviceConfig,
  type DeviceStatus,
  type OtaImageInfo,
  type OtaManifest,
  type OtaStatus,
  type OtaUsbSpeed,
  type RemapEntry,
} from "./lib/protocol";
import {
  Ds5DongleClient,
  deviceLabel,
  waitForReconnectedVersion,
  webHidAvailable,
} from "./lib/hid";
import {
  ButtonRemapEditor,
  FullConfigEditor,
  TelemetryStrip,
} from "./LegacyDevicePanels";
import { PwaRegistrar } from "./PwaRegistrar";
import { ReferenceCenter } from "./ReferenceCenter";
import { DiagnosticsPanel } from "./DiagnosticsPanel";
import { withHidMaintenanceLock } from "./lib/diagnostics";
import type { LegacyTelemetry } from "./lib/legacy-hid";

type Language = "zh" | "en";
type Tab = "config" | "remap" | "ota" | "diagnostics" | "reference";

interface FirmwareCandidate {
  bytes: Uint8Array;
  info: OtaImageInfo;
  name: string;
  manifest: OtaManifest | null;
  trusted: boolean;
}

const DEFAULT_MANIFEST_URLS: Record<OtaUsbSpeed, string> = {
  fs: "https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases/latest/download/DS5Dongle-aim61-fs-stable.ota.json",
  hs: "https://github.com/zhaohyperion/DS5DONGLE-AIM61/releases/latest/download/DS5Dongle-aim61-hs-stable.ota.json",
};

const ERROR_NAMES = [
  "OK",
  "BAD_MAGIC",
  "BAD_VERSION",
  "BAD_OPCODE",
  "BAD_STATE",
  "BAD_SESSION",
  "BAD_OFFSET",
  "BAD_LENGTH",
  "BAD_CRC",
  "BAD_TARGET",
  "TOO_LARGE",
  "QUEUE_FULL",
  "FLASH",
  "HASH",
  "HEADER",
  "INTERNAL",
  "AUTH_REQUIRED",
  "AUTH_FAILED",
  "KEY_MISSING",
  "TIMEOUT",
];

export function DeviceConsole() {
  const [language, setLanguage] = useState<Language>("zh");
  const [tab, setTab] = useState<Tab>("config");
  const [client, setClient] = useState<Ds5DongleClient | null>(null);
  const [firmwareVersion, setFirmwareVersion] = useState("—");
  const [deviceStatus, setDeviceStatus] = useState<DeviceStatus | null>(null);
  const [config, setConfig] = useState<DeviceConfig>(DEFAULT_CONFIG);
  const [remap, setRemap] = useState<RemapEntry[]>(identityRemap());
  const [otaCapability, setOtaCapability] = useState<OtaStatus | null>(null);
  const [busy, setBusy] = useState("");
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [candidate, setCandidate] = useState<FirmwareCandidate | null>(null);
  const [otaProfile, setOtaProfile] = useState<OtaUsbSpeed>("fs");
  const [manifestUrl, setManifestUrl] = useState(DEFAULT_MANIFEST_URLS.fs);
  const [otaProgress, setOtaProgress] = useState(0);
  const [otaPhase, setOtaPhase] = useState("idle");
  const [otaConfirmed, setOtaConfirmed] = useState(false);
  const [hidReady, setHidReady] = useState(false);
  const abortRef = useRef<AbortController | null>(null);

  const tr = useCallback(
    (zh: string, en: string) => (language === "zh" ? zh : en),
    [language],
  );

  const connected = Boolean(client?.device.opened);
  const product = client ? deviceLabel(client.device) : tr("未连接", "Not connected");
  const telemetry = useMemo<LegacyTelemetry | null>(() => deviceStatus ? {
    rssiDbm: deviceStatus.rssi,
    speakerActive: deviceStatus.speakerActive,
    microphoneActive: deviceStatus.microphoneActive,
    audioValid: deviceStatus.audioStatusValid,
    audioFlags:
      (deviceStatus.audioStatusValid ? 0x80 : 0) |
      (deviceStatus.speakerActive ? 0x02 : 0) |
      (deviceStatus.microphoneActive ? 0x01 : 0),
    batteryPercent: deviceStatus.batteryLevel,
    batteryState: deviceStatus.batteryState ?? 0xff,
  } : null, [deviceStatus]);
  const refreshDevice = useCallback(async (next: Ds5DongleClient) => {
    const [nextConfig, nextVersion, nextStatus, nextRemap] = await Promise.all([
      next.readConfig(),
      next.readFirmwareVersion(),
      next.readStatus(),
      next.readRemap(),
    ]);
    setConfig(nextConfig);
    setFirmwareVersion(nextVersion || "—");
    setDeviceStatus(nextStatus);
    setRemap(nextRemap);
    try {
      const capability = await next.otaCapability();
      const speed: OtaUsbSpeed = capability.usbSpeed === 1 ? "hs" : "fs";
      setOtaCapability(capability);
      setOtaProfile(speed);
      setManifestUrl(DEFAULT_MANIFEST_URLS[speed]);
      setCandidate(null);
      setOtaConfirmed(false);
    } catch {
      setOtaCapability(null);
    }
  }, []);

  const attach = useCallback(
    async (next: Ds5DongleClient) => {
      setBusy("connect");
      setError("");
      try {
        await next.open();
        await refreshDevice(next);
        setClient(next);
        setNotice(tr("设备已就绪", "Device ready"));
      } catch (reason) {
        await next.close().catch(() => undefined);
        setError(messageOf(reason));
      } finally {
        setBusy("");
      }
    },
    [refreshDevice, tr],
  );

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setHidReady(webHidAvailable());
      const storedLanguage = localStorage.getItem("ds5-language");
      if (storedLanguage === "en" || storedLanguage === "zh") setLanguage(storedLanguage);
      const requestedView = new URLSearchParams(window.location.search).get("view");
      if (requestedView === "config" || requestedView === "remap" || requestedView === "ota" || requestedView === "diagnostics" || requestedView === "reference") {
        setTab(requestedView);
      }
      const storedManifest = localStorage.getItem("ds5-manifest-url");
      if (storedManifest) setManifestUrl(storedManifest);
      Ds5DongleClient.authorized().then((authorized) => authorized && attach(authorized)).catch(() => undefined);
    }, 0);
    return () => window.clearTimeout(timer);
  }, [attach]);

  useEffect(() => {
    if (!client || !navigator.hid) return;
    const onDisconnect = (event: HIDConnectionEvent) => {
      if (event.device === client.device) {
        setClient(null);
        setOtaCapability(null);
        setNotice(tr("设备已断开", "Device disconnected"));
      }
    };
    navigator.hid.addEventListener("disconnect", onDisconnect);
    return () => navigator.hid?.removeEventListener("disconnect", onDisconnect);
  }, [client, tr]);

  useEffect(() => {
    if (!client || otaPhase === "transferring" || otaPhase === "verifying") return;
    const timer = window.setInterval(() => {
      client.readStatus().then(setDeviceStatus).catch(() => undefined);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [client, otaPhase]);

  const run = async (label: string, action: () => Promise<void>, success?: string) => {
    setBusy(label);
    setError("");
    setNotice("");
    try {
      await action();
      if (success) setNotice(success);
    } catch (reason) {
      setError(messageOf(reason));
    } finally {
      setBusy("");
    }
  };

  const chooseDevice = () => run("connect", async () => attach(await Ds5DongleClient.choose()));

  const selectOtaProfile = (speed: OtaUsbSpeed) => {
    if (otaCapability && otaCapability.usbSpeed !== (speed === "hs" ? 1 : 0)) {
      setError(tr("所选USB档位与当前设备不匹配", "Selected USB profile does not match the device"));
      return;
    }
    setOtaProfile(speed);
    setManifestUrl(DEFAULT_MANIFEST_URLS[speed]);
    setCandidate(null);
    setOtaConfirmed(false);
    setError("");
  };

  const assertManifestTarget = (manifest: OtaManifest) => {
    if (manifest.usb_speed !== otaProfile) {
      throw new Error(tr("清单USB档位与当前选择不匹配", "Manifest USB profile does not match the selection"));
    }
    if (otaCapability && otaCapability.usbSpeed !== (manifest.usb_speed === "hs" ? 1 : 0)) {
      throw new Error(tr("清单USB档位与当前设备不匹配", "Manifest USB profile does not match the device"));
    }
  };

  const loadLocalFirmware = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    await run("firmware", async () => {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const info = await inspectOtaImage(bytes);
      validateLocalOtaFilename(file.name, info, otaProfile);
      setCandidate({ bytes, info, name: file.name, manifest: null, trusted: false });
      setOtaConfirmed(false);
      setNotice(tr("本地镜像校验通过", "Local image verified"));
    });
  };

  const checkOnline = async () => {
    await run("online", async () => {
      localStorage.setItem("ds5-manifest-url", manifestUrl);
      const manifestResponse = await fetch(manifestUrl, { cache: "no-store" });
      if (!manifestResponse.ok) throw new Error(`Manifest HTTP ${manifestResponse.status}`);
      const manifest = parseManifest(await manifestResponse.json(), manifestResponse.url);
      assertManifestTarget(manifest);
      const imageResponse = await fetch(manifest.url, { cache: "no-store" });
      if (!imageResponse.ok) throw new Error(`Firmware HTTP ${imageResponse.status}`);
      const bytes = new Uint8Array(await imageResponse.arrayBuffer());
      const info = await validateManifestImage(manifest, bytes);
      await verifyManifestSignature(
        manifest,
        info,
        process.env.NEXT_PUBLIC_OTA_KEY_ID,
        process.env.NEXT_PUBLIC_OTA_P256_PUBLIC_KEY,
      );
      if (firmwareVersion !== "—" && compareVersions(manifest.version, firmwareVersion) <= 0) {
        throw new Error(tr("在线版本不高于设备当前版本", "Online version is not newer than the device"));
      }
      setCandidate({
        bytes,
        info,
        name: manifest.url.split("/").pop() || "firmware.bin.ota",
        manifest,
        trusted: true,
      });
      setOtaConfirmed(false);
      setNotice(tr("稳定版签名、文件和 OTA 头均已验证", "Release signature, file, and OTA header verified"));
    });
  };

  const attachLocalManifest = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file || !candidate) return;
    await run("manifest", async () => {
      const manifest = parseManifest(JSON.parse(await file.text()));
      assertManifestTarget(manifest);
      const info = await validateManifestImage(manifest, candidate.bytes);
      await verifyManifestSignature(
        manifest,
        info,
        process.env.NEXT_PUBLIC_OTA_KEY_ID,
        process.env.NEXT_PUBLIC_OTA_P256_PUBLIC_KEY,
      );
      setCandidate((current) => current && { ...current, info, manifest, trusted: true });
      setNotice(tr("本地镜像与生产签名清单匹配", "Local image matches the trusted release manifest"));
    });
  };

  const startOta = async () => {
    if (!client || !candidate || !otaCapability || !otaConfirmed) return;
    setError("");
    setNotice("");
    setOtaProgress(0);
    setOtaPhase("transferring");
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      if (!candidate.manifest) {
        throw new Error(tr("安全升级需要签名清单", "Safe OTA requires a signed manifest"));
      }
      assertManifestTarget(candidate.manifest);
      const finalStatus = await withHidMaintenanceLock(
        () => client.transferOta(
          candidate.bytes,
          candidate.info,
          candidate.manifest?.version || candidate.info.firmwareVersion,
          candidate.manifest?.signature || null,
          candidate.manifest.usb_speed,
          {
            signal: controller.signal,
            onProgress: (accepted, total, status) => {
              setOtaProgress(Math.min(100, (accepted / total) * 100));
              if (status.state === OtaState.Verifying) setOtaPhase("verifying");
            },
          },
        ),
        controller.signal,
      );
      setOtaPhase("reconnecting");
      const expected = candidate.manifest?.version || candidate.info.firmwareVersion;
      await new Promise((resolve) => window.setTimeout(resolve, finalStatus.rebootDelayMs + 300));
      await client.close().catch(() => undefined);
      const verifiedVersion = await waitForReconnectedVersion(expected);
      setFirmwareVersion(verifiedVersion);
      setOtaProgress(100);
      setOtaPhase("done");
      setNotice(tr(`升级完成，已确认版本 ${verifiedVersion}`, `Update complete; version ${verifiedVersion} confirmed`));
      const authorized = await Ds5DongleClient.authorized();
      if (authorized) {
        setClient(authorized);
        await refreshDevice(authorized);
      }
    } catch (reason) {
      setOtaPhase(controller.signal.aborted ? "cancelled" : "error");
      setError(messageOf(reason));
    } finally {
      abortRef.current = null;
    }
  };

  const otaStateText = useMemo(() => {
    const map: Record<string, [string, string]> = {
      idle: ["等待开始", "Ready"],
      transferring: ["正在传输", "Transferring"],
      verifying: ["设备正在校验并切换分区", "Verifying and switching slot"],
      reconnecting: ["设备正在重启，等待重新连接", "Rebooting and reconnecting"],
      done: ["升级完成", "Complete"],
      cancelled: ["升级已取消", "Cancelled"],
      error: ["升级失败，原分区保持可启动", "Update failed; original slot remains bootable"],
    };
    const item = map[otaPhase] || map.idle;
    return language === "zh" ? item[0] : item[1];
  }, [language, otaPhase]);

  return (
    <main className="app-shell">
      <header className="topbar">
        <a className="brand" href="#top" aria-label="DS5Dongle home">
          <span className="brand-mark">D5</span>
          <span>
            <strong>DS5Dongle</strong>
            <small>Ai-M61 Device Studio</small>
          </span>
        </a>
        <div className="header-actions">
          <span className={`connection-pill ${connected ? "is-online" : ""}`}>
            <i /> {connected ? tr("已连接", "Connected") : tr("离线", "Offline")}
          </span>
          <button
            className="language-button"
            type="button"
            onClick={() => {
              const next = language === "zh" ? "en" : "zh";
              setLanguage(next);
              localStorage.setItem("ds5-language", next);
            }}
            aria-label={tr("切换为英文", "Switch to Chinese")}
          >
            {language === "zh" ? "EN" : "中文"}
          </button>
        </div>
      </header>

      <section className="hero" id="top">
        <div>
          <p className="eyebrow">WEBHID · LOCAL FIRST · A/B OTA</p>
          <h1>{tr("把适配器调到刚刚好。", "Tune your adapter, precisely.")}</h1>
          <p className="hero-copy">
            {tr(
              "配置直接在浏览器与设备之间传输；固件升级先验证目标、大小、双重 SHA-256 与发布签名，再写入非活动分区。",
              "Configuration stays between browser and device. Firmware is checked for target, size, two SHA-256 hashes, and release signature before the inactive slot is written.",
            )}
          </p>
        </div>
        <div className="hero-target" aria-label="target hardware">
          <span>{tr("当前目标", "Target")}</span>
          <strong>Ai-M61-32S-Kit</strong>
          <small>BL618 · USB {otaProfile === "hs" ? "High-Speed" : "Full-Speed"} · 4 MB Flash</small>
        </div>
      </section>

      {!hidReady && (
        <div className="browser-warning" role="alert">
          <strong>{tr("此浏览器无法连接设备。", "This browser cannot connect to the device.")}</strong>
          <span>{tr("请使用最新版 Chrome 或 Edge，并通过 HTTPS 或 localhost 打开。", "Use current Chrome or Edge over HTTPS or localhost.")}</span>
        </div>
      )}

      <section className="device-card" aria-label={tr("设备连接", "Device connection")}>
        <div className="device-identity">
          <div className="device-icon"><span /></div>
          <div>
            <span className="section-kicker">{tr("设备", "DEVICE")}</span>
            <h2>{product}</h2>
          </div>
        </div>
        <TelemetryStrip
          status={telemetry}
          firmwareVersion={firmwareVersion}
          otaReady={Boolean(otaCapability)}
          language={language}
        />
        <button className="primary-button connect-button" type="button" disabled={!hidReady || Boolean(busy)} onClick={chooseDevice}>
          {busy === "connect" ? tr("正在连接…", "Connecting…") : connected ? tr("更换设备", "Change device") : tr("连接适配器", "Connect adapter")}
        </button>
      </section>

      {(notice || error) && (
        <div className={`message ${error ? "is-error" : "is-success"}`} role={error ? "alert" : "status"}>
          <span>{error ? "!" : "✓"}</span>
          {error || notice}
          <button type="button" onClick={() => { setError(""); setNotice(""); }} aria-label={tr("关闭", "Dismiss")}>×</button>
        </div>
      )}

      <nav className="tabs" aria-label={tr("工作区", "Workspace")}>
        {([
          ["config", tr("设备配置", "Device config")],
          ["remap", tr("按键映射", "Button mapping")],
          ["ota", tr("固件更新", "Firmware update")],
          ["diagnostics", tr("一键诊断", "Diagnostics")],
          ["reference", tr("帮助与更新", "Guide & updates")],
        ] as Array<[Tab, string]>).map(([id, label]) => (
          <button key={id} type="button" className={tab === id ? "is-active" : ""} onClick={() => setTab(id)}>
            {label}
            {id === "ota" && <span className="new-dot" />}
          </button>
        ))}
      </nav>

      {tab === "config" && (
        <section className="workspace">
          <FullConfigEditor
            config={config}
            onConfigChange={setConfig}
            onRead={() => client ? run("read", async () => setConfig(await client.readConfig()), tr("已读取设备配置", "Configuration read")) : Promise.resolve()}
            onReset={() => setConfig({ ...DEFAULT_CONFIG })}
            onApply={(next) => client ? run("apply", () => client.applyConfig(next), tr("配置已应用", "Configuration applied")) : Promise.resolve()}
            onSave={(next) => client ? run("save", async () => {
              await client.applyConfig(next);
              await client.saveConfig();
            }, tr("配置已保存到 Flash", "Saved to flash")) : Promise.resolve()}
            onReconnect={() => client ? run("reconnect", () => client.reconnectUsb(), tr("设备正在重连 USB", "USB is reconnecting")) : Promise.resolve()}
            busy={busy}
            disabled={!client}
            language={language}
          />
        </section>
      )}

      {tab === "remap" && (
        <section className="workspace">
          <ButtonRemapEditor
            remap={remap}
            onRemapChange={setRemap}
            onRead={() => client ? run("remap-read", async () => setRemap(await client.readRemap()), tr("映射已读取", "Mapping read")) : Promise.resolve()}
            onReset={() => client ? run("remap-reset", async () => {
              await client.resetRemap();
              setRemap(identityRemap());
            }, tr("映射已恢复默认", "Mapping reset")) : Promise.resolve()}
            onSave={(next) => client ? run("remap-save", () => client.saveRemap(next), tr("按键映射已保存", "Mapping saved")) : Promise.resolve()}
            busy={busy}
            disabled={!client}
            language={language}
          />
        </section>
      )}

      {tab === "ota" && (
        <section className="workspace ota-workspace">
          <div className="ota-intro">
            <div>
              <span className="section-kicker">SAFE A/B UPDATE</span>
              <h2>{tr("固件更新", "Firmware update")}</h2>
              <p>{tr("只写入非活动分区；完整校验成功后才切换启动槽。更新时手柄音频会进入维护模式。", "Only the inactive slot is written and activated after full verification. Controller audio enters maintenance mode during update.")}</p>
            </div>
            <div className={`ota-capability ${otaCapability ? "is-ready" : ""}`}>
              <span>{tr("设备能力", "DEVICE CAPABILITY")}</span>
              <strong>{otaCapability ? tr("可以安全升级", "Ready for safe OTA") : tr("请连接支持 OTA 的固件", "Connect OTA-capable firmware")}</strong>
              <small>{otaCapability ? `Ai-M61 · ${otaCapability.usbSpeed === 1 ? "HS" : "FS"} · A/B · SHA-256 · RAW · ${otaCapability.maxImageSize.toLocaleString()} B` : "0xFC capability probe"}</small>
            </div>
          </div>

          <div className="ota-columns">
            <article className="firmware-source-card">
              <div className="card-number">01</div>
              <h3>{tr("选择固件来源", "Choose firmware source")}</h3>
              <p>{tr("先确认 USB 档位，再选择同档位的签名发布或本地镜像。FS 与 HS 清单、签名和设备端目标必须完全一致。", "Choose the USB profile first, then use a signed release or local image for the same profile. The manifest, signature, and device target must all agree.")}</p>
              <div className="firmware-profile-grid" aria-label={tr("固件档位", "Firmware profile")}>
                <button
                  className={`firmware-profile ${otaProfile === "fs" ? "is-active" : ""}`}
                  type="button"
                  aria-pressed={otaProfile === "fs"}
                  onClick={() => selectOtaProfile("fs")}
                >
                  <span>USB FS · 12 Mb/s</span>
                  <strong>{tr("普通 / 全速", "Standard / Full-Speed")}</strong>
                  <small>{tr("Ai-M61 已验证 · 可升级", "Validated for Ai-M61 · available")}</small>
                </button>
                <button
                  className={`firmware-profile ${otaProfile === "hs" ? "is-active" : ""}`}
                  type="button"
                  aria-pressed={otaProfile === "hs"}
                  onClick={() => selectOtaProfile("hs")}
                >
                  <span>USB HS · 480 Mb/s</span>
                  <strong>{tr("高速", "High-Speed")}</strong>
                  <small>{tr("Ai-M61 已验证 · 可升级", "Validated for Ai-M61 · available")}</small>
                </button>
              </div>
              <div className="firmware-profile-note">
                <strong>{tr("同速升级保护", "Same-speed update protection")}</strong>
                <span>{tr("连接设备后会自动锁定其 FS/HS 档位；网页签名校验和设备端都会拒绝另一档位或其他板型镜像。", "After connection, the device FS/HS profile is locked automatically; both web signature validation and firmware reject the other profile or board.")}</span>
              </div>
              <label className="upload-zone">
                <input type="file" accept=".ota,.bin.ota,application/octet-stream" onChange={loadLocalFirmware} disabled={Boolean(busy) || otaPhase === "transferring"} />
                <span className="upload-icon">↑</span>
                <strong>{tr("选择本地 RAW .bin.ota", "Choose local RAW .bin.ota")}</strong>
                <small>{tr("不会上传到服务器", "Never uploaded to a server")}</small>
              </label>
              {candidate && !candidate.trusted && (
                <label className="sidecar-upload">
                  <input type="file" accept=".json,application/json" onChange={attachLocalManifest} />
                  <span>{tr("为本地镜像选择签名 .ota.json 清单", "Attach signed .ota.json manifest for local image")}</span>
                </label>
              )}
              <div className="or-divider"><span>{tr("或检查在线稳定版", "OR CHECK STABLE RELEASE")}</span></div>
              <label className="url-field">
                <span>Manifest URL</span>
                <input value={manifestUrl} onChange={(event) => setManifestUrl(event.target.value)} spellCheck={false} />
              </label>
              <button className="secondary-button wide" type="button" disabled={Boolean(busy) || otaPhase === "transferring"} onClick={checkOnline}>{busy === "online" ? tr("正在验证…", "Verifying…") : tr("检查并下载稳定版", "Check & download stable")}</button>
            </article>

            <article className="firmware-source-card">
              <div className="card-number">02</div>
              <h3>{tr("验证与安装", "Verify and install")}</h3>
              {!candidate ? (
                <div className="empty-firmware">
                  <span>◌</span>
                  <strong>{tr("尚未选择固件", "No firmware selected")}</strong>
                  <p>{tr("选择文件后，这里会显示目标、版本和所有校验结果。", "Target, version, and verification results appear here after selection.")}</p>
                </div>
              ) : (
                <div className="firmware-details">
                  <div className="firmware-name"><span>{candidate.trusted ? "✓" : "L"}</span><div><strong>{candidate.name}</strong><small>{candidate.trusted ? tr("已验证的在线发布", "Verified online release") : tr("用户选择的本地镜像", "User-selected local image")}</small></div></div>
                  <dl>
                    <div><dt>{tr("目标", "Target")}</dt><dd>Ai-M61 · {(candidate.manifest?.usb_speed || otaProfile).toUpperCase()}</dd></div>
                    <div><dt>{tr("版本", "Version")}</dt><dd>{candidate.info.firmwareVersion}</dd></div>
                    <div><dt>{tr("大小", "Size")}</dt><dd>{formatBytes(candidate.info.fileSize)}</dd></div>
                    <div><dt>RAW Body SHA-256</dt><dd title={candidate.info.bodySha256}>{shortHash(candidate.info.bodySha256)}</dd></div>
                    <div><dt>{tr("容器 SHA-256", "Container SHA-256")}</dt><dd title={candidate.info.fileSha256}>{shortHash(candidate.info.fileSha256)}</dd></div>
                    <div><dt>{tr("发布签名", "Release signature")}</dt><dd className={candidate.trusted ? "verified" : "manual"}>{candidate.trusted ? tr("可信", "Trusted") : tr("未授权", "Not authorized")}</dd></div>
                  </dl>
                  <label className="confirmation"><input type="checkbox" checked={otaConfirmed} onChange={(event) => setOtaConfirmed(event.target.checked)} /><span>{tr("我确认设备为 Ai-M61-32S-Kit、USB档位匹配，升级期间不会拔线。", "I confirm this is an Ai-M61-32S-Kit with a matching USB profile and will keep it connected.")}</span></label>
                </div>
              )}
              <div className="progress-block">
                <div><span>{otaStateText}</span><strong>{otaProgress.toFixed(1)}%</strong></div>
                <progress max="100" value={otaProgress} />
                <small>{tr("进度只按设备确认的连续字节推进；断线或超时后必须重新开始。", "Progress follows device-confirmed contiguous bytes; reconnect or timeout requires a fresh update.")}</small>
              </div>
              {otaPhase === "transferring" || otaPhase === "verifying" ? (
                <button className="danger-button wide" type="button" onClick={() => abortRef.current?.abort()}>{tr("取消升级", "Cancel update")}</button>
              ) : (
                <button className="primary-button wide" type="button" disabled={!client || !otaCapability || !candidate?.trusted || !otaConfirmed || Boolean(busy)} onClick={startOta}>{tr("开始安全升级", "Start safe update")}</button>
              )}
            </article>
          </div>

          <div className="safety-strip">
            <SafetyItem index="A" title={tr("目标锁定", "Target locked")} text={`Ai-M61 · ${otaProfile.toUpperCase()} · RAW`} />
            <SafetyItem index="B" title={tr("双重哈希", "Two hashes")} text="Container + firmware body" />
            <SafetyItem index="C" title={tr("非活动分区", "Inactive slot")} text="Power-loss safe before switch" />
            <SafetyItem index="D" title={tr("重启确认", "Reboot verification")} text="Reconnect + version check" />
          </div>
        </section>
      )}

      {tab === "reference" && (
        <section className="workspace reference-workspace">
          <ReferenceCenter locale={language} />
        </section>
      )}

      {tab === "diagnostics" && (
        <section className="workspace diagnostics-workspace">
          <DiagnosticsPanel
            device={client?.device ?? null}
            firmwareVersion={firmwareVersion}
            otaActive={otaPhase === "transferring" || otaPhase === "verifying" || otaPhase === "reconnecting"}
            language={language}
          />
        </section>
      )}

      <aside className="persistent-pwa" aria-label={tr("网页应用状态", "Web app status")}>
        <PwaRegistrar locale={language} />
      </aside>

      <footer>
        <span>DS5DONGLE-AIM61 / zhaohyperion</span>
        <span>{tr("配置数据不经过云端", "Configuration never passes through the cloud")}</span>
        <a href="https://github.com/zhaohyperion/DS5DONGLE-AIM61" target="_blank" rel="noreferrer">GitHub ↗</a>
      </footer>
    </main>
  );
}

function SafetyItem({ index, title, text }: { index: string; title: string; text: string }) {
  return <div><span>{index}</span><p><strong>{title}</strong><small>{text}</small></p></div>;
}

function formatBytes(bytes: number): string {
  return bytes > 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(2)} MiB` : `${(bytes / 1024).toFixed(1)} KiB`;
}

function shortHash(hash: string): string {
  return `${hash.slice(0, 10)}…${hash.slice(-8)}`;
}

function messageOf(reason: unknown): string {
  if (reason instanceof DOMException && reason.name === "NotFoundError") return "未选择设备";
  if (reason instanceof DOMException && reason.name === "AbortError") return "操作已取消";
  return reason instanceof Error ? reason.message : String(reason);
}

export function otaErrorName(code: number): string {
  return ERROR_NAMES[code] || `UNKNOWN_${code}`;
}
