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
const OTA_RELEASE_PUBLIC_KEY_SEC1_B64: &str =
    "BMquuIp6ocsO2MsjrjjB5a67pWJDHNL27+OcZx6hbUGDzcJ6higdFgtJv7iLV/QEYcuuMiizffKnbuoN+kxnCNc=";
const OTA_RELEASE_KEY_ID: &str = "local-m61-2026";

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

fn ota_error_name(error: u8) -> &'static str {
    match error {
        0 => "OK",
        1 => "BAD_MAGIC",
        2 => "BAD_VERSION",
        3 => "BAD_OPCODE",
        4 => "BAD_STATE",
        5 => "BAD_SESSION",
        6 => "BAD_OFFSET",
        7 => "BAD_LENGTH",
        8 => "BAD_CRC",
        9 => "BAD_TARGET",
        10 => "TOO_LARGE",
        11 => "QUEUE_FULL",
        12 => "FLASH",
        13 => "HASH",
        14 => "HEADER",
        15 => "INTERNAL",
        16 => "AUTH_REQUIRED",
        17 => "AUTH_FAILED",
        18 => "KEY_MISSING",
        19 => "TIMEOUT",
        _ => "UNKNOWN",
    }
}

fn ota_error_hint(error: u8) -> &'static str {
    match error {
        4 => "device OTA state changed unexpectedly; reconnect the device before retrying",
        9 => {
            "target/profile/version rejected; downgrades and reinstalling the same profile/version are blocked"
        }
        11 => "device receive queue overflowed; retry with the updated flasher transfer pacing",
        12 => "inactive-slot flash erase/write failed",
        13 => "flashed image SHA-256 verification failed",
        14 => "device rejected the Bouffalo OTA header",
        17 => "device rejected the release signature",
        19 => "OTA session was inactive for too long",
        _ => "see the OTA diagnostics documentation for this device error",
    }
}

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
    version_text: String,
    channel: String,
    key_id: String,
}

#[derive(Clone, Debug)]
pub struct OtaPackageInfo {
    pub profile: BuildProfile,
    pub version: String,
    pub channel: String,
    pub key_id: String,
    pub image_size: usize,
    pub archive_sha256: String,
}

pub fn inspect_zip(path: &Path) -> Result<OtaPackageInfo> {
    let profile = manifest_profile(path)?;
    let package = load_ota_package(path, profile)?;
    Ok(OtaPackageInfo {
        profile: package.profile,
        version: package.version_text,
        channel: package.channel,
        key_id: package.key_id,
        image_size: package.image.len(),
        archive_sha256: hex_digest(
            &std::fs::read(path).with_context(|| format!("cannot hash {}", path.display()))?,
        ),
    })
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

/// Small transport boundary around the OTA state machine.
///
/// The production implementation talks to the Windows HID device. Tests use
/// an in-memory scripted transport so every protocol failure can be exercised
/// without a controller or an AIM61 board connected.
trait OtaTransport {
    fn send_feature(&mut self, wire: &[u8; WIRE_REPORT_SIZE]) -> Result<()>;
    fn send_output(&mut self, wire: &[u8; WIRE_REPORT_SIZE]) -> Result<()>;
    fn read_status(&mut self) -> Result<Status>;
    fn monotonic(&self) -> Duration;
    fn sleep(&mut self, duration: Duration);
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
    let mut checksum_manifest = false;
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
        } else if name == "sha256sums.txt" {
            if checksum_manifest {
                bail!("OTA ZIP contains multiple SHA256SUMS.txt files")
            }
            checksum_manifest = true;
        } else {
            bail!("OTA ZIP contains a non-OTA file: {name}")
        }
    }

    if !checksum_manifest {
        bail!("OTA ZIP has no SHA256SUMS.txt")
    }
    let image = image.context("firmware ZIP has no signed .bin.ota image")?;
    let manifest: OtaManifest =
        serde_json::from_slice(&manifest.context("firmware ZIP has no .ota.json manifest")?)
            .context("invalid OTA manifest JSON")?;
    validate_package(image, manifest, expected_profile, true)
}

fn manifest_profile(path: &Path) -> Result<BuildProfile> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("invalid firmware ZIP")?;
    let mut profile = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .enclosed_name()
            .and_then(|path| path.file_name().map(|name| name.to_owned()))
            .ok_or_else(|| anyhow!("unsafe ZIP path: {}", entry.name()))?
            .to_string_lossy()
            .to_ascii_lowercase();
        if name.ends_with(".ota.json") {
            if profile.is_some() {
                bail!("firmware ZIP contains multiple OTA manifests");
            }
            if entry.size() > 64 * 1024 {
                bail!("OTA manifest is too large");
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            let manifest: OtaManifest =
                serde_json::from_slice(&bytes).context("invalid OTA manifest JSON")?;
            if manifest.schema != 1 || manifest.board != "aim61" || manifest.usb_speed != "hs" {
                bail!("local OTA package is not for AIM61 High-Speed");
            }
            profile = Some(manifest.profile);
        }
    }
    profile.context("firmware ZIP has no .ota.json manifest")
}

fn validate_package(
    image: Vec<u8>,
    manifest: OtaManifest,
    expected_profile: BuildProfile,
    verify_signature: bool,
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
    let key_id = signature.key_id.clone();
    let raw = base64::engine::general_purpose::STANDARD
        .decode(signature.value)
        .context("OTA signature is not valid base64")?;
    let signature: [u8; 64] = raw
        .try_into()
        .map_err(|_| anyhow!("OTA P-256 signature must be 64 bytes"))?;

    if verify_signature {
        if key_id != OTA_RELEASE_KEY_ID {
            bail!(
                "OTA signing key ID is not trusted by this tool: expected {}, got {}",
                OTA_RELEASE_KEY_ID,
                key_id
            );
        }
        let mut canonical = Vec::with_capacity(58);
        canonical.extend_from_slice(b"DS5DONGLE-OTA-V2");
        canonical.extend_from_slice(&[
            1,
            1,
            if expected_profile == BuildProfile::Diagnostic {
                1
            } else {
                0
            },
        ]);
        canonical.extend_from_slice(&version);
        canonical.extend_from_slice(&(body.len() as u32).to_le_bytes());
        canonical.extend_from_slice(&body_hash);
        if canonical.len() != 58 {
            bail!("internal OTA signature canonical length mismatch");
        }
        let public_key = base64::engine::general_purpose::STANDARD
            .decode(OTA_RELEASE_PUBLIC_KEY_SEC1_B64)
            .context("embedded OTA release public key is invalid")?;
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_FIXED,
            public_key,
        )
        .verify(&canonical, &signature)
        .map_err(|_| anyhow!("OTA P-256 signature verification failed on this PC"))?;
    }

    Ok(OtaPackage {
        image,
        body_sha256: body_hash,
        version,
        profile: expected_profile,
        signature,
        version_text: manifest.version,
        channel: manifest.channel,
        key_id,
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

    let mut transport = HidOtaTransport {
        device,
        started_at: Instant::now(),
    };
    run_update(&mut transport, package, new_session_id(), progress)
}

fn run_update(
    transport: &mut impl OtaTransport,
    package: &OtaPackage,
    session: u32,
    progress: &mut impl FnMut(String),
) -> Result<()> {
    let caps = transport.read_status()?;

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
        transport.send_feature(&frame(
            CTRL_BEGIN,
            session,
            package.image.len() as u32,
            &begin,
        )?)?;
        wait_status(transport, session, Duration::from_secs(3), |status| {
            status.state == STATE_AUTHORIZING
        })?;
        progress("Device accepted BEGIN; authorizing signed image".to_owned());

        transport.send_feature(&frame(CTRL_AUTH, session, 0, &package.signature[..46])?)?;
        transport.send_feature(&frame(CTRL_AUTH, session, 46, &package.signature[46..])?)?;
        wait_status(transport, session, Duration::from_secs(15), |status| {
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
                transport.send_output(&frame(
                    MSG_DATA,
                    session,
                    offset as u32,
                    &package.image[offset..end],
                )?)?;
                offset = end;
            }
            let expected = offset as u32;
            wait_status(transport, session, Duration::from_secs(5), |status| {
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

        transport.send_feature(&frame(CTRL_COMMIT, session, 0, &[])?)?;
        progress("Transfer complete; device is verifying and switching the A/B slot".to_owned());
        match wait_status(transport, session, Duration::from_secs(15), |status| {
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
                // Never turn a protocol rejection (whose message also contains
                // the word "device") into a false success.
                if !is_expected_post_commit_disconnect(&error) {
                    return Err(error);
                }
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = transport.send_feature(&frame(CTRL_ABORT, session, 0, &[])?);
    }
    result
}

fn is_expected_post_commit_disconnect(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}").to_ascii_lowercase();
    message.contains("failed to read ota status hid feature report")
        && (message.contains("disconnected")
            || message.contains("not connected")
            || message.contains("no such device")
            || message.contains("device not found")
            || message.contains("could not read from hid device")
            || message.contains("hidapi"))
}

#[cfg(windows)]
struct HidOtaTransport {
    device: hidapi::HidDevice,
    started_at: Instant,
}

#[cfg(windows)]
impl OtaTransport for HidOtaTransport {
    fn send_feature(&mut self, wire: &[u8; WIRE_REPORT_SIZE]) -> Result<()> {
        self.device
            .send_feature_report(wire)
            .context("failed to write OTA Feature Report")?;
        Ok(())
    }

    fn send_output(&mut self, wire: &[u8; WIRE_REPORT_SIZE]) -> Result<()> {
        let written = self
            .device
            .write(wire)
            .context("failed to write OTA DATA report")?;
        if written != wire.len() {
            bail!("short OTA DATA report write: {written}/{}", wire.len())
        }
        Ok(())
    }

    fn read_status(&mut self) -> Result<Status> {
        let mut wire = [0_u8; WIRE_REPORT_SIZE];
        wire[0] = CONTROL_REPORT_ID;
        let length = self
            .device
            .get_feature_report(&mut wire)
            .context("failed to read OTA status HID Feature Report")?;
        decode_status(&wire[..length])
    }

    fn monotonic(&self) -> Duration {
        self.started_at.elapsed()
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn wait_status(
    transport: &mut impl OtaTransport,
    session: u32,
    timeout: Duration,
    predicate: impl Fn(Status) -> bool,
) -> Result<Status> {
    let deadline = transport.monotonic().saturating_add(timeout);
    loop {
        let status = match transport.read_status() {
            Ok(status) => status,
            Err(_) if transport.monotonic() < deadline => {
                // Windows can transiently reject a Feature Report while the
                // firmware is erasing/writing flash. Keep polling inside the
                // caller's bounded timeout instead of aborting a valid OTA.
                transport.sleep(Duration::from_millis(20));
                continue;
            }
            Err(error) => {
                return Err(error).context("timed out while polling OTA device status");
            }
        };
        if status.opcode == CTRL_ERROR || status.state == STATE_ERROR || status.error != 0 {
            bail!(
                "device rejected OTA command: {} (error={}), state={}, accepted={}, committed={}; {}",
                ota_error_name(status.error),
                status.error,
                status.state,
                status.accepted_offset,
                status.committed_offset,
                ota_error_hint(status.error)
            )
        }
        if status.opcode == CTRL_ACK
            && (status.session == session || status.session == 0)
            && predicate(status)
        {
            return Ok(status);
        }
        if transport.monotonic() >= deadline {
            bail!(
                "timed out waiting for OTA state (state={}, accepted={}, committed={})",
                status.state,
                status.accepted_offset,
                status.committed_offset
            )
        }
        transport.sleep(Duration::from_millis(20));
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
    use std::collections::VecDeque;

    struct MockTransport {
        statuses: VecDeque<std::result::Result<Status, String>>,
        fallback_error: Option<String>,
        feature_frames: Vec<[u8; WIRE_REPORT_SIZE]>,
        output_frames: Vec<[u8; WIRE_REPORT_SIZE]>,
        elapsed: Duration,
        fail_output_at: Option<usize>,
    }

    impl MockTransport {
        fn scripted(statuses: impl IntoIterator<Item = Status>) -> Self {
            Self {
                statuses: statuses.into_iter().map(Ok).collect(),
                fallback_error: None,
                feature_frames: Vec::new(),
                output_frames: Vec::new(),
                elapsed: Duration::ZERO,
                fail_output_at: None,
            }
        }

        fn abort_count(&self) -> usize {
            self.feature_frames
                .iter()
                .filter(|wire| wire[4] == CTRL_ABORT)
                .count()
        }
    }

    impl OtaTransport for MockTransport {
        fn send_feature(&mut self, wire: &[u8; WIRE_REPORT_SIZE]) -> Result<()> {
            self.feature_frames.push(*wire);
            Ok(())
        }

        fn send_output(&mut self, wire: &[u8; WIRE_REPORT_SIZE]) -> Result<()> {
            if self.fail_output_at == Some(self.output_frames.len()) {
                bail!("injected OTA DATA write failure")
            }
            self.output_frames.push(*wire);
            Ok(())
        }

        fn read_status(&mut self) -> Result<Status> {
            match self.statuses.pop_front() {
                Some(Ok(status)) => Ok(status),
                Some(Err(message)) => bail!(message),
                None => bail!(
                    "{}",
                    self.fallback_error
                        .as_deref()
                        .unwrap_or("mock status script exhausted")
                ),
            }
        }

        fn monotonic(&self) -> Duration {
            self.elapsed
        }

        fn sleep(&mut self, duration: Duration) {
            self.elapsed = self.elapsed.saturating_add(duration);
        }
    }

    fn status(session: u32, state: u8, accepted: u32, committed: u32) -> Status {
        Status {
            opcode: CTRL_ACK,
            session,
            accepted_offset: accepted,
            state,
            error: 0,
            committed_offset: committed,
            total_size: accepted.max(committed),
            max_file_size: 2 * 1024 * 1024,
            capabilities: CAP_SIGNATURE_REQUIRED | CAP_KEY_CONFIGURED,
            board: 1,
            speed: 1,
            max_data: DATA_BYTES_MAX as u8,
            window: 8,
        }
    }

    fn ota_package(profile: BuildProfile) -> OtaPackage {
        let (image, manifest) = package_fixture(profile);
        validate_package(image, manifest, profile, false).unwrap()
    }

    fn success_script(package: &OtaPackage, session: u32) -> Vec<Status> {
        let size = package.image.len() as u32;
        let batch = (DATA_BYTES_MAX * 8) as u32;
        let mut statuses = vec![
            status(0, STATE_IDLE, 0, 0),
            status(session, STATE_AUTHORIZING, 0, 0),
            status(session, STATE_RECEIVING, 0, 0),
        ];
        let mut accepted = 0;
        while accepted < size {
            accepted = accepted.saturating_add(batch).min(size);
            statuses.push(status(session, STATE_RECEIVING, accepted, 0));
        }
        let mut final_status = status(session, STATE_READY_REBOOT, size, size);
        final_status.total_size = size;
        statuses.push(final_status);
        statuses
    }

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
                key_id: OTA_RELEASE_KEY_ID.to_owned(),
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
    fn renders_device_ota_errors_for_users() {
        assert_eq!(ota_error_name(9), "BAD_TARGET");
        assert_eq!(ota_error_name(12), "FLASH");
        assert_eq!(ota_error_name(255), "UNKNOWN");
        assert!(ota_error_hint(9).contains("profile/version"));
    }

    #[test]
    fn post_commit_disconnect_never_masks_a_protocol_rejection() {
        let rejected = anyhow::anyhow!(
            "device rejected OTA command: HASH (error=13), state=6, accepted=927360"
        );
        assert!(!is_expected_post_commit_disconnect(&rejected));

        let generic_hid_error = anyhow::anyhow!("failed to read OTA status HID Feature Report");
        assert!(!is_expected_post_commit_disconnect(&generic_hid_error));

        let disconnected = anyhow::anyhow!(
            "failed to read OTA status HID Feature Report: HIDAPI device not connected"
        );
        assert!(is_expected_post_commit_disconnect(&disconnected));
    }

    #[test]
    fn status_crc_corruption_is_rejected() {
        let mut wire = frame(CTRL_STATUS, 0, 0, &[]).unwrap();
        // Convert the request into a structurally valid status first.
        wire[4] = CTRL_ACK;
        wire[13] = 44;
        let crc = crc32fast::hash(&wire[1..60]);
        wire[60..64].copy_from_slice(&crc.to_le_bytes());
        assert!(decode_status(&wire).is_ok());

        wire[20] ^= 0x80;
        assert!(decode_status(&wire).is_err());
    }

    #[test]
    fn validates_profile_bound_raw_package() {
        let (image, manifest) = package_fixture(BuildProfile::Diagnostic);
        let package =
            validate_package(image.clone(), manifest, BuildProfile::Diagnostic, false).unwrap();
        assert_eq!(package.image, image);
        assert_eq!(package.profile, BuildProfile::Diagnostic);

        let (image, manifest) = package_fixture(BuildProfile::Diagnostic);
        assert!(validate_package(image, manifest, BuildProfile::Standard, false).is_err());

        let (image, manifest) = package_fixture(BuildProfile::Diagnostic);
        let error = validate_package(image, manifest, BuildProfile::Diagnostic, true).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("P-256 signature verification failed")
        );
    }

    #[test]
    fn scripted_transport_completes_full_state_machine() {
        let package = ota_package(BuildProfile::Standard);
        let session = 0x1020_3040;
        let mut transport = MockTransport::scripted(success_script(&package, session));
        let mut progress = Vec::new();

        run_update(&mut transport, &package, session, &mut |line| {
            progress.push(line)
        })
        .unwrap();

        assert_eq!(transport.abort_count(), 0);
        assert!(!transport.output_frames.is_empty());
        assert_eq!(transport.feature_frames[0][4], CTRL_BEGIN);
        assert_eq!(transport.feature_frames[1][4], CTRL_AUTH);
        assert_eq!(transport.feature_frames[2][4], CTRL_AUTH);
        assert_eq!(transport.feature_frames.last().unwrap()[4], CTRL_COMMIT);
        assert!(progress.iter().any(|line| line.contains("100%")));
    }

    #[test]
    fn authorization_rejection_aborts_without_sending_image() {
        let package = ota_package(BuildProfile::Diagnostic);
        let session = 0x1122_3344;
        let mut rejected = status(session, STATE_ERROR, 0, 0);
        rejected.opcode = CTRL_ERROR;
        rejected.error = 17;
        let mut transport = MockTransport::scripted([
            status(0, STATE_IDLE, 0, 0),
            status(session, STATE_AUTHORIZING, 0, 0),
            rejected,
        ]);

        let error = run_update(&mut transport, &package, session, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("AUTH_FAILED"));
        assert!(transport.output_frames.is_empty());
        assert_eq!(transport.abort_count(), 1);
    }

    #[test]
    fn data_transport_failure_aborts_session() {
        let package = ota_package(BuildProfile::Standard);
        let session = 0x5566_7788;
        let mut transport = MockTransport::scripted([
            status(0, STATE_IDLE, 0, 0),
            status(session, STATE_AUTHORIZING, 0, 0),
            status(session, STATE_RECEIVING, 0, 0),
        ]);
        transport.fail_output_at = Some(2);

        let error = run_update(&mut transport, &package, session, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("injected OTA DATA write failure"));
        assert_eq!(transport.abort_count(), 1);
    }

    #[test]
    fn inconsistent_final_offsets_are_rejected_and_aborted() {
        let package = ota_package(BuildProfile::Standard);
        let session = 0x99aa_bbcc;
        let mut script = success_script(&package, session);
        let final_status = script.last_mut().unwrap();
        final_status.committed_offset = final_status.committed_offset.saturating_sub(1);
        let mut transport = MockTransport::scripted(script);

        let error = run_update(&mut transport, &package, session, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("inconsistent final OTA offsets"));
        assert_eq!(transport.abort_count(), 1);
    }

    #[test]
    fn protocol_error_after_commit_is_never_accepted_as_reboot() {
        let package = ota_package(BuildProfile::Standard);
        let session = 0xddee_ff01;
        let mut script = success_script(&package, session);
        let final_status = script.last_mut().unwrap();
        final_status.state = STATE_ERROR;
        final_status.opcode = CTRL_ERROR;
        final_status.error = 13;
        let mut transport = MockTransport::scripted(script);

        let error = run_update(&mut transport, &package, session, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("HASH"));
        assert_eq!(transport.abort_count(), 1);
    }
}
