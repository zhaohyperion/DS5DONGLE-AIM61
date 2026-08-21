use anyhow::{Context, Result, bail};
use crc32fast::Hasher;
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "ds5dongle-macros/v3";
pub const DEVICE_FORMAT_VERSION: u8 = 3;
pub const PROFILE_COUNT: u8 = 4;
pub const MAX_CONTROLS: u8 = 19;
pub const MAX_MACROS: usize = 76;
pub const MAX_TOTAL_STEPS: usize = 1024;
pub const MAX_STEPS_PER_MACRO: usize = 512;
pub const MAX_DEVICE_BYTES: usize = 8 * 1024;
pub const MAX_ONCE_DURATION_MS: u64 = 10 * 60 * 1000;
const MACRO_REPORT_ID: u8 = 0xfe;
const FEATURE_REPORT_SIZE: usize = 64;
const MACRO_PAYLOAD_SIZE: usize = 63;
const SONY_VENDOR_ID: u16 = 0x054c;
const DUALSENSE_PRODUCT_IDS: [u16; 2] = [0x0ce6, 0x0df2];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerMode {
    #[default]
    Press,
    Release,
    LongPress {
        threshold_ms: u16,
    },
    DoubleTap {
        window_ms: u16,
    },
    Hold,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackMode {
    #[default]
    Once,
    Hold,
    Repeat {
        count: u16,
        gap_ms: u16,
    },
    LoopWhileHeld {
        gap_ms: u16,
    },
    ToggleLoop {
        gap_ms: u16,
    },
    Turbo {
        frequency_hz: u8,
        fixed_count: Option<u16>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchPoint {
    pub active: bool,
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroStep {
    pub duration_ms: u16,
    pub random_extra_ms: u16,
    pub digital_mask: Option<u32>,
    pub axes: Option<[u8; 6]>,
    pub touches: Option<[TouchPoint; 2]>,
}

impl Default for MacroStep {
    fn default() -> Self {
        Self {
            duration_ms: 50,
            random_extra_ms: 0,
            digital_mask: Some(0),
            axes: None,
            touches: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroDefinition {
    pub id: u32,
    pub name: String,
    pub profile: u8,
    pub trigger: u8,
    pub enabled: bool,
    pub trigger_passthrough: bool,
    pub trigger_mode: TriggerMode,
    pub playback_mode: PlaybackMode,
    pub steps: Vec<MacroStep>,
    pub recorded_on_device: bool,
}

impl Default for MacroDefinition {
    fn default() -> Self {
        Self {
            id: 1,
            name: "Macro 1".to_owned(),
            profile: 0,
            trigger: 0,
            enabled: true,
            trigger_passthrough: false,
            trigger_mode: TriggerMode::Press,
            playback_mode: PlaybackMode::Once,
            steps: vec![MacroStep::default()],
            recorded_on_device: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroSet {
    pub schema: String,
    pub generation: u32,
    pub active_profile: u8,
    pub globally_enabled: bool,
    pub sequence_enabled: bool,
    pub repeat_enabled: bool,
    pub macros: Vec<MacroDefinition>,
}

impl Default for MacroSet {
    fn default() -> Self {
        Self {
            schema: SCHEMA.to_owned(),
            generation: 0,
            active_profile: 0,
            globally_enabled: false,
            sequence_enabled: true,
            repeat_enabled: true,
            macros: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationSummary {
    pub macro_count: usize,
    pub step_count: usize,
    pub duration_ms: u64,
    pub compiled_bytes: usize,
    pub warnings: Vec<String>,
}

impl MacroSet {
    pub fn decode(blob: &[u8]) -> Result<Self> {
        if blob.len() < 32 || &blob[..4] != b"MAC3" || blob[4] != DEVICE_FORMAT_VERSION {
            bail!("device macro data is not MAC3/v{DEVICE_FORMAT_VERSION}");
        }
        if blob[5] != PROFILE_COUNT || blob[6] >= PROFILE_COUNT {
            bail!("device macro profile metadata is invalid");
        }
        let macro_count = usize::from(u16::from_le_bytes([blob[12], blob[13]]));
        let descriptor_len = usize::from(u16::from_le_bytes([blob[16], blob[17]]));
        let program_len = usize::from(u16::from_le_bytes([blob[18], blob[19]]));
        if macro_count > MAX_MACROS
            || descriptor_len != macro_count * 24
            || 32 + descriptor_len + program_len != blob.len()
        {
            bail!("device macro table lengths are inconsistent");
        }
        let mut hasher = Hasher::new();
        hasher.update(&blob[32..]);
        if hasher.finalize() != u32::from_le_bytes(blob[20..24].try_into().unwrap()) {
            bail!("device macro table CRC is invalid");
        }
        let program_base = 32 + descriptor_len;
        let mut macros = Vec::with_capacity(macro_count);
        for index in 0..macro_count {
            let d = &blob[32 + index * 24..32 + (index + 1) * 24];
            let id = u32::from_le_bytes(d[0..4].try_into().unwrap());
            let trigger_mode = match d[6] {
                0 => TriggerMode::Press,
                1 => TriggerMode::Release,
                2 => TriggerMode::LongPress {
                    threshold_ms: u16::from_le_bytes([d[10], d[11]]),
                },
                3 => TriggerMode::DoubleTap {
                    window_ms: u16::from_le_bytes([d[10], d[11]]),
                },
                4 => TriggerMode::Hold,
                tag => bail!("macro {id} uses unknown trigger mode {tag}"),
            };
            let repeat = u16::from_le_bytes([d[12], d[13]]);
            let gap = u16::from_le_bytes([d[14], d[15]]);
            let playback_mode = match d[7] {
                0 => PlaybackMode::Once,
                1 => PlaybackMode::Hold,
                2 => PlaybackMode::Repeat {
                    count: repeat,
                    gap_ms: gap,
                },
                3 => PlaybackMode::LoopWhileHeld { gap_ms: gap },
                4 => PlaybackMode::ToggleLoop { gap_ms: gap },
                5 => PlaybackMode::Turbo {
                    frequency_hz: u8::try_from(gap).context("turbo frequency is too large")?,
                    fixed_count: (repeat != 0).then_some(repeat),
                },
                tag => bail!("macro {id} uses unknown playback mode {tag}"),
            };
            let expected_steps = usize::from(u16::from_le_bytes([d[16], d[17]]));
            let offset = usize::from(u16::from_le_bytes([d[18], d[19]]));
            let length = usize::from(u16::from_le_bytes([d[20], d[21]]));
            if offset + length > program_len {
                bail!("macro {id} program exceeds the device table");
            }
            let mut cursor = program_base + offset;
            let end = cursor + length;
            let mut steps = Vec::with_capacity(expected_steps);
            let mut pending = MacroStep {
                duration_ms: 0,
                random_extra_ms: 0,
                digital_mask: None,
                axes: None,
                touches: None,
            };
            while cursor < end {
                let op = blob[cursor];
                cursor += 1;
                match op {
                    0x01 if cursor + 3 <= end => {
                        pending.digital_mask = Some(
                            u32::from(blob[cursor])
                                | (u32::from(blob[cursor + 1]) << 8)
                                | (u32::from(blob[cursor + 2]) << 16),
                        );
                        cursor += 3;
                    }
                    0x02 if cursor + 6 <= end => {
                        pending.axes = Some(blob[cursor..cursor + 6].try_into().unwrap());
                        cursor += 6;
                    }
                    0x03 if cursor + 10 <= end => {
                        let mut points = [TouchPoint::default(); 2];
                        for (point_index, point) in points.iter_mut().enumerate() {
                            let start = cursor + point_index * 5;
                            point.active = blob[start] != 0;
                            point.x = u16::from_le_bytes([blob[start + 1], blob[start + 2]]);
                            point.y = u16::from_le_bytes([blob[start + 3], blob[start + 4]]);
                        }
                        pending.touches = Some(points);
                        cursor += 10;
                    }
                    0x04 if cursor + 4 <= end => {
                        pending.duration_ms = u16::from_le_bytes([blob[cursor], blob[cursor + 1]]);
                        pending.random_extra_ms =
                            u16::from_le_bytes([blob[cursor + 2], blob[cursor + 3]]);
                        cursor += 4;
                        steps.push(pending);
                        pending = MacroStep {
                            duration_ms: 0,
                            random_extra_ms: 0,
                            digital_mask: None,
                            axes: None,
                            touches: None,
                        };
                    }
                    _ => bail!("macro {id} contains malformed bytecode at {cursor}"),
                }
            }
            if cursor != end || steps.len() != expected_steps {
                bail!("macro {id} step count does not match its descriptor");
            }
            macros.push(MacroDefinition {
                id,
                name: format!("Macro {id}"),
                profile: d[4],
                trigger: d[5],
                enabled: d[8] & 1 != 0,
                trigger_passthrough: d[8] & 2 != 0,
                trigger_mode,
                playback_mode,
                steps,
                recorded_on_device: d[8] & 4 != 0,
            });
        }
        let decoded = Self {
            schema: SCHEMA.to_owned(),
            generation: u32::from_le_bytes(blob[8..12].try_into().unwrap()),
            active_profile: blob[6],
            globally_enabled: blob[7] & 1 != 0,
            sequence_enabled: blob[7] & 2 != 0,
            repeat_enabled: blob[7] & 4 != 0,
            macros,
        };
        decoded.validate()?;
        Ok(decoded)
    }

    pub fn validate(&self) -> Result<ValidationSummary> {
        if self.schema != SCHEMA {
            bail!("unsupported macro schema {}", self.schema);
        }
        if self.active_profile >= PROFILE_COUNT {
            bail!("active profile must be in 1..={PROFILE_COUNT}");
        }
        if self.macros.len() > MAX_MACROS {
            bail!("macro count exceeds {MAX_MACROS}");
        }

        let mut total_steps = 0_usize;
        let mut total_duration = 0_u64;
        let mut warnings = Vec::new();
        for definition in &self.macros {
            if definition.profile >= PROFILE_COUNT {
                bail!("macro {} has an invalid profile", definition.name);
            }
            if definition.trigger >= MAX_CONTROLS {
                bail!("macro {} has an invalid trigger", definition.name);
            }
            if definition.trigger == 12 {
                if let TriggerMode::LongPress { threshold_ms } = definition.trigger_mode {
                    if threshold_ms >= 2000 {
                        bail!("PS long-press >= 2 seconds is reserved for emergency stop");
                    }
                }
            }
            match definition.trigger_mode {
                TriggerMode::LongPress { threshold_ms }
                    if !(200..=3000).contains(&threshold_ms) =>
                {
                    bail!("long-press threshold must be 200..=3000 ms")
                }
                TriggerMode::DoubleTap { window_ms } if !(100..=600).contains(&window_ms) => {
                    bail!("double-tap window must be 100..=600 ms")
                }
                _ => {}
            }
            if let PlaybackMode::Turbo { frequency_hz, .. } = definition.playback_mode
                && !(1..=50).contains(&frequency_hz)
            {
                bail!("turbo frequency must be 1..=50 Hz");
            }
            if definition.steps.is_empty() {
                bail!("macro {} has no steps", definition.name);
            }
            if definition.steps.len() > MAX_STEPS_PER_MACRO {
                bail!(
                    "macro {} exceeds {MAX_STEPS_PER_MACRO} steps",
                    definition.name
                );
            }
            total_steps += definition.steps.len();
            let duration = definition.steps.iter().try_fold(0_u64, |sum, step| {
                if step.duration_ms == 0 && step.random_extra_ms == 0 {
                    bail!("macro {} contains a zero-duration step", definition.name);
                }
                if let Some(mask) = step.digital_mask
                    && mask & !((1_u32 << MAX_CONTROLS) - 1) != 0
                {
                    bail!(
                        "macro {} contains invalid digital target bits",
                        definition.name
                    );
                }
                if let Some(points) = step.touches {
                    for point in points {
                        if point.x > 1919 || point.y > 1079 {
                            bail!("macro {} has an invalid touch coordinate", definition.name);
                        }
                    }
                }
                Ok(sum + u64::from(step.duration_ms) + u64::from(step.random_extra_ms))
            })?;
            if matches!(
                definition.playback_mode,
                PlaybackMode::Once | PlaybackMode::Hold
            ) && duration > MAX_ONCE_DURATION_MS
            {
                bail!(
                    "macro {} exceeds the 10-minute one-shot limit",
                    definition.name
                );
            }
            if definition.steps.len() > 256 {
                warnings.push(format!("{} uses more than 256 steps", definition.name));
            }
            total_duration += duration;
        }
        if total_steps > MAX_TOTAL_STEPS {
            bail!("total macro steps exceed {MAX_TOTAL_STEPS}");
        }
        let compiled = self.compile()?;
        Ok(ValidationSummary {
            macro_count: self.macros.len(),
            step_count: total_steps,
            duration_ms: total_duration,
            compiled_bytes: compiled.len(),
            warnings,
        })
    }

    pub fn compile(&self) -> Result<Vec<u8>> {
        let descriptor_bytes = self.macros.len() * 24;
        let mut programs = Vec::new();
        let mut descriptors = Vec::with_capacity(descriptor_bytes);
        let mut logical_steps = 0_u16;
        for definition in &self.macros {
            let start = programs.len();
            for step in &definition.steps {
                if let Some(mask) = step.digital_mask {
                    programs.push(0x01);
                    programs.extend_from_slice(&mask.to_le_bytes()[..3]);
                }
                if let Some(axes) = step.axes {
                    programs.push(0x02);
                    programs.extend_from_slice(&axes);
                }
                if let Some(points) = step.touches {
                    programs.push(0x03);
                    for point in points {
                        programs.push(u8::from(point.active));
                        programs.extend_from_slice(&point.x.to_le_bytes());
                        programs.extend_from_slice(&point.y.to_le_bytes());
                    }
                }
                programs.push(0x04);
                programs.extend_from_slice(&step.duration_ms.to_le_bytes());
                programs.extend_from_slice(&step.random_extra_ms.to_le_bytes());
            }
            logical_steps = logical_steps.saturating_add(definition.steps.len() as u16);
            let (trigger_tag, trigger_param) = match definition.trigger_mode {
                TriggerMode::Press => (0, 0),
                TriggerMode::Release => (1, 0),
                TriggerMode::LongPress { threshold_ms } => (2, threshold_ms),
                TriggerMode::DoubleTap { window_ms } => (3, window_ms),
                TriggerMode::Hold => (4, 0),
            };
            let (playback_tag, repeat, gap) = match definition.playback_mode {
                PlaybackMode::Once => (0, 1, 0),
                PlaybackMode::Hold => (1, 1, 0),
                PlaybackMode::Repeat { count, gap_ms } => (2, count, gap_ms),
                PlaybackMode::LoopWhileHeld { gap_ms } => (3, 0, gap_ms),
                PlaybackMode::ToggleLoop { gap_ms } => (4, 0, gap_ms),
                PlaybackMode::Turbo {
                    frequency_hz,
                    fixed_count,
                } => (5, fixed_count.unwrap_or(0), u16::from(frequency_hz)),
            };
            let program_len = programs.len() - start;
            if start > u16::MAX as usize || program_len > u16::MAX as usize {
                bail!("compiled macro program is too large");
            }
            descriptors.extend_from_slice(&definition.id.to_le_bytes());
            descriptors.push(definition.profile);
            descriptors.push(definition.trigger);
            descriptors.push(trigger_tag);
            descriptors.push(playback_tag);
            descriptors.push(
                u8::from(definition.enabled)
                    | (u8::from(definition.trigger_passthrough) << 1)
                    | (u8::from(definition.recorded_on_device) << 2),
            );
            descriptors.push(0);
            descriptors.extend_from_slice(&trigger_param.to_le_bytes());
            descriptors.extend_from_slice(&repeat.to_le_bytes());
            descriptors.extend_from_slice(&gap.to_le_bytes());
            descriptors.extend_from_slice(&(definition.steps.len() as u16).to_le_bytes());
            descriptors.extend_from_slice(&(start as u16).to_le_bytes());
            descriptors.extend_from_slice(&(program_len as u16).to_le_bytes());
            descriptors.extend_from_slice(&[0, 0]);
        }

        let total_len = 32 + descriptors.len() + programs.len();
        if total_len > MAX_DEVICE_BYTES {
            bail!("compiled macro data uses {total_len} bytes; device limit is {MAX_DEVICE_BYTES}");
        }
        let mut output = vec![0_u8; 32];
        output[0..4].copy_from_slice(b"MAC3");
        output[4] = DEVICE_FORMAT_VERSION;
        output[5] = PROFILE_COUNT;
        output[6] = self.active_profile;
        output[7] = u8::from(self.globally_enabled)
            | (u8::from(self.sequence_enabled) << 1)
            | (u8::from(self.repeat_enabled) << 2);
        output[8..12].copy_from_slice(&self.generation.to_le_bytes());
        output[12..14].copy_from_slice(&(self.macros.len() as u16).to_le_bytes());
        output[14..16].copy_from_slice(&logical_steps.to_le_bytes());
        output[16..18].copy_from_slice(&(descriptors.len() as u16).to_le_bytes());
        output[18..20].copy_from_slice(&(programs.len() as u16).to_le_bytes());
        output.extend_from_slice(&descriptors);
        output.extend_from_slice(&programs);
        let mut hasher = Hasher::new();
        hasher.update(&output[32..]);
        output[20..24].copy_from_slice(&hasher.finalize().to_le_bytes());
        Ok(output)
    }
}

#[cfg(windows)]
fn open_macro_device() -> Result<hidapi::HidDevice> {
    let api = hidapi::HidApi::new()?;
    let mut found = Vec::new();
    for info in api.device_list().filter(|info| {
        info.vendor_id() == SONY_VENDOR_ID && DUALSENSE_PRODUCT_IDS.contains(&info.product_id())
    }) {
        let Ok(device) = info.open_device(&api) else {
            continue;
        };
        let mut identity = [0_u8; FEATURE_REPORT_SIZE];
        identity[0] = 0xf8;
        let Ok(length) = device.get_feature_report(&mut identity) else {
            continue;
        };
        let payload = if identity.first() == Some(&0xf8) {
            &identity[1..length]
        } else {
            &identity[..length]
        };
        if std::str::from_utf8(payload)
            .ok()
            .is_some_and(|value| value.contains("|standard") || value.contains("|diagnostic"))
        {
            found.push(device);
        }
    }
    match found.len() {
        0 => bail!("未找到支持宏协议 v3 的 DS5DONGLE-AIM61（需要固件 v3.6.0 或更新版本）"),
        1 => Ok(found.remove(0)),
        count => bail!("检测到 {count} 个 M61 设备，请只连接一个后重试"),
    }
}

#[cfg(windows)]
fn send_macro_frame(device: &hidapi::HidDevice, frame: &[u8; MACRO_PAYLOAD_SIZE]) -> Result<()> {
    let mut report = [0_u8; FEATURE_REPORT_SIZE];
    report[0] = MACRO_REPORT_ID;
    report[1..].copy_from_slice(frame);
    device
        .send_feature_report(&report)
        .context("写入设备宏 Feature Report 失败")?;
    Ok(())
}

#[cfg(windows)]
fn macro_frame(command: u8) -> [u8; MACRO_PAYLOAD_SIZE] {
    let mut frame = [0_u8; MACRO_PAYLOAD_SIZE];
    frame[..4].copy_from_slice(&[b'M', b'C', DEVICE_FORMAT_VERSION, command]);
    frame
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeviceMacroStatus {
    state: u8,
    error: u8,
    generation: u32,
    length: u16,
    offset: u16,
    chunk_length: u8,
}

#[cfg(windows)]
fn read_status(device: &hidapi::HidDevice) -> Result<(DeviceMacroStatus, [u8; 46])> {
    let mut report = [0_u8; FEATURE_REPORT_SIZE];
    report[0] = MACRO_REPORT_ID;
    let length = device
        .get_feature_report(&mut report)
        .context("读取设备宏状态失败")?;
    let payload = if length == FEATURE_REPORT_SIZE && report[0] == MACRO_REPORT_ID {
        &report[1..]
    } else if length == MACRO_PAYLOAD_SIZE {
        &report[..length]
    } else {
        bail!("设备宏状态长度异常：{length} 字节");
    };
    if payload[..3] != [b'M', b'C', DEVICE_FORMAT_VERSION] {
        bail!("设备未提供宏协议 v3");
    }
    let chunk_length = payload[16];
    if chunk_length > 46 {
        bail!("设备返回了过长的宏数据块");
    }
    let mut chunk = [0_u8; 46];
    chunk.copy_from_slice(&payload[17..63]);
    Ok((
        DeviceMacroStatus {
            state: payload[3],
            error: payload[4],
            generation: u32::from_le_bytes(payload[8..12].try_into().unwrap()),
            length: u16::from_le_bytes(payload[12..14].try_into().unwrap()),
            offset: u16::from_le_bytes(payload[14..16].try_into().unwrap()),
            chunk_length,
        },
        chunk,
    ))
}

#[cfg(windows)]
fn wait_for_state(device: &hidapi::HidDevice, expected: &[u8]) -> Result<DeviceMacroStatus> {
    for _ in 0..80 {
        let (status, _) = read_status(device)?;
        if status.state == 3 {
            bail!("设备拒绝宏命令，错误码 {}", status.error);
        }
        if expected.contains(&status.state) {
            return Ok(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    bail!("等待设备宏命令完成超时")
}

#[cfg(windows)]
pub fn read_device_macro_set() -> Result<MacroSet> {
    let device = open_macro_device()?;
    let (initial, _) = read_status(&device)?;
    if initial.length == 0 {
        return Ok(MacroSet {
            generation: initial.generation,
            ..MacroSet::default()
        });
    }
    let mut blob = Vec::with_capacity(usize::from(initial.length));
    let mut offset = 0_u16;
    while offset < initial.length {
        let mut select = macro_frame(0x05);
        select[4..6].copy_from_slice(&offset.to_le_bytes());
        send_macro_frame(&device, &select)?;
        std::thread::sleep(std::time::Duration::from_millis(8));
        let (status, chunk) = read_status(&device)?;
        if status.offset != offset || status.chunk_length == 0 {
            bail!("设备宏读取在偏移 {offset} 处未前进");
        }
        blob.extend_from_slice(&chunk[..usize::from(status.chunk_length)]);
        offset = offset.saturating_add(u16::from(status.chunk_length));
    }
    blob.truncate(usize::from(initial.length));
    let decoded = MacroSet::decode(&blob)?;
    if decoded.generation != initial.generation {
        bail!("读取宏期间设备数据发生变化，请重试");
    }
    Ok(decoded)
}

#[cfg(not(windows))]
pub fn read_device_macro_set() -> Result<MacroSet> {
    bail!("设备宏仅支持 Windows")
}

#[cfg(windows)]
pub fn write_device_macro_set(set: &MacroSet) -> Result<MacroSet> {
    set.validate()?;
    let device = open_macro_device()?;
    let (current, _) = read_status(&device)?;
    if current.generation != set.generation {
        bail!(
            "设备宏已被其他操作修改（设备代数 {}，当前草稿代数 {}），请先重新读取",
            current.generation,
            set.generation
        );
    }
    let blob = set.compile()?;
    let new_generation = current.generation.checked_add(1).context("宏代数已用尽")?;
    let mut hasher = Hasher::new();
    hasher.update(&blob);
    let mut begin = macro_frame(0x01);
    begin[4..8].copy_from_slice(&current.generation.to_le_bytes());
    begin[8..12].copy_from_slice(&new_generation.to_le_bytes());
    begin[12..14].copy_from_slice(&(blob.len() as u16).to_le_bytes());
    begin[14..18].copy_from_slice(&hasher.finalize().to_le_bytes());
    send_macro_frame(&device, &begin)?;
    wait_for_state(&device, &[1])?;

    for (index, chunk) in blob.chunks(56).enumerate() {
        let offset = index * 56;
        let mut frame = macro_frame(0x02);
        frame[4..6].copy_from_slice(&(offset as u16).to_le_bytes());
        frame[6] = chunk.len() as u8;
        frame[7..7 + chunk.len()].copy_from_slice(chunk);
        send_macro_frame(&device, &frame)?;
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    send_macro_frame(&device, &macro_frame(0x03))?;
    let committed = wait_for_state(&device, &[2])?;
    if committed.generation != new_generation || usize::from(committed.length) != blob.len() {
        bail!("设备宏提交后的长度或代数验证失败");
    }
    read_device_macro_set()
}

#[cfg(not(windows))]
pub fn write_device_macro_set(_set: &MacroSet) -> Result<MacroSet> {
    bail!("设备宏仅支持 Windows")
}

pub fn export_json(set: &MacroSet) -> Result<String> {
    set.validate()?;
    Ok(serde_json::to_string_pretty(set)?)
}

pub fn import_json(source: &str) -> Result<MacroSet> {
    let set: MacroSet = serde_json::from_str(source)?;
    set.validate()?;
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_macro_compiles_to_bounded_device_format() {
        let mut set = MacroSet::default();
        set.macros.push(MacroDefinition::default());
        let summary = set.validate().unwrap();
        assert_eq!(summary.macro_count, 1);
        assert_eq!(summary.step_count, 1);
        let binary = set.compile().unwrap();
        assert_eq!(&binary[..4], b"MAC3");
        assert!(binary.len() < MAX_DEVICE_BYTES);
    }

    #[test]
    fn ps_emergency_hold_is_reserved() {
        let mut set = MacroSet::default();
        let mut definition = MacroDefinition::default();
        definition.trigger = 12;
        definition.trigger_mode = TriggerMode::LongPress { threshold_ms: 2000 };
        set.macros.push(definition);
        assert!(set.validate().is_err());
    }

    #[test]
    fn turbo_is_limited_to_fifty_hertz() {
        let mut set = MacroSet::default();
        let mut definition = MacroDefinition::default();
        definition.playback_mode = PlaybackMode::Turbo {
            frequency_hz: 51,
            fixed_count: None,
        };
        set.macros.push(definition);
        assert!(set.validate().is_err());
    }

    #[test]
    fn compiled_macros_round_trip_through_device_format() {
        let mut set = MacroSet::default();
        set.generation = 17;
        set.globally_enabled = true;
        let mut definition = MacroDefinition::default();
        definition.id = 42;
        definition.trigger_mode = TriggerMode::DoubleTap { window_ms: 250 };
        definition.playback_mode = PlaybackMode::Repeat {
            count: 3,
            gap_ms: 40,
        };
        definition.steps[0].axes = Some([127, 0, 127, 255, 20, 30]);
        set.macros.push(definition);
        let decoded = MacroSet::decode(&set.compile().unwrap()).unwrap();
        assert_eq!(decoded.generation, 17);
        assert_eq!(decoded.macros[0].id, 42);
        assert_eq!(decoded.macros[0].steps[0].axes, set.macros[0].steps[0].axes);
        assert_eq!(decoded.macros[0].playback_mode, set.macros[0].playback_mode);
    }
}
