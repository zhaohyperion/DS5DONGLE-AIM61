import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

test("device studio integrates legacy configuration without replacing secure OTA", async () => {
  const [consoleSource, hidSource, panelsSource, stylesSource] = await Promise.all([
    readFile(new URL("app/DeviceConsole.tsx", root), "utf8"),
    readFile(new URL("app/lib/hid.ts", root), "utf8"),
    readFile(new URL("app/LegacyDevicePanels.tsx", root), "utf8"),
    readFile(new URL("app/globals.css", root), "utf8"),
  ]);

  assert.match(consoleSource, /FullConfigEditor/);
  assert.match(consoleSource, /ButtonRemapEditor/);
  assert.match(consoleSource, /TelemetryStrip/);
  assert.match(consoleSource, /普通 \/ 全速/);
  assert.match(consoleSource, /高速/);
  assert.match(consoleSource, /selectOtaProfile\("fs"\)/);
  assert.match(consoleSource, /selectOtaProfile\("hs"\)/);
  assert.match(consoleSource, /DS5Dongle-aim61-hs-stable\.ota\.json/);
  assert.match(consoleSource, /zhaohyperion\/DS5DONGLE-AIM61/);
  assert.doesNotMatch(consoleSource, /sqlCRT\/ds5dongle-bl618-opensource\/releases\/latest/);
  assert.match(hidSource, /new LegacyHidClient\(this\.device\)/);
  assert.match(hidSource, /transferOta/);
  assert.match(hidSource, /capability\.usbSpeed !== targetSpeedId/);
  assert.match(hidSource, /REPORT_OTA_CONTROL/);
  assert.match(panelsSource, /Keyboard HID/);
  assert.match(panelsSource, /REMAP_FLAG_EXTRA_KEY/);
  assert.match(panelsSource, /REMAP_FLAG_SUPPRESS/);
  assert.match(panelsSource, /Square: \{ glyph: "□"/);
  assert.match(panelsSource, /Cross: \{ glyph: "×"/);
  assert.match(panelsSource, /Circle: \{ glyph: "○"/);
  assert.match(panelsSource, /Triangle: \{ glyph: "△"/);
  assert.match(panelsSource, /ControllerButtonLabel/);
  assert.match(stylesSource, /\.controller-button-icon\.is-square/);
  assert.match(stylesSource, /\.visually-hidden/);
});

test("help center ships FAQ, changelog, and PWA controls", async () => {
  const [consoleSource, referenceSource, registrarSource] = await Promise.all([
    readFile(new URL("app/DeviceConsole.tsx", root), "utf8"),
    readFile(new URL("app/ReferenceCenter.tsx", root), "utf8"),
    readFile(new URL("app/PwaRegistrar.tsx", root), "utf8"),
  ]);

  assert.match(consoleSource, /ReferenceCenter/);
  assert.match(consoleSource, /PwaRegistrar/);
  assert.match(referenceSource, /Web 0\.3\.0/);
  assert.match(referenceSource, /Web 0\.3\.1/);
  assert.match(referenceSource, /Web 0\.3\.2/);
  assert.match(referenceSource, /Firmware 3\.5\.2/);
  assert.match(referenceSource, /HID maintenance lock/);
  assert.match(referenceSource, /常见问题/);
  assert.match(referenceSource, /项目更新日志/);
  assert.match(registrarSource, /serviceWorker/);
  assert.match(registrarSource, /REFRESH_CACHES/);
});
