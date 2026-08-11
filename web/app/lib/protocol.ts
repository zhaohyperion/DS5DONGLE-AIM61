export const SONY_VENDOR_ID = 0x054c;
export const SUPPORTED_PRODUCT_IDS = [0x0ce6, 0x0df2] as const;

export const REPORT_CONFIG_COMMAND = 0xf6;
export const REPORT_CONFIG = 0xf7;
export const REPORT_FIRMWARE_VERSION = 0xf8;
export const REPORT_DEVICE_STATUS = 0xf9;
export const REPORT_OTA_DATA = 0xfa;
export const REPORT_REMAP = 0xfb;
export const REPORT_OTA_CONTROL = 0xfc;

export const FEATURE_PAYLOAD_SIZE = 63;
export const CONFIG_VERSION = 2;
export const CONFIG_BODY_SIZE = 25;
export const REMAP_BUTTON_COUNT = 15;
export const REMAP_ENTRY_SIZE = 4;
export const OTA_HEADER_SIZE = 512;
export const OTA_DATA_SIZE = 46;
export const OTA_FRAME_SIZE = 63;
export const OTA_PROTOCOL_VERSION = 1;

export const OTA_FRAME_DATA = 0x10;

export const OtaOpcode = {
  Begin: 0x01,
  Auth: 0x02,
  Commit: 0x03,
  Abort: 0x04,
  Status: 0x05,
  Ack: 0x80,
  Error: 0xff,
} as const;

export const OtaState = {
  Idle: 0x00,
  Authorizing: 0x01,
  Preparing: 0x02,
  Receiving: 0x03,
  Verifying: 0x04,
  ReadyToReboot: 0x05,
  Error: 0x06,
  Aborted: 0x07,
} as const;

export type OtaStateValue = (typeof OtaState)[keyof typeof OtaState];

export interface DeviceConfig {
  configVersion: number;
  hapticsGain: number;
  speakerVolume: number;
  headsetVolume: number;
  speakerGain: number;
  inactiveTime: number;
  disableLed: boolean;
  pollingRateMode: 0 | 1 | 2;
  audioBufferLength: number;
  controllerMode: 0 | 1 | 2;
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

export interface RemapEntry {
  type: 0 | 1;
  value: number;
  modifier: number;
  flags: number;
}

export interface DeviceStatus {
  rssi: number | null;
  audioStatusValid: boolean;
  speakerActive: boolean;
  microphoneActive: boolean;
  batteryLevel: number | null;
  batteryState: number | null;
}

export interface OtaHeader {
  marker: string;
  type: "RAW";
  bodyLength: number;
  hardwareVersion: string;
  softwareVersion: string;
  bodySha256: string;
}

export interface OtaImageInfo extends OtaHeader {
  fileSize: number;
  fileSha256: string;
  firmwareVersion: string;
}

export interface OtaSignature {
  algorithm: "ECDSA-P256-SHA256";
  key_id: string;
  scope: "DS5DONGLE-OTA-V1";
  value: string;
}

export interface OtaManifest {
  schema: 1;
  channel: "dev" | "beta" | "stable";
  board: "aim61";
  usb_speed: "fs";
  version: string;
  size: number;
  sha256: string;
  body_size: number;
  body_sha256: string;
  url: string;
  signature: OtaSignature | null;
}

export interface OtaStatus {
  protocolVersion: number;
  state: OtaStateValue;
  session: number;
  acceptedOffset: number;
  committedOffset: number;
  totalSize: number;
  error: number;
  lastRequest: number;
  flags: number;
  capabilities: number;
  maxData: number;
  windowFrames: number;
  maxImageSize: number;
  board: number;
  usbSpeed: number;
  format: number;
  activeSlot: number;
  trialRetryFlag: number;
  rebootDelayMs: number;
  firmwareVersion: string;
}

export const DEFAULT_CONFIG: DeviceConfig = {
  configVersion: CONFIG_VERSION,
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
  ledR: 255,
  ledG: 255,
  ledB: 255,
};

export const BUTTON_NAMES = [
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

export function identityRemap(): RemapEntry[] {
  return Array.from({ length: REMAP_BUTTON_COUNT }, (_, value) => ({
    type: 0,
    value,
    modifier: 0,
    flags: 0,
  }));
}

export function encodeConfig(config: DeviceConfig): Uint8Array {
  validateConfig(config);
  const bytes = new Uint8Array(CONFIG_BODY_SIZE);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, CONFIG_VERSION);
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

export function decodeConfig(source: BufferSourceLike): DeviceConfig {
  const bytes = asBytes(source);
  const offset = bytes[0] === REPORT_CONFIG && bytes[1] === CONFIG_VERSION ? 1 : 0;
  if (bytes.byteLength - offset < CONFIG_BODY_SIZE) {
    throw new ProtocolError("配置报告长度不足", "config-size");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset + offset, CONFIG_BODY_SIZE);
  if (view.getUint8(0) !== CONFIG_VERSION) {
    throw new ProtocolError(
      `配置版本不匹配：设备 ${view.getUint8(0)}，网页 ${CONFIG_VERSION}`,
      "config-version",
    );
  }
  const config: DeviceConfig = {
    configVersion: view.getUint8(0),
    hapticsGain: view.getFloat32(1, true),
    speakerVolume: view.getUint8(5),
    headsetVolume: view.getUint8(6),
    speakerGain: view.getUint8(7),
    inactiveTime: view.getUint8(8),
    disableLed: Boolean(view.getUint8(9)),
    pollingRateMode: view.getUint8(10) as 0 | 1 | 2,
    audioBufferLength: view.getUint8(11),
    controllerMode: view.getUint8(12) as 0 | 1 | 2,
    enableUsbSerial: Boolean(view.getUint8(13)),
    psShortcutEnabled: Boolean(view.getUint8(14)),
    disableMic: Boolean(view.getUint8(15)),
    disableSpeaker: Boolean(view.getUint8(16)),
    enableWake: Boolean(view.getUint8(17)),
    triggerReduce: view.getUint8(18),
    lockVolume: Boolean(view.getUint8(19)),
    dseDetected: Boolean(view.getUint8(20)),
    usbStealth: Boolean(view.getUint8(21)),
    ledR: view.getUint8(22),
    ledG: view.getUint8(23),
    ledB: view.getUint8(24),
  };
  validateConfig(config);
  return config;
}

export function validateConfig(config: DeviceConfig): void {
  const integerRange = (value: number, min: number, max: number, name: string) => {
    if (!Number.isInteger(value) || value < min || value > max) {
      throw new ProtocolError(`${name} 超出范围`, "config-range");
    }
  };
  if (!Number.isFinite(config.hapticsGain) || config.hapticsGain < 1 || config.hapticsGain > 2) {
    throw new ProtocolError("触觉增益超出范围", "config-range");
  }
  integerRange(config.speakerVolume, 0, 127, "扬声器音量");
  integerRange(config.headsetVolume, 0, 127, "耳机音量");
  integerRange(config.speakerGain, 0, 7, "扬声器增益");
  integerRange(config.inactiveTime, 0, 60, "闲置时间");
  integerRange(config.pollingRateMode, 0, 2, "轮询模式");
  integerRange(config.audioBufferLength, 16, 127, "音频缓冲");
  integerRange(config.controllerMode, 0, 2, "控制器模式");
  integerRange(config.triggerReduce, 0, 10, "扳机反馈削减");
  integerRange(config.ledR, 0, 255, "LED R");
  integerRange(config.ledG, 0, 255, "LED G");
  integerRange(config.ledB, 0, 255, "LED B");
}

export function configCommand(command: 1 | 2 | 3, config?: DeviceConfig): Uint8Array {
  const report = new Uint8Array(FEATURE_PAYLOAD_SIZE);
  report[0] = command;
  if (command === 1) {
    if (!config) throw new ProtocolError("应用配置命令缺少配置体", "config-command");
    report.set(encodeConfig(config), 1);
  }
  return report;
}

export function decodeFirmwareVersion(source: BufferSourceLike): string {
  let bytes = asBytes(source);
  if (bytes[0] === REPORT_FIRMWARE_VERSION) bytes = bytes.subarray(1);
  let end = bytes.length;
  while (end > 0 && (bytes[end - 1] === 0 || bytes[end - 1] === 0xff)) end--;
  return new TextDecoder().decode(bytes.subarray(0, end)).trim();
}

export function decodeDeviceStatus(source: BufferSourceLike): DeviceStatus {
  const bytes = asBytes(source);
  const offset = bytes[0] === REPORT_DEVICE_STATUS ? 1 : 0;
  const payloadLength = bytes.byteLength - offset;
  const rawRssi = bytes[offset];
  const signed = rawRssi === undefined ? null : rawRssi > 127 ? rawRssi - 256 : null;
  const flags = bytes[offset + 1] ?? 0;
  // The original ds5678 layout reserves bytes 2..10 for OTA progress and
  // exposes battery at 11/12. Our Ai-M61 firmware uses a compact 4-byte F9
  // payload because authenticated OTA status lives on 0xFC. Accept both.
  const batteryOffset = payloadLength >= 13 ? 11 : 2;
  const rawBattery = bytes[offset + batteryOffset];
  const rawBatteryState = bytes[offset + batteryOffset + 1];
  return {
    rssi: signed,
    audioStatusValid: Boolean(flags & 0x80),
    speakerActive: Boolean(flags & 0x80) && Boolean(flags & 0x02),
    microphoneActive: Boolean(flags & 0x80) && Boolean(flags & 0x01),
    batteryLevel: rawBattery !== undefined && rawBattery <= 100 ? rawBattery : null,
    batteryState: rawBatteryState === undefined ? null : rawBatteryState & 0x0f,
  };
}

export function encodeRemap(entries: RemapEntry[]): Uint8Array {
  if (entries.length !== REMAP_BUTTON_COUNT) {
    throw new ProtocolError("按键映射数量必须为 15", "remap-size");
  }
  const report = new Uint8Array(FEATURE_PAYLOAD_SIZE);
  report[0] = 1;
  entries.forEach((entry, index) => {
    const validValue = Number.isInteger(entry.value) && entry.value >= 0 && entry.value <= 0xff;
    const validModifier = Number.isInteger(entry.modifier) && entry.modifier >= 0 && entry.modifier <= 0xff;
    const validFlags = Number.isInteger(entry.flags) && entry.flags >= 0 && entry.flags <= 0x03;
    if ((entry.type !== 0 && entry.type !== 1) || !validValue || !validModifier || !validFlags) {
      throw new ProtocolError("按键映射字段超出 1 字节协议范围", "remap-value");
    }
    if (entry.type === 0 && entry.value >= REMAP_BUTTON_COUNT) {
      throw new ProtocolError("手柄按键目标超出范围", "remap-value");
    }
    const base = 1 + index * REMAP_ENTRY_SIZE;
    report[base] = entry.type;
    report[base + 1] = entry.value;
    report[base + 2] = entry.modifier;
    report[base + 3] = entry.flags;
  });
  return report;
}

export function decodeRemap(source: BufferSourceLike): RemapEntry[] {
  const bytes = asBytes(source);
  const offset = bytes[0] === REPORT_REMAP ? 1 : 0;
  if (bytes.byteLength - offset < REMAP_BUTTON_COUNT * REMAP_ENTRY_SIZE) {
    throw new ProtocolError("按键映射报告长度不足", "remap-size");
  }
  return Array.from({ length: REMAP_BUTTON_COUNT }, (_, index) => {
    const base = offset + index * REMAP_ENTRY_SIZE;
    return {
      type: bytes[base] as 0 | 1,
      value: bytes[base + 1],
      modifier: bytes[base + 2],
      flags: bytes[base + 3],
    };
  });
}

export function parseOtaHeader(source: BufferSourceLike): OtaHeader {
  const bytes = asBytes(source);
  if (bytes.byteLength < OTA_HEADER_SIZE) {
    throw new ProtocolError("文件不足 512 字节，不是 BL60X OTA 镜像", "ota-header-size");
  }
  const marker = ascii(bytes.subarray(0, 16), false);
  if (marker !== "BL60X_OTA_Ver1.0") {
    throw new ProtocolError("OTA 头标记不是 BL60X_OTA_Ver1.0", "ota-header-marker");
  }
  const rawType = ascii(bytes.subarray(16, 20), false);
  if (rawType !== "RAW ") {
    throw new ProtocolError("仅支持 RAW .bin.ota，不支持 XZ 或普通 BIN", "ota-header-type");
  }
  const bodyLength = new DataView(bytes.buffer, bytes.byteOffset + 20, 4).getUint32(0, true);
  if (bodyLength === 0) {
    throw new ProtocolError("OTA 头中的固件长度为 0", "ota-header-length");
  }
  const bodySha256 = hex(bytes.subarray(64, 96));
  if (/^0+$/.test(bodySha256) || /^f+$/.test(bodySha256)) {
    throw new ProtocolError("OTA 头缺少有效的固件 SHA-256", "ota-header-hash");
  }
  return {
    marker,
    type: "RAW",
    bodyLength,
    hardwareVersion: ascii(bytes.subarray(32, 48)),
    softwareVersion: ascii(bytes.subarray(48, 64)),
    bodySha256,
  };
}

export async function inspectOtaImage(source: BufferSourceLike): Promise<OtaImageInfo> {
  const bytes = asBytes(source);
  const header = parseOtaHeader(bytes);
  if (header.bodyLength !== bytes.byteLength - OTA_HEADER_SIZE) {
    throw new ProtocolError(
      `OTA 长度不一致：头部声明 ${header.bodyLength}，实际 ${bytes.byteLength - OTA_HEADER_SIZE}`,
      "ota-body-length",
    );
  }
  const bodySha256 = await sha256Hex(bytes.subarray(OTA_HEADER_SIZE));
  if (bodySha256 !== header.bodySha256) {
    throw new ProtocolError("OTA 固件体 SHA-256 与头部不一致", "ota-body-hash");
  }
  const match = header.softwareVersion.match(/^EVENT_V((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))$/);
  if (!match) {
    throw new ProtocolError("OTA 头软件版本必须严格为 EVENT_VX.Y.Z", "ota-version");
  }
  versionTriplet(match[1]);
  return {
    ...header,
    fileSize: bytes.byteLength,
    fileSha256: await sha256Hex(bytes),
    firmwareVersion: match[1],
  };
}

export function parseManifest(
  value: unknown,
  baseUrl?: string,
  options: { allowUnsignedDev?: boolean } = {},
): OtaManifest {
  if (!value || typeof value !== "object") {
    throw new ProtocolError("在线清单不是 JSON 对象", "manifest-shape");
  }
  const raw = value as Record<string, unknown>;
  const expectedKeys = [
    "board",
    "body_sha256",
    "body_size",
    "channel",
    "schema",
    "sha256",
    "signature",
    "size",
    "url",
    "usb_speed",
    "version",
  ];
  const actualKeys = Object.keys(raw).sort();
  if (actualKeys.length !== expectedKeys.length || actualKeys.some((key, index) => key !== expectedKeys[index])) {
    throw new ProtocolError("在线清单字段集合不符合 OTA schema 1", "manifest-fields");
  }
  if (raw.schema !== 1) throw new ProtocolError("不支持的清单版本", "manifest-schema");
  if (!(["dev", "beta", "stable"] as unknown[]).includes(raw.channel)) {
    throw new ProtocolError("清单通道无效", "manifest-channel");
  }
  if (raw.board !== "aim61" || raw.usb_speed !== "fs") {
    throw new ProtocolError("当前网页仅接受 Ai-M61 Full-Speed 固件", "manifest-target");
  }
  if (
    typeof raw.version !== "string" ||
    !/^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/.test(raw.version)
  ) {
    throw new ProtocolError("清单固件版本无效", "manifest-version");
  }
  versionTriplet(raw.version);
  if (!Number.isSafeInteger(raw.size) || (raw.size as number) <= OTA_HEADER_SIZE) {
    throw new ProtocolError("清单固件大小无效", "manifest-size");
  }
  if (typeof raw.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(raw.sha256)) {
    throw new ProtocolError("清单 SHA-256 必须是 64 位小写十六进制", "manifest-hash");
  }
  if (!Number.isSafeInteger(raw.body_size) || (raw.body_size as number) <= 0) {
    throw new ProtocolError("清单 RAW body_size 无效", "manifest-body-size");
  }
  if (typeof raw.body_sha256 !== "string" || !/^[0-9a-f]{64}$/.test(raw.body_sha256)) {
    throw new ProtocolError("清单 RAW body_sha256 无效", "manifest-body-hash");
  }
  if ((raw.body_size as number) + OTA_HEADER_SIZE !== raw.size) {
    throw new ProtocolError("清单 size 必须等于 body_size + 512", "manifest-body-size");
  }
  if (typeof raw.url !== "string") throw new ProtocolError("清单缺少下载地址", "manifest-url");
  let resolved: URL;
  try {
    resolved = new URL(raw.url, baseUrl);
  } catch {
    throw new ProtocolError("清单下载地址无效", "manifest-url");
  }
  const loopback = resolved.protocol === "http:" &&
    (resolved.hostname === "localhost" || resolved.hostname === "127.0.0.1" || resolved.hostname === "[::1]");
  if (resolved.protocol !== "https:" && !loopback) {
    throw new ProtocolError("固件下载必须使用 HTTPS", "manifest-url");
  }
  if (resolved.username || resolved.password || resolved.hash) {
    throw new ProtocolError("固件下载地址不能包含凭据或片段", "manifest-url");
  }
  const resolvedUrl = resolved.toString();
  const signatureRequired = raw.channel !== "dev" || !options.allowUnsignedDev;
  const signature = parseSignature(raw.signature, signatureRequired);
  return {
    schema: 1,
    channel: raw.channel as OtaManifest["channel"],
    board: "aim61",
    usb_speed: "fs",
    version: raw.version,
    size: raw.size as number,
    sha256: raw.sha256,
    body_size: raw.body_size as number,
    body_sha256: raw.body_sha256,
    url: resolvedUrl,
    signature,
  };
}

function parseSignature(value: unknown, required: boolean): OtaSignature | null {
  if (value == null) {
    if (required) throw new ProtocolError("beta/stable 清单必须带 ECDSA P-256 签名", "manifest-signature");
    return null;
  }
  if (typeof value !== "object") throw new ProtocolError("清单签名格式无效", "manifest-signature");
  const raw = value as Record<string, unknown>;
  const signatureKeys = Object.keys(raw).sort();
  const expectedSignatureKeys = ["algorithm", "key_id", "scope", "value"];
  if (
    signatureKeys.length !== expectedSignatureKeys.length ||
    signatureKeys.some((key, index) => key !== expectedSignatureKeys[index])
  ) {
    throw new ProtocolError("清单签名字段集合无效", "manifest-signature");
  }
  if (
    raw.algorithm !== "ECDSA-P256-SHA256" ||
    raw.scope !== "DS5DONGLE-OTA-V1" ||
    typeof raw.key_id !== "string" ||
    !/^[A-Za-z0-9._-]{1,64}$/.test(raw.key_id) ||
    typeof raw.value !== "string"
  ) {
    throw new ProtocolError("清单 ECDSA P-256 签名字段无效", "manifest-signature");
  }
  let decoded: Uint8Array;
  try {
    decoded = base64Bytes(raw.value);
  } catch {
    throw new ProtocolError("清单签名不是有效的 Base64", "manifest-signature");
  }
  if (decoded.byteLength !== 64) {
    throw new ProtocolError("ECDSA raw r||s 签名必须为 64 字节", "manifest-signature");
  }
  validateP256RawSignature(decoded);
  return raw as unknown as OtaSignature;
}

export async function validateManifestImage(
  manifest: OtaManifest,
  source: BufferSourceLike,
): Promise<OtaImageInfo> {
  const bytes = asBytes(source);
  if (bytes.byteLength !== manifest.size) {
    throw new ProtocolError(`文件大小与清单不一致：${bytes.byteLength} / ${manifest.size}`, "manifest-size");
  }
  const info = await inspectOtaImage(bytes);
  if (info.fileSha256 !== manifest.sha256) {
    throw new ProtocolError("下载文件 SHA-256 与清单不一致", "manifest-hash");
  }
  if (info.bodyLength !== manifest.body_size || info.bodySha256 !== manifest.body_sha256) {
    throw new ProtocolError("OTA 固件体大小或 SHA-256 与清单不一致", "manifest-body-hash");
  }
  if (
    info.firmwareVersion !== manifest.version ||
    info.softwareVersion !== `EVENT_V${manifest.version}`
  ) {
    throw new ProtocolError(
      `OTA 头版本 ${info.firmwareVersion} 与清单 ${manifest.version} 不一致`,
      "manifest-version",
    );
  }
  return info;
}

export function otaAuthorizationCanonical(info: OtaImageInfo, version: string): Uint8Array {
  const marker = new TextEncoder().encode("DS5DONGLE-OTA-V1");
  if (marker.byteLength !== 16) throw new ProtocolError("OTA canonical marker 长度错误", "ota-canonical");
  const semver = versionTriplet(version);
  const payload = new Uint8Array(57);
  const view = new DataView(payload.buffer);
  payload.set(marker, 0);
  payload[16] = 1; // Ai-M61
  payload[17] = 0; // Full-Speed
  payload.set(semver, 18);
  view.setUint32(21, info.bodyLength, true);
  payload.set(hexBytes(info.bodySha256), 25);
  return payload;
}

export async function verifyManifestSignature(
  manifest: OtaManifest,
  info: OtaImageInfo,
  trustedKeyId: string | undefined,
  publicKeyBase64: string | undefined,
): Promise<void> {
  if (!manifest.signature) {
    if (manifest.channel !== "dev") {
      throw new ProtocolError("beta/stable 清单缺少发布签名", "manifest-signature");
    }
    return;
  }
  if (!trustedKeyId || !publicKeyBase64) {
    throw new ProtocolError("网页未配置生产 OTA 验签公钥，禁止升级", "manifest-trust-key");
  }
  if (manifest.signature.key_id !== trustedKeyId) {
    throw new ProtocolError("清单签名 key_id 不受信任", "manifest-trust-key");
  }
  let publicKey: Uint8Array;
  let signature: Uint8Array;
  try {
    publicKey = base64Bytes(publicKeyBase64);
    signature = base64Bytes(manifest.signature.value);
  } catch {
    throw new ProtocolError("OTA 验签密钥或签名 Base64 无效", "manifest-signature");
  }
  if (publicKey.byteLength !== 65 || publicKey[0] !== 0x04) {
    throw new ProtocolError("OTA P-256 公钥必须为 65 字节 SEC1 uncompressed raw key", "manifest-trust-key");
  }
  validateP256RawSignature(signature);
  let key: CryptoKey;
  try {
    key = await crypto.subtle.importKey(
      "raw",
      copyBuffer(publicKey),
      { name: "ECDSA", namedCurve: "P-256" },
      false,
      ["verify"],
    );
  } catch {
    throw new ProtocolError("当前浏览器无法加载 ECDSA P-256 验签公钥", "manifest-crypto");
  }
  const valid = await crypto.subtle.verify(
    { name: "ECDSA", hash: "SHA-256" },
    key,
    copyBuffer(signature),
    copyBuffer(otaAuthorizationCanonical(info, manifest.version)),
  );
  if (!valid) throw new ProtocolError("OTA 发布签名验证失败", "manifest-signature");
}

export function validateLocalOtaFilename(filename: string, info: OtaImageInfo): void {
  if (!filename.toLowerCase().endsWith(".bin.ota")) {
    throw new ProtocolError("请选择 RAW .bin.ota 文件", "ota-file-name");
  }
  const lower = filename.toLowerCase();
  if (!lower.includes("aim61") || !/(?:^|[-_.])fs(?:[-_.]|$)/.test(lower)) {
    throw new ProtocolError(
      "本地文件名必须明确包含 aim61 和 fs，避免把其他板型或 High-Speed 固件刷入设备",
      "ota-file-target",
    );
  }
  const version = lower.match(/[-_.]v(\d+(?:\.\d+){1,3})/i)?.[1];
  if (version && normalizeVersion(version) !== normalizeVersion(info.firmwareVersion)) {
    throw new ProtocolError("文件名版本与 OTA 头版本不一致", "ota-file-version");
  }
}

export function encodeOtaDataFrame(
  session: number,
  offset: number,
  data: Uint8Array,
): Uint8Array {
  return encodeOtaFrame(OTA_FRAME_DATA, session, offset, data);
}

export function decodeOtaDataFrame(source: BufferSourceLike): {
  session: number;
  offset: number;
  data: Uint8Array;
} {
  const bytes = asBytes(source);
  if (bytes.byteLength !== OTA_FRAME_SIZE || bytes[0] !== 0x4f || bytes[1] !== 0x54) {
    throw new ProtocolError("OTA 数据帧格式无效", "ota-frame-format");
  }
  if (bytes[2] !== OTA_PROTOCOL_VERSION || bytes[3] !== OTA_FRAME_DATA || bytes[12] > OTA_DATA_SIZE) {
    throw new ProtocolError("OTA 数据帧版本、类型或长度无效", "ota-frame-format");
  }
  if (bytes[12] === 0) {
    throw new ProtocolError("OTA DATA 帧不能为空", "ota-frame-size");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(59, true) !== crc32(bytes.subarray(0, 59))) {
    throw new ProtocolError("OTA 数据帧 CRC32 错误", "ota-frame-crc");
  }
  if (bytes.subarray(13 + bytes[12], 59).some((byte) => byte !== 0)) {
    throw new ProtocolError("OTA 数据帧 padding 必须为 0", "ota-frame-padding");
  }
  return {
    session: view.getUint32(4, true),
    offset: view.getUint32(8, true),
    data: bytes.slice(13, 13 + bytes[12]),
  };
}

export function encodeOtaFrame(
  opcode: number,
  session = 0,
  argument = 0,
  data: Uint8Array = new Uint8Array(),
): Uint8Array {
  if (data.byteLength > OTA_DATA_SIZE) {
    throw new ProtocolError("OTA 帧最多携带 46 字节", "ota-frame-size");
  }
  assertUint32(session, "session");
  assertUint32(argument, "argument");
  const report = new Uint8Array(FEATURE_PAYLOAD_SIZE);
  const view = new DataView(report.buffer);
  report[0] = 0x4f;
  report[1] = 0x54;
  report[2] = OTA_PROTOCOL_VERSION;
  report[3] = opcode;
  view.setUint32(4, session, true);
  view.setUint32(8, argument, true);
  report[12] = data.byteLength;
  report.set(data, 13);
  view.setUint32(59, crc32(report.subarray(0, 59)), true);
  return report;
}

export function encodeOtaBegin(
  session: number,
  totalFileSize: number,
  info: OtaImageInfo,
  version: string,
  signed: boolean,
): Uint8Array {
  const semver = versionTriplet(version);
  const data = new Uint8Array(42);
  const view = new DataView(data.buffer);
  data[0] = 1; // Ai-M61
  data[1] = 0; // USB Full-Speed
  data.set(semver, 2);
  data[5] = signed ? 1 : 0;
  view.setUint32(6, info.bodyLength, true);
  data.set(hexBytes(info.bodySha256), 10);
  return encodeOtaFrame(OtaOpcode.Begin, session, totalFileSize, data);
}

export function encodeOtaAuthFrames(session: number, signature: OtaSignature): Uint8Array[] {
  const bytes = base64Bytes(signature.value);
  if (bytes.byteLength !== 64) {
    throw new ProtocolError("OTA AUTH 签名必须为 64 字节 raw r||s", "ota-auth-size");
  }
  return [
    encodeOtaFrame(OtaOpcode.Auth, session, 0, bytes.subarray(0, 46)),
    encodeOtaFrame(OtaOpcode.Auth, session, 46, bytes.subarray(46)),
  ];
}

export function decodeOtaStatus(source: BufferSourceLike): OtaStatus {
  let bytes = asBytes(source);
  if (bytes[0] === REPORT_OTA_CONTROL && bytes[1] === 0x4f) bytes = bytes.subarray(1);
  if (bytes.byteLength !== FEATURE_PAYLOAD_SIZE || bytes[0] !== 0x4f || bytes[1] !== 0x54) {
    throw new ProtocolError("设备没有返回有效的 OTA 状态", "ota-status-format");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes[2] !== OTA_PROTOCOL_VERSION) {
    throw new ProtocolError(`设备 OTA 协议版本 ${bytes[2]} 不受支持`, "ota-status-version");
  }
  if (bytes[3] !== OtaOpcode.Ack && bytes[3] !== OtaOpcode.Error) {
    throw new ProtocolError("设备返回的不是 OTA ACK/ERROR", "ota-status-opcode");
  }
  if (view.getUint32(59, true) !== crc32(bytes.subarray(0, 59))) {
    throw new ProtocolError("OTA 状态 CRC32 错误", "ota-status-crc");
  }
  if (bytes[12] !== 44) {
    throw new ProtocolError("OTA ACK/ERROR 状态长度无效", "ota-status-length");
  }
  if (bytes[57] !== 0 || bytes[58] !== 0) {
    throw new ProtocolError("OTA 状态 padding 必须为 0", "ota-status-padding");
  }
  return {
    protocolVersion: bytes[2],
    state: bytes[13] as OtaStateValue,
    session: view.getUint32(4, true),
    acceptedOffset: view.getUint32(8, true),
    error: bytes[14],
    lastRequest: bytes[15],
    flags: bytes[16],
    committedOffset: view.getUint32(17, true),
    totalSize: view.getUint32(21, true),
    maxImageSize: view.getUint32(25, true),
    capabilities: view.getUint32(29, true),
    board: bytes[33],
    usbSpeed: bytes[34],
    format: bytes[35],
    maxData: bytes[36],
    windowFrames: Math.max(1, bytes[37] || 1),
    activeSlot: bytes[38],
    trialRetryFlag: bytes[39],
    rebootDelayMs: bytes[40] * 100,
    firmwareVersion: ascii(bytes.subarray(41, 57)),
  };
}

export function crc32(source: BufferSourceLike): number {
  const bytes = asBytes(source);
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

export async function sha256Hex(source: BufferSourceLike): Promise<string> {
  const bytes = asBytes(source);
  const digest = await globalThis.crypto.subtle.digest("SHA-256", copyBuffer(bytes));
  return hex(new Uint8Array(digest));
}

export function normalizeVersion(version: string): string {
  const match = version.trim().match(/^(?:EVENT_)?V?((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))$/);
  if (!match) throw new ProtocolError("固件版本不是规范 X.Y.Z triplet", "ota-version");
  versionTriplet(match[1]);
  return match[1];
}

export function versionTriplet(version: string): Uint8Array {
  const normalized = version.trim().replace(/^EVENT_V/, "").replace(/^v/, "");
  const match = normalized.match(/^((?:0|[1-9]\d*))\.((?:0|[1-9]\d*))\.((?:0|[1-9]\d*))$/);
  if (!match) throw new ProtocolError("固件版本必须为 X.Y.Z", "ota-version");
  if (normalized.length > 8) {
    throw new ProtocolError("固件版本无法放入 BL60X EVENT_V 头（X.Y.Z 最多 8 字节）", "ota-version");
  }
  const values = match.slice(1).map(Number);
  if (values.some((value) => !Number.isInteger(value) || value < 0 || value >= 255)) {
    throw new ProtocolError("固件版本的每一段必须在 0..254", "ota-version");
  }
  return Uint8Array.from(values);
}

export function compareVersions(left: string, right: string): number {
  const a = normalizeVersion(left).split(".").map(Number);
  const b = normalizeVersion(right).split(".").map(Number);
  for (let index = 0; index < Math.max(a.length, b.length); index++) {
    const delta = (a[index] || 0) - (b[index] || 0);
    if (delta !== 0) return Math.sign(delta);
  }
  return 0;
}

export function validateOtaWindowAck(
  previousAccepted: number,
  windowEnd: number,
  nextAccepted: number,
): "pending" | "complete" {
  assertUint32(previousAccepted, "previousAccepted");
  assertUint32(windowEnd, "windowEnd");
  assertUint32(nextAccepted, "nextAccepted");
  if (previousAccepted > windowEnd || nextAccepted < previousAccepted) {
    throw new ProtocolError("设备 OTA ACK offset 回退", "ota-ack-regression");
  }
  if (nextAccepted > windowEnd) {
    throw new ProtocolError("设备 OTA ACK 超过当前发送窗口", "ota-ack-window");
  }
  return nextAccepted === windowEnd ? "complete" : "pending";
}

export class ProtocolError extends Error {
  readonly code: string;

  constructor(message: string, code: string) {
    super(message);
    this.name = "ProtocolError";
    this.code = code;
  }
}

export type BufferSourceLike = ArrayBuffer | ArrayBufferView | Uint8Array;

function asBytes(source: BufferSourceLike): Uint8Array {
  if (source instanceof Uint8Array) return source;
  if (source instanceof ArrayBuffer) return new Uint8Array(source);
  return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
}

function ascii(bytes: Uint8Array, trim = true): string {
  let end = bytes.byteLength;
  if (trim) {
    while (end > 0 && (bytes[end - 1] === 0 || bytes[end - 1] === 0xff || bytes[end - 1] === 0x20)) end--;
  }
  const value = new TextDecoder("ascii", { fatal: true }).decode(bytes.subarray(0, end));
  if (!/^[\x20-\x7e]*$/.test(value)) {
    throw new ProtocolError("OTA 头包含非法文本字节", "ota-header-text");
  }
  return value;
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function hexBytes(value: string): Uint8Array {
  const bytes = new Uint8Array(value.length / 2);
  for (let index = 0; index < bytes.length; index++) {
    bytes[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
  }
  return bytes;
}

function base64Bytes(value: string): Uint8Array {
  if (typeof atob === "function") {
    return Uint8Array.from(atob(value), (char) => char.charCodeAt(0));
  }
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const clean = value.replace(/=+$/, "");
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(value) || clean.length % 4 === 1) throw new Error("base64");
  const output: number[] = [];
  let bits = 0;
  let count = 0;
  for (const char of clean) {
    bits = (bits << 6) | alphabet.indexOf(char);
    count += 6;
    if (count >= 8) {
      count -= 8;
      output.push((bits >>> count) & 0xff);
    }
  }
  return Uint8Array.from(output);
}

function validateP256RawSignature(signature: Uint8Array): void {
  if (signature.byteLength !== 64) {
    throw new ProtocolError("ECDSA P-256 签名必须为 64 字节 raw r||s", "manifest-signature");
  }
  const order = BigInt("0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551");
  const halfOrder = order >> 1n;
  const r = bigEndianInteger(signature.subarray(0, 32));
  const s = bigEndianInteger(signature.subarray(32, 64));
  if (r < 1n || r >= order || s < 1n || s >= order) {
    throw new ProtocolError("ECDSA P-256 r/s 超出曲线阶范围", "manifest-signature-range");
  }
  if (s > halfOrder) {
    throw new ProtocolError("ECDSA P-256 签名不是 low-S 规范形式", "manifest-signature-high-s");
  }
}

function bigEndianInteger(bytes: Uint8Array): bigint {
  let value = 0n;
  for (const byte of bytes) value = (value << 8n) | BigInt(byte);
  return value;
}

function copyBuffer(bytes: Uint8Array): ArrayBuffer {
  return Uint8Array.from(bytes).buffer;
}

function assertUint32(value: number, name: string): void {
  if (!Number.isSafeInteger(value) || value < 0 || value > 0xffffffff) {
    throw new ProtocolError(`${name} 必须是 uint32`, "uint32");
  }
}
