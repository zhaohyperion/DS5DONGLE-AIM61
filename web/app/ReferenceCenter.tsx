export type ReferenceLocale = "zh" | "en";

interface LocalizedText {
  zh: string;
  en: string;
}

interface FaqItem {
  question: LocalizedText;
  answer: LocalizedText;
}

const FAQ: FaqItem[] = [
  {
    question: {
      zh: "为什么点击连接后看不到设备？",
      en: "Why does no device appear after I choose Connect?",
    },
    answer: {
      zh: "请使用桌面版 Chrome 或 Edge，通过 USB 连接已刷入本项目固件的 Ai-M61，并关闭可能占用设备的其他配置器。iOS、Firefox 和普通内置浏览器通常不支持 WebHID。",
      en: "Use desktop Chrome or Edge, connect an Ai-M61 running this project firmware over USB, and close other configurators that may own the device. iOS, Firefox, and most embedded browsers do not support WebHID.",
    },
  },
  {
    question: {
      zh: "临时应用和保存到设备有什么区别？",
      en: "What is the difference between Apply and Save to device?",
    },
    answer: {
      zh: "临时应用会立即验证配置，但设备重启后恢复原值；确认手感、音频和轮询率正常后，再保存到 Flash。OTA 或高风险操作前建议先导出或记录当前配置。",
      en: "Apply lets you validate settings immediately but they revert after restart. Save to Flash only after input feel, audio, and polling are confirmed. Record your working settings before OTA or other high-risk operations.",
    },
  },
  {
    question: {
      zh: "普通与高速固件应该怎么选？",
      en: "Should I choose standard or high-speed firmware?",
    },
    answer: {
      zh: "M61 默认使用普通/全速 USB 固件，兼容性和稳定性优先。只有确认你的板型、USB 链路和固件发布清单都明确支持高速配置时才选择高速版；不要刷入 BL616、LCTech 或其他板型固件。",
      en: "For M61, use the standard/full-speed USB build by default for compatibility and stability. Select a high-speed build only when the board, USB path, and signed release manifest explicitly support it. Never install BL616, LCTech, or another board's image.",
    },
  },
  {
    question: {
      zh: "OTA 过程中可以拔线或关闭网页吗？",
      en: "Can I unplug the device or close the page during OTA?",
    },
    answer: {
      zh: "不可以。传输和校验完成前请保持 USB 与网页连接。固件写入非活动分区并带有启动回滚保护，但中断仍会导致本次升级失败；重新连接后应检查当前版本和设备状态。",
      en: "No. Keep USB and the page connected until transfer and verification finish. The inactive-slot write and boot rollback protect the running image, but interruption still fails the update; reconnect and check version and device status afterward.",
    },
  },
  {
    question: {
      zh: "为什么网页拒绝某个 OTA 文件？",
      en: "Why does the page reject an OTA file?",
    },
    answer: {
      zh: "网页会检查 BL60X RAW 头、M61 目标、大小、版本、SHA-256 和 P-256 签名。文件与清单不匹配、签名密钥不受信任、版本降级或镜像超出 A/B 分区容量都会被拒绝。不要通过修改网页绕过这些检查。",
      en: "The page checks the BL60X RAW header, M61 target, size, version, SHA-256, and P-256 signature. A file/manifest mismatch, untrusted key, downgrade, or image larger than the A/B slot is rejected. Do not bypass these checks by modifying the page.",
    },
  },
  {
    question: {
      zh: "网页会上传手柄或固件数据吗？",
      en: "Does the page upload controller or firmware data?",
    },
    answer: {
      zh: "设备配置与固件数据通过本机 WebHID 直连传输。在线升级只从你选择的 HTTPS 发布地址下载签名文件；OTA 固件不会进入 PWA 缓存。",
      en: "Configuration and firmware bytes travel locally over WebHID. Online update only downloads signed artifacts from the HTTPS release source you choose; OTA images are excluded from the PWA cache.",
    },
  },
];

const STEPS: LocalizedText[] = [
  {
    zh: "使用数据线连接 Ai-M61，打开桌面版 Chrome 或 Edge，然后授权 DS5Dongle HID 设备。",
    en: "Connect the Ai-M61 with a data cable, open desktop Chrome or Edge, then authorize the DS5Dongle HID device.",
  },
  {
    zh: "先读取设备版本、RSSI、电量和音频状态；异常时不要立即写入配置或执行 OTA。",
    en: "Read device version, RSSI, battery, and audio state first. Do not write settings or start OTA while status is abnormal.",
  },
  {
    zh: "调整参数或按键映射，先临时应用并实际测试，再保存到 Flash。",
    en: "Adjust parameters or remaps, apply temporarily and test them, then save to Flash.",
  },
  {
    zh: "升级时优先选择稳定通道；本地升级必须同时选择 `.bin.ota` 和匹配的签名清单。",
    en: "Prefer the stable channel for updates. A local update requires both the `.bin.ota` image and its matching signed manifest.",
  },
  {
    zh: "确认目标为 Ai-M61、版本和校验值正确后开始升级；设备重新连接后核对新版本与运行状态。",
    en: "Confirm the Ai-M61 target, version, and hashes before update; after reconnection, verify the new version and runtime status.",
  },
];

const CHANGES: Array<{ version: string; date: string; items: LocalizedText[] }> = [
  {
    version: "Web 0.3.2",
    date: "2026-08-11",
    items: [
      {
        zh: "开放 Ai-M61 High-Speed 签名 OTA；网页会按设备能力自动选择 FS/HS 清单，并在签名、传输和设备端执行同速校验。",
        en: "Enabled signed Ai-M61 High-Speed OTA with automatic FS/HS manifest selection and same-speed validation in the signature, transport, and firmware.",
      },
    ],
  },
  {
    version: "Web 0.3.1",
    date: "2026-08-10",
    items: [
      {
        zh: "按键映射中的方块、叉、圆圈和三角键改为对应手柄符号，并保留完整的中英文无障碍名称。",
        en: "Replaced the four face-button names in the remap editor with controller glyphs while preserving complete accessible labels.",
      },
    ],
  },
  {
    version: "Web 0.3.0",
    date: "2026-08-10",
    items: [
      {
        zh: "新增一键诊断中心，通过 WebHID 0xFD 每秒或手动读取带 CRC32 的结构化设备快照。",
        en: "Added one-click diagnostics with manual or 1 Hz CRC32-protected structured snapshots over WebHID report 0xFD.",
      },
      {
        zh: "支持 CH340 WebSerial 实时日志、有限内存环形缓冲，以及默认脱敏的单文件 JSON 诊断包导出。",
        en: "Added CH340 WebSerial live logs, bounded ring buffering, and single-file JSON diagnostic exports with redaction enabled by default.",
      },
      {
        zh: "诊断与 OTA 共用跨标签页 HID 维护锁；OTA 会等待当前诊断结束，并阻止其他页面并发访问维护报告。",
        en: "Diagnostics and OTA now share a cross-tab HID maintenance lock, so OTA waits for an active capture and excludes concurrent maintenance traffic.",
      },
    ],
  },
  {
    version: "Firmware 3.5.1",
    date: "2026-08-10",
    items: [
      {
        zh: "加入低优先级 1 Hz 诊断任务，以双缓冲原子发布 0xFD 分页快照，不进入音频或 USB 实时热路径。",
        en: "Added a low-priority 1 Hz diagnostics task that atomically publishes double-buffered 0xFD pages outside audio and USB real-time hot paths.",
      },
      {
        zh: "加入 LOG2 诊断信息与 USB、蓝牙、音频、OTA、堆内存计数；最低空闲内存为诊断任务启动以来的 1 Hz 采样最低值。",
        en: "Added LOG2 diagnostics and USB, Bluetooth, audio, OTA, and heap counters; minimum free heap is the lowest 1 Hz sample observed since the diagnostics task started.",
      },
    ],
  },
  {
    version: "Web 0.2.0",
    date: "2026-08-10",
    items: [
      {
        zh: "加入 F6/F7/F8/F9/FB 配置兼容层、完整参数面板、按键映射与实时设备状态。",
        en: "Added F6/F7/F8/F9/FB configuration compatibility, full controls, button remapping, and live device status.",
      },
      {
        zh: "加入普通/高速发布通道选择、PWA 离线网页壳、安装提示和缓存更新控制。",
        en: "Added standard/high-speed release selection, an offline PWA shell, install prompt, and cache refresh controls.",
      },
      {
        zh: "新增中英文操作说明、FAQ 与项目内更新日志。",
        en: "Added bilingual instructions, FAQ, and project-specific changelog.",
      },
    ],
  },
  {
    version: "Firmware 3.5.0",
    date: "2026-08-10",
    items: [
      {
        zh: "Ai-M61 固件加入受签名保护的 A/B OTA、P-256 验证、反降级和 Boot2 试运行回滚。",
        en: "Added signed A/B OTA, P-256 verification, downgrade prevention, and Boot2 trial rollback for Ai-M61.",
      },
      {
        zh: "完善 OTA 会话、CRC、超时退出和非活动分区容量检查。",
        en: "Hardened OTA sessions, CRC handling, inactivity timeout, and inactive-slot capacity checks.",
      },
      {
        zh: "首个面向 Ai-M61 实机的安全 OTA 基线版本；用于首次串口刷写与后续升级验证。",
        en: "Initial secure-OTA baseline for physical Ai-M61 hardware, intended for first serial flash and subsequent update validation.",
      },
    ],
  },
];

export function ReferenceCenter({ locale = "zh" }: { locale?: ReferenceLocale }) {
  const text = (value: LocalizedText) => value[locale];

  return (
    <section className="reference-center" aria-labelledby="reference-heading">
      <div className="section-heading">
        <div>
          <span className="section-kicker">GUIDE / FAQ / CHANGELOG</span>
          <h2 id="reference-heading">{locale === "zh" ? "使用与维护" : "Use & maintenance"}</h2>
        </div>
        <p>
          {locale === "zh"
            ? "适用于当前 Ai-M61 硬件与本项目固件；其他 BL616/BL618 开发板的分区和 USB 参数不能直接套用。"
            : "For the current Ai-M61 hardware and this project firmware. Partition and USB settings from other BL616/BL618 boards are not interchangeable."}
        </p>
      </div>

      <div className="reference-grid">
        <article className="panel reference-guide">
          <div className="panel-heading">
            <span className="section-kicker">01 / QUICK START</span>
            <h3>{locale === "zh" ? "安全操作步骤" : "Safe workflow"}</h3>
            <p>
              {locale === "zh"
                ? "从连接、试调到保存和升级的推荐顺序。"
                : "Recommended order from connection and testing through save and update."}
            </p>
          </div>
          <div className="panel-body">
            <ol className="reference-steps">
              {STEPS.map((step, index) => (
                <li key={step.en}>
                  <span>{String(index + 1).padStart(2, "0")}</span>
                  <p>{text(step)}</p>
                </li>
              ))}
            </ol>
          </div>
        </article>

        <article className="panel reference-faq">
          <div className="panel-heading">
            <span className="section-kicker">02 / FAQ</span>
            <h3>{locale === "zh" ? "常见问题" : "Frequently asked questions"}</h3>
            <p>
              {locale === "zh"
                ? "连接、配置、固件选择与 OTA 安全说明。"
                : "Connection, configuration, build selection, and OTA safety."}
            </p>
          </div>
          <div className="panel-body faq-list">
            {FAQ.map((item, index) => (
              <details key={item.question.en} open={index === 0}>
                <summary>{text(item.question)}</summary>
                <p>{text(item.answer)}</p>
              </details>
            ))}
          </div>
        </article>
      </div>

      <article className="panel changelog-panel">
        <div className="panel-heading">
          <span className="section-kicker">03 / CHANGELOG</span>
          <h3>{locale === "zh" ? "项目更新日志" : "Project changelog"}</h3>
          <p>
            {locale === "zh"
              ? "仅记录当前开源 Ai-M61 分支实际提供的功能。"
              : "Only features actually shipped by the current open-source Ai-M61 branch are listed."}
          </p>
        </div>
        <div className="panel-body changelog-list">
          {CHANGES.map((release) => (
            <section key={release.version}>
              <header>
                <strong>{release.version}</strong>
                <time dateTime={release.date}>{release.date}</time>
              </header>
              <ul>
                {release.items.map((item) => (
                  <li key={item.en}>{text(item)}</li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      </article>
    </section>
  );
}
