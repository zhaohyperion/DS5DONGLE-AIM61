import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  CONFIG_BODY_SIZE,
  DEFAULT_CONFIG,
  OtaOpcode,
  ProtocolError,
  crc32,
  decodeConfig,
  decodeOtaDataFrame,
  decodeOtaStatus,
  encodeConfig,
  encodeOtaAuthFrames,
  encodeOtaBegin,
  encodeOtaDataFrame,
  inspectOtaImage,
  otaAuthorizationCanonical,
  parseManifest,
  sha256Hex,
  validateManifestImage,
  validateOtaWindowAck,
  verifyManifestSignature,
  versionTriplet,
  type OtaImageInfo,
  type OtaManifest,
  type OtaSignature,
} from "../app/lib/protocol.ts";

const BEGIN_VECTOR =
  "4f54010178563412f02b0d002a" +
  "010001020301f0290d00" +
  "b0d51c58c8b9c1f458fadf16c7d375630ef51da4df81915893b05c0fa4ed8bc6" +
  "00000000f9c2342b";
const DATA_VECTOR =
  "4f540110785634122e00000019" +
  "445335446f6e676c65204f5441207465737420766563746f72" +
  "000000000000000000000000000000000000000000" +
  "84c23988";
const AUTH0_VECTOR =
  "4f54010278563412000000002e" +
  "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f" +
  "202122232425262728292a2b2c2d" +
  "ea3513bc";
const AUTH46_VECTOR =
  "4f540102785634122e00000012" +
  "2e2f303132333435363738393a3b3c3d3e3f" +
  "00000000000000000000000000000000000000000000000000000000" +
  "ae93fa7f";
const ACK_VECTOR =
  "4f54018078563412401000002c" +
  "0300020100100000f02b0d0000821600fb0300000100012e1000f20f" +
  "424c3631382d44533520332e35000000" +
  "0000f75742f8";
const ERROR_VECTOR =
  "4f5401ff78563412001000002c" +
  "0611030100100000f02b0d0000821600fb0300000100012e1000f20f" +
  "424c3631382d44533520332e35000000" +
  "0000ef54ddf6";

const BODY_HASH = "b0d51c58c8b9c1f458fadf16c7d375630ef51da4df81915893b05c0fa4ed8bc6";
const CURVE_ORDER = BigInt("0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551");

test("CRC-32/ISO-HDLC and DATA frame match the Python vectors", () => {
  assert.equal(crc32(new TextEncoder().encode("123456789")), 0xcbf43926);
  const data = new TextEncoder().encode("DS5Dongle OTA test vector");
  const frame = encodeOtaDataFrame(0x12345678, 0x2e, data);
  assert.equal(hex(frame), DATA_VECTOR);
  assert.deepEqual(decodeOtaDataFrame(frame).data, data);

  const corrupted = frame.slice();
  corrupted[13] ^= 0x80;
  assert.throws(() => decodeOtaDataFrame(corrupted), /CRC32/);

  const badPadding = frame.slice();
  badPadding[50] = 1;
  new DataView(badPadding.buffer).setUint32(59, crc32(badPadding.subarray(0, 59)), true);
  assert.throws(() => decodeOtaDataFrame(badPadding), /padding/);
});

test("BEGIN and AUTH frames match the shared firmware/Python vectors", () => {
  const info = imageInfo({ bodyLength: 0x000d29f0, bodySha256: BODY_HASH });
  const begin = encodeOtaBegin(0x12345678, 0x000d2bf0, info, "1.2.3", true);
  assert.equal(hex(begin), BEGIN_VECTOR);

  const signature: OtaSignature = {
    algorithm: "ECDSA-P256-SHA256",
    key_id: "test",
    scope: "DS5DONGLE-OTA-V1",
    value: Buffer.from(Uint8Array.from({ length: 64 }, (_, index) => index)).toString("base64"),
  };
  const [first, second] = encodeOtaAuthFrames(0x12345678, signature);
  assert.equal(hex(first), AUTH0_VECTOR);
  assert.equal(hex(second), AUTH46_VECTOR);
});

test("ACK and ERROR status snapshots match the shared vectors", () => {
  const ack = decodeOtaStatus(bytes(ACK_VECTOR));
  assert.equal(ack.session, 0x12345678);
  assert.equal(ack.acceptedOffset, 0x1040);
  assert.equal(ack.committedOffset, 0x1000);
  assert.equal(ack.lastRequest, OtaOpcode.Auth);
  assert.equal(ack.capabilities, 0x3fb);
  assert.equal(ack.maxData, 46);
  assert.equal(ack.windowFrames, 16);
  assert.equal(ack.firmwareVersion, "BL618-DS5 3.5");

  const error = decodeOtaStatus(bytes(ERROR_VECTOR));
  assert.equal(error.acceptedOffset, 0x1000);
  assert.equal(error.error, 17);
  assert.equal(error.state, 6);
});

test("57-byte P-256 canonical matches the cross-language vector", () => {
  const info = imageInfo({ bodyLength: 0x01020304, bodySha256: "00".repeat(31) + "ff" });
  assert.equal(
    hex(otaAuthorizationCanonical(info, "1.2.3")),
    "445335444f4e474c452d4f54412d5631010001020304030201" + "00".repeat(31) + "ff",
  );
});

test("window ACK waits for the exact window end and rejects regression/overshoot", () => {
  assert.equal(validateOtaWindowAck(100, 200, 150), "pending");
  assert.equal(validateOtaWindowAck(150, 200, 200), "complete");
  assert.throws(() => validateOtaWindowAck(150, 200, 149), /回退/);
  assert.throws(() => validateOtaWindowAck(150, 200, 201), /超过/);
});

test("transport polling reads published snapshots without enqueueing STATUS commands", () => {
  const source = readFileSync(new URL("../app/lib/hid.ts", import.meta.url), "utf8");
  const method = source.match(/async readOtaStatus\(\): Promise<OtaStatus> \{([\s\S]*?)\n[ ]{2}\}/)?.[1] ?? "";
  assert.match(method, /receiveFeatureReport/);
  assert.doesNotMatch(method, /sendFeatureReport/);
  assert.match(source, /CONTROL_TIMEOUT_MS = 45000/);
  assert.match(source, /capabilities & 0x3fb/);
  assert.doesNotMatch(source, /duplicate|idempotent/i);
});

test("manifest rejects extra fields, wrong target, insecure URL, and unsigned release channels", () => {
  const valid = manifestFixture();
  assert.equal(parseManifest(valid).board, "aim61");
  assert.throws(() => parseManifest({ ...valid, extra: true }), /字段集合/);
  assert.throws(() => parseManifest({ ...valid, board: "m62" }), /Ai-M61/);
  assert.throws(() => parseManifest({ ...valid, usb_speed: "hs" }), /Ai-M61/);
  assert.throws(() => parseManifest({ ...valid, url: "http://example.com/fw.bin.ota" }), /HTTPS/);
  assert.throws(() => parseManifest({ ...valid, url: "http://localhost.evil.example/fw.bin.ota" }), /HTTPS/);
  assert.match(parseManifest({ ...valid, url: "http://localhost/fw.bin.ota" }).url, /^http:\/\/localhost\//);
  assert.throws(() => parseManifest({ ...valid, url: "https://user@example.test/fw.bin.ota" }), /凭据/);
  assert.throws(() => parseManifest({ ...valid, url: "https://example.test/fw.bin.ota#hash" }), /片段/);
  assert.throws(() => parseManifest({ ...valid, channel: "beta", signature: null }), /签名/);
  assert.throws(() => parseManifest({ ...valid, channel: "dev", signature: null }), /签名/);
  assert.equal(
    parseManifest({ ...valid, channel: "dev", signature: null }, undefined, { allowUnsignedDev: true }).signature,
    null,
  );
});

test("signature verifier independently rejects unsigned beta/stable objects", async () => {
  const dev = parseManifest(
    { ...manifestFixture(), channel: "dev", signature: null },
    undefined,
    { allowUnsignedDev: true },
  );
  const info = imageInfo({ bodyLength: dev.body_size, bodySha256: dev.body_sha256 });
  await assert.doesNotReject(() => verifyManifestSignature(dev, info, undefined, undefined));
  await assert.rejects(
    () => verifyManifestSignature({ ...dev, channel: "beta" }, info, undefined, undefined),
    /beta\/stable/,
  );
  await assert.rejects(
    () => verifyManifestSignature({ ...dev, channel: "stable" }, info, undefined, undefined),
    /beta\/stable/,
  );
});

test("semver respects uint8 and the BL60X EVENT_V header limit", () => {
  assert.deepEqual(versionTriplet("254.5.0"), Uint8Array.of(254, 5, 0));
  for (const invalid of ["255.0.0", "0.255.0", "0.0.255", "254.254.254", "01.2.3", "1.2", "1.2.3.4"]) {
    assert.throws(() => versionTriplet(invalid), ProtocolError, invalid);
  }
  assert.throws(() => parseManifest({ ...manifestFixture(), version: "v1.2.3" }), /版本/);
});

test("manifest rejects out-of-range and high-S P-256 signatures before WebCrypto", () => {
  const zeroR = rawSignature(0n, 1n);
  const highS = rawSignature(1n, (CURVE_ORDER >> 1n) + 1n);
  const outOfRangeR = rawSignature(CURVE_ORDER, 1n);
  for (const value of [zeroR, highS, outOfRangeR]) {
    assert.throws(
      () => parseManifest({ ...manifestFixture(), signature: { ...signatureFixture(), value } }),
      /r\/s|low-S|曲线阶/,
    );
  }
});

test("configuration is exactly 25 bytes and audio buffer is bounded to 16..127", () => {
  for (const value of [16, 127]) {
    const config = { ...DEFAULT_CONFIG, audioBufferLength: value };
    const encoded = encodeConfig(config);
    assert.equal(encoded.byteLength, CONFIG_BODY_SIZE);
    assert.equal(decodeConfig(encoded).audioBufferLength, value);
  }
  for (const value of [15, 128]) {
    assert.throws(() => encodeConfig({ ...DEFAULT_CONFIG, audioBufferLength: value }), /音频缓冲/);
  }
});

test("RAW OTA header, body, and manifest size/hash/version form one strict chain", async () => {
  const body = new TextEncoder().encode("M61 OTA body fixture");
  const bodyHash = await sha256Hex(body);
  const image = new Uint8Array(512 + body.length);
  image.set(new TextEncoder().encode("BL60X_OTA_Ver1.0"), 0);
  image.set(new TextEncoder().encode("RAW "), 16);
  new DataView(image.buffer).setUint32(20, body.length, true);
  image.set(new TextEncoder().encode("BFL_Module_v1.1"), 32);
  image.set(new TextEncoder().encode("EVENT_V1.2.3"), 48);
  image.set(bytes(bodyHash), 64);
  image.fill(0xff, 96, 512);
  image.set(body, 512);
  await inspectOtaImage(image);
  const manifest = {
    ...manifestFixture(),
    size: image.length,
    sha256: await sha256Hex(image),
    body_size: body.length,
    body_sha256: bodyHash,
  } as OtaManifest;
  assert.equal((await validateManifestImage(manifest, image)).firmwareVersion, "1.2.3");

  image[48] = "X".charCodeAt(0);
  await assert.rejects(() => inspectOtaImage(image), /EVENT_VX\.Y\.Z/);
});

function manifestFixture(): Record<string, unknown> {
  return {
    schema: 1,
    channel: "stable",
    board: "aim61",
    usb_speed: "fs",
    version: "1.2.3",
    size: 513,
    sha256: "11".repeat(32),
    body_size: 1,
    body_sha256: "22".repeat(32),
    url: "https://example.test/DS5Dongle-aim61-fs-v1.2.3.bin.ota",
    signature: signatureFixture(),
  };
}

function signatureFixture(): OtaSignature {
  return {
    algorithm: "ECDSA-P256-SHA256",
    key_id: "release-2026",
    scope: "DS5DONGLE-OTA-V1",
    value: rawSignature(1n, 1n),
  };
}

function rawSignature(r: bigint, s: bigint): string {
  const raw = new Uint8Array(64);
  writeBigEndian(raw.subarray(0, 32), r);
  writeBigEndian(raw.subarray(32), s);
  return Buffer.from(raw).toString("base64");
}

function writeBigEndian(target: Uint8Array, value: bigint): void {
  for (let index = target.length - 1; index >= 0; index--) {
    target[index] = Number(value & 0xffn);
    value >>= 8n;
  }
}

function imageInfo(overrides: Partial<OtaImageInfo>): OtaImageInfo {
  return {
    marker: "BL60X_OTA_Ver1.0",
    type: "RAW",
    bodyLength: 1,
    hardwareVersion: "BFL_Module_v1.1",
    softwareVersion: "EVENT_V1.2.3",
    bodySha256: "22".repeat(32),
    fileSize: 513,
    fileSha256: "11".repeat(32),
    firmwareVersion: "1.2.3",
    ...overrides,
  };
}

function bytes(value: string): Uint8Array {
  return Uint8Array.from(value.match(/../g) ?? [], (pair) => Number.parseInt(pair, 16));
}

function hex(value: Uint8Array): string {
  return Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
