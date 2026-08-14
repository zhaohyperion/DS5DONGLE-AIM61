#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]

mod device_test;
mod diagnostics;
mod guided_test;
mod ota_client;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const PRODUCT_NAME: &str = "DS5Dongle AIM61 工具中心";
const FLASHER_VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES_API: &str =
    "https://api.github.com/repos/zhaohyperion/DS5DONGLE-AIM61/releases?per_page=30";
const WCH_DRIVER_URL: &str = "https://www.wch-ic.com/download/file?id=65";
const WCH_SIGNER_FRAGMENT: &str = "Nanjing Qinheng Microelectronics Co., Ltd.";
const KNOWN_WCH_DRIVER_SHA256: &str =
    "458c37bdafbe4ce3cd0baf728c232b4b765b36d7463956e9b94cdf099212cad1";

fn flash_config(firmware_name: &str) -> String {
    format!(
        r#"[cfg]
# 0: no erase, 1: programmed section erase, 2: chip erase
erase = 1
skip_mode = 0x0, 0x0
boot2_isp_mode = 0

[boot2]
filedir = ./boot2_bl616_*.bin
address = 0x000000

[partition]
filedir = ./partition.bin
address = 0xE000

[FW]
filedir = ./{firmware_name}
address = @partition
"#
    )
}

const BLFLASH_BYTES: &[u8] = include_bytes!(env!("M61_BLFLASHCOMMAND_EMBED"));
const BLFLASH_SHA256: &str = "329517ce5220a2807f4e24e3fc745a4e249818914a7ac93da727d7566f36fc67";
const EFLASH_LOADER_INI_BYTES: &[u8] = include_bytes!(env!("M61_EFLASH_LOADER_INI_EMBED"));
const EFLASH_LOADER_INI_SHA256: &str =
    "78a33e4ffecb682673328135d87bb413f9c0b92bcdd330463e5934843fc8de0c";
const EFLASH_LOADER_CONF_BYTES: &[u8] = include_bytes!(env!("M61_EFLASH_LOADER_CONF_EMBED"));
const EFLASH_LOADER_CONF_SHA256: &str =
    "78a33e4ffecb682673328135d87bb413f9c0b92bcdd330463e5934843fc8de0c";
const FLASH_PARA_BYTES: &[u8] = include_bytes!(env!("M61_FLASH_PARA_EMBED"));
const FLASH_PARA_SHA256: &str = "002c41f38bb652bdaf89136183aaef70ae11abbf85f554a59d7242748f32e1e0";
const PARTITION_NAME: &str = "partition.bin";
const FIRMWARE_MANIFEST_NAME: &str = "firmware.json";
const CHECKSUM_MANIFEST_NAME: &str = "SHA256SUMS.txt";
const MAX_FIRMWARE_BYTES: usize = 8 * 1024 * 1024;
const MAX_BOOT2_BYTES: usize = 1024 * 1024;
const SONY_VENDOR_ID: u16 = 0x054c;
const DUALSENSE_PRODUCT_IDS: [u16; 2] = [0x0ce6, 0x0df2];
const FIRMWARE_VERSION_REPORT_ID: u8 = 0xf8;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    html_url: String,
    published_at: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Clone, Debug)]
struct FlashRelease {
    tag: String,
    name: String,
    url: String,
    published_at: Option<String>,
    prerelease: bool,
    archive: GithubAsset,
    board: Board,
    usb_speed: UsbSpeed,
    profile: BuildProfile,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum BuildProfile {
    Standard,
    Diagnostic,
}

impl BuildProfile {
    fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Diagnostic => "Diagnostic",
        }
    }
}

fn default_build_profile() -> BuildProfile {
    BuildProfile::Standard
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum UsbSpeed {
    Fs,
    Hs,
}

impl UsbSpeed {
    fn label(self) -> &'static str {
        match self {
            Self::Fs => "Full-Speed",
            Self::Hs => "High-Speed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Board {
    Lctech616,
    Aim61,
    M0sdock,
}

impl Board {
    fn id(self) -> &'static str {
        match self {
            Self::Lctech616 => "lctech616",
            Self::Aim61 => "aim61",
            Self::M0sdock => "m0sdock",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Lctech616 => "LCTech BL616",
            Self::Aim61 => "AI-M61-32S-KIT",
            Self::M0sdock => "Sipeed M0S Dock",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct FirmwareManifest {
    schema: u32,
    project: String,
    version: String,
    board: Board,
    usb_speed: UsbSpeed,
    #[serde(default = "default_build_profile")]
    profile: BuildProfile,
    chip: String,
    flash_size: u32,
    boot2: String,
    partition: String,
    firmware: String,
}

#[derive(Clone, Debug)]
struct FirmwareSet {
    label: String,
    boot2_name: String,
    boot2: Vec<u8>,
    partition: Vec<u8>,
    firmware: Vec<u8>,
    firmware_name: String,
    manifest: FirmwareManifest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FirmwareMode {
    Online,
    LocalZip,
    LocalDirectory,
}

#[derive(Clone, Debug)]
enum SelectedFirmware {
    Online(FlashRelease),
    Local(FirmwareSet),
}

impl FirmwareMode {
    fn tr(self, language: Language) -> &'static str {
        match self {
            Self::Online => language.tr("在线 Release", "Online Release"),
            Self::LocalZip => language.tr("本地固件 ZIP", "Local firmware ZIP"),
            Self::LocalDirectory => language.tr("本地目录（高级）", "Local directory (advanced)"),
        }
    }
}

#[derive(Debug, Default)]
struct Options {
    port: Option<String>,
    baud: u32,
    dry_run: bool,
    assume_yes: bool,
    list: bool,
    device_info: bool,
    diagnostics: bool,
    list_releases: bool,
    assets_info: bool,
    install_driver: bool,
    release: Option<String>,
    verify_release: bool,
    tool_preflight: bool,
    board: Option<Board>,
    usb_speed: Option<UsbSpeed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Ch340Device {
    name: String,
    instance_id: String,
    error_code: u32,
    status: String,
    port: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SerialDiagnosticRecord<'a> {
    name: &'a str,
    port: Option<&'a str>,
    target_ch340: bool,
    usable: bool,
    pnp_error_code: u32,
    status: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FlasherDiagnosticBundle<'a> {
    schema: &'static str,
    created_at_unix_ms: u64,
    flasher_version: &'static str,
    serial_devices: Vec<SerialDiagnosticRecord<'a>>,
    runtime_devices: &'a [diagnostics::DeviceDiagnostic],
    test_phases: Vec<serde_json::Value>,
    rx_metrics: serde_json::Value,
    tx_metrics: serde_json::Value,
    audio_input_metrics: serde_json::Value,
    audio_output_metrics: serde_json::Value,
    user_confirmations: Vec<serde_json::Value>,
    result: &'static str,
    raw_trace: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FirmwareDeviceInfo {
    product_name: String,
    vendor_id: u16,
    product_id: u16,
    firmware_version: String,
    build_profile: String,
}

struct RuntimeDirectory {
    path: PathBuf,
    preserve: bool,
}

impl Drop for RuntimeDirectory {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn main() -> ExitCode {
    if env::args_os().len() == 1 {
        return match run_gui() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Failed to start DS5Dongle Flasher GUI: {error}");
                ExitCode::FAILURE
            }
        };
    }
    run_cli()
}

fn run_cli() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("\n错误 / Error: {error:#}");
            if io::stdin().is_terminal() {
                eprintln!("按 Enter 退出 / Press Enter to exit.");
                let _ = read_line();
            }
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let options = parse_options(env::args().skip(1))?;
    if !cfg!(windows) {
        bail!("DS5Dongle Flasher currently supports Windows 10/11 only");
    }

    print_banner();
    verify_embedded_tool()?;

    if options.assets_info {
        print_tool_info();
        return Ok(());
    }

    if options.list {
        print_devices(&probe_ch340_devices()?);
        return Ok(());
    }

    if options.device_info {
        print_firmware_devices(&probe_firmware_devices()?);
        return Ok(());
    }

    if options.diagnostics {
        let serial_devices = probe_ch340_devices()?;
        let runtime_devices = diagnostics::probe_runtime_diagnostics()?;
        println!(
            "{}",
            diagnostic_bundle_json(&serial_devices, &runtime_devices)?
        );
        return Ok(());
    }

    if options.install_driver {
        install_ch340_driver(options.assume_yes)?;
        return Ok(());
    }

    let client = github_client()?;
    let mut releases = fetch_flash_releases(&client)?;
    if let Some(board) = options.board {
        releases.retain(|release| release.board == board);
    }
    if let Some(speed) = options.usb_speed {
        releases.retain(|release| release.usb_speed == speed);
    }
    if releases.is_empty() {
        bail!("no firmware release matches the requested board/USB mode");
    }
    if options.list_releases {
        print_releases(&releases);
        return Ok(());
    }
    let release = choose_release(&releases, options.release.as_deref(), options.assume_yes)?;
    println!(
        "已选择固件 / Firmware: {} — {} / {} — {}",
        release.tag,
        release.board.label(),
        release.usb_speed.label(),
        release.name
    );
    if options.verify_release {
        let _runtime = prepare_runtime(&client, &release)?;
        println!("\n{} 的完整刷写文件已下载并通过 SHA256 校验。", release.tag);
        println!("Verification completed; no device was opened and nothing was flashed.");
        return Ok(());
    }
    if options.tool_preflight {
        let runtime = prepare_runtime(&client, &release)?;
        run_tool_preflight(&runtime.path)?;
        println!("Bouffalo support-file preflight passed; no real COM port was opened.");
        return Ok(());
    }

    let mut devices = probe_ch340_devices()?;

    let port = if let Some(port) = options.port.as_deref() {
        normalize_port(port)?
    } else {
        loop {
            let usable: Vec<&Ch340Device> = devices
                .iter()
                .filter(|device| {
                    is_ch340_device(device) && device.error_code == 0 && device.port.is_some()
                })
                .collect();

            if !usable.is_empty() {
                break choose_port(&usable)?;
            }

            if devices.is_empty() {
                println!("未检测到可用串口或 BootROM COM 设备。");
                println!("请连接开发板并进入 ISP 模式，然后按 Enter 重试；输入 q 退出。");
                let answer = read_line()?;
                if answer.trim().eq_ignore_ascii_case("q") {
                    bail!("cancelled by user");
                }
            } else {
                println!("检测到了串口硬件，但没有可用 COM 口，驱动可能未安装或异常。");
                print_devices(&devices);
                if prompt_yes_no(
                    "现在从 WCH 官方下载并安装 CH340 驱动？",
                    true,
                    options.assume_yes,
                )? {
                    install_ch340_driver(options.assume_yes)?;
                    println!("驱动安装器已结束。请重新插拔开发板，然后按 Enter 继续。");
                    let _ = read_line()?;
                } else {
                    bail!("a working serial driver is required before flashing");
                }
            }
            devices = probe_ch340_devices()?;
        }
    };

    println!("\n已选择 / Selected: {port}");
    println!("固件来源 / Source: {}", release.url);
    println!("\n请让开发板进入 UART ISP 下载模式：");
    println!("  1. 按住 BOOT");
    println!("  2. 点按并松开 RESET/RST");
    println!("  3. 松开 BOOT");
    println!("完成后按 Enter 开始刷写。刷写中不要拔线或按 Reset。");
    if !options.assume_yes {
        let answer = read_line()?;
        if answer.trim().eq_ignore_ascii_case("q") {
            bail!("cancelled by user");
        }
    }

    if options.dry_run {
        println!(
            "\n[dry-run] 将下载并刷写 {}，使用 {port} @ {} baud；未启动下载或刷写。",
            release.tag, options.baud
        );
        return Ok(());
    }

    let mut runtime = prepare_runtime(&client, &release)?;
    let first_status = run_flash(&runtime.path, &port, options.baud)?;
    if first_status.success() {
        println!("\n刷写成功。请松开 BOOT，并按一次 RESET/RST 正常启动开发板。");
        println!("Flashing completed successfully.");
        if !options.assume_yes {
            println!("按 Enter 退出 / Press Enter to exit.");
            let _ = read_line()?;
        }
        return Ok(());
    }

    runtime.preserve = true;
    eprintln!("\n第一次刷写失败，退出码：{:?}", first_status.code());
    eprintln!("日志保留在：{}", runtime.path.display());
    if options.baud != 115_200
        && prompt_yes_no(
            "确认仍处于 BOOT+RESET 的 ISP 模式后，是否用兼容档 115200 baud 重试？",
            true,
            options.assume_yes,
        )?
    {
        let retry_status = run_flash(&runtime.path, &port, 115_200)?;
        if retry_status.success() {
            runtime.preserve = false;
            println!("\n115200 baud 重试刷写成功。请按 RESET/RST 正常启动。");
            return Ok(());
        }
        bail!(
            "115200-baud retry also failed; logs: {}",
            runtime.path.display()
        );
    }

    bail!("flashing failed; logs: {}", runtime.path.display())
}

fn print_banner() {
    println!("============================================================");
    println!("{PRODUCT_NAME} {FLASHER_VERSION}");
    println!("Windows 一键刷写工具 / Windows one-file flasher");
    println!("============================================================\n");
}

fn print_help() {
    println!(
        "{PRODUCT_NAME} {FLASHER_VERSION}\n\n\
         Usage: ds5dongle-flasher.exe [options]\n\n\
         Options:\n  \
           --board BOARD     aim61 (preferred), lctech616, or m0sdock\n  \
           --usb-speed MODE  fs (recommended) or hs\n  \
           --port COM5       Select a serial/BootROM COM port\n  \
           --baud RATE       460800 (default) or 115200\n  \
           --list            List detected M61 CH340 devices\n  \
           --device-info     Read running DS5DONGLE-AIM61 firmware information over USB HID\n  \
           --diagnostics     Capture CH340 status and the native 0xFD runtime diagnostic snapshot\n  \
           --list-releases   List complete firmware Releases\n  \
           --release TAG     Select a Release without the menu\n  \
           --verify-release  Download and verify a Release without flashing\n  \
           --tool-preflight  Verify embedded Bouffalo support files without a board\n  \
           --assets-info     Print the embedded flashing-tool hash\n  \
           --install-driver  Download, verify, and launch the official WCH driver\n  \
           --dry-run         Check everything without starting the flash process\n  \
           --yes             Accept prompts (intended for controlled automation)\n  \
           -h, --help        Show this help"
    );
}

fn parse_options(arguments: impl Iterator<Item = String>) -> Result<Options> {
    let mut options = Options {
        baud: 460_800,
        ..Options::default()
    };
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--port" => {
                options.port = Some(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow!("--port requires COM name"))?,
                );
            }
            "--board" => {
                options.board = Some(match arguments.next().as_deref() {
                    Some("lctech616") => Board::Lctech616,
                    Some("aim61") => Board::Aim61,
                    Some("m0sdock") => Board::M0sdock,
                    _ => bail!("--board must be lctech616, aim61, or m0sdock"),
                });
            }
            "--usb-speed" => {
                options.usb_speed = Some(match arguments.next().as_deref() {
                    Some("fs") => UsbSpeed::Fs,
                    Some("hs") => UsbSpeed::Hs,
                    _ => bail!("--usb-speed must be fs or hs"),
                });
            }
            "--baud" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| anyhow!("--baud requires a value"))?;
                options.baud = value.parse().context("invalid --baud value")?;
                if !matches!(options.baud, 460_800 | 115_200) {
                    bail!("--baud must be 460800 or 115200");
                }
            }
            "--dry-run" => options.dry_run = true,
            "--yes" => options.assume_yes = true,
            "--list" => options.list = true,
            "--device-info" => options.device_info = true,
            "--diagnostics" => options.diagnostics = true,
            "--list-releases" => options.list_releases = true,
            "--release" => {
                options.release = Some(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow!("--release requires a tag"))?,
                );
            }
            "--assets-info" => options.assets_info = true,
            "--install-driver" => options.install_driver = true,
            "--verify-release" => options.verify_release = true,
            "--tool-preflight" => options.tool_preflight = true,
            _ => bail!("unknown option: {argument} (use --help)"),
        }
    }
    Ok(options)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn verify_embedded_tool() -> Result<()> {
    for (name, bytes, expected) in [
        ("BLFlashCommand.exe", BLFLASH_BYTES, BLFLASH_SHA256),
        (
            "eflash_loader_cfg.ini",
            EFLASH_LOADER_INI_BYTES,
            EFLASH_LOADER_INI_SHA256,
        ),
        (
            "eflash_loader_cfg.conf",
            EFLASH_LOADER_CONF_BYTES,
            EFLASH_LOADER_CONF_SHA256,
        ),
        ("flash_para.bin", FLASH_PARA_BYTES, FLASH_PARA_SHA256),
    ] {
        let actual = sha256(bytes);
        if actual != expected {
            bail!("embedded {name} checksum mismatch: expected {expected}, got {actual}");
        }
    }
    Ok(())
}

fn print_tool_info() {
    println!("Embedded flashing tool:");
    println!(
        "{}  BLFlashCommand.exe  ({} bytes)",
        BLFLASH_SHA256,
        BLFLASH_BYTES.len()
    );
    println!("{}  eflash_loader_cfg.ini", EFLASH_LOADER_INI_SHA256);
    println!("{}  eflash_loader_cfg.conf", EFLASH_LOADER_CONF_SHA256);
    println!("{}  flash_para.bin", FLASH_PARA_SHA256);
    println!("Firmware ZIP is selected from GitHub Releases or local storage at runtime.");
}

fn github_client() -> Result<Client> {
    Client::builder()
        .user_agent(format!("DS5Dongle-Flasher/{FLASHER_VERSION}"))
        .build()
        .context("failed to initialize HTTPS client")
}

fn sha256_digest(asset: &GithubAsset) -> Option<&str> {
    asset.digest.as_deref()?.strip_prefix("sha256:")
}

fn asset_variant(name: &str) -> Option<(Board, UsbSpeed, BuildProfile)> {
    let lower = name.to_ascii_lowercase();
    if !lower.starts_with("ds5dongle-") || !lower.ends_with(".zip") {
        return None;
    }
    let board = if lower.contains("-lctech616-") {
        Board::Lctech616
    } else if lower.contains("-aim61-") {
        Board::Aim61
    } else if lower.contains("-m0sdock-") {
        Board::M0sdock
    } else {
        return None;
    };
    let speed = if lower.contains("-fs-") {
        UsbSpeed::Fs
    } else if lower.contains("-hs-") {
        UsbSpeed::Hs
    } else {
        return None;
    };
    let profile = if lower.contains("-diag-") {
        BuildProfile::Diagnostic
    } else {
        BuildProfile::Standard
    };
    Some((board, speed, profile))
}

fn fetch_flash_releases(client: &Client) -> Result<Vec<FlashRelease>> {
    println!("正在读取 GitHub 固件列表 / Loading firmware Releases...");
    let github_releases: Vec<GithubRelease> = client
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .context("failed to query GitHub Releases")?
        .error_for_status()
        .context("GitHub Releases API returned an error")?
        .json()
        .context("failed to parse GitHub Releases response")?;

    let mut releases = Vec::new();
    for release in github_releases.into_iter().filter(|release| !release.draft) {
        for asset in release
            .assets
            .iter()
            .filter(|asset| sha256_digest(asset).is_some())
        {
            let Some((board, usb_speed, profile)) = asset_variant(&asset.name) else {
                continue;
            };
            releases.push(FlashRelease {
                tag: release.tag_name.clone(),
                name: release
                    .name
                    .clone()
                    .unwrap_or_else(|| release.tag_name.clone()),
                url: release.html_url.clone(),
                published_at: release.published_at.clone(),
                prerelease: release.prerelease,
                archive: asset.clone(),
                board,
                usb_speed,
                profile,
            });
        }
    }

    if releases.is_empty() {
        bail!("no Release contains verified DS5Dongle-<board>-<fs|hs>-*.zip assets");
    }
    Ok(releases)
}

fn print_releases(releases: &[FlashRelease]) {
    println!("\n可刷写固件 / Flashable firmware:");
    for (index, release) in releases.iter().enumerate() {
        let channel = if release.prerelease {
            "预发布 / prerelease"
        } else {
            "稳定 / stable"
        };
        let date = release
            .published_at
            .as_deref()
            .and_then(|value| value.get(..10))
            .unwrap_or("unknown date");
        println!(
            "  {}. {} | {} / {} / {} | {} | {} | {:.1} KiB\n     {}",
            index + 1,
            release.tag,
            release.board.label(),
            release.usb_speed.label(),
            release.profile.label(),
            channel,
            date,
            release.archive.size as f64 / 1024.0,
            release.name
        );
    }
}

/* GitHub returns releases newest-first. Prefer the newest AIM61 HS Standard
 * asset even when it is a prerelease; Diagnostic stays behind the GUI's
 * explicit advanced switch. */
fn preferred_release_index(releases: &[FlashRelease]) -> Option<usize> {
    releases
        .iter()
        .position(|release| {
            release.board == Board::Aim61
                && release.usb_speed == UsbSpeed::Hs
                && release.profile == BuildProfile::Standard
        })
        .or_else(|| releases.iter().position(|release| !release.prerelease))
        .or_else(|| (!releases.is_empty()).then_some(0))
}

fn choose_release(
    releases: &[FlashRelease],
    requested: Option<&str>,
    assume_yes: bool,
) -> Result<FlashRelease> {
    if let Some(tag) = requested {
        let matching: Vec<FlashRelease> = releases
            .iter()
            .filter(|release| release.tag.eq_ignore_ascii_case(tag))
            .cloned()
            .collect();
        return preferred_release_index(&matching)
            .map(|index| matching[index].clone())
            .ok_or_else(|| {
                anyhow!("Release {tag} is unavailable or lacks a complete verified flash set")
            });
    }

    if assume_yes {
        return preferred_release_index(releases)
            .map(|index| releases[index].clone())
            .ok_or_else(|| anyhow!("no flashable Release"));
    }

    print_releases(releases);
    println!("请选择要下载并刷写的固件版本。通常选择列表中最新的稳定版。");
    loop {
        print!("输入序号 / Select firmware: ");
        io::stdout().flush()?;
        let answer = read_line()?;
        if answer.trim().eq_ignore_ascii_case("q") {
            bail!("cancelled by user");
        }
        if let Ok(index) = answer.trim().parse::<usize>()
            && (1..=releases.len()).contains(&index)
        {
            return Ok(releases[index - 1].clone());
        }
        println!("请输入 1 到 {}，或输入 q 退出。", releases.len());
    }
}

fn download_release_asset(client: &Client, asset: &GithubAsset, destination: &Path) -> Result<()> {
    println!(
        "下载 {} ({:.1} KiB)...",
        asset.name,
        asset.size as f64 / 1024.0
    );
    let mut response = client
        .get(&asset.browser_download_url)
        .send()
        .with_context(|| format!("failed to download {}", asset.name))?
        .error_for_status()
        .with_context(|| format!("download server rejected {}", asset.name))?;
    let mut bytes = Vec::with_capacity(asset.size.try_into().unwrap_or(0));
    response
        .copy_to(&mut bytes)
        .with_context(|| format!("failed while downloading {}", asset.name))?;
    if bytes.len() as u64 != asset.size {
        bail!(
            "download size mismatch for {}: expected {}, got {}",
            asset.name,
            asset.size,
            bytes.len()
        );
    }
    let expected = sha256_digest(asset)
        .ok_or_else(|| anyhow!("GitHub did not provide a SHA256 digest for {}", asset.name))?;
    let actual = sha256(&bytes);
    if actual != expected {
        bail!(
            "download checksum mismatch for {}: expected {}, got {}",
            asset.name,
            expected,
            actual
        );
    }
    fs::write(destination, bytes)
        .with_context(|| format!("failed to save {}", destination.display()))?;
    println!("校验通过 / Verified: {actual}");
    Ok(())
}

fn validate_firmware_set(
    label: String,
    manifest: FirmwareManifest,
    boot2: Vec<u8>,
    partition: Vec<u8>,
    firmware: Vec<u8>,
    checksum_manifest: Option<&str>,
) -> Result<FirmwareSet> {
    if manifest.schema != 1 || manifest.project != "DS5Dongle" {
        bail!("unsupported firmware manifest");
    }
    if manifest.chip != "bl616" {
        bail!("unsupported chip: {}", manifest.chip);
    }
    if !matches!(manifest.flash_size, 4 | 8 | 16) {
        bail!("invalid flash_size; expected MiB value 4, 8, or 16");
    }
    let boot2_name = manifest.boot2.clone();
    let firmware_name = manifest.firmware.clone();
    if manifest.partition != PARTITION_NAME {
        bail!("partition must be partition.bin");
    }
    for name in [&boot2_name, &firmware_name] {
        if Path::new(name)
            .file_name()
            .is_none_or(|value| value != name.as_str())
        {
            bail!("unsafe manifest filename: {name}");
        }
    }
    let expected_firmware = format!(
        "ds5dongle-{}{}{}.bin",
        manifest.board.id(),
        if manifest.usb_speed == UsbSpeed::Hs {
            "-hs"
        } else {
            ""
        },
        if manifest.profile == BuildProfile::Diagnostic {
            "-diag"
        } else {
            ""
        },
    );
    if firmware_name != expected_firmware {
        bail!("firmware filename does not match board/USB mode: expected {expected_firmware}");
    }
    if !boot2_name.starts_with("boot2_bl616_") || !boot2_name.ends_with(".bin") {
        bail!("invalid BL616 boot2 filename: {boot2_name}");
    }
    if !boot2_name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        bail!("unsafe BL616 boot2 filename: {boot2_name}");
    }
    if boot2.len() < 16 * 1024 || boot2.len() > MAX_BOOT2_BYTES {
        bail!("invalid boot2 size: {} bytes", boot2.len());
    }
    if partition.len() != 308 {
        bail!(
            "invalid partition.bin size: {} bytes; expected 308",
            partition.len()
        );
    }
    if firmware.len() < 64 * 1024 || firmware.len() > MAX_FIRMWARE_BYTES {
        bail!(
            "invalid application firmware size: {} bytes",
            firmware.len()
        );
    }

    if let Some(manifest) = checksum_manifest {
        let expected = manifest
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                Some((
                    fields.next()?.to_ascii_lowercase(),
                    fields.next()?.replace('*', ""),
                ))
            })
            .collect::<Vec<_>>();
        for (name, bytes) in [
            (boot2_name.as_str(), boot2.as_slice()),
            (PARTITION_NAME, partition.as_slice()),
            (firmware_name.as_str(), firmware.as_slice()),
        ] {
            let expected_hash = expected
                .iter()
                .find(|(_, manifest_name)| {
                    Path::new(manifest_name)
                        .file_name()
                        .is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case(name))
                })
                .map(|(hash, _)| hash)
                .ok_or_else(|| anyhow!("checksum manifest is missing {name}"))?;
            let actual = sha256(bytes);
            if &actual != expected_hash {
                bail!("checksum mismatch for {name}: expected {expected_hash}, got {actual}");
            }
        }
    }

    Ok(FirmwareSet {
        label,
        boot2_name,
        boot2,
        partition,
        firmware,
        firmware_name,
        manifest,
    })
}

fn read_zip_entry_limited(
    entry: &mut zip::read::ZipFile<'_, File>,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0).min(limit));
    entry
        .take(u64::try_from(limit + 1).unwrap())
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("ZIP entry exceeds the {}-byte safety limit", limit);
    }
    Ok(bytes)
}

fn read_firmware_zip(path: &Path, require_manifest: bool) -> Result<FirmwareSet> {
    let file = File::open(path)
        .with_context(|| format!("failed to open firmware ZIP: {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("invalid or unsupported firmware ZIP")?;
    if archive.len() > 128 {
        bail!("firmware ZIP has too many entries: {}", archive.len());
    }

    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("unsafe path in firmware ZIP: {}", entry.name()))?;
        let Some(file_name) = enclosed
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
        else {
            continue;
        };
        let size = usize::try_from(entry.size()).context("ZIP entry is too large")?;
        if size > MAX_FIRMWARE_BYTES {
            bail!("ZIP entry is too large: {file_name}");
        }
        if files.contains_key(&file_name.to_ascii_lowercase()) {
            bail!("duplicate ZIP filename: {file_name}");
        }
        files.insert(
            file_name.to_ascii_lowercase(),
            read_zip_entry_limited(&mut entry, MAX_FIRMWARE_BYTES)?,
        );
    }
    let manifest_bytes = files
        .remove(FIRMWARE_MANIFEST_NAME)
        .ok_or_else(|| anyhow!("firmware ZIP is missing {FIRMWARE_MANIFEST_NAME}"))?;
    let firmware_manifest: FirmwareManifest =
        serde_json::from_slice(&manifest_bytes).context("invalid firmware.json")?;
    let checksum = files.remove(&CHECKSUM_MANIFEST_NAME.to_ascii_lowercase());
    if require_manifest && checksum.is_none() {
        bail!("official firmware ZIP is missing {CHECKSUM_MANIFEST_NAME}");
    }
    let checksum = checksum
        .map(String::from_utf8)
        .transpose()
        .context("SHA256 manifest is not UTF-8")?;
    let boot2 = files
        .remove(&firmware_manifest.boot2.to_ascii_lowercase())
        .ok_or_else(|| anyhow!("firmware ZIP is missing {}", firmware_manifest.boot2))?;
    let partition = files
        .remove(&firmware_manifest.partition.to_ascii_lowercase())
        .ok_or_else(|| anyhow!("firmware ZIP is missing {}", firmware_manifest.partition))?;
    let firmware = files
        .remove(&firmware_manifest.firmware.to_ascii_lowercase())
        .ok_or_else(|| anyhow!("firmware ZIP is missing {}", firmware_manifest.firmware))?;
    validate_firmware_set(
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        firmware_manifest,
        boot2,
        partition,
        firmware,
        checksum.as_deref(),
    )
}

fn read_firmware_directory(path: &Path) -> Result<FirmwareSet> {
    let manifest_path = path.join(FIRMWARE_MANIFEST_NAME);
    let firmware_manifest: FirmwareManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("missing {}", manifest_path.display()))?,
    )
    .context("invalid firmware.json")?;
    let checksum_path = path.join(CHECKSUM_MANIFEST_NAME);
    let checksum = checksum_path
        .is_file()
        .then(|| fs::read_to_string(&checksum_path))
        .transpose()
        .context("failed to read SHA256 manifest")?;
    validate_firmware_set(
        path.display().to_string(),
        firmware_manifest.clone(),
        fs::read(path.join(&firmware_manifest.boot2)).context("missing boot2")?,
        fs::read(path.join(&firmware_manifest.partition)).context("missing partition.bin")?,
        fs::read(path.join(&firmware_manifest.firmware)).context("missing application firmware")?,
        checksum.as_deref(),
    )
}

fn background_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn powershell_output(script: &str) -> Result<std::process::Output> {
    background_command("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .output()
        .context("unable to start Windows PowerShell")
}

fn probe_ch340_devices() -> Result<Vec<Ch340Device>> {
    let script = r#"
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.Encoding]::UTF8
Get-CimInstance Win32_PnPEntity | Where-Object { ([string]$_.Name) -match '\(COM[0-9]+\)' -or ([string]$_.PNPDeviceID) -like 'USB\VID_1A86&PID_7523*' } | ForEach-Object {
  $name=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$_.Name))
  $id=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$_.PNPDeviceID))
  "$name`t$id`t$([int]$_.ConfigManagerErrorCode)`t$([string]$_.Status)"
}
"#;
    let output = powershell_output(script)?;
    if !output.status.success() {
        bail!(
            "serial device query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_probe_output(&String::from_utf8_lossy(&output.stdout))
}

fn parse_probe_output(output: &str) -> Result<Vec<Ch340Device>> {
    let mut devices = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 4 {
            bail!("unexpected CH340 probe output: {line}");
        }
        let decode = |value: &str| -> Result<String> {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(value)
                .context("invalid base64 from CH340 probe")?;
            String::from_utf8(bytes).context("invalid UTF-8 from CH340 probe")
        };
        let name = decode(fields[0])?;
        let instance_id = decode(fields[1])?;
        devices.push(Ch340Device {
            port: find_com_port(&name),
            name,
            instance_id,
            error_code: fields[2].parse().context("invalid PnP error code")?,
            status: fields[3].to_owned(),
        });
    }
    Ok(devices)
}

fn find_com_port(name: &str) -> Option<String> {
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        if &bytes[index..index + 3] == b"COM" {
            let start = index + 3;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start {
                return Some(upper[index..end].to_owned());
            }
        }
        index += 1;
    }
    None
}

fn normalize_port(port: &str) -> Result<String> {
    let upper = port.trim().to_ascii_uppercase();
    if upper.len() <= 3
        || !upper.starts_with("COM")
        || !upper[3..].bytes().all(|byte| byte.is_ascii_digit())
    {
        bail!("invalid COM port: {port}");
    }
    Ok(upper)
}

fn print_devices(devices: &[Ch340Device]) {
    if devices.is_empty() {
        println!("No connected serial/BootROM COM device was detected.");
        return;
    }
    for (index, device) in devices.iter().enumerate() {
        println!(
            "{}. {} | port={} | target={} | PnP={} ({}) | {}",
            index + 1,
            device.name,
            device.port.as_deref().unwrap_or("unavailable"),
            if is_ch340_device(device) {
                "CH340"
            } else {
                "ignored"
            },
            device.status,
            device.error_code,
            device.instance_id
        );
    }
}

fn is_ch340_device(device: &Ch340Device) -> bool {
    device
        .instance_id
        .to_ascii_uppercase()
        .contains("VID_1A86&PID_7523")
}

fn diagnostic_bundle_json(
    serial_devices: &[Ch340Device],
    runtime_devices: &[diagnostics::DeviceDiagnostic],
) -> Result<String> {
    let serial_devices = serial_devices
        .iter()
        .map(|device| SerialDiagnosticRecord {
            name: &device.name,
            port: device.port.as_deref(),
            target_ch340: is_ch340_device(device),
            usable: is_ch340_device(device) && device.error_code == 0 && device.port.is_some(),
            pnp_error_code: device.error_code,
            status: &device.status,
        })
        .collect();
    serde_json::to_string_pretty(&FlasherDiagnosticBundle {
        schema: "ds5dongle-flasher-diagnostics/v2",
        created_at_unix_ms: diagnostics::now_unix_ms(),
        flasher_version: FLASHER_VERSION,
        serial_devices,
        runtime_devices,
        test_phases: Vec::new(),
        rx_metrics: serde_json::json!({"status": "notRun"}),
        tx_metrics: serde_json::json!({"status": "notRun"}),
        audio_input_metrics: serde_json::json!({"status": "notRun"}),
        audio_output_metrics: serde_json::json!({"status": "notRun"}),
        user_confirmations: Vec::new(),
        result: "snapshotOnly",
        raw_trace: Vec::new(),
    })
    .context("unable to serialize diagnostic bundle")
}

fn decode_firmware_version_report(report: &[u8]) -> Option<String> {
    decode_firmware_identity_report(report).map(|identity| identity.0)
}

fn decode_firmware_identity_report(report: &[u8]) -> Option<(String, String)> {
    let payload = report
        .first()
        .is_some_and(|byte| *byte == FIRMWARE_VERSION_REPORT_ID)
        .then(|| &report[1..])
        .unwrap_or(report);
    let end = payload
        .iter()
        .rposition(|byte| !matches!(byte, 0x00 | 0xff))
        .map(|index| index + 1)
        .unwrap_or(0);
    let identity = std::str::from_utf8(&payload[..end]).ok()?.trim();
    let (version, profile) = identity
        .split_once('|')
        .map_or((identity, "legacy"), |(version, profile)| {
            (version, profile)
        });
    if !matches!(profile, "standard" | "diagnostic" | "legacy") {
        return None;
    }
    let components = version
        .split('.')
        .map(|component| component.parse::<u16>().ok())
        .collect::<Option<Vec<_>>>()?;
    if components.len() != 3 || components.iter().any(|component| *component > 254) {
        return None;
    }
    Some((version.to_owned(), profile.to_owned()))
}

#[cfg(windows)]
fn probe_firmware_devices() -> Result<Vec<FirmwareDeviceInfo>> {
    let api = hidapi::HidApi::new().context("failed to initialize Windows HID access")?;
    let mut devices = Vec::new();
    for info in api.device_list().filter(|info| {
        info.vendor_id() == SONY_VENDOR_ID
            && DUALSENSE_PRODUCT_IDS.contains(&info.product_id())
            && info.usage_page() == 0x01
            && info.usage() == 0x05
    }) {
        let Ok(device) = info.open_device(&api) else {
            continue;
        };
        let mut report = [0_u8; 64];
        report[0] = FIRMWARE_VERSION_REPORT_ID;
        let Ok(length) = device.get_feature_report(&mut report) else {
            continue;
        };
        let Some((firmware_version, build_profile)) =
            decode_firmware_identity_report(&report[..length])
        else {
            continue;
        };
        devices.push(FirmwareDeviceInfo {
            product_name: info
                .product_string()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("DS5DONGLE-AIM61")
                .to_owned(),
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            firmware_version,
            build_profile,
        });
    }
    devices.sort_by(|left, right| {
        (&left.product_name, left.product_id, &left.firmware_version).cmp(&(
            &right.product_name,
            right.product_id,
            &right.firmware_version,
        ))
    });
    devices.dedup();
    Ok(devices)
}

#[cfg(not(windows))]
fn probe_firmware_devices() -> Result<Vec<FirmwareDeviceInfo>> {
    bail!("firmware information is available on Windows only")
}

fn print_firmware_devices(devices: &[FirmwareDeviceInfo]) {
    if devices.is_empty() {
        println!("No running DS5DONGLE-AIM61 firmware was detected over USB HID.");
        return;
    }
    for (index, device) in devices.iter().enumerate() {
        println!(
            "{}. {} | firmware={} | profile={} | VID:PID={:04X}:{:04X}",
            index + 1,
            device.product_name,
            device.firmware_version,
            device.build_profile,
            device.vendor_id,
            device.product_id
        );
    }
}

fn choose_port(devices: &[&Ch340Device]) -> Result<String> {
    if devices.len() == 1 {
        let device = devices[0];
        println!("自动检测到：{}", device.name);
        return Ok(device.port.clone().expect("filtered port"));
    }

    println!("检测到多个串口，请选择目标开发板对应的端口：");
    for (index, device) in devices.iter().enumerate() {
        println!("  {}. {}", index + 1, device.name);
    }
    loop {
        print!("输入序号 / Select: ");
        io::stdout().flush()?;
        let answer = read_line()?;
        if let Ok(index) = answer.trim().parse::<usize>()
            && (1..=devices.len()).contains(&index)
        {
            return Ok(devices[index - 1].port.clone().expect("filtered port"));
        }
        println!("请输入 1 到 {}。", devices.len());
    }
}

fn prompt_yes_no(question: &str, default_yes: bool, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        println!("{question} [auto: yes]");
        return Ok(true);
    }
    print!(
        "{question} {} ",
        if default_yes { "[Y/n]" } else { "[y/N]" }
    );
    io::stdout().flush()?;
    let answer = read_line()?;
    let answer = answer.trim().to_ascii_lowercase();
    if answer.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(answer.as_str(), "y" | "yes" | "是"))
}

fn read_line() -> Result<String> {
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line)
}

fn quote_powershell_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn install_ch340_driver(assume_yes: bool) -> Result<()> {
    if !prompt_yes_no(
        "将从 wch-ic.com 下载官方 CH341SER 驱动并弹出 Windows UAC，继续？",
        true,
        assume_yes,
    )? {
        bail!("driver installation cancelled");
    }

    let temp_path = env::temp_dir().join(format!("CH341SER-M61-{}.EXE", std::process::id()));
    println!("正在下载官方驱动 / Downloading official WCH driver...");
    let client = Client::builder()
        .user_agent(format!("M61-Flasher/{FLASHER_VERSION}"))
        .build()
        .context("failed to initialize HTTPS client")?;
    let mut response = client
        .get(WCH_DRIVER_URL)
        .send()
        .context("failed to download the WCH driver")?
        .error_for_status()
        .context("WCH driver server returned an error")?;
    let mut file =
        File::create(&temp_path).context("failed to create temporary driver installer")?;
    io::copy(&mut response, &mut file).context("failed to save WCH driver installer")?;
    file.flush()?;

    let mut downloaded = Vec::new();
    File::open(&temp_path)?.read_to_end(&mut downloaded)?;
    let downloaded_hash = sha256(&downloaded);
    println!("驱动 SHA256: {downloaded_hash}");
    if downloaded_hash != KNOWN_WCH_DRIVER_SHA256 {
        println!("提示：WCH 官方安装包已更新，将以有效数字签名作为信任依据。");
    }

    let quoted = quote_powershell_literal(&temp_path);
    let signature_script = format!(
        "$s=Get-AuthenticodeSignature -LiteralPath {quoted}; [Console]::OutputEncoding=[Text.Encoding]::UTF8; Write-Output ($s.Status.ToString() + \"`t\" + $s.SignerCertificate.Subject)"
    );
    let signature = powershell_output(&signature_script)?;
    if !signature.status.success() {
        let _ = fs::remove_file(&temp_path);
        bail!("unable to verify WCH driver Authenticode signature");
    }
    let signature_text = String::from_utf8_lossy(&signature.stdout);
    let (status, signer) = signature_text
        .trim()
        .split_once('\t')
        .ok_or_else(|| anyhow!("unexpected Authenticode result"))?;
    if status != "Valid" || !signer.contains(WCH_SIGNER_FRAGMENT) {
        let _ = fs::remove_file(&temp_path);
        bail!("driver signature rejected: status={status}, signer={signer}");
    }
    println!("数字签名有效：{signer}");
    println!("即将请求管理员权限，请在官方 WCH 安装窗口中完成安装。");

    let install_script = format!(
        "$p=Start-Process -FilePath {quoted} -Verb RunAs -Wait -PassThru; exit $p.ExitCode"
    );
    let install_status = background_command("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &install_script,
        ])
        .status()
        .context("unable to launch the WCH driver installer")?;
    let _ = fs::remove_file(&temp_path);
    if !install_status.success() {
        bail!("WCH driver installer failed or UAC was cancelled");
    }
    Ok(())
}

fn unique_runtime_path() -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX epoch")?
        .as_nanos();
    Ok(env::temp_dir().join(format!(
        "M61Flasher-{}-{}-{nonce}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    )))
}

fn write_checked(path: &Path, bytes: &[u8], expected_sha256: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    let actual = sha256(&fs::read(path)?);
    if actual != expected_sha256 {
        bail!(
            "temporary asset checksum mismatch for {}: expected {}, got {}",
            path.display(),
            expected_sha256,
            actual
        );
    }
    Ok(())
}

fn create_runtime_base() -> Result<RuntimeDirectory> {
    let path = unique_runtime_path()?;
    fs::create_dir(&path).with_context(|| format!("failed to create {}", path.display()))?;
    write_checked(
        &path.join("BLFlashCommand.exe"),
        BLFLASH_BYTES,
        BLFLASH_SHA256,
    )?;
    write_checked(
        &path.join("chips/bl616/eflash_loader/eflash_loader_cfg.ini"),
        EFLASH_LOADER_INI_BYTES,
        EFLASH_LOADER_INI_SHA256,
    )?;
    write_checked(
        &path.join("chips/bl616/eflash_loader/eflash_loader_cfg.conf"),
        EFLASH_LOADER_CONF_BYTES,
        EFLASH_LOADER_CONF_SHA256,
    )?;
    write_checked(
        &path.join("chips/bl616/efuse_bootheader/flash_para.bin"),
        FLASH_PARA_BYTES,
        FLASH_PARA_SHA256,
    )?;
    Ok(RuntimeDirectory {
        path,
        preserve: false,
    })
}

fn write_firmware_set(runtime: &RuntimeDirectory, set: &FirmwareSet) -> Result<()> {
    write_checked(
        &runtime.path.join(&set.boot2_name),
        &set.boot2,
        &sha256(&set.boot2),
    )?;
    write_checked(
        &runtime.path.join(PARTITION_NAME),
        &set.partition,
        &sha256(&set.partition),
    )?;
    write_checked(
        &runtime.path.join(&set.firmware_name),
        &set.firmware,
        &sha256(&set.firmware),
    )?;
    fs::write(
        runtime.path.join("flash_prog_cfg.ini"),
        flash_config(&set.firmware_name),
    )?;
    fs::write(
        runtime.path.join(FIRMWARE_MANIFEST_NAME),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": set.manifest.schema, "project": set.manifest.project, "version": set.manifest.version,
            "board": set.manifest.board.id(), "usb_speed": if set.manifest.usb_speed == UsbSpeed::Fs { "fs" } else { "hs" },
            "chip": set.manifest.chip, "flash_size": set.manifest.flash_size,
            "boot2": set.manifest.boot2, "partition": set.manifest.partition, "firmware": set.manifest.firmware
        }))?,
    )?;
    Ok(())
}

fn prepare_runtime(client: &Client, release: &FlashRelease) -> Result<RuntimeDirectory> {
    let runtime = create_runtime_base()?;
    let archive_path = runtime.path.join(&release.archive.name);
    download_release_asset(client, &release.archive, &archive_path)?;
    let set = read_firmware_zip(&archive_path, true)?;
    if set.manifest.board != release.board || set.manifest.usb_speed != release.usb_speed {
        bail!("release asset filename does not match firmware.json board/USB mode");
    }
    write_firmware_set(&runtime, &set)?;
    fs::remove_file(&archive_path).ok();
    Ok(runtime)
}

fn prepare_local_runtime(set: &FirmwareSet) -> Result<RuntimeDirectory> {
    let runtime = create_runtime_base()?;
    write_firmware_set(&runtime, set)?;
    Ok(runtime)
}

fn preflight_runtime_layout(runtime: &Path) -> Result<()> {
    for relative in [
        "BLFlashCommand.exe",
        "flash_prog_cfg.ini",
        "chips/bl616/eflash_loader/eflash_loader_cfg.ini",
        "chips/bl616/eflash_loader/eflash_loader_cfg.conf",
        "chips/bl616/efuse_bootheader/flash_para.bin",
        PARTITION_NAME,
    ] {
        if !runtime.join(relative).is_file() {
            bail!("runtime preflight is missing {relative}");
        }
    }
    let firmware_count = fs::read_dir(runtime)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("ds5dongle-") && name.ends_with(".bin")
        })
        .count();
    if firmware_count != 1 {
        bail!("runtime preflight expected one application firmware, found {firmware_count}");
    }
    let boot2_count = fs::read_dir(runtime)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("boot2_bl616_") && name.ends_with(".bin")
        })
        .count();
    if boot2_count != 1 {
        bail!("runtime preflight expected one boot2 file, found {boot2_count}");
    }
    Ok(())
}

fn run_tool_preflight(runtime: &Path) -> Result<()> {
    preflight_runtime_layout(runtime)?;
    let output = background_command(runtime.join("BLFlashCommand.exe"))
        .args([
            "--interface=uart",
            "--baudrate=460800",
            "--port=COM_M61_PREFLIGHT_DOES_NOT_EXIST",
            "--chipname=bl616",
            "--config=flash_prog_cfg.ini",
            "--reset",
        ])
        .current_dir(runtime)
        .stdin(Stdio::null())
        .output()
        .context("failed to start Bouffalo support-file preflight")?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if text.contains("CONFIG FILE NOT FOUND") || text.contains("Config file is not found") {
        bail!("Bouffalo support-file preflight still reports CONFIG FILE NOT FOUND");
    }
    if !text.contains("The chip type is bl616") {
        bail!("Bouffalo support-file preflight did not reach BL616 loader initialization");
    }
    Ok(())
}

fn run_flash(runtime: &Path, port: &str, baud: u32) -> Result<std::process::ExitStatus> {
    preflight_runtime_layout(runtime)?;
    println!("\n开始刷写 / Flashing {port} @ {baud} baud...");
    background_command(runtime.join("BLFlashCommand.exe"))
        .args([
            "--interface=uart",
            &format!("--baudrate={baud}"),
            &format!("--port={port}"),
            "--chipname=bl616",
            "--config=flash_prog_cfg.ini",
            "--reset",
        ])
        .current_dir(runtime)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to start embedded Bouffalo flashing tool")
}

enum GuiEvent {
    Releases(std::result::Result<Vec<FlashRelease>, String>),
    Devices(std::result::Result<Vec<Ch340Device>, String>),
    FirmwareDevices(std::result::Result<Vec<FirmwareDeviceInfo>, String>),
    Diagnostics(std::result::Result<Vec<diagnostics::DeviceDiagnostic>, String>),
    AudioTestDone(std::result::Result<(), String>),
    OtaDone {
        profile: BuildProfile,
        result: std::result::Result<(), String>,
    },
    Log(String),
    DriverDone(std::result::Result<(), String>),
    FlashDone {
        success: bool,
        message: String,
        runtime: Option<PathBuf>,
        port: String,
        baud: u32,
    },
}

struct RetryState {
    runtime: PathBuf,
    port: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppTab {
    TestCenter,
    Flasher,
    DeviceDebug,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Language {
    ZhCn,
    En,
}

impl Language {
    fn tr<'a>(self, zh_cn: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::ZhCn => zh_cn,
            Self::En => en,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::ZhCn => "简体中文",
            Self::En => "English",
        }
    }
}

fn language_override() -> Option<Language> {
    match env::var("M61_FLASHER_LANG")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "zh" | "zh-cn" | "zh_cn" => Some(Language::ZhCn),
        "en" | "en-us" | "en_us" => Some(Language::En),
        _ => None,
    }
}

#[cfg(windows)]
fn system_language() -> Language {
    if let Some(language) = language_override() {
        return language;
    }
    let mut locale = [0_u16; 85];
    let length = unsafe {
        windows_sys::Win32::Globalization::GetUserDefaultLocaleName(
            locale.as_mut_ptr(),
            locale.len() as i32,
        )
    };
    if length > 1 {
        let value = String::from_utf16_lossy(&locale[..length as usize - 1]);
        if value.to_ascii_lowercase().starts_with("zh") {
            return Language::ZhCn;
        }
    }
    Language::En
}

#[cfg(not(windows))]
fn system_language() -> Language {
    if let Some(language) = language_override() {
        return language;
    }
    if env::var("LANG")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .starts_with("zh")
    {
        Language::ZhCn
    } else {
        Language::En
    }
}

struct FlasherApp {
    tx: Sender<GuiEvent>,
    rx: Receiver<GuiEvent>,
    releases: Vec<FlashRelease>,
    devices: Vec<Ch340Device>,
    firmware_devices: Vec<FirmwareDeviceInfo>,
    runtime_diagnostics: Vec<diagnostics::DeviceDiagnostic>,
    device_test_session: Option<device_test::TestSession>,
    device_test_input: device_test::InputState,
    device_test_output: device_test::OutputState,
    device_test_status: String,
    device_test_audio_busy: bool,
    device_test_controller_tone: Option<device_test::ControllerAudioTarget>,
    device_test_connected: bool,
    device_debug_metrics: device_test::DebugMetrics,
    device_debug_duration_secs: u32,
    device_debug_stress_enabled: bool,
    device_debug_stress_rate_hz: u32,
    device_debug_started: Option<Instant>,
    device_debug_complete: bool,
    device_debug_baseline: Option<diagnostics::DiagnosticSnapshot>,
    device_debug_final: Option<diagnostics::DiagnosticSnapshot>,
    device_debug_runtime_samples: Vec<diagnostics::DiagnosticSnapshot>,
    device_debug_next_snapshot: Option<Instant>,
    device_debug_alert_snapshot_max_ms: f32,
    device_debug_last_alert_snapshot: Option<Instant>,
    guided_test: guided_test::GuidedTest,
    selected_release: usize,
    show_advanced_firmware: bool,
    firmware_mode: FirmwareMode,
    local_firmware: Option<FirmwareSet>,
    selected_port: Option<String>,
    baud: u32,
    loading_releases: bool,
    loading_devices: bool,
    loading_firmware_devices: bool,
    loading_diagnostics: bool,
    diagnostics_error: Option<String>,
    busy: Option<String>,
    status: String,
    log: String,
    show_isp_dialog: bool,
    show_driver_dialog: bool,
    show_diagnostics_window: bool,
    current_tab: AppTab,
    retry: Option<RetryState>,
    language: Language,
}

impl FlasherApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_font(&cc.egui_ctx);
        configure_visual_style(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let language = system_language();
        let mut app = Self {
            tx,
            rx,
            releases: Vec::new(),
            devices: Vec::new(),
            firmware_devices: Vec::new(),
            runtime_diagnostics: Vec::new(),
            device_test_session: None,
            device_test_input: device_test::InputState::default(),
            device_test_output: device_test::OutputState::default(),
            device_test_status: language
                .tr("尚未连接测试设备", "Test device is not connected")
                .to_owned(),
            device_test_audio_busy: false,
            device_test_controller_tone: None,
            device_test_connected: false,
            device_debug_metrics: device_test::DebugMetrics::default(),
            device_debug_duration_secs: 300,
            device_debug_stress_enabled: true,
            device_debug_stress_rate_hz: 50,
            device_debug_started: None,
            device_debug_complete: false,
            device_debug_baseline: None,
            device_debug_final: None,
            device_debug_runtime_samples: Vec::new(),
            device_debug_next_snapshot: None,
            device_debug_alert_snapshot_max_ms: 0.0,
            device_debug_last_alert_snapshot: None,
            guided_test: guided_test::GuidedTest::default(),
            selected_release: 0,
            show_advanced_firmware: false,
            firmware_mode: FirmwareMode::Online,
            local_firmware: None,
            selected_port: None,
            baud: 460_800,
            loading_releases: false,
            loading_devices: false,
            loading_firmware_devices: false,
            loading_diagnostics: false,
            diagnostics_error: None,
            busy: None,
            status: language.tr("正在初始化...", "Initializing...").to_owned(),
            log: String::new(),
            show_isp_dialog: false,
            show_driver_dialog: false,
            show_diagnostics_window: false,
            current_tab: AppTab::TestCenter,
            retry: None,
            language,
        };
        app.open_device_test();
        app.refresh_releases();
        app.refresh_devices();
        app.refresh_firmware_devices();
        app
    }

    fn append_log(&mut self, message: impl AsRef<str>) {
        if !self.log.is_empty() {
            self.log.push('\n');
        }
        self.log.push_str(message.as_ref().trim_end());
        if self.log.len() > 200_000 {
            let boundary = self.log.len() - 150_000;
            self.log.drain(..boundary);
        }
    }

    fn refresh_releases(&mut self) {
        if self.loading_releases {
            return;
        }
        self.loading_releases = true;
        self.status = self
            .language
            .tr(
                "正在读取 GitHub 固件列表...",
                "Loading firmware list from GitHub...",
            )
            .to_owned();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = github_client()
                .and_then(|client| fetch_flash_releases(&client))
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(GuiEvent::Releases(result));
        });
    }

    fn refresh_devices(&mut self) {
        if self.loading_devices {
            return;
        }
        self.loading_devices = true;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = probe_ch340_devices().map_err(|error| format!("{error:#}"));
            let _ = tx.send(GuiEvent::Devices(result));
        });
    }

    fn refresh_firmware_devices(&mut self) {
        if self.loading_firmware_devices {
            return;
        }
        self.loading_firmware_devices = true;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = probe_firmware_devices().map_err(|error| format!("{error:#}"));
            let _ = tx.send(GuiEvent::FirmwareDevices(result));
        });
    }

    fn start_diagnostics(&mut self) {
        if self.loading_diagnostics || self.busy.is_some() {
            return;
        }
        self.loading_diagnostics = true;
        self.diagnostics_error = None;
        self.show_diagnostics_window = true;
        self.status = self
            .language
            .tr(
                "正在运行串口环境检查并读取 0xFD 设备快照...",
                "Checking the serial environment and capturing the 0xFD device snapshot...",
            )
            .to_owned();
        self.refresh_devices();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                diagnostics::probe_runtime_diagnostics().map_err(|error| format!("{error:#}"));
            let _ = tx.send(GuiEvent::Diagnostics(result));
        });
    }

    fn open_device_test(&mut self) {
        self.ensure_device_session();
        self.current_tab = AppTab::TestCenter;
    }

    fn ensure_device_session(&mut self) {
        if self.device_test_session.is_none() {
            self.device_test_connected = false;
            self.device_test_session = Some(device_test::TestSession::start());
            self.device_test_status = self
                .language
                .tr(
                    "正在连接 DS5 游戏控制器接口...",
                    "Connecting to the DS5 gamepad interface...",
                )
                .to_owned();
        }
    }

    fn process_device_test_events(&mut self) {
        let events = self
            .device_test_session
            .as_ref()
            .map(|session| session.events.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in events {
            match event {
                device_test::TestEvent::Connected(name) => {
                    self.device_test_connected = true;
                    self.device_test_status = match self.language {
                        Language::ZhCn => format!("已连接：{name}；正在实时读取输入"),
                        Language::En => format!("Connected: {name}; reading live input"),
                    };
                }
                device_test::TestEvent::Input(input) => {
                    self.guided_test.observe(&input);
                    self.device_test_input = input;
                }
                device_test::TestEvent::Metrics(metrics) => {
                    self.device_debug_metrics = metrics;
                }
                device_test::TestEvent::OutputSent => {
                    self.device_test_status = self
                        .language
                        .tr("测试输出已发送", "Test output sent")
                        .to_owned();
                }
                device_test::TestEvent::ControllerToneChanged(target) => {
                    self.device_test_controller_tone = target;
                    self.device_test_status = match (self.language, target) {
                        (Language::ZhCn, Some(device_test::ControllerAudioTarget::Speaker)) => {
                            "手柄扬声器 1 kHz 测试已启动；再次点击或点击停止可结束".to_owned()
                        }
                        (Language::ZhCn, Some(device_test::ControllerAudioTarget::Headphone)) => {
                            "耳机 1 kHz 测试已启动；再次点击或点击停止可结束".to_owned()
                        }
                        (Language::En, Some(device_test::ControllerAudioTarget::Speaker)) => {
                            "Controller speaker 1 kHz test started; click again or Stop to end"
                                .to_owned()
                        }
                        (Language::En, Some(device_test::ControllerAudioTarget::Headphone)) => {
                            "Headphone 1 kHz test started; click again or Stop to end".to_owned()
                        }
                        (Language::ZhCn, None) => "手柄 1 kHz 声音测试已停止".to_owned(),
                        (Language::En, None) => "Controller 1 kHz audio test stopped".to_owned(),
                    };
                }
                device_test::TestEvent::Error(error) => {
                    self.device_test_connected = false;
                    self.device_test_controller_tone = None;
                    self.device_test_status = match self.language {
                        Language::ZhCn => format!("测试设备错误：{error}"),
                        Language::En => format!("Test device error: {error}"),
                    };
                    self.append_log(self.device_test_status.clone());
                    self.device_test_session = None;
                    self.device_debug_started = None;
                    self.device_debug_complete = false;
                    self.device_debug_next_snapshot = None;
                    self.device_debug_last_alert_snapshot = None;
                }
                device_test::TestEvent::Stopped => {
                    self.device_test_connected = false;
                    self.device_test_controller_tone = None;
                    self.device_test_status = self
                        .language
                        .tr("测试设备已断开", "Test device disconnected")
                        .to_owned();
                }
            }
        }
    }

    fn start_audio_test(&mut self, channel: device_test::AudioChannel) {
        if self.device_test_audio_busy {
            return;
        }
        self.device_test_audio_busy = true;
        self.device_test_status = self
            .language
            .tr("正在播放 2 秒测试音...", "Playing a 2-second test tone...")
            .to_owned();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = device_test::play_test_tone(channel).map_err(|error| format!("{error:#}"));
            let _ = tx.send(GuiEvent::AudioTestDone(result));
        });
    }

    fn start_microphone_test(&mut self) {
        if self.device_test_audio_busy {
            return;
        }
        self.device_test_audio_busy = true;
        self.device_test_status = self
            .language
            .tr(
                "正在录制麦克风 5 秒，随后自动回放...",
                "Recording the microphone for 5 seconds, then playing it back...",
            )
            .to_owned();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                device_test::record_and_play_microphone(5).map_err(|error| format!("{error:#}"));
            let _ = tx.send(GuiEvent::AudioTestDone(result));
        });
    }

    fn start_debug_benchmark(&mut self) {
        self.ensure_device_session();
        let Some(session) = &self.device_test_session else {
            return;
        };
        if session.reset_metrics().is_ok() && session.set_metrics_active(true).is_ok() {
            let _ = session.set_stress_active(
                self.device_debug_stress_enabled,
                self.device_debug_stress_rate_hz,
            );
            self.device_debug_metrics = device_test::DebugMetrics::default();
            self.device_debug_started = Some(Instant::now());
            self.device_debug_complete = false;
            self.device_debug_baseline = None;
            self.device_debug_final = None;
            self.device_debug_runtime_samples.clear();
            self.device_debug_next_snapshot = Some(Instant::now() + Duration::from_secs(5));
            self.device_debug_alert_snapshot_max_ms = 0.0;
            self.device_debug_last_alert_snapshot = None;
            self.capture_debug_snapshot();
        }
    }

    fn stop_debug_benchmark(&mut self, complete: bool) {
        if let Some(session) = &self.device_test_session {
            let _ = session.set_metrics_active(false);
            let _ = session.set_stress_active(false, self.device_debug_stress_rate_hz);
        }
        self.device_debug_started = None;
        self.device_debug_next_snapshot = None;
        self.device_debug_last_alert_snapshot = None;
        self.device_debug_complete = complete && self.device_debug_metrics.sample_count > 0;
        if complete {
            self.capture_debug_snapshot();
        }
    }

    fn capture_debug_snapshot(&mut self) {
        if self.loading_diagnostics {
            return;
        }
        self.loading_diagnostics = true;
        self.diagnostics_error = None;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                diagnostics::probe_runtime_diagnostics().map_err(|error| format!("{error:#}"));
            let _ = tx.send(GuiEvent::Diagnostics(result));
        });
    }

    fn export_debug_report(&mut self) {
        let filename = format!("DS5Dongle-performance-{}.json", diagnostics::now_unix_ms());
        let Some(path) = rfd::FileDialog::new()
            .set_title(
                self.language
                    .tr("保存设备性能报告", "Save device performance report"),
            )
            .add_filter("JSON", &["json"])
            .set_file_name(&filename)
            .save_file()
        else {
            return;
        };
        let report = serde_json::json!({
            "schema": "ds5dongle-flasher-diagnostics/v2",
            "createdAtUnixMs": diagnostics::now_unix_ms(),
            "flasherVersion": FLASHER_VERSION,
            "measurementScope": "Windows HID report arrival intervals; not absolute controller-to-display latency",
            "configuredDurationSeconds": serde_json::Value::Null,
            "stressOutputsEnabled": self.device_debug_stress_enabled,
            "stressOutputRateHz": self.device_debug_stress_rate_hz,
            "stressDutyCycle": "15 seconds active / 5 seconds fully released",
            "runtimeSnapshotIntervalSeconds": 5,
            "hidMetrics": &self.device_debug_metrics,
            "runtimeBaseline": &self.device_debug_baseline,
            "runtimeSamples": &self.device_debug_runtime_samples,
            "runtimeFinal": &self.device_debug_final,
            "runtimeDiagnostics": &self.runtime_diagnostics,
            "testPhases": &self.guided_test.phases,
            "rxMetrics": {
                "windowsHid": &self.device_debug_metrics,
                "firmware": &self.device_debug_final,
            },
            "txMetrics": {
                "stressOutputReports": self.device_debug_metrics.stress_output_reports,
                "rateHz": self.device_debug_stress_rate_hz,
            },
            "audioInputMetrics": {
                "firmwareSnapshot": &self.device_debug_final,
                "method": "Windows M61 UAC capture plus user playback confirmation",
            },
            "audioOutputMetrics": {
                "method": "frozen ds.evua.cc-compatible HID 0x02/0x80 vectors",
            },
            "userConfirmations": self.guided_test.phases.iter().map(|phase| serde_json::json!({
                "phase": phase.id, "result": phase.result, "note": phase.note,
            })).collect::<Vec<_>>(),
            "result": if self.guided_test.phases.iter().any(|phase| phase.result == guided_test::PhaseResult::NotEffective) {
                "fail"
            } else if self.guided_test.active || self.guided_test.phases.iter().any(|phase| phase.result == guided_test::PhaseResult::Pending) {
                "warning"
            } else {
                "pass"
            },
            "rawTrace": &self.device_debug_runtime_samples,
        });
        match serde_json::to_vec_pretty(&report)
            .context("unable to serialize performance report")
            .and_then(|bytes| fs::write(&path, bytes).context("unable to write performance report"))
        {
            Ok(()) => {
                self.status = match self.language {
                    Language::ZhCn => format!("性能报告已保存：{}", path.display()),
                    Language::En => format!("Performance report saved: {}", path.display()),
                };
            }
            Err(error) => {
                self.status = match self.language {
                    Language::ZhCn => format!("性能报告保存失败：{error:#}"),
                    Language::En => format!("Failed to save performance report: {error:#}"),
                };
            }
        }
    }

    fn usable_ports(&self) -> Vec<String> {
        self.devices
            .iter()
            .filter(|device| is_ch340_device(device) && device.error_code == 0)
            .filter_map(|device| device.port.clone())
            .collect()
    }

    fn has_ch340(&self) -> bool {
        self.devices.iter().any(is_ch340_device)
    }

    fn selected_release(&self) -> Option<FlashRelease> {
        self.releases.get(self.selected_release).cloned()
    }

    fn latest_release_for_profile(&self, profile: BuildProfile) -> Option<FlashRelease> {
        self.releases
            .iter()
            .find(|release| {
                release.board == Board::Aim61
                    && release.usb_speed == UsbSpeed::Hs
                    && release.profile == profile
            })
            .cloned()
    }

    fn current_build_profile(&self) -> Option<BuildProfile> {
        self.firmware_devices.iter().find_map(|device| {
            if device.build_profile.eq_ignore_ascii_case("standard") {
                Some(BuildProfile::Standard)
            } else if device.build_profile.eq_ignore_ascii_case("diagnostic") {
                Some(BuildProfile::Diagnostic)
            } else {
                None
            }
        })
    }

    fn start_profile_ota(&mut self, profile: BuildProfile) {
        if self.busy.is_some() {
            return;
        }
        let Some(release) = self.latest_release_for_profile(profile) else {
            self.status = self
                .language
                .tr(
                    "未找到目标配置的在线 OTA 固件。请先刷新固件列表。",
                    "No online OTA firmware was found for the target profile. Refresh the list first.",
                )
                .to_owned();
            return;
        };

        if let Some(session) = self.device_test_session.take() {
            let _ = session.stop_all();
            session.shutdown();
        }
        self.device_test_connected = false;
        self.device_test_controller_tone = None;
        self.device_debug_started = None;
        self.busy = Some(match (self.language, profile) {
            (Language::ZhCn, BuildProfile::Standard) => "正在 OTA 恢复常用版…".to_owned(),
            (Language::ZhCn, BuildProfile::Diagnostic) => "正在 OTA 进入诊断模式…".to_owned(),
            (Language::En, BuildProfile::Standard) => "Restoring Standard with OTA…".to_owned(),
            (Language::En, BuildProfile::Diagnostic) => {
                "Entering Diagnostic mode with OTA…".to_owned()
            }
        });
        self.status = self
            .language
            .tr(
                "正在校验并传输签名 OTA 固件；请勿断开 USB 或关闭程序。",
                "Verifying and transferring the signed OTA image. Do not disconnect USB or close the app.",
            )
            .to_owned();
        self.append_log(format!(
            "OTA: {} -> {} / {}",
            release.tag,
            release.profile.label(),
            release.archive.name
        ));

        let tx = self.tx.clone();
        thread::spawn(move || {
            // Let the test-center HID worker close its handle and reset outputs.
            thread::sleep(Duration::from_millis(350));
            let temp_path = env::temp_dir().join(format!(
                "DS5Dongle-OTA-{}-{}-{}.zip",
                std::process::id(),
                diagnostics::now_unix_ms(),
                profile.label().to_ascii_lowercase()
            ));
            let result = (|| -> Result<()> {
                let client = github_client()?;
                download_release_asset(&client, &release.archive, &temp_path)?;
                ota_client::update_from_zip(&temp_path, profile, |message| {
                    let _ = tx.send(GuiEvent::Log(message));
                })?;
                Ok(())
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = fs::remove_file(&temp_path);
            if result.is_ok() {
                // Firmware waits before rebooting. Give Windows time to enumerate the new profile.
                thread::sleep(Duration::from_secs(3));
            }
            let _ = tx.send(GuiEvent::OtaDone { profile, result });
        });
    }

    fn selected_firmware(&self) -> Option<SelectedFirmware> {
        match self.firmware_mode {
            FirmwareMode::Online => self.selected_release().map(SelectedFirmware::Online),
            FirmwareMode::LocalZip | FirmwareMode::LocalDirectory => {
                self.local_firmware.clone().map(SelectedFirmware::Local)
            }
        }
    }

    fn choose_local_zip(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(
                self.language
                    .tr("选择 DS5Dongle 固件 ZIP", "Select DS5Dongle firmware ZIP"),
            )
            .add_filter("DS5Dongle firmware ZIP", &["zip"])
            .pick_file()
        else {
            return;
        };
        match read_firmware_zip(&path, false) {
            Ok(set) => {
                self.status = match self.language {
                    Language::ZhCn => format!("已读取本地固件：{}", set.label),
                    Language::En => format!("Loaded local firmware: {}", set.label),
                };
                self.append_log(match self.language {
                    Language::ZhCn => format!(
                        "本地 ZIP 校验通过：boot2={}，partition={}，firmware={}",
                        sha256(&set.boot2),
                        sha256(&set.partition),
                        sha256(&set.firmware)
                    ),
                    Language::En => format!(
                        "Local ZIP validated: boot2={}, partition={}, firmware={}",
                        sha256(&set.boot2),
                        sha256(&set.partition),
                        sha256(&set.firmware)
                    ),
                });
                self.local_firmware = Some(set);
            }
            Err(error) => {
                self.local_firmware = None;
                self.status = self
                    .language
                    .tr("本地固件 ZIP 无效。", "The local firmware ZIP is invalid.")
                    .to_owned();
                self.append_log(match self.language {
                    Language::ZhCn => format!("ZIP 错误：{error:#}"),
                    Language::En => format!("ZIP error: {error:#}"),
                });
            }
        }
    }

    fn choose_local_directory(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(self.language.tr(
                "选择 DS5Dongle 完整固件目录",
                "Select DS5Dongle firmware-set directory",
            ))
            .pick_folder()
        else {
            return;
        };
        match read_firmware_directory(&path) {
            Ok(set) => {
                self.status = match self.language {
                    Language::ZhCn => format!("已读取本地固件目录：{}", set.label),
                    Language::En => format!("Loaded local firmware directory: {}", set.label),
                };
                self.local_firmware = Some(set);
            }
            Err(error) => {
                self.local_firmware = None;
                self.status = self
                    .language
                    .tr(
                        "本地固件目录无效。",
                        "The local firmware directory is invalid.",
                    )
                    .to_owned();
                self.append_log(match self.language {
                    Language::ZhCn => format!("目录错误：{error:#}"),
                    Language::En => format!("Directory error: {error:#}"),
                });
            }
        }
    }

    fn relocalize_status(&mut self) {
        if self.busy.is_some() {
            self.busy = Some(
                self.language
                    .tr("操作进行中...", "Operation in progress...")
                    .to_owned(),
            );
            self.status = self
                .language
                .tr(
                    "请等待当前操作完成。",
                    "Wait for the current operation to finish.",
                )
                .to_owned();
        } else if self.loading_releases {
            self.status = self
                .language
                .tr(
                    "正在读取 GitHub 固件列表...",
                    "Loading firmware list from GitHub...",
                )
                .to_owned();
        } else if let Some(port) = &self.selected_port {
            self.status = match self.language {
                Language::ZhCn => format!("CH340 已就绪：{port}"),
                Language::En => format!("CH340 is ready: {port}"),
            };
        } else if self.devices.is_empty() {
            self.status = self
                .language
                .tr(
                    "未检测到 M61 CH340，请连接开发板串口 USB。",
                    "M61 CH340 was not detected. Connect the board's serial USB port.",
                )
                .to_owned();
        } else {
            self.status = self
                .language
                .tr(
                    "检测到 CH340，但驱动/COM 口不可用。",
                    "CH340 was detected, but its driver/COM port is unavailable.",
                )
                .to_owned();
        }
    }

    fn start_driver_install(&mut self) {
        if self.busy.is_some() {
            return;
        }
        let language = self.language;
        self.busy = Some(
            language
                .tr(
                    "正在下载并启动 WCH 官方驱动安装器...",
                    "Downloading and starting the official WCH driver installer...",
                )
                .to_owned(),
        );
        self.status = language
            .tr(
                "请留意 Windows UAC 和 WCH 安装窗口。",
                "Watch for the Windows UAC and WCH installer windows.",
            )
            .to_owned();
        self.append_log(language.tr(
            "开始下载 WCH 官方 CH341SER 驱动，并验证 Authenticode 签名。",
            "Downloading the official WCH CH341SER driver and verifying its Authenticode signature.",
        ));
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = install_ch340_driver(true).map_err(|error| format!("{error:#}"));
            let _ = tx.send(GuiEvent::DriverDone(result));
        });
    }

    fn start_flash(&mut self) {
        let Some(firmware_selection) = self.selected_firmware() else {
            self.status = self
                .language
                .tr("请先选择固件。", "Select firmware first.")
                .to_owned();
            return;
        };
        let Some(port) = self.selected_port.clone() else {
            self.status = self
                .language
                .tr(
                    "没有可用的 CH340 COM 口。",
                    "No usable CH340 COM port is available.",
                )
                .to_owned();
            return;
        };
        let language = self.language;
        let baud = self.baud;
        let firmware_label = match &firmware_selection {
            SelectedFirmware::Online(release) => release.tag.clone(),
            SelectedFirmware::Local(set) => set.label.clone(),
        };
        self.busy = Some(match language {
            Language::ZhCn => format!("正在准备并刷写 {}...", firmware_label),
            Language::En => format!("Preparing and flashing {}...", firmware_label),
        });
        self.status = match &firmware_selection {
            SelectedFirmware::Online(_) => language
                .tr(
                    "正在下载并校验所选固件 ZIP，请勿断开开发板。",
                    "Downloading and verifying the selected firmware ZIP. Do not disconnect the board.",
                )
                .to_owned(),
            SelectedFirmware::Local(_) => language
                .tr(
                    "正在校验本地固件并准备刷写，请勿断开开发板。",
                    "Validating local firmware and preparing to flash. Do not disconnect the board.",
                )
                .to_owned(),
        };
        self.append_log(match language {
            Language::ZhCn => format!("准备刷写 {} 到 {} @ {} baud", firmware_label, port, baud),
            Language::En => format!("Preparing {} for {} @ {} baud", firmware_label, port, baud),
        });
        let tx = self.tx.clone();
        thread::spawn(move || {
            let outcome = (|| -> Result<(std::process::ExitStatus, RuntimeDirectory)> {
                let runtime = match firmware_selection {
                    SelectedFirmware::Online(release) => {
                        let client = github_client()?;
                        let _ = tx.send(GuiEvent::Log(match language {
                            Language::ZhCn => {
                                format!("从 GitHub Release 下载并校验 {}...", release.archive.name)
                            }
                            Language::En => format!(
                                "Downloading and verifying {} from GitHub Release...",
                                release.archive.name
                            ),
                        }));
                        prepare_runtime(&client, &release)?
                    }
                    SelectedFirmware::Local(set) => {
                        let _ = tx.send(GuiEvent::Log(
                            language
                                .tr(
                                    "校验并载入本地完整固件...",
                                    "Validating and loading the local firmware set...",
                                )
                                .to_owned(),
                        ));
                        prepare_local_runtime(&set)?
                    }
                };
                let _ = tx.send(GuiEvent::Log(
                    language
                        .tr(
                            "下载校验完成，启动 Bouffalo 刷写核心。",
                            "Download verification passed. Starting the Bouffalo flashing core.",
                        )
                        .to_owned(),
                ));
                let status = run_flash_streaming(&runtime.path, &port, baud, &tx)?;
                Ok((status, runtime))
            })();

            match outcome {
                Ok((status, _runtime)) if status.success() => {
                    let _ = tx.send(GuiEvent::FlashDone {
                        success: true,
                        message: language
                            .tr(
                                "刷写成功。请松开 BOOT，并按一次 RESET/RST 正常启动。",
                                "Flashing succeeded. Release BOOT and press RESET/RST once to start normally.",
                            )
                            .to_owned(),
                        runtime: None,
                        port,
                        baud,
                    });
                }
                Ok((status, mut runtime)) => {
                    runtime.preserve = true;
                    let runtime_path = runtime.path.clone();
                    let _ = tx.send(GuiEvent::FlashDone {
                        success: false,
                        message: match language {
                            Language::ZhCn => format!(
                                "刷写失败（退出码 {:?}）。可确认 ISP 后用 115200 baud 重试。日志目录：{}",
                                status.code(),
                                runtime_path.display()
                            ),
                            Language::En => format!(
                                "Flashing failed (exit code {:?}). Confirm ISP mode, then retry at 115200 baud. Logs: {}",
                                status.code(),
                                runtime_path.display()
                            ),
                        },
                        runtime: Some(runtime_path),
                        port,
                        baud,
                    });
                }
                Err(error) => {
                    let _ = tx.send(GuiEvent::FlashDone {
                        success: false,
                        message: match language {
                            Language::ZhCn => format!("准备或刷写失败：{error:#}"),
                            Language::En => format!("Preparation or flashing failed: {error:#}"),
                        },
                        runtime: None,
                        port,
                        baud,
                    });
                }
            }
        });
    }

    fn start_retry(&mut self, retry: RetryState) {
        let language = self.language;
        self.busy = Some(
            language
                .tr("正在以 115200 baud 重试...", "Retrying at 115200 baud...")
                .to_owned(),
        );
        self.status = language
            .tr(
                "兼容速度重试中，请勿断开开发板。",
                "Compatibility-speed retry in progress. Do not disconnect the board.",
            )
            .to_owned();
        self.append_log(match language {
            Language::ZhCn => format!("使用 {} @ 115200 baud 重试", retry.port),
            Language::En => format!("Retrying {} @ 115200 baud", retry.port),
        });
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = run_flash_streaming(&retry.runtime, &retry.port, 115_200, &tx);
            match result {
                Ok(status) if status.success() => {
                    let _ = fs::remove_dir_all(&retry.runtime);
                    let _ = tx.send(GuiEvent::FlashDone {
                        success: true,
                        message: language
                            .tr(
                                "115200 baud 重试成功。请按 RESET/RST 正常启动。",
                                "The 115200-baud retry succeeded. Press RESET/RST to start normally.",
                            )
                            .to_owned(),
                        runtime: None,
                        port: retry.port,
                        baud: 115_200,
                    });
                }
                Ok(status) => {
                    let path = retry.runtime;
                    let _ = tx.send(GuiEvent::FlashDone {
                        success: false,
                        message: match language {
                            Language::ZhCn => format!(
                                "115200 baud 重试仍失败（退出码 {:?}）。日志目录：{}",
                                status.code(),
                                path.display()
                            ),
                            Language::En => format!(
                                "The 115200-baud retry failed (exit code {:?}). Logs: {}",
                                status.code(),
                                path.display()
                            ),
                        },
                        runtime: Some(path),
                        port: retry.port,
                        baud: 115_200,
                    });
                }
                Err(error) => {
                    let path = retry.runtime;
                    let _ = tx.send(GuiEvent::FlashDone {
                        success: false,
                        message: match language {
                            Language::ZhCn => {
                                format!("重试失败：{error:#}；日志目录：{}", path.display())
                            }
                            Language::En => {
                                format!("Retry failed: {error:#}; logs: {}", path.display())
                            }
                        },
                        runtime: Some(path),
                        port: retry.port,
                        baud: 115_200,
                    });
                }
            }
        });
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                GuiEvent::Releases(Ok(releases)) => {
                    self.loading_releases = false;
                    self.releases = releases;
                    self.selected_release = preferred_release_index(&self.releases).unwrap_or(0);
                    self.status = match self.language {
                        Language::ZhCn => {
                            format!("已加载 {} 个完整固件 Release。", self.releases.len())
                        }
                        Language::En => {
                            format!("Loaded {} complete firmware Releases.", self.releases.len())
                        }
                    };
                    self.append_log(self.language.tr(
                        "GitHub 固件列表已更新。仅显示带 GitHub SHA256 的完整固件 ZIP。",
                        "The GitHub firmware list was updated. Only complete firmware ZIPs with GitHub SHA256 are shown.",
                    ));
                }
                GuiEvent::Releases(Err(error)) => {
                    self.loading_releases = false;
                    self.status = self
                        .language
                        .tr("读取固件列表失败。", "Failed to load the firmware list.")
                        .to_owned();
                    self.append_log(match self.language {
                        Language::ZhCn => format!("固件列表错误：{error}"),
                        Language::En => format!("Firmware list error: {error}"),
                    });
                }
                GuiEvent::Devices(Ok(devices)) => {
                    self.loading_devices = false;
                    self.devices = devices;
                    let ports = self.usable_ports();
                    if self
                        .selected_port
                        .as_ref()
                        .is_none_or(|selected| !ports.contains(selected))
                    {
                        self.selected_port = ports.first().cloned();
                    }
                    if let Some(port) = &self.selected_port {
                        self.status = match self.language {
                            Language::ZhCn => format!("CH340 已就绪：{port}"),
                            Language::En => format!("CH340 is ready: {port}"),
                        };
                    } else if self.has_ch340() {
                        self.status = self
                            .language
                            .tr(
                                "检测到 CH340，但驱动/COM 口不可用。",
                                "CH340 was detected, but its driver/COM port is unavailable.",
                            )
                            .to_owned();
                    } else if self.devices.is_empty() {
                        self.status = self
                            .language
                            .tr(
                                "未检测到 M61 CH340，请连接开发板串口 USB。",
                                "M61 CH340 was not detected. Connect the board's serial USB port.",
                            )
                            .to_owned();
                    } else {
                        self.status = self
                            .language
                            .tr(
                                "只检测到普通串口（例如 COM1），已忽略；请连接 M61 的 CH340 USB。",
                                "Only non-target serial ports (such as COM1) were found and ignored; connect the M61 CH340 USB port.",
                            )
                            .to_owned();
                    }
                }
                GuiEvent::Devices(Err(error)) => {
                    self.loading_devices = false;
                    self.status = self
                        .language
                        .tr("CH340 检测失败。", "CH340 detection failed.")
                        .to_owned();
                    self.append_log(match self.language {
                        Language::ZhCn => format!("设备检测错误：{error}"),
                        Language::En => format!("Device detection error: {error}"),
                    });
                }
                GuiEvent::FirmwareDevices(Ok(devices)) => {
                    self.loading_firmware_devices = false;
                    self.firmware_devices = devices;
                    if self.firmware_devices.is_empty() {
                        self.append_log(self.language.tr(
                            "未检测到已正常启动且支持 0xF8 信息报告的 DS5DONGLE-AIM61。",
                            "No running DS5DONGLE-AIM61 with the 0xF8 information report was detected.",
                        ));
                    } else {
                        let summaries = self
                            .firmware_devices
                            .iter()
                            .map(|device| {
                                format!(
                                    "{} {} ({:04X}:{:04X})",
                                    device.product_name,
                                    device.firmware_version,
                                    device.vendor_id,
                                    device.product_id
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("; ");
                        self.append_log(match self.language {
                            Language::ZhCn => format!("设备固件信息：{summaries}"),
                            Language::En => format!("Device firmware information: {summaries}"),
                        });
                    }
                }
                GuiEvent::FirmwareDevices(Err(error)) => {
                    self.loading_firmware_devices = false;
                    self.firmware_devices.clear();
                    self.append_log(match self.language {
                        Language::ZhCn => format!("读取设备固件信息失败：{error}"),
                        Language::En => {
                            format!("Failed to read device firmware information: {error}")
                        }
                    });
                }
                GuiEvent::Diagnostics(Ok(reports)) => {
                    self.loading_diagnostics = false;
                    self.diagnostics_error = None;
                    self.runtime_diagnostics = reports;
                    if let Some(snapshot) = self
                        .runtime_diagnostics
                        .iter()
                        .find_map(|report| report.snapshot.clone())
                    {
                        if self.device_debug_started.is_some()
                            && self.device_debug_baseline.is_none()
                        {
                            self.device_debug_baseline = Some(snapshot);
                        } else if self.device_debug_started.is_some() {
                            self.device_debug_runtime_samples.push(snapshot);
                        } else if self.device_debug_complete {
                            self.device_debug_final = Some(snapshot);
                        }
                    }
                    let captured = self
                        .runtime_diagnostics
                        .iter()
                        .filter(|report| report.snapshot.is_some())
                        .count();
                    if captured > 0 {
                        self.status = match self.language {
                            Language::ZhCn => format!("一键诊断完成：已读取 {captured} 台设备。"),
                            Language::En => {
                                format!("Diagnostics completed for {captured} device(s).")
                            }
                        };
                        self.append_log(match self.language {
                            Language::ZhCn => format!(
                                "0xFD 运行态诊断完成：{captured} 份 CRC32 分页快照校验通过。"
                            ),
                            Language::En => format!(
                                "0xFD runtime diagnostics completed: {captured} CRC32-protected paged snapshot(s) validated."
                            ),
                        });
                    } else if self.runtime_diagnostics.is_empty() {
                        self.status = self
                            .language
                            .tr(
                                "未检测到运行中的 DS5DONGLE-AIM61。串口环境结果仍可查看。",
                                "No running DS5DONGLE-AIM61 was detected. Serial diagnostics are still available.",
                            )
                            .to_owned();
                    } else {
                        self.status = self
                            .language
                            .tr(
                                "检测到 USB 设备，但无法读取 0xFD 诊断；请查看诊断详情。",
                                "A USB device was found, but its 0xFD diagnostics could not be read. See the diagnostic details.",
                            )
                            .to_owned();
                    }
                }
                GuiEvent::Diagnostics(Err(error)) => {
                    self.loading_diagnostics = false;
                    self.runtime_diagnostics.clear();
                    self.diagnostics_error = Some(error.clone());
                    self.status = self
                        .language
                        .tr(
                            "一键诊断失败，请查看详情。",
                            "Diagnostics failed. See details.",
                        )
                        .to_owned();
                    self.append_log(match self.language {
                        Language::ZhCn => format!("一键诊断错误：{error}"),
                        Language::En => format!("Diagnostics error: {error}"),
                    });
                }
                GuiEvent::AudioTestDone(result) => {
                    self.device_test_audio_busy = false;
                    match result {
                        Ok(()) => {
                            self.device_test_status = self
                                .language
                                .tr(
                                    "音频测试已完成，请根据听感确认结果",
                                    "Audio test completed; confirm the result by listening",
                                )
                                .to_owned();
                        }
                        Err(error) => {
                            self.device_test_status = match self.language {
                                Language::ZhCn => format!("音频测试失败：{error}"),
                                Language::En => format!("Audio test failed: {error}"),
                            };
                            self.append_log(self.device_test_status.clone());
                        }
                    }
                }
                GuiEvent::OtaDone { profile, result } => {
                    self.busy = None;
                    match result {
                        Ok(()) => {
                            self.status = match (self.language, profile) {
                                (Language::ZhCn, BuildProfile::Standard) => {
                                    "常用版 OTA 已完成，设备已重新启动。".to_owned()
                                }
                                (Language::ZhCn, BuildProfile::Diagnostic) => {
                                    "诊断版 OTA 已完成，设备已重新启动。".to_owned()
                                }
                                (Language::En, BuildProfile::Standard) => {
                                    "Standard OTA completed and the device restarted.".to_owned()
                                }
                                (Language::En, BuildProfile::Diagnostic) => {
                                    "Diagnostic OTA completed and the device restarted.".to_owned()
                                }
                            };
                            self.append_log(self.status.clone());
                            self.refresh_firmware_devices();
                            self.ensure_device_session();
                        }
                        Err(error) => {
                            self.status = self
                                .language
                                .tr(
                                    "OTA 切换失败；设备原有固件未被激活槽覆盖。",
                                    "OTA profile switch failed; the active firmware slot was not replaced.",
                                )
                                .to_owned();
                            self.append_log(match self.language {
                                Language::ZhCn => format!("OTA 错误：{error}"),
                                Language::En => format!("OTA error: {error}"),
                            });
                            self.ensure_device_session();
                        }
                    }
                }
                GuiEvent::Log(line) => self.append_log(line),
                GuiEvent::DriverDone(Ok(())) => {
                    self.busy = None;
                    self.status = self
                        .language
                        .tr(
                            "驱动安装器已结束，请重新插拔开发板。",
                            "The driver installer finished. Reconnect the board.",
                        )
                        .to_owned();
                    self.append_log(self.language.tr(
                        "WCH 驱动安装器已结束；正在重新检测 CH340。",
                        "The WCH driver installer finished; detecting CH340 again.",
                    ));
                    self.refresh_devices();
                }
                GuiEvent::DriverDone(Err(error)) => {
                    self.busy = None;
                    self.status = self
                        .language
                        .tr(
                            "驱动安装失败或 UAC 被取消。",
                            "Driver installation failed or UAC was cancelled.",
                        )
                        .to_owned();
                    self.append_log(match self.language {
                        Language::ZhCn => format!("驱动安装错误：{error}"),
                        Language::En => format!("Driver installation error: {error}"),
                    });
                }
                GuiEvent::FlashDone {
                    success,
                    message,
                    runtime,
                    port,
                    baud,
                } => {
                    self.busy = None;
                    self.status = message.clone();
                    self.append_log(&message);
                    if success {
                        self.retry = None;
                    } else if baud != 115_200 {
                        self.retry = runtime.map(|runtime| RetryState { runtime, port });
                    } else {
                        self.retry = None;
                    }
                }
            }
        }
    }

    fn device_status_text(&self) -> (&'static str, eframe::egui::Color32) {
        if self.loading_devices {
            (
                self.language.tr("正在检测...", "Detecting..."),
                COLOR_WARNING,
            )
        } else if self.selected_port.is_some() {
            (self.language.tr("驱动正常", "Ready"), COLOR_SUCCESS)
        } else if self.has_ch340() {
            (
                self.language.tr("驱动异常", "Driver required"),
                eframe::egui::Color32::LIGHT_RED,
            )
        } else if self.devices.is_empty() {
            (
                self.language.tr("未连接", "Not detected"),
                eframe::egui::Color32::GRAY,
            )
        } else {
            (
                self.language
                    .tr("普通串口已忽略", "Non-target port ignored"),
                COLOR_WARNING,
            )
        }
    }

    fn firmware_device_text(&self) -> String {
        if self.loading_firmware_devices {
            return self.language.tr("正在读取...", "Reading...").to_owned();
        }
        if self.firmware_devices.is_empty() {
            return self
                .language
                .tr(
                    "未检测到运行中的项目固件",
                    "No running project firmware detected",
                )
                .to_owned();
        }
        self.firmware_devices
            .iter()
            .map(|device| {
                format!(
                    "{} · v{} · {:04X}:{:04X}",
                    device.product_name,
                    device.firmware_version,
                    device.vendor_id,
                    device.product_id
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn diagnostic_status_text(&self) -> String {
        if self.loading_diagnostics {
            return self
                .language
                .tr(
                    "正在读取 0xFD 分页快照...",
                    "Reading the paged 0xFD snapshot...",
                )
                .to_owned();
        }
        if self
            .runtime_diagnostics
            .iter()
            .any(|report| report.snapshot.is_some())
        {
            return self
                .language
                .tr("诊断快照已就绪", "Diagnostic snapshot ready")
                .to_owned();
        }
        if self.diagnostics_error.is_some() {
            return self
                .language
                .tr("诊断读取失败", "Diagnostic capture failed")
                .to_owned();
        }
        self.language.tr("尚未运行", "Not run yet").to_owned()
    }

    fn diagnostic_summary_text(&self) -> String {
        let mut lines = Vec::new();
        let ports = self.usable_ports();
        if !ports.is_empty() {
            lines.push(match self.language {
                Language::ZhCn => format!("CH340：{}（可刷写）", ports.join(", ")),
                Language::En => format!("CH340: {} (ready for flashing)", ports.join(", ")),
            });
        } else if self.has_ch340() {
            lines.push(
                self.language
                    .tr(
                        "CH340：已检测到，但驱动或 COM 口不可用",
                        "CH340: detected, but the driver or COM port is unavailable",
                    )
                    .to_owned(),
            );
        } else if self.devices.is_empty() {
            lines.push(
                self.language
                    .tr(
                        "CH340：未检测到；检查数据线、UART USB 接口和驱动",
                        "CH340: not detected; check the data cable, UART USB port, and driver",
                    )
                    .to_owned(),
            );
        } else {
            let ignored = self
                .devices
                .iter()
                .filter_map(|device| device.port.as_deref())
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(match self.language {
                Language::ZhCn => format!("串口：仅检测到 {ignored}，不是 CH340，已禁止用于刷写"),
                Language::En => format!(
                    "Serial: only {ignored} was detected; it is not CH340 and is blocked for flashing"
                ),
            });
        }

        if let Some(error) = &self.diagnostics_error {
            lines.push(match self.language {
                Language::ZhCn => format!("USB HID 诊断错误：{error}"),
                Language::En => format!("USB HID diagnostic error: {error}"),
            });
        } else if self.runtime_diagnostics.is_empty() {
            lines.push(
                self.language
                    .tr(
                        "USB HID：未检测到运行中的 DS5DONGLE-AIM61",
                        "USB HID: no running DS5DONGLE-AIM61 was detected",
                    )
                    .to_owned(),
            );
        } else {
            for report in &self.runtime_diagnostics {
                if let Some(snapshot) = &report.snapshot {
                    lines.push(match self.language {
                        Language::ZhCn => format!(
                            "USB HID：{}，固件 {}，快照 #{}，运行 {}，空闲堆 {}，RSSI {}，压力/丢失 {}，麦克风欠载 {}，OTA {}/{}",
                            report.product_name,
                            report.firmware_version.as_deref().unwrap_or("未知"),
                            snapshot.snapshot_seq,
                            format_duration(snapshot.uptime_ms),
                            format_bytes(snapshot.heap_free_bytes),
                            snapshot.bt_rssi_dbm.map_or_else(|| "—".to_owned(), |value| format!("{value} dBm")),
                            snapshot.loss_pressure,
                            snapshot.mic_underruns,
                            snapshot.ota_state,
                            snapshot.ota_error,
                        ),
                        Language::En => format!(
                            "USB HID: {}, firmware {}, snapshot #{}, uptime {}, free heap {}, RSSI {}, loss/pressure {}, mic underruns {}, OTA {}/{}",
                            report.product_name,
                            report.firmware_version.as_deref().unwrap_or("unknown"),
                            snapshot.snapshot_seq,
                            format_duration(snapshot.uptime_ms),
                            format_bytes(snapshot.heap_free_bytes),
                            snapshot.bt_rssi_dbm.map_or_else(|| "—".to_owned(), |value| format!("{value} dBm")),
                            snapshot.loss_pressure,
                            snapshot.mic_underruns,
                            snapshot.ota_state,
                            snapshot.ota_error,
                        ),
                    });
                } else {
                    lines.push(match self.language {
                        Language::ZhCn => format!(
                            "USB HID：{} 已连接，但 0xFD 不可用：{}",
                            report.product_name,
                            report.error.as_deref().unwrap_or("未知错误")
                        ),
                        Language::En => format!(
                            "USB HID: {} is connected, but 0xFD is unavailable: {}",
                            report.product_name,
                            report.error.as_deref().unwrap_or("unknown error")
                        ),
                    });
                }
            }
        }
        lines.join("\n")
    }

    fn export_diagnostics(&mut self) {
        let filename = format!("DS5Dongle-diagnostics-{}.json", diagnostics::now_unix_ms());
        let Some(path) = rfd::FileDialog::new()
            .set_title(
                self.language
                    .tr("保存 DS5Dongle 诊断包", "Save DS5Dongle diagnostic bundle"),
            )
            .add_filter("JSON", &["json"])
            .set_file_name(&filename)
            .save_file()
        else {
            return;
        };
        match diagnostic_bundle_json(&self.devices, &self.runtime_diagnostics)
            .and_then(|json| fs::write(&path, json).context("unable to write diagnostic bundle"))
        {
            Ok(()) => {
                self.status = match self.language {
                    Language::ZhCn => format!("诊断包已保存：{}", path.display()),
                    Language::En => format!("Diagnostic bundle saved: {}", path.display()),
                };
                self.append_log(&self.status.clone());
            }
            Err(error) => {
                self.status = match self.language {
                    Language::ZhCn => format!("保存诊断包失败：{error:#}"),
                    Language::En => format!("Failed to save diagnostic bundle: {error:#}"),
                };
                self.append_log(&self.status.clone());
            }
        }
    }
}

impl Drop for FlasherApp {
    fn drop(&mut self) {
        if let Some(session) = self.device_test_session.take() {
            let _ = session.stop_all();
            // Give the HID worker one scheduling slice to deliver the neutral
            // frame before the process tears down its Windows handles.
            thread::sleep(Duration::from_millis(35));
            session.shutdown();
        }
    }
}

fn format_bytes(bytes: u32) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}

fn format_duration(milliseconds: u32) -> String {
    let seconds = milliseconds / 1000;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn dpad_name(value: u8, language: Language) -> &'static str {
    match value {
        0 => language.tr("上", "Up"),
        1 => language.tr("右上", "Up-right"),
        2 => language.tr("右", "Right"),
        3 => language.tr("右下", "Down-right"),
        4 => language.tr("下", "Down"),
        5 => language.tr("左下", "Down-left"),
        6 => language.tr("左", "Left"),
        7 => language.tr("左上", "Up-left"),
        _ => language.tr("未按", "Released"),
    }
}

fn input_progress(ui: &mut eframe::egui::Ui, label: &str, value: u8) {
    ui.label(label);
    ui.add(
        eframe::egui::ProgressBar::new(value as f32 / 255.0)
            .desired_width(150.0)
            .text(value.to_string()),
    );
}

fn input_button(ui: &mut eframe::egui::Ui, label: &str, pressed: bool) {
    let (fill, stroke, color) = if pressed {
        (COLOR_SUCCESS_SOFT, COLOR_SUCCESS, COLOR_TEXT_PRIMARY)
    } else {
        (COLOR_SURFACE_RAISED, COLOR_BORDER, COLOR_TEXT_MUTED)
    };
    eframe::egui::Frame::new()
        .fill(fill)
        .stroke(eframe::egui::Stroke::new(1.0_f32, stroke))
        .corner_radius(7.0)
        .inner_margin(eframe::egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(eframe::egui::RichText::new(label).color(color).strong());
        });
}

const COLOR_APP_BG: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(244, 247, 251);
const COLOR_HEADER: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(255, 255, 255);
const COLOR_SURFACE: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(255, 255, 255);
const COLOR_SURFACE_RAISED: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(236, 242, 249);
const COLOR_BORDER: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(198, 210, 224);
const COLOR_TEXT_PRIMARY: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(19, 35, 57);
const COLOR_TEXT_MUTED: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(73, 91, 113);
const COLOR_ACCENT: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(0, 103, 184);
const COLOR_ACCENT_HOVER: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(0, 82, 148);
const COLOR_ACCENT_SOFT: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(222, 239, 253);
const COLOR_SUCCESS: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(20, 112, 68);
const COLOR_SUCCESS_SOFT: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(226, 246, 235);
const COLOR_WARNING: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(148, 91, 0);
const COLOR_WARNING_SOFT: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(255, 244, 214);
const COLOR_ERROR: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(178, 32, 47);
const COLOR_ERROR_SOFT: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(253, 230, 233);

#[derive(Clone, Copy)]
enum NoticeTone {
    Info,
    Success,
    Warning,
    Error,
}

fn configure_visual_style(ctx: &eframe::egui::Context) {
    let mut visuals = eframe::egui::Visuals::light();
    visuals.override_text_color = Some(COLOR_TEXT_PRIMARY);
    visuals.panel_fill = COLOR_APP_BG;
    visuals.window_fill = COLOR_SURFACE;
    visuals.extreme_bg_color = eframe::egui::Color32::WHITE;
    visuals.faint_bg_color = COLOR_SURFACE_RAISED;
    visuals.warn_fg_color = COLOR_WARNING;
    visuals.error_fg_color = COLOR_ERROR;
    visuals.hyperlink_color = COLOR_ACCENT_HOVER;
    visuals.selection.bg_fill = COLOR_ACCENT_SOFT;
    visuals.selection.stroke = eframe::egui::Stroke::new(1.5_f32, COLOR_ACCENT_HOVER);
    visuals.widgets.noninteractive.bg_fill = COLOR_SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = COLOR_SURFACE;
    visuals.widgets.noninteractive.bg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER);
    visuals.widgets.inactive.bg_fill = COLOR_SURFACE_RAISED;
    visuals.widgets.inactive.weak_bg_fill = COLOR_SURFACE_RAISED;
    visuals.widgets.inactive.bg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER);
    visuals.widgets.inactive.fg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_TEXT_PRIMARY);
    visuals.widgets.hovered.bg_fill = eframe::egui::Color32::from_rgb(225, 237, 250);
    visuals.widgets.hovered.weak_bg_fill = eframe::egui::Color32::from_rgb(225, 237, 250);
    visuals.widgets.hovered.bg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_ACCENT_HOVER);
    visuals.widgets.active.bg_fill = COLOR_ACCENT_SOFT;
    visuals.widgets.active.weak_bg_fill = COLOR_ACCENT_SOFT;
    visuals.widgets.active.bg_stroke = eframe::egui::Stroke::new(1.5_f32, COLOR_ACCENT);
    visuals.window_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = [10.0, 9.0].into();
        style.spacing.button_padding = [12.0, 7.0].into();
        style.spacing.interact_size.y = 34.0;
    });
}

fn surface_frame() -> eframe::egui::Frame {
    eframe::egui::Frame::new()
        .fill(COLOR_SURFACE)
        .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER))
        .corner_radius(12.0)
        .inner_margin(16)
}

fn notice(ui: &mut eframe::egui::Ui, tone: NoticeTone, title: &str, body: &str) {
    let (icon, color, fill) = match tone {
        NoticeTone::Info => ("●", COLOR_ACCENT_HOVER, COLOR_ACCENT_SOFT),
        NoticeTone::Success => ("✓", COLOR_SUCCESS, COLOR_SUCCESS_SOFT),
        NoticeTone::Warning => ("!", COLOR_WARNING, COLOR_WARNING_SOFT),
        NoticeTone::Error => ("×", COLOR_ERROR, COLOR_ERROR_SOFT),
    };
    eframe::egui::Frame::new()
        .fill(fill)
        .stroke(eframe::egui::Stroke::new(1.0_f32, color))
        .corner_radius(9.0)
        .inner_margin(eframe::egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    eframe::egui::RichText::new(icon)
                        .color(color)
                        .size(18.0)
                        .strong(),
                );
                ui.vertical(|ui| {
                    ui.label(eframe::egui::RichText::new(title).color(color).strong());
                    if !body.is_empty() {
                        ui.label(eframe::egui::RichText::new(body).color(COLOR_TEXT_PRIMARY));
                    }
                });
            });
        });
}

fn primary_button(text: &str) -> eframe::egui::Button<'_> {
    eframe::egui::Button::new(
        eframe::egui::RichText::new(text)
            .color(eframe::egui::Color32::WHITE)
            .strong(),
    )
    .fill(COLOR_ACCENT_SOFT)
    .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_ACCENT))
}

fn metric_card(
    ui: &mut eframe::egui::Ui,
    label: &str,
    value: impl Into<String>,
    detail: &str,
    color: eframe::egui::Color32,
) {
    eframe::egui::Frame::new()
        .fill(COLOR_SURFACE_RAISED)
        .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER))
        .corner_radius(9.0)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_min_width(185.0);
            ui.label(eframe::egui::RichText::new(label).color(COLOR_TEXT_MUTED));
            ui.label(
                eframe::egui::RichText::new(value.into())
                    .color(color)
                    .size(22.0)
                    .strong()
                    .monospace(),
            );
            ui.label(
                eframe::egui::RichText::new(detail)
                    .color(COLOR_TEXT_MUTED)
                    .small(),
            );
        });
}

fn status_tone(status: &str) -> NoticeTone {
    let lower = status.to_ascii_lowercase();
    if status.contains("失败")
        || status.contains("错误")
        || lower.contains("failed")
        || lower.contains("error")
    {
        NoticeTone::Error
    } else if status.contains("未检测")
        || status.contains("未连接")
        || status.contains("忽略")
        || status.contains("不可")
        || lower.contains("not detected")
        || lower.contains("not connected")
        || lower.contains("ignored")
        || lower.contains("unavailable")
    {
        NoticeTone::Warning
    } else if status.contains("完成")
        || status.contains("成功")
        || status.contains("已连接")
        || lower.contains("completed")
        || lower.contains("success")
        || lower.contains("connected")
    {
        NoticeTone::Success
    } else {
        NoticeTone::Info
    }
}

impl eframe::App for FlasherApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        self.process_events();
        self.process_device_test_events();
        if ctx.input(|input| input.key_pressed(eframe::egui::Key::Escape)) {
            self.device_test_output = device_test::OutputState::default();
            self.device_test_controller_tone = None;
            if let Some(session) = &self.device_test_session {
                let _ = session.stop_all();
            }
            self.device_test_status = self
                .language
                .tr(
                    "紧急停止：全部输出和扳机已复位",
                    "Emergency stop: all outputs and triggers reset",
                )
                .to_owned();
        }
        let debug_now = Instant::now();
        let new_severe_gap = self.device_debug_metrics.maximum_interval_ms >= 20.0
            && (self.device_debug_alert_snapshot_max_ms < 20.0
                || self.device_debug_metrics.maximum_interval_ms
                    >= self.device_debug_alert_snapshot_max_ms + 5.0);
        let alert_cooldown_ready = self
            .device_debug_last_alert_snapshot
            .is_none_or(|last| debug_now.duration_since(last) >= Duration::from_secs(5));
        if self.device_debug_started.is_some()
            && new_severe_gap
            && alert_cooldown_ready
            && !self.loading_diagnostics
        {
            self.device_debug_alert_snapshot_max_ms = self.device_debug_metrics.maximum_interval_ms;
            self.device_debug_last_alert_snapshot = Some(debug_now);
            self.device_debug_next_snapshot = Some(debug_now + Duration::from_secs(5));
            self.capture_debug_snapshot();
        } else if self.device_debug_started.is_some()
            && self
                .device_debug_next_snapshot
                .is_some_and(|deadline| debug_now >= deadline)
            && !self.loading_diagnostics
        {
            self.device_debug_next_snapshot = Some(debug_now + Duration::from_secs(5));
            self.capture_debug_snapshot();
        }
        let busy = self.busy.is_some();
        let language = self.language;
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Title(format!(
            "{} {FLASHER_VERSION}",
            language.tr("DS5DONGLE-AIM61 刷写器", "DS5DONGLE-AIM61 Flasher")
        )));
        if busy && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::CancelClose);
            self.status = language
                .tr(
                    "当前操作尚未结束，暂时不能关闭窗口。",
                    "The current operation is still running; the window cannot close yet.",
                )
                .to_owned();
        }

        eframe::egui::TopBottomPanel::top("header")
            .frame(
                eframe::egui::Frame::new()
                    .fill(COLOR_HEADER)
                    .inner_margin(eframe::egui::Margin::symmetric(20, 14)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            eframe::egui::RichText::new("DS5DONGLE · AIM61")
                                .size(22.0)
                                .color(COLOR_TEXT_PRIMARY)
                                .strong(),
                        );
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "手柄功能测试、运行诊断与安全固件刷写",
                                "Controller tests, runtime diagnostics and safe firmware flashing",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                    });
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            eframe::egui::ComboBox::from_id_salt("language_combo")
                                .selected_text(self.language.display_name())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.language,
                                        Language::ZhCn,
                                        Language::ZhCn.display_name(),
                                    );
                                    ui.selectable_value(
                                        &mut self.language,
                                        Language::En,
                                        Language::En.display_name(),
                                    );
                                });
                            ui.label(
                                eframe::egui::RichText::new(format!("v{FLASHER_VERSION}"))
                                    .color(COLOR_ACCENT_HOVER)
                                    .monospace(),
                            );
                        },
                    );
                });
            });

        let previous_tab = self.current_tab;
        eframe::egui::TopBottomPanel::top("main_tabs")
            .frame(
                eframe::egui::Frame::new()
                    .fill(COLOR_HEADER)
                    .inner_margin(eframe::egui::Margin::symmetric(20, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, icon, label) in [
                        (
                            AppTab::TestCenter,
                            "◉",
                            language.tr("测试中心", "Test center"),
                        ),
                        (
                            AppTab::Flasher,
                            "⇧",
                            language.tr("固件刷写", "Firmware flasher"),
                        ),
                        (
                            AppTab::DeviceDebug,
                            "⌁",
                            language.tr("设备调试", "Device debug"),
                        ),
                    ] {
                        let selected = self.current_tab == tab;
                        let button = eframe::egui::Button::new(
                            eframe::egui::RichText::new(format!("{icon}  {label}"))
                                .color(if selected {
                                    COLOR_TEXT_PRIMARY
                                } else {
                                    COLOR_TEXT_MUTED
                                })
                                .strong(),
                        )
                        .fill(if selected {
                            COLOR_ACCENT_SOFT
                        } else {
                            COLOR_HEADER
                        })
                        .stroke(eframe::egui::Stroke::new(
                            if selected { 1.5_f32 } else { 1.0_f32 },
                            if selected { COLOR_ACCENT } else { COLOR_BORDER },
                        ))
                        .min_size([160.0, 40.0].into());
                        if ui.add(button).clicked() {
                            self.current_tab = tab;
                        }
                    }
                });
            });
        if previous_tab != self.current_tab && self.current_tab == AppTab::TestCenter {
            self.open_device_test();
        } else if previous_tab != self.current_tab && self.current_tab == AppTab::DeviceDebug {
            self.ensure_device_session();
        }

        eframe::egui::TopBottomPanel::bottom("footer")
            .frame(
                eframe::egui::Frame::new()
                    .fill(COLOR_HEADER)
                    .inner_margin(eframe::egui::Margin::symmetric(20, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(eframe::egui::RichText::new("●").color(COLOR_SUCCESS));
                    ui.label(
                        eframe::egui::RichText::new(language.tr(
                            "本机运行 · 诊断数据不会上传",
                            "Runs locally · diagnostic data is never uploaded",
                        ))
                        .color(COLOR_TEXT_MUTED)
                        .small(),
                    );
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            ui.label(
                                eframe::egui::RichText::new("zhaohyperion/DS5DONGLE-AIM61")
                                    .color(COLOR_TEXT_MUTED)
                                    .small()
                                    .monospace(),
                            );
                        },
                    );
                });
            });

        if self.language != language {
            self.relocalize_status();
            ctx.request_repaint();
        }

        if self.current_tab == AppTab::Flasher {
            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame::new().fill(COLOR_APP_BG).inner_margin(20))
                .show(ctx, |ui| {
            ui.heading(language.tr("固件刷写", "Firmware flasher"));
            ui.label(
                eframe::egui::RichText::new(language.tr(
                    "选择经过验证的固件与目标串口，刷写前会再次确认 ISP 模式。",
                    "Choose verified firmware and the target port. ISP mode is confirmed before flashing.",
                ))
                .color(COLOR_TEXT_MUTED),
            );
            ui.add_space(10.0);
            surface_frame().show(ui, |ui| {
            eframe::egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([18.0, 12.0])
                .show(ui, |ui| {
                    ui.label(language.tr("固件来源", "Firmware source"));
                    let previous_firmware_mode = self.firmware_mode;
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            for mode in [
                                FirmwareMode::Online,
                                FirmwareMode::LocalZip,
                                FirmwareMode::LocalDirectory,
                            ] {
                                ui.radio_value(&mut self.firmware_mode, mode, mode.tr(language));
                            }
                        });
                    });
                    if self.firmware_mode != previous_firmware_mode {
                        self.local_firmware = None;
                    }
                    ui.end_row();

                    ui.label(language.tr("固件版本", "Firmware"));
                    ui.horizontal(|ui| match self.firmware_mode {
                        FirmwareMode::Online => {
                            if !self.show_advanced_firmware
                                && self
                                    .selected_release()
                                    .is_some_and(|release| release.profile == BuildProfile::Diagnostic)
                            {
                                self.selected_release =
                                    preferred_release_index(&self.releases).unwrap_or(0);
                            }
                            if self.loading_releases {
                                ui.spinner();
                            }
                            let selected_text = self
                                .selected_release()
                                .map(|release| {
                                    format!(
                                        "{} — {} / {} / {} — {}",
                                        release.tag,
                                        release.board.label(),
                                        release.usb_speed.label(),
                                        release.profile.label(),
                                        release.name
                                    )
                                })
                                .unwrap_or_else(|| {
                                    language.tr("暂无可用固件", "Unavailable").to_owned()
                                });
                            ui.add_enabled_ui(!busy && !self.releases.is_empty(), |ui| {
                                eframe::egui::ComboBox::from_id_salt("release_combo")
                                    .selected_text(selected_text)
                                    .width(390.0)
                                    .show_ui(ui, |ui| {
                                        let preferred = preferred_release_index(&self.releases);
                                        for (index, release) in self.releases.iter().enumerate() {
                                            if !self.show_advanced_firmware && Some(index) != preferred {
                                                continue;
                                            }
                                            let channel = if release.prerelease {
                                                language.tr(" [预发布]", " [prerelease]")
                                            } else {
                                                ""
                                            };
                                            ui.selectable_value(
                                                &mut self.selected_release,
                                                index,
                                                format!(
                                                    "{}{} — {} / {} / {} — {}",
                                                    release.tag,
                                                    channel,
                                                    release.board.label(),
                                                    release.usb_speed.label(),
                                                    release.profile.label(),
                                                    release.name
                                                ),
                                            );
                                        }
                                    });
                            });
                            if ui
                                .add_enabled(
                                    !busy,
                                    eframe::egui::Button::new(language.tr("刷新", "Refresh")),
                                )
                                .clicked()
                            {
                                self.refresh_releases();
                            }
                            ui.checkbox(
                                &mut self.show_advanced_firmware,
                                language.tr(
                                    "高级固件（显示诊断版）",
                                    "Advanced firmware (show Diagnostic)",
                                ),
                            );
                        }
                        FirmwareMode::LocalZip => {
                            if ui
                                .add_enabled(
                                    !busy,
                                    eframe::egui::Button::new(
                                        language.tr("选择固件 ZIP...", "Choose firmware ZIP..."),
                                    ),
                                )
                                .clicked()
                            {
                                self.choose_local_zip();
                            }
                            ui.label(
                                self.local_firmware
                                    .as_ref()
                                    .map(|set| set.label.as_str())
                                    .unwrap_or(language.tr("尚未选择", "Not selected")),
                            );
                        }
                        FirmwareMode::LocalDirectory => {
                            if ui
                                .add_enabled(
                                    !busy,
                                    eframe::egui::Button::new(
                                        language.tr(
                                            "选择完整固件目录...",
                                            "Choose firmware directory...",
                                        ),
                                    ),
                                )
                                .clicked()
                            {
                                self.choose_local_directory();
                            }
                            ui.label(
                                self.local_firmware
                                    .as_ref()
                                    .map(|set| set.label.as_str())
                                    .unwrap_or(language.tr("尚未选择", "Not selected")),
                            );
                        }
                    });
                    ui.end_row();

                    ui.label(language.tr("设备固件信息", "Device firmware"));
                    ui.horizontal(|ui| {
                        if self.loading_firmware_devices {
                            ui.spinner();
                        }
                        ui.label(self.firmware_device_text());
                        if ui
                            .add_enabled(
                                !busy && !self.loading_firmware_devices,
                                eframe::egui::Button::new(language.tr("重新读取", "Read again")),
                            )
                            .clicked()
                        {
                            self.refresh_firmware_devices();
                        }
                    });
                    ui.end_row();

                    ui.label(language.tr("一键诊断", "Diagnostics"));
                    ui.horizontal(|ui| {
                        if self.loading_diagnostics {
                            ui.spinner();
                        }
                        ui.label(self.diagnostic_status_text());
                        if ui
                            .add_enabled(
                                !busy && !self.loading_diagnostics,
                                eframe::egui::Button::new(
                                    language.tr("运行一键诊断", "Run diagnostics"),
                                ),
                            )
                            .clicked()
                        {
                            self.start_diagnostics();
                        }
                        if ui
                            .add_enabled(
                                !self.loading_diagnostics
                                    && (!self.runtime_diagnostics.is_empty()
                                        || !self.devices.is_empty()
                                        || self.diagnostics_error.is_some()),
                                eframe::egui::Button::new(language.tr("查看详情", "Details")),
                            )
                            .clicked()
                        {
                            self.show_diagnostics_window = true;
                        }
                    });
                    ui.end_row();

                    ui.label(language.tr("串口设备状态", "Serial device status"));
                    ui.horizontal(|ui| {
                        let (text, color) = self.device_status_text();
                        ui.colored_label(color, text);
                        if ui
                            .add_enabled(
                                !busy,
                                eframe::egui::Button::new(language.tr("重新检测", "Detect again")),
                            )
                            .clicked()
                        {
                            self.refresh_devices();
                        }
                        if ui
                            .add_enabled(
                                !busy && self.has_ch340(),
                                eframe::egui::Button::new(
                                    language.tr("安装/修复驱动", "Install/repair driver"),
                                ),
                            )
                            .clicked()
                        {
                            self.show_driver_dialog = true;
                        }
                    });
                    ui.end_row();

                    ui.label(language.tr("串口", "COM port"));
                    let ports = self.usable_ports();
                    ui.add_enabled_ui(!busy && !ports.is_empty(), |ui| {
                        eframe::egui::ComboBox::from_id_salt("port_combo")
                            .selected_text(
                                self.selected_port
                                    .as_deref()
                                    .unwrap_or(language.tr("没有可用串口", "No usable port")),
                            )
                            .show_ui(ui, |ui| {
                                for port in ports {
                                    ui.selectable_value(
                                        &mut self.selected_port,
                                        Some(port.clone()),
                                        port,
                                    );
                                }
                            });
                    });
                    ui.end_row();

                    ui.label(language.tr("刷写速度", "Baud rate"));
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.radio_value(
                                &mut self.baud,
                                460_800,
                                language.tr("460800（推荐/快速）", "460800 (recommended/fast)"),
                            );
                            ui.radio_value(
                                &mut self.baud,
                                115_200,
                                language.tr("115200（兼容）", "115200 (compatibility)"),
                            );
                        });
                    });
                    ui.end_row();
                });
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let can_flash =
                    !busy && self.selected_firmware().is_some() && self.selected_port.is_some();
                if ui
                    .add_enabled(
                        can_flash,
                        eframe::egui::Button::new(
                            eframe::egui::RichText::new(
                                language.tr("下载并刷写", "Download & Flash"),
                            )
                            .strong()
                            .color(eframe::egui::Color32::WHITE),
                        )
                        .fill(COLOR_ACCENT_SOFT)
                        .stroke(eframe::egui::Stroke::new(1.5_f32, COLOR_ACCENT))
                        .min_size([260.0, 42.0].into()),
                    )
                    .clicked()
                {
                    self.show_isp_dialog = true;
                }
                if let Some(operation) = &self.busy {
                    ui.spinner();
                    ui.label(operation);
                }
            });

            ui.add_space(10.0);
            notice(
                ui,
                status_tone(&self.status),
                language.tr("当前状态", "Current status"),
                &self.status,
            );
            ui.add_space(12.0);
            surface_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(language.tr("操作日志", "Activity log"));
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            if ui.button(language.tr("复制全部", "Copy all")).clicked() {
                                ui.ctx().copy_text(self.log.clone());
                            }
                        },
                    );
                });
                ui.separator();
                eframe::egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(160.0)
                    .show(ui, |ui| {
                        ui.add(
                            eframe::egui::Label::new(
                                eframe::egui::RichText::new(&self.log)
                                    .color(COLOR_TEXT_MUTED)
                                    .monospace(),
                            )
                            .selectable(true)
                            .wrap(),
                        );
                    });
            });
            });
        }

        if self.current_tab == AppTab::TestCenter {
            ctx.request_repaint_after(Duration::from_millis(16));
            let mut apply_output = false;
            let mut stop_output = false;
            let mut reconnect = false;
            let mut tone = None;
            let mut mic_test = false;
            let mut start_controller_tone = None;
            let mut stop_controller_tone = false;
            let input = self.device_test_input.clone();
            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame::new().fill(COLOR_APP_BG).inner_margin(20))
                .show(ctx, |ui| {
                ui.heading(language.tr(
                    "DS5 手柄功能测试中心",
                    "DS5 controller test center",
                ));
                ui.label(
                    eframe::egui::RichText::new(language.tr(
                        "实时验证 DualSense 的输入、输出、传感器和 USB 音频链路。",
                        "Validate DualSense input, output, sensors and USB audio in real time.",
                    ))
                    .color(COLOR_TEXT_MUTED),
                );
                ui.add_space(8.0);
                let connected = self.device_test_connected;
                let worker_active = self.device_test_session.is_some();
                notice(
                    ui,
                    if connected {
                        NoticeTone::Success
                    } else if worker_active {
                        NoticeTone::Info
                    } else {
                        status_tone(&self.device_test_status)
                    },
                    if connected {
                        language.tr("设备已连接", "Device connected")
                    } else if worker_active {
                        language.tr("正在连接设备", "Connecting device")
                    } else {
                        language.tr("设备未连接", "Device not connected")
                    },
                    &self.device_test_status,
                );
                if !worker_active
                    && ui
                        .add(primary_button(language.tr("重新连接", "Reconnect")))
                        .clicked()
                {
                    reconnect = true;
                }
                ui.add_space(8.0);
                notice(
                    ui,
                    NoticeTone::Warning,
                    language.tr("避免设备冲突", "Avoid device conflicts"),
                    language.tr(
                        "请关闭 Steam、DS4Windows 和浏览器手柄测试页。关闭程序时会自动复位所有输出。",
                        "Close Steam, DS4Windows and other controller tools. All outputs reset automatically when the app closes.",
                    ),
                );
                ui.add_space(12.0);

                eframe::egui::ScrollArea::vertical().show(ui, |ui| {
                    surface_frame().show(ui, |ui| {
                    ui.heading(language.tr("实时输入", "Live input"));
                    ui.horizontal_wrapped(|ui| {
                        input_button(ui, "□ Square", input.square);
                        input_button(ui, "× Cross", input.cross);
                        input_button(ui, "○ Circle", input.circle);
                        input_button(ui, "△ Triangle", input.triangle);
                        input_button(ui, "L1", input.l1);
                        input_button(ui, "R1", input.r1);
                        input_button(ui, "L2", input.l2_button);
                        input_button(ui, "R2", input.r2_button);
                        input_button(ui, "L3", input.l3);
                        input_button(ui, "R3", input.r3);
                        input_button(ui, "Create", input.create);
                        input_button(ui, "Options", input.options);
                        input_button(ui, "PS", input.ps);
                        input_button(ui, "Touch", input.touchpad_click);
                        input_button(ui, "Mute", input.mute);
                        ui.separator();
                        ui.label(format!("D-pad: {}", dpad_name(input.dpad, language)));
                    });
                    ui.add_space(6.0);
                    eframe::egui::Grid::new("device_test_axes")
                        .num_columns(4)
                        .spacing([12.0, 5.0])
                        .show(ui, |ui| {
                            input_progress(ui, "LX", input.lx);
                            input_progress(ui, "LY", input.ly);
                            ui.end_row();
                            input_progress(ui, "RX", input.rx);
                            input_progress(ui, "RY", input.ry);
                            ui.end_row();
                            input_progress(ui, "L2", input.l2);
                            input_progress(ui, "R2", input.r2);
                            ui.end_row();
                        });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.group(|ui| {
                            ui.strong(language.tr("六轴传感器", "Motion sensors"));
                            eframe::egui::Grid::new("device_test_motion").show(ui, |ui| {
                                ui.label("Gyro X/Y/Z");
                                ui.monospace(format!(
                                    "{} / {} / {}",
                                    input.gyro_x, input.gyro_y, input.gyro_z
                                ));
                                ui.end_row();
                                ui.label("Accel X/Y/Z");
                                ui.monospace(format!(
                                    "{} / {} / {}",
                                    input.accel_x, input.accel_y, input.accel_z
                                ));
                                ui.end_row();
                            });
                        });
                        ui.group(|ui| {
                            ui.strong(language.tr("触摸板", "Touchpad"));
                            for (index, touch) in input.touch.iter().enumerate() {
                                ui.label(if touch.active {
                                    format!("#{} ID {}: {}, {}", index + 1, touch.id, touch.x, touch.y)
                                } else {
                                    format!("#{}: {}", index + 1, language.tr("未触摸", "inactive"))
                                });
                            }
                        });
                        ui.group(|ui| {
                            ui.strong(language.tr("状态", "Status"));
                            ui.label(match input.battery_percent {
                                Some(value) => format!("{}: {value}%", language.tr("电量", "Battery")),
                                None => language.tr("电量：未知", "Battery: unknown").to_owned(),
                            });
                            ui.label(format!("{}: {}", language.tr("报告数", "Reports"), input.report_count));
                            ui.label(format!(
                                "{}: {:.1} Hz",
                                language.tr("输入报告频率", "Input report rate"),
                                input.report_rate_hz
                            ));
                        });
                    });
                    });

                    ui.add_space(12.0);
                    surface_frame().show(ui, |ui| {
                    ui.heading(language.tr("输出功能", "Output functions"));
                    ui.columns(2, |columns| {
                        columns[0].group(|ui| {
                            ui.strong(language.tr("震动与灯效", "Rumble and lights"));
                            ui.add(
                                eframe::egui::Slider::new(
                                    &mut self.device_test_output.rumble_left,
                                    0..=200,
                                )
                                .text(language.tr("左侧低频", "Left / low frequency")),
                            );
                            ui.add(
                                eframe::egui::Slider::new(
                                    &mut self.device_test_output.rumble_right,
                                    0..=200,
                                )
                                .text(language.tr("右侧高频", "Right / high frequency")),
                            );
                            ui.checkbox(
                                &mut self.device_test_output.lightbar_enabled,
                                language.tr("启用灯条", "Enable lightbar"),
                            );
                            ui.horizontal(|ui| {
                                ui.label("R/G/B");
                                for value in &mut self.device_test_output.lightbar_rgb {
                                    ui.add(eframe::egui::DragValue::new(value).range(0..=255));
                                }
                            });
                            ui.add(
                                eframe::egui::Slider::new(
                                    &mut self.device_test_output.player_leds,
                                    0..=31,
                                )
                                .text(language.tr("玩家灯位图", "Player LED mask")),
                            );
                            ui.add(
                                eframe::egui::Slider::new(
                                    &mut self.device_test_output.mute_led,
                                    0..=2,
                                )
                                .text(language.tr("静音灯 0/1/2", "Mute LED 0/1/2")),
                            );
                        });
                        columns[1].group(|ui| {
                            ui.strong(language.tr("自适应扳机", "Adaptive triggers"));
                            for (id, label, value) in [
                                ("left_trigger_preset", "L2", &mut self.device_test_output.left_trigger),
                                ("right_trigger_preset", "R2", &mut self.device_test_output.right_trigger),
                            ] {
                                ui.horizontal(|ui| {
                                    ui.label(label);
                                    eframe::egui::ComboBox::from_id_salt(id)
                                        .selected_text(match *value {
                                            device_test::TriggerPreset::Off => language.tr("关闭", "Off"),
                                            device_test::TriggerPreset::Resistance => language.tr("阻力", "Resistance"),
                                            device_test::TriggerPreset::Weapon => language.tr("扳机", "Weapon"),
                                            device_test::TriggerPreset::Automatic => language.tr("自动扳机", "Automatic"),
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(value, device_test::TriggerPreset::Off, language.tr("关闭", "Off"));
                                            ui.selectable_value(value, device_test::TriggerPreset::Resistance, language.tr("阻力", "Resistance"));
                                            ui.selectable_value(value, device_test::TriggerPreset::Weapon, language.tr("扳机", "Weapon"));
                                            ui.selectable_value(value, device_test::TriggerPreset::Automatic, language.tr("自动扳机", "Automatic"));
                                        });
                                });
                            }
                            notice(
                                ui,
                                NoticeTone::Warning,
                                language.tr("扳机安全提示", "Trigger safety"),
                                language.tr(
                                    "测试预设使用受限强度；测试后请点击“全部停止并复位”。",
                                    "Presets use limited force. Click Stop all and reset after testing.",
                                ),
                            );
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.add(primary_button(language.tr("应用输出测试", "Apply output test"))).clicked() {
                            apply_output = true;
                        }
                        if ui.add(
                            eframe::egui::Button::new(
                                eframe::egui::RichText::new(language.tr(
                                    "全部停止并复位",
                                    "Stop all and reset",
                                ))
                                .color(COLOR_ERROR)
                                .strong(),
                            )
                            .fill(COLOR_ERROR_SOFT)
                            .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_ERROR)),
                        ).clicked() {
                            stop_output = true;
                        }
                    });
                    });

                    ui.add_space(12.0);
                    surface_frame().show(ui, |ui| {
                    ui.heading(language.tr("手柄声音测试", "Controller audio test"));
                    notice(
                        ui,
                        NoticeTone::Info,
                        language.tr("原生 1 kHz 测试", "Native 1 kHz test"),
                        language.tr(
                            "通过与 ds.evua.cc 相同的 DualSense Feature Report 0x80，直接测试手柄扬声器或已插入手柄的耳机；无需更改 Windows 默认音频设备。",
                            "Uses the same DualSense Feature Report 0x80 flow as ds.evua.cc to test the controller speaker or a headset connected to the controller; no Windows default-device change is required.",
                        ),
                    );
                    ui.horizontal(|ui| {
                        let speaker_active = self.device_test_controller_tone
                            == Some(device_test::ControllerAudioTarget::Speaker);
                        let headphone_active = self.device_test_controller_tone
                            == Some(device_test::ControllerAudioTarget::Headphone);
                        if ui.add_enabled(
                            connected,
                            eframe::egui::Button::new(if speaker_active {
                                language.tr("停止扬声器", "Stop speaker")
                            } else {
                                language.tr("扬声器 1 kHz", "Speaker 1 kHz")
                            }),
                        ).clicked() {
                            if speaker_active {
                                stop_controller_tone = true;
                            } else {
                                start_controller_tone = Some(device_test::ControllerAudioTarget::Speaker);
                            }
                        }
                        if ui.add_enabled(
                            connected,
                            eframe::egui::Button::new(if headphone_active {
                                language.tr("停止耳机", "Stop headphone")
                            } else {
                                language.tr("耳机 1 kHz", "Headphone 1 kHz")
                            }),
                        ).clicked() {
                            if headphone_active {
                                stop_controller_tone = true;
                            } else {
                                start_controller_tone = Some(device_test::ControllerAudioTarget::Headphone);
                            }
                        }
                        if ui.add_enabled(
                            connected && self.device_test_controller_tone.is_some(),
                            eframe::egui::Button::new(language.tr("停止 1 kHz", "Stop 1 kHz")),
                        ).clicked() {
                            stop_controller_tone = true;
                        }
                    });
                    });

                    ui.add_space(12.0);
                    surface_frame().show(ui, |ui| {
                    ui.heading(language.tr("USB 音频链路", "USB audio path"));
                    notice(
                        ui,
                        NoticeTone::Info,
                        language.tr("Windows 音频设备", "Windows audio device"),
                        language.tr(
                            "此区域单独测试 M61 的 USB 声卡链路。先把 DualSense Wireless Controller 设为 Windows 默认输出和输入；测试音为 2 秒，麦克风录制 5 秒后自动回放。",
                            "This section separately tests the M61 USB audio path. Set DualSense Wireless Controller as the Windows default output and input first; tones last 2 seconds and microphone audio records for 5 seconds before playback.",
                        ),
                    );
                    ui.horizontal(|ui| {
                        if self.device_test_audio_busy {
                            ui.spinner();
                        }
                        if ui.add_enabled(!self.device_test_audio_busy, eframe::egui::Button::new(language.tr("左声道", "Left tone"))).clicked() {
                            tone = Some(device_test::AudioChannel::Left);
                        }
                        if ui.add_enabled(!self.device_test_audio_busy, eframe::egui::Button::new(language.tr("右声道", "Right tone"))).clicked() {
                            tone = Some(device_test::AudioChannel::Right);
                        }
                        if ui.add_enabled(!self.device_test_audio_busy, eframe::egui::Button::new(language.tr("双声道", "Stereo tone"))).clicked() {
                            tone = Some(device_test::AudioChannel::Both);
                        }
                        if ui.add_enabled(!self.device_test_audio_busy, eframe::egui::Button::new(language.tr("录音 5 秒并回放", "Record 5s and play back"))).clicked() {
                            mic_test = true;
                        }
                    });
                    });
                });
            });

            if reconnect {
                self.device_test_connected = false;
                self.device_test_controller_tone = None;
                self.device_test_session = Some(device_test::TestSession::start());
            }
            if apply_output {
                if let Some(session) = &self.device_test_session {
                    if let Err(error) = session.set_output(self.device_test_output.clone()) {
                        self.device_test_status = format!("{error:#}");
                    }
                }
            }
            if stop_output {
                self.device_test_output = device_test::OutputState::default();
                if let Some(session) = &self.device_test_session {
                    let _ = session.stop_all();
                }
            }
            if let Some(target) = start_controller_tone {
                if let Some(session) = &self.device_test_session {
                    if let Err(error) = session.start_controller_tone(target) {
                        self.device_test_status = format!("{error:#}");
                    }
                }
            } else if stop_controller_tone {
                if let Some(session) = &self.device_test_session {
                    if let Err(error) = session.stop_controller_tone() {
                        self.device_test_status = format!("{error:#}");
                    }
                }
            }
            if let Some(channel) = tone {
                self.start_audio_test(channel);
            }
            if mic_test {
                self.start_microphone_test();
            }
        }

        if self.current_tab == AppTab::DeviceDebug {
            ctx.request_repaint_after(Duration::from_millis(100));
            let mut start_benchmark = false;
            let mut stop_benchmark = false;
            let mut capture_snapshot = false;
            let mut export_report = false;
            let mut guide_start = false;
            let mut guide_signal = false;
            let mut guide_retest = false;
            let mut guide_result = None;
            let mut ota_switch = None;
            let metrics = self.device_debug_metrics.clone();
            let running = self.device_debug_started.is_some();
            let extreme_duration_blocked = self.device_debug_stress_enabled
                && self.device_debug_stress_rate_hz == 50
                && self.device_debug_duration_secs > 900;
            let elapsed_secs = self
                .device_debug_started
                .map_or(metrics.elapsed_ms as f32 / 1000.0, |started| {
                    started.elapsed().as_secs_f32()
                });

            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame::new().fill(COLOR_APP_BG).inner_margin(20))
                .show(ctx, |ui| {
                    ui.heading(language.tr("设备调试与性能分析", "Device debug and performance"));
                    ui.label(
                        eframe::egui::RichText::new(language.tr(
                            "同时测量 M61 内部蓝牙输入到 USB 完成的转发延迟，以及 Windows 收到 HID 报告的频率、间隔分布与抖动。",
                            "Measure the M61 Bluetooth-input-to-USB-completion bridge latency together with Windows HID arrival rate, interval distribution and jitter.",
                        ))
                        .color(COLOR_TEXT_MUTED),
                    );
                    ui.add_space(10.0);
                    notice(
                        ui,
                        NoticeTone::Info,
                        language.tr("测量边界", "Measurement scope"),
                        language.tr(
                            "这里测得的是 USB HID 报告到达 Windows 的调度间隔，不是从按下按键到屏幕显示的绝对端到端延迟。",
                            "These values are Windows USB HID arrival intervals, not absolute button-to-display end-to-end latency.",
                        ),
                    );
                    ui.add_space(12.0);

                    surface_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading(language.tr("固件运行模式", "Firmware runtime profile"));
                            ui.with_layout(
                                eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                                |ui| {
                                    ui.monospace(
                                        self.current_build_profile()
                                            .map(BuildProfile::label)
                                            .unwrap_or(language.tr("未知", "Unknown")),
                                    );
                                },
                            );
                        });
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "常用版关闭运行诊断采样以获得最低开销；诊断版开放 0xFD 快照和 M61 内部转发延迟。两者可通过签名 OTA 来回切换。",
                                "Standard disables runtime diagnostic sampling for minimum overhead. Diagnostic enables 0xFD snapshots and internal M61 bridge latency. Signed OTA can switch both ways.",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                        ui.add_space(6.0);
                        notice(
                            ui,
                            NoticeTone::Warning,
                            language.tr("OTA 安全提示", "OTA safety"),
                            language.tr(
                                "切换期间会停止所有测试输出并暂时断开手柄接口。请保持 USB 供电，直到设备重新连接。",
                                "All test outputs stop and the gamepad interface disconnects briefly. Keep USB powered until the device reconnects.",
                            ),
                        );
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            let current = self.current_build_profile();
                            if ui
                                .add_enabled(
                                    self.busy.is_none()
                                        && current != Some(BuildProfile::Diagnostic)
                                        && self.latest_release_for_profile(BuildProfile::Diagnostic).is_some(),
                                    primary_button(language.tr(
                                        "进入诊断模式（OTA）",
                                        "Enter Diagnostic mode (OTA)",
                                    )),
                                )
                                .clicked()
                            {
                                ota_switch = Some(BuildProfile::Diagnostic);
                            }
                            if ui
                                .add_enabled(
                                    self.busy.is_none()
                                        && current == Some(BuildProfile::Diagnostic)
                                        && self.latest_release_for_profile(BuildProfile::Standard).is_some(),
                                    eframe::egui::Button::new(language.tr(
                                        "恢复常用版（OTA）",
                                        "Restore Standard (OTA)",
                                    )),
                                )
                                .clicked()
                            {
                                ota_switch = Some(BuildProfile::Standard);
                            }
                            if ui
                                .add_enabled(
                                    self.busy.is_none() && !self.loading_firmware_devices,
                                    eframe::egui::Button::new(language.tr(
                                        "重新读取模式",
                                        "Read profile again",
                                    )),
                                )
                                .clicked()
                            {
                                self.refresh_firmware_devices();
                            }
                        });
                    });

                    ui.add_space(12.0);

                    surface_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading(language.tr("引导式设备诊断", "Guided device diagnostics"));
                            ui.with_layout(
                                eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                                |ui| {
                                    ui.label(format!(
                                        "{}/{}",
                                        self.guided_test.current + 1,
                                        self.guided_test.phases.len()
                                    ));
                                },
                            );
                        });
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "不限时长；可重复操作多次，采样充足后由用户手动进入下一项。断开设备时输出会自动停止。",
                                "No time limit. Repeat actions as needed, then continue manually when samples are sufficient. Outputs stop on disconnect.",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                        if !self.guided_test.active {
                            if ui
                                .add_enabled(
                                    self.device_test_connected,
                                    primary_button(language.tr("开始引导测试", "Start guided test")),
                                )
                                .clicked()
                            {
                                guide_start = true;
                            }
                        } else {
                            let phase = self.guided_test.phase();
                            ui.separator();
                            ui.strong(language.tr(phase.title_zh, phase.title_en));
                            ui.label(language.tr(phase.instruction_zh, phase.instruction_en));
                            ui.monospace(format!("Target: {}", phase.target));
                            if !phase.samples.is_empty() {
                                ui.label(
                                    phase
                                        .samples
                                        .iter()
                                        .map(|(name, count)| format!("{name}={count}"))
                                        .collect::<Vec<_>>()
                                        .join(" · "),
                                );
                            }
                            let output_phase = matches!(
                                phase.id,
                                "leds" | "rumble" | "triggers_output" | "sound" | "microphone" | "summary"
                            );
                            if output_phase
                                && ui
                                    .add(primary_button(language.tr("生成当前测试信号", "Generate current test signal")))
                                    .clicked()
                            {
                                guide_signal = true;
                            }
                            ui.horizontal_wrapped(|ui| {
                                if ui.button(language.tr("通过并下一项", "Pass and next")).clicked() {
                                    guide_result = Some(guided_test::PhaseResult::Pass);
                                }
                                if ui.button(language.tr("未生效", "Not effective")).clicked() {
                                    guide_result = Some(guided_test::PhaseResult::NotEffective);
                                }
                                if ui.button(language.tr("跳过", "Skip")).clicked() {
                                    guide_result = Some(guided_test::PhaseResult::Skipped);
                                }
                                if ui.button(language.tr("重新测试本项", "Retest phase")).clicked() {
                                    guide_retest = true;
                                }
                            });
                            if self.guided_test.phase().result == guided_test::PhaseResult::NotEffective {
                                ui.label(language.tr("失败备注（可选）", "Failure note (optional)"));
                                ui.text_edit_singleline(&mut self.guided_test.phase_mut().note);
                            }
                        }
                    });

                    ui.add_space(12.0);

                    surface_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(language.tr("自动压力测试", "Automated stress test"));
                            ui.separator();
                            ui.label(language.tr("时长", "Duration"));
                            for seconds in [300_u32, 900, 1800, 3600] {
                                ui.add_enabled_ui(!running, |ui| {
                                    ui.radio_value(
                                        &mut self.device_debug_duration_secs,
                                        seconds,
                                        format!(
                                            "{} {}",
                                            seconds / 60,
                                            language.tr("分钟", "min")
                                        ),
                                    );
                                });
                            }
                            if !running {
                                if ui
                                    .add_enabled(
                                        self.device_test_connected && !extreme_duration_blocked,
                                        primary_button(language.tr("开始压力测试", "Start stress test")),
                                    )
                                    .clicked()
                                {
                                    start_benchmark = true;
                                }
                            } else if ui
                                .add(
                                    eframe::egui::Button::new(language.tr("提前停止", "Stop early"))
                                        .fill(COLOR_WARNING_SOFT)
                                        .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_WARNING)),
                                )
                                .clicked()
                            {
                                stop_benchmark = true;
                            }
                        });
                        ui.add_enabled_ui(!running, |ui| {
                            ui.checkbox(
                                &mut self.device_debug_stress_enabled,
                                language.tr(
                                    "启用输出负载（循环灯效、限强度震动和自适应扳机）",
                                    "Enable output load (cycling lights, limited rumble and adaptive triggers)",
                                ),
                            );
                        });
                        ui.add_enabled_ui(!running && self.device_debug_stress_enabled, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(language.tr("输出频率", "Output rate"));
                                ui.radio_value(
                                    &mut self.device_debug_stress_rate_hz,
                                    20,
                                    language.tr("20 Hz 推荐", "20 Hz recommended"),
                                );
                                ui.radio_value(
                                    &mut self.device_debug_stress_rate_hz,
                                    50,
                                    language.tr("50 Hz 极限", "50 Hz extreme"),
                                );
                            });
                        });
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "执行器按 15 秒工作、5 秒完全释放循环；每 5 秒自动采集 M61 快照，出现新的 ≥20 ms 停顿时追加采集。",
                                "Actuators cycle through 15 seconds active and 5 seconds fully released; M61 snapshots run every 5 seconds and additionally on new ≥20 ms stalls.",
                            ))
                            .color(COLOR_TEXT_MUTED)
                            .small(),
                        );
                        if self.device_debug_stress_enabled {
                            notice(
                                ui,
                                if self.device_debug_stress_rate_hz == 50 {
                                    NoticeTone::Warning
                                } else {
                                    NoticeTone::Info
                                },
                                language.tr("硬件负载提示", "Hardware load notice"),
                                language.tr(
                                    "长测会增加手柄耗电、马达温升和扳机机械负担。20 Hz 适合常规稳定性测试；50 Hz 仅建议短时极限链路测试，并保持手柄通风。",
                                    "Long runs increase battery drain, motor temperature and trigger mechanical duty. Use 20 Hz for normal stability testing; reserve 50 Hz for shorter extreme transport tests and keep the controller ventilated.",
                                ),
                            );
                            if extreme_duration_blocked {
                                notice(
                                    ui,
                                    NoticeTone::Error,
                                    language.tr(
                                        "极限模式时长过长",
                                        "Extreme mode duration is too long",
                                    ),
                                    language.tr(
                                        "由于设备没有执行器温度反馈，50 Hz 模式最多允许选择 15 分钟；30–60 分钟请使用 20 Hz。",
                                        "Because actuator temperature is unavailable, 50 Hz is limited to 15 minutes; use 20 Hz for 30–60 minute runs.",
                                    ),
                                );
                            }
                        }
                        if running {
                            let progress =
                                (elapsed_secs / self.device_debug_duration_secs as f32).clamp(0.0, 1.0);
                            ui.add(
                                eframe::egui::ProgressBar::new(progress)
                                    .animate(true)
                                    .text(format!(
                                        "{} / {}",
                                        format_duration((elapsed_secs * 1000.0) as u32),
                                        format_duration(
                                            self.device_debug_duration_secs.saturating_mul(1000)
                                        )
                                    )),
                            );
                        }
                    });

                    ui.add_space(12.0);
                    eframe::egui::ScrollArea::vertical().show(ui, |ui| {
                        let enough_samples = metrics.sample_count >= 100;
                        let average = metrics.average_interval_ms.max(0.001);
                        let latest_runtime = self.device_debug_final.as_ref().or_else(|| {
                            self.device_debug_runtime_samples.last().or_else(|| {
                                self.runtime_diagnostics
                                    .iter()
                                    .find_map(|report| report.snapshot.as_ref())
                            })
                        });
                        let (firmware_fault, firmware_warning) = self
                            .device_debug_baseline
                            .as_ref()
                            .zip(latest_runtime)
                            .map_or((false, false), |(before, after)| {
                                let loss_delta = after
                                    .loss_pressure
                                    .wrapping_sub(before.loss_pressure);
                                let mic_delta = after
                                    .mic_underruns
                                    .wrapping_sub(before.mic_underruns)
                                    + after.mic_overruns.wrapping_sub(before.mic_overruns);
                                let heap_drop = before
                                    .heap_free_bytes
                                    .saturating_sub(after.heap_free_bytes);
                                (loss_delta > 0 || mic_delta > 0, heap_drop > 8 * 1024)
                            });
                        let severe = enough_samples
                            && (metrics.report_rate_hz < 100.0
                                || metrics.maximum_interval_ms > 50.0
                                || metrics.gaps_over_10ms * 200 > metrics.sample_count
                                || firmware_fault);
                        let warning = enough_samples
                            && !severe
                            && (metrics.p99_interval_ms > average * 2.5
                                || metrics.jitter_stddev_ms > average * 0.75
                                || metrics.gaps_over_10ms > 0
                                || firmware_warning);
                        let (tone, result_title, result_body) = if !enough_samples {
                            (
                                NoticeTone::Info,
                                language.tr("等待有效样本", "Waiting for samples"),
                                language.tr(
                                    "连接设备后启动自动压力测试，建议至少运行 15 分钟。",
                                    "Connect the device and start the automated stress test; at least 15 minutes is recommended.",
                                ),
                            )
                        } else if severe {
                            (
                                NoticeTone::Error,
                                language.tr("发现明显性能异常", "Significant performance issue detected"),
                                language.tr(
                                    "报告频率过低、存在超过 50 ms 的停顿、长间隔占比过高，或固件丢失/麦克风错误计数增加。请检查后续快照、USB 和蓝牙环境。",
                                    "Report rate is too low, a stall exceeded 50 ms, long gaps are excessive, or firmware loss/microphone error counters increased. Inspect the snapshots and USB/Bluetooth conditions.",
                                ),
                            )
                        } else if warning {
                            (
                                NoticeTone::Warning,
                                language.tr("存在调度抖动", "Scheduling jitter detected"),
                                language.tr(
                                    "平均吞吐正常，但尾部间隔、抖动或空闲堆变化偏高。建议运行 30 至 60 分钟压力测试并检查周期快照。",
                                    "Average throughput is normal, but tail intervals, jitter, or free-heap change is elevated. Run a 30- to 60-minute stress test and inspect periodic snapshots.",
                                ),
                            )
                        } else {
                            (
                                NoticeTone::Success,
                                language.tr("HID 调度表现稳定", "HID scheduling is stable"),
                                language.tr(
                                    "当前样本中未发现明显的长停顿或异常抖动。",
                                    "No material long stalls or abnormal jitter were found in this sample.",
                                ),
                            )
                        };
                        notice(ui, tone, result_title, result_body);
                        ui.add_space(12.0);

                        surface_frame().show(ui, |ui| {
                            ui.strong(language.tr("HID 延迟与抖动", "HID interval and jitter"));
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                metric_card(
                                    ui,
                                    language.tr("报告频率", "Report rate"),
                                    format!("{:.1} Hz", metrics.report_rate_hz),
                                    language.tr("Windows 实测", "Measured by Windows"),
                                    COLOR_ACCENT_HOVER,
                                );
                                metric_card(
                                    ui,
                                    language.tr("平均间隔", "Average interval"),
                                    format!("{:.3} ms", metrics.average_interval_ms),
                                    language.tr("越低越快", "Lower is faster"),
                                    COLOR_TEXT_PRIMARY,
                                );
                                metric_card(
                                    ui,
                                    "P95 / P99",
                                    format!(
                                        "{:.3} / {:.3} ms",
                                        metrics.p95_interval_ms, metrics.p99_interval_ms
                                    ),
                                    language.tr("尾部调度间隔", "Tail scheduling interval"),
                                    if warning || severe { COLOR_WARNING } else { COLOR_SUCCESS },
                                );
                                metric_card(
                                    ui,
                                    language.tr("最大间隔", "Maximum interval"),
                                    format!("{:.3} ms", metrics.maximum_interval_ms),
                                    language.tr("最长一次停顿", "Longest observed stall"),
                                    if metrics.maximum_interval_ms > 20.0 {
                                        COLOR_ERROR
                                    } else {
                                        COLOR_TEXT_PRIMARY
                                    },
                                );
                                metric_card(
                                    ui,
                                    language.tr("标准差抖动", "Jitter stddev"),
                                    format!("{:.3} ms", metrics.jitter_stddev_ms),
                                    language.tr("间隔离散程度", "Interval dispersion"),
                                    if warning || severe { COLOR_WARNING } else { COLOR_SUCCESS },
                                );
                                metric_card(
                                    ui,
                                    language.tr("长间隔次数", "Long gaps"),
                                    format!(
                                        ">5 ms: {}  ·  >10 ms: {}",
                                        metrics.gaps_over_5ms, metrics.gaps_over_10ms
                                    ),
                                    format!("{} {}", metrics.sample_count, language.tr("个样本", "samples")).as_str(),
                                    if metrics.gaps_over_10ms > 0 { COLOR_WARNING } else { COLOR_SUCCESS },
                                );
                                metric_card(
                                    ui,
                                    language.tr("压力输出报告", "Stress output reports"),
                                    metrics.stress_output_reports.to_string(),
                                    format!(
                                        "{} Hz · {}",
                                        self.device_debug_stress_rate_hz,
                                        language.tr(
                                            "15 秒负载 / 5 秒释放",
                                            "15 s load / 5 s release"
                                        )
                                    )
                                    .as_str(),
                                    if self.device_debug_stress_enabled {
                                        COLOR_ACCENT_HOVER
                                    } else {
                                        COLOR_TEXT_MUTED
                                    },
                                );
                            });
                        });

                        ui.add_space(12.0);
                        surface_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(language.tr("固件运行快照", "Firmware runtime snapshot"));
                                if running {
                                    ui.label(format!(
                                        "{}: {}",
                                        language.tr("周期样本", "Periodic samples"),
                                        self.device_debug_runtime_samples.len()
                                    ));
                                }
                                if self.loading_diagnostics {
                                    ui.spinner();
                                }
                                if ui
                                    .add_enabled(
                                        !self.loading_diagnostics,
                                        eframe::egui::Button::new(language.tr(
                                            "立即采集 0xFD",
                                            "Capture 0xFD now",
                                        )),
                                    )
                                    .clicked()
                                {
                                    capture_snapshot = true;
                                }
                            });
                            if let Some(snapshot) = self
                                .runtime_diagnostics
                                .iter()
                                .find_map(|report| report.snapshot.as_ref())
                            {
                                eframe::egui::Grid::new("debug_runtime_snapshot")
                                    .num_columns(4)
                                    .spacing([20.0, 8.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.label("USB complete");
                                        ui.monospace(snapshot.usb_in_completed.to_string());
                                        ui.label("BT input");
                                        ui.monospace(snapshot.bt_input_reports.to_string());
                                        ui.end_row();
                                        ui.label("BT output");
                                        ui.monospace(snapshot.bt_output_completed.to_string());
                                        ui.label("RSSI");
                                        ui.monospace(snapshot.bt_rssi_dbm.map_or_else(
                                            || "—".to_owned(),
                                            |value| format!("{value} dBm"),
                                        ));
                                        ui.end_row();
                                        ui.label(language.tr("空闲堆", "Free heap"));
                                        ui.monospace(format_bytes(snapshot.heap_free_bytes));
                                        ui.label(language.tr("丢失压力", "Loss pressure"));
                                        ui.monospace(snapshot.loss_pressure.to_string());
                                        ui.end_row();
                                        ui.label("Mic underrun / overrun");
                                        ui.monospace(format!(
                                            "{} / {}",
                                            snapshot.mic_underruns, snapshot.mic_overruns
                                        ));
                                        ui.label("Audio pairs");
                                        ui.monospace(snapshot.bt_audio_pairs_submitted.to_string());
                                        ui.end_row();
                                    });

                                ui.add_space(10.0);
                                ui.separator();
                                ui.strong(language.tr(
                                    "M61 内部转发延迟",
                                    "M61 internal bridge latency",
                                ));
                                ui.label(
                                    eframe::egui::RichText::new(language.tr(
                                        "蓝牙 HID 输入回调进入 M61 → USB IN 提交 → USB 传输完成；统计最近一个固件诊断窗口。",
                                        "Bluetooth HID callback enters M61 → USB IN submitted → USB transfer completed; values cover the latest firmware diagnostic window.",
                                    ))
                                    .color(COLOR_TEXT_MUTED),
                                );
                                if let Some(samples) = snapshot.bridge_latency_samples {
                                    if samples == 0 {
                                        notice(
                                            ui,
                                            NoticeTone::Info,
                                            language.tr("等待手柄输入", "Waiting for controller input"),
                                            language.tr(
                                                "当前诊断窗口没有完成的转发样本。保持手柄连接并操作按键或摇杆后重新采集。",
                                                "No completed bridge samples were recorded in this window. Keep the controller connected, use a button or stick, then capture again.",
                                            ),
                                        );
                                    } else {
                                        ui.horizontal_wrapped(|ui| {
                                            metric_card(
                                                ui,
                                                language.tr("窗口样本", "Window samples"),
                                                samples.to_string(),
                                                language.tr("约 1 秒窗口", "Approximately 1-second window"),
                                                COLOR_ACCENT_HOVER,
                                            );
                                            metric_card(
                                                ui,
                                                language.tr("接收 → USB 提交", "Receive → USB submit"),
                                                format!(
                                                    "{} / {} µs",
                                                    snapshot.bridge_rx_to_submit_avg_us.unwrap_or(0),
                                                    snapshot.bridge_rx_to_submit_max_us.unwrap_or(0)
                                                ),
                                                language.tr("平均 / 最大", "Average / maximum"),
                                                COLOR_TEXT_PRIMARY,
                                            );
                                            metric_card(
                                                ui,
                                                language.tr("USB 提交 → 完成", "USB submit → complete"),
                                                format!(
                                                    "{} / {} µs",
                                                    snapshot.bridge_usb_transfer_avg_us.unwrap_or(0),
                                                    snapshot.bridge_usb_transfer_max_us.unwrap_or(0)
                                                ),
                                                language.tr("平均 / 最大", "Average / maximum"),
                                                COLOR_TEXT_PRIMARY,
                                            );
                                            metric_card(
                                                ui,
                                                language.tr("M61 内部总延迟", "Total inside M61"),
                                                format!(
                                                    "{} / {} µs",
                                                    snapshot.bridge_total_avg_us.unwrap_or(0),
                                                    snapshot.bridge_total_max_us.unwrap_or(0)
                                                ),
                                                language.tr("平均 / 最大", "Average / maximum"),
                                                COLOR_SUCCESS,
                                            );
                                            metric_card(
                                                ui,
                                                language.tr("内部尾延迟", "Internal tail latency"),
                                                format!(
                                                    "≤{} / ≤{} µs",
                                                    snapshot.bridge_total_p95_us.unwrap_or(0),
                                                    snapshot.bridge_total_p99_us.unwrap_or(0)
                                                ),
                                                "P95 / P99",
                                                COLOR_SUCCESS,
                                            );
                                        });
                                        if snapshot.bridge_timing_flags.unwrap_or(0) != 0 {
                                            notice(
                                                ui,
                                                NoticeTone::Warning,
                                                language.tr("计时值已饱和", "Timing value saturated"),
                                                language.tr(
                                                    "至少一个内部延迟超过 65535 µs；请结合最大停顿、丢失压力和 USB 状态排查。",
                                                    "At least one internal latency exceeded 65535 µs; inspect maximum stalls, loss pressure and USB state.",
                                                ),
                                            );
                                        }
                                    }
                                } else {
                                    notice(
                                        ui,
                                        NoticeTone::Warning,
                                        language.tr("当前固件不支持内部计时", "Firmware does not expose internal timing"),
                                        language.tr(
                                            "旧版六页诊断仍可读取，但需要刷入带第 7 页桥接延迟数据的新版固件才能显示该项。",
                                            "The legacy six-page snapshot remains readable, but firmware with the seventh bridge-latency page is required for this section.",
                                        ),
                                    );
                                }
                            } else if let Some(error) = &self.diagnostics_error {
                                notice(
                                    ui,
                                    NoticeTone::Error,
                                    language.tr("快照采集失败", "Snapshot capture failed"),
                                    error,
                                );
                            } else {
                                ui.label(
                                    eframe::egui::RichText::new(language.tr(
                                        "尚未采集运行快照。基准测试开始与结束时会自动采集。",
                                        "No runtime snapshot yet. Benchmarks capture one at start and finish.",
                                    ))
                                    .color(COLOR_TEXT_MUTED),
                                );
                            }

                            if let (Some(before), Some(after)) =
                                (&self.device_debug_baseline, &self.device_debug_final)
                            {
                                let duration_ms = after.monotonic_ms.wrapping_sub(before.monotonic_ms);
                                if duration_ms > 0 {
                                    let seconds = duration_ms as f64 / 1000.0;
                                    ui.separator();
                                    ui.strong(language.tr("基准区间吞吐", "Benchmark interval throughput"));
                                    ui.label(format!(
                                        "USB {:.1}/s  ·  BT input {:.1}/s  ·  BT output {:.1}/s  ·  loss Δ{}  ·  mic error Δ{}",
                                        after.usb_in_completed.wrapping_sub(before.usb_in_completed) as f64 / seconds,
                                        after.bt_input_reports.wrapping_sub(before.bt_input_reports) as f64 / seconds,
                                        after.bt_output_completed.wrapping_sub(before.bt_output_completed) as f64 / seconds,
                                        after.loss_pressure.wrapping_sub(before.loss_pressure),
                                        after.mic_underruns.wrapping_sub(before.mic_underruns)
                                            + after.mic_overruns.wrapping_sub(before.mic_overruns),
                                    ));
                                }
                            }
                        });

                        ui.add_space(12.0);
                        if ui
                            .add_enabled(
                                metrics.sample_count > 0,
                                primary_button(language.tr(
                                    "导出性能 JSON",
                                    "Export performance JSON",
                                )),
                            )
                            .clicked()
                        {
                            export_report = true;
                        }
                    });
                });

            if start_benchmark {
                self.start_debug_benchmark();
            }
            if stop_benchmark {
                self.stop_debug_benchmark(true);
            }
            if capture_snapshot {
                self.capture_debug_snapshot();
            }
            if export_report {
                self.export_debug_report();
            }
            if let Some(profile) = ota_switch {
                self.start_profile_ota(profile);
            }
            if guide_start {
                self.ensure_device_session();
                self.guided_test.start();
                self.capture_debug_snapshot();
            }
            if guide_retest {
                self.guided_test.retest();
            }
            if guide_signal {
                let phase_id = self.guided_test.phase().id;
                match phase_id {
                    "leds" => {
                        let mut output = device_test::OutputState::default();
                        output.lightbar_enabled = true;
                        output.lightbar_rgb = [0, 96, 220];
                        output.player_leds = 0x15;
                        output.mute_led = 1;
                        if let Some(session) = &self.device_test_session {
                            let _ = session.set_output(output);
                        }
                    }
                    "rumble" => {
                        let mut output = device_test::OutputState::default();
                        output.rumble_left = 89;
                        output.rumble_right = 89;
                        if let Some(session) = &self.device_test_session {
                            let _ = session.set_output(output);
                        }
                    }
                    "triggers_output" => {
                        let mut output = device_test::OutputState::default();
                        output.left_trigger = device_test::TriggerPreset::Resistance;
                        output.right_trigger = device_test::TriggerPreset::Resistance;
                        if let Some(session) = &self.device_test_session {
                            let _ = session.set_output(output);
                        }
                    }
                    "sound" => {
                        if let Some(session) = &self.device_test_session {
                            let _ = session
                                .start_controller_tone(device_test::ControllerAudioTarget::Speaker);
                        }
                    }
                    "microphone" => self.start_microphone_test(),
                    "summary" => {
                        if let Some(session) = &self.device_test_session {
                            let _ = session.stop_all();
                        }
                        self.capture_debug_snapshot();
                    }
                    _ => {}
                }
            }
            if let Some(result) = guide_result {
                if result == guided_test::PhaseResult::NotEffective {
                    self.guided_test.phase_mut().result = result;
                    self.capture_debug_snapshot();
                } else {
                    self.guided_test.mark_and_next(result);
                }
            }
        }

        if self.show_diagnostics_window {
            let summary = self.diagnostic_summary_text();
            let mut rerun = false;
            let mut export = false;
            let mut close = false;
            eframe::egui::Window::new(language.tr(
                "DS5Dongle 一键诊断",
                "DS5Dongle diagnostics",
            ))
            .collapsible(false)
            .resizable(true)
            .default_size([760.0, 560.0])
            .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(language.tr(
                    "同时检查 Windows 串口环境，并读取 0xFD/CRC32 七页运行态快照（兼容旧六页固件）。诊断数据仅保存在本机。",
                    "Checks the Windows serial environment and captures the seven-page 0xFD/CRC32 runtime snapshot, with legacy six-page compatibility. Diagnostic data stays local.",
                ));
                ui.add_space(6.0);
                if self.loading_diagnostics {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(language.tr("正在诊断...", "Running diagnostics..."));
                    });
                }
                ui.group(|ui| {
                    ui.strong(language.tr("诊断结论", "Diagnostic summary"));
                    ui.add(
                        eframe::egui::Label::new(
                            eframe::egui::RichText::new(&summary).monospace(),
                        )
                        .selectable(true)
                        .wrap(),
                    );
                });

                eframe::egui::ScrollArea::vertical()
                    .max_height(350.0)
                    .show(ui, |ui| {
                        for (index, report) in self.runtime_diagnostics.iter().enumerate() {
                            ui.add_space(8.0);
                            ui.strong(format!(
                                "{} · {:04X}:{:04X} · {}",
                                report.product_name,
                                report.vendor_id,
                                report.product_id,
                                report.firmware_version.as_deref().unwrap_or(language.tr(
                                    "固件版本未知",
                                    "firmware unknown"
                                ))
                            ));
                            if let Some(snapshot) = &report.snapshot {
                                eframe::egui::Grid::new(format!("diagnostic_metrics_{index}"))
                                    .num_columns(4)
                                    .spacing([18.0, 6.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.label(language.tr("快照", "Snapshot"));
                                        ui.monospace(format!("#{}", snapshot.snapshot_seq));
                                        ui.label(language.tr("运行时间", "Uptime"));
                                        ui.monospace(format_duration(snapshot.uptime_ms));
                                        ui.end_row();
                                        ui.label(language.tr("空闲堆", "Free heap"));
                                        ui.monospace(format!(
                                            "{} / {}",
                                            format_bytes(snapshot.heap_free_bytes),
                                            format_bytes(snapshot.heap_min_free_bytes)
                                        ));
                                        ui.label("RSSI");
                                        ui.monospace(snapshot.bt_rssi_dbm.map_or_else(
                                            || "—".to_owned(),
                                            |value| format!("{value} dBm"),
                                        ));
                                        ui.end_row();
                                        ui.label(language.tr("USB 完成", "USB complete"));
                                        ui.monospace(snapshot.usb_in_completed.to_string());
                                        ui.label(language.tr("蓝牙输入", "BT input"));
                                        ui.monospace(snapshot.bt_input_reports.to_string());
                                        ui.end_row();
                                        ui.label(language.tr("丢失/压力", "Loss/pressure"));
                                        ui.colored_label(
                                            if snapshot.loss_pressure == 0 {
                                                COLOR_SUCCESS
                                            } else {
                                                COLOR_WARNING
                                            },
                                            snapshot.loss_pressure.to_string(),
                                        );
                                        ui.label(language.tr("麦克风欠载", "Mic underrun"));
                                        ui.colored_label(
                                            if snapshot.mic_underruns == 0 {
                                                COLOR_SUCCESS
                                            } else {
                                                COLOR_WARNING
                                            },
                                            snapshot.mic_underruns.to_string(),
                                        );
                                        ui.end_row();
                                        ui.label("OTA");
                                        ui.monospace(format!(
                                            "{} / {}",
                                            snapshot.ota_state, snapshot.ota_error
                                        ));
                                        ui.label(language.tr("电量", "Battery"));
                                        ui.monospace(snapshot.battery_percent.map_or_else(
                                            || "—".to_owned(),
                                            |value| format!("{value}%"),
                                        ));
                                        ui.end_row();
                                    });
                            } else if let Some(error) = &report.error {
                                notice(
                                    ui,
                                    NoticeTone::Error,
                                    language.tr("0xFD 诊断读取失败", "0xFD diagnostic capture failed"),
                                    error,
                                );
                            }
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !busy && !self.loading_diagnostics,
                            eframe::egui::Button::new(language.tr("重新诊断", "Run again")),
                        )
                        .clicked()
                    {
                        rerun = true;
                    }
                    if ui
                        .add_enabled(
                            !self.loading_diagnostics
                                && (!self.runtime_diagnostics.is_empty()
                                    || !self.devices.is_empty()),
                            eframe::egui::Button::new(
                                language.tr("保存 JSON 诊断包", "Save JSON bundle"),
                            ),
                        )
                        .clicked()
                    {
                        export = true;
                    }
                    if ui
                        .button(language.tr("复制摘要", "Copy summary"))
                        .clicked()
                    {
                        ui.ctx().copy_text(summary.clone());
                    }
                    if ui.button(language.tr("关闭", "Close")).clicked() {
                        close = true;
                    }
                });
            });
            if rerun {
                self.start_diagnostics();
            }
            if export {
                self.export_diagnostics();
            }
            if close {
                self.show_diagnostics_window = false;
            }
        }

        if self.show_isp_dialog {
            eframe::egui::Window::new(language.tr("进入 UART ISP", "Enter download mode"))
                .collapsible(false)
                .resizable(false)
                .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(language.tr(
                        "请按以下顺序操作开发板：",
                        "Use this sequence on the board:",
                    ));
                    ui.label(language.tr("1. 按住 BOOT", "1. Hold BOOT"));
                    ui.label(
                        language.tr("2. 点按并松开 RESET/RST", "2. Press and release RESET/RST"),
                    );
                    ui.label(language.tr("3. 松开 BOOT", "3. Release BOOT"));
                    ui.add_space(8.0);
                    notice(
                        ui,
                        NoticeTone::Error,
                        language.tr("刷写期间请勿断电", "Do not interrupt flashing"),
                        language.tr(
                            "刷写开始后不要拔线，也不要按 Reset。",
                            "Do not disconnect the cable or press Reset after flashing starts.",
                        ),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(language.tr("取消", "Cancel")).clicked() {
                            self.show_isp_dialog = false;
                        }
                        if ui
                            .button(
                                language
                                    .tr("我已进入 ISP，开始刷写", "ISP is ready — start flashing"),
                            )
                            .clicked()
                        {
                            self.show_isp_dialog = false;
                            self.start_flash();
                        }
                    });
                });
        }

        if self.show_driver_dialog {
            eframe::egui::Window::new(language.tr("安装 CH340 驱动", "Install CH340 driver"))
                .collapsible(false)
                .resizable(false)
                .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(language.tr(
                        "将从 wch-ic.com 通过 HTTPS 下载官方 CH341SER 安装器。",
                        "The official CH341SER installer will be downloaded over HTTPS from wch-ic.com.",
                    ));
                    ui.label(language.tr(
                        "工具会先验证 Authenticode 签名者，再弹出 Windows UAC。",
                        "The Authenticode signer is verified before Windows UAC is requested.",
                    ));
                    ui.horizontal(|ui| {
                        if ui.button(language.tr("取消", "Cancel")).clicked() {
                            self.show_driver_dialog = false;
                        }
                        if ui
                            .button(language.tr(
                                "下载并安装官方驱动",
                                "Download and install official driver",
                            ))
                            .clicked()
                        {
                            self.show_driver_dialog = false;
                            self.start_driver_install();
                        }
                    });
                });
        }

        if self.retry.is_some() {
            eframe::egui::Window::new(language.tr("刷写失败", "Flashing failed"))
                .collapsible(false)
                .resizable(false)
                .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(language.tr(
                        "请再次确认开发板处于 BOOT+RESET 的 UART ISP 模式。",
                        "Confirm that the board is still in BOOT+RESET UART ISP mode.",
                    ));
                    ui.label(language.tr(
                        "可以用更慢但更兼容的 115200 baud 重试。失败日志会保留在临时目录。",
                        "You can retry at the slower, more compatible 115200 baud. Failure logs remain in the temporary directory.",
                    ));
                    ui.horizontal(|ui| {
                        if ui
                            .button(language.tr("关闭，不重试", "Close without retry"))
                            .clicked()
                        {
                            self.retry = None;
                        }
                        if ui
                            .button(language.tr(
                                "115200 baud 重试",
                                "Retry at 115200 baud",
                            ))
                            .clicked()
                            && let Some(retry) = self.retry.take()
                        {
                            self.start_retry(retry);
                        }
                    });
                });
        }

        if busy || self.loading_devices || self.loading_firmware_devices || self.loading_releases {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn install_cjk_font(context: &eframe::egui::Context) {
    let Some(windows_directory) = env::var_os("WINDIR").map(PathBuf::from) else {
        return;
    };
    let font_directory = windows_directory.join("Fonts");
    let candidates = [
        font_directory.join("msyh.ttc"),
        font_directory.join("msyh.ttf"),
        font_directory.join("simhei.ttf"),
    ];
    let Some(bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) else {
        return;
    };
    let mut fonts = eframe::egui::FontDefinitions::default();
    fonts.font_data.insert(
        "windows-cjk".to_owned(),
        eframe::egui::FontData::from_owned(bytes).into(),
    );
    for family in [
        eframe::egui::FontFamily::Proportional,
        eframe::egui::FontFamily::Monospace,
    ] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "windows-cjk".to_owned());
    }
    context.set_fonts(fonts);
}

fn run_flash_streaming(
    runtime: &Path,
    port: &str,
    baud: u32,
    tx: &Sender<GuiEvent>,
) -> Result<std::process::ExitStatus> {
    preflight_runtime_layout(runtime)?;
    let mut child = background_command(runtime.join("BLFlashCommand.exe"))
        .args([
            "--interface=uart",
            &format!("--baudrate={baud}"),
            &format!("--port={port}"),
            "--chipname=bl616",
            "--config=flash_prog_cfg.ini",
            "--reset",
        ])
        .current_dir(runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start embedded Bouffalo flashing tool")?;

    let stdout = child.stdout.take().context("missing flasher stdout")?;
    let stderr = child.stderr.take().context("missing flasher stderr")?;
    let stdout_tx = tx.clone();
    let stderr_tx = tx.clone();
    let stdout_thread = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|line| line.ok()) {
            let _ = stdout_tx.send(GuiEvent::Log(line));
        }
    });
    let stderr_thread = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(|line| line.ok()) {
            let _ = stderr_tx.send(GuiEvent::Log(line));
        }
    });
    let status = child.wait()?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    Ok(status)
}

fn run_gui() -> Result<()> {
    verify_embedded_tool()?;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1280.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        &format!("DS5Dongle AIM61 工具中心 {FLASHER_VERSION}"),
        options,
        Box::new(|cc| Ok(Box::new(FlasherApp::new(cc)))),
    )
    .map_err(|error| anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_release(
        tag: &str,
        board: Board,
        usb_speed: UsbSpeed,
        prerelease: bool,
    ) -> FlashRelease {
        FlashRelease {
            tag: tag.into(),
            name: tag.into(),
            url: "https://example.invalid/release".into(),
            published_at: None,
            prerelease,
            archive: GithubAsset {
                name: format!("DS5Dongle-{}-{}-{tag}.zip", board.id(), usb_speed.label()),
                browser_download_url: "https://example.invalid/firmware.zip".into(),
                size: 1024,
                digest: Some(format!("sha256:{}", "0".repeat(64))),
            },
            board,
            usb_speed,
            profile: BuildProfile::Standard,
        }
    }

    fn test_manifest() -> FirmwareManifest {
        FirmwareManifest {
            schema: 1,
            project: "DS5Dongle".into(),
            version: "test".into(),
            board: Board::Lctech616,
            usb_speed: UsbSpeed::Fs,
            profile: BuildProfile::Standard,
            chip: "bl616".into(),
            flash_size: 4,
            boot2: "boot2_bl616_test.bin".into(),
            partition: PARTITION_NAME.into(),
            firmware: "ds5dongle-lctech616.bin".into(),
        }
    }

    #[test]
    fn extracts_com_port_from_friendly_name() {
        assert_eq!(
            find_com_port("USB-SERIAL CH340 (COM5)"),
            Some("COM5".to_owned())
        );
        assert_eq!(find_com_port("CH340 driver missing"), None);
    }

    #[test]
    fn parses_pnp_probe_record() {
        let output = "VVNCLVNFUklBTCBDSDM0MCAoQ09NNSk=\tVVNCXFZJRF8xQTg2JlBJRF83NTIzXDE=\t0\tOK\n";
        let parsed = parse_probe_output(output).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "USB-SERIAL CH340 (COM5)");
        assert_eq!(parsed[0].port.as_deref(), Some("COM5"));
        assert_eq!(parsed[0].error_code, 0);
    }

    #[test]
    fn rejects_non_ch340_ports_from_flash_candidates() {
        let built_in = Ch340Device {
            name: "Communications Port (COM1)".into(),
            instance_id: "ACPI\\PNP0501\\0".into(),
            error_code: 0,
            status: "OK".into(),
            port: Some("COM1".into()),
        };
        let ch340 = Ch340Device {
            name: "USB-SERIAL CH340 (COM8)".into(),
            instance_id: "USB\\VID_1A86&PID_7523\\ABC".into(),
            error_code: 0,
            status: "OK".into(),
            port: Some("COM8".into()),
        };
        assert!(!is_ch340_device(&built_in));
        assert!(is_ch340_device(&ch340));
    }

    #[test]
    fn diagnostic_bundle_marks_ignored_ports_without_exporting_pnp_identifiers() {
        let built_in = Ch340Device {
            name: "Communications Port (COM1)".into(),
            instance_id: "ACPI\\PNP0501\\SENSITIVE".into(),
            error_code: 0,
            status: "OK".into(),
            port: Some("COM1".into()),
        };
        let json = diagnostic_bundle_json(&[built_in], &[]).unwrap();
        assert!(json.contains("\"targetCh340\": false"));
        assert!(json.contains("\"usable\": false"));
        assert!(!json.contains("PNP0501"));
        assert!(!json.contains("SENSITIVE"));
    }

    #[test]
    fn validates_port_names() {
        assert_eq!(normalize_port("com12").unwrap(), "COM12");
        assert!(normalize_port("COM").is_err());
        assert!(normalize_port("ttyUSB0").is_err());
    }

    #[test]
    fn decodes_only_project_semver_firmware_reports() {
        let mut report = [0_u8; 64];
        report[0] = FIRMWARE_VERSION_REPORT_ID;
        report[1..6].copy_from_slice(b"3.5.1");
        assert_eq!(
            decode_firmware_version_report(&report),
            Some("3.5.1".to_owned())
        );
        assert_eq!(
            decode_firmware_version_report(b"254.5.0\0\0"),
            Some("254.5.0".into())
        );
        assert_eq!(decode_firmware_version_report(b"255.0.0"), None);
        assert_eq!(decode_firmware_version_report(b"DualSense"), None);
    }

    #[test]
    fn parses_release_asset_board_and_speed() {
        assert_eq!(
            asset_variant("DS5Dongle-lctech616-fs-v3.15.zip"),
            Some((Board::Lctech616, UsbSpeed::Fs, BuildProfile::Standard))
        );
        assert_eq!(
            asset_variant("DS5Dongle-aim61-hs-v3.15.zip"),
            Some((Board::Aim61, UsbSpeed::Hs, BuildProfile::Standard))
        );
        assert_eq!(asset_variant("firmware.zip"), None);
    }

    #[test]
    fn defaults_to_aim61_high_speed_standard_independent_of_asset_order() {
        let releases = vec![
            test_release("v4", Board::Lctech616, UsbSpeed::Fs, false),
            test_release("v4", Board::Aim61, UsbSpeed::Hs, false),
            test_release("v5-rc", Board::Aim61, UsbSpeed::Fs, true),
            test_release("v4", Board::Aim61, UsbSpeed::Fs, false),
            test_release("v4", Board::M0sdock, UsbSpeed::Fs, false),
        ];

        assert_eq!(preferred_release_index(&releases), Some(1));
        let automatic = choose_release(&releases, None, true).unwrap();
        assert_eq!(automatic.board, Board::Aim61);
        assert_eq!(automatic.usb_speed, UsbSpeed::Hs);

        // A tag shared by every board must use the same safe preference.
        let tagged = choose_release(&releases, Some("v4"), true).unwrap();
        assert_eq!(tagged.board, Board::Aim61);
        assert_eq!(tagged.usb_speed, UsbSpeed::Hs);
    }

    #[test]
    fn release_preference_has_deterministic_safe_fallbacks() {
        let stable = vec![
            test_release("v3", Board::Lctech616, UsbSpeed::Fs, false),
            test_release("v3", Board::Aim61, UsbSpeed::Hs, false),
        ];
        assert_eq!(preferred_release_index(&stable), Some(1));

        let prereleases = vec![
            test_release("v4-rc", Board::M0sdock, UsbSpeed::Fs, true),
            test_release("v4-rc", Board::Aim61, UsbSpeed::Fs, true),
        ];
        assert_eq!(preferred_release_index(&prereleases), Some(0));
        assert_eq!(preferred_release_index(&[]), None);
    }

    #[test]
    fn newest_standard_prerelease_precedes_older_stable_standard() {
        let releases = vec![
            test_release("v3.5.2", Board::Aim61, UsbSpeed::Hs, true),
            test_release("v3.5.1", Board::Aim61, UsbSpeed::Hs, false),
        ];
        assert_eq!(preferred_release_index(&releases), Some(0));
    }

    #[test]
    fn embedded_flashing_tool_matches_pinned_hash() {
        verify_embedded_tool().unwrap();
    }

    #[test]
    fn gui_language_catalog_switches_both_languages() {
        assert_eq!(Language::ZhCn.tr("中文", "English"), "中文");
        assert_eq!(Language::En.tr("中文", "English"), "English");
        assert_eq!(Language::ZhCn.display_name(), "简体中文");
        assert_eq!(Language::En.display_name(), "English");
    }

    #[test]
    fn firmware_zip_and_bouffalo_runtime_preflight_are_complete() {
        let boot2 = vec![0x42; 52_576];
        let partition = vec![0x50; 308];
        let firmware = vec![0x61; 65_536];
        let boot2_name = "boot2_bl616_test.bin";
        let checksums = format!(
            "{}  {}\n{}  {}\n{}  {}\n",
            sha256(&boot2),
            boot2_name,
            sha256(&partition),
            PARTITION_NAME,
            sha256(&firmware),
            "ds5dongle-lctech616.bin"
        );
        let firmware_json = br#"{"schema":1,"project":"DS5Dongle","version":"test","board":"lctech616","usb_speed":"fs","chip":"bl616","flash_size":4,"boot2":"boot2_bl616_test.bin","partition":"partition.bin","firmware":"ds5dongle-lctech616.bin"}"#;
        let zip_path = env::temp_dir().join(format!(
            "m61-flasher-test-{}-{}.zip",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let file = File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, bytes) in [
                (boot2_name, boot2.as_slice()),
                (PARTITION_NAME, partition.as_slice()),
                ("ds5dongle-lctech616.bin", firmware.as_slice()),
                (CHECKSUM_MANIFEST_NAME, checksums.as_bytes()),
                (FIRMWARE_MANIFEST_NAME, firmware_json),
            ] {
                writer.start_file(name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }

        let set = read_firmware_zip(&zip_path, true).unwrap();
        assert_eq!(set.boot2_name, boot2_name);
        assert_eq!(set.partition.len(), 308);
        let runtime = prepare_local_runtime(&set).unwrap();
        preflight_runtime_layout(&runtime.path).unwrap();
        assert!(
            runtime
                .path
                .join("chips/bl616/eflash_loader/eflash_loader_cfg.ini")
                .is_file()
        );
        fs::remove_file(zip_path).unwrap();
    }

    #[test]
    fn local_firmware_validation_rejects_unsafe_names_and_bad_hashes() {
        let boot2 = vec![0x42; 52_576];
        let partition = vec![0x50; 308];
        let firmware = vec![0x61; 65_536];
        assert!(
            validate_firmware_set(
                "unsafe".to_owned(),
                FirmwareManifest {
                    boot2: "boot2_bl616_:stream.bin".into(),
                    ..test_manifest()
                },
                boot2.clone(),
                partition.clone(),
                firmware.clone(),
                None,
            )
            .is_err()
        );
        let bad_manifest = format!(
            "{}  boot2_bl616_test.bin\n{}  {}\n{}  {}\n",
            "0".repeat(64),
            sha256(&partition),
            PARTITION_NAME,
            sha256(&firmware),
            "ds5dongle-lctech616.bin"
        );
        assert!(
            validate_firmware_set(
                "bad hash".to_owned(),
                test_manifest(),
                boot2,
                partition,
                firmware,
                Some(&bad_manifest),
            )
            .is_err()
        );
    }
}
