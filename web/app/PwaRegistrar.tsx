"use client";

import { useCallback, useEffect, useRef, useState } from "react";

export type PwaLocale = "zh" | "en";

interface BeforeInstallPromptEvent extends Event {
  prompt(): Promise<void>;
  userChoice: Promise<{ outcome: "accepted" | "dismissed"; platform: string }>;
}

interface WorkerReply {
  ok: boolean;
  version?: string;
  error?: string;
}

function sendWorkerMessage(
  worker: ServiceWorker,
  message: { type: string },
  timeoutMs = 8_000,
): Promise<WorkerReply> {
  return new Promise((resolve, reject) => {
    const channel = new MessageChannel();
    const timeout = window.setTimeout(() => reject(new Error("Service worker timeout")), timeoutMs);
    channel.port1.onmessage = (event: MessageEvent<WorkerReply>) => {
      window.clearTimeout(timeout);
      resolve(event.data);
    };
    worker.postMessage(message, [channel.port2]);
  });
}

export function PwaRegistrar({ locale = "zh" }: { locale?: PwaLocale }) {
  const [supported, setSupported] = useState(true);
  const [ready, setReady] = useState(false);
  const [installed, setInstalled] = useState(false);
  const [updateReady, setUpdateReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const registrationRef = useRef<ServiceWorkerRegistration | null>(null);
  const installPromptRef = useRef<BeforeInstallPromptEvent | null>(null);
  const tr = useCallback((zh: string, en: string) => (locale === "zh" ? zh : en), [locale]);

  useEffect(() => {
    if (!("serviceWorker" in navigator)) {
      const unsupportedTimer = window.setTimeout(() => setSupported(false), 0);
      return () => window.clearTimeout(unsupportedTimer);
    }

    const installedTimer = window.setTimeout(
      () => setInstalled(window.matchMedia("(display-mode: standalone)").matches),
      0,
    );
    let disposed = false;
    let refreshing = false;

    const onInstallPrompt = (event: Event) => {
      event.preventDefault();
      installPromptRef.current = event as BeforeInstallPromptEvent;
      if (!disposed) setReady(true);
    };
    const onInstalled = () => {
      installPromptRef.current = null;
      setInstalled(true);
      setReady(false);
    };
    const onControllerChange = () => {
      if (refreshing) return;
      refreshing = true;
      window.location.reload();
    };

    window.addEventListener("beforeinstallprompt", onInstallPrompt);
    window.addEventListener("appinstalled", onInstalled);
    navigator.serviceWorker.addEventListener("controllerchange", onControllerChange);

    navigator.serviceWorker
      .register("/sw.js", { scope: "/", updateViaCache: "none" })
      .then((registration) => {
        if (disposed) return;
        registrationRef.current = registration;
        setUpdateReady(Boolean(registration.waiting));

        const watchInstalling = () => {
          const worker = registration.installing;
          if (!worker) return;
          worker.addEventListener("statechange", () => {
            if (worker.state === "installed" && navigator.serviceWorker.controller) {
              setUpdateReady(true);
              setNote(tr("网页更新已就绪", "A web update is ready"));
            }
          });
        };
        registration.addEventListener("updatefound", watchInstalling);
        void registration.update();
      })
      .catch(() => {
        if (!disposed) setSupported(false);
      });

    return () => {
      disposed = true;
      window.clearTimeout(installedTimer);
      window.removeEventListener("beforeinstallprompt", onInstallPrompt);
      window.removeEventListener("appinstalled", onInstalled);
      navigator.serviceWorker.removeEventListener("controllerchange", onControllerChange);
    };
  }, [tr]);

  const install = async () => {
    const prompt = installPromptRef.current;
    if (!prompt) return;
    setBusy(true);
    try {
      await prompt.prompt();
      const choice = await prompt.userChoice;
      if (choice.outcome === "dismissed") {
        setNote(tr("已取消安装，稍后仍可从浏览器菜单安装", "Installation dismissed; you can install later from the browser menu"));
      }
    } finally {
      installPromptRef.current = null;
      setReady(false);
      setBusy(false);
    }
  };

  const applyUpdate = () => {
    const waiting = registrationRef.current?.waiting;
    if (!waiting) return;
    setBusy(true);
    waiting.postMessage({ type: "SKIP_WAITING" });
  };

  const refreshCaches = async () => {
    setBusy(true);
    setNote("");
    try {
      const registration = registrationRef.current;
      await registration?.update();
      const worker = navigator.serviceWorker.controller ?? registration?.active;
      if (!worker) throw new Error("Service worker is not active");
      const reply = await sendWorkerMessage(worker, { type: "REFRESH_CACHES" });
      if (!reply.ok) throw new Error(reply.error ?? "Cache refresh failed");
      setNote(tr("网页缓存已刷新；OTA 固件始终不会缓存", "Web cache refreshed; OTA firmware is never cached"));
    } catch {
      setNote(tr("缓存刷新失败，请保持联网后重试", "Cache refresh failed; reconnect and try again"));
    } finally {
      setBusy(false);
    }
  };

  if (!supported) return null;

  return (
    <section className="panel pwa-panel" aria-labelledby="pwa-heading">
      <div className="panel-heading">
        <span className="section-kicker">PWA / OFFLINE SHELL</span>
        <h3 id="pwa-heading">{tr("安装网页工具", "Install web tool")}</h3>
        <p>
          {tr(
            "网页壳与静态资源可离线使用；出于安全考虑，OTA 固件和签名清单永不缓存。",
            "The web shell and static assets work offline. OTA images and signed manifests are never cached for safety.",
          )}
        </p>
      </div>
      <div className="panel-body">
        <div className="pwa-actions">
          {ready && !installed ? (
            <button className="secondary-button" type="button" disabled={busy} onClick={install}>
              {tr("安装到设备", "Install app")}
            </button>
          ) : null}
          {updateReady ? (
            <button className="primary-button" type="button" disabled={busy} onClick={applyUpdate}>
              {tr("应用网页更新", "Apply web update")}
            </button>
          ) : null}
          <button className="ghost-button" type="button" disabled={busy} onClick={refreshCaches}>
            {busy ? tr("处理中…", "Working…") : tr("刷新网页缓存", "Refresh web cache")}
          </button>
        </div>
        <small className="pwa-status" role="status" aria-live="polite">
          {note ||
            (installed
              ? tr("已作为应用运行", "Running as an installed app")
              : tr("可继续在浏览器中使用", "Ready in the browser"))}
        </small>
      </div>
    </section>
  );
}
