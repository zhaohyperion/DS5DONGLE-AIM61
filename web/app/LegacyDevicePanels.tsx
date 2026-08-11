"use client";

import type { ReactNode } from "react";
import {
  LEGACY_BUTTON_NAMES,
  type LegacyConfig,
  type LegacyRemapEntry,
  type LegacyTelemetry,
} from "./lib/legacy-hid";

type Language = "zh" | "en";
type MaybePromise = void | Promise<void>;

export interface TelemetryStripProps {
  status: LegacyTelemetry | null;
  firmwareVersion?: string;
  otaReady?: boolean;
  language?: Language;
}

export interface FullConfigEditorProps {
  config: LegacyConfig;
  onConfigChange: (next: LegacyConfig) => void;
  onRead: () => MaybePromise;
  onApply: (config: LegacyConfig) => MaybePromise;
  onSave: (config: LegacyConfig) => MaybePromise;
  onReconnect: () => MaybePromise;
  onReset: () => MaybePromise;
  busy?: boolean | string;
  disabled?: boolean;
  language?: Language;
}

export interface ButtonRemapEditorProps {
  remap: LegacyRemapEntry[];
  onRemapChange: (next: LegacyRemapEntry[]) => void;
  onRead: () => MaybePromise;
  onSave: (remap: LegacyRemapEntry[]) => MaybePromise;
  onReset: () => MaybePromise;
  busy?: boolean | string;
  disabled?: boolean;
  language?: Language;
}

const REMAP_TYPE_BUTTON = 0;
const REMAP_TYPE_KEYBOARD = 1;
const REMAP_FLAG_SUPPRESS = 0x01;
const REMAP_FLAG_EXTRA_KEY = 0x02;

const KEYBOARD_KEYS: ReadonlyArray<readonly [number, string]> = [
  [0x00, "None"],
  ...Array.from({ length: 26 }, (_, index) => [0x04 + index, String.fromCharCode(65 + index)] as const),
  ...["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].map(
    (name, index) => [0x1e + index, name] as const,
  ),
  [0x28, "Enter"],
  [0x29, "Escape"],
  [0x2a, "Backspace"],
  [0x2b, "Tab"],
  [0x2c, "Space"],
  [0x2d, "- / _"],
  [0x2e, "= / +"],
  [0x2f, "[ / {"],
  [0x30, "] / }"],
  [0x31, "\\ / |"],
  [0x33, "; / :"],
  [0x34, "' / \""],
  [0x35, "` / ~"],
  [0x36, ", / <"],
  [0x37, ". / >"],
  [0x38, "/ / ?"],
  [0x39, "Caps Lock"],
  ...Array.from({ length: 12 }, (_, index) => [0x3a + index, `F${index + 1}`] as const),
  [0x46, "Print Screen"],
  [0x47, "Scroll Lock"],
  [0x48, "Pause"],
  [0x49, "Insert"],
  [0x4a, "Home"],
  [0x4b, "Page Up"],
  [0x4c, "Delete"],
  [0x4d, "End"],
  [0x4e, "Page Down"],
  [0x4f, "Right"],
  [0x50, "Left"],
  [0x51, "Down"],
  [0x52, "Up"],
  [0x53, "Num Lock"],
  [0x54, "Keypad /"],
  [0x55, "Keypad *"],
  [0x56, "Keypad -"],
  [0x57, "Keypad +"],
  [0x58, "Keypad Enter"],
  ...["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].map(
    (name, index) => [0x59 + index, `Keypad ${name}`] as const,
  ),
  [0x63, "Keypad ."],
  [0x65, "Application"],
  ...Array.from({ length: 12 }, (_, index) => [0x68 + index, `F${index + 13}`] as const),
];

const FACE_BUTTONS: Readonly<Partial<Record<string, {
  glyph: string;
  tone: string;
  zh: string;
  en: string;
}>>> = {
  Square: { glyph: "□", tone: "square", zh: "方块键", en: "Square button" },
  Cross: { glyph: "×", tone: "cross", zh: "叉键", en: "Cross button" },
  Circle: { glyph: "○", tone: "circle", zh: "圆圈键", en: "Circle button" },
  Triangle: { glyph: "△", tone: "triangle", zh: "三角键", en: "Triangle button" },
};

function tr(language: Language, zh: string, en: string): string {
  return language === "zh" ? zh : en;
}

export function TelemetryStrip({
  status,
  firmwareVersion = "—",
  otaReady = false,
  language = "zh",
}: TelemetryStripProps) {
  const batteryState = status?.batteryPercent == null
    ? "—"
    : describeBatteryState(status.batteryState, language);
  const audioState = status?.audioValid
    ? `${tr(language, "扬声器", "SPK")} ${onOff(status.speakerActive, language)} · ${tr(language, "麦克风", "MIC")} ${onOff(status.microphoneActive, language)}`
    : "—";

  return (
    <div
      className="device-metrics"
      aria-label={tr(language, "设备实时状态", "Live device telemetry")}
      aria-live="polite"
    >
      <Metric label={tr(language, "固件", "Firmware")} value={firmwareVersion} mono />
      <Metric label="RSSI" value={status?.rssiDbm == null ? "—" : `${status.rssiDbm} dBm`} mono />
      <Metric
        label={tr(language, "电量", "Battery")}
        value={status?.batteryPercent == null ? "—" : `${status.batteryPercent}%`}
        mono
      />
      <Metric label={tr(language, "充电状态", "Power")} value={batteryState} accent={status?.batteryState === 1 || status?.batteryState === 2} />
      <Metric label={tr(language, "音频状态", "Audio")} value={audioState} accent={Boolean(status?.speakerActive || status?.microphoneActive)} />
      <Metric
        label="OTA"
        value={otaReady ? tr(language, "可用", "Ready") : tr(language, "未探测", "Unavailable")}
        accent={otaReady}
      />
    </div>
  );
}

export function FullConfigEditor({
  config,
  onConfigChange,
  onRead,
  onApply,
  onSave,
  onReconnect,
  onReset,
  busy = false,
  disabled = false,
  language = "zh",
}: FullConfigEditorProps) {
  const locked = disabled || Boolean(busy);
  const update = <K extends keyof LegacyConfig>(key: K, value: LegacyConfig[K]) => {
    onConfigChange({ ...config, [key]: value });
  };

  return (
    <section className="config-workspace" aria-label={tr(language, "完整设备参数", "Complete device configuration")}>
      <Panel
        title={tr(language, "反馈与音频", "Feedback & audio")}
        subtitle={tr(language, "触觉、音量和音频传输参数", "Haptics, volume, and audio transport")}
      >
        <RangeControl label={tr(language, "触觉增益", "Haptics gain")} value={config.hapticsGain} min={1} max={2} step={0.05} display={config.hapticsGain.toFixed(2)} disabled={locked} onChange={(value) => update("hapticsGain", value)} />
        <RangeControl label={tr(language, "扬声器音量", "Speaker volume")} value={config.speakerVolume} min={0} max={127} disabled={locked} onChange={(value) => update("speakerVolume", value)} />
        <RangeControl label={tr(language, "耳机音量", "Headset volume")} value={config.headsetVolume} min={0} max={127} disabled={locked} onChange={(value) => update("headsetVolume", value)} />
        <RangeControl label={tr(language, "扬声器增益", "Speaker gain")} value={config.speakerGain} min={0} max={7} disabled={locked} onChange={(value) => update("speakerGain", value)} />
        <RangeControl label={tr(language, "扳机反馈削减", "Trigger reduction")} value={config.triggerReduce} min={0} max={10} disabled={locked} onChange={(value) => update("triggerReduce", value)} />
        <RangeControl label={tr(language, "音频缓冲长度", "Audio buffer length")} value={config.audioBufferLength} min={16} max={127} disabled={locked} onChange={(value) => update("audioBufferLength", value)} />
      </Panel>

      <Panel
        title={tr(language, "性能与兼容", "Performance & compatibility")}
        subtitle={tr(language, "带 * 的项目保存后需要重新连接 USB", "Reconnect USB after saving items marked *")}
      >
        <Segmented label={`${tr(language, "轮询率", "Polling rate")} *`} value={config.pollingRateMode} options={[[0, "250 Hz"], [1, "500 Hz"], [2, tr(language, "实时", "Realtime")]]} disabled={locked} onChange={(value) => update("pollingRateMode", value as LegacyConfig["pollingRateMode"])} />
        <Segmented label={`${tr(language, "控制器模式", "Controller mode")} *`} value={config.controllerMode} options={[[0, "DS5"], [1, "DSE"], [2, tr(language, "自动", "Auto")]]} disabled={locked} onChange={(value) => update("controllerMode", value as LegacyConfig["controllerMode"])} />
        <RangeControl label={tr(language, "闲置断开（分钟）", "Idle disconnect (minutes)")} value={config.inactiveTime} min={0} max={60} disabled={locked} onChange={(value) => update("inactiveTime", value)} />
        <Toggle label={`${tr(language, "USB 序列号", "USB serial number")} *`} checked={config.enableUsbSerial} disabled={locked} onChange={(value) => update("enableUsbSerial", value)} />
        <Toggle label={`${tr(language, "等待蓝牙后显示 USB", "USB stealth until Bluetooth")} *`} checked={config.usbStealth} disabled={locked} onChange={(value) => update("usbStealth", value)} />
        <Toggle label={tr(language, "PS 键 Xbox 指南快捷键", "PS button Xbox guide shortcut")} checked={config.psShortcutEnabled} disabled={locked} onChange={(value) => update("psShortcutEnabled", value)} />
        <Toggle label={tr(language, "已检测到 DSE（只读）", "DSE detected (read only)")} checked={config.dseDetected} disabled readOnly />
      </Panel>

      <Panel
        title={tr(language, "电源与灯光", "Power & lighting")}
        subtitle={tr(language, "待机、音频接口和状态灯行为", "Standby, audio interfaces, and status light")}
      >
        <Toggle label={tr(language, "自动关闭板载 LED", "Auto-disable onboard LED")} checked={config.disableLed} disabled={locked} onChange={(value) => update("disableLed", value)} />
        <Toggle label={tr(language, "允许 USB 唤醒", "Allow USB wake")} checked={config.enableWake} disabled={locked} onChange={(value) => update("enableWake", value)} />
        <Toggle label={tr(language, "禁用控制器麦克风", "Disable controller microphone")} checked={config.disableMic} disabled={locked} onChange={(value) => update("disableMic", value)} />
        <Toggle label={tr(language, "禁用扬声器与耳机", "Disable speaker and headset")} checked={config.disableSpeaker} disabled={locked} onChange={(value) => update("disableSpeaker", value)} />
        <Toggle label={tr(language, "锁定设备音量", "Lock device volume")} checked={config.lockVolume} disabled={locked} onChange={(value) => update("lockVolume", value)} />
        <label className="color-control">
          <span>{tr(language, "自定义状态灯", "Custom status light")}</span>
          <input
            aria-label={tr(language, "状态灯颜色", "Status light color")}
            type="color"
            value={rgbHex(config.ledR, config.ledG, config.ledB)}
            disabled={locked}
            onChange={(event) => {
              const [ledR, ledG, ledB] = parseColor(event.target.value);
              onConfigChange({ ...config, ledR, ledG, ledB });
            }}
          />
          <code>{rgbHex(config.ledR, config.ledG, config.ledB).toUpperCase()}</code>
        </label>
      </Panel>

      <div className="action-dock">
        <div>
          <strong>{tr(language, `配置 v${config.configVersion} · 25 字节`, `Config v${config.configVersion} · 25 bytes`)}</strong>
          <span>{tr(language, "应用只修改运行参数；保存会写入设备 Flash。", "Apply changes runtime settings; Save persists them to flash.")}</span>
        </div>
        <div>
          <button className="ghost-button" type="button" disabled={locked} onClick={() => void onRead()}>{tr(language, "读取", "Read")}</button>
          <button className="secondary-button" type="button" disabled={locked} onClick={() => void onReset()}>{tr(language, "恢复默认", "Defaults")}</button>
          <button className="secondary-button" type="button" disabled={locked} onClick={() => void onApply(config)}>{tr(language, "应用", "Apply")}</button>
          <button className="primary-button" type="button" disabled={locked} onClick={() => void onSave(config)}>{tr(language, "应用并保存", "Apply & save")}</button>
          <button className="ghost-button" type="button" disabled={locked} onClick={() => void onReconnect()}>{tr(language, "重连 USB", "Reconnect USB")}</button>
        </div>
      </div>
    </section>
  );
}

export function ButtonRemapEditor({
  remap,
  onRemapChange,
  onRead,
  onSave,
  onReset,
  busy = false,
  disabled = false,
  language = "zh",
}: ButtonRemapEditorProps) {
  const locked = disabled || Boolean(busy);
  const entryAt = (index: number): LegacyRemapEntry => remap[index] ?? {
    type: REMAP_TYPE_BUTTON,
    value: index,
    modifier: 0,
    flags: 0,
  };
  const updateEntry = (index: number, next: LegacyRemapEntry) => {
    onRemapChange(LEGACY_BUTTON_NAMES.map((_, entryIndex) => entryIndex === index ? next : entryAt(entryIndex)));
  };

  return (
    <section aria-label={tr(language, "按键映射编辑器", "Button remap editor")}>
      <div className="section-heading">
        <div><span className="section-kicker">0xFB · 15 × 4 B</span><h2>{tr(language, "按键映射", "Button mapping")}</h2></div>
        <p>{tr(language, "每个 DS5 输入可映射到另一手柄键或 USB HID 键盘键，并可保留或屏蔽原手柄输入。", "Map each DS5 input to another controller button or USB HID key, while preserving or suppressing the original input.")}</p>
      </div>

      <div className="remap-grid">
        {LEGACY_BUTTON_NAMES.map((sourceName, index) => {
          const entry = entryAt(index);
          const keyboard = entry.type === REMAP_TYPE_KEYBOARD;
          const extraKey = Boolean(entry.flags & REMAP_FLAG_EXTRA_KEY);
          const accessibleSourceName = controllerButtonAccessibleName(sourceName, language);
          return (
            <article className="panel" key={sourceName}>
              <div className="panel-heading">
                <h3 className="remap-source-title">
                  <ControllerButtonLabel name={sourceName} language={language} />
                </h3>
                <p>{tr(language, `输入 ${String(index + 1).padStart(2, "0")}`, `Input ${String(index + 1).padStart(2, "0")}`)}</p>
              </div>
              <div className="panel-body">
                <Segmented
                  label={tr(language, "目标类型", "Target type")}
                  value={entry.type}
                  options={[[REMAP_TYPE_BUTTON, tr(language, "手柄", "Controller")], [REMAP_TYPE_KEYBOARD, tr(language, "键盘 HID", "Keyboard HID")]]}
                  disabled={locked}
                  onChange={(type) => updateEntry(index, {
                    type: type as LegacyRemapEntry["type"],
                    value: type === REMAP_TYPE_KEYBOARD ? 0x04 : index,
                    modifier: 0,
                    flags: entry.flags & REMAP_FLAG_SUPPRESS,
                  })}
                />

                <label className="remap-row">
                  <span><small>{tr(language, "目标", "TARGET")}</small>{keyboard ? tr(language, "主按键", "Primary key") : tr(language, "手柄按键", "Controller button")}</span>
                  <i aria-hidden="true">→</i>
                  <select
                    aria-label={`${accessibleSourceName} ${tr(language, "映射目标", "mapping target")}`}
                    value={entry.value}
                    disabled={locked}
                    onChange={(event) => updateEntry(index, { ...entry, value: Number(event.target.value) })}
                  >
                    {(keyboard ? KEYBOARD_KEYS : LEGACY_BUTTON_NAMES.map((name, buttonIndex) => [buttonIndex, name] as const)).map(([value, name]) => (
                      <option value={value} key={value}>
                        {keyboard ? name : controllerButtonOptionLabel(name, language)}
                      </option>
                    ))}
                  </select>
                </label>

                {keyboard && (
                  <>
                    <Segmented
                      label={tr(language, "组合方式", "Combination mode")}
                      value={extraKey ? 1 : 0}
                      options={[[0, tr(language, "修饰键", "Modifier")], [1, tr(language, "第二键", "Second key")]]}
                      disabled={locked}
                      onChange={(mode) => updateEntry(index, {
                        ...entry,
                        modifier: 0,
                        flags: mode === 1 ? entry.flags | REMAP_FLAG_EXTRA_KEY : entry.flags & ~REMAP_FLAG_EXTRA_KEY,
                      })}
                    />
                    {extraKey ? (
                      <label className="remap-row">
                        <span><small>HID</small>{tr(language, "第二按键", "Second key")}</span>
                        <i aria-hidden="true">+</i>
                        <select
                          aria-label={`${accessibleSourceName} ${tr(language, "第二按键", "second key")}`}
                          value={entry.modifier}
                          disabled={locked}
                          onChange={(event) => updateEntry(index, { ...entry, modifier: Number(event.target.value) })}
                        >
                          {KEYBOARD_KEYS.map(([value, name]) => <option value={value} key={value}>{name}</option>)}
                        </select>
                      </label>
                    ) : (
                      <RangeControl
                        label={tr(language, "USB 修饰键位掩码", "USB modifier mask")}
                        value={entry.modifier}
                        min={0}
                        max={255}
                        display={`0x${entry.modifier.toString(16).padStart(2, "0").toUpperCase()}`}
                        disabled={locked}
                        onChange={(modifier) => updateEntry(index, { ...entry, modifier })}
                      />
                    )}
                  </>
                )}

                <Toggle
                  label={tr(language, "屏蔽原手柄输入", "Suppress original controller input")}
                  checked={Boolean(entry.flags & REMAP_FLAG_SUPPRESS)}
                  disabled={locked}
                  onChange={(checked) => updateEntry(index, {
                    ...entry,
                    flags: checked ? entry.flags | REMAP_FLAG_SUPPRESS : entry.flags & ~REMAP_FLAG_SUPPRESS,
                  })}
                />
              </div>
            </article>
          );
        })}
      </div>

      <div className="action-dock compact">
        <div>
          <strong>{tr(language, "15 个来源 · 手柄 / 键盘 HID", "15 sources · Controller / keyboard HID")}</strong>
          <span>{tr(language, "键盘映射会完整保存；是否输出取决于连接设备的固件能力。", "Keyboard mappings are preserved; output depends on the connected firmware capability.")}</span>
        </div>
        <div>
          <button className="ghost-button" type="button" disabled={locked} onClick={() => void onRead()}>{tr(language, "读取", "Read")}</button>
          <button className="secondary-button" type="button" disabled={locked} onClick={() => void onReset()}>{tr(language, "恢复默认", "Defaults")}</button>
          <button className="primary-button" type="button" disabled={locked} onClick={() => void onSave(remap)}>{tr(language, "保存映射", "Save mapping")}</button>
        </div>
      </div>
    </section>
  );
}

function Metric({ label, value, mono = false, accent = false }: { label: string; value: string; mono?: boolean; accent?: boolean }) {
  return <div className="metric"><span>{label}</span><strong className={`${mono ? "mono" : ""} ${accent ? "accent" : ""}`}>{value}</strong></div>;
}

function Panel({ title, subtitle, children }: { title: string; subtitle: string; children: ReactNode }) {
  return <article className="panel"><div className="panel-heading"><h3>{title}</h3><p>{subtitle}</p></div><div className="panel-body">{children}</div></article>;
}

function RangeControl({ label, value, min, max, step = 1, display, disabled = false, onChange }: { label: string; value: number; min: number; max: number; step?: number; display?: string; disabled?: boolean; onChange: (value: number) => void }) {
  return <label className="range-control"><span><strong>{label}</strong><code>{display ?? value}</code></span><input aria-label={label} type="range" min={min} max={max} step={step} value={value} disabled={disabled} onChange={(event) => onChange(Number(event.target.value))} /></label>;
}

function Toggle({ label, checked, disabled = false, readOnly = false, onChange }: { label: string; checked: boolean; disabled?: boolean; readOnly?: boolean; onChange?: (value: boolean) => void }) {
  return <label className="toggle-row"><span>{label}</span><input aria-label={label} type="checkbox" checked={checked} disabled={disabled} readOnly={readOnly || !onChange} onChange={(event) => onChange?.(event.target.checked)} /><i aria-hidden="true" /></label>;
}

function Segmented({ label, value, options, disabled = false, onChange }: { label: string; value: number; options: Array<[number, string]>; disabled?: boolean; onChange: (value: number) => void }) {
  return <div className="segmented-row"><span>{label}</span><div role="group" aria-label={label}>{options.map(([id, text]) => <button type="button" key={id} className={value === id ? "is-active" : ""} aria-pressed={value === id} disabled={disabled} onClick={() => onChange(id)}>{text}</button>)}</div></div>;
}

function ControllerButtonLabel({ name, language }: { name: string; language: Language }) {
  const face = FACE_BUTTONS[name];
  if (!face) return name;
  const accessibleName = tr(language, face.zh, face.en);
  return (
    <span className={`controller-button-icon is-${face.tone}`} title={accessibleName}>
      <span aria-hidden="true">{face.glyph}</span>
      <span className="visually-hidden">{accessibleName}</span>
    </span>
  );
}

function controllerButtonAccessibleName(name: string, language: Language): string {
  const face = FACE_BUTTONS[name];
  return face ? tr(language, face.zh, face.en) : name;
}

function controllerButtonOptionLabel(name: string, language: Language): string {
  const face = FACE_BUTTONS[name];
  return face ? `${face.glyph} ${tr(language, face.zh, face.en)}` : name;
}

function onOff(active: boolean, language: Language): string {
  return active ? tr(language, "开启", "ON") : tr(language, "关闭", "OFF");
}

function describeBatteryState(state: number | null, language: Language): string {
  if (state == null) return "—";
  if (state === 0) return tr(language, "放电中", "Discharging");
  if (state === 1) return tr(language, "充电中", "Charging");
  if (state === 2) return tr(language, "已充满", "Full");
  return `${tr(language, "状态", "State")} ${state}`;
}

function rgbHex(red: number, green: number, blue: number): string {
  return `#${[red, green, blue].map((value) => value.toString(16).padStart(2, "0")).join("")}`;
}

function parseColor(value: string): [number, number, number] {
  return [Number.parseInt(value.slice(1, 3), 16), Number.parseInt(value.slice(3, 5), 16), Number.parseInt(value.slice(5, 7), 16)];
}
