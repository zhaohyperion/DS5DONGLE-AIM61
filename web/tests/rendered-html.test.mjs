import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

async function render() {
  const workerUrl = new URL("../dist/server/index.js", import.meta.url);
  workerUrl.searchParams.set("test", `${process.pid}-${Date.now()}`);
  const { default: worker } = await import(workerUrl.href);
  return worker.fetch(
    new Request("http://localhost/", { headers: { accept: "text/html" } }),
    { ASSETS: { fetch: async () => new Response("Not found", { status: 404 }) } },
    { waitUntil() {}, passThroughOnException() {} },
  );
}

test("server-renders the DS5Dongle device studio", async () => {
  const response = await render();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html\b/i);
  const html = await response.text();
  assert.match(html, /<title>DS5Dongle · Ai-M61 设备工作台<\/title>/i);
  assert.match(html, /Ai-M61 Device Studio/);
  assert.match(html, /Ai-M61-32S-Kit/);
  assert.match(html, /固件更新/);
  assert.doesNotMatch(html, /codex-preview|Your site is taking shape|react-loading-skeleton/i);
});

test("keeps Sites hosting wiring and removes preview scaffolding", async () => {
  const [vite, page, layout, packageJson] = await Promise.all([
    readFile(new URL("../vite.config.cjs", import.meta.url), "utf8"),
    readFile(new URL("../app/page.tsx", import.meta.url), "utf8"),
    readFile(new URL("../app/layout.tsx", import.meta.url), "utf8"),
    readFile(new URL("../package.json", import.meta.url), "utf8"),
  ]);
  assert.match(vite, /sites\(\)/);
  assert.match(page, /DeviceConsole/);
  assert.match(layout, /Ai-M61 设备工作台/);
  assert.doesNotMatch(packageJson, /react-loading-skeleton/);
});
