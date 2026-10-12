//! Read-only DualSense firmware identity decoding.
//!
//! The report layout and board-model mapping were cross-checked against
//! `KaiwetN/windows-vds` (`src/vds_device_info.cc`, MIT) and Linux
//! `hid-playstation`.  Privacy-sensitive shell serial and Bluetooth address
//! fields are deliberately not collected by the flasher.

use serde::Serialize;

pub const FIRMWARE_INFO_REPORT_ID: u8 = 0x20;
const DUALSENSE_EDGE_PRODUCT_ID: u16 = 0x0df2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerFirmwareInfo {
    pub update_version: String,
    pub main_firmware_version: String,
    pub hardware_version: String,
    pub hardware_model: Option<String>,
    pub build_time: Option<String>,
}

pub fn decode_firmware_info_report(
    report: &[u8],
    product_id: u16,
) -> Option<ControllerFirmwareInfo> {
    if report.len() < 46 || report[0] != FIRMWARE_INFO_REPORT_ID {
        return None;
    }

    let hardware = read_u32_le(report, 24)?;
    let firmware = read_u32_le(report, 28)?;
    let update = read_u16_le(report, 44)?;
    let build_time = format_build_time(
        ascii_field(report.get(1..12)?),
        ascii_field(report.get(12..20)?),
    );

    // usb_gamepad.c intentionally supplies this report while the Bluetooth
    // cache is empty so operating-system probes can complete.  Never present
    // those compatibility constants as information read from a real pad.
    if hardware == 0x0000_0200
        && firmware == 0x0000_0256
        && update == 0x0221
        && build_time.is_none()
    {
        return None;
    }

    // A live Sony report carries at least one meaningful identity field.
    // Reject all-zero and all-ones transport artefacts rather than exporting
    // plausible-looking A-0000/0x00000000 values.
    if matches!(hardware, 0 | u32::MAX)
        && matches!(firmware, 0 | u32::MAX)
        && matches!(update, 0 | u16::MAX)
        && build_time.is_none()
    {
        return None;
    }

    Some(ControllerFirmwareInfo {
        update_version: format!("A-{update:04X}"),
        main_firmware_version: format!("0x{firmware:08X}"),
        hardware_version: format!("0x{hardware:08X}"),
        hardware_model: hardware_model(hardware, product_id == DUALSENSE_EDGE_PRODUCT_ID)
            .map(str::to_owned),
        build_time,
    })
}

#[cfg(windows)]
pub fn read_controller_firmware_info(
    device: &hidapi::HidDevice,
    product_id: u16,
) -> Option<ControllerFirmwareInfo> {
    for attempt in 0..3 {
        let mut report = [0_u8; 64];
        report[0] = FIRMWARE_INFO_REPORT_ID;
        if let Ok(length) = device.get_feature_report(&mut report)
            && let Some(info) = decode_firmware_info_report(&report[..length], product_id)
        {
            return Some(info);
        }
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    None
}

fn read_u16_le(source: &[u8], offset: usize) -> Option<u16> {
    source
        .get(offset..offset + 2)?
        .try_into()
        .ok()
        .map(u16::from_le_bytes)
}

fn read_u32_le(source: &[u8], offset: usize) -> Option<u32> {
    source
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn ascii_field(bytes: &[u8]) -> &str {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or("").trim()
}

fn format_build_time(date: &str, time: &str) -> Option<String> {
    fn parse_decimal(value: &str) -> Option<u32> {
        (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| value.parse().ok())
            .flatten()
    }

    let numeric_date = if date.len() >= 10
        && date.as_bytes().get(4) == Some(&b'-')
        && date.as_bytes().get(7) == Some(&b'-')
    {
        Some((
            parse_decimal(date.get(0..4)?)?,
            parse_decimal(date.get(5..7)?)?,
            parse_decimal(date.get(8..10)?)?,
        ))
    } else {
        let mut parts = date.split_ascii_whitespace();
        let month = match parts.next()? {
            "Jan" => 1,
            "Feb" => 2,
            "Mar" => 3,
            "Apr" => 4,
            "May" => 5,
            "Jun" => 6,
            "Jul" => 7,
            "Aug" => 8,
            "Sep" => 9,
            "Oct" => 10,
            "Nov" => 11,
            "Dec" => 12,
            _ => return None,
        };
        let day = parse_decimal(parts.next()?)?;
        let year = parse_decimal(parts.next()?)?;
        (parts.next().is_none()).then_some((year, month, day))
    }?;
    let numeric_time = (time.len() >= 8).then(|| {
        Some((
            parse_decimal(time.get(0..2)?)?,
            parse_decimal(time.get(3..5)?)?,
            parse_decimal(time.get(6..8)?)?,
        ))
    })??;
    let (year, month, day) = numeric_date;
    let (hour, minute, second) = numeric_time;
    (month.clamp(1, 12) == month
        && day.clamp(1, 31) == day
        && hour < 24
        && minute < 60
        && second < 60)
        .then(|| format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"))
}

fn hardware_model(hardware_version: u32, edge: bool) -> Option<&'static str> {
    let board = (hardware_version >> 8) & 0xff;
    if edge {
        return (board == 2).then_some("HDM-010");
    }
    match board {
        2 => Some("HMB-010"),
        3 => Some("BDM-010"),
        4 => Some("BDM-020"),
        5 => Some("BDM-030"),
        6 => Some("BDM-040"),
        7 | 8 => Some("BDM-050"),
        17 => Some("BDM-060M"),
        19 => Some("BDM-060X"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(hardware: u32, firmware: u32, update: u16) -> [u8; 64] {
        let mut report = [0_u8; 64];
        report[0] = FIRMWARE_INFO_REPORT_ID;
        report[24..28].copy_from_slice(&hardware.to_le_bytes());
        report[28..32].copy_from_slice(&firmware.to_le_bytes());
        report[44..46].copy_from_slice(&update.to_le_bytes());
        report
    }

    #[test]
    fn decodes_live_dualsense_identity_without_private_fields() {
        let mut report = report(0x0000_0500, 0x0000_0630, 0x0402);
        report[1..11].copy_from_slice(b"2026-02-04");
        report[12..20].copy_from_slice(b"12:34:56");
        let decoded = decode_firmware_info_report(&report, 0x0ce6).unwrap();
        assert_eq!(decoded.update_version, "A-0402");
        assert_eq!(decoded.main_firmware_version, "0x00000630");
        assert_eq!(decoded.hardware_model.as_deref(), Some("BDM-030"));
        assert_eq!(decoded.build_time.as_deref(), Some("2026-02-04 12:34:56"));
    }

    #[test]
    fn maps_edge_hardware_separately() {
        let decoded = decode_firmware_info_report(
            &report(0x0000_0200, 0x0000_0630, 0x0402),
            DUALSENSE_EDGE_PRODUCT_ID,
        )
        .unwrap();
        assert_eq!(decoded.hardware_model.as_deref(), Some("HDM-010"));
    }

    #[test]
    fn decodes_dualsenseclient_hardware_golden_vector() {
        // Independent Feature Report 0x20 vector from DualSenseClient commit
        // fff354fb7d37cb6e4f5ecb7ba4e72ba5b72094a0 (GPL-3.0).
        let decoded =
            decode_firmware_info_report(&report(0x0100_0208, 0x0000_0630, 0x0402), 0x0ce6)
                .expect("valid generation-2 DualSense report");
        assert_eq!(decoded.hardware_version, "0x01000208");
        assert_eq!(decoded.hardware_model.as_deref(), Some("HMB-010"));
        assert_eq!(decoded.main_firmware_version, "0x00000630");
        assert_eq!(decoded.update_version, "A-0402");
    }

    #[test]
    fn decodes_month_name_build_date_used_by_some_controllers() {
        let mut report = report(0x0000_0400, 0x0000_0630, 0x0402);
        report[1..12].copy_from_slice(b"Apr 8 2022\0");
        report[12..20].copy_from_slice(b"01:02:03");
        let decoded = decode_firmware_info_report(&report, 0x0ce6).unwrap();
        assert_eq!(decoded.build_time.as_deref(), Some("2022-04-08 01:02:03"));
    }

    #[test]
    fn rejects_m61_compatibility_placeholder() {
        assert_eq!(
            decode_firmware_info_report(&report(0x0000_0200, 0x0000_0256, 0x0221), 0x0ce6),
            None
        );
    }

    #[test]
    fn rejects_truncated_and_empty_reports() {
        assert_eq!(decode_firmware_info_report(&[0x20; 12], 0x0ce6), None);
        assert_eq!(decode_firmware_info_report(&report(0, 0, 0), 0x0ce6), None);
    }
}
