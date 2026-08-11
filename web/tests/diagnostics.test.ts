import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  BoundedTextBuffer,
  Ch340SerialMonitor,
  DIAGNOSTIC_REPORT_ID,
  HID_MAINTENANCE_LOCK_NAME,
  DiagnosticHidClient,
  createDiagnosticBundle,
  decodeDiagnosticPage,
  decodeDiagnosticSnapshot,
  redactDiagnosticText,
  serializeDiagnosticBundle,
  withHidMaintenanceLock,
  type DiagnosticPage,
} from "../app/lib/diagnostics.ts";
import { crc32 } from "../app/lib/protocol.ts";

test("DG/v1 page decoder accepts WebHID payloads with or without report ID", () => {
  const payload = makePage(0, 6, 42, 123_456, Uint8Array.of(1, 2, 3));
  const decoded = decodeDiagnosticPage(payload);
  assert.equal(decoded.index, 0);
  assert.equal(decoded.count, 6);
  assert.equal(decoded.snapshotSeq, 42);
  assert.equal(decoded.monotonicMs, 123_456);
  assert.deepEqual(decoded.data, Uint8Array.of(1, 2, 3));
  const prefixed = new Uint8Array(64);
  prefixed[0] = DIAGNOSTIC_REPORT_ID;
  prefixed.set(payload, 1);
  assert.deepEqual(decodeDiagnosticPage(prefixed).data, decoded.data);
  const corrupt = payload.slice();
  corrupt[16] ^= 1;
  assert.throws(() => decodeDiagnosticPage(corrupt), /CRC32/);
  const badPadding = payload.slice();
  badPadding[20] = 1;
  writeCrc(badPadding);
  assert.throws(() => decodeDiagnosticPage(badPadding), /填充/);
});

test("six diagnostic pages decode the stable firmware byte map", () => {
  const data = Array.from({ length: 6 }, () => new Uint8Array(43));
  setU32(data[0], 0, 99_000);
  data[0].set([3, 0xa5, 0xd6, 87, 1], 4);
  setU32(data[1], 0, 1001);
  setU32(data[1], 4, 2002);
  setU32(data[1], 8, 3003);
  setU32(data[2], 0, 7);
  setU32(data[4], 0, 4004);
  setU32(data[4], 4, 5005);
  setU32(data[4], 8, 6);
  setU32(data[4], 12, 8);
  setU32(data[5], 0, 64 * 1024);
  setU32(data[5], 4, 48 * 1024);
  data[5].set([2, 0], 8);
  const pages: DiagnosticPage[] = data.map((page, index) => decodeDiagnosticPage(makePage(index, 6, 9, 99_000, page)));
  const snapshot = decodeDiagnosticSnapshot(pages);
  assert.equal(snapshot.uptimeMs, 99_000);
  assert.equal(snapshot.btState, 3);
  assert.equal(snapshot.healthFlags, 0xa5);
  assert.equal(snapshot.btRssiDbm, -42);
  assert.equal(snapshot.batteryPercent, 87);
  assert.equal(snapshot.usbInCompleted, 1001);
  assert.equal(snapshot.btInputReports, 2002);
  assert.equal(snapshot.btOutputCompleted, 3003);
  assert.equal(snapshot.lossPressure, 7);
  assert.equal(snapshot.pcmBlocksQueued, 4004);
  assert.equal(snapshot.btAudioPairsSubmitted, 5005);
  assert.equal(snapshot.micUnderruns, 6);
  assert.equal(snapshot.micOverruns, 8);
  assert.equal(snapshot.heapFreeBytes, 64 * 1024);
  assert.equal(snapshot.heapMinFreeBytes, 48 * 1024);
  assert.equal(snapshot.otaState, 2);
  assert.equal(snapshot.rawPages.length, 6);
});

test("HID client recovers when another tab races the global page selector", async () => {
  const frames = Array.from({ length: 6 }, (_, index) => makePage(index, 6, 77, 88, new Uint8Array(43)));
  let selected = 0;
  let raceOnce = true;
  let sends = 0;
  const device = {
    opened: true,
    open: async () => undefined,
    sendFeatureReport: async (_reportId: number, request: Uint8Array) => { selected = request[2]; sends++; },
    receiveFeatureReport: async () => {
      if (raceOnce) { raceOnce = false; return new DataView(frames[1].buffer); }
      return new DataView(frames[selected].buffer);
    },
  } as unknown as HIDDevice;
  const snapshot = await new DiagnosticHidClient(device).capture();
  assert.equal(snapshot.snapshotSeq, 77);
  assert.ok(sends >= 7);
});

test("capture aborts after the current uninterruptible WebHID call and sends no later pages", async () => {
  const controller = new AbortController();
  const frame = makePage(0, 6, 12, 34, new Uint8Array(43));
  let sends = 0;
  let receives = 0;
  const device = {
    opened: true,
    open: async () => undefined,
    sendFeatureReport: async () => { sends++; },
    receiveFeatureReport: async () => {
      receives++;
      const error = new Error("OTA started");
      error.name = "AbortError";
      controller.abort(error);
      return new DataView(frame.buffer);
    },
  } as unknown as HIDDevice;
  await assert.rejects(() => new DiagnosticHidClient(device).capture(controller.signal), { name: "AbortError" });
  assert.equal(sends, 1);
  assert.equal(receives, 1);
});

test("capture reports timeout while a stuck Feature GET keeps the lock and sends no later pages", async () => {
  const frame = makePage(0, 6, 21, 55, new Uint8Array(43));
  let releaseReceive!: () => void;
  const blockedReceive = new Promise<DataView>((resolve) => {
    releaseReceive = () => resolve(new DataView(frame.buffer));
  });
  let sends = 0;
  let receives = 0;
  const device = {
    opened: true,
    open: async () => undefined,
    sendFeatureReport: async () => { sends++; },
    receiveFeatureReport: () => { receives++; return blockedReceive; },
  } as unknown as HIDDevice;
  const started = Date.now();
  await assert.rejects(
    () => new DiagnosticHidClient(device).capture(undefined, 20),
    (reason: unknown) => reason instanceof Error && reason.message.includes("超时"),
  );
  assert.ok(Date.now() - started < 500);
  assert.equal(sends, 1);
  assert.equal(receives, 1);
  releaseReceive();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(sends, 1);
});

test("maintenance lock serializes same-tab fallback operations", async () => {
  assert.equal(HID_MAINTENANCE_LOCK_NAME, "ds5dongle-hid-maintenance-v1");
  const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: {} });
  try {
    const order: string[] = [];
    let releaseFirst!: () => void;
    let markStarted!: () => void;
    const firstGate = new Promise<void>((resolve) => { releaseFirst = resolve; });
    const started = new Promise<void>((resolve) => { markStarted = resolve; });
    const first = withHidMaintenanceLock(async () => {
      order.push("first-start");
      markStarted();
      await firstGate;
      order.push("first-end");
    });
    await started;
    const second = withHidMaintenanceLock(async () => { order.push("second"); });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.deepEqual(order, ["first-start"]);
    releaseFirst();
    await Promise.all([first, second]);
    assert.deepEqual(order, ["first-start", "first-end", "second"]);
  } finally {
    if (originalNavigator) Object.defineProperty(globalThis, "navigator", originalNavigator);
    else Reflect.deleteProperty(globalThis, "navigator");
  }
});

test("maintenance lock requests the named Chromium Web Lock", async () => {
  const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  let requestedName = "";
  let requestedMode = "";
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      locks: {
        request: async (name: string, options: { mode: string }, callback: () => Promise<number>) => {
          requestedName = name;
          requestedMode = options.mode;
          return callback();
        },
      },
    },
  });
  try {
    assert.equal(await withHidMaintenanceLock(async () => 7), 7);
    assert.equal(requestedName, HID_MAINTENANCE_LOCK_NAME);
    assert.equal(requestedMode, "exclusive");
  } finally {
    if (originalNavigator) Object.defineProperty(globalThis, "navigator", originalNavigator);
    else Reflect.deleteProperty(globalThis, "navigator");
  }
});

test("CH340 setup failure closes the opened port", async () => {
  const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  let closes = 0;
  const port = {
    readable: null,
    open: async () => undefined,
    setSignals: async () => { throw new Error("setSignals failed"); },
    close: async () => { closes++; },
  };
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { serial: { requestPort: async () => port } },
  });
  try {
    const monitor = new Ch340SerialMonitor({ onText() {} });
    await assert.rejects(() => monitor.connect(), /setSignals failed/);
    assert.equal(monitor.connected, false);
    assert.equal(closes, 1);
  } finally {
    if (originalNavigator) Object.defineProperty(globalThis, "navigator", originalNavigator);
    else Reflect.deleteProperty(globalThis, "navigator");
  }
});

test("UART ring is byte-bounded and records dropped bytes", () => {
  const buffer = new BoundedTextBuffer(8);
  buffer.append("abcd");
  buffer.append("efghij");
  assert.equal(buffer.text, "cdefghij");
  assert.equal(buffer.byteLength, 8);
  assert.equal(buffer.droppedBytes, 2);
  buffer.clear();
  buffer.append("你ab");
  buffer.append("cdef");
  assert.ok(buffer.byteLength <= 8);
  assert.doesNotMatch(buffer.text, /�/);
  assert.ok(buffer.droppedBytes > 0);
});

test("diagnostic bundle redacts identifiers by default and records retention losses", () => {
  const sensitive = "BD_ADDR=112233AABBCC USB_SERIAL=DS5-123 SSP passkey=123456 ptr=0x2300ABCD 11:22:33:44:55:66";
  assert.doesNotMatch(redactDiagnosticText(sensitive), /112233AABBCC|DS5-123|123456|2300ABCD|11:22:33:44:55:66/);
  const bundle = createDiagnosticBundle({
    firmwareVersion: "3.6.0",
    snapshots: [],
    events: [],
    uartText: sensitive,
    uartDroppedBytes: 4096,
    snapshotsDropped: 2,
    eventsDropped: 3,
    uartConnected: false,
    now: new Date("2026-08-10T00:00:00.000Z"),
    userAgent: "test",
    platform: "test",
  });
  const serialized = serializeDiagnosticBundle(bundle);
  assert.equal(bundle.redacted, true);
  assert.equal(bundle.uart.droppedBytes, 4096);
  assert.equal(bundle.retention.snapshotsDropped, 2);
  assert.equal(bundle.retention.eventsDropped, 3);
  assert.doesNotMatch(serialized, /DS5-123|123456|2300ABCD/);
  const raw = createDiagnosticBundle({ snapshots: [], events: [], uartText: sensitive, uartDroppedBytes: 0, uartConnected: false, redact: false });
  assert.match(raw.uart.text, /DS5-123/);
});

test("diagnostics UI batches UART rendering and pauses polling for OTA", () => {
  const source = readFileSync(new URL("../app/DiagnosticsPanel.tsx", import.meta.url), "utf8");
  assert.match(source, /\}, 250\);/);
  assert.match(source, /, 1_000\);/);
  assert.match(source, /if \(!hidClient \|\| otaActive \|\| captureInFlight\.current\) return/);
  assert.match(source, /uartBuffer\.current\.append\(chunk\)/);
  const onText = source.match(/onText:\s*\(chunk\)\s*=>\s*\{([\s\S]*?)\n\s*\},/)?.[1] ?? "";
  assert.doesNotMatch(onText, /setUartView|setState/);
  assert.match(source, /checked=\{!redactExport\}/);
  assert.match(source, /captureAbort\.current\?\.abort/);
  const consoleSource = readFileSync(new URL("../app/DeviceConsole.tsx", import.meta.url), "utf8");
  assert.match(consoleSource, /withHidMaintenanceLock\([\s\S]*?client\.transferOta/);
});

function makePage(index: number, count: number, sequence: number, monotonicMs: number, data: Uint8Array): Uint8Array {
  const frame = new Uint8Array(63);
  frame.set([0x44, 0x47, 0x01, 0x10, index, count, data.byteLength, 0], 0);
  const view = new DataView(frame.buffer);
  view.setUint32(8, sequence, true);
  view.setUint32(12, monotonicMs, true);
  frame.set(data, 16);
  writeCrc(frame);
  return frame;
}

function setU32(target: Uint8Array, offset: number, value: number): void {
  new DataView(target.buffer, target.byteOffset, target.byteLength).setUint32(offset, value, true);
}

function writeCrc(frame: Uint8Array): void {
  new DataView(frame.buffer, frame.byteOffset, frame.byteLength).setUint32(59, crc32(frame.subarray(0, 59)), true);
}
