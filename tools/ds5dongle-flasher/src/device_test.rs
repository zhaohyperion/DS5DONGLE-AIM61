use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
#[cfg(windows)]
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::{DUALSENSE_PRODUCT_IDS, SONY_VENDOR_ID};

#[cfg(windows)]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const INPUT_REPORT_ID: u8 = 0x01;
const OUTPUT_REPORT_ID: u8 = 0x02;
const WAVEOUT_FEATURE_REPORT_ID: u8 = 0x80;
const STICK_CALIBRATION_SET_REPORT_ID: u8 = 0x82;
const STICK_CALIBRATION_STATUS_REPORT_ID: u8 = 0x83;
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

#[derive(Clone, Debug, Serialize)]
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
    pub battery_charging: bool,
    pub battery_cable_connected: bool,
    pub battery_error: bool,
    pub headphone_connected: bool,
    pub headset_microphone_connected: bool,
    pub report_count: u64,
    pub report_rate_hz: f32,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            lx: 128,
            ly: 128,
            rx: 128,
            ry: 128,
            l2: 0,
            r2: 0,
            dpad: 8,
            square: false,
            cross: false,
            circle: false,
            triangle: false,
            l1: false,
            r1: false,
            l2_button: false,
            r2_button: false,
            create: false,
            options: false,
            l3: false,
            r3: false,
            ps: false,
            touchpad_click: false,
            mute: false,
            gyro_x: 0,
            gyro_y: 0,
            gyro_z: 0,
            accel_x: 0,
            accel_y: 0,
            accel_z: 0,
            touch: [TouchPoint::default(); 2],
            battery_percent: None,
            battery_charging: false,
            battery_cable_connected: false,
            battery_error: false,
            headphone_connected: false,
            headset_microphone_connected: false,
            report_count: 0,
            report_rate_hz: 0.0,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuidedOutputDemo {
    Lights,
    Rumble,
    Triggers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationStep {
    CenterBegin,
    CenterSample,
    CenterCommit,
    RangeBegin,
    RangeCommit,
}

impl CalibrationStep {
    pub fn id(self) -> &'static str {
        match self {
            Self::CenterBegin => "centerBegin",
            Self::CenterSample => "centerSample",
            Self::CenterCommit => "centerCommit",
            Self::RangeBegin => "rangeBegin",
            Self::RangeCommit => "rangeCommit",
        }
    }

    fn request(self) -> [u8; 3] {
        match self {
            Self::CenterBegin => [1, 1, 1],
            Self::CenterSample => [3, 1, 1],
            Self::CenterCommit => [2, 1, 1],
            Self::RangeBegin => [1, 1, 2],
            Self::RangeCommit => [2, 1, 2],
        }
    }
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
    pub left_trigger_force: u8,
    pub right_trigger_force: u8,
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
            left_trigger_force: 230,
            right_trigger_force: 230,
        }
    }
}

enum TestCommand {
    Output(OutputState),
    GuidedOutputDemo(GuidedOutputDemo),
    StartControllerTone(ControllerAudioTarget),
    StopControllerTone,
    StopAll,
    ResetMetrics,
    SetMetricsActive(bool),
    SetStressActive { active: bool, rate_hz: u32 },
    Calibrate(CalibrationStep),
    Shutdown,
}

#[derive(Debug)]
pub enum TestEvent {
    Connected(String),
    Input(InputState),
    Metrics(DebugMetrics),
    OutputSent,
    ControllerToneChanged(Option<ControllerAudioTarget>),
    CalibrationCompleted(CalibrationStep),
    CalibrationFailed(CalibrationStep, String),
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

    pub fn start_guided_output_demo(&self, demo: GuidedOutputDemo) -> Result<()> {
        self.commands
            .send(TestCommand::GuidedOutputDemo(demo))
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

    pub fn calibrate(&self, step: CalibrationStep) -> Result<()> {
        self.commands
            .send(TestCommand::Calibrate(step))
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

struct GuidedDemoState {
    kind: GuidedOutputDemo,
    frame: usize,
    next_frame_at: Instant,
}

impl GuidedDemoState {
    fn new(kind: GuidedOutputDemo) -> Self {
        Self {
            kind,
            frame: 0,
            next_frame_at: Instant::now(),
        }
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
        let mut last_published_input: Option<InputState> = None;
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
        let mut guided_demo: Option<GuidedDemoState> = None;
        let mut report = [0_u8; REPORT_BYTES];
        loop {
            loop {
                match commands.try_recv() {
                    Ok(TestCommand::Output(state)) => {
                        guided_demo = None;
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
                    Ok(TestCommand::GuidedOutputDemo(kind)) => {
                        guided_demo = Some(GuidedDemoState::new(kind));
                    }
                    Ok(TestCommand::StartControllerTone(target)) => {
                        if guided_demo.take().is_some() {
                            output_state = OutputState::default();
                        }
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
                        guided_demo = None;
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
                        guided_demo = None;
                        stress_active = active;
                        stress_interval = Duration::from_millis(1000 / rate_hz.max(1) as u64);
                        stress_started = Instant::now();
                        stress_last_output = Instant::now() - stress_interval;
                        if !active {
                            output_state = OutputState::default();
                            let report = pad_edge_report(build_stop_report(), edge_report);
                            device
                                .write(&report)
                                .context("failed to stop DS5 stress-test outputs")?;
                        }
                    }
                    Ok(TestCommand::Calibrate(step)) => {
                        guided_demo = None;
                        if controller_tone.take().is_some() {
                            send_waveout_control(&device, false)
                                .context("failed to stop the DualSense tone before calibration")?;
                            let _ = events.send(TestEvent::ControllerToneChanged(None));
                        }
                        output_state = OutputState::default();
                        let report = pad_edge_report(build_stop_report(), edge_report);
                        device
                            .write(&report)
                            .context("failed to release test outputs before calibration")?;
                        match run_calibration_step(&device, step, edge_report) {
                            Ok(()) => {
                                let _ = events.send(TestEvent::CalibrationCompleted(step));
                            }
                            Err(error) => {
                                let _ = events
                                    .send(TestEvent::CalibrationFailed(step, format!("{error:#}")));
                            }
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

            if guided_demo
                .as_ref()
                .is_some_and(|demo| Instant::now() >= demo.next_frame_at)
            {
                let mut demo = guided_demo.take().expect("guided demo state disappeared");
                if let Some((state, duration)) = guided_demo_frame(demo.kind, demo.frame) {
                    let report = pad_edge_report(
                        build_output_report_with_audio(&state, controller_tone),
                        edge_report,
                    );
                    device
                        .write(&report)
                        .context("failed to write guided DS5 output sequence")?;
                    output_state = state;
                    demo.frame += 1;
                    demo.next_frame_at = Instant::now() + duration;
                    guided_demo = Some(demo);
                    let _ = events.send(TestEvent::OutputSent);
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
                        let urgent_input_change =
                            last_published_input.as_ref().is_none_or(|previous| {
                                input_change_requires_immediate_publish(previous, &state)
                            });
                        if urgent_input_change
                            || last_ui_emit.elapsed() >= Duration::from_millis(16)
                        {
                            last_published_input = Some(state.clone());
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

fn input_change_requires_immediate_publish(previous: &InputState, current: &InputState) -> bool {
    previous.dpad != current.dpad
        || previous.square != current.square
        || previous.cross != current.cross
        || previous.circle != current.circle
        || previous.triangle != current.triangle
        || previous.l1 != current.l1
        || previous.r1 != current.r1
        || previous.l2_button != current.l2_button
        || previous.r2_button != current.r2_button
        || previous.create != current.create
        || previous.options != current.options
        || previous.l3 != current.l3
        || previous.r3 != current.r3
        || previous.ps != current.ps
        || previous.touchpad_click != current.touchpad_click
        || previous.mute != current.mute
        || previous.touch[0].active != current.touch[0].active
        || previous.touch[1].active != current.touch[1].active
        || previous.battery_charging != current.battery_charging
        || previous.battery_cable_connected != current.battery_cable_connected
        || previous.battery_error != current.battery_error
        || previous.headphone_connected != current.headphone_connected
        || previous.headset_microphone_connected != current.headset_microphone_connected
}

#[cfg(windows)]
fn run_calibration_step(
    device: &hidapi::HidDevice,
    step: CalibrationStep,
    dualsense_edge: bool,
) -> Result<()> {
    let transaction_count = if dualsense_edge
        && matches!(
            step,
            CalibrationStep::CenterCommit | CalibrationStep::RangeCommit
        ) {
        2
    } else {
        1
    };
    for transaction in 0..transaction_count {
        send_calibration_transaction(device, step, dualsense_edge, transaction)?;
    }
    Ok(())
}

#[cfg(windows)]
fn send_calibration_transaction(
    device: &hidapi::HidDevice,
    step: CalibrationStep,
    dualsense_edge: bool,
    transaction: usize,
) -> Result<()> {
    // The DS5 descriptor declares report 0x82 as nine payload bytes. Keep the
    // unused and CRC-reserved bytes zero; M61 fills the Bluetooth feature CRC.
    let mut request = [0_u8; 10];
    request[0] = STICK_CALIBRATION_SET_REPORT_ID;
    request[1..4].copy_from_slice(&step.request());
    device
        .send_feature_report(&request)
        .with_context(|| format!("failed to send stick calibration step {step:?}"))?;

    // Through M61, the first 0x83 read intentionally causes an asynchronous
    // Bluetooth GET_REPORT and can return no data. Poll with a tight bound so
    // a missing controller or unsupported firmware never hangs the UI.
    let expected = calibration_expected_statuses(step, dualsense_edge, transaction);
    let mut last_status = None;
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(40));
        let mut response = [0_u8; REPORT_BYTES];
        response[0] = STICK_CALIBRATION_STATUS_REPORT_ID;
        match device.get_feature_report(&mut response) {
            Ok(length) if length >= 4 => {
                let actual = [response[0], response[1], response[2], response[3]];
                if expected.contains(&actual) {
                    return Ok(());
                }
                last_status = Some(actual);
            }
            Ok(_) | Err(_) => {}
        }
    }

    if let Some(actual) = last_status {
        bail!(
            "controller rejected stick calibration step {step:?}: expected {}, received {}",
            expected
                .iter()
                .map(hex::encode_upper)
                .collect::<Vec<_>>()
                .join(" or "),
            hex::encode_upper(actual)
        );
    }
    bail!(
        "no 0x83 calibration response; update M61 firmware to a build that supports the guarded 0x82 bridge"
    )
}

fn calibration_expected_statuses(
    step: CalibrationStep,
    dualsense_edge: bool,
    transaction: usize,
) -> &'static [[u8; 4]] {
    const CENTER_ACTIVE: &[[u8; 4]] = &[[0x83, 0x01, 0x01, 0x01]];
    const CENTER_COMMITTED: &[[u8; 4]] = &[[0x83, 0x01, 0x01, 0x02]];
    const EDGE_CENTER_COMMITTED: &[[u8; 4]] = &[[0x83, 0x01, 0x01, 0x03], [0x83, 0x01, 0x03, 0x12]];
    const RANGE_ACTIVE: &[[u8; 4]] = &[[0x83, 0x01, 0x02, 0x01]];
    const RANGE_COMMITTED: &[[u8; 4]] = &[[0x83, 0x01, 0x02, 0x02]];
    const EDGE_RANGE_COMMITTED: &[[u8; 4]] = &[[0x83, 0x01, 0x02, 0x03]];

    match (step, dualsense_edge, transaction) {
        (CalibrationStep::CenterBegin | CalibrationStep::CenterSample, _, _) => CENTER_ACTIVE,
        (CalibrationStep::CenterCommit, true, 0) => CENTER_ACTIVE,
        (CalibrationStep::CenterCommit, true, _) => EDGE_CENTER_COMMITTED,
        (CalibrationStep::CenterCommit, false, _) => CENTER_COMMITTED,
        (CalibrationStep::RangeBegin, _, _) => RANGE_ACTIVE,
        (CalibrationStep::RangeCommit, true, 0) => RANGE_ACTIVE,
        (CalibrationStep::RangeCommit, true, _) => EDGE_RANGE_COMMITTED,
        (CalibrationStep::RangeCommit, false, _) => RANGE_COMMITTED,
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

fn guided_demo_frame(kind: GuidedOutputDemo, frame: usize) -> Option<(OutputState, Duration)> {
    match kind {
        GuidedOutputDemo::Rumble => {
            if frame >= 12 {
                return None;
            }
            let (state, duration_ms) = match frame % 4 {
                0 => (
                    OutputState {
                        rumble_left: 89,
                        ..OutputState::default()
                    },
                    500,
                ),
                1 => (OutputState::default(), 200),
                2 => (
                    OutputState {
                        rumble_right: 89,
                        ..OutputState::default()
                    },
                    500,
                ),
                _ => (OutputState::default(), 300),
            };
            Some((state, Duration::from_millis(duration_ms)))
        }
        GuidedOutputDemo::Lights => {
            if frame > 9 {
                return None;
            }
            if frame == 9 {
                return Some((OutputState::default(), Duration::from_millis(100)));
            }
            let colors = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
            let player_patterns = [0b10001, 0b01010, 0b00100];
            Some((
                OutputState {
                    lightbar_enabled: true,
                    lightbar_rgb: colors[frame / 3],
                    player_leds: player_patterns[frame % 3],
                    mute_led: 2,
                    ..OutputState::default()
                },
                Duration::from_millis(250),
            ))
        }
        GuidedOutputDemo::Triggers => {
            if frame >= 6 {
                return None;
            }
            let active = frame % 2 == 0;
            Some((
                if active {
                    OutputState {
                        left_trigger: TriggerPreset::Resistance,
                        right_trigger: TriggerPreset::Resistance,
                        left_trigger_force: 89,
                        right_trigger_force: 89,
                        ..OutputState::default()
                    }
                } else {
                    OutputState::default()
                },
                Duration::from_millis(if active { 700 } else { 300 }),
            ))
        }
    }
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
            left_trigger_force: 70,
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
            right_trigger_force: 70,
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
            left_trigger_force: 70,
            right_trigger_force: 70,
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
    if payload.len() < 54 {
        return None;
    }
    let buttons0 = payload[7];
    let buttons1 = payload[8];
    let buttons2 = payload[9];
    let battery = parse_battery_status(payload[52]);
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
        gyro_y: read_i16(payload, 17),
        gyro_z: read_i16(payload, 19),
        accel_x: read_i16(payload, 21),
        accel_y: read_i16(payload, 23),
        accel_z: read_i16(payload, 25),
        touch: [parse_touch(payload, 32), parse_touch(payload, 36)],
        battery_percent: battery.percent,
        battery_charging: battery.charging,
        battery_cable_connected: battery.cable_connected,
        battery_error: battery.error,
        headphone_connected: payload[53] & 0x01 != 0,
        headset_microphone_connected: payload[53] & 0x02 != 0,
        report_count: 0,
        report_rate_hz: 0.0,
    })
}

struct BatteryStatus {
    percent: Option<u8>,
    charging: bool,
    cable_connected: bool,
    error: bool,
}

fn parse_battery_status(raw: u8) -> BatteryStatus {
    let level = raw & 0x0f;
    match raw >> 4 {
        0 => BatteryStatus {
            percent: Some((level.saturating_mul(10) + 5).min(100)),
            charging: false,
            cable_connected: false,
            error: false,
        },
        1 => BatteryStatus {
            percent: Some((level.saturating_mul(10) + 5).min(100)),
            charging: true,
            cable_connected: true,
            error: false,
        },
        2 => BatteryStatus {
            percent: Some(100),
            charging: false,
            cable_connected: true,
            error: false,
        },
        15 => BatteryStatus {
            percent: Some(0),
            charging: true,
            cable_connected: true,
            error: false,
        },
        _ => BatteryStatus {
            percent: None,
            charging: false,
            cable_connected: false,
            error: true,
        },
    }
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
            data[5] = 85;
            data[7] = 48;
        }
        Some(ControllerAudioTarget::Headphone) => {
            data[0] |= 0x90;
            data[4] = 65;
        }
        None => {}
    }
    data[8] = state.mute_led.min(2);
    data[38] = 0x03;
    data[39] = 0x02;
    data[41] = 0x02;
    data[42] = 0;
    data[43] = state.player_leds & 0x1f;
    encode_trigger(data, 10, state.right_trigger, state.right_trigger_force);
    encode_trigger(data, 21, state.left_trigger, state.left_trigger_force);
    if state.lightbar_enabled {
        data[44..47].copy_from_slice(&state.lightbar_rgb);
    }
    report
}

fn encode_trigger(data: &mut [u8], offset: usize, preset: TriggerPreset, force: u8) {
    data[offset..offset + 8].fill(0);
    match preset {
        TriggerPreset::Off => {}
        TriggerPreset::Resistance => {
            data[offset] = 0x01;
            data[offset + 1] = 40;
            data[offset + 2] = force;
        }
        TriggerPreset::Weapon => {
            data[offset] = 0x02;
            data[offset + 1] = 15;
            data[offset + 2] = 100;
            data[offset + 3] = force;
        }
        TriggerPreset::Automatic => {
            data[offset] = 0x06;
            data[offset + 1] = 10;
            data[offset + 2] = force;
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneTestMetrics {
    pub capture_backend: String,
    pub capture_endpoint: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub duration_ms: u64,
    pub rms_percent: f32,
    pub peak_percent: f32,
    pub active_windows: u32,
    pub signal_detected: bool,
    pub speech_rms_percent: Option<f32>,
    pub silence_rms_percent: Option<f32>,
    pub signal_to_silence_db: Option<f32>,
    pub stereo_difference_rms_percent: Option<f32>,
    pub playback_succeeded: bool,
    pub playback_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MicrophoneTestResult {
    pub metrics: MicrophoneTestMetrics,
    pub wav: Vec<u8>,
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
        bail!(
            "Windows could not play the test tone; verify that the M61 USB audio output is enabled and selected as the default output device"
        )
    }
    Ok(())
}

#[cfg(not(windows))]
fn play_wav_memory(_wav: &[u8]) -> Result<()> {
    bail!("audio tests are available on Windows only")
}

pub fn record_and_play_microphone(seconds: u32) -> Result<MicrophoneTestResult> {
    #[cfg(not(windows))]
    bail!("audio tests are available on Windows only");

    #[cfg(windows)]
    {
        let (endpoint, sample_rate_hz, channels, samples) = capture_m61_microphone_wasapi(seconds)?;
        let wav = make_pcm16_wav(&samples, sample_rate_hz, channels)?;
        let mut metrics = analyze_microphone_wav(&wav, seconds)?;
        metrics.capture_backend = "Windows WASAPI".to_owned();
        metrics.capture_endpoint = endpoint;
        match play_wav_memory(&wav) {
            Ok(()) => {
                metrics.playback_succeeded = true;
                metrics.playback_error = None;
            }
            Err(error) => {
                // Capture and signal analysis remain valid even when the
                // independently selected Windows playback endpoint fails.
                metrics.playback_succeeded = false;
                metrics.playback_error = Some(format!("{error:#}"));
            }
        }
        Ok(MicrophoneTestResult { metrics, wav })
    }
}

#[cfg(windows)]
fn capture_m61_microphone_wasapi(seconds: u32) -> Result<(String, u32, u16, Vec<f32>)> {
    let host = cpal::default_host();
    let mut available = Vec::new();
    let mut candidates = Vec::new();
    for device in host
        .input_devices()
        .context("unable to enumerate Windows WASAPI input endpoints")?
    {
        let name = device
            .name()
            .unwrap_or_else(|_| "<unnamed input>".to_owned());
        available.push(name.clone());
        let lower = name.to_ascii_lowercase();
        let rank = if lower.contains("dualsense wireless controller") {
            Some(0_u8)
        } else if lower.contains("m61") {
            Some(1)
        } else if lower.contains("wireless controller") {
            Some(2)
        } else {
            None
        };
        if let Some(rank) = rank {
            candidates.push((rank, name, device));
        }
    }
    candidates.sort_by_key(|candidate| candidate.0);
    let (_, endpoint_name, device) = candidates.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "M61/DualSense WASAPI microphone endpoint was not found; available inputs: {}",
            if available.is_empty() {
                "none".to_owned()
            } else {
                available.join(", ")
            }
        )
    })?;

    let mut configs = device
        .supported_input_configs()
        .context("unable to query the M61 WASAPI microphone formats")?
        .collect::<Vec<_>>();
    configs.sort_by_key(|range| {
        let contains_48k =
            range.min_sample_rate().0 <= 48_000 && range.max_sample_rate().0 >= 48_000;
        (
            !contains_48k,
            range.channels() != 2,
            match range.sample_format() {
                cpal::SampleFormat::I16 => 0_u8,
                cpal::SampleFormat::F32 => 1,
                cpal::SampleFormat::U16 => 2,
                _ => 3,
            },
        )
    });
    let range = configs
        .into_iter()
        .next()
        .context("the M61 WASAPI endpoint exposes no supported input format")?;
    let sample_rate = if range.min_sample_rate().0 <= 48_000 && range.max_sample_rate().0 >= 48_000
    {
        cpal::SampleRate(48_000)
    } else {
        range.max_sample_rate()
    };
    let sample_format = range.sample_format();
    let supported = range.with_sample_rate(sample_rate);
    let config = supported.config();
    let channels = config.channels;
    let capture_seconds = microphone_recording_seconds(seconds);
    let max_samples = sample_rate.0 as usize * channels as usize * capture_seconds as usize;
    let samples = Arc::new(Mutex::new(Vec::<f32>::with_capacity(max_samples)));
    let callback_samples = Arc::clone(&samples);
    let stream_error = Arc::new(Mutex::new(None::<String>));
    let callback_error = Arc::clone(&stream_error);
    let error_callback = move |error: cpal::StreamError| {
        if let Ok(mut slot) = callback_error.lock() {
            *slot = Some(error.to_string());
        }
    };

    macro_rules! build_stream {
        ($sample:ty, $convert:expr) => {{
            let target = Arc::clone(&callback_samples);
            device.build_input_stream(
                &config,
                move |data: &[$sample], _| {
                    if let Ok(mut output) = target.lock() {
                        let remaining = max_samples.saturating_sub(output.len());
                        output.extend(data.iter().take(remaining).map($convert));
                    }
                },
                error_callback,
                None,
            )
        }};
    }
    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_stream!(f32, |sample: &f32| sample.clamp(-1.0, 1.0)),
        cpal::SampleFormat::I16 => {
            build_stream!(i16, |sample: &i16| *sample as f32 / i16::MAX as f32)
        }
        cpal::SampleFormat::U16 => build_stream!(u16, |sample: &u16| {
            (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0
        }),
        other => bail!("unsupported M61 WASAPI sample format: {other:?}"),
    }
    .context("unable to open the selected M61 WASAPI microphone endpoint")?;
    stream
        .play()
        .context("unable to start M61 WASAPI microphone capture")?;
    thread::sleep(Duration::from_secs(capture_seconds as u64));
    drop(stream);
    if let Some(error) = stream_error.lock().ok().and_then(|mut value| value.take()) {
        bail!("M61 WASAPI capture failed: {error}");
    }
    let captured = Arc::try_unwrap(samples)
        .map_err(|_| anyhow::anyhow!("WASAPI capture buffer is still in use"))?
        .into_inner()
        .map_err(|_| anyhow::anyhow!("WASAPI capture buffer was poisoned"))?;
    if captured.len() < sample_rate.0 as usize * channels as usize / 2 {
        bail!(
            "M61 WASAPI capture returned too little audio: {} samples",
            captured.len()
        );
    }
    Ok((endpoint_name, sample_rate.0, channels, captured))
}

#[cfg(windows)]
fn make_pcm16_wav(samples: &[f32], sample_rate_hz: u32, channels: u16) -> Result<Vec<u8>> {
    if channels == 0 || sample_rate_hz == 0 {
        bail!("invalid WASAPI capture format");
    }
    let data_bytes =
        u32::try_from(samples.len().saturating_mul(2)).context("captured WAV is too large")?;
    let block_align = channels * 2;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36_u32 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate_hz.to_le_bytes());
    wav.extend_from_slice(&(sample_rate_hz * block_align as u32).to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    Ok(wav)
}

fn analyze_microphone_wav(wav: &[u8], requested_seconds: u32) -> Result<MicrophoneTestMetrics> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        bail!("recorded microphone data is not a RIFF/WAVE file");
    }
    let mut cursor = 12_usize;
    let mut format = None;
    let mut pcm_data = None;
    while cursor.checked_add(8).is_some_and(|end| end <= wav.len()) {
        let id = &wav[cursor..cursor + 4];
        let length = u32::from_le_bytes(wav[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let data_start = cursor + 8;
        let Some(data_end) = data_start.checked_add(length) else {
            bail!("recorded microphone WAV contains an overflowing chunk");
        };
        if data_end > wav.len() {
            bail!("recorded microphone WAV contains a truncated chunk");
        }
        if id == b"fmt " && length >= 16 {
            format = Some((
                u16::from_le_bytes(wav[data_start..data_start + 2].try_into().unwrap()),
                u16::from_le_bytes(wav[data_start + 2..data_start + 4].try_into().unwrap()),
                u32::from_le_bytes(wav[data_start + 4..data_start + 8].try_into().unwrap()),
                u16::from_le_bytes(wav[data_start + 14..data_start + 16].try_into().unwrap()),
            ));
        } else if id == b"data" {
            pcm_data = Some(&wav[data_start..data_end]);
        }
        cursor = data_end + (length & 1);
    }
    let (audio_format, channels, sample_rate_hz, bits_per_sample) =
        format.context("recorded microphone WAV has no usable format chunk")?;
    if audio_format != 1 || bits_per_sample != 16 || channels == 0 {
        bail!(
            "unsupported microphone WAV format: encoding {audio_format}, {channels} channel(s), {bits_per_sample}-bit"
        );
    }
    let pcm_data = pcm_data.context("recorded microphone WAV has no data chunk")?;
    let frame_bytes = channels as usize * 2;
    let frame_count = pcm_data.len() / frame_bytes;
    if frame_count == 0 || sample_rate_hz == 0 {
        bail!("recorded microphone WAV contains no PCM frames");
    }

    let mut mono = Vec::with_capacity(frame_count);
    let mut stereo_diff_squared = 0.0_f64;
    for frame in pcm_data[..frame_count * frame_bytes].chunks_exact(frame_bytes) {
        let left = i16::from_le_bytes([frame[0], frame[1]]) as f64 / i16::MAX as f64;
        mono.push(left);
        if channels >= 2 {
            let right = i16::from_le_bytes([frame[2], frame[3]]) as f64 / i16::MAX as f64;
            stereo_diff_squared += (left - right).powi(2);
        }
    }
    let rms = |samples: &[f64]| -> f64 {
        if samples.is_empty() {
            0.0
        } else {
            (samples.iter().map(|sample| sample * sample).sum::<f64>() / samples.len() as f64)
                .sqrt()
        }
    };
    let total_rms = rms(&mono);
    let peak = mono
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f64, f64::max);
    let silence_frames = sample_rate_hz as usize * 5;
    let guided_split = requested_seconds >= 25 && frame_count > silence_frames;
    let (speech_rms, silence_rms) = if guided_split {
        let silence_start = frame_count - silence_frames;
        let speech_end = (sample_rate_hz as usize * 20).min(silence_start);
        (
            Some(rms(&mono[..speech_end])),
            Some(rms(&mono[silence_start..])),
        )
    } else {
        (None, None)
    };
    let activity_threshold = silence_rms
        .map(|value| (value * 3.0).max(0.01))
        .unwrap_or(0.01);
    let activity_source = if guided_split {
        &mono[..(sample_rate_hz as usize * 20).min(frame_count - silence_frames)]
    } else {
        mono.as_slice()
    };
    let window_frames = (sample_rate_hz as usize / 20).max(1);
    let active_windows = activity_source
        .chunks(window_frames)
        .filter(|window| rms(window) >= activity_threshold)
        .count() as u32;
    let signal_detected = active_windows >= 5 && peak >= 0.02;
    let signal_to_silence_db = speech_rms.zip(silence_rms).map(|(speech, silence)| {
        (20.0 * (speech.max(1.0e-9) / silence.max(1.0e-9)).log10()) as f32
    });

    Ok(MicrophoneTestMetrics {
        capture_backend: String::new(),
        capture_endpoint: String::new(),
        sample_rate_hz,
        channels,
        bits_per_sample,
        duration_ms: frame_count as u64 * 1000 / sample_rate_hz as u64,
        rms_percent: (total_rms * 100.0) as f32,
        peak_percent: (peak * 100.0) as f32,
        active_windows,
        signal_detected,
        speech_rms_percent: speech_rms.map(|value| (value * 100.0) as f32),
        silence_rms_percent: silence_rms.map(|value| (value * 100.0) as f32),
        signal_to_silence_db,
        stereo_difference_rms_percent: (channels >= 2)
            .then_some((stereo_diff_squared / frame_count as f64).sqrt() as f32 * 100.0),
        playback_succeeded: false,
        playback_error: None,
    })
}

fn microphone_recording_seconds(requested: u32) -> u32 {
    requested.clamp(1, 30)
}

#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const u16, module: *mut core::ffi::c_void, flags: u32) -> i32;
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
        report[18..20].copy_from_slice(&(456_i16).to_le_bytes());
        report[20..22].copy_from_slice(&(-789_i16).to_le_bytes());
        report[33..37].copy_from_slice(&[0x02, 0x34, 0x12, 0x56]);
        report[53] = 8;
        report[54] = 0x03;
        let state = parse_input_report(&report).unwrap();
        assert_eq!((state.lx, state.ry, state.l2, state.r2), (1, 4, 5, 6));
        assert!(state.cross && state.l1 && state.r3 && state.ps && state.touchpad_click);
        assert_eq!(
            (state.gyro_x, state.gyro_y, state.gyro_z),
            (-123, 456, -789)
        );
        assert!(state.touch[0].active);
        assert_eq!(state.battery_percent, Some(85));
        assert!(!state.battery_charging && !state.battery_cable_connected);
        assert!(state.headphone_connected && state.headset_microphone_connected);
        let charging = parse_battery_status(0x18);
        assert_eq!(charging.percent, Some(85));
        assert!(charging.charging && charging.cable_connected && !charging.error);
        assert_eq!(parse_battery_status(0x20).percent, Some(100));
        assert_eq!(parse_battery_status(0xf0).percent, Some(0));
        assert!(parse_battery_status(0xb0).error);
    }

    #[test]
    fn short_digital_transitions_bypass_the_ui_rate_limit() {
        let idle = InputState::default();
        let mut pressed = idle.clone();
        pressed.cross = true;
        assert!(input_change_requires_immediate_publish(&idle, &pressed));
        let mut touched = idle.clone();
        touched.touch[0].active = true;
        assert!(input_change_requires_immediate_publish(&idle, &touched));
        let mut analog_only = idle.clone();
        analog_only.lx = 255;
        assert!(!input_change_requires_immediate_publish(
            &idle,
            &analog_only
        ));
    }

    #[test]
    fn microphone_recording_duration_supports_guided_test_and_has_a_safe_limit() {
        assert_eq!(microphone_recording_seconds(0), 1);
        assert_eq!(microphone_recording_seconds(5), 5);
        assert_eq!(microphone_recording_seconds(25), 25);
        assert_eq!(microphone_recording_seconds(300), 30);
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
            left_trigger_force: 111,
            right_trigger_force: 222,
        };
        let report = build_output_report(&state);
        assert_eq!(report.len(), 48);
        assert_eq!(report[0], OUTPUT_REPORT_ID);
        assert_eq!(report[1], 0x0f);
        assert_eq!(report[2], 0x15);
        assert_eq!(&report[11..15], &[2, 15, 100, 222]);
        assert_eq!(&report[22..25], &[1, 40, 111]);
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
        let metrics = analyze_microphone_wav(&wav, 2).unwrap();
        assert_eq!((metrics.sample_rate_hz, metrics.channels), (48_000, 2));
        assert_eq!(metrics.duration_ms, 2_000);
        assert!(metrics.signal_detected);
        assert!(metrics.rms_percent > 15.0 && metrics.rms_percent < 16.0);
        assert!(metrics.peak_percent > 21.0 && metrics.peak_percent < 23.0);
        assert_eq!(metrics.stereo_difference_rms_percent, Some(0.0));
        assert!(analyze_microphone_wav(b"not a wave", 2).is_err());
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
        assert_eq!(speaker[6], 85);
        assert_eq!(speaker[8], 48);
        assert_eq!(headphone[5], 65);
        assert_eq!((restored[5], restored[6], restored[8]), (88, 77, 0));
        assert_eq!(restored[1] & 0xb0, 0xb0);
    }

    #[test]
    fn guided_output_sequences_are_bounded_and_end_released() {
        let rumble = (0..12)
            .map(|frame| {
                guided_demo_frame(GuidedOutputDemo::Rumble, frame)
                    .unwrap()
                    .0
            })
            .collect::<Vec<_>>();
        assert_eq!((rumble[0].rumble_left, rumble[0].rumble_right), (89, 0));
        assert_eq!((rumble[2].rumble_left, rumble[2].rumble_right), (0, 89));
        assert!(
            rumble
                .iter()
                .all(|state| state.rumble_left <= 89 && state.rumble_right <= 89)
        );
        assert_eq!((rumble[11].rumble_left, rumble[11].rumble_right), (0, 0));
        assert!(guided_demo_frame(GuidedOutputDemo::Rumble, 12).is_none());

        let light_colors = (0..9)
            .map(|frame| {
                guided_demo_frame(GuidedOutputDemo::Lights, frame)
                    .unwrap()
                    .0
                    .lightbar_rgb
            })
            .collect::<Vec<_>>();
        assert!(light_colors.contains(&[255, 0, 0]));
        assert!(light_colors.contains(&[0, 255, 0]));
        assert!(light_colors.contains(&[0, 0, 255]));
        let light_release = guided_demo_frame(GuidedOutputDemo::Lights, 9).unwrap().0;
        assert!(!light_release.lightbar_enabled);

        let trigger_release = guided_demo_frame(GuidedOutputDemo::Triggers, 5).unwrap().0;
        assert_eq!(trigger_release.left_trigger, TriggerPreset::Off);
        assert_eq!(trigger_release.right_trigger, TriggerPreset::Off);
        assert!(guided_demo_frame(GuidedOutputDemo::Triggers, 6).is_none());
    }

    #[test]
    fn edge_calibration_uses_the_reference_two_stage_commit_statuses() {
        assert_eq!(
            calibration_expected_statuses(CalibrationStep::CenterCommit, true, 0),
            &[[0x83, 0x01, 0x01, 0x01]]
        );
        assert_eq!(
            calibration_expected_statuses(CalibrationStep::CenterCommit, true, 1),
            &[[0x83, 0x01, 0x01, 0x03], [0x83, 0x01, 0x03, 0x12]]
        );
        assert_eq!(
            calibration_expected_statuses(CalibrationStep::RangeCommit, true, 1),
            &[[0x83, 0x01, 0x02, 0x03]]
        );
        assert_eq!(
            calibration_expected_statuses(CalibrationStep::CenterCommit, false, 0),
            &[[0x83, 0x01, 0x01, 0x02]]
        );
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
