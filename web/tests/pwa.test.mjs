import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";

const root = new URL("../", import.meta.url);

test("web app manifest is installable and scoped to this site", async () => {
  const manifest = JSON.parse(
    await readFile(new URL("public/manifest.webmanifest", root), "utf8"),
  );

  assert.equal(manifest.id, "/");
  assert.equal(manifest.scope, "/");
  assert.match(manifest.start_url, /^\//);
  assert.equal(manifest.display, "standalone");
  assert.ok(manifest.icons.some((icon) => icon.purpose === "any"));
  assert.ok(manifest.icons.some((icon) => icon.purpose === "maskable"));
});

test("service worker excludes every supported OTA artifact from caching", async () => {
  const source = await readFile(new URL("public/sw.js", root), "utf8");
  assert.match(source, /SW_VERSION = "2026\.08\.10-3"/);
  const listeners = new Map();
  const context = {
    URL,
    caches: {},
    fetch() {},
    self: {
      addEventListener(type, handler) {
        listeners.set(type, handler);
      },
      clients: {},
      location: { origin: "https://example.test" },
    },
  };
  vm.createContext(context);
  vm.runInContext(source, context);

  for (const path of [
    "/firmware/m61",
    "/release/update.ota",
    "/release/update.bin.ota",
    "/release/stable.ota.json",
    "/release/update.bin",
    "/release/update.uf2",
    "/release/update.hex",
    "/release/update.img",
    "/release/download?firmware=m61",
  ]) {
    assert.equal(context.isOtaRequest(new URL(path, "https://example.test")), true, path);
  }

  assert.equal(context.isOtaRequest(new URL("/assets/app.js", "https://example.test")), false);
  assert.ok(listeners.has("fetch"));
});
