/**
 * DS5Dongle legacy configuration reports (0xF6-0xF9/0xFB).
 *
 * OTA deliberately does not live in this module. Firmware updates must keep
 * using the authenticated 0xFA/0xFC protocol implemented by Ds5DongleClient.
 */

export const LEGACY_SONY_VENDOR_ID = 0x054c;
export const LEGACY_SUPPORTED_PRODUCT_IDS = [0x0ce6, 0x0df2] as const;
export const LEGACY_GAMEPAD_USAGE_PAGE = 0x01;
export const LEGACY_GAMEPAD_USAGE = 0x05;

export const LEGACY_REPORT_CONFIG_COMMAND = 0xf6;
export const LEGACY_REPORT_CONFIG = 0xf7;
export const LEGACY_REPORT_FIRMWARE_VERSION = 0xf8;
export const LEGACY_REPORT_TELEMETRY = 0xf9;
export const LEGACY_REPORT_REMAP = 0xfb;

export const LEGACY_FEATURE_PAYLOAD_SIZE = 63;
export const LEGACY_CONFIG_VERSION = 2;
export const LEGACY_CONFIG_SIZE = 25;
export const LEGACY_REMAP_COUNT = 15;
export const LEGACY_REMAP_ENTRY_SIZE = 4;
export const LEGACY_REMAP_SIZE = LEGACY_REMAP_COUNT * LEGACY_REMAP_ENTRY_SIZE;

export const LEGACY_HID_FILTERS: HIDDeviceFilter[] = LEGACY_SUPPORTED_PRODUCT_IDS.map(
  (productId) => ({
    vendorId: LEGACY_SONY_VENDOR_ID,
    productId,
    usagePage: LEGACY_GAMEPAD_USAGE_PAGE,
    usage: LEGACY_GAMEPAD_USAGE,
  }),
);

export type LegacyPollingRateMode = 0 | 1 | 2;
export type LegacyControllerMode = 0 | 1 | 2;

export interface LegacyConfig {
  configVersion: number;
  hapticsGain: number;
  speakerVolume: number;
  headsetVolume: number;
  speakerGain: number;
  inactiveTime: number;
  disableLed: boolean;
  pollingRateMode: LegacyPollingRateMode;
  audioBufferLength: number;
  controllerMode: LegacyControllerMode;
  enableUsbSerial: boolean;
  psShortcutEnabled: boolean;
  disableMic: boolean;
  disableSpeaker: boolean;
  enableWake: boolean;
  triggerReduce: number;
  lockVolume: boolean;
  dseDetected: boolean;
  usbStealth: boolean;
  ledR: number;
  ledG: number;
  ledB: number;
}

export const DEFAULT_LEGACY_CONFIG: Readonly<LegacyConfig> = Object.freeze({
  configVersion: LEGACY_CONFIG_VERSION,
  hapticsGain: 1,
  speakerVolume: 100,
  headsetVolume: 100,
  speakerGain: 2,
  inactiveTime: 30,
  disableLed: true,
  pollingRateMode: 0,
  audioBufferLength: 64,
  controllerMode: 2,
  enableUsbSerial: true,
  psShortcutEnabled: false,
  disableMic: false,
  disableSpeaker: false,
  enableWake: false,
  triggerReduce: 0,
  lockVolume: false,
  dseDetected: false,
  usbStealth: false,
  ledR: 0xff,
  ledG: 0xff,
  ledB: 0xff,
});

export interface LegacyTelemetry {
  /** Negative RSSI in dBm; null means the controller is not connected yet. */
  rssiDbm: number | null;
  speakerActive: boolean;
  microphoneActive: boolean;
  /** Bit 7 tells us that the audio status bits are supplied by firmware. */
  audioValid: boolean;
  /** Raw F9 audio flag byte, including firmware's validity bit 7. */
  audioFlags: number;
  /** 0..100, or null when firmware reports 0xFF/another invalid value. */
  batteryPercent: number | null;
  batteryState: number;
}

export type LegacyRemapType = 0 | 1;

export interface LegacyRemapEntry {
  type: LegacyRemapType;
  value: number;
  modifier: number;
  flags: number;
}

export const LEGACY_BUTTON_NAMES = [
  "Square",
  "Cross",
  "Circle",
  "Triangle",
  "L1",
  "R1",
  "L2",
  "R2",
  "Create",
  "Options",
  "L3",
  "R3",
  "PS",
  "Touchpad",
  "Mute",
] as const;

export class LegacyHidError extends Error {
  readonly code: string;

  constructor(message: string, code: string) {
    super(message);
    this.name = "LegacyHidError";
    this.code = code;
  }
}

export type LegacyBufferSource = ArrayBuffer | ArrayBufferView | Uint8Array;

export function validateLegacyConfig(config: LegacyConfig): void {
  if (config.configVersion !== LEGACY_CONFIG_VERSION) {
    throw new LegacyHidError(
      `配置版本必须为 ${LEGACY_CONFIG_VERSION}`,
      "config-version",
    );
  }
  if (!Number.isFinite(config.hapticsGain) || config.hapticsGain < 1 || config.hapticsGain > 2) {
    throw new LegacyHidError("触觉增益必须在 1.0 到 2.0 之间", "config-range");
  }

  assertInteger(config.speakerVolume, 0, 127, "扬声器音量");
  assertInteger(config.headsetVolume, 0, 127, "耳机音量");
  assertInteger(config.speakerGain, 0, 7, "扬声器增益");
  assertInteger(config.inactiveTime, 0, 60, "闲置断开时间");
  assertInteger(config.pollingRateMode, 0, 2, "轮询率模式");
  assertInteger(config.audioBufferLength, 16, 127, "音频缓冲长度");
  assertInteger(config.controllerMode, 0, 2, "手柄模式");
  assertInteger(config.triggerReduce, 0, 10, "扳机功率削减");
  assertInteger(config.ledR, 0, 255, "LED 红色");
  assertInteger(config.ledG, 0, 255, "LED 绿色");
  assertInteger(config.ledB, 0, 255, "LED 蓝色");

  for (const [name, value] of Object.entries({
    disableLed: config.disableLed,
    enableUsbSerial: config.enableUsbSerial,
    psShortcutEnabled: config.psShortcutEnabled,
    disableMic: config.disableMic,
    disableSpeaker: config.disableSpeaker,
    enableWake: config.enableWake,
    lockVolume: config.lockVolume,
    dseDetected: config.dseDetected,
    usbStealth: config.usbStealth,
  })) {
    if (typeof value !== "boolean") {
      throw new LegacyHidError(`${name} 必须是布尔值`, "config-type");
    }
  }
}

export function encodeLegacyConfig(config: LegacyConfig): Uint8Array {
  validateLegacyConfig(config);
  const bytes = new Uint8Array(LEGACY_CONFIG_SIZE);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, LEGACY_CONFIG_VERSION);
  view.setFloat32(1, config.hapticsGain, true);
  view.setUint8(5, config.speakerVolume);
  view.setUint8(6, config.headsetVolume);
  view.setUint8(7, config.speakerGain);
  view.setUint8(8, config.inactiveTime);
  view.setUint8(9, Number(config.disableLed));
  view.setUint8(10, config.pollingRateMode);
  view.setUint8(11, config.audioBufferLength);
  view.setUint8(12, config.controllerMode);
  view.setUint8(13, Number(config.enableUsbSerial));
  view.setUint8(14, Number(config.psShortcutEnabled));
  view.setUint8(15, Number(config.disableMic));
  view.setUint8(16, Number(config.disableSpeaker));
  view.setUint8(17, Number(config.enableWake));
  view.setUint8(18, config.triggerReduce);
  view.setUint8(19, Number(config.lockVolume));
  view.setUint8(20, Number(config.dseDetected));
  view.setUint8(21, Number(config.usbStealth));
  view.setUint8(22, config.ledR);
  view.setUint8(23, config.ledG);
  view.setUint8(24, config.ledB);
  return bytes;
}

export function decodeLegacyConfig(source: LegacyBufferSource): LegacyConfig {
  const bytes = reportPayload(source, LEGACY_REPORT_CONFIG);
  if (bytes.byteLength < LEGACY_CONFIG_SIZE) {
    throw new LegacyHidError(
      `F7 配置长度不足：${bytes.byteLength}/${LEGACY_CONFIG_SIZE}`,
      "config-size",
    );
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, LEGACY_CONFIG_SIZE);
  const config: LegacyConfig = {
    configVersion: view.getUint8(0),
    hapticsGain: view.getFloat32(1, true),
    speakerVolume: view.getUint8(5),
    headsetVolume: view.getUint8(6),
    speakerGain: view.getUint8(7),
    inactiveTime: view.getUint8(8),
    disableLed: decodeBoolean(view.getUint8(9), "disableLed"),
    pollingRateMode: view.getUint8(10) as LegacyPollingRateMode,
    audioBufferLength: view.getUint8(11),
    controllerMode: view.getUint8(12) as LegacyControllerMode,
    enableUsbSerial: decodeBoolean(view.getUint8(13), "enableUsbSerial"),
    psShortcutEnabled: decodeBoolean(view.getUint8(14), "psShortcutEnabled"),
    disableMic: decodeBoolean(view.getUint8(15), "disableMic"),
    disableSpeaker: decodeBoolean(view.getUint8(16), "disableSpeaker"),
    enableWake: decodeBoolean(view.getUint8(17), "enableWake"),
    triggerReduce: view.getUint8(18),
    lockVolume: decodeBoolean(view.getUint8(19), "lockVolume"),
    dseDetected: decodeBoolean(view.getUint8(20), "dseDetected"),
    usbStealth: decodeBoolean(view.getUint8(21), "usbStealth"),
    ledR: view.getUint8(22),
    ledG: view.getUint8(23),
    ledB: view.getUint8(24),
  };
  validateLegacyConfig(config);
  return config;
}

export function encodeLegacyConfigCommand(
  command: 1 | 2 | 3,
  config?: LegacyConfig,
): Uint8Array {
  const report = new Uint8Array(LEGACY_FEATURE_PAYLOAD_SIZE);
  report[0] = command;
  if (command === 1) {
    if (!config) {
      throw new LegacyHidError("应用配置命令缺少配置内容", "config-command");
    }
    report.set(encodeLegacyConfig(config), 1);
  }
  return report;
}

export function decodeLegacyFirmwareVersion(source: LegacyBufferSource): string {
  const bytes = reportPayload(source, LEGACY_REPORT_FIRMWARE_VERSION);
  let end = bytes.byteLength;
  while (end > 0 && (bytes[end - 1] === 0 || bytes[end - 1] === 0xff)) end--;
  const version = new TextDecoder("ascii", { fatal: true }).decode(bytes.subarray(0, end)).trim();
  if (!version || !/^[\x20-\x7e]+$/.test(version)) {
    throw new LegacyHidError("F8 固件版本为空或包含非法字符", "firmware-version");
  }
  return version;
}

export function decodeLegacyTelemetry(source: LegacyBufferSource): LegacyTelemetry {
  const bytes = reportPayload(source, LEGACY_REPORT_TELEMETRY);
  if (bytes.byteLength < 4) {
    throw new LegacyHidError(`F9 遥测长度不足：${bytes.byteLength}/4`, "telemetry-size");
  }
  const rawRssi = bytes[0] > 0x7f ? bytes[0] - 0x100 : bytes[0];
  const audioFlags = bytes[1];
  // Current M61 firmware returns a compact four-byte payload. The reference
  // site's older firmware keeps battery level/state at offsets 11/12.
  const batteryOffset = bytes.byteLength >= 13 ? 11 : 2;
  const rawBattery = bytes[batteryOffset];
  return {
    rssiDbm: rawRssi < 0 ? rawRssi : null,
    speakerActive: Boolean(audioFlags & 0x02),
    microphoneActive: Boolean(audioFlags & 0x01),
    audioValid: Boolean(audioFlags & 0x80),
    audioFlags,
    batteryPercent: rawBattery <= 100 ? rawBattery : null,
    batteryState: bytes[batteryOffset + 1],
  };
}

export function identityLegacyRemap(): LegacyRemapEntry[] {
  return Array.from({ length: LEGACY_REMAP_COUNT }, (_, value) => ({
    type: 0,
    value,
    modifier: 0,
    flags: 0,
  }));
}

/** Encode only the 15 x 4-byte table, without the 0xFB command byte. */
export function encodeLegacyRemap(entries: readonly LegacyRemapEntry[]): Uint8Array {
  if (entries.length !== LEGACY_REMAP_COUNT) {
    throw new LegacyHidError(`按键映射必须正好有 ${LEGACY_REMAP_COUNT} 项`, "remap-size");
  }
  const bytes = new Uint8Array(LEGACY_REMAP_SIZE);
  entries.forEach((entry, index) => {
    validateLegacyRemapEntry(entry, index);
    const base = index * LEGACY_REMAP_ENTRY_SIZE;
    bytes[base] = entry.type;
    bytes[base + 1] = entry.value;
    bytes[base + 2] = entry.modifier;
    bytes[base + 3] = entry.flags;
  });
  return bytes;
}

export function decodeLegacyRemap(source: LegacyBufferSource): LegacyRemapEntry[] {
  const bytes = reportPayload(source, LEGACY_REPORT_REMAP);
  if (bytes.byteLength < LEGACY_REMAP_SIZE) {
    throw new LegacyHidError(
      `FB 映射长度不足：${bytes.byteLength}/${LEGACY_REMAP_SIZE}`,
      "remap-size",
    );
  }
  return Array.from({ length: LEGACY_REMAP_COUNT }, (_, index) => {
    const base = index * LEGACY_REMAP_ENTRY_SIZE;
    const entry: LegacyRemapEntry = {
      type: bytes[base] as LegacyRemapType,
      value: bytes[base + 1],
      modifier: bytes[base + 2],
      flags: bytes[base + 3],
    };
    validateLegacyRemapEntry(entry, index);
    return entry;
  });
}

export function encodeLegacyRemapCommand(entries: readonly LegacyRemapEntry[]): Uint8Array {
  const report = new Uint8Array(LEGACY_FEATURE_PAYLOAD_SIZE);
  report[0] = 1;
  report.set(encodeLegacyRemap(entries), 1);
  return report;
}

export function encodeLegacyRemapResetCommand(): Uint8Array {
  const report = new Uint8Array(LEGACY_FEATURE_PAYLOAD_SIZE);
  report[0] = 2;
  return report;
}

export class LegacyHidClient {
  readonly device: HIDDevice;

  constructor(device: HIDDevice) {
    this.device = device;
  }

  static supports(device: HIDDevice): boolean {
    return (
      device.vendorId === LEGACY_SONY_VENDOR_ID &&
      LEGACY_SUPPORTED_PRODUCT_IDS.includes(
        device.productId as (typeof LEGACY_SUPPORTED_PRODUCT_IDS)[number],
      ) &&
      device.collections.some(
        (collection) =>
          collection.usagePage === LEGACY_GAMEPAD_USAGE_PAGE &&
          collection.usage === LEGACY_GAMEPAD_USAGE,
      )
    );
  }

  static async choose(): Promise<LegacyHidClient> {
    const hid = requireLegacyWebHid();
    const devices = await hid.requestDevice({ filters: LEGACY_HID_FILTERS.map((item) => ({ ...item })) });
    const device = devices.find(LegacyHidClient.supports);
    if (!device) throw new LegacyHidError("没有选择兼容的 DS5Dongle", "device-not-selected");
    const client = new LegacyHidClient(device);
    await client.open();
    return client;
  }

  static async authorized(): Promise<LegacyHidClient | null> {
    const hid = typeof navigator === "undefined" ? undefined : navigator.hid;
    if (!hid) return null;
    const device = (await hid.getDevices()).find(LegacyHidClient.supports);
    if (!device) return null;
    const client = new LegacyHidClient(device);
    await client.open();
    return client;
  }

  async open(): Promise<void> {
    if (!this.device.opened) await this.device.open();
  }

  async close(): Promise<void> {
    if (this.device.opened) await this.device.close();
  }

  async readConfig(): Promise<LegacyConfig> {
    await this.open();
    return decodeLegacyConfig(await this.device.receiveFeatureReport(LEGACY_REPORT_CONFIG));
  }

  async applyConfig(config: LegacyConfig): Promise<void> {
    await this.open();
    await this.device.sendFeatureReport(
      LEGACY_REPORT_CONFIG_COMMAND,
      encodeLegacyConfigCommand(1, config),
    );
  }

  async saveConfig(): Promise<void> {
    await this.open();
    await this.device.sendFeatureReport(
      LEGACY_REPORT_CONFIG_COMMAND,
      encodeLegacyConfigCommand(2),
    );
  }

  async reconnectUsb(): Promise<void> {
    await this.open();
    await this.device.sendFeatureReport(
      LEGACY_REPORT_CONFIG_COMMAND,
      encodeLegacyConfigCommand(3),
    );
  }

  async readFirmwareVersion(): Promise<string> {
    await this.open();
    return decodeLegacyFirmwareVersion(
      await this.device.receiveFeatureReport(LEGACY_REPORT_FIRMWARE_VERSION),
    );
  }

  async readTelemetry(): Promise<LegacyTelemetry> {
    await this.open();
    return decodeLegacyTelemetry(await this.device.receiveFeatureReport(LEGACY_REPORT_TELEMETRY));
  }

  async readRemap(): Promise<LegacyRemapEntry[]> {
    await this.open();
    return decodeLegacyRemap(await this.device.receiveFeatureReport(LEGACY_REPORT_REMAP));
  }

  async writeRemap(entries: readonly LegacyRemapEntry[]): Promise<void> {
    await this.open();
    await this.device.sendFeatureReport(LEGACY_REPORT_REMAP, encodeLegacyRemapCommand(entries));
  }

  async resetRemap(): Promise<void> {
    await this.open();
    await this.device.sendFeatureReport(LEGACY_REPORT_REMAP, encodeLegacyRemapResetCommand());
  }
}

function requireLegacyWebHid(): HID {
  const hid = typeof navigator === "undefined" ? undefined : navigator.hid;
  if (!hid) {
    throw new LegacyHidError(
      "当前浏览器不支持 WebHID，请使用最新版 Chrome 或 Edge",
      "webhid-unavailable",
    );
  }
  return hid;
}

function asBytes(source: LegacyBufferSource): Uint8Array {
  if (source instanceof Uint8Array) return source;
  if (source instanceof ArrayBuffer) return new Uint8Array(source);
  return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
}

function reportPayload(source: LegacyBufferSource, reportId: number): Uint8Array {
  const bytes = asBytes(source);
  return bytes[0] === reportId ? bytes.subarray(1) : bytes;
}

function decodeBoolean(value: number, name: string): boolean {
  if (value !== 0 && value !== 1) {
    throw new LegacyHidError(`${name} 的设备值不是 0/1`, "config-boolean");
  }
  return value === 1;
}

function assertInteger(value: number, min: number, max: number, name: string): void {
  if (!Number.isInteger(value) || value < min || value > max) {
    throw new LegacyHidError(`${name} 必须是 ${min} 到 ${max} 的整数`, "config-range");
  }
}

function validateLegacyRemapEntry(entry: LegacyRemapEntry, index: number): void {
  if (entry.type !== 0 && entry.type !== 1) {
    throw new LegacyHidError(`映射 ${index} 的类型无效`, "remap-type");
  }
  assertByte(entry.value, `映射 ${index} 的目标`);
  assertByte(entry.modifier, `映射 ${index} 的修饰键`);
  assertByte(entry.flags, `映射 ${index} 的标志`);
  if (entry.type === 0 && entry.value >= LEGACY_REMAP_COUNT) {
    throw new LegacyHidError(`映射 ${index} 的手柄目标无效`, "remap-value");
  }
  if ((entry.flags & ~0x03) !== 0) {
    throw new LegacyHidError(`映射 ${index} 含未知标志位`, "remap-flags");
  }
}

function assertByte(value: number, name: string): void {
  if (!Number.isInteger(value) || value < 0 || value > 0xff) {
    throw new LegacyHidError(`${name} 必须是一个字节`, "remap-byte");
  }
}
