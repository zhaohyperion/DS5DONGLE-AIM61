import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_LEGACY_CONFIG,
  LEGACY_CONFIG_SIZE,
  LEGACY_HID_FILTERS,
  LEGACY_REMAP_COUNT,
  LEGACY_REMAP_SIZE,
  LEGACY_REPORT_CONFIG,
  LEGACY_REPORT_CONFIG_COMMAND,
  LEGACY_REPORT_FIRMWARE_VERSION,
  LEGACY_REPORT_REMAP,
  LEGACY_REPORT_TELEMETRY,
  LegacyHidClient,
  LegacyHidError,
  decodeLegacyConfig,
  decodeLegacyFirmwareVersion,
  decodeLegacyRemap,
  decodeLegacyTelemetry,
  encodeLegacyConfig,
  encodeLegacyConfigCommand,
  encodeLegacyRemap,
  encodeLegacyRemapCommand,
  identityLegacyRemap,
  type LegacyConfig,
} from "../app/lib/legacy-hid.ts";

test("WebHID filters target both Sony-compatible DS5 products and gamepad usage", () => {
  assert.deepEqual(LEGACY_HID_FILTERS, [
    { vendorId: 0x054c, productId: 0x0ce6, usagePage: 1, usage: 5 },
    { vendorId: 0x054c, productId: 0x0df2, usagePage: 1, usage: 5 },
  ]);
});

test("config v2 uses the packed 25-byte firmware layout", () => {
  const config: LegacyConfig = {
    ...DEFAULT_LEGACY_CONFIG,
    hapticsGain: 1.5,
    speakerVolume: 1,
    headsetVolume: 2,
    speakerGain: 3,
    inactiveTime: 4,
    disableLed: false,
    pollingRateMode: 2,
    audioBufferLength: 65,
    controllerMode: 1,
    enableUsbSerial: false,
    psShortcutEnabled: true,
    disableMic: true,
    disableSpeaker: true,
    enableWake: true,
    triggerReduce: 10,
    lockVolume: true,
    dseDetected: true,
    usbStealth: true,
    ledR: 0x12,
    ledG: 0x34,
    ledB: 0x56,
  };
  const encoded = encodeLegacyConfig(config);
  assert.equal(encoded.byteLength, LEGACY_CONFIG_SIZE);
  assert.deepEqual(Array.from(encoded), [
    2,
    0x00, 0x00, 0xc0, 0x3f,
    1, 2, 3, 4, 0, 2, 65, 1, 0, 1, 1, 1, 1, 10, 1, 1, 1,
    0x12, 0x34, 0x56,
  ]);
  assert.deepEqual(decodeLegacyConfig(encoded), config);

  const withReportId = Uint8Array.of(LEGACY_REPORT_CONFIG, ...encoded);
  assert.deepEqual(decodeLegacyConfig(withReportId), config);
});

test("config validation rejects unsupported versions, ranges, and malformed booleans", () => {
  assert.throws(
    () => encodeLegacyConfig({ ...DEFAULT_LEGACY_CONFIG, configVersion: 1 }),
    (error: unknown) => error instanceof LegacyHidError && error.code === "config-version",
  );
  assert.throws(
    () => encodeLegacyConfig({ ...DEFAULT_LEGACY_CONFIG, audioBufferLength: 15 }),
    /音频缓冲长度/,
  );
  assert.throws(
    () => encodeLegacyConfig({ ...DEFAULT_LEGACY_CONFIG, pollingRateMode: 3 as 2 }),
    /轮询率模式/,
  );
  const malformed = encodeLegacyConfig({ ...DEFAULT_LEGACY_CONFIG });
  malformed[9] = 2;
  assert.throws(() => decodeLegacyConfig(malformed), /不是 0\/1/);
});

test("F6 command payloads keep the command byte separate from the config body", () => {
  const apply = encodeLegacyConfigCommand(1, { ...DEFAULT_LEGACY_CONFIG });
  assert.equal(apply.byteLength, 63);
  assert.equal(apply[0], 1);
  assert.deepEqual(apply.subarray(1, 26), encodeLegacyConfig({ ...DEFAULT_LEGACY_CONFIG }));
  assert.ok(apply.subarray(26).every((byte) => byte === 0));

  for (const command of [2, 3] as const) {
    const report = encodeLegacyConfigCommand(command);
    assert.equal(report[0], command);
    assert.ok(report.subarray(1).every((byte) => byte === 0));
  }
});

test("F8 version and F9 signed RSSI/audio/battery telemetry parse with optional report IDs", () => {
  const version = new TextEncoder().encode("3.5.1\0\0");
  assert.equal(decodeLegacyFirmwareVersion(version), "3.5.1");
  assert.equal(
    decodeLegacyFirmwareVersion(Uint8Array.of(LEGACY_REPORT_FIRMWARE_VERSION, ...version)),
    "3.5.1",
  );

  assert.deepEqual(
    decodeLegacyTelemetry(Uint8Array.of(LEGACY_REPORT_TELEMETRY, 0xc9, 0x83, 80, 2)),
    {
      rssiDbm: -55,
      speakerActive: true,
      microphoneActive: true,
      audioValid: true,
      audioFlags: 0x83,
      batteryPercent: 80,
      batteryState: 2,
    },
  );
  assert.deepEqual(decodeLegacyTelemetry(Uint8Array.of(1, 0x80, 0xff, 0)), {
    rssiDbm: null,
    speakerActive: false,
    microphoneActive: false,
    audioValid: true,
    audioFlags: 0x80,
    batteryPercent: null,
    batteryState: 0,
  });
  assert.throws(() => decodeLegacyTelemetry(Uint8Array.of(0, 0, 0)), /长度不足/);

  const referenceLayout = new Uint8Array(13);
  referenceLayout.set([0xb0, 0x02], 0);
  referenceLayout.set([70, 3], 11);
  assert.deepEqual(decodeLegacyTelemetry(referenceLayout), {
    rssiDbm: -80,
    speakerActive: true,
    microphoneActive: false,
    audioValid: false,
    audioFlags: 0x02,
    batteryPercent: 70,
    batteryState: 3,
  });
});

test("FB remap codec is exactly 15 x 4 bytes and command framing is 63 bytes", () => {
  const entries = identityLegacyRemap();
  entries[0] = { type: 0, value: 14, modifier: 0, flags: 1 };
  entries[14] = { type: 1, value: 0x2c, modifier: 0x02, flags: 0x03 };

  const encoded = encodeLegacyRemap(entries);
  assert.equal(entries.length, LEGACY_REMAP_COUNT);
  assert.equal(encoded.byteLength, LEGACY_REMAP_SIZE);
  assert.deepEqual(Array.from(encoded.subarray(0, 4)), [0, 14, 0, 1]);
  assert.deepEqual(Array.from(encoded.subarray(56, 60)), [1, 0x2c, 0x02, 0x03]);
  assert.deepEqual(decodeLegacyRemap(encoded), entries);
  assert.deepEqual(decodeLegacyRemap(Uint8Array.of(LEGACY_REPORT_REMAP, ...encoded)), entries);

  const command = encodeLegacyRemapCommand(entries);
  assert.equal(command.byteLength, 63);
  assert.equal(command[0], 1);
  assert.deepEqual(command.subarray(1, 61), encoded);
  assert.deepEqual(Array.from(command.subarray(61)), [0, 0]);
});

test("LegacyHidClient sends and receives only F6/F7/F8/F9/FB reports", async () => {
  const configBytes = encodeLegacyConfig({ ...DEFAULT_LEGACY_CONFIG });
  const remapBytes = encodeLegacyRemap(identityLegacyRemap());
  const sent: Array<{ reportId: number; data: Uint8Array }> = [];
  let opened = false;
  const device = {
    get opened() { return opened; },
    vendorId: 0x054c,
    productId: 0x0ce6,
    productName: "DS5Dongle",
    collections: [{ usagePage: 1, usage: 5 }],
    async open() { opened = true; },
    async close() { opened = false; },
    async receiveFeatureReport(reportId: number) {
      if (reportId === LEGACY_REPORT_CONFIG) return dataView(configBytes);
      if (reportId === LEGACY_REPORT_FIRMWARE_VERSION) {
        return dataView(new TextEncoder().encode("3.5.1\0"));
      }
      if (reportId === LEGACY_REPORT_TELEMETRY) return dataView(Uint8Array.of(0xd8, 0x82, 90, 1));
      if (reportId === LEGACY_REPORT_REMAP) return dataView(remapBytes);
      throw new Error(`unexpected report ${reportId}`);
    },
    async sendFeatureReport(reportId: number, data: Uint8Array) {
      sent.push({ reportId, data: data.slice() });
    },
    async sendReport() {},
    addEventListener() {},
    removeEventListener() {},
    dispatchEvent() { return true; },
  } as unknown as HIDDevice;

  assert.equal(LegacyHidClient.supports(device), true);
  const client = new LegacyHidClient(device);
  assert.equal((await client.readConfig()).configVersion, 2);
  assert.equal(await client.readFirmwareVersion(), "3.5.1");
  assert.equal((await client.readTelemetry()).rssiDbm, -40);
  assert.equal((await client.readRemap()).length, 15);
  await client.applyConfig({ ...DEFAULT_LEGACY_CONFIG });
  await client.saveConfig();
  await client.reconnectUsb();
  await client.writeRemap(identityLegacyRemap());
  await client.resetRemap();

  assert.deepEqual(sent.map(({ reportId, data }) => [reportId, data[0]]), [
    [LEGACY_REPORT_CONFIG_COMMAND, 1],
    [LEGACY_REPORT_CONFIG_COMMAND, 2],
    [LEGACY_REPORT_CONFIG_COMMAND, 3],
    [LEGACY_REPORT_REMAP, 1],
    [LEGACY_REPORT_REMAP, 2],
  ]);
});

function dataView(bytes: Uint8Array): DataView {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}
