import {
  OtaOpcode,
  OtaState,
  REPORT_OTA_CONTROL,
  REPORT_OTA_DATA,
  SONY_VENDOR_ID,
  SUPPORTED_PRODUCT_IDS,
  decodeOtaStatus,
  encodeOtaAuthFrames,
  encodeOtaBegin,
  encodeOtaDataFrame,
  encodeOtaFrame,
  normalizeVersion,
  validateOtaWindowAck,
  type DeviceConfig,
  type DeviceStatus,
  type OtaImageInfo,
  type OtaSignature,
  type OtaStatus,
  type RemapEntry,
} from "./protocol";
import { LegacyHidClient } from "./legacy-hid";

const GAMEPAD_USAGE_PAGE = 0x01;
const GAMEPAD_USAGE = 0x05;
const WINDOW_ACK_TIMEOUT_MS = 10000;
const CONTROL_TIMEOUT_MS = 45000;

export class Ds5DongleClient {
  constructor(public readonly device: HIDDevice) {}

  static supported(device: HIDDevice): boolean {
    return (
      device.vendorId === SONY_VENDOR_ID &&
      SUPPORTED_PRODUCT_IDS.includes(device.productId as (typeof SUPPORTED_PRODUCT_IDS)[number]) &&
      device.collections.some(
        (collection) => collection.usagePage === GAMEPAD_USAGE_PAGE && collection.usage === GAMEPAD_USAGE,
      )
    );
  }

  static async choose(): Promise<Ds5DongleClient> {
    const hid = requireWebHid();
    const devices = await hid.requestDevice({
      filters: SUPPORTED_PRODUCT_IDS.map((productId) => ({
        vendorId: SONY_VENDOR_ID,
        productId,
        usagePage: GAMEPAD_USAGE_PAGE,
        usage: GAMEPAD_USAGE,
      })),
    });
    const device = devices.find(Ds5DongleClient.supported);
    if (!device) throw new Error("没有选择兼容的 DS5Dongle");
    const client = new Ds5DongleClient(device);
    await client.open();
    return client;
  }

  static async authorized(): Promise<Ds5DongleClient | null> {
    if (!navigator.hid) return null;
    const device = (await navigator.hid.getDevices()).find(Ds5DongleClient.supported);
    if (!device) return null;
    const client = new Ds5DongleClient(device);
    await client.open();
    return client;
  }

  async open(): Promise<void> {
    if (!this.device.opened) await this.device.open();
  }

  async close(): Promise<void> {
    if (this.device.opened) await this.device.close();
  }

  async readConfig(): Promise<DeviceConfig> {
    return new LegacyHidClient(this.device).readConfig();
  }

  async readFirmwareVersion(): Promise<string> {
    return new LegacyHidClient(this.device).readFirmwareVersion();
  }

  async readStatus(): Promise<DeviceStatus> {
    const status = await new LegacyHidClient(this.device).readTelemetry();
    return {
      rssi: status.rssiDbm,
      audioStatusValid: status.audioValid,
      speakerActive: status.audioValid && status.speakerActive,
      microphoneActive: status.audioValid && status.microphoneActive,
      batteryLevel: status.batteryPercent,
      batteryState: status.batteryState & 0x0f,
    };
  }

  async readRemap(): Promise<RemapEntry[]> {
    return new LegacyHidClient(this.device).readRemap();
  }

  async applyConfig(config: DeviceConfig): Promise<void> {
    await new LegacyHidClient(this.device).applyConfig(config);
  }

  async saveConfig(): Promise<void> {
    await new LegacyHidClient(this.device).saveConfig();
  }

  async reconnectUsb(): Promise<void> {
    await new LegacyHidClient(this.device).reconnectUsb();
  }

  async saveRemap(entries: RemapEntry[]): Promise<void> {
    await new LegacyHidClient(this.device).writeRemap(entries);
  }

  async resetRemap(): Promise<void> {
    await new LegacyHidClient(this.device).resetRemap();
  }

  async otaCapability(): Promise<OtaStatus> {
    await this.open();
    const status = await this.readOtaStatus();
    if (
      status.board !== 1 ||
      status.usbSpeed !== 0 ||
      status.format !== 1 ||
      status.maxData !== 46 ||
      (status.capabilities & 0x3fb) !== 0x3fb
    ) {
      throw new Error("设备 OTA 能力、目标或生产验签密钥不符合升级要求");
    }
    return status;
  }

  async transferOta(
    bytes: Uint8Array,
    info: OtaImageInfo,
    version: string,
    signature: OtaSignature | null,
    options: {
      signal: AbortSignal;
      onProgress: (accepted: number, total: number, status: OtaStatus) => void;
    },
  ): Promise<OtaStatus> {
    const capability = await this.otaCapability();
    if (bytes.byteLength > capability.maxImageSize) {
      throw new Error(`固件 ${bytes.byteLength} B 超过设备 OTA 上限 ${capability.maxImageSize} B`);
    }
    const signatureRequired = Boolean(capability.capabilities & (1 << 8));
    if (signatureRequired && !signature) {
      throw new Error("设备要求生产签名；本地镜像需要匹配的签名清单");
    }

    const session = randomSession();
    try {
      await this.device.sendFeatureReport(
        REPORT_OTA_CONTROL,
        encodeOtaBegin(session, bytes.byteLength, info, version, Boolean(signature)),
      );

      let status = await this.waitForStatus(
        (next) =>
          next.session === session &&
          (next.state === OtaState.Authorizing ||
            next.state === OtaState.Preparing ||
            next.state === OtaState.Receiving),
        5000,
      );
      throwForOtaError(status);
      if (signature) {
        for (const frame of encodeOtaAuthFrames(session, signature)) {
          await this.device.sendFeatureReport(REPORT_OTA_CONTROL, frame);
        }
        status = await this.waitForStatus(
          (next) => next.session === session && next.state === OtaState.Receiving,
          CONTROL_TIMEOUT_MS,
        );
        throwForOtaError(status);
      } else if (status.state !== OtaState.Receiving) {
        status = await this.waitForStatus(
          (next) => next.session === session && next.state === OtaState.Receiving,
          CONTROL_TIMEOUT_MS,
        );
        throwForOtaError(status);
      }
      let accepted = status.acceptedOffset;
      if (accepted !== 0) throw new Error("新 OTA 会话返回了非零起始 offset，已拒绝跨会话续传");
      let windowFrames = Math.max(1, Math.min(status.windowFrames, 16));
      options.onProgress(accepted, bytes.byteLength, status);

      while (accepted < bytes.byteLength) {
        if (options.signal.aborted) throw abortError();
        let cursor: number = accepted;
        for (let frameIndex = 0; frameIndex < windowFrames && cursor < bytes.byteLength; frameIndex++) {
          const end = Math.min(cursor + 46, bytes.byteLength);
          await this.device.sendReport(
            REPORT_OTA_DATA,
            encodeOtaDataFrame(session, cursor, bytes.subarray(cursor, end)),
          );
          cursor = end;
        }
        const windowEnd: number = cursor;
        const deadline = performance.now() + WINDOW_ACK_TIMEOUT_MS;
        while (accepted < windowEnd && performance.now() < deadline) {
          if (options.signal.aborted) throw abortError();
          status = await this.readOtaStatus();
          throwForOtaError(status);
          if (status.session !== session) throw new Error("设备 OTA 会话意外变化");
          windowFrames = Math.max(1, Math.min(status.windowFrames, 16));
          const ackState = validateOtaWindowAck(accepted, windowEnd, status.acceptedOffset);
          if (status.acceptedOffset !== accepted) {
            accepted = status.acceptedOffset;
            options.onProgress(accepted, bytes.byteLength, status);
          }
          if (ackState === "complete") break;
          await delay(50, options.signal);
        }
        if (accepted !== windowEnd) {
          throw new Error(`OTA 窗口确认超时（${accepted} / ${windowEnd}）`);
        }
      }

      await this.device.sendFeatureReport(
        REPORT_OTA_CONTROL,
        encodeOtaFrame(OtaOpcode.Commit, session),
      );
      status = await this.waitForStatus(
        (next) => next.session === session && next.state === OtaState.ReadyToReboot,
        CONTROL_TIMEOUT_MS,
      );
      throwForOtaError(status);
      options.onProgress(bytes.byteLength, bytes.byteLength, status);
      return status;
    } catch (error) {
      await this.device
        .sendFeatureReport(REPORT_OTA_CONTROL, encodeOtaFrame(OtaOpcode.Abort, session))
        .catch(() => undefined);
      throw error;
    }
  }

  async readOtaStatus(): Promise<OtaStatus> {
    return decodeOtaStatus(await this.device.receiveFeatureReport(REPORT_OTA_CONTROL));
  }

  private async waitForStatus(
    predicate: (status: OtaStatus) => boolean,
    timeoutMs: number,
  ): Promise<OtaStatus> {
    const deadline = performance.now() + timeoutMs;
    let last: OtaStatus | null = null;
    while (performance.now() < deadline) {
      last = await this.readOtaStatus();
      throwForOtaError(last);
      if (predicate(last)) return last;
      await delay(40);
    }
    throw new Error(last ? `等待设备 OTA 状态超时（state=${last.state}, error=${last.error}）` : "读取 OTA 状态超时");
  }
}

export function webHidAvailable(): boolean {
  return typeof navigator !== "undefined" && Boolean(navigator.hid) && window.isSecureContext;
}

export function deviceLabel(device: HIDDevice): string {
  return `${device.productName || "DS5Dongle"} · 054C:${device.productId.toString(16).padStart(4, "0").toUpperCase()}`;
}

export async function waitForReconnectedVersion(expected: string, timeoutMs = 25000): Promise<string> {
  const hid = requireWebHid();
  const deadline = performance.now() + timeoutMs;
  while (performance.now() < deadline) {
    const device = (await hid.getDevices()).find(Ds5DongleClient.supported);
    if (device) {
      try {
        const client = new Ds5DongleClient(device);
        await client.open();
        const version = await client.readFirmwareVersion();
        if (normalizeVersion(version) === normalizeVersion(expected)) return version;
      } catch {
        // USB re-enumeration can expose the device before feature reports are ready.
      }
    }
    await delay(350);
  }
  throw new Error("设备已重启，但未能在时限内确认新固件版本");
}

function requireWebHid(): HID {
  if (!navigator.hid || !window.isSecureContext) {
    throw new Error("WebHID 仅可在 Chrome/Edge 的 HTTPS 或 localhost 页面使用");
  }
  return navigator.hid;
}

function throwForOtaError(status: OtaStatus): void {
  if (status.error || status.state === OtaState.Error) {
    throw new Error(`设备拒绝 OTA：${otaErrorName(status.error)}（错误码 ${status.error}）`);
  }
  if (status.state === OtaState.Aborted) throw new Error("设备已中止 OTA 会话");
}

function otaErrorName(code: number): string {
  return [
    "OK", "BAD_MAGIC", "BAD_VERSION", "BAD_OPCODE", "BAD_STATE", "BAD_SESSION",
    "BAD_OFFSET", "BAD_LENGTH", "BAD_CRC", "BAD_TARGET", "TOO_LARGE", "QUEUE_FULL",
    "FLASH", "HASH", "HEADER", "INTERNAL", "AUTH_REQUIRED", "AUTH_FAILED",
    "KEY_MISSING", "TIMEOUT",
  ][code] || `UNKNOWN_${code}`;
}

function randomSession(): number {
  const value = new Uint32Array(1);
  crypto.getRandomValues(value);
  return value[0] || 1;
}

function delay(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(abortError());
    const timer = window.setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        window.clearTimeout(timer);
        reject(abortError());
      },
      { once: true },
    );
  });
}

function abortError(): DOMException {
  return new DOMException("用户取消升级", "AbortError");
}
