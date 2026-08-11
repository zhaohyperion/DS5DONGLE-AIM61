/**
 * Local DS5Dongle diagnostics helpers.
 *
 * This module never performs network I/O. HID snapshots and optional UART
 * text stay in the browser until the user explicitly downloads a JSON file.
 */

import { crc32 } from "./protocol.ts";

export const DIAGNOSTIC_REPORT_ID = 0xfd;
export const DIAGNOSTIC_REPORT_SIZE = 63;
export const DIAGNOSTIC_CRC_OFFSET = 59;
export const DIAGNOSTIC_MAX_SNAPSHOTS = 3_600;
export const DIAGNOSTIC_MAX_EVENTS = 2_048;
export const DIAGNOSTIC_UART_LIMIT = 2 * 1024 * 1024;
export const DIAGNOSTIC_UART_VIEW_LIMIT = 64 * 1024;
export const DIAGNOSTIC_MAX_PAGES = 16;
export const DIAGNOSTIC_REQUIRED_PAGES = 6;
export const DIAGNOSTIC_PAGE_DATA_SIZE = 43;
export const DIAGNOSTIC_CAPTURE_TIMEOUT_MS = 8_000;
export const HID_MAINTENANCE_LOCK_NAME = "ds5dongle-hid-maintenance-v1";

export const CH340_USB_VENDOR_ID = 0x1a86;
export const CH340_USB_PRODUCT_ID = 0x7523;
export const CH340_SERIAL_FILTERS = [
  { usbVendorId: CH340_USB_VENDOR_ID, usbProductId: CH340_USB_PRODUCT_ID },
] as const;

export interface DiagnosticEvent {
  timestamp: string;
  source: "device" | "webhid" | "uart" | "system";
  kind: string;
  severity: "info" | "warning" | "error";
  message: string;
  data?: Record<string, string | number | boolean | null>;
}

export interface DiagnosticSnapshot {
  snapshotSeq: number;
  capturedAt: string;
  monotonicMs: number;
  rawFlags: number;
  uptimeMs: number;
  btState: number;
  healthFlags: number;
  heapFreeBytes: number;
  heapMinFreeBytes: number;
  btRssiDbm: number | null;
  batteryPercent: number | null;
  batteryState: number;
  usbInCompleted: number;
  btInputReports: number;
  btOutputCompleted: number;
  pcmBlocksQueued: number;
  btAudioPairsSubmitted: number;
  micUnderruns: number;
  micOverruns: number;
  lossPressure: number;
  otaState: number;
  otaError: number;
  rawPages: string[];
}

export interface DiagnosticPage {
  index: number;
  count: number;
  flags: number;
  snapshotSeq: number;
  monotonicMs: number;
  data: Uint8Array;
}

export interface DiagnosticBundle {
  schema: "ds5dongle-diagnostics/v1";
  createdAt: string;
  redacted: boolean;
  metadata: {
    app: string;
    firmwareVersion: string | null;
    device: {
      productName: string | null;
      vendorId: number | null;
      productId: number | null;
    };
    browser: {
      userAgent: string | null;
      platform: string | null;
    };
    transports: {
      webHid: boolean;
      webSerial: boolean;
      uartConnected: boolean;
    };
  };
  snapshots: DiagnosticSnapshot[];
  events: DiagnosticEvent[];
  uart: {
    encoding: "utf-8";
    text: string;
    droppedBytes: number;
  };
  retention: {
    snapshotLimit: number;
    eventLimit: number;
    uartByteLimit: number;
    snapshotsDropped: number;
    eventsDropped: number;
  };
}

export interface CreateDiagnosticBundleOptions {
  firmwareVersion?: string | null;
  device?: Pick<HIDDevice, "productName" | "vendorId" | "productId"> | null;
  snapshots: readonly DiagnosticSnapshot[];
  events: readonly DiagnosticEvent[];
  uartText: string;
  uartDroppedBytes: number;
  snapshotsDropped?: number;
  eventsDropped?: number;
  uartConnected: boolean;
  redact?: boolean;
  now?: Date;
  userAgent?: string | null;
  platform?: string | null;
}

export class DiagnosticProtocolError extends Error {
  readonly code: string;

  constructor(message: string, code: string) {
    super(message);
    this.name = "DiagnosticProtocolError";
    this.code = code;
  }
}

export class DiagnosticHidClient {
  readonly device: HIDDevice;

  constructor(device: HIDDevice) {
    this.device = device;
  }

  async capture(
    signal?: AbortSignal,
    timeoutMs = DIAGNOSTIC_CAPTURE_TIMEOUT_MS,
  ): Promise<DiagnosticSnapshot> {
    const controller = new AbortController();
    const relayAbort = () => controller.abort(signal?.reason ?? createAbortError("诊断抓取已取消"));
    if (signal?.aborted) relayAbort();
    else signal?.addEventListener("abort", relayAbort, { once: true });
    const timer = setTimeout(
      () => controller.abort(new DiagnosticProtocolError("读取诊断快照超时", "timeout")),
      timeoutMs,
    );
    try {
      const lockedOperation = withHidMaintenanceLock(
        () => this.#captureConsistentPages(controller.signal),
        controller.signal,
      );
      // A Feature GET cannot be cancelled. Let the caller leave promptly on
      // abort/timeout, but keep the underlying operation (and lock) alive until
      // that one call returns and observes the aborted signal.
      void lockedOperation.catch(() => undefined);
      return await rejectOnAbort(lockedOperation, controller.signal);
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener("abort", relayAbort);
    }
  }

  async #captureConsistentPages(signal: AbortSignal): Promise<DiagnosticSnapshot> {
    throwIfAborted(signal);
    if (!this.device.opened) {
      await this.device.open();
      throwIfAborted(signal);
    }
    for (let attempt = 0; attempt < 4; attempt++) {
      throwIfAborted(signal);
      const pages: DiagnosticPage[] = [];
      let pageCount = 1;
      let expectedSequence = -1;
      let expectedMonotonic = -1;
      let inconsistent = false;

      for (let pageIndex = 0; pageIndex < pageCount; pageIndex++) {
        throwIfAborted(signal);
        await this.device.sendFeatureReport(
          DIAGNOSTIC_REPORT_ID,
          Uint8Array.of(0x01, 0x01, pageIndex),
        );
        throwIfAborted(signal);
        const report = await this.device.receiveFeatureReport(DIAGNOSTIC_REPORT_ID);
        throwIfAborted(signal);
        const page = decodeDiagnosticPage(report);
        // The selector is interface-global. A second browser tab can change it
        // between SET and GET, so treat a wrong page as a recoverable race.
        if (page.index !== pageIndex) {
          inconsistent = true;
          break;
        }
        if (pageIndex === 0) {
          pageCount = page.count;
          expectedSequence = page.snapshotSeq;
          expectedMonotonic = page.monotonicMs;
        } else if (
          page.count !== pageCount ||
          page.snapshotSeq !== expectedSequence ||
          page.monotonicMs !== expectedMonotonic
        ) {
          inconsistent = true;
          break;
        }
        pages.push(page);
      }

      throwIfAborted(signal);
      if (!inconsistent && pages.length === pageCount) return decodeDiagnosticSnapshot(pages);
    }
    throw new DiagnosticProtocolError("诊断快照在分页读取期间持续变化，请稍后重试", "snapshot-race");
  }
}

interface HidLockManagerLike {
  request<T>(
    name: string,
    options: { mode: "exclusive"; signal?: AbortSignal },
    callback: () => Promise<T>,
  ): Promise<T>;
}

let fallbackMaintenanceTail: Promise<void> = Promise.resolve();

/**
 * Serialize all maintenance traffic that shares the HID Feature endpoint.
 * Chromium Web Locks provide same-origin, cross-tab exclusion. Browsers without
 * Web Locks retain a module-local queue plus the protocol consistency retries.
 */
export async function withHidMaintenanceLock<T>(
  operation: () => Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  throwIfAborted(signal);
  const manager = hidLockManager();
  if (manager) {
    const options: { mode: "exclusive"; signal?: AbortSignal } = { mode: "exclusive" };
    if (signal) options.signal = signal;
    return manager.request(HID_MAINTENANCE_LOCK_NAME, options, async () => {
      throwIfAborted(signal);
      return operation();
    });
  }

  const previous = fallbackMaintenanceTail.catch(() => undefined);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  fallbackMaintenanceTail = previous.then(() => gate);
  try {
    await waitForTurn(previous, signal);
    throwIfAborted(signal);
    return await operation();
  } finally {
    release();
  }
}

export class BoundedTextBuffer {
  #chunks: Array<{ text: string; bytes: number }> = [];
  #byteLength = 0;
  #droppedBytes = 0;
  readonly #encoder = new TextEncoder();
  readonly limit: number;

  constructor(limit = DIAGNOSTIC_UART_LIMIT) {
    if (!Number.isInteger(limit) || limit < 1) throw new RangeError("limit must be a positive integer");
    this.limit = limit;
  }

  get text(): string {
    return this.#chunks.map((chunk) => chunk.text).join("");
  }

  get byteLength(): number {
    return this.#byteLength;
  }

  get droppedBytes(): number {
    return this.#droppedBytes;
  }

  append(chunk: string): void {
    if (!chunk) return;
    let encoded = this.#encoder.encode(chunk);
    if (encoded.byteLength > this.limit) {
      const originalLength = encoded.byteLength;
      encoded = validUtf8Tail(encoded, this.limit);
      this.#droppedBytes += this.#byteLength + originalLength - encoded.byteLength;
      this.#chunks = [{ text: new TextDecoder().decode(encoded), bytes: encoded.byteLength }];
      this.#byteLength = encoded.byteLength;
      return;
    }
    this.#chunks.push({ text: chunk, bytes: encoded.byteLength });
    this.#byteLength += encoded.byteLength;
    while (this.#byteLength > this.limit && this.#chunks.length > 0) {
      const overflow = this.#byteLength - this.limit;
      const oldest = this.#chunks[0];
      if (oldest.bytes <= overflow) {
        this.#chunks.shift();
        this.#byteLength -= oldest.bytes;
        this.#droppedBytes += oldest.bytes;
        continue;
      }
      const oldestBytes = this.#encoder.encode(oldest.text);
      const kept = validUtf8Tail(oldestBytes, oldest.bytes - overflow);
      const actuallyDropped = oldest.bytes - kept.byteLength;
      this.#chunks[0] = { text: new TextDecoder().decode(kept), bytes: kept.byteLength };
      this.#byteLength -= actuallyDropped;
      this.#droppedBytes += actuallyDropped;
    }
  }

  clear(): void {
    this.#chunks = [];
    this.#byteLength = 0;
    this.#droppedBytes = 0;
  }
}

interface SerialPortLike {
  readonly readable: ReadableStream<Uint8Array> | null;
  open(options: {
    baudRate: number;
    dataBits: 8;
    stopBits: 1;
    parity: "none";
    flowControl: "none";
  }): Promise<void>;
  setSignals?(signals: {
    dataTerminalReady?: boolean;
    requestToSend?: boolean;
  }): Promise<void>;
  close(): Promise<void>;
}

interface SerialApiLike {
  requestPort(options: { filters: ReadonlyArray<{ usbVendorId: number; usbProductId: number }> }): Promise<SerialPortLike>;
}

export interface Ch340SerialMonitorOptions {
  onText: (chunk: string) => void;
  onDisconnect?: () => void;
  onError?: (reason: unknown) => void;
}

export class Ch340SerialMonitor {
  #port: SerialPortLike | null = null;
  #reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
  #running = false;
  readonly #decoder = new TextDecoder();
  readonly #options: Ch340SerialMonitorOptions;

  constructor(options: Ch340SerialMonitorOptions) {
    this.#options = options;
  }

  get connected(): boolean {
    return this.#port !== null && this.#running;
  }

  async connect(): Promise<void> {
    if (this.connected) return;
    const serial = serialApi();
    const port = await serial.requestPort({ filters: CH340_SERIAL_FILTERS });
    try {
      await port.open({
        baudRate: 115200,
        dataBits: 8,
        stopBits: 1,
        parity: "none",
        flowControl: "none",
      });
      await port.setSignals?.({ dataTerminalReady: false, requestToSend: false });
    } catch (reason) {
      await port.close().catch(() => undefined);
      throw reason;
    }
    this.#port = port;
    this.#running = true;
    void this.#readLoop();
  }

  async disconnect(): Promise<void> {
    this.#running = false;
    const reader = this.#reader;
    this.#reader = null;
    if (reader) {
      await reader.cancel().catch(() => undefined);
      reader.releaseLock();
    }
    const port = this.#port;
    this.#port = null;
    if (port) await port.close().catch(() => undefined);
  }

  async #readLoop(): Promise<void> {
    const port = this.#port;
    if (!port?.readable) {
      this.#running = false;
      this.#port = null;
      await port?.close().catch(() => undefined);
      this.#options.onError?.(new Error("串口没有可读数据流"));
      return;
    }
    const reader = port.readable.getReader();
    this.#reader = reader;
    try {
      while (this.#running) {
        const { value, done } = await reader.read();
        if (done) break;
        if (value?.byteLength) this.#options.onText(this.#decoder.decode(value, { stream: true }));
      }
      const tail = this.#decoder.decode();
      if (tail) this.#options.onText(tail);
    } catch (reason) {
      if (this.#running) this.#options.onError?.(reason);
    } finally {
      this.#running = false;
      if (this.#reader === reader) {
        this.#reader = null;
        reader.releaseLock();
      }
      if (this.#port === port) {
        this.#port = null;
        await port.close().catch(() => undefined);
      }
      this.#options.onDisconnect?.();
    }
  }
}

export function webSerialAvailable(): boolean {
  return typeof navigator !== "undefined" && Boolean((navigator as unknown as { serial?: unknown }).serial);
}

export function verifyDiagnosticCrc(frame: Uint8Array): void {
  if (frame.byteLength !== DIAGNOSTIC_REPORT_SIZE) {
    throw new DiagnosticProtocolError(`诊断报告长度错误：${frame.byteLength}/${DIAGNOSTIC_REPORT_SIZE}`, "frame-size");
  }
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  const expected = view.getUint32(DIAGNOSTIC_CRC_OFFSET, true);
  const actual = crc32(frame.subarray(0, DIAGNOSTIC_CRC_OFFSET));
  if (expected !== actual) throw new DiagnosticProtocolError("诊断报告 CRC32 校验失败", "frame-crc");
}

export function decodeDiagnosticPage(source: ArrayBuffer | ArrayBufferView<ArrayBufferLike>): DiagnosticPage {
  let frame = source instanceof ArrayBuffer
    ? new Uint8Array(source)
    : new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
  if (frame.byteLength === DIAGNOSTIC_REPORT_SIZE + 1 && frame[0] === DIAGNOSTIC_REPORT_ID) {
    frame = frame.subarray(1);
  }
  verifyDiagnosticCrc(frame);
  if (frame[0] !== 0x44 || frame[1] !== 0x47) {
    throw new DiagnosticProtocolError("设备不支持 DG 诊断协议", "frame-magic");
  }
  if (frame[2] !== 1) throw new DiagnosticProtocolError(`不支持诊断协议版本 ${frame[2]}`, "frame-version");
  if (frame[3] !== 16) throw new DiagnosticProtocolError(`诊断页头长度无效：${frame[3]}`, "header-size");
  const index = frame[4];
  const count = frame[5];
  const dataLength = frame[6];
  if (count < 1 || count > DIAGNOSTIC_MAX_PAGES || index >= count) {
    throw new DiagnosticProtocolError(`诊断分页无效：${index}/${count}`, "page-range");
  }
  if (count < DIAGNOSTIC_REQUIRED_PAGES) {
    throw new DiagnosticProtocolError(`诊断协议 v1 缺少必需页面：${count}/${DIAGNOSTIC_REQUIRED_PAGES}`, "page-count");
  }
  if (dataLength > DIAGNOSTIC_PAGE_DATA_SIZE) {
    throw new DiagnosticProtocolError(`诊断页数据过长：${dataLength}`, "page-length");
  }
  for (const byte of frame.subarray(16 + dataLength, DIAGNOSTIC_CRC_OFFSET)) {
    if (byte !== 0) throw new DiagnosticProtocolError("诊断页包含非零填充", "page-padding");
  }
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  return {
    index,
    count,
    flags: frame[7],
    snapshotSeq: view.getUint32(8, true),
    monotonicMs: view.getUint32(12, true),
    data: frame.slice(16, 16 + dataLength),
  };
}

export function decodeDiagnosticSnapshot(pages: readonly DiagnosticPage[]): DiagnosticSnapshot {
  if (pages.length < 1) throw new DiagnosticProtocolError("诊断快照没有数据页", "snapshot-empty");
  const first = pages[0];
  if (pages.length !== first.count) throw new DiagnosticProtocolError("诊断快照页数不完整", "snapshot-pages");
  pages.forEach((page, index) => {
    if (
      page.index !== index ||
      page.count !== first.count ||
      page.snapshotSeq !== first.snapshotSeq ||
      page.monotonicMs !== first.monotonicMs
    ) {
      throw new DiagnosticProtocolError("诊断快照分页不一致", "snapshot-identity");
    }
  });

  // Field decoding is deliberately page-scoped: later protocol versions can
  // extend one page without shifting every value that follows it.
  const p0 = pageView(pages, 0);
  const p1 = pageView(pages, 1);
  const p2 = pageView(pages, 2);
  const p4 = pageView(pages, 4);
  const p5 = pageView(pages, 5);
  return {
    snapshotSeq: first.snapshotSeq,
    capturedAt: new Date().toISOString(),
    monotonicMs: first.monotonicMs,
    rawFlags: first.flags,
    uptimeMs: readU32(p0, 0),
    btState: readU8(p0, 4),
    healthFlags: readU8(p0, 5),
    heapFreeBytes: readU32(p5, 0),
    heapMinFreeBytes: readU32(p5, 4),
    btRssiDbm: readI8Nullable(p0, 6),
    batteryPercent: readU8Nullable(p0, 7),
    batteryState: readU8(p0, 8, 0xff),
    usbInCompleted: readU32(p1, 0),
    btInputReports: readU32(p1, 4),
    btOutputCompleted: readU32(p1, 8),
    pcmBlocksQueued: readU32(p4, 0),
    btAudioPairsSubmitted: readU32(p4, 4),
    micUnderruns: readU32(p4, 8),
    micOverruns: readU32(p4, 12),
    lossPressure: readU32(p2, 0),
    otaState: readU8(p5, 8),
    otaError: readU8(p5, 9),
    rawPages: pages.map((page) => bytesToHex(page.data)),
  };
}

export function createDiagnosticBundle(options: CreateDiagnosticBundleOptions): DiagnosticBundle {
  const redact = options.redact !== false;
  const now = options.now ?? new Date();
  const browserNavigator = typeof navigator === "undefined" ? null : navigator;
  const raw: DiagnosticBundle = {
    schema: "ds5dongle-diagnostics/v1",
    createdAt: now.toISOString(),
    redacted: redact,
    metadata: {
      app: "DS5Dongle Ai-M61 Device Studio",
      firmwareVersion: options.firmwareVersion ?? null,
      device: {
        productName: options.device?.productName ?? null,
        vendorId: options.device?.vendorId ?? null,
        productId: options.device?.productId ?? null,
      },
      browser: {
        userAgent: options.userAgent ?? browserNavigator?.userAgent ?? null,
        platform: options.platform ?? browserNavigator?.platform ?? null,
      },
      transports: {
        webHid: Boolean(options.device),
        webSerial: webSerialAvailable(),
        uartConnected: options.uartConnected,
      },
    },
    snapshots: options.snapshots.map(cloneSnapshot),
    events: options.events.map((event) => ({ ...event, data: event.data ? { ...event.data } : undefined })),
    uart: {
      encoding: "utf-8",
      text: options.uartText,
      droppedBytes: options.uartDroppedBytes,
    },
    retention: {
      snapshotLimit: DIAGNOSTIC_MAX_SNAPSHOTS,
      eventLimit: DIAGNOSTIC_MAX_EVENTS,
      uartByteLimit: DIAGNOSTIC_UART_LIMIT,
      snapshotsDropped: options.snapshotsDropped ?? 0,
      eventsDropped: options.eventsDropped ?? 0,
    },
  };
  return redact ? redactDiagnosticValue(raw) : raw;
}

export function serializeDiagnosticBundle(bundle: DiagnosticBundle): string {
  return `${JSON.stringify(bundle, null, 2)}\n`;
}

export function downloadDiagnosticBundle(bundle: DiagnosticBundle): string {
  if (typeof document === "undefined") throw new Error("只能在浏览器中下载诊断包");
  const stamp = bundle.createdAt.replace(/[:.]/g, "-");
  const filename = `DS5Dongle-diagnostics-${stamp}.json`;
  const url = URL.createObjectURL(new Blob([serializeDiagnosticBundle(bundle)], { type: "application/json" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
  return filename;
}

export function redactDiagnosticText(text: string): string {
  return text
    .replace(/\b(?:[0-9a-f]{2}[:-]){5}[0-9a-f]{2}\b/gi, "[REDACTED-BD_ADDR]")
    .replace(/(\[USB-INIT\]\s+Serial:\s*)\S+/gi, "$1[REDACTED]")
    .replace(/\b((?:bd[_ -]?addr|bluetooth[_ -]?addr|mac[_ -]?addr)\s*[:=]\s*)[0-9a-f]{12}\b/gi, "$1[REDACTED]")
    .replace(/\b((?:usb[_ -]?serial|serial[_ -]?number)\s*[:=]\s*)[^\s,;]+/gi, "$1[REDACTED]")
    .replace(/\b((?:ssp[_ -]?(?:passkey|pin)|passkey)\s*[:=]\s*)\d{4,8}\b/gi, "$1[REDACTED]")
    .replace(/\b((?:conn|chan|mic_q)\s*=?\s*)0x[0-9a-f]{6,16}\b/gi, "$1[REDACTED]")
    .replace(/\b((?:ptr|pointer|pc|lr|ra|sp|address|addr)\s*[:=]\s*)0x[0-9a-f]{6,16}\b/gi, "$1[REDACTED]");
}

export function redactDiagnosticValue<T>(value: T): T {
  return redactUnknown(value, "") as T;
}

function redactUnknown(value: unknown, key: string): unknown {
  if (isSensitiveKey(key)) return "[REDACTED]";
  if (typeof value === "string") return redactDiagnosticText(value);
  if (Array.isArray(value)) return value.map((entry) => redactUnknown(entry, ""));
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([entryKey, entryValue]) => [entryKey, redactUnknown(entryValue, entryKey)]),
    );
  }
  return value;
}

function isSensitiveKey(key: string): boolean {
  return /(?:bd.?addr|bluetooth.?addr|mac.?addr|usb.*serial|serial.*usb|ssp.*(?:passkey|pin)|passkey|(?:^|_)(?:ptr|pointer|pc|lr|ra|sp|address|addr)$)/i.test(key);
}

function cloneSnapshot(snapshot: DiagnosticSnapshot): DiagnosticSnapshot {
  return { ...snapshot, rawPages: [...snapshot.rawPages] };
}

function serialApi(): SerialApiLike {
  if (typeof navigator === "undefined") throw new Error("当前环境不支持 Web Serial");
  const serial = (navigator as unknown as { serial?: SerialApiLike }).serial;
  if (!serial) throw new Error("当前浏览器不支持 Web Serial，请使用最新版 Chrome 或 Edge");
  return serial;
}

function pageView(pages: readonly DiagnosticPage[], index: number): DataView | null {
  const page = pages[index];
  return page ? new DataView(page.data.buffer, page.data.byteOffset, page.data.byteLength) : null;
}

function readU8(view: DataView | null, offset: number, fallback = 0): number {
  return view && offset < view.byteLength ? view.getUint8(offset) : fallback;
}

function readU8Nullable(view: DataView | null, offset: number): number | null {
  const value = readU8(view, offset, 0xff);
  return value <= 100 ? value : null;
}

function readI8Nullable(view: DataView | null, offset: number): number | null {
  if (!view || offset >= view.byteLength) return null;
  const value = view.getInt8(offset);
  return value === 127 ? null : value;
}

function readU32(view: DataView | null, offset: number): number {
  return view && offset + 4 <= view.byteLength ? view.getUint32(offset, true) : 0;
}

function bytesToHex(value: Uint8Array): string {
  return Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function validUtf8Tail(value: Uint8Array<ArrayBufferLike>, keepAtMost: number): Uint8Array<ArrayBuffer> {
  let start = Math.max(0, value.byteLength - keepAtMost);
  while (start < value.byteLength && (value[start] & 0xc0) === 0x80) start++;
  return Uint8Array.from(value.subarray(start));
}

function hidLockManager(): HidLockManagerLike | null {
  if (typeof navigator === "undefined") return null;
  return (navigator as unknown as { locks?: HidLockManagerLike }).locks ?? null;
}

function throwIfAborted(signal?: AbortSignal): void {
  if (!signal?.aborted) return;
  throw signal.reason ?? createAbortError("操作已取消");
}

function createAbortError(message: string): Error {
  const error = new Error(message);
  error.name = "AbortError";
  return error;
}

function waitForTurn(turn: Promise<void>, signal?: AbortSignal): Promise<void> {
  if (!signal) return turn;
  if (signal.aborted) return Promise.reject(signal.reason ?? createAbortError("操作已取消"));
  return new Promise<void>((resolve, reject) => {
    const onAbort = () => reject(signal.reason ?? createAbortError("操作已取消"));
    signal.addEventListener("abort", onAbort, { once: true });
    void turn.then(resolve, reject).finally(() => signal.removeEventListener("abort", onAbort));
  });
}

function rejectOnAbort<T>(operation: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(signal.reason ?? createAbortError("操作已取消"));
  let onAbort!: () => void;
  const aborted = new Promise<never>((_, reject) => {
    onAbort = () => reject(signal.reason ?? createAbortError("操作已取消"));
    signal.addEventListener("abort", onAbort, { once: true });
  });
  return Promise.race([operation, aborted]).finally(() => signal.removeEventListener("abort", onAbort));
}
