"use client";

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  BoundedTextBuffer,
  Ch340SerialMonitor,
  DIAGNOSTIC_MAX_EVENTS,
  DIAGNOSTIC_MAX_SNAPSHOTS,
  DIAGNOSTIC_UART_VIEW_LIMIT,
  DiagnosticHidClient,
  createDiagnosticBundle,
  downloadDiagnosticBundle,
  webSerialAvailable,
  type DiagnosticEvent,
  type DiagnosticSnapshot,
} from "./lib/diagnostics";

type Language = "zh" | "en";

export interface DiagnosticsPanelProps {
  device: HIDDevice | null;
  firmwareVersion?: string | null;
  otaActive: boolean;
  language?: Language;
}

interface UartView {
  text: string;
  byteLength: number;
  droppedBytes: number;
}

export function DiagnosticsPanel({
  device,
  firmwareVersion = null,
  otaActive,
  language = "zh",
}: DiagnosticsPanelProps) {
  const hidClient = useMemo(() => device ? new DiagnosticHidClient(device) : null, [device]);
  const [snapshots, setSnapshots] = useState<DiagnosticSnapshot[]>([]);
  const snapshotsRef = useRef<DiagnosticSnapshot[]>([]);
  const snapshotsDropped = useRef(0);
  const [events, setEvents] = useState<DiagnosticEvent[]>([]);
  const eventsRef = useRef<DiagnosticEvent[]>([]);
  const eventsDropped = useRef(0);
  const [autoCapture, setAutoCapture] = useState(true);
  const [captureBusy, setCaptureBusy] = useState(false);
  const captureInFlight = useRef(false);
  const captureAbort = useRef<AbortController | null>(null);
  const mounted = useRef(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const serialSupported = useSyncExternalStore(subscribeNever, webSerialAvailable, () => false);
  const [serialConnected, setSerialConnected] = useState(false);
  const serialMonitor = useRef<Ch340SerialMonitor | null>(null);
  const uartBuffer = useRef(new BoundedTextBuffer());
  const uartDirty = useRef(false);
  const [uartView, setUartView] = useState<UartView>({ text: "", byteLength: 0, droppedBytes: 0 });
  const [redactExport, setRedactExport] = useState(true);

  const text = useCallback(
    (zh: string, en: string) => language === "zh" ? zh : en,
    [language],
  );

  const recordEvent = useCallback((event: DiagnosticEvent) => {
    const unbounded = [event, ...eventsRef.current];
    if (unbounded.length > DIAGNOSTIC_MAX_EVENTS) eventsDropped.current += unbounded.length - DIAGNOSTIC_MAX_EVENTS;
    const next = unbounded.slice(0, DIAGNOSTIC_MAX_EVENTS);
    eventsRef.current = next;
    setEvents(next);
  }, []);

  useEffect(() => {
    const flush = window.setInterval(() => {
      if (!uartDirty.current) return;
      uartDirty.current = false;
      setUartView({
        text: uartBuffer.current.text.slice(-DIAGNOSTIC_UART_VIEW_LIMIT),
        byteLength: uartBuffer.current.byteLength,
        droppedBytes: uartBuffer.current.droppedBytes,
      });
    }, 250);
    return () => window.clearInterval(flush);
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      captureAbort.current?.abort(abortError("诊断页面已关闭"));
      void serialMonitor.current?.disconnect();
    };
  }, []);

  useEffect(() => () => {
    captureAbort.current?.abort(abortError("诊断设备已更换"));
  }, [hidClient]);

  useEffect(() => {
    if (otaActive) captureAbort.current?.abort(abortError("OTA 进行中，诊断读取已暂停"));
  }, [otaActive]);

  const captureNow = useCallback(async () => {
    if (!hidClient || otaActive || captureInFlight.current) return;
    const controller = new AbortController();
    captureAbort.current = controller;
    captureInFlight.current = true;
    setCaptureBusy(true);
    setError("");
    try {
      const snapshot = await hidClient.capture(controller.signal);
      if (!mounted.current || controller.signal.aborted) return;
      const previous = snapshotsRef.current.at(-1) ?? null;
      if (
        previous?.snapshotSeq !== snapshot.snapshotSeq ||
        previous?.monotonicMs !== snapshot.monotonicMs
      ) {
        const unbounded = [...snapshotsRef.current, snapshot];
        if (unbounded.length > DIAGNOSTIC_MAX_SNAPSHOTS) snapshotsDropped.current += unbounded.length - DIAGNOSTIC_MAX_SNAPSHOTS;
        const next = unbounded.slice(-DIAGNOSTIC_MAX_SNAPSHOTS);
        snapshotsRef.current = next;
        setSnapshots(next);
        for (const event of deriveSnapshotEvents(previous, snapshot, language)) recordEvent(event);
      }
      setNotice(text(`已抓取快照 #${snapshot.snapshotSeq}`, `Captured snapshot #${snapshot.snapshotSeq}`));
    } catch (reason) {
      if (!mounted.current || isAbortReason(reason)) return;
      const message = messageOf(reason);
      setError(message);
      setAutoCapture(false);
      recordEvent({
        timestamp: new Date().toISOString(),
        source: "webhid",
        kind: "capture-error",
        severity: "error",
        message,
      });
    } finally {
      if (captureAbort.current === controller) captureAbort.current = null;
      captureInFlight.current = false;
      if (mounted.current) setCaptureBusy(false);
    }
  }, [hidClient, language, otaActive, recordEvent, text]);

  useEffect(() => {
    if (!autoCapture || !hidClient || otaActive) return;
    void captureNow();
    const timer = window.setInterval(() => void captureNow(), 1_000);
    return () => window.clearInterval(timer);
  }, [autoCapture, captureNow, hidClient, otaActive]);

  const connectSerial = async () => {
    if (!serialSupported || serialConnected) return;
    setError("");
    setNotice("");
    const monitor = new Ch340SerialMonitor({
      onText: (chunk) => {
        uartBuffer.current.append(chunk);
        uartDirty.current = true;
      },
      onDisconnect: () => setSerialConnected(false),
      onError: (reason) => {
        const message = messageOf(reason);
        setError(message);
        recordEvent({
          timestamp: new Date().toISOString(),
          source: "uart",
          kind: "serial-error",
          severity: "error",
          message,
        });
      },
    });
    serialMonitor.current = monitor;
    try {
      await monitor.connect();
      setSerialConnected(true);
      setNotice(text("CH340 串口已连接：115200 8N1", "CH340 connected: 115200 8N1"));
      recordEvent({
        timestamp: new Date().toISOString(),
        source: "uart",
        kind: "serial-connected",
        severity: "info",
        message: "CH340 115200 8N1",
      });
    } catch (reason) {
      serialMonitor.current = null;
      setError(messageOf(reason));
    }
  };

  const disconnectSerial = async () => {
    const monitor = serialMonitor.current;
    serialMonitor.current = null;
    await monitor?.disconnect();
    setSerialConnected(false);
    setNotice(text("串口已断开", "Serial disconnected"));
    recordEvent({
      timestamp: new Date().toISOString(),
      source: "uart",
      kind: "serial-disconnected",
      severity: "info",
      message: text("用户断开串口", "Serial disconnected by user"),
    });
  };

  const clearUart = () => {
    uartBuffer.current.clear();
    uartDirty.current = false;
    setUartView({ text: "", byteLength: 0, droppedBytes: 0 });
  };

  const exportBundle = () => {
    const bundle = createDiagnosticBundle({
      firmwareVersion,
      device,
      snapshots,
      events,
      uartText: uartBuffer.current.text,
      uartDroppedBytes: uartBuffer.current.droppedBytes,
      snapshotsDropped: snapshotsDropped.current,
      eventsDropped: eventsDropped.current,
      uartConnected: serialConnected,
      redact: redactExport,
    });
    const filename = downloadDiagnosticBundle(bundle);
    setNotice(text(`已导出 ${filename}`, `Exported ${filename}`));
  };

  const latest = snapshots.at(-1) ?? null;
  const canExport = snapshots.length > 0 || events.length > 0 || uartView.text.length > 0;

  return (
    <section className="diagnostics-center" aria-label={text("一键诊断中心", "Diagnostics center")}>
      <div className="section-heading">
        <div>
          <span className="section-kicker">0xFD · LOCAL DIAGNOSTICS</span>
          <h2>{text("一键诊断中心", "One-click diagnostics")}</h2>
        </div>
        <p>{text("每秒读取设备原子快照，并可合并 CH340 串口日志。所有处理均在本地浏览器完成，不会上传。", "Read atomic device snapshots once per second and optionally merge CH340 UART logs. Everything stays in this browser and is never uploaded.")}</p>
      </div>

      <div className="diagnostics-privacy" role="note">
        <strong>{text("本地处理", "LOCAL ONLY")}</strong>
        <span>{text("诊断数据不会发送到服务器；默认导出会脱敏蓝牙地址、USB 序列号、SSP 密钥和指针。", "No diagnostic data is sent to a server. Exports redact Bluetooth addresses, USB serials, SSP passkeys, and pointers by default.")}</span>
      </div>

      <div className="diagnostics-toolbar">
        <div>
          <button className="primary-button" type="button" disabled={!hidClient || otaActive || captureBusy} onClick={() => void captureNow()}>
            {captureBusy ? text("正在抓取…", "Capturing…") : text("立即抓取", "Capture now")}
          </button>
          <label className="diagnostics-auto">
            <input type="checkbox" checked={autoCapture} disabled={!hidClient || otaActive} onChange={(event) => setAutoCapture(event.target.checked)} />
            <span>{text("每秒自动抓取", "Capture every second")}</span>
          </label>
        </div>
        <span className={`diagnostics-poll-state ${otaActive ? "is-paused" : ""}`}>
          {otaActive
            ? text("OTA 进行中：诊断读取已暂停", "OTA active: diagnostic reads paused")
            : !device
              ? text("请先连接适配器", "Connect the adapter first")
              : autoCapture
                ? text("1 Hz 自动抓取", "1 Hz auto capture")
                : text("手动抓取", "Manual capture")}
        </span>
      </div>

      {(error || notice) && (
        <div className={`message ${error ? "is-error" : "is-success"}`} role={error ? "alert" : "status"}>
          <span>{error ? "!" : "✓"}</span>
          {error || notice}
          <button type="button" onClick={() => { setError(""); setNotice(""); }} aria-label={text("关闭", "Dismiss")}>×</button>
        </div>
      )}

      <div className="diagnostics-summary">
        <DiagnosticMetric label={text("快照序号", "Snapshot")} value={latest ? `#${latest.snapshotSeq}` : "—"} />
        <DiagnosticMetric label={text("运行时间", "Uptime")} value={latest ? duration(latest.uptimeMs) : "—"} />
        <DiagnosticMetric label={text("空闲堆 / 采样最低", "Free / sampled min")} value={latest ? `${formatBytes(latest.heapFreeBytes)} / ${formatBytes(latest.heapMinFreeBytes)}` : "—"} warn={Boolean(latest && latest.heapFreeBytes < 32 * 1024)} />
        <DiagnosticMetric label="RSSI" value={latest?.btRssiDbm == null ? "—" : `${latest.btRssiDbm} dBm`} />
        <DiagnosticMetric label={text("USB 完成", "USB complete")} value={latest ? latest.usbInCompleted.toLocaleString() : "—"} />
        <DiagnosticMetric label={text("丢失/压力", "Loss/pressure")} value={latest ? latest.lossPressure.toLocaleString() : "—"} warn={Boolean(latest?.lossPressure)} />
        <DiagnosticMetric label={text("麦克风欠载", "Mic underrun")} value={latest ? latest.micUnderruns.toLocaleString() : "—"} warn={Boolean(latest?.micUnderruns)} />
        <DiagnosticMetric label="OTA" value={latest ? `${latest.otaState} / ${latest.otaError}` : "—"} warn={Boolean(latest?.otaError)} />
      </div>

      <div className="diagnostics-columns">
        <article className="panel diagnostics-snapshots">
          <div className="panel-heading">
            <h3>{text("结构化快照", "Structured snapshots")}</h3>
            <p>{text(`内存中保留最近 ${DIAGNOSTIC_MAX_SNAPSHOTS} 份`, `Keeps the latest ${DIAGNOSTIC_MAX_SNAPSHOTS} in memory`)}</p>
          </div>
          <div className="panel-body">
            {snapshots.length === 0 ? (
              <div className="diagnostics-empty">{text("尚无快照。连接支持 0xFD 的固件后点击抓取。", "No snapshots yet. Connect 0xFD-capable firmware and capture.")}</div>
            ) : (
              <div className="diagnostics-table-wrap">
                <table className="diagnostics-table">
                  <thead><tr><th>#</th><th>{text("运行时间", "Uptime")}</th><th>{text("堆内存", "Heap")}</th><th>BT RX/TX</th><th>{text("异常", "Pressure")}</th></tr></thead>
                  <tbody>
                    {[...snapshots].reverse().slice(0, 20).map((snapshot) => (
                      <tr key={`${snapshot.snapshotSeq}-${snapshot.monotonicMs}`}>
                        <td>{snapshot.snapshotSeq}</td>
                        <td>{duration(snapshot.uptimeMs)}</td>
                        <td title={text("当前 / 诊断任务启动以来的采样最低值", "Current / lowest sample since diagnostics started")}>{formatBytes(snapshot.heapFreeBytes)} / {formatBytes(snapshot.heapMinFreeBytes)}</td>
                        <td>{snapshot.btInputReports.toLocaleString()} / {snapshot.btOutputCompleted.toLocaleString()}</td>
                        <td>{snapshot.lossPressure.toLocaleString()}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </article>

        <article className="panel diagnostics-uart">
          <div className="panel-heading">
            <h3>{text("CH340 实时日志", "CH340 live log")}</h3>
            <p>VID 1A86 · PID 7523 · 115200 8N1 · {text("无流控", "no flow control")}</p>
          </div>
          <div className="panel-body">
            {!serialSupported && (
              <div className="browser-warning">
                <strong>{text("当前浏览器不支持 Web Serial", "Web Serial is unavailable")}</strong>
                <span>{text("HID 快照和诊断包导出仍可使用。", "HID snapshots and bundle export still work.")}</span>
              </div>
            )}
            <div className="uart-actions">
              {serialConnected ? (
                <button className="danger-button" type="button" onClick={() => void disconnectSerial()}>{text("断开串口", "Disconnect")}</button>
              ) : (
                <button className="secondary-button" type="button" disabled={!serialSupported} onClick={() => void connectSerial()}>{text("连接 CH340", "Connect CH340")}</button>
              )}
              <button className="ghost-button" type="button" disabled={!uartView.text && uartView.droppedBytes === 0} onClick={clearUart}>{text("清空日志", "Clear log")}</button>
              <span>{serialConnected ? text("正在接收", "Receiving") : text("未连接", "Disconnected")}</span>
            </div>
            <pre className="uart-console" aria-label={text("串口日志", "UART log")}>{uartView.text || text("串口输出会显示在这里。", "UART output will appear here.")}</pre>
            <div className="uart-footnote">
              <span>{formatBytes(uartView.byteLength)} {text("日志保留", "log retained")}</span>
              <span>{uartView.droppedBytes > 0 ? text(`已丢弃 ${formatBytes(uartView.droppedBytes)} 旧日志`, `${formatBytes(uartView.droppedBytes)} of old log dropped`) : text("未发生截断", "No truncation")}</span>
            </div>
          </div>
        </article>
      </div>

      <article className="panel diagnostics-events">
        <div className="panel-heading">
          <h3>{text("诊断事件", "Diagnostic events")}</h3>
          <p>{text("由网页根据设备状态变化、计数器增长和传输错误生成", "Generated locally from device state changes, counter growth, and transport errors")}</p>
        </div>
        <div className="panel-body">
          {events.length === 0 ? (
            <div className="diagnostics-empty">{text("暂未发现异常或状态变化。", "No changes or anomalies recorded.")}</div>
          ) : (
            <ol className="diagnostics-event-list">
              {events.slice(0, 40).map((event, index) => (
                <li className={`is-${event.severity}`} key={`${event.timestamp}-${event.kind}-${index}`}>
                  <time dateTime={event.timestamp}>{new Date(event.timestamp).toLocaleTimeString()}</time>
                  <strong>{event.kind}</strong>
                  <span>{event.message}</span>
                </li>
              ))}
            </ol>
          )}
        </div>
      </article>

      <div className="diagnostics-export">
        <div>
          <strong>{text("导出单个 JSON 诊断包", "Export one JSON diagnostic bundle")}</strong>
          <span>{text("包含元数据、快照、事件、UART 文本和日志截断计数。", "Includes metadata, snapshots, events, UART text, and the truncation count.")}</span>
        </div>
        <label className="confirmation diagnostics-raw-export">
          <input type="checkbox" checked={!redactExport} onChange={(event) => setRedactExport(!event.target.checked)} />
          <span>{text("导出未经脱敏的原始标识（仅在明确需要时启用）", "Include unredacted identifiers (enable only when explicitly needed)")}</span>
        </label>
        <button className="primary-button" type="button" disabled={!canExport} onClick={exportBundle}>{text("下载诊断包", "Download bundle")}</button>
      </div>
    </section>
  );
}

function DiagnosticMetric({ label, value, warn = false }: { label: string; value: string; warn?: boolean }) {
  return <div className={warn ? "is-warning" : ""}><span>{label}</span><strong>{value}</strong></div>;
}

function deriveSnapshotEvents(previous: DiagnosticSnapshot | null, next: DiagnosticSnapshot, language: Language): DiagnosticEvent[] {
  const at = next.capturedAt;
  const zh = language === "zh";
  if (!previous) {
    return [{ timestamp: at, source: "device", kind: "snapshot-ready", severity: "info", message: zh ? "已建立诊断基线" : "Diagnostic baseline established" }];
  }
  const events: DiagnosticEvent[] = [];
  if (next.uptimeMs < previous.uptimeMs || next.snapshotSeq < previous.snapshotSeq) {
    return [{
      timestamp: at,
      source: "device",
      kind: "device-restarted",
      severity: "info",
      message: zh ? "检测到设备重新启动，性能计数基线已重置" : "Device restart detected; performance counter baseline reset",
      data: { previousUptimeMs: previous.uptimeMs, uptimeMs: next.uptimeMs },
    }];
  }
  if (previous.btState !== next.btState) {
    events.push({ timestamp: at, source: "device", kind: "bt-state", severity: "info", message: `BT ${previous.btState} → ${next.btState}` });
  }
  if (previous.healthFlags !== next.healthFlags) {
    events.push({ timestamp: at, source: "device", kind: "health-flags", severity: "info", message: `0x${previous.healthFlags.toString(16)} → 0x${next.healthFlags.toString(16)}` });
  }
  addCounterEvent(events, at, "loss-pressure", previous.lossPressure, next.lossPressure, zh ? "丢失/压力计数增加" : "Loss/pressure counter increased");
  addCounterEvent(events, at, "mic-underrun", previous.micUnderruns, next.micUnderruns, zh ? "麦克风欠载增加" : "Microphone underruns increased");
  addCounterEvent(events, at, "mic-overrun", previous.micOverruns, next.micOverruns, zh ? "麦克风过载增加" : "Microphone overruns increased");
  if (previous.otaState !== next.otaState || previous.otaError !== next.otaError) {
    events.push({ timestamp: at, source: "device", kind: "ota-state", severity: next.otaError ? "error" : "info", message: `state ${next.otaState}, error ${next.otaError}` });
  }
  if (next.heapFreeBytes < 32 * 1024 && previous.heapFreeBytes >= 32 * 1024) {
    events.push({ timestamp: at, source: "device", kind: "low-heap", severity: "warning", message: zh ? "可用堆内存低于 32 KiB" : "Free heap fell below 32 KiB", data: { heapFreeBytes: next.heapFreeBytes } });
  }
  return events;
}

function addCounterEvent(events: DiagnosticEvent[], timestamp: string, kind: string, before: number, after: number, message: string) {
  if (after <= before) return;
  events.push({ timestamp, source: "device", kind, severity: "warning", message, data: { before, after, delta: after - before } });
}

function messageOf(reason: unknown): string {
  if (reason instanceof DOMException && reason.name === "NotFoundError") return "未选择设备";
  if (reason instanceof DOMException && reason.name === "AbortError") return "操作已取消";
  return reason instanceof Error ? reason.message : String(reason);
}

function duration(milliseconds: number): string {
  const seconds = Math.floor(milliseconds / 1_000);
  const hours = Math.floor(seconds / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m ${seconds % 60}s`;
}

function formatBytes(bytes: number): string {
  return bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(2)} MiB` : `${(bytes / 1024).toFixed(1)} KiB`;
}

function abortError(message: string): Error {
  const error = new Error(message);
  error.name = "AbortError";
  return error;
}

function isAbortReason(reason: unknown): boolean {
  return reason instanceof Error && reason.name === "AbortError";
}

function subscribeNever(): () => void {
  return () => undefined;
}
