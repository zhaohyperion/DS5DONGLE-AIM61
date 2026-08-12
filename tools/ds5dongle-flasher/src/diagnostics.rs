use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    DUALSENSE_PRODUCT_IDS, FIRMWARE_VERSION_REPORT_ID, SONY_VENDOR_ID,
    decode_firmware_version_report,
};

pub const REPORT_ID: u8 = 0xfd;
const REPORT_SIZE: usize = 63;
const WIRE_REPORT_SIZE: usize = REPORT_SIZE + 1;
const CRC_OFFSET: usize = 59;
const HEADER_SIZE: usize = 16;
const DATA_SIZE: usize = 43;
const REQUIRED_PAGES: usize = 6;
const MAX_PAGES: usize = 16;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSnapshot {
    pub snapshot_seq: u32,
    pub captured_at_unix_ms: u64,
    pub monotonic_ms: u32,
    pub raw_flags: u8,
    pub uptime_ms: u32,
    pub bt_state: u8,
    pub health_flags: u8,
    pub heap_free_bytes: u32,
    pub heap_min_free_bytes: u32,
    pub bt_rssi_dbm: Option<i8>,
    pub battery_percent: Option<u8>,
    pub battery_state: u8,
    pub usb_in_completed: u32,
    pub bt_input_reports: u32,
    pub bt_output_completed: u32,
    pub pcm_blocks_queued: u32,
    pub bt_audio_pairs_submitted: u32,
    pub mic_underruns: u32,
    pub mic_overruns: u32,
    pub loss_pressure: u32,
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
    pub raw_pages: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDiagnostic {
    pub product_name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub firmware_version: Option<String>,
    pub snapshot: Option<DiagnosticSnapshot>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
struct DiagnosticPage {
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
            snapshot: None,
            error: None,
        };
        match info.open_device(&api) {
            Ok(device) => {
                report.firmware_version = read_firmware_version(&device);
                match capture_snapshot(&device) {
                    Ok(snapshot) => report.snapshot = Some(snapshot),
                    Err(error) => report.error = Some(format!("{error:#}")),
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
fn read_firmware_version(device: &hidapi::HidDevice) -> Option<String> {
    let mut report = [0_u8; 64];
    report[0] = FIRMWARE_VERSION_REPORT_ID;
    let length = device.get_feature_report(&mut report).ok()?;
    decode_firmware_version_report(&report[..length])
}

#[cfg(windows)]
fn capture_snapshot(device: &hidapi::HidDevice) -> Result<DiagnosticSnapshot> {
    for _attempt in 0..4 {
        let mut pages = Vec::new();
        let mut page_count = 1_usize;
        let mut inconsistent = false;
        let mut page_index = 0_usize;
        let mut expected_sequence = None;
        let mut expected_monotonic = None;

        while page_index < page_count {
            let selector = selector_report(page_index as u8);
            device
                .send_feature_report(&selector)
                .with_context(|| {
                    format!(
                        "unable to select diagnostic page {page_index}; Windows rejected the full-length 0xFD Feature Report. Close other HID/controller tools, reconnect the normal USB port, then retry"
                    )
                })?;
            let mut report = [0_u8; WIRE_REPORT_SIZE];
            report[0] = REPORT_ID;
            let length = device
                .get_feature_report(&mut report)
                .with_context(|| format!("unable to read diagnostic page {page_index}"))?;
            let page = decode_page(&report[..length])?;
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
            return decode_snapshot(&pages);
        }
    }
    bail!("diagnostic snapshot changed during paged capture; retry")
}

fn selector_report(page_index: u8) -> [u8; WIRE_REPORT_SIZE] {
    let mut report = [0_u8; WIRE_REPORT_SIZE];
    report[0] = REPORT_ID;
    report[1] = 0x01;
    report[2] = 0x01;
    report[3] = page_index;
    report
}

fn decode_page(source: &[u8]) -> Result<DiagnosticPage> {
    let frame = if source.len() == REPORT_SIZE + 1 && source[0] == REPORT_ID {
        &source[1..]
    } else {
        source
    };
    if frame.len() != REPORT_SIZE {
        bail!("diagnostic report length is {}/{REPORT_SIZE}", frame.len());
    }
    let expected_crc = read_u32(frame, CRC_OFFSET);
    let actual_crc = crc32fast::hash(&frame[..CRC_OFFSET]);
    if expected_crc != actual_crc {
        bail!("diagnostic report CRC32 mismatch");
    }
    if frame[0..2] != *b"DG" {
        bail!("device does not support the DG diagnostic protocol");
    }
    if frame[2] != 1 {
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
    let p4 = page_data(pages, 4);
    let p5 = page_data(pages, 5);
    let p6 = page_data(pages, 6);
    let bridge_supported = p6.len() >= 24 && read_u8(p6, 23, 0) == 1;
    let rssi = read_i8(p0, 6);
    let battery = read_u8(p0, 7, 0xff);
    Ok(DiagnosticSnapshot {
        snapshot_seq: first.snapshot_seq,
        captured_at_unix_ms: now_unix_ms(),
        monotonic_ms: first.monotonic_ms,
        raw_flags: first.flags,
        uptime_ms: read_u32(p0, 0),
        bt_state: read_u8(p0, 4, 0),
        health_flags: read_u8(p0, 5, 0),
        heap_free_bytes: read_u32(p5, 0),
        heap_min_free_bytes: read_u32(p5, 4),
        bt_rssi_dbm: (rssi != 127).then_some(rssi),
        battery_percent: (battery <= 100).then_some(battery),
        battery_state: read_u8(p0, 8, 0xff),
        usb_in_completed: read_u32(p1, 0),
        bt_input_reports: read_u32(p1, 4),
        bt_output_completed: read_u32(p1, 8),
        pcm_blocks_queued: read_u32(p4, 0),
        bt_audio_pairs_submitted: read_u32(p4, 4),
        mic_underruns: read_u32(p4, 8),
        mic_overruns: read_u32(p4, 12),
        loss_pressure: read_u32(p2, 0),
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

        let mut corrupt = page;
        corrupt[16] ^= 1;
        assert!(decode_page(&corrupt).is_err());
    }

    #[test]
    fn diagnostic_selector_uses_the_full_hid_report_length() {
        let selector = selector_report(5);
        assert_eq!(selector.len(), 64);
        assert_eq!(&selector[..4], &[REPORT_ID, 0x01, 0x01, 5]);
        assert!(selector[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn decodes_native_runtime_summary_fields() {
        let mut data = vec![vec![0_u8; DATA_SIZE]; REQUIRED_PAGES];
        data[0][0..4].copy_from_slice(&99_000_u32.to_le_bytes());
        data[0][4..9].copy_from_slice(&[3, 0xa5, 0xd6, 87, 1]);
        data[1][0..4].copy_from_slice(&1001_u32.to_le_bytes());
        data[1][4..8].copy_from_slice(&2002_u32.to_le_bytes());
        data[1][8..12].copy_from_slice(&3003_u32.to_le_bytes());
        data[2][0..4].copy_from_slice(&7_u32.to_le_bytes());
        data[4][0..4].copy_from_slice(&4004_u32.to_le_bytes());
        data[4][4..8].copy_from_slice(&5005_u32.to_le_bytes());
        data[4][8..12].copy_from_slice(&6_u32.to_le_bytes());
        data[4][12..16].copy_from_slice(&8_u32.to_le_bytes());
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
        assert_eq!(snapshot.loss_pressure, 7);
        assert_eq!(snapshot.mic_underruns, 6);
        assert_eq!(snapshot.heap_free_bytes, 64 * 1024);
        assert_eq!(snapshot.ota_state, 2);
        assert_eq!(snapshot.raw_pages.len(), 6);
        assert_eq!(snapshot.bridge_latency_samples, None);
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
    }
}
