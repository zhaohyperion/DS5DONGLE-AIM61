use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{DUALSENSE_PRODUCT_IDS, SONY_VENDOR_ID};

const INPUT_REPORT_ID: u8 = 0x01;
const OUTPUT_REPORT_ID: u8 = 0x02;
const WAVEOUT_FEATURE_REPORT_ID: u8 = 0x80;
const REPORT_BYTES: usize = 64;
const OUTPUT_PAYLOAD_BYTES: usize = 47;
const FEATURE_PAYLOAD_BYTES: usize = 63;

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchPoint {
    pub active: bool,
    pub id: u8,
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputState {
    pub lx: u8,
    pub ly: u8,
    pub rx: u8,
    pub ry: u8,
    pub l2: u8,
    pub r2: u8,
    pub dpad: u8,
    pub square: bool,
    pub cross: bool,
    pub circle: bool,
    pub triangle: bool,
    pub l1: bool,
    pub r1: bool,
    pub l2_button: bool,
    pub r2_button: bool,
    pub create: bool,
    pub options: bool,
    pub l3: bool,
    pub r3: bool,
    pub ps: bool,
    pub touchpad_click: bool,
    pub mute: bool,
    pub gyro_x: i16,
    pub gyro_y: i16,
    pub gyro_z: i16,
    pub accel_x: i16,
    pub accel_y: i16,
    pub accel_z: i16,
    pub touch: [TouchPoint; 2],
    pub battery_percent: Option<u8>,
    pub report_count: u64,
    pub report_rate_hz: f32,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugMetrics {
    pub sample_count: u64,
    pub elapsed_ms: u64,
    pub report_rate_hz: f32,
    pub average_interval_ms: f32,
    pub minimum_interval_ms: f32,
    pub p95_interval_ms: f32,
    pub p99_interval_ms: f32,
    pub maximum_interval_ms: f32,
    pub jitter_stddev_ms: f32,
    pub gaps_over_5ms: u64,
    pub gaps_over_10ms: u64,
    pub stress_output_reports: u64,
}

const METRIC_HISTOGRAM_STEP_US: u32 = 10;
const METRIC_HISTOGRAM_MAX_US: u32 = 200_000;
const METRIC_HISTOGRAM_BUCKETS: usize =
    (METRIC_HISTOGRAM_MAX_US / METRIC_HISTOGRAM_STEP_US) as usize + 1;

struct DebugAccumulator {
    samples: u64,
    sum_us: f64,
    sum_squares_us: f64,
    minimum_us: u32,
    maximum_us: u32,
    gaps_over_5ms: u64,
    gaps_over_10ms: u64,
    histogram: Vec<u64>,
}

impl DebugAccumulator {
    fn new() -> Self {
        Self {
            samples: 0,
            sum_us: 0.0,
            sum_squares_us: 0.0,
            minimum_us: u32::MAX,
            maximum_us: 0,
            gaps_over_5ms: 0,
            gaps_over_10ms: 0,
            histogram: vec![0; METRIC_HISTOGRAM_BUCKETS],
        }
    }

    fn reset(&mut self) {
        self.samples = 0;
        self.sum_us = 0.0;
        self.sum_squares_us = 0.0;
        self.minimum_us = u32::MAX;
        self.maximum_us = 0;
        self.gaps_over_5ms = 0;
        self.gaps_over_10ms = 0;
        self.histogram.fill(0);
    }

    fn record(&mut self, interval_us: u32) {
        let value = interval_us as f64;
        self.samples += 1;
        self.sum_us += value;
        self.sum_squares_us += value * value;
        self.minimum_us = self.minimum_us.min(interval_us);
        self.maximum_us = self.maximum_us.max(interval_us);
        self.gaps_over_5ms += u64::from(interval_us > 5_000);
        self.gaps_over_10ms += u64::from(interval_us > 10_000);
        let bucket = (interval_us / METRIC_HISTOGRAM_STEP_US)
            .min((METRIC_HISTOGRAM_BUCKETS - 1) as u32) as usize;
        self.histogram[bucket] += 1;
    }

    fn percentile_us(&self, numerator: u64) -> u32 {
        if self.samples == 0 {
            return 0;
        }
        let target = (self.samples * numerator + 99) / 100;
        let mut cumulative = 0_u64;
        for (index, count) in self.histogram.iter().enumerate() {
            cumulative += count;
            if cumulative >= target {
                if index == self.histogram.len() - 1 {
                    return self.maximum_us;
                }
                return ((index as u32 + 1) * METRIC_HISTOGRAM_STEP_US).min(self.maximum_us);
            }
        }
        self.maximum_us
    }

    fn metrics(&self, elapsed: Duration, stress_output_reports: u64) -> DebugMetrics {
        if self.samples == 0 {
            return DebugMetrics {
                elapsed_ms: elapsed.as_millis() as u64,
                stress_output_reports,
                ..DebugMetrics::default()
            };
        }
        let count = self.samples as f64;
        let mean = self.sum_us / count;
        let variance = (self.sum_squares_us / count - mean * mean).max(0.0);
        DebugMetrics {
            sample_count: self.samples,
            elapsed_ms: elapsed.as_millis() as u64,
            report_rate_hz: if mean > 0.0 {
                (1_000_000.0 / mean) as f32
            } else {
                0.0
            },
            average_interval_ms: (mean / 1000.0) as f32,
            minimum_interval_ms: self.minimum_us as f32 / 1000.0,
            p95_interval_ms: self.percentile_us(95) as f32 / 1000.0,
            p99_interval_ms: self.percentile_us(99) as f32 / 1000.0,
            maximum_interval_ms: self.maximum_us as f32 / 1000.0,
            jitter_stddev_ms: variance.sqrt() as f32 / 1000.0,
            gaps_over_5ms: self.gaps_over_5ms,
            gaps_over_10ms: self.gaps_over_10ms,
            stress_output_reports,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerPreset {
    Off,
    Resistance,
    Weapon,
    Automatic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerAudioTarget {
    Speaker,
    Headphone,
}

#[derive(Clone, Debug)]
pub struct OutputState {
    pub rumble_right: u8,
    pub rumble_left: u8,
    pub lightbar_enabled: bool,
    pub lightbar_rgb: [u8; 3],
    pub player_leds: u8,
    pub mute_led: u8,
    pub left_trigger: TriggerPreset,
    pub right_trigger: TriggerPreset,
}

impl Default for OutputState {
    fn default() -> Self {
        Self {
            rumble_right: 0,
            rumble_left: 0,
            lightbar_enabled: false,
            lightbar_rgb: [0, 80, 255],
            player_leds: 0,
            mute_led: 0,
            left_trigger: TriggerPreset::Off,
            right_trigger: TriggerPreset::Off,
        }
    }
}

enum TestCommand {
    Output(OutputState),
    StartControllerTone(ControllerAudioTarget),
    StopControllerTone,
    StopAll,
    ResetMetrics,
    SetMetricsActive(bool),
    SetStressActive { active: bool, rate_hz: u32 },
    Shutdown,
}

#[derive(Debug)]
pub enum TestEvent {
    Connected(String),
    Input(InputState),
    Metrics(DebugMetrics),
    OutputSent,
    ControllerToneChanged(Option<ControllerAudioTarget>),
    Error(String),
    Stopped,
}

pub struct TestSession {
    commands: Sender<TestCommand>,
    pub events: Receiver<TestEvent>,
}

impl TestSession {
    pub fn start() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        thread::spawn(move || run_hid_session(command_rx, event_tx));
        Self {
            commands: command_tx,
            events: event_rx,
        }
    }

    pub fn set_output(&self, state: OutputState) -> Result<()> {
        self.commands
            .send(TestCommand::Output(state))
            .context("device test worker has stopped")
    }

    pub fn stop_all(&self) -> Result<()> {
        self.commands
            .send(TestCommand::StopAll)
            .context("device test worker has stopped")
    }

    pub fn start_controller_tone(&self, target: ControllerAudioTarget) -> Result<()> {
        self.commands
            .send(TestCommand::StartControllerTone(target))
            .context("device test worker has stopped")
    }

    pub fn stop_controller_tone(&self) -> Result<()> {
        self.commands
            .send(TestCommand::StopControllerTone)
            .context("device test worker has stopped")
    }

    pub fn reset_metrics(&self) -> Result<()> {
        self.commands
            .send(TestCommand::ResetMetrics)
            .context("device test worker has stopped")
    }

    pub fn set_metrics_active(&self, active: bool) -> Result<()> {
        self.commands
            .send(TestCommand::SetMetricsActive(active))
            .context("device test worker has stopped")
    }

    pub fn set_stress_active(&self, active: bool, rate_hz: u32) -> Result<()> {
        self.commands
            .send(TestCommand::SetStressActive {
                active,
                rate_hz: rate_hz.clamp(1, 50),
            })
            .context("device test worker has stopped")
    }

    pub fn shutdown(&self) {
        let _ = self.commands.send(TestCommand::Shutdown);
    }
}

impl Drop for TestSession {
    fn drop(&mut self) {
        let _ = self.commands.send(TestCommand::Shutdown);
    }
}

#[cfg(windows)]
struct HidOutputReset<'a> {
    device: &'a hidapi::HidDevice,
    edge_report: bool,
    audio_restore: (u8, u8),
}

#[cfg(windows)]
impl Drop for HidOutputReset<'_> {
    fn drop(&mut self) {
        let _ = send_waveout_control(self.device, false);
        let report = pad_edge_report(
            build_audio_restore_report(&OutputState::default(), self.audio_restore),
            self.edge_report,
        );
        let _ = self.device.write(&report);
    }
}

#[cfg(windows)]
fn run_hid_session(commands: Receiver<TestCommand>, events: Sender<TestEvent>) {
    let result = (|| -> Result<()> {
        let api = hidapi::HidApi::new().context("failed to initialize Windows HID access")?;
        let info = api
            .device_list()
            .find(|info| {
                info.vendor_id() == SONY_VENDOR_ID
                    && DUALSENSE_PRODUCT_IDS.contains(&info.product_id())
                    && info.usage_page() == 0x01
                    && info.usage() == 0x05
            })
            .context("no running DS5DONGLE-AIM61 gamepad interface was found")?;
        let name = info
            .product_string()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("DS5DONGLE-AIM61")
            .to_owned();
        let edge_report = info.product_id() == 0x0df2;
        let device = info
            .open_device(&api)
            .context("unable to open the DS5 gamepad HID interface; close Steam, DS4Windows and other controller tools")?;
        let audio_restore = read_configured_audio_volumes(&device).unwrap_or((100, 100));
        let _output_reset = HidOutputReset {
            device: &device,
            edge_report,
            audio_restore,
        };
        let _ = events.send(TestEvent::Connected(name));

        let mut report_count = 0_u64;
        let mut rate_count = 0_u64;
        let mut report_rate_hz = 0.0_f32;
        let mut rate_started = Instant::now();
        let mut last_ui_emit = Instant::now() - Duration::from_millis(20);
        let mut metrics_active = false;
        let mut metrics_started = Instant::now();
        let mut metrics_last_report: Option<Instant> = None;
        let mut metrics_last_emit = Instant::now();
        let mut metrics = DebugAccumulator::new();
        let mut stress_active = false;
        let mut stress_started = Instant::now();
        let mut stress_last_output = Instant::now();
        let mut stress_interval = Duration::from_millis(50);
        let mut stress_output_reports = 0_u64;
        let mut output_state = OutputState::default();
        let mut controller_tone = None;
        let mut report = [0_u8; REPORT_BYTES];
        loop {
            loop {
                match commands.try_recv() {
                    Ok(TestCommand::Output(state)) => {
                        output_state = state;
                        let report = pad_edge_report(
                            build_output_report_with_audio(&output_state, controller_tone),
                            edge_report,
                        );
                        device
                            .write(&report)
                            .context("failed to write DS5 output report")?;
                        let _ = events.send(TestEvent::OutputSent);
                    }
                    Ok(TestCommand::StartControllerTone(target)) => {
                        if controller_tone.is_some() {
                            send_waveout_control(&device, false)
                                .context("failed to stop the previous DualSense tone")?;
                        }
                        let report = pad_edge_report(
                            build_output_report_with_audio(&output_state, Some(target)),
                            edge_report,
                        );
                        device
                            .write(&report)
                            .context("failed to select the DualSense audio output")?;
                        thread::sleep(Duration::from_millis(40));
                        send_waveout_setup(&device, target)
                            .context("failed to configure the DualSense 1 kHz tone")?;
                        send_waveout_control(&device, true)
                            .context("failed to start the DualSense 1 kHz tone")?;
                        controller_tone = Some(target);
                        let _ = events.send(TestEvent::ControllerToneChanged(controller_tone));
                    }
                    Ok(TestCommand::StopControllerTone) => {
                        send_waveout_control(&device, false)
                            .context("failed to stop the DualSense 1 kHz tone")?;
                        controller_tone = None;
                        let report = pad_edge_report(
                            build_audio_restore_report(&output_state, audio_restore),
                            edge_report,
                        );
                        device
                            .write(&report)
                            .context("failed to restore the DualSense audio output")?;
                        let _ = events.send(TestEvent::ControllerToneChanged(None));
                    }
                    Ok(TestCommand::StopAll) => {
                        if controller_tone.take().is_some() {
                            send_waveout_control(&device, false)
                                .context("failed to stop the DualSense 1 kHz tone")?;
                            let _ = events.send(TestEvent::ControllerToneChanged(None));
                        }
                        output_state = OutputState::default();
                        let report = pad_edge_report(
                            build_audio_restore_report(&output_state, audio_restore),
                            edge_report,
                        );
                        device
                            .write(&report)
                            .context("failed to stop DS5 test outputs")?;
                        let _ = events.send(TestEvent::OutputSent);
                    }
                    Ok(TestCommand::ResetMetrics) => {
                        metrics.reset();
                        stress_output_reports = 0;
                        metrics_started = Instant::now();
                        metrics_last_report = None;
                        metrics_last_emit = Instant::now();
                        let _ = events.send(TestEvent::Metrics(DebugMetrics::default()));
                    }
                    Ok(TestCommand::SetMetricsActive(active)) => {
                        if metrics_active && !active {
                            let _ = events.send(TestEvent::Metrics(
                                metrics.metrics(metrics_started.elapsed(), stress_output_reports),
                            ));
                        }
                        metrics_active = active;
                        if active {
                            metrics_started = Instant::now();
                            metrics_last_report = None;
                        }
                    }
                    Ok(TestCommand::SetStressActive { active, rate_hz }) => {
                        stress_active = active;
                        stress_interval = Duration::from_millis(1000 / rate_hz.max(1) as u64);
                        stress_started = Instant::now();
                        stress_last_output = Instant::now() - stress_interval;
                        if !active {
                            let report = pad_edge_report(build_stop_report(), edge_report);
                            device
                                .write(&report)
                                .context("failed to stop DS5 stress-test outputs")?;
                        }
                    }
                    Ok(TestCommand::Shutdown) => {
                        if controller_tone.is_some() {
                            let _ = send_waveout_control(&device, false);
                        }
                        let report = pad_edge_report(build_stop_report(), edge_report);
                        let _ = device.write(&report);
                        let _ = events.send(TestEvent::Stopped);
                        return Ok(());
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if controller_tone.is_some() {
                            let _ = send_waveout_control(&device, false);
                        }
                        let report = pad_edge_report(build_stop_report(), edge_report);
                        let _ = device.write(&report);
                        return Ok(());
                    }
                }
            }

            if stress_active && stress_last_output.elapsed() >= stress_interval {
                let state = stress_output_state(stress_started.elapsed());
                let report = pad_edge_report(
                    build_output_report_with_audio(&state, controller_tone),
                    edge_report,
                );
                device
                    .write(&report)
                    .context("failed to write DS5 stress-test output report")?;
                stress_output_reports += 1;
                stress_last_output = Instant::now();
            }

            match device.read_timeout(&mut report, 16) {
                Ok(0) => {}
                Ok(length) => {
                    if let Some(mut state) = parse_input_report(&report[..length]) {
                        let now = Instant::now();
                        if metrics_active {
                            if let Some(previous) = metrics_last_report {
                                metrics.record(
                                    now.duration_since(previous)
                                        .as_micros()
                                        .min(u32::MAX as u128)
                                        as u32,
                                );
                            }
                            metrics_last_report = Some(now);
                            if metrics_last_emit.elapsed() >= Duration::from_millis(250) {
                                let _ = events.send(TestEvent::Metrics(
                                    metrics
                                        .metrics(metrics_started.elapsed(), stress_output_reports),
                                ));
                                metrics_last_emit = Instant::now();
                            }
                        }
                        report_count += 1;
                        rate_count += 1;
                        let rate_elapsed = rate_started.elapsed();
                        if rate_elapsed >= Duration::from_millis(500) {
                            report_rate_hz = rate_count as f32 / rate_elapsed.as_secs_f32();
                            rate_count = 0;
                            rate_started = Instant::now();
                        }
                        state.report_count = report_count;
                        state.report_rate_hz = report_rate_hz;
                        // Read every packet for an honest rate measurement, but
                        // publish only the latest state at approximately 60 Hz.
                        if last_ui_emit.elapsed() >= Duration::from_millis(16) {
                            let _ = events.send(TestEvent::Input(state));
                            last_ui_emit = Instant::now();
                        }
                    }
                }
                Err(error) => bail!("failed to read DS5 input report: {error}"),
            }
        }
    })();
    if let Err(error) = result {
        let _ = events.send(TestEvent::Error(format!("{error:#}")));
    }
}

#[cfg(test)]
fn calculate_metrics(intervals_us: &[u32], elapsed: Duration) -> DebugMetrics {
    let mut accumulator = DebugAccumulator::new();
    for interval in intervals_us {
        accumulator.record(*interval);
    }
    accumulator.metrics(elapsed, 0)
}

fn stress_output_state(elapsed: Duration) -> OutputState {
    match (elapsed.as_secs() / 5) % 4 {
        0 => OutputState {
            rumble_right: 45,
            rumble_left: 70,
            lightbar_enabled: true,
            lightbar_rgb: [0, 120, 255],
            player_leds: 0x05,
            left_trigger: TriggerPreset::Resistance,
            right_trigger: TriggerPreset::Off,
            ..OutputState::default()
        },
        1 => OutputState {
            rumble_right: 95,
            rumble_left: 55,
            lightbar_enabled: true,
            lightbar_rgb: [40, 220, 100],
            player_leds: 0x0a,
            left_trigger: TriggerPreset::Off,
            right_trigger: TriggerPreset::Resistance,
            ..OutputState::default()
        },
        2 => OutputState {
            rumble_right: 60,
            rumble_left: 105,
            lightbar_enabled: true,
            lightbar_rgb: [190, 50, 255],
            player_leds: 0x11,
            left_trigger: TriggerPreset::Resistance,
            right_trigger: TriggerPreset::Resistance,
            ..OutputState::default()
        },
        /* Five seconds of complete actuator release limits motor heating,
         * battery drain and adaptive-trigger mechanical duty during long runs.
         * Zero-state reports continue to exercise the output transport. */
        _ => OutputState::default(),
    }
}

#[cfg(not(windows))]
fn run_hid_session(_commands: Receiver<TestCommand>, events: Sender<TestEvent>) {
    let _ = events.send(TestEvent::Error(
        "the device test center is available on Windows only".to_owned(),
    ));
}

fn parse_input_report(source: &[u8]) -> Option<InputState> {
    let payload = if source.first().copied() == Some(INPUT_REPORT_ID) {
        source.get(1..)?
    } else {
        source
    };
    if payload.len() < 53 {
        return None;
    }
    let buttons0 = payload[7];
    let buttons1 = payload[8];
    let buttons2 = payload[9];
    let battery = payload[52] & 0x0f;
    Some(InputState {
        lx: payload[0],
        ly: payload[1],
        rx: payload[2],
        ry: payload[3],
        l2: payload[4],
        r2: payload[5],
        dpad: buttons0 & 0x0f,
        square: buttons0 & 0x10 != 0,
        cross: buttons0 & 0x20 != 0,
        circle: buttons0 & 0x40 != 0,
        triangle: buttons0 & 0x80 != 0,
        l1: buttons1 & 0x01 != 0,
        r1: buttons1 & 0x02 != 0,
        l2_button: buttons1 & 0x04 != 0,
        r2_button: buttons1 & 0x08 != 0,
        create: buttons1 & 0x10 != 0,
        options: buttons1 & 0x20 != 0,
        l3: buttons1 & 0x40 != 0,
        r3: buttons1 & 0x80 != 0,
        ps: buttons2 & 0x01 != 0,
        touchpad_click: buttons2 & 0x02 != 0,
        mute: buttons2 & 0x04 != 0,
        gyro_x: read_i16(payload, 15),
        gyro_z: read_i16(payload, 17),
        gyro_y: read_i16(payload, 19),
        accel_x: read_i16(payload, 21),
        accel_y: read_i16(payload, 23),
        accel_z: read_i16(payload, 25),
        touch: [parse_touch(payload, 32), parse_touch(payload, 36)],
        battery_percent: (battery <= 10).then_some((battery * 10).min(100)),
        report_count: 0,
        report_rate_hz: 0.0,
    })
}

fn read_i16(source: &[u8], offset: usize) -> i16 {
    source
        .get(offset..offset + 2)
        .and_then(|bytes| bytes.try_into().ok())
        .map(i16::from_le_bytes)
        .unwrap_or_default()
}

fn parse_touch(source: &[u8], offset: usize) -> TouchPoint {
    let Some(bytes) = source.get(offset..offset + 4) else {
        return TouchPoint::default();
    };
    TouchPoint {
        active: bytes[0] & 0x80 == 0,
        id: bytes[0] & 0x7f,
        x: bytes[1] as u16 | ((bytes[2] as u16 & 0x0f) << 8),
        y: ((bytes[2] as u16 & 0xf0) >> 4) | ((bytes[3] as u16) << 4),
    }
}

fn build_output_report(state: &OutputState) -> Vec<u8> {
    build_output_report_with_audio(state, None)
}

fn build_output_report_with_audio(
    state: &OutputState,
    audio_target: Option<ControllerAudioTarget>,
) -> Vec<u8> {
    let mut report = vec![0_u8; OUTPUT_PAYLOAD_BYTES + 1];
    report[0] = OUTPUT_REPORT_ID;
    let data = &mut report[1..];

    // Always claim every safe, transient control so moving a slider back to
    // zero also clears the controller's state from the previous test frame.
    data[0] = 0x0f;
    data[1] = 0x15;
    data[2] = state.rumble_right;
    data[3] = state.rumble_left;
    match audio_target {
        Some(ControllerAudioTarget::Speaker) => {
            data[0] |= 0xa0;
            data[5] = 220;
            data[7] = 48;
        }
        Some(ControllerAudioTarget::Headphone) => {
            data[0] |= 0x90;
            data[4] = 150;
        }
        None => {}
    }
    data[8] = state.mute_led.min(2);
    data[38] = 0x03;
    data[39] = 0x02;
    data[41] = 0x02;
    data[42] = 0;
    data[43] = state.player_leds & 0x1f;
    encode_trigger(data, 10, state.right_trigger);
    encode_trigger(data, 21, state.left_trigger);
    if state.lightbar_enabled {
        data[44..47].copy_from_slice(&state.lightbar_rgb);
    }
    report
}

fn encode_trigger(data: &mut [u8], offset: usize, preset: TriggerPreset) {
    data[offset..offset + 8].fill(0);
    match preset {
        TriggerPreset::Off => {}
        TriggerPreset::Resistance => {
            data[offset] = 0x01;
            data[offset + 1] = 40;
            data[offset + 2] = 230;
        }
        TriggerPreset::Weapon => {
            data[offset] = 0x02;
            data[offset + 1] = 15;
            data[offset + 2] = 100;
            data[offset + 3] = 255;
        }
        TriggerPreset::Automatic => {
            data[offset] = 0x06;
            data[offset + 1] = 10;
            data[offset + 2] = 255;
            data[offset + 3] = 20;
        }
    }
}

fn build_stop_report() -> Vec<u8> {
    let mut report = vec![0_u8; OUTPUT_PAYLOAD_BYTES + 1];
    report[0] = OUTPUT_REPORT_ID;
    let data = &mut report[1..];
    data[0] = 0x0f;
    data[1] = 0x15;
    data[8] = 0;
    data[10] = 0;
    data[21] = 0;
    data[38] = 0x03;
    data[39] = 0x02;
    data[41] = 0x02;
    report
}

fn build_audio_restore_report(state: &OutputState, audio_restore: (u8, u8)) -> Vec<u8> {
    let mut report = build_output_report(state);
    let data = &mut report[1..];
    data[0] |= 0xb0;
    data[4] = audio_restore.1;
    data[5] = audio_restore.0;
    data[7] = 0;
    report
}

#[cfg(windows)]
fn read_configured_audio_volumes(device: &hidapi::HidDevice) -> Option<(u8, u8)> {
    let mut report = [0_u8; REPORT_BYTES];
    report[0] = 0xf7;
    let length = device.get_feature_report(&mut report).ok()?;
    (length >= 8 && report[0] == 0xf7).then_some((report[6], report[7]))
}

#[cfg(windows)]
fn send_waveout_setup(device: &hidapi::HidDevice, target: ControllerAudioTarget) -> Result<()> {
    send_waveout_feature(device, &waveout_setup_payload(target))
}

fn waveout_setup_payload(target: ControllerAudioTarget) -> Vec<u8> {
    let mut waveout = [0_u8; 20];
    match target {
        ControllerAudioTarget::Speaker => waveout[2] = 8,
        ControllerAudioTarget::Headphone => {
            waveout[4] = 4;
            waveout[6] = 6;
        }
    }
    let mut payload = Vec::with_capacity(22);
    payload.extend_from_slice(&[6, 4]);
    payload.extend_from_slice(&waveout);
    payload
}

#[cfg(windows)]
fn send_waveout_control(device: &hidapi::HidDevice, enabled: bool) -> Result<()> {
    send_waveout_feature(device, &waveout_control_payload(enabled))
}

fn waveout_control_payload(enabled: bool) -> [u8; 5] {
    [6, 2, u8::from(enabled), 1, 0]
}

#[cfg(windows)]
fn send_waveout_feature(device: &hidapi::HidDevice, payload: &[u8]) -> Result<()> {
    let report = build_waveout_feature_report(payload);
    device
        .send_feature_report(&report)
        .context("failed to write DualSense Feature Report 0x80")?;
    Ok(())
}

fn build_waveout_feature_report(payload: &[u8]) -> Vec<u8> {
    let mut report = vec![0_u8; FEATURE_PAYLOAD_BYTES + 1];
    report[0] = WAVEOUT_FEATURE_REPORT_ID;
    let copy_len = payload.len().min(FEATURE_PAYLOAD_BYTES);
    report[1..1 + copy_len].copy_from_slice(&payload[..copy_len]);
    report
}

fn pad_edge_report(mut report: Vec<u8>, edge: bool) -> Vec<u8> {
    if edge {
        report.resize(REPORT_BYTES, 0);
    }
    report
}

pub fn play_test_tone(channel: AudioChannel) -> Result<()> {
    let wav = make_test_wav(channel);
    play_wav_memory(&wav)
}

#[derive(Clone, Copy, Debug)]
pub enum AudioChannel {
    Left,
    Right,
    Both,
}

fn make_test_wav(channel: AudioChannel) -> Vec<u8> {
    const RATE: u32 = 48_000;
    const SECONDS: u32 = 2;
    const CHANNELS: u16 = 2;
    const BITS: u16 = 16;
    let frames = RATE * SECONDS;
    let data_bytes = frames * CHANNELS as u32 * (BITS as u32 / 8);
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 4).to_le_bytes());
    wav.extend_from_slice(&4_u16.to_le_bytes());
    wav.extend_from_slice(&BITS.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for frame in 0..frames {
        let phase = frame as f32 * 440.0 * std::f32::consts::TAU / RATE as f32;
        let sample = (phase.sin() * 0.22 * i16::MAX as f32) as i16;
        let left = if matches!(channel, AudioChannel::Left | AudioChannel::Both) {
            sample
        } else {
            0
        };
        let right = if matches!(channel, AudioChannel::Right | AudioChannel::Both) {
            sample
        } else {
            0
        };
        wav.extend_from_slice(&left.to_le_bytes());
        wav.extend_from_slice(&right.to_le_bytes());
    }
    wav
}

#[cfg(windows)]
fn play_wav_memory(wav: &[u8]) -> Result<()> {
    const SND_SYNC: u32 = 0x0000;
    const SND_MEMORY: u32 = 0x0004;
    const SND_NODEFAULT: u32 = 0x0002;
    let ok = unsafe {
        PlaySoundW(
            wav.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_SYNC | SND_MEMORY | SND_NODEFAULT,
        )
    };
    if ok == 0 {
        bail!("Windows could not play the test tone")
    }
    Ok(())
}

#[cfg(not(windows))]
fn play_wav_memory(_wav: &[u8]) -> Result<()> {
    bail!("audio tests are available on Windows only")
}

pub fn record_and_play_microphone(seconds: u32) -> Result<()> {
    #[cfg(not(windows))]
    bail!("audio tests are available on Windows only");

    #[cfg(windows)]
    {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path = std::env::temp_dir().join(format!("DS5Dongle-mic-test-{suffix}.wav"));
        let path_text = path.to_string_lossy().replace('"', "");
        let _ = mci("close ds5mic");
        mci("open new type waveaudio alias ds5mic")?;
        let result = (|| -> Result<()> {
            mci("set ds5mic time format milliseconds")?;
            // The firmware exposes the controller's mono source duplicated as
            // 48 kHz, 16-bit stereo UAC1 input, so request that exact format.
            mci(
                "set ds5mic channels 2 samplespersec 48000 bitspersample 16 alignment 4 bytespersec 192000",
            )?;
            mci("record ds5mic")?;
            thread::sleep(Duration::from_secs(seconds.clamp(1, 10) as u64));
            mci("stop ds5mic")?;
            mci(&format!("save ds5mic \"{path_text}\""))?;
            Ok(())
        })();
        let _ = mci("close ds5mic");
        result?;

        let _ = mci("close ds5play");
        let playback = (|| -> Result<()> {
            mci(&format!(
                "open \"{path_text}\" type waveaudio alias ds5play"
            ))?;
            mci("play ds5play wait")
        })();
        let _ = mci("close ds5play");
        let _ = std::fs::remove_file(&path);
        playback
    }
}

#[cfg(windows)]
fn mci(command: &str) -> Result<()> {
    let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
    let code =
        unsafe { mciSendStringW(wide.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut()) };
    if code != 0 {
        bail!("Windows audio command failed ({code}): {command}")
    }
    Ok(())
}

#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const u16, module: *mut core::ffi::c_void, flags: u32) -> i32;
    fn mciSendStringW(
        command: *const u16,
        return_string: *mut u16,
        return_length: u32,
        callback: *mut core::ffi::c_void,
    ) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usb_input_layout() {
        let mut report = [0_u8; REPORT_BYTES];
        report[0] = INPUT_REPORT_ID;
        report[1..7].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        report[8] = 0x20;
        report[9] = 0x81;
        report[10] = 0x07;
        report[16..18].copy_from_slice(&(-123_i16).to_le_bytes());
        report[33..37].copy_from_slice(&[0x02, 0x34, 0x12, 0x56]);
        report[53] = 8;
        let state = parse_input_report(&report).unwrap();
        assert_eq!((state.lx, state.ry, state.l2, state.r2), (1, 4, 5, 6));
        assert!(state.cross && state.l1 && state.r3 && state.ps && state.touchpad_click);
        assert_eq!(state.gyro_x, -123);
        assert!(state.touch[0].active);
        assert_eq!(state.battery_percent, Some(80));
    }

    #[test]
    fn builds_safe_standard_output_report() {
        let state = OutputState {
            rumble_right: 20,
            rumble_left: 30,
            lightbar_enabled: true,
            lightbar_rgb: [1, 2, 3],
            player_leds: 0x1f,
            mute_led: 1,
            left_trigger: TriggerPreset::Resistance,
            right_trigger: TriggerPreset::Weapon,
        };
        let report = build_output_report(&state);
        assert_eq!(report.len(), 48);
        assert_eq!(report[0], OUTPUT_REPORT_ID);
        assert_eq!(report[1], 0x0f);
        assert_eq!(report[2], 0x15);
        assert_eq!(&report[11..15], &[2, 15, 100, 255]);
        assert_eq!(&report[22..25], &[1, 40, 230]);
        assert_eq!(&report[45..48], &[1, 2, 3]);
    }

    #[test]
    fn wav_header_matches_payload() {
        let wav = make_test_wav(AudioChannel::Both);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
            wav.len() - 44
        );
    }

    #[test]
    fn controller_audio_reports_match_reference_waveout_protocol() {
        let speaker =
            build_waveout_feature_report(&waveout_setup_payload(ControllerAudioTarget::Speaker));
        let headphone =
            build_waveout_feature_report(&waveout_setup_payload(ControllerAudioTarget::Headphone));
        let start = build_waveout_feature_report(&waveout_control_payload(true));
        let stop = build_waveout_feature_report(&waveout_control_payload(false));
        assert_eq!(speaker.len(), 64);
        assert_eq!(speaker[0], 0x80);
        assert_eq!(&speaker[1..6], &[6, 4, 0, 0, 8]);
        assert_eq!(&headphone[1..10], &[6, 4, 0, 0, 0, 0, 4, 0, 6]);
        assert_eq!(&start[1..6], &[6, 2, 1, 1, 0]);
        assert_eq!(&stop[1..6], &[6, 2, 0, 1, 0]);
    }

    #[test]
    fn controller_audio_output_restores_saved_volumes() {
        let state = OutputState::default();
        let speaker = build_output_report_with_audio(&state, Some(ControllerAudioTarget::Speaker));
        let headphone =
            build_output_report_with_audio(&state, Some(ControllerAudioTarget::Headphone));
        let restored = build_audio_restore_report(&state, (77, 88));
        assert_eq!(speaker[6], 220);
        assert_eq!(speaker[8], 48);
        assert_eq!(headphone[5], 150);
        assert_eq!((restored[5], restored[6], restored[8]), (88, 77, 0));
        assert_eq!(restored[1] & 0xb0, 0xb0);
    }

    #[test]
    fn calculates_hid_interval_percentiles_and_jitter() {
        let metrics = calculate_metrics(
            &[1000, 1000, 1000, 2000, 6000, 11_000],
            Duration::from_millis(22),
        );
        assert_eq!(metrics.sample_count, 6);
        assert!(metrics.report_rate_hz > 270.0 && metrics.report_rate_hz < 275.0);
        assert_eq!(metrics.minimum_interval_ms, 1.0);
        assert_eq!(metrics.maximum_interval_ms, 11.0);
        assert_eq!(metrics.gaps_over_5ms, 2);
        assert_eq!(metrics.gaps_over_10ms, 1);
        assert!(metrics.p99_interval_ms >= metrics.p95_interval_ms);
        assert!(metrics.jitter_stddev_ms > 3.0);
    }

    #[test]
    fn long_stress_metrics_use_bounded_storage() {
        let mut accumulator = DebugAccumulator::new();
        for index in 0..250_000_u32 {
            accumulator.record(1_000 + index % 200);
        }
        let metrics = accumulator.metrics(Duration::from_secs(300), 15_000);
        assert_eq!(metrics.sample_count, 250_000);
        assert_eq!(metrics.stress_output_reports, 15_000);
        assert_eq!(accumulator.histogram.len(), METRIC_HISTOGRAM_BUCKETS);
        assert!(metrics.p99_interval_ms <= 1.2);
    }

    #[test]
    fn stress_profiles_keep_outputs_within_safe_limits() {
        for seconds in [0, 5, 10] {
            let state = stress_output_state(Duration::from_secs(seconds));
            assert!(state.rumble_left <= 105);
            assert!(state.rumble_right <= 95);
            assert!(state.lightbar_enabled);
            assert!(
                state.left_trigger != TriggerPreset::Off
                    || state.right_trigger != TriggerPreset::Off
            );
        }
        let rest = stress_output_state(Duration::from_secs(15));
        assert_eq!(rest.rumble_left, 0);
        assert_eq!(rest.rumble_right, 0);
        assert!(!rest.lightbar_enabled);
        assert_eq!(rest.left_trigger, TriggerPreset::Off);
        assert_eq!(rest.right_trigger, TriggerPreset::Off);
    }
}
