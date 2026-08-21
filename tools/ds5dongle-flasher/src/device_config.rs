use anyhow::{Result, bail};

const SONY_VENDOR_ID: u16 = 0x054c;
const DUALSENSE_PRODUCT_IDS: [u16; 2] = [0x0ce6, 0x0df2];
const CONFIG_SET_REPORT_ID: u8 = 0xf6;
const CONFIG_GET_REPORT_ID: u8 = 0xf7;
const FIRMWARE_ID_REPORT_ID: u8 = 0xf8;
const REMAP_REPORT_ID: u8 = 0xfb;
const REMAP_WIRE_VERSION: u8 = 3;
const REMAP_LEGACY_WIRE_VERSION: u8 = 2;
const FEATURE_REPORT_SIZE: usize = 64;
const CONFIG_POLLING_MODE_OFFSET: usize = 10;
const MAX_ATTEMPTS: usize = 4;
pub const REMAP_CONTROL_COUNT: usize = 19;
const REMAP_MASK_BYTES: usize = 3;
const REMAP_VALID_MASK: u32 = (1_u32 << REMAP_CONTROL_COUNT) - 1;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ButtonMapping {
    pub target_masks: [u32; REMAP_CONTROL_COUNT],
    pub revision: u16,
    pub imported_legacy_v2: bool,
}

impl Default for ButtonMapping {
    fn default() -> Self {
        let mut target_masks = [0_u32; REMAP_CONTROL_COUNT];
        for (index, target_mask) in target_masks.iter_mut().enumerate() {
            *target_mask = 1_u32 << index;
        }
        Self {
            target_masks,
            revision: u16::MAX,
            imported_legacy_v2: false,
        }
    }
}

impl ButtonMapping {
    pub fn is_identity(&self) -> bool {
        self.target_masks
            .iter()
            .enumerate()
            .all(|(index, target_mask)| *target_mask == 1_u32 << index)
    }

    pub fn is_target_enabled(&self, source: usize, target: usize) -> bool {
        self.target_masks
            .get(source)
            .is_some_and(|mask| (*mask & (1_u32 << target)) != 0)
    }

    pub fn set_target_enabled(&mut self, source: usize, target: usize, enabled: bool) {
        if source >= REMAP_CONTROL_COUNT || target >= REMAP_CONTROL_COUNT {
            return;
        }
        if enabled {
            self.target_masks[source] |= 1_u32 << target;
        } else {
            self.target_masks[source] &= !(1_u32 << target);
        }
    }

    pub fn target_count(&self, source: usize) -> u32 {
        self.target_masks
            .get(source)
            .map_or(0, |mask| mask.count_ones())
    }

    fn validate(&self) -> Result<()> {
        for (source, target_mask) in self.target_masks.iter().enumerate() {
            if (*target_mask & !REMAP_VALID_MASK) != 0 {
                bail!("mapping source {source} has invalid target bits 0x{target_mask:08x}");
            }
        }
        Ok(())
    }
}

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
    feature_command_report(CONFIG_SET_REPORT_ID, command, payload)
}

fn feature_command_report(
    report_id: u8,
    command: u8,
    payload: &[u8],
) -> Result<[u8; FEATURE_REPORT_SIZE]> {
    if payload.len() > FEATURE_REPORT_SIZE - 2 {
        bail!(
            "configuration payload is too large: {} bytes",
            payload.len()
        );
    }
    let mut report = [0_u8; FEATURE_REPORT_SIZE];
    report[0] = report_id;
    report[1] = command;
    report[2..2 + payload.len()].copy_from_slice(payload);
    Ok(report)
}

fn decode_button_mapping(source: &[u8]) -> Result<ButtonMapping> {
    let payload = payload_without_report_id(source, REMAP_REPORT_ID);
    if payload.len() < 2 + REMAP_CONTROL_COUNT {
        bail!(
            "19-control mapping report is too short: {} bytes",
            payload.len()
        );
    }
    if usize::from(payload[1]) != REMAP_CONTROL_COUNT {
        bail!(
            "device mapping contains {} controls, expected {REMAP_CONTROL_COUNT}",
            payload[1]
        );
    }
    if payload[0] == REMAP_LEGACY_WIRE_VERSION {
        let mut mapping = ButtonMapping::default();
        for (source, target) in payload[2..2 + REMAP_CONTROL_COUNT].iter().enumerate() {
            if usize::from(*target) >= REMAP_CONTROL_COUNT {
                bail!("legacy mapping source {source} has invalid target {target}");
            }
            mapping.target_masks[source] = 1_u32 << target;
        }
        mapping.imported_legacy_v2 = true;
        return Ok(mapping);
    }
    if payload[0] != REMAP_WIRE_VERSION || payload.len() < 63 {
        bail!("device does not expose mapping protocol v{REMAP_WIRE_VERSION}");
    }
    let mut mapping = ButtonMapping::default();
    for source in 0..REMAP_CONTROL_COUNT {
        let offset = 2 + source * REMAP_MASK_BYTES;
        mapping.target_masks[source] = u32::from(payload[offset])
            | (u32::from(payload[offset + 1]) << 8)
            | (u32::from(payload[offset + 2]) << 16);
    }
    mapping.revision = u16::from_le_bytes([payload[59], payload[60]]);
    mapping.imported_legacy_v2 = false;
    mapping.validate()?;
    Ok(mapping)
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
fn send_remap_command(device: &hidapi::HidDevice, command: u8, payload: &[u8]) -> Result<()> {
    let report = feature_command_report(REMAP_REPORT_ID, command, payload)?;
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
    bail!("unable to send 19-control mapping command: {last_error}")
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

#[cfg(windows)]
pub fn read_button_mapping() -> Result<ButtonMapping> {
    let device = open_config_device()?;
    decode_button_mapping(&read_feature(&device, REMAP_REPORT_ID)?)
}

#[cfg(not(windows))]
pub fn read_button_mapping() -> Result<ButtonMapping> {
    bail!("19-control mapping is available on Windows only")
}

#[cfg(windows)]
pub fn apply_button_mapping(mapping: &ButtonMapping) -> Result<ButtonMapping> {
    mapping.validate()?;
    let device = open_config_device()?;
    let mut payload = Vec::with_capacity(FEATURE_REPORT_SIZE - 2);
    payload.push(REMAP_WIRE_VERSION);
    payload.push(REMAP_CONTROL_COUNT as u8);
    for target_mask in mapping.target_masks {
        let bytes = target_mask.to_le_bytes();
        payload.extend_from_slice(&bytes[..REMAP_MASK_BYTES]);
    }
    payload.extend_from_slice(&mapping.revision.to_le_bytes());
    payload.push(0);
    send_remap_command(&device, 0x01, &payload)?;
    std::thread::sleep(std::time::Duration::from_millis(150));
    let verified = decode_button_mapping(&read_feature(&device, REMAP_REPORT_ID)?)?;
    if verified.target_masks != mapping.target_masks {
        bail!("device mapping verification did not match the requested 19-control table");
    }
    Ok(verified)
}

#[cfg(not(windows))]
pub fn apply_button_mapping(_mapping: &ButtonMapping) -> Result<ButtonMapping> {
    bail!("19-control mapping is available on Windows only")
}

#[cfg(windows)]
pub fn reset_button_mapping() -> Result<ButtonMapping> {
    let device = open_config_device()?;
    send_remap_command(&device, 0x02, &[])?;
    std::thread::sleep(std::time::Duration::from_millis(150));
    let verified = decode_button_mapping(&read_feature(&device, REMAP_REPORT_ID)?)?;
    if !verified.is_identity() {
        bail!("device did not restore the identity 19-control mapping");
    }
    Ok(verified)
}

#[cfg(not(windows))]
pub fn reset_button_mapping() -> Result<ButtonMapping> {
    bail!("19-control mapping is available on Windows only")
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
    fn decodes_versioned_19_control_mapping_and_legacy_v2() {
        let identity = ButtonMapping::default();
        let mut wire = vec![
            REMAP_REPORT_ID,
            REMAP_WIRE_VERSION,
            REMAP_CONTROL_COUNT as u8,
        ];
        for mask in identity.target_masks {
            wire.extend_from_slice(&mask.to_le_bytes()[..REMAP_MASK_BYTES]);
        }
        wire.extend_from_slice(&7_u16.to_le_bytes());
        wire.extend_from_slice(&[1, 3]);
        let decoded = decode_button_mapping(&wire).unwrap();
        assert_eq!(decoded.target_masks, identity.target_masks);
        assert_eq!(decoded.revision, 7);

        let mut legacy = vec![
            REMAP_REPORT_ID,
            REMAP_LEGACY_WIRE_VERSION,
            REMAP_CONTROL_COUNT as u8,
        ];
        legacy.extend(0..REMAP_CONTROL_COUNT as u8);
        let decoded = decode_button_mapping(&legacy).unwrap();
        assert!(decoded.imported_legacy_v2);
        assert!(decoded.is_identity());
    }

    #[test]
    fn mapping_command_fits_one_feature_report() {
        let identity = ButtonMapping::default();
        let mut payload = vec![REMAP_WIRE_VERSION, REMAP_CONTROL_COUNT as u8];
        for mask in identity.target_masks {
            payload.extend_from_slice(&mask.to_le_bytes()[..REMAP_MASK_BYTES]);
        }
        payload.extend_from_slice(&identity.revision.to_le_bytes());
        payload.push(0);
        let report = feature_command_report(REMAP_REPORT_ID, 0x01, &payload).unwrap();
        assert_eq!(report[0], REMAP_REPORT_ID);
        assert_eq!(report[1], 0x01);
        assert_eq!(report[2], REMAP_WIRE_VERSION);
        assert_eq!(report[3], REMAP_CONTROL_COUNT as u8);
        assert_eq!(report.len(), FEATURE_REPORT_SIZE);
        assert_eq!(report[4], 1);
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
