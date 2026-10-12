use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    DUALSENSE_PRODUCT_IDS, FIRMWARE_VERSION_REPORT_ID, SONY_VENDOR_ID, controller_identity,
    decode_firmware_identity_report,
};

pub const REPORT_ID: u8 = 0xfd;
const FALLBACK_REPORT_ID: u8 = 0xf8;
const REPORT_SIZE: usize = 63;
const WIRE_REPORT_SIZE: usize = REPORT_SIZE + 1;
const CRC_OFFSET: usize = 59;
const HEADER_SIZE: usize = 16;
const DATA_SIZE: usize = 43;
const REQUIRED_PAGES: usize = 6;
const MAX_PAGES: usize = 16;
const SCHEDULER_TASK_NAMES: [&str; 7] = ["bt", "usb", "audio", "mic", "led", "ota", "diagnostics"];
#[cfg(windows)]
const READ_ATTEMPTS: usize = 4;
#[cfg(windows)]
const READ_RETRY_DELAY_MS: u64 = 20;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSnapshot {
    pub protocol_version: u8,
    pub build_profile: String,
    pub snapshot_seq: u32,
    pub captured_at_unix_ms: u64,
    pub monotonic_ms: u32,
    pub raw_flags: u8,
    pub uptime_ms: u32,
    pub bt_state: u8,
    pub health_flags: u8,
    pub heap_free_bytes: u32,
    pub heap_min_free_bytes: u32,
    pub memory_status_flags: u8,
    pub bt_rssi_dbm: Option<i8>,
    pub battery_percent: Option<u8>,
    pub battery_state: u8,
    pub usb_in_completed: u32,
    pub bt_input_reports: u32,
    pub bt_output_completed: u32,
    pub usb_input_updates: u32,
    pub usb_input_coalesced: u32,
    pub usb_transfers_started: u32,
    pub usb_start_errors: u32,
    pub bt_game_enqueued: u32,
    pub bt_game_coalesced: u32,
    pub bt_game_completed: u32,
    pub bt_game_pending: bool,
    pub bt_tx_in_flight: bool,
    pub bt_audio_queue_depth: u8,
    pub bt_control_queue_depth: u8,
    pub pcm_blocks_queued: u32,
    pub bt_audio_pairs_submitted: u32,
    pub mic_underruns: u32,
    pub mic_overruns: u32,
    pub loss_pressure: u32,
    pub bt_audio_backpressure: u32,
    pub bt_audio_dropped_stale: u32,
    pub bt_control_backpressure: u32,
    pub bt_alloc_failures: u32,
    pub bt_send_failures: u32,
    pub bt_stale_completions: u32,
    pub bt_audio_queue_high_watermark: u8,
    pub bt_control_queue_high_watermark: u8,
    pub audio_frames_processed: u32,
    pub audio_pairs_rejected: u32,
    pub audio_discontinuities: u32,
    pub audio_encode_errors: u32,
    pub audio_pipeline_resets: u32,
    pub audio_deadline_misses: u32,
    pub audio_block_p99_us: u16,
    pub audio_block_max_us: u16,
    pub audio_resample_p99_us: u16,
    pub audio_resample_max_us: u16,
    pub audio_encode_p99_us: u16,
    pub audio_encode_max_us: u16,
    pub audio_timing_flags: u8,
    pub pcm_blocks_dropped: u32,
    pub pcm_pool_starvations: u32,
    pub pcm_queue_overruns: u32,
    pub pcm_stale_blocks: u32,
    pub pcm_ready_high_watermark: u32,
    pub mic_endpoint_write_errors: u32,
    pub speaker_stream_active: bool,
    pub microphone_stream_active: bool,
    pub mic_frames_received: Option<u32>,
    pub mic_frames_decoded: Option<u32>,
    pub mic_queue_overruns: Option<u32>,
    pub mic_decode_errors: Option<u32>,
    pub mic_queue_high_watermark: Option<u32>,
    pub mic_decode_p99_us: Option<u16>,
    pub mic_decode_max_us: Option<u16>,
    pub mic_decode_timing_flags: Option<u8>,
    pub ota_state: u8,
    pub ota_error: u8,
    pub bridge_latency_samples: Option<u32>,
    pub bridge_rx_to_submit_avg_us: Option<u16>,
    pub bridge_rx_to_submit_max_us: Option<u16>,
    pub bridge_usb_transfer_avg_us: Option<u16>,
    pub bridge_usb_transfer_max_us: Option<u16>,
    pub bridge_total_avg_us: Option<u16>,
    pub bridge_total_p95_us: Option<u16>,
    pub bridge_total_p99_us: Option<u16>,
    pub bridge_total_max_us: Option<u16>,
    pub bridge_timing_flags: Option<u8>,
    pub scheduler_tasks: Option<Vec<FirmwareTaskMetric>>,
    pub bridge_input_queue_depth: Option<u8>,
    pub bridge_output_queue_depth: Option<u8>,
    pub bridge_input_queue_high_watermark: Option<u8>,
    pub bridge_output_queue_high_watermark: Option<u8>,
    pub raw_pages: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareTaskMetric {
    pub name: String,
    pub stack_min_free_words: Option<u16>,
    pub cpu_permille: Option<u16>,
    pub priority: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeHealthStatus {
    Healthy,
    Warning,
    Critical,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthFinding {
    pub code: &'static str,
    pub severity: RuntimeHealthStatus,
    pub component: String,
    pub observed: u64,
    pub threshold: u64,
    pub unit: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthAssessment {
    pub status: RuntimeHealthStatus,
    pub scheduler_data_available: bool,
    pub cpu_sample_available: bool,
    pub findings: Vec<RuntimeHealthFinding>,
}

pub fn assess_runtime_health(snapshot: &DiagnosticSnapshot) -> RuntimeHealthAssessment {
    const STACK_WARNING_WORDS: u64 = 256;
    const STACK_CRITICAL_WORDS: u64 = 128;
    const CPU_WARNING_PERMILLE: u64 = 850;
    const CPU_CRITICAL_PERMILLE: u64 = 950;
    const HEAP_WARNING_BYTES: u64 = 32 * 1024;
    const HEAP_CRITICAL_BYTES: u64 = 16 * 1024;
    const BRIDGE_P99_WARNING_US: u64 = 5_000;
    const BRIDGE_P99_CRITICAL_US: u64 = 10_000;
    const BRIDGE_MIN_SAMPLES: u32 = 100;
    const OUTPUT_QUEUE_CAPACITY: u64 = 5;

    fn add_finding(
        findings: &mut Vec<RuntimeHealthFinding>,
        code: &'static str,
        severity: RuntimeHealthStatus,
        component: impl Into<String>,
        observed: u64,
        threshold: u64,
        unit: &'static str,
    ) {
        findings.push(RuntimeHealthFinding {
            code,
            severity,
            component: component.into(),
            observed,
            threshold,
            unit,
        });
    }

    let mut findings = Vec::new();
    let scheduler_data_available = snapshot.scheduler_tasks.is_some();
    let cpu_sample_available = snapshot
        .scheduler_tasks
        .as_ref()
        .is_some_and(|tasks| tasks.iter().any(|task| task.cpu_permille.is_some()));

    if let Some(tasks) = &snapshot.scheduler_tasks {
        for task in tasks {
            if let Some(words) = task.stack_min_free_words {
                let words = u64::from(words);
                if words < STACK_CRITICAL_WORDS {
                    add_finding(
                        &mut findings,
                        "task_stack_critical",
                        RuntimeHealthStatus::Critical,
                        &task.name,
                        words,
                        STACK_CRITICAL_WORDS,
                        "words",
                    );
                } else if words < STACK_WARNING_WORDS {
                    add_finding(
                        &mut findings,
                        "task_stack_low",
                        RuntimeHealthStatus::Warning,
                        &task.name,
                        words,
                        STACK_WARNING_WORDS,
                        "words",
                    );
                }
            }
            if let Some(permille) = task.cpu_permille {
                let permille = u64::from(permille);
                if permille >= CPU_CRITICAL_PERMILLE {
                    add_finding(
                        &mut findings,
                        "task_cpu_critical",
                        RuntimeHealthStatus::Critical,
                        &task.name,
                        permille,
                        CPU_CRITICAL_PERMILLE,
                        "permille",
                    );
                } else if permille >= CPU_WARNING_PERMILLE {
                    add_finding(
                        &mut findings,
                        "task_cpu_high",
                        RuntimeHealthStatus::Warning,
                        &task.name,
                        permille,
                        CPU_WARNING_PERMILLE,
                        "permille",
                    );
                }
            }
        }
    }

    if snapshot.memory_status_flags & 1 != 0 {
        let minimum_heap = u64::from(snapshot.heap_min_free_bytes);
        if minimum_heap < HEAP_CRITICAL_BYTES {
            add_finding(
                &mut findings,
                "heap_headroom_critical",
                RuntimeHealthStatus::Critical,
                "heap",
                minimum_heap,
                HEAP_CRITICAL_BYTES,
                "bytes",
            );
        } else if minimum_heap < HEAP_WARNING_BYTES {
            add_finding(
                &mut findings,
                "heap_headroom_low",
                RuntimeHealthStatus::Warning,
                "heap",
                minimum_heap,
                HEAP_WARNING_BYTES,
                "bytes",
            );
        }
    }

    if snapshot.bridge_output_queue_high_watermark == Some(OUTPUT_QUEUE_CAPACITY as u8) {
        add_finding(
            &mut findings,
            "bridge_output_queue_saturated",
            RuntimeHealthStatus::Warning,
            "outputQueue",
            OUTPUT_QUEUE_CAPACITY,
            OUTPUT_QUEUE_CAPACITY,
            "items",
        );
    }

    if snapshot.bridge_latency_samples.unwrap_or(0) >= BRIDGE_MIN_SAMPLES
        && let Some(p99_us) = snapshot.bridge_total_p99_us
    {
        let p99_us = u64::from(p99_us);
        if p99_us >= BRIDGE_P99_CRITICAL_US {
            add_finding(
                &mut findings,
                "bridge_p99_critical",
                RuntimeHealthStatus::Critical,
                "bridgeLatency",
                p99_us,
                BRIDGE_P99_CRITICAL_US,
                "us",
            );
        } else if p99_us >= BRIDGE_P99_WARNING_US {
            add_finding(
                &mut findings,
                "bridge_p99_high",
                RuntimeHealthStatus::Warning,
                "bridgeLatency",
                p99_us,
                BRIDGE_P99_WARNING_US,
                "us",
            );
        }
    }

    if snapshot.bridge_timing_flags.unwrap_or(0) != 0 {
        add_finding(
            &mut findings,
            "bridge_timing_saturated",
            RuntimeHealthStatus::Critical,
            "bridgeLatency",
            u64::from(snapshot.bridge_timing_flags.unwrap_or(0)),
            0,
            "flags",
        );
    }

    let status = if findings
        .iter()
        .any(|finding| finding.severity == RuntimeHealthStatus::Critical)
    {
        RuntimeHealthStatus::Critical
    } else if !findings.is_empty() {
        RuntimeHealthStatus::Warning
    } else if scheduler_data_available {
        RuntimeHealthStatus::Healthy
    } else {
        RuntimeHealthStatus::Unavailable
    };

    RuntimeHealthAssessment {
        status,
        scheduler_data_available,
        cpu_sample_available,
        findings,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDiagnostic {
    pub product_name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub firmware_version: Option<String>,
    pub build_profile: Option<String>,
    pub controller_firmware: Option<controller_identity::ControllerFirmwareInfo>,
    pub snapshot: Option<DiagnosticSnapshot>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
struct DiagnosticPage {
    protocol_version: u8,
    index: usize,
    count: usize,
    flags: u8,
    snapshot_seq: u32,
    monotonic_ms: u32,
    data: Vec<u8>,
}

#[cfg(windows)]
pub fn probe_runtime_diagnostics() -> Result<Vec<DeviceDiagnostic>> {
    let api = hidapi::HidApi::new().context("failed to initialize Windows HID access")?;
    let mut reports = Vec::new();
    for info in api.device_list().filter(|info| {
        info.vendor_id() == SONY_VENDOR_ID
            && DUALSENSE_PRODUCT_IDS.contains(&info.product_id())
            && info.usage_page() == 0x01
            && info.usage() == 0x05
    }) {
        let product_name = info
            .product_string()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("DS5DONGLE-AIM61")
            .to_owned();
        let mut report = DeviceDiagnostic {
            product_name,
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            firmware_version: None,
            build_profile: None,
            controller_firmware: None,
            snapshot: None,
            error: None,
        };
        match info.open_device(&api) {
            Ok(device) => {
                report.controller_firmware =
                    controller_identity::read_controller_firmware_info(&device, info.product_id());
                if let Some((version, profile)) = read_firmware_identity(&device) {
                    report.firmware_version = Some(version);
                    report.build_profile = Some(profile.clone());
                    match profile.as_str() {
                        "diagnostic" => match capture_snapshot(&device) {
                            Ok(snapshot) => report.snapshot = Some(snapshot),
                            Err(error) => report.error = Some(format!("{error:#}")),
                        },
                        "standard" => {}
                        unknown => {
                            report.error = Some(format!(
                                "firmware identity returned unsupported build profile '{unknown}'; runtime snapshot capture was not attempted"
                            ));
                        }
                    }
                } else {
                    report.error =
                        Some("unable to read the M61 firmware identity report".to_owned());
                }
            }
            Err(error) => report.error = Some(format!("unable to open HID device: {error}")),
        }
        reports.push(report);
    }
    reports.sort_by(|left, right| {
        (&left.product_name, left.product_id).cmp(&(&right.product_name, right.product_id))
    });
    Ok(reports)
}

#[cfg(not(windows))]
pub fn probe_runtime_diagnostics() -> Result<Vec<DeviceDiagnostic>> {
    bail!("runtime diagnostics are available on Windows only")
}

#[cfg(windows)]
fn read_firmware_identity(device: &hidapi::HidDevice) -> Option<(String, String)> {
    for attempt in 0..READ_ATTEMPTS {
        let mut report = [0_u8; 64];
        report[0] = FIRMWARE_VERSION_REPORT_ID;
        if let Ok(length) = device.get_feature_report(&mut report)
            && let Some(identity) = decode_firmware_identity_report(&report[..length])
        {
            return Some(identity);
        }
        if attempt + 1 < READ_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(READ_RETRY_DELAY_MS));
        }
    }
    None
}

#[cfg(windows)]
fn capture_snapshot(device: &hidapi::HidDevice) -> Result<DiagnosticSnapshot> {
    let primary = read_protocol_version(device).and_then(|protocol_version| {
        capture_snapshot_over_report(device, REPORT_ID, protocol_version)
    });
    match primary {
        Ok(snapshot) => Ok(snapshot),
        Err(primary_error) => capture_snapshot_over_report(device, FALLBACK_REPORT_ID, 2)
            .with_context(|| {
                format!(
                    "0xFD diagnostic transport failed ({primary_error:#}); the one-shot 0xF8 identity-channel fallback also failed"
                )
            }),
    }
}

#[cfg(windows)]
fn capture_snapshot_over_report(
    device: &hidapi::HidDevice,
    report_id: u8,
    protocol_version: u8,
) -> Result<DiagnosticSnapshot> {
    if protocol_version >= 2 {
        send_feature_report_with_retry(
            device,
            &session_report(report_id, protocol_version, true),
            "start the diagnostic session",
        )?;
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    let result = capture_snapshot_with_protocol(device, report_id, protocol_version);
    if protocol_version >= 2 {
        let _ = device.send_feature_report(&session_report(report_id, protocol_version, false));
    }
    result
}

#[cfg(windows)]
fn send_feature_report_with_retry(
    device: &hidapi::HidDevice,
    report: &[u8],
    operation: &str,
) -> Result<()> {
    let mut last_error = String::new();
    for attempt in 1..=READ_ATTEMPTS {
        match device.send_feature_report(report) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = format!("attempt {attempt}: {error}"),
        }
        if attempt < READ_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(READ_RETRY_DELAY_MS));
        }
    }
    bail!(
        "unable to {operation} after {READ_ATTEMPTS} attempts; last error: {last_error}. Close other HID/controller tools and reconnect the normal USB port"
    )
}

#[cfg(windows)]
fn read_protocol_version(device: &hidapi::HidDevice) -> Result<u8> {
    let mut last_error = String::new();
    for attempt in 1..=READ_ATTEMPTS {
        let mut report = [0_u8; WIRE_REPORT_SIZE];
        report[0] = REPORT_ID;
        match device.get_feature_report(&mut report) {
            Ok(length) => match decode_page(&report[..length]) {
                Ok(page) => return Ok(page.protocol_version),
                Err(error) => last_error = format!("attempt {attempt}: {error:#}"),
            },
            Err(error) => last_error = format!("attempt {attempt}: {error}"),
        }
        if attempt < READ_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(READ_RETRY_DELAY_MS));
        }
    }
    bail!(
        "unable to probe diagnostic protocol after {READ_ATTEMPTS} attempts; last error: {last_error}"
    )
}

#[cfg(windows)]
fn read_selected_page(
    device: &hidapi::HidDevice,
    report_id: u8,
    protocol_version: u8,
    page_index: usize,
) -> Result<DiagnosticPage> {
    let mut last_error = String::new();
    for attempt in 1..=READ_ATTEMPTS {
        let selector = selector_report(report_id, protocol_version, page_index as u8);
        match device.send_feature_report(&selector) {
            Ok(_) => {
                let mut report = [0_u8; WIRE_REPORT_SIZE];
                report[0] = report_id;
                match device.get_feature_report(&mut report) {
                    Ok(length) => match decode_page(&report[..length]) {
                        Ok(page) => return Ok(page),
                        Err(error) => last_error = format!("attempt {attempt}: {error:#}"),
                    },
                    Err(error) => last_error = format!("attempt {attempt}: {error}"),
                }
            }
            Err(error) => last_error = format!("attempt {attempt}: {error}"),
        }
        if attempt < READ_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(READ_RETRY_DELAY_MS));
        }
    }
    bail!(
        "unable to read diagnostic page {page_index} after {READ_ATTEMPTS} attempts; last error: {last_error}. Close other HID/controller tools and reconnect the normal USB port"
    )
}

#[cfg(windows)]
fn capture_snapshot_with_protocol(
    device: &hidapi::HidDevice,
    report_id: u8,
    protocol_version: u8,
) -> Result<DiagnosticSnapshot> {
    for _attempt in 0..4 {
        let mut pages = Vec::new();
        let mut page_count = 1_usize;
        let mut inconsistent = false;
        let mut page_index = 0_usize;
        let mut expected_sequence = None;
        let mut expected_monotonic = None;

        while page_index < page_count {
            let page = read_selected_page(device, report_id, protocol_version, page_index)?;
            if page.index != page_index {
                inconsistent = true;
                break;
            }
            if page_index == 0 {
                page_count = page.count;
                expected_sequence = Some(page.snapshot_seq);
                expected_monotonic = Some(page.monotonic_ms);
            } else if page.count != page_count
                || Some(page.snapshot_seq) != expected_sequence
                || Some(page.monotonic_ms) != expected_monotonic
            {
                inconsistent = true;
                break;
            }
            pages.push(page);
            page_index += 1;
        }

        if !inconsistent && pages.len() == page_count {
            let snapshot = decode_snapshot(&pages)?;
            validate_runtime_snapshot(&snapshot)?;
            return Ok(snapshot);
        }
    }
    bail!("diagnostic snapshot changed during paged capture; retry")
}

pub fn validate_runtime_snapshot(snapshot: &DiagnosticSnapshot) -> Result<()> {
    if snapshot.build_profile != "diagnostic" {
        bail!("runtime snapshot does not identify the diagnostic profile");
    }
    if snapshot.raw_pages.len() < REQUIRED_PAGES {
        bail!(
            "runtime snapshot contains only {} page(s); at least {REQUIRED_PAGES} are required",
            snapshot.raw_pages.len()
        );
    }
    if snapshot.snapshot_seq == 0 || snapshot.monotonic_ms == 0 || snapshot.uptime_ms == 0 {
        bail!(
            "runtime snapshot is only the boot placeholder (sequence={}, monotonic_ms={}, uptime_ms={})",
            snapshot.snapshot_seq,
            snapshot.monotonic_ms,
            snapshot.uptime_ms
        );
    }
    if snapshot.raw_flags & 0x03 != 0x03 {
        bail!(
            "runtime snapshot is not marked valid and coherent (flags=0x{:02x})",
            snapshot.raw_flags
        );
    }
    if snapshot.raw_pages.iter().any(|page| page.is_empty()) {
        bail!("runtime snapshot contains an empty decoded page");
    }
    Ok(())
}

pub fn microphone_error_delta(before: &DiagnosticSnapshot, after: &DiagnosticSnapshot) -> u32 {
    let mut delta = after
        .mic_underruns
        .wrapping_sub(before.mic_underruns)
        .wrapping_add(after.mic_overruns.wrapping_sub(before.mic_overruns))
        .wrapping_add(
            after
                .mic_endpoint_write_errors
                .wrapping_sub(before.mic_endpoint_write_errors),
        );
    if let (Some(before), Some(after)) = (before.mic_queue_overruns, after.mic_queue_overruns) {
        delta = delta.wrapping_add(after.wrapping_sub(before));
    }
    if let (Some(before), Some(after)) = (before.mic_decode_errors, after.mic_decode_errors) {
        delta = delta.wrapping_add(after.wrapping_sub(before));
    }
    delta
}

pub fn audio_error_delta(before: &DiagnosticSnapshot, after: &DiagnosticSnapshot) -> u32 {
    after
        .audio_pairs_rejected
        .wrapping_sub(before.audio_pairs_rejected)
        .wrapping_add(
            after
                .audio_encode_errors
                .wrapping_sub(before.audio_encode_errors),
        )
        .wrapping_add(
            after
                .audio_deadline_misses
                .wrapping_sub(before.audio_deadline_misses),
        )
        .wrapping_add(
            after
                .pcm_blocks_dropped
                .wrapping_sub(before.pcm_blocks_dropped),
        )
        .wrapping_add(
            after
                .pcm_pool_starvations
                .wrapping_sub(before.pcm_pool_starvations),
        )
        .wrapping_add(
            after
                .pcm_queue_overruns
                .wrapping_sub(before.pcm_queue_overruns),
        )
}

fn selector_report(report_id: u8, protocol_version: u8, page_index: u8) -> [u8; WIRE_REPORT_SIZE] {
    let mut report = [0_u8; WIRE_REPORT_SIZE];
    report[0] = report_id;
    report[1] = 0x01;
    report[2] = protocol_version;
    report[3] = page_index;
    report
}

fn session_report(report_id: u8, protocol_version: u8, active: bool) -> [u8; WIRE_REPORT_SIZE] {
    let mut report = [0_u8; WIRE_REPORT_SIZE];
    report[0] = report_id;
    report[1] = 0x02;
    report[2] = protocol_version;
    report[3] = u8::from(active);
    report
}

fn decode_page(source: &[u8]) -> Result<DiagnosticPage> {
    let requested_report_id = source
        .first()
        .copied()
        .filter(|report_id| matches!(*report_id, REPORT_ID | FALLBACK_REPORT_ID))
        .unwrap_or(REPORT_ID);
    let all_zero_payload = !source.is_empty()
        && (source.iter().all(|byte| *byte == 0)
            || matches!(
                source,
                [report_id, payload @ ..]
                    if matches!(*report_id, REPORT_ID | FALLBACK_REPORT_ID)
                        && payload.len() == REPORT_SIZE
                        && payload.iter().all(|byte| *byte == 0)
            ));
    if all_zero_payload {
        bail!(
            "the Windows HID stack returned an all-zero 0x{requested_report_id:02X} report; no usable runtime snapshot data is present on this transport. Close controller tools, reconnect M61, and retry"
        );
    }
    let frame = match source {
        [report_id, payload @ ..]
            if matches!(*report_id, REPORT_ID | FALLBACK_REPORT_ID)
                && payload.len() == REPORT_SIZE =>
        {
            payload
        }
        [b'D', b'G', ..] if source.len() == WIRE_REPORT_SIZE => &source[..REPORT_SIZE],
        _ if source.len() == REPORT_SIZE => source,
        _ => {
            let head = hex::encode(&source[..source.len().min(16)]);
            bail!(
                "diagnostic report length/layout is invalid: wire_length={}, expected_payload={REPORT_SIZE}, head={head}",
                source.len()
            );
        }
    };
    let expected_crc = read_u32(frame, CRC_OFFSET);
    let actual_crc = crc32fast::hash(&frame[..CRC_OFFSET]);
    if expected_crc != actual_crc {
        let head = hex::encode(&frame[..16]);
        bail!(
            "diagnostic report CRC32 mismatch: wire_length={}, expected=0x{expected_crc:08x}, actual=0x{actual_crc:08x}, head={head}",
            source.len()
        );
    }
    if frame[0..2] != *b"DG" {
        bail!("device does not support the DG diagnostic protocol");
    }
    if !matches!(frame[2], 1 | 2) {
        bail!("unsupported diagnostic protocol version {}", frame[2]);
    }
    if frame[3] as usize != HEADER_SIZE {
        bail!("invalid diagnostic header size {}", frame[3]);
    }
    let index = frame[4] as usize;
    let count = frame[5] as usize;
    let data_length = frame[6] as usize;
    if !(REQUIRED_PAGES..=MAX_PAGES).contains(&count) || index >= count {
        bail!("invalid diagnostic page {index}/{count}");
    }
    if data_length > DATA_SIZE {
        bail!("diagnostic page data is too long: {data_length}");
    }
    if frame[HEADER_SIZE + data_length..CRC_OFFSET]
        .iter()
        .any(|byte| *byte != 0)
    {
        bail!("diagnostic page contains non-zero padding");
    }
    Ok(DiagnosticPage {
        protocol_version: frame[2],
        index,
        count,
        flags: frame[7],
        snapshot_seq: read_u32(frame, 8),
        monotonic_ms: read_u32(frame, 12),
        data: frame[HEADER_SIZE..HEADER_SIZE + data_length].to_vec(),
    })
}

fn decode_snapshot(pages: &[DiagnosticPage]) -> Result<DiagnosticSnapshot> {
    let first = pages.first().context("diagnostic snapshot has no pages")?;
    if pages.len() != first.count {
        bail!("diagnostic snapshot is incomplete");
    }
    for (index, page) in pages.iter().enumerate() {
        if page.index != index
            || page.count != first.count
            || page.snapshot_seq != first.snapshot_seq
            || page.monotonic_ms != first.monotonic_ms
        {
            bail!("diagnostic snapshot pages are inconsistent");
        }
    }
    let p0 = page_data(pages, 0);
    let p1 = page_data(pages, 1);
    let p2 = page_data(pages, 2);
    let p3 = page_data(pages, 3);
    let p4 = page_data(pages, 4);
    let p5 = page_data(pages, 5);
    let p6 = page_data(pages, 6);
    let p7 = page_data(pages, 7);
    let p8 = page_data(pages, 8);
    let bridge_supported = p6.len() >= 24 && read_u8(p6, 23, 0) == 1;
    let mic_pipeline_supported = p7.len() >= 26 && read_u8(p7, 24, 0) == 1;
    let scheduler_supported = p8.len() >= 43
        && read_u8(p8, 0, 0) == 1
        && read_u8(p8, 2, 0) as usize == SCHEDULER_TASK_NAMES.len();
    let scheduler_cpu_valid = scheduler_supported && read_u8(p8, 1, 0) & 1 != 0;
    let scheduler_tasks = scheduler_supported.then(|| {
        SCHEDULER_TASK_NAMES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let stack = read_u16(p8, 4 + index * 2);
                let cpu = read_u16(p8, 18 + index * 2);
                let priority = read_u8(p8, 32 + index, u8::MAX);
                FirmwareTaskMetric {
                    name: (*name).to_owned(),
                    stack_min_free_words: (stack != u16::MAX).then_some(stack),
                    cpu_permille: (scheduler_cpu_valid && cpu != u16::MAX).then_some(cpu),
                    priority: (priority != u8::MAX).then_some(priority),
                }
            })
            .collect()
    });
    let rssi = read_i8(p0, 6);
    let battery = read_u8(p0, 7, 0xff);
    Ok(DiagnosticSnapshot {
        protocol_version: first.protocol_version,
        build_profile: "diagnostic".to_owned(),
        snapshot_seq: first.snapshot_seq,
        captured_at_unix_ms: now_unix_ms(),
        monotonic_ms: first.monotonic_ms,
        raw_flags: first.flags,
        uptime_ms: read_u32(p0, 0),
        bt_state: read_u8(p0, 4, 0),
        health_flags: read_u8(p0, 5, 0),
        heap_free_bytes: read_u32(p5, 0),
        heap_min_free_bytes: read_u32(p5, 4),
        memory_status_flags: read_u8(p5, 42, 0),
        bt_rssi_dbm: (rssi != 127).then_some(rssi),
        battery_percent: (battery <= 100).then_some(battery),
        battery_state: read_u8(p0, 8, 0xff),
        usb_in_completed: read_u32(p1, 0),
        bt_input_reports: read_u32(p1, 4),
        bt_output_completed: read_u32(p1, 8),
        usb_input_updates: read_u32(p1, 12),
        usb_input_coalesced: read_u32(p1, 16),
        usb_transfers_started: read_u32(p1, 20),
        usb_start_errors: read_u32(p1, 24),
        bt_game_enqueued: read_u32(p1, 28),
        bt_game_coalesced: read_u32(p1, 32),
        bt_game_completed: read_u32(p1, 36),
        bt_game_pending: read_u8(p1, 40, 0) & 1 != 0,
        bt_tx_in_flight: read_u8(p1, 40, 0) & 2 != 0,
        bt_audio_queue_depth: read_u8(p1, 41, 0),
        bt_control_queue_depth: read_u8(p1, 42, 0),
        pcm_blocks_queued: read_u32(p4, 0),
        bt_audio_pairs_submitted: read_u32(p4, 4),
        mic_underruns: read_u32(p4, 8),
        mic_overruns: read_u32(p4, 12),
        loss_pressure: read_u32(p2, 0),
        bt_audio_backpressure: read_u32(p2, 8),
        bt_audio_dropped_stale: read_u32(p2, 12),
        bt_control_backpressure: read_u32(p2, 16),
        bt_alloc_failures: read_u32(p2, 20),
        bt_send_failures: read_u32(p2, 24),
        bt_stale_completions: read_u32(p2, 28),
        bt_audio_queue_high_watermark: read_u8(p2, 40, 0),
        bt_control_queue_high_watermark: read_u8(p2, 41, 0),
        audio_frames_processed: read_u32(p3, 0),
        audio_pairs_rejected: read_u32(p3, 8),
        audio_discontinuities: read_u32(p3, 12),
        audio_encode_errors: read_u32(p3, 16),
        audio_pipeline_resets: read_u32(p3, 20),
        audio_deadline_misses: read_u32(p3, 24),
        audio_block_p99_us: read_u16(p3, 28),
        audio_block_max_us: read_u16(p3, 30),
        audio_resample_p99_us: read_u16(p3, 32),
        audio_resample_max_us: read_u16(p3, 34),
        audio_encode_p99_us: read_u16(p3, 36),
        audio_encode_max_us: read_u16(p3, 38),
        audio_timing_flags: read_u8(p3, 40, 0),
        pcm_blocks_dropped: read_u32(p4, 16),
        pcm_pool_starvations: read_u32(p4, 20),
        pcm_queue_overruns: read_u32(p4, 24),
        pcm_stale_blocks: read_u32(p4, 28),
        pcm_ready_high_watermark: read_u32(p4, 32),
        mic_endpoint_write_errors: read_u32(p4, 36),
        speaker_stream_active: read_u8(p4, 40, 0) & 1 != 0,
        microphone_stream_active: read_u8(p4, 40, 0) & 2 != 0,
        mic_frames_received: mic_pipeline_supported.then(|| read_u32(p7, 0)),
        mic_frames_decoded: mic_pipeline_supported.then(|| read_u32(p7, 4)),
        mic_queue_overruns: mic_pipeline_supported.then(|| read_u32(p7, 8)),
        mic_decode_errors: mic_pipeline_supported.then(|| read_u32(p7, 12)),
        mic_queue_high_watermark: mic_pipeline_supported.then(|| read_u32(p7, 16)),
        mic_decode_p99_us: mic_pipeline_supported.then(|| read_u16(p7, 20)),
        mic_decode_max_us: mic_pipeline_supported.then(|| read_u16(p7, 22)),
        mic_decode_timing_flags: mic_pipeline_supported.then(|| read_u8(p7, 25, 0)),
        ota_state: read_u8(p5, 8, 0xff),
        ota_error: read_u8(p5, 9, 0xff),
        bridge_latency_samples: bridge_supported.then(|| read_u32(p6, 0)),
        bridge_rx_to_submit_avg_us: bridge_supported.then(|| read_u16(p6, 4)),
        bridge_rx_to_submit_max_us: bridge_supported.then(|| read_u16(p6, 6)),
        bridge_usb_transfer_avg_us: bridge_supported.then(|| read_u16(p6, 8)),
        bridge_usb_transfer_max_us: bridge_supported.then(|| read_u16(p6, 10)),
        bridge_total_avg_us: bridge_supported.then(|| read_u16(p6, 12)),
        bridge_total_p95_us: bridge_supported.then(|| read_u16(p6, 14)),
        bridge_total_p99_us: bridge_supported.then(|| read_u16(p6, 16)),
        bridge_total_max_us: bridge_supported.then(|| read_u16(p6, 18)),
        bridge_timing_flags: bridge_supported.then(|| read_u8(p6, 22, 0)),
        scheduler_tasks,
        bridge_input_queue_depth: scheduler_supported.then(|| read_u8(p8, 39, 0)),
        bridge_output_queue_depth: scheduler_supported.then(|| read_u8(p8, 40, 0)),
        bridge_input_queue_high_watermark: scheduler_supported.then(|| read_u8(p8, 41, 0)),
        bridge_output_queue_high_watermark: scheduler_supported.then(|| read_u8(p8, 42, 0)),
        raw_pages: pages.iter().map(|page| hex::encode(&page.data)).collect(),
    })
}

fn page_data(pages: &[DiagnosticPage], index: usize) -> &[u8] {
    pages.get(index).map_or(&[], |page| page.data.as_slice())
}

fn read_u8(source: &[u8], offset: usize, fallback: u8) -> u8 {
    source.get(offset).copied().unwrap_or(fallback)
}

fn read_i8(source: &[u8], offset: usize) -> i8 {
    read_u8(source, offset, 127) as i8
}

fn read_u32(source: &[u8], offset: usize) -> u32 {
    source
        .get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

fn read_u16(source: &[u8], offset: usize) -> u16 {
    source
        .get(offset..offset + 2)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u16::from_le_bytes)
        .unwrap_or(0)
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_page(index: u8, count: u8, sequence: u32, monotonic: u32, data: &[u8]) -> Vec<u8> {
        let mut page = vec![0_u8; REPORT_SIZE];
        page[0..2].copy_from_slice(b"DG");
        page[2] = 1;
        page[3] = HEADER_SIZE as u8;
        page[4] = index;
        page[5] = count;
        page[6] = data.len() as u8;
        page[7] = 3;
        page[8..12].copy_from_slice(&sequence.to_le_bytes());
        page[12..16].copy_from_slice(&monotonic.to_le_bytes());
        page[16..16 + data.len()].copy_from_slice(data);
        let crc = crc32fast::hash(&page[..CRC_OFFSET]);
        page[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        page
    }

    #[test]
    fn decodes_prefixed_crc_checked_page() {
        let page = make_page(0, 6, 42, 123_456, &[1, 2, 3]);
        let mut prefixed = vec![REPORT_ID];
        prefixed.extend_from_slice(&page);
        let decoded = decode_page(&prefixed).unwrap();
        assert_eq!(decoded.index, 0);
        assert_eq!(decoded.count, 6);
        assert_eq!(decoded.snapshot_seq, 42);
        assert_eq!(decoded.monotonic_ms, 123_456);
        assert_eq!(decoded.data, [1, 2, 3]);

        let mut trailing = page.clone();
        trailing.push(0);
        assert_eq!(decode_page(&trailing).unwrap().snapshot_seq, 42);

        let mut corrupt = page;
        corrupt[16] ^= 1;
        let error = decode_page(&corrupt).unwrap_err().to_string();
        assert!(error.contains("CRC32 mismatch"));
        assert!(error.contains("expected=0x"));
        assert!(error.contains("actual=0x"));
        assert!(error.contains("head=4447"));
    }

    #[test]
    fn rejects_truncated_or_oversized_diagnostic_frames() {
        let page = make_page(0, 6, 42, 123_456, &[1, 2, 3]);
        let truncated = &page[..REPORT_SIZE - 1];
        assert!(
            decode_page(truncated)
                .unwrap_err()
                .to_string()
                .contains("wire_length=62")
        );

        let mut oversized = page;
        oversized.extend_from_slice(&[0, 0]);
        assert!(
            decode_page(&oversized)
                .unwrap_err()
                .to_string()
                .contains("wire_length=65")
        );
    }

    #[test]
    fn diagnostic_selector_uses_the_full_hid_report_length() {
        let selector = selector_report(REPORT_ID, 2, 5);
        assert_eq!(selector.len(), 64);
        assert_eq!(&selector[..4], &[REPORT_ID, 0x01, 0x02, 5]);
        assert!(selector[4..].iter().all(|byte| *byte == 0));

        let fallback = selector_report(FALLBACK_REPORT_ID, 2, 5);
        assert_eq!(fallback.len(), 64);
        assert_eq!(&fallback[..4], &[FALLBACK_REPORT_ID, 0x01, 0x02, 5]);
        assert!(fallback[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn decodes_cache_safe_f8_prefixed_diagnostic_page() {
        let page = make_page(0, 6, 42, 123_456, &[1, 2, 3]);
        let mut prefixed = vec![FALLBACK_REPORT_ID];
        prefixed.extend_from_slice(&page);
        let decoded = decode_page(&prefixed).unwrap();
        assert_eq!(decoded.index, 0);
        assert_eq!(decoded.snapshot_seq, 42);
        assert_eq!(decoded.data, [1, 2, 3]);
    }

    #[test]
    fn rejects_all_zero_feature_reports_before_snapshot_decode() {
        for report in [
            [0_u8; WIRE_REPORT_SIZE],
            {
                let mut report = [0_u8; WIRE_REPORT_SIZE];
                report[0] = REPORT_ID;
                report
            },
            {
                let mut report = [0_u8; WIRE_REPORT_SIZE];
                report[0] = FALLBACK_REPORT_ID;
                report
            },
        ] {
            let error = decode_page(&report).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("no usable runtime snapshot data is present")
            );
        }
    }

    #[test]
    fn decodes_native_runtime_summary_fields() {
        let mut data = vec![vec![0_u8; DATA_SIZE]; REQUIRED_PAGES];
        data[0][0..4].copy_from_slice(&99_000_u32.to_le_bytes());
        data[0][4..9].copy_from_slice(&[3, 0xa5, 0xd6, 87, 1]);
        data[1][0..4].copy_from_slice(&1001_u32.to_le_bytes());
        data[1][4..8].copy_from_slice(&2002_u32.to_le_bytes());
        data[1][8..12].copy_from_slice(&3003_u32.to_le_bytes());
        data[1][12..16].copy_from_slice(&3100_u32.to_le_bytes());
        data[1][16..20].copy_from_slice(&11_u32.to_le_bytes());
        data[1][20..24].copy_from_slice(&3200_u32.to_le_bytes());
        data[1][24..28].copy_from_slice(&2_u32.to_le_bytes());
        data[1][28..32].copy_from_slice(&3300_u32.to_le_bytes());
        data[1][32..36].copy_from_slice(&12_u32.to_le_bytes());
        data[1][36..40].copy_from_slice(&3288_u32.to_le_bytes());
        data[1][40..43].copy_from_slice(&[3, 2, 1]);
        data[2][0..4].copy_from_slice(&7_u32.to_le_bytes());
        data[2][8..12].copy_from_slice(&1_u32.to_le_bytes());
        data[2][12..16].copy_from_slice(&2_u32.to_le_bytes());
        data[2][16..20].copy_from_slice(&3_u32.to_le_bytes());
        data[2][20..24].copy_from_slice(&4_u32.to_le_bytes());
        data[2][24..28].copy_from_slice(&5_u32.to_le_bytes());
        data[2][28..32].copy_from_slice(&6_u32.to_le_bytes());
        data[2][40..42].copy_from_slice(&[3, 2]);
        data[3][0..4].copy_from_slice(&41_u32.to_le_bytes());
        data[3][8..12].copy_from_slice(&1_u32.to_le_bytes());
        data[3][12..16].copy_from_slice(&2_u32.to_le_bytes());
        data[3][16..20].copy_from_slice(&3_u32.to_le_bytes());
        data[3][20..24].copy_from_slice(&4_u32.to_le_bytes());
        data[3][24..28].copy_from_slice(&5_u32.to_le_bytes());
        for (offset, value) in [
            (28, 100_u16),
            (30, 200_u16),
            (32, 300_u16),
            (34, 400_u16),
            (36, 500_u16),
            (38, 600_u16),
        ] {
            data[3][offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        data[3][40] = 4;
        data[4][0..4].copy_from_slice(&4004_u32.to_le_bytes());
        data[4][4..8].copy_from_slice(&5005_u32.to_le_bytes());
        data[4][8..12].copy_from_slice(&6_u32.to_le_bytes());
        data[4][12..16].copy_from_slice(&8_u32.to_le_bytes());
        data[4][16..20].copy_from_slice(&9_u32.to_le_bytes());
        data[4][20..24].copy_from_slice(&10_u32.to_le_bytes());
        data[4][24..28].copy_from_slice(&11_u32.to_le_bytes());
        data[4][28..32].copy_from_slice(&12_u32.to_le_bytes());
        data[4][32..36].copy_from_slice(&3_u32.to_le_bytes());
        data[4][36..40].copy_from_slice(&13_u32.to_le_bytes());
        data[4][40] = 3;
        data[5][0..4].copy_from_slice(&(64_u32 * 1024).to_le_bytes());
        data[5][4..8].copy_from_slice(&(48_u32 * 1024).to_le_bytes());
        data[5][8..10].copy_from_slice(&[2, 0]);
        let pages = data
            .iter()
            .enumerate()
            .map(|(index, data)| decode_page(&make_page(index as u8, 6, 9, 99_000, data)).unwrap())
            .collect::<Vec<_>>();
        let snapshot = decode_snapshot(&pages).unwrap();
        assert_eq!(snapshot.uptime_ms, 99_000);
        assert_eq!(snapshot.bt_state, 3);
        assert_eq!(snapshot.bt_rssi_dbm, Some(-42));
        assert_eq!(snapshot.battery_percent, Some(87));
        assert_eq!(snapshot.usb_in_completed, 1001);
        assert_eq!(snapshot.usb_input_updates, 3100);
        assert_eq!(snapshot.usb_input_coalesced, 11);
        assert_eq!(snapshot.usb_start_errors, 2);
        assert!(snapshot.bt_game_pending);
        assert!(snapshot.bt_tx_in_flight);
        assert_eq!(snapshot.bt_audio_queue_depth, 2);
        assert_eq!(snapshot.loss_pressure, 7);
        assert_eq!(snapshot.bt_audio_backpressure, 1);
        assert_eq!(snapshot.bt_send_failures, 5);
        assert_eq!(snapshot.audio_frames_processed, 41);
        assert_eq!(snapshot.audio_encode_errors, 3);
        assert_eq!(snapshot.audio_encode_max_us, 600);
        assert_eq!(snapshot.mic_underruns, 6);
        assert_eq!(snapshot.pcm_blocks_dropped, 9);
        assert_eq!(snapshot.mic_endpoint_write_errors, 13);
        assert!(snapshot.speaker_stream_active);
        assert!(snapshot.microphone_stream_active);
        assert_eq!(snapshot.heap_free_bytes, 64 * 1024);
        assert_eq!(snapshot.ota_state, 2);
        assert_eq!(snapshot.raw_pages.len(), 6);
        assert_eq!(snapshot.bridge_latency_samples, None);
        validate_runtime_snapshot(&snapshot).unwrap();
    }

    #[test]
    fn rejects_boot_placeholder_as_report_data() {
        let data = vec![vec![0_u8; DATA_SIZE]; REQUIRED_PAGES];
        let pages = data
            .iter()
            .enumerate()
            .map(|(index, data)| {
                decode_page(&make_page(index as u8, REQUIRED_PAGES as u8, 0, 0, data)).unwrap()
            })
            .collect::<Vec<_>>();
        let snapshot = decode_snapshot(&pages).unwrap();
        assert!(
            validate_runtime_snapshot(&snapshot)
                .unwrap_err()
                .to_string()
                .contains("boot placeholder")
        );
    }

    #[test]
    fn decodes_bridge_latency_page_and_keeps_old_snapshots_compatible() {
        let mut data = vec![vec![0_u8; DATA_SIZE]; 7];
        data[6][0..4].copy_from_slice(&742_u32.to_le_bytes());
        for (offset, value) in [
            (4, 180_u16),
            (6, 900_u16),
            (8, 320_u16),
            (10, 1100_u16),
            (12, 500_u16),
            (14, 750_u16),
            (16, 1500_u16),
            (18, 2100_u16),
            (20, 1000_u16),
        ] {
            data[6][offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        data[6][22] = 0;
        data[6][23] = 1;
        let pages = data
            .iter()
            .enumerate()
            .map(|(index, data)| {
                decode_page(&make_page(index as u8, 7, 10, 100_000, data)).unwrap()
            })
            .collect::<Vec<_>>();

        let snapshot = decode_snapshot(&pages).unwrap();
        assert_eq!(snapshot.bridge_latency_samples, Some(742));
        assert_eq!(snapshot.bridge_rx_to_submit_avg_us, Some(180));
        assert_eq!(snapshot.bridge_usb_transfer_avg_us, Some(320));
        assert_eq!(snapshot.bridge_total_avg_us, Some(500));
        assert_eq!(snapshot.bridge_total_p95_us, Some(750));
        assert_eq!(snapshot.bridge_total_p99_us, Some(1500));
        assert_eq!(snapshot.bridge_total_max_us, Some(2100));
        assert_eq!(snapshot.bridge_timing_flags, Some(0));
        assert_eq!(snapshot.raw_pages.len(), 7);
        assert_eq!(snapshot.mic_frames_received, None);
    }

    #[test]
    fn decodes_optional_microphone_pipeline_page() {
        let mut data = vec![vec![0_u8; DATA_SIZE]; 8];
        for (offset, value) in [
            (0, 10_001_u32),
            (4, 9_998_u32),
            (8, 2_u32),
            (12, 1_u32),
            (16, 4_u32),
        ] {
            data[7][offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[7][20..22].copy_from_slice(&750_u16.to_le_bytes());
        data[7][22..24].copy_from_slice(&1250_u16.to_le_bytes());
        data[7][24] = 1;
        data[7][25] = 0;
        let pages = data
            .iter()
            .enumerate()
            .map(|(index, data)| {
                decode_page(&make_page(index as u8, 8, 11, 101_000, data)).unwrap()
            })
            .collect::<Vec<_>>();

        let snapshot = decode_snapshot(&pages).unwrap();
        assert_eq!(snapshot.mic_frames_received, Some(10_001));
        assert_eq!(snapshot.mic_frames_decoded, Some(9_998));
        assert_eq!(snapshot.mic_queue_overruns, Some(2));
        assert_eq!(snapshot.mic_decode_errors, Some(1));
        assert_eq!(snapshot.mic_queue_high_watermark, Some(4));
        assert_eq!(snapshot.mic_decode_p99_us, Some(750));
        assert_eq!(snapshot.mic_decode_max_us, Some(1250));
        assert_eq!(snapshot.raw_pages.len(), 8);
    }

    #[test]
    fn decodes_optional_scheduler_page() {
        let mut data = vec![vec![0_u8; DATA_SIZE]; 9];
        data[8][0] = 1;
        data[8][1] = 1;
        data[8][2] = 7;
        for index in 0..7 {
            let stack = 100_u16 + index as u16;
            let cpu = 10_u16 + index as u16;
            data[8][4 + index * 2..6 + index * 2].copy_from_slice(&stack.to_le_bytes());
            data[8][18 + index * 2..20 + index * 2].copy_from_slice(&cpu.to_le_bytes());
            data[8][32 + index] = 6 - index as u8;
        }
        data[8][39] = 1;
        data[8][40] = 2;
        data[8][41] = 1;
        data[8][42] = 5;
        let pages = data
            .iter()
            .enumerate()
            .map(|(index, data)| {
                decode_page(&make_page(index as u8, 9, 12, 102_000, data)).unwrap()
            })
            .collect::<Vec<_>>();

        let snapshot = decode_snapshot(&pages).unwrap();
        let tasks = snapshot.scheduler_tasks.as_ref().unwrap();
        assert_eq!(tasks.len(), 7);
        assert_eq!(tasks[0].name, "bt");
        assert_eq!(tasks[0].stack_min_free_words, Some(100));
        assert_eq!(tasks[0].cpu_permille, Some(10));
        assert_eq!(tasks[0].priority, Some(6));
        assert_eq!(snapshot.bridge_input_queue_depth, Some(1));
        assert_eq!(snapshot.bridge_output_queue_depth, Some(2));
        assert_eq!(snapshot.bridge_input_queue_high_watermark, Some(1));
        assert_eq!(snapshot.bridge_output_queue_high_watermark, Some(5));
        assert_eq!(snapshot.raw_pages.len(), 9);

        let assessment = assess_runtime_health(&snapshot);
        assert_eq!(assessment.status, RuntimeHealthStatus::Critical);
        assert!(assessment.scheduler_data_available);
        assert!(assessment.cpu_sample_available);
        assert!(
            assessment
                .findings
                .iter()
                .any(|finding| finding.code == "task_stack_critical")
        );
        assert!(
            assessment
                .findings
                .iter()
                .any(|finding| finding.code == "bridge_output_queue_saturated")
        );
    }
}
