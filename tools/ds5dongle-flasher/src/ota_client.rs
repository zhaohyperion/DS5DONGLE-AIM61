use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{BuildProfile, DUALSENSE_PRODUCT_IDS, SONY_VENDOR_ID};

const DATA_REPORT_ID: u8 = 0xfa;
const CONTROL_REPORT_ID: u8 = 0xfc;
const PROTOCOL_VERSION: u8 = 2;
const REPORT_PAYLOAD_SIZE: usize = 63;
const WIRE_REPORT_SIZE: usize = REPORT_PAYLOAD_SIZE + 1;
const CRC_OFFSET: usize = 59;
const DATA_BYTES_MAX: usize = 46;
const OTA_HEADER_SIZE: usize = 512;
const OTA_MAGIC: &[u8; 16] = b"BL60X_OTA_Ver1.0";
const OTA_RAW_TYPE: &[u8; 4] = b"RAW ";

const CTRL_BEGIN: u8 = 0x01;
const CTRL_AUTH: u8 = 0x02;
const CTRL_COMMIT: u8 = 0x03;
const CTRL_ABORT: u8 = 0x04;
#[cfg(test)]
const CTRL_STATUS: u8 = 0x05;
const CTRL_ACK: u8 = 0x80;
const CTRL_ERROR: u8 = 0xff;
const MSG_DATA: u8 = 0x10;

const STATE_IDLE: u8 = 0;
const STATE_AUTHORIZING: u8 = 1;
const STATE_RECEIVING: u8 = 3;
const STATE_READY_REBOOT: u8 = 5;
const STATE_ERROR: u8 = 6;

const CAP_SIGNATURE_REQUIRED: u32 = 1 << 8;
const CAP_KEY_CONFIGURED: u32 = 1 << 9;

#[derive(Clone, Debug, Deserialize)]
struct OtaSignature {
    algorithm: String,
    key_id: String,
    scope: String,
    value: String,
}

#[derive(Clone, Debug, Deserialize)]
struct OtaManifest {
    schema: u32,
    channel: String,
    board: String,
    usb_speed: String,
    profile: BuildProfile,
    version: String,
    size: usize,
    sha256: String,
    body_size: usize,
    body_sha256: String,
    url: String,
    signature: Option<OtaSignature>,
}

#[derive(Debug)]
struct OtaPackage {
    image: Vec<u8>,
    body_sha256: [u8; 32],
    version: [u8; 3],
    profile: BuildProfile,
    signature: [u8; 64],
}

#[derive(Clone, Copy, Debug)]
struct Status {
    opcode: u8,
    session: u32,
    accepted_offset: u32,
    state: u8,
    error: u8,
    committed_offset: u32,
    total_size: u32,
    max_file_size: u32,
    capabilities: u32,
    board: u8,
    speed: u8,
    max_data: u8,
    window: u8,
}

pub fn update_from_zip(
    archive_path: &Path,
    expected_profile: BuildProfile,
    mut progress: impl FnMut(String),
) -> Result<()> {
    let package = load_ota_package(archive_path, expected_profile)?;
    progress(format!(
        "OTA package verified: {} bytes, target {}",
        package.image.len(),
        expected_profile.label()
    ));

    #[cfg(windows)]
    {
        update_device(&package, &mut progress)
    }
    #[cfg(not(windows))]
    {
        let _ = progress;
        bail!("DS5Dongle OTA is available on Windows only")
    }
}

fn load_ota_package(path: &Path, expected_profile: BuildProfile) -> Result<OtaPackage> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("invalid firmware ZIP")?;
    if archive.len() > 64 {
        bail!("firmware ZIP contains too many entries")
    }

    let mut image = None;
    let mut manifest = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("unsafe ZIP path: {}", entry.name()))?;
        let name = enclosed
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if name.ends_with(".bin.ota") {
            if image.is_some() {
                bail!("firmware ZIP contains multiple OTA images")
            }
            if entry.size() > 2 * 1024 * 1024 {
                bail!("OTA image is too large")
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            image = Some(bytes);
        } else if name.ends_with(".ota.json") {
            if manifest.is_some() {
                bail!("firmware ZIP contains multiple OTA manifests")
            }
            if entry.size() > 64 * 1024 {
                bail!("OTA manifest is too large")
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            manifest = Some(bytes);
        }
    }

    let image = image.context("firmware ZIP has no signed .bin.ota image")?;
    let manifest: OtaManifest =
        serde_json::from_slice(&manifest.context("firmware ZIP has no .ota.json manifest")?)
            .context("invalid OTA manifest JSON")?;
    validate_package(image, manifest, expected_profile)
}

fn validate_package(
    image: Vec<u8>,
    manifest: OtaManifest,
    expected_profile: BuildProfile,
) -> Result<OtaPackage> {
    if manifest.schema != 1
        || manifest.board != "aim61"
        || manifest.usb_speed != "hs"
        || manifest.profile != expected_profile
    {
        bail!(
            "OTA manifest target/profile does not match AIM61 High-Speed {}",
            expected_profile.label()
        )
    }
    if !matches!(manifest.channel.as_str(), "beta" | "stable" | "dev") {
        bail!("unsupported OTA release channel")
    }
    if !manifest.url.starts_with("https://") {
        bail!("OTA manifest URL must use HTTPS")
    }
    if image.len() < OTA_HEADER_SIZE
        || image.get(..16) != Some(OTA_MAGIC)
        || image.get(16..20) != Some(OTA_RAW_TYPE)
    {
        bail!("OTA image is not a Bouffalo RAW OTA container")
    }
    if manifest.size != image.len() || manifest.sha256 != hex_digest(&image) {
        bail!("OTA manifest full-image SHA-256/size mismatch")
    }
    let declared_body_size = u32::from_le_bytes(image[20..24].try_into().unwrap()) as usize;
    let body = &image[OTA_HEADER_SIZE..];
    if manifest.body_size != body.len() || declared_body_size != body.len() {
        bail!("OTA body size disagrees with the header or manifest")
    }
    let body_hash: [u8; 32] = Sha256::digest(body).into();
    if image[64..96] != body_hash || manifest.body_sha256 != hex::encode(body_hash) {
        bail!("OTA body SHA-256 mismatch")
    }

    let version = parse_version(&manifest.version)?;
    let expected_header_version = format!("EVENT_V{}", manifest.version);
    let header_version = image[48..64]
        .split(|byte| *byte == 0)
        .next()
        .unwrap_or_default();
    if header_version != expected_header_version.as_bytes() {
        bail!("OTA header software version does not match the manifest")
    }

    let signature = manifest.signature.context("OTA manifest is unsigned")?;
    if signature.algorithm != "ECDSA-P256-SHA256"
        || signature.scope != "DS5DONGLE-OTA-V2"
        || signature.key_id.is_empty()
        || signature.key_id.len() > 64
    {
        bail!("OTA signature metadata is invalid")
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(signature.value)
        .context("OTA signature is not valid base64")?;
    let signature: [u8; 64] = raw
        .try_into()
        .map_err(|_| anyhow!("OTA P-256 signature must be 64 bytes"))?;

    Ok(OtaPackage {
        image,
        body_sha256: body_hash,
        version,
        profile: expected_profile,
        signature,
    })
}

fn parse_version(value: &str) -> Result<[u8; 3]> {
    let parts = value
        .split('.')
        .map(str::parse::<u8>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("OTA version is not MAJOR.MINOR.PATCH")?;
    if parts.len() != 3 || parts.iter().any(|value| *value == u8::MAX) {
        bail!("OTA version is not a supported MAJOR.MINOR.PATCH")
    }
    Ok([parts[0], parts[1], parts[2]])
}

fn hex_digest(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn frame(opcode: u8, session: u32, argument: u32, data: &[u8]) -> Result<[u8; WIRE_REPORT_SIZE]> {
    if data.len() > DATA_BYTES_MAX {
        bail!("OTA frame payload exceeds {DATA_BYTES_MAX} bytes")
    }
    let mut wire = [0_u8; WIRE_REPORT_SIZE];
    wire[0] = if opcode == MSG_DATA {
        DATA_REPORT_ID
    } else {
        CONTROL_REPORT_ID
    };
    let payload = &mut wire[1..];
    payload[0..2].copy_from_slice(b"OT");
    payload[2] = PROTOCOL_VERSION;
    payload[3] = opcode;
    payload[4..8].copy_from_slice(&session.to_le_bytes());
    payload[8..12].copy_from_slice(&argument.to_le_bytes());
    payload[12] = data.len() as u8;
    payload[13..13 + data.len()].copy_from_slice(data);
    let crc = crc32fast::hash(&payload[..CRC_OFFSET]);
    payload[CRC_OFFSET..CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());
    Ok(wire)
}

fn decode_status(wire: &[u8]) -> Result<Status> {
    if wire.len() != WIRE_REPORT_SIZE || wire[0] != CONTROL_REPORT_ID {
        bail!("invalid OTA status report length or report ID")
    }
    let payload = &wire[1..];
    if &payload[0..2] != b"OT" || payload[2] != PROTOCOL_VERSION {
        bail!("device does not implement OTA protocol v2")
    }
    let expected = u32::from_le_bytes(payload[CRC_OFFSET..CRC_OFFSET + 4].try_into().unwrap());
    if crc32fast::hash(&payload[..CRC_OFFSET]) != expected || payload[12] != 44 {
        bail!("invalid OTA status CRC or payload length")
    }
    let data = &payload[13..57];
    Ok(Status {
        opcode: payload[3],
        session: u32::from_le_bytes(payload[4..8].try_into().unwrap()),
        accepted_offset: u32::from_le_bytes(payload[8..12].try_into().unwrap()),
        state: data[0],
        error: data[1],
        committed_offset: u32::from_le_bytes(data[4..8].try_into().unwrap()),
        total_size: u32::from_le_bytes(data[8..12].try_into().unwrap()),
        max_file_size: u32::from_le_bytes(data[12..16].try_into().unwrap()),
        capabilities: u32::from_le_bytes(data[16..20].try_into().unwrap()),
        board: data[20],
        speed: data[21],
        max_data: data[23],
        window: data[24],
    })
}

#[cfg(windows)]
fn update_device(package: &OtaPackage, progress: &mut impl FnMut(String)) -> Result<()> {
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
    let device = info.open_device(&api).context(
        "unable to open the gamepad HID interface; close Steam, DS4Windows and the test session",
    )?;

    let caps = read_status(&device)?;
    if caps.state != STATE_IDLE || caps.board != 1 || caps.speed != 1 {
        bail!(
            "device is busy or is not AIM61 High-Speed (state={})",
            caps.state
        )
    }
    if caps.max_data < DATA_BYTES_MAX as u8
        || caps.max_file_size < package.image.len() as u32
        || caps.capabilities & CAP_SIGNATURE_REQUIRED == 0
        || caps.capabilities & CAP_KEY_CONFIGURED == 0
    {
        bail!("device OTA capabilities/key configuration do not accept this signed image")
    }

    let session = new_session_id();
    let result = (|| -> Result<()> {
        let mut begin = [0_u8; 43];
        begin[0] = 1;
        begin[1] = 1;
        begin[2..5].copy_from_slice(&package.version);
        begin[5] = 1;
        begin[6..10]
            .copy_from_slice(&(package.image.len() as u32 - OTA_HEADER_SIZE as u32).to_le_bytes());
        begin[10..42].copy_from_slice(&package.body_sha256);
        begin[42] = match package.profile {
            BuildProfile::Standard => 0,
            BuildProfile::Diagnostic => 1,
        };
        send_feature(
            &device,
            &frame(CTRL_BEGIN, session, package.image.len() as u32, &begin)?,
        )?;
        wait_status(&device, session, Duration::from_secs(3), |status| {
            status.state == STATE_AUTHORIZING
        })?;
        progress("Device accepted BEGIN; authorizing signed image".to_owned());

        send_feature(
            &device,
            &frame(CTRL_AUTH, session, 0, &package.signature[..46])?,
        )?;
        send_feature(
            &device,
            &frame(CTRL_AUTH, session, 46, &package.signature[46..])?,
        )?;
        wait_status(&device, session, Duration::from_secs(15), |status| {
            status.state == STATE_RECEIVING
        })?;
        progress("Signature accepted; transferring OTA image".to_owned());

        let window = usize::from(caps.window.clamp(1, 8));
        let mut offset = 0_usize;
        let mut last_percent = 0_usize;
        while offset < package.image.len() {
            let batch_start = offset;
            for _ in 0..window {
                if offset >= package.image.len() {
                    break;
                }
                let end = (offset + DATA_BYTES_MAX).min(package.image.len());
                send_output(
                    &device,
                    &frame(
                        MSG_DATA,
                        session,
                        offset as u32,
                        &package.image[offset..end],
                    )?,
                )?;
                offset = end;
            }
            let expected = offset as u32;
            wait_status(&device, session, Duration::from_secs(5), |status| {
                status.accepted_offset >= expected
            })?;
            let percent = offset * 100 / package.image.len();
            if percent >= last_percent + 5 || offset == package.image.len() {
                progress(format!(
                    "OTA transfer {percent}% ({offset}/{})",
                    package.image.len()
                ));
                last_percent = percent;
            }
            if offset == batch_start {
                bail!("OTA transfer made no progress")
            }
        }

        send_feature(&device, &frame(CTRL_COMMIT, session, 0, &[])?)?;
        progress("Transfer complete; device is verifying and switching the A/B slot".to_owned());
        match wait_status(&device, session, Duration::from_secs(15), |status| {
            status.state == STATE_READY_REBOOT
        }) {
            Ok(status) => {
                if status.accepted_offset != package.image.len() as u32
                    || status.committed_offset != package.image.len() as u32
                    || status.total_size != package.image.len() as u32
                {
                    bail!("device reported inconsistent final OTA offsets")
                }
            }
            Err(error) => {
                // A planned reboot can remove the HID interface before the final poll.
                if !format!("{error:#}").contains("HID") && !format!("{error:#}").contains("device")
                {
                    return Err(error);
                }
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = send_feature(&device, &frame(CTRL_ABORT, session, 0, &[])?);
    }
    result
}

#[cfg(windows)]
fn send_feature(device: &hidapi::HidDevice, wire: &[u8; WIRE_REPORT_SIZE]) -> Result<()> {
    device
        .send_feature_report(wire)
        .context("failed to write OTA Feature Report")?;
    Ok(())
}

#[cfg(windows)]
fn send_output(device: &hidapi::HidDevice, wire: &[u8; WIRE_REPORT_SIZE]) -> Result<()> {
    let written = device
        .write(wire)
        .context("failed to write OTA DATA report")?;
    if written != wire.len() {
        bail!("short OTA DATA report write: {written}/{}", wire.len())
    }
    Ok(())
}

#[cfg(windows)]
fn read_status(device: &hidapi::HidDevice) -> Result<Status> {
    let mut wire = [0_u8; WIRE_REPORT_SIZE];
    wire[0] = CONTROL_REPORT_ID;
    let length = device
        .get_feature_report(&mut wire)
        .context("failed to read OTA status HID Feature Report")?;
    decode_status(&wire[..length])
}

#[cfg(windows)]
fn wait_status(
    device: &hidapi::HidDevice,
    session: u32,
    timeout: Duration,
    predicate: impl Fn(Status) -> bool,
) -> Result<Status> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = read_status(device)?;
        if status.opcode == CTRL_ERROR || status.state == STATE_ERROR || status.error != 0 {
            bail!(
                "device rejected OTA command: error={}, state={}, accepted={}, committed={}",
                status.error,
                status.state,
                status.accepted_offset,
                status.committed_offset
            )
        }
        if status.opcode == CTRL_ACK
            && (status.session == session || status.session == 0)
            && predicate(status)
        {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            bail!(
                "timed out waiting for OTA state (state={}, accepted={}, committed={})",
                status.state,
                status.accepted_offset,
                status.committed_offset
            )
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn new_session_id() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mixed = (nanos as u64) ^ ((nanos >> 32) as u64) ^ u64::from(std::process::id());
    (mixed as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_fixture(profile: BuildProfile) -> (Vec<u8>, OtaManifest) {
        let body = b"signed-test-body";
        let body_hash: [u8; 32] = Sha256::digest(body).into();
        let mut image = vec![0_u8; OTA_HEADER_SIZE];
        image[0..16].copy_from_slice(OTA_MAGIC);
        image[16..20].copy_from_slice(OTA_RAW_TYPE);
        image[20..24].copy_from_slice(&(body.len() as u32).to_le_bytes());
        image[48..60].copy_from_slice(b"EVENT_V3.5.2");
        image[64..96].copy_from_slice(&body_hash);
        image.extend_from_slice(body);
        let manifest = OtaManifest {
            schema: 1,
            channel: "beta".to_owned(),
            board: "aim61".to_owned(),
            usb_speed: "hs".to_owned(),
            profile,
            version: "3.5.2".to_owned(),
            size: image.len(),
            sha256: hex_digest(&image),
            body_size: body.len(),
            body_sha256: hex::encode(body_hash),
            url: "https://example.invalid/firmware.zip".to_owned(),
            signature: Some(OtaSignature {
                algorithm: "ECDSA-P256-SHA256".to_owned(),
                key_id: "test-key".to_owned(),
                scope: "DS5DONGLE-OTA-V2".to_owned(),
                value: base64::engine::general_purpose::STANDARD.encode([0x5a; 64]),
            }),
        };
        (image, manifest)
    }

    #[test]
    fn protocol_frame_has_valid_crc() {
        let wire = frame(CTRL_STATUS, 0x1234_5678, 9, b"abc").unwrap();
        assert_eq!(wire[0], CONTROL_REPORT_ID);
        assert_eq!(&wire[1..3], b"OT");
        assert_eq!(wire[4], CTRL_STATUS);
        assert_eq!(wire[13], 3);
        assert_eq!(&wire[14..17], b"abc");
        assert_eq!(
            u32::from_le_bytes(wire[60..64].try_into().unwrap()),
            crc32fast::hash(&wire[1..60])
        );
    }

    #[test]
    fn decodes_locked_v2_status_vector() {
        let payload = hex::decode(
            "4f54028078563412401000002c0300020100100000f02b0d0000821600fb0300000100012e1000f20f424c3631382d44533520332e3500000000005b21bdd1",
        )
        .unwrap();
        let mut wire = vec![CONTROL_REPORT_ID];
        wire.extend_from_slice(&payload);
        let status = decode_status(&wire).unwrap();
        assert_eq!(status.opcode, CTRL_ACK);
        assert_eq!(status.session, 0x1234_5678);
        assert_eq!(status.accepted_offset, 0x1040);
        assert_eq!(status.state, STATE_RECEIVING);
        assert_eq!(status.committed_offset, 0x1000);
        assert_eq!(status.board, 1);
        assert_eq!(status.speed, 0);
        assert_eq!(status.max_data, 46);
        assert_eq!(status.window, 16);
    }

    #[test]
    fn parses_normalized_version() {
        assert_eq!(parse_version("3.5.2").unwrap(), [3, 5, 2]);
        assert!(parse_version("v3.5.2").is_err());
        assert!(parse_version("3.5").is_err());
        assert!(parse_version("3.5.255").is_err());
    }

    #[test]
    fn validates_profile_bound_raw_package() {
        let (image, manifest) = package_fixture(BuildProfile::Diagnostic);
        let package = validate_package(image.clone(), manifest, BuildProfile::Diagnostic).unwrap();
        assert_eq!(package.image, image);
        assert_eq!(package.profile, BuildProfile::Diagnostic);

        let (image, manifest) = package_fixture(BuildProfile::Diagnostic);
        assert!(validate_package(image, manifest, BuildProfile::Standard).is_err());
    }
}
