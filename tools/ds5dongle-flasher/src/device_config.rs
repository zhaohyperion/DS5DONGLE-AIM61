use anyhow::{Result, bail};

const SONY_VENDOR_ID: u16 = 0x054c;
const DUALSENSE_PRODUCT_IDS: [u16; 2] = [0x0ce6, 0x0df2];
const CONFIG_SET_REPORT_ID: u8 = 0xf6;
const CONFIG_GET_REPORT_ID: u8 = 0xf7;
const FIRMWARE_ID_REPORT_ID: u8 = 0xf8;
const FEATURE_REPORT_SIZE: usize = 64;
const CONFIG_POLLING_MODE_OFFSET: usize = 10;
const MAX_ATTEMPTS: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PollingRate {
    Hz250,
    Hz500,
    #[default]
    Realtime,
}

impl PollingRate {
    pub const ALL: [Self; 3] = [Self::Hz250, Self::Hz500, Self::Realtime];

    pub fn wire_value(self) -> u8 {
        match self {
            Self::Hz250 => 0,
            Self::Hz500 => 1,
            Self::Realtime => 2,
        }
    }

    pub fn from_wire(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Hz250),
            1 => Ok(Self::Hz500),
            2 => Ok(Self::Realtime),
            _ => bail!("device returned unsupported polling mode {value}"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hz250 => "250 Hz",
            Self::Hz500 => "500 Hz",
            Self::Realtime => "Realtime (~750 Hz)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyResult {
    pub mode: PollingRate,
    pub reset_requested: bool,
}

fn payload_without_report_id(source: &[u8], report_id: u8) -> &[u8] {
    source.strip_prefix(&[report_id]).unwrap_or(source)
}

fn decode_identity(source: &[u8]) -> Option<&str> {
    let payload = payload_without_report_id(source, FIRMWARE_ID_REPORT_ID);
    let end = payload
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(payload.len());
    let text = std::str::from_utf8(&payload[..end]).ok()?.trim();
    let (_, profile) = text.split_once('|')?;
    matches!(profile.trim(), "standard" | "diagnostic").then_some(text)
}

fn decode_config(source: &[u8]) -> Result<Vec<u8>> {
    let payload = payload_without_report_id(source, CONFIG_GET_REPORT_ID);
    if payload.len() <= CONFIG_POLLING_MODE_OFFSET {
        bail!(
            "configuration report is too short: {} bytes, need at least {}",
            payload.len(),
            CONFIG_POLLING_MODE_OFFSET + 1
        );
    }
    PollingRate::from_wire(payload[CONFIG_POLLING_MODE_OFFSET])?;
    Ok(payload.to_vec())
}

fn set_polling_in_config(config: &mut [u8], mode: PollingRate) -> Result<()> {
    if config.len() <= CONFIG_POLLING_MODE_OFFSET {
        bail!("configuration payload is too short");
    }
    config[CONFIG_POLLING_MODE_OFFSET] = mode.wire_value();
    Ok(())
}

fn command_report(command: u8, payload: &[u8]) -> Result<[u8; FEATURE_REPORT_SIZE]> {
    if payload.len() > FEATURE_REPORT_SIZE - 2 {
        bail!(
            "configuration payload is too large: {} bytes",
            payload.len()
        );
    }
    let mut report = [0_u8; FEATURE_REPORT_SIZE];
    report[0] = CONFIG_SET_REPORT_ID;
    report[1] = command;
    report[2..2 + payload.len()].copy_from_slice(payload);
    Ok(report)
}

#[cfg(windows)]
fn read_feature(device: &hidapi::HidDevice, report_id: u8) -> Result<Vec<u8>> {
    let mut last_error = String::new();
    for attempt in 1..=MAX_ATTEMPTS {
        let mut report = [0_u8; FEATURE_REPORT_SIZE];
        report[0] = report_id;
        match device.get_feature_report(&mut report) {
            Ok(length) if length > 0 => return Ok(report[..length].to_vec()),
            Ok(_) => last_error = format!("attempt {attempt}: empty report"),
            Err(error) => last_error = format!("attempt {attempt}: {error}"),
        }
        if attempt < MAX_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    bail!("unable to read feature report 0x{report_id:02x}: {last_error}")
}

#[cfg(windows)]
fn send_command(device: &hidapi::HidDevice, command: u8, payload: &[u8]) -> Result<()> {
    let report = command_report(command, payload)?;
    let mut last_error = String::new();
    for attempt in 1..=MAX_ATTEMPTS {
        match device.send_feature_report(&report) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = format!("attempt {attempt}: {error}"),
        }
        if attempt < MAX_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    bail!("unable to send configuration command 0x{command:02x}: {last_error}")
}

#[cfg(windows)]
fn open_config_device() -> Result<hidapi::HidDevice> {
    let api = hidapi::HidApi::new()?;
    let mut found = Vec::new();
    for info in api.device_list().filter(|info| {
        info.vendor_id() == SONY_VENDOR_ID && DUALSENSE_PRODUCT_IDS.contains(&info.product_id())
    }) {
        let Ok(device) = info.open_device(&api) else {
            continue;
        };
        let Ok(identity) = read_feature(&device, FIRMWARE_ID_REPORT_ID) else {
            continue;
        };
        if decode_identity(&identity).is_some() {
            found.push(device);
        }
    }
    match found.len() {
        0 => bail!("no running DS5DONGLE-AIM61 with the 0xF8 identity report was found"),
        1 => Ok(found.remove(0)),
        count => {
            bail!("found {count} configurable DS5DONGLE-AIM61 devices; connect only one and retry")
        }
    }
}

#[cfg(windows)]
pub fn read_polling_rate() -> Result<PollingRate> {
    let device = open_config_device()?;
    let config = decode_config(&read_feature(&device, CONFIG_GET_REPORT_ID)?)?;
    PollingRate::from_wire(config[CONFIG_POLLING_MODE_OFFSET])
}

#[cfg(not(windows))]
pub fn read_polling_rate() -> Result<PollingRate> {
    bail!("device polling configuration is available on Windows only")
}

#[cfg(windows)]
pub fn apply_polling_rate(mode: PollingRate) -> Result<ApplyResult> {
    let device = open_config_device()?;
    let mut config = decode_config(&read_feature(&device, CONFIG_GET_REPORT_ID)?)?;
    set_polling_in_config(&mut config, mode)?;

    // 0x01 applies the full preserved configuration; 0x02 persists it.
    send_command(&device, 0x01, &config)?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    send_command(&device, 0x02, &[])?;
    std::thread::sleep(std::time::Duration::from_millis(80));

    let verified = decode_config(&read_feature(&device, CONFIG_GET_REPORT_ID)?)?;
    let verified_mode = PollingRate::from_wire(verified[CONFIG_POLLING_MODE_OFFSET])?;
    if verified_mode != mode {
        bail!(
            "device verified polling mode as {}, expected {}",
            verified_mode.label(),
            mode.label()
        );
    }

    // 0x03 saves once more and performs a system reset so Windows reads the
    // new endpoint bInterval.  A disappearing HID handle can surface as an
    // error even when the command was accepted, hence it is advisory here.
    let reset_requested = send_command(&device, 0x03, &[]).is_ok();
    Ok(ApplyResult {
        mode,
        reset_requested,
    })
}

#[cfg(not(windows))]
pub fn apply_polling_rate(_mode: PollingRate) -> Result<ApplyResult> {
    bail!("device polling configuration is available on Windows only")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config(mode: PollingRate) -> Vec<u8> {
        let mut config = vec![0_u8; 27];
        config[0] = 2;
        config[CONFIG_POLLING_MODE_OFFSET] = mode.wire_value();
        config
    }

    #[test]
    fn decodes_reports_with_or_without_report_id() {
        let config = sample_config(PollingRate::Hz500);
        assert_eq!(
            PollingRate::from_wire(decode_config(&config).unwrap()[CONFIG_POLLING_MODE_OFFSET])
                .unwrap(),
            PollingRate::Hz500
        );

        let mut wire = vec![CONFIG_GET_REPORT_ID];
        wire.extend_from_slice(&config);
        assert_eq!(
            PollingRate::from_wire(decode_config(&wire).unwrap()[CONFIG_POLLING_MODE_OFFSET])
                .unwrap(),
            PollingRate::Hz500
        );
    }

    #[test]
    fn preserves_config_while_changing_only_polling_mode() {
        let original = sample_config(PollingRate::Hz250);
        let mut changed = original.clone();
        set_polling_in_config(&mut changed, PollingRate::Realtime).unwrap();
        for index in 0..changed.len() {
            if index == CONFIG_POLLING_MODE_OFFSET {
                assert_eq!(changed[index], 2);
            } else {
                assert_eq!(changed[index], original[index]);
            }
        }
    }

    #[test]
    fn builds_full_length_feature_commands() {
        let config = sample_config(PollingRate::Realtime);
        let report = command_report(0x01, &config).unwrap();
        assert_eq!(report.len(), FEATURE_REPORT_SIZE);
        assert_eq!(report[0], CONFIG_SET_REPORT_ID);
        assert_eq!(report[1], 0x01);
        assert_eq!(&report[2..2 + config.len()], config.as_slice());
    }

    #[test]
    fn accepts_only_aim61_profile_identity() {
        assert_eq!(decode_identity(b"3.5.2|standard"), Some("3.5.2|standard"));
        assert_eq!(
            decode_identity(b"3.5.2|diagnostic"),
            Some("3.5.2|diagnostic")
        );
        assert_eq!(decode_identity(b"LCT616-DS5 3.17H"), None);
    }
}
