/* DS5Dongle Ai-M61 PWA service worker.
 * OTA payloads and manifests intentionally bypass every cache. Firmware must
 * always be fetched and verified by the foreground OTA protocol implementation.
 */

const SW_VERSION = "2026.08.10-3";
const CACHE_PREFIX = "ds5dongle-m61-";
const SHELL_CACHE = `${CACHE_PREFIX}shell-${SW_VERSION}`;
const STATIC_CACHE = `${CACHE_PREFIX}static-${SW_VERSION}`;
const SHELL_URLS = ["/", "/manifest.webmanifest", "/favicon.svg"];

function isOtaRequest(url) {
  const path = url.pathname.toLowerCase();
  return (
    path.includes("/firmware/") ||
    path.endsWith(".ota") ||
    path.endsWith(".ota.json") ||
    path.endsWith(".bin") ||
    path.endsWith(".uf2") ||
    path.endsWith(".hex") ||
    path.endsWith(".img") ||
    url.searchParams.has("ota") ||
    url.searchParams.has("firmware")
  );
}

function isStaticAsset(url) {
  return (
    url.pathname.startsWith("/_next/static/") ||
    url.pathname.startsWith("/assets/") ||
    /\.(?:css|js|mjs|woff2?|ttf|svg|png|webp|ico)$/i.test(url.pathname)
  );
}

async function clearAppCaches() {
  const keys = await caches.keys();
  await Promise.all(
    keys.filter((key) => key.startsWith(CACHE_PREFIX)).map((key) => caches.delete(key)),
  );
}

async function warmShellCache() {
  const cache = await caches.open(SHELL_CACHE);
  await Promise.allSettled(
    SHELL_URLS.map(async (path) => {
      const response = await fetch(path, { cache: "no-store", credentials: "same-origin" });
      if (response.ok) await cache.put(path, response);
    }),
  );
}

self.addEventListener("install", (event) => {
  event.waitUntil(warmShellCache());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(
        keys
          .filter(
            (key) =>
              key.startsWith(CACHE_PREFIX) && key !== SHELL_CACHE && key !== STATIC_CACHE,
          )
          .map((key) => caches.delete(key)),
      );
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("message", (event) => {
  const type = event.data?.type;
  if (type === "SKIP_WAITING") {
    self.skipWaiting();
    return;
  }

  if (type === "GET_VERSION") {
    event.ports[0]?.postMessage({ ok: true, version: SW_VERSION });
    return;
  }

  if (type === "REFRESH_CACHES") {
    event.waitUntil(
      (async () => {
        try {
          await clearAppCaches();
          await warmShellCache();
          event.ports[0]?.postMessage({ ok: true, version: SW_VERSION });
        } catch (error) {
          event.ports[0]?.postMessage({
            ok: false,
            error: error instanceof Error ? error.message : "cache refresh failed",
          });
        }
      })(),
    );
  }
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin || isOtaRequest(url)) return;

  if (request.mode === "navigate") {
    event.respondWith(
      (async () => {
        try {
          const response = await fetch(request);
          if (response.ok) {
            const cache = await caches.open(SHELL_CACHE);
            await cache.put("/", response.clone());
          }
          return response;
        } catch (error) {
          const cached = await caches.match("/");
          if (cached) return cached;
          throw error;
        }
      })(),
    );
    return;
  }

  if (isStaticAsset(url)) {
    event.respondWith(
      (async () => {
        const cached = await caches.match(request);
        if (cached) return cached;
        const response = await fetch(request);
        if (response.ok) {
          const cache = await caches.open(STATIC_CACHE);
          await cache.put(request, response.clone());
        }
        return response;
      })(),
    );
  }
});
