use anyhow::{Context, Result};
use chrono::{Local, Utc};
use clap::{ArgAction, ArgGroup, Parser};
use colored::*;
use csv::WriterBuilder;
use hound::WavReader;
use log::{error, info, warn};
use rayon::prelude::*;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{IpAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use which::which;

use viralclip_swarm::ai::{
    build_storyboard, rerank_candidates, write_storyboard, AiClipContext, AiOptions,
};
use viralclip_swarm::model::{
    ApiClientRecord, ApiJobStatus, ApiPrincipal, ApiQuotaState, ApiRunResponse, ApiSecurityConfig,
    BenchmarkLog, ClipTask, ClipTiming, CropPlan, ExportBundle, ExportClipBundle, JobMap,
    PlatformExport, ProcessingContext, ProcessingOptions, ProofReport, QuotaLock, RateLimitMap,
    RunSummary, TranscriptEntry, WindowMetrics,
};
use viralclip_swarm::runtime::{
    command_output_checked, constant_time_eq, normalize_sha256_hex, sha256_hex,
};
use viralclip_swarm::subtitles::{
    burn_subtitles, burn_subtitles_via_ass, SubtitleAnimationPreset, SubtitleRenderOptions,
    SubtitleStyle,
};

#[derive(Parser, Debug)]
#[command(name = "viralclip-swarm")]
#[command(about = "AI-powered viral clip generator", long_about = None)]
#[command(group(
    ArgGroup::new("input_source")
        .args(["url", "input"])
        .multiple(false)
))]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    api: bool,
    #[arg(long, default_value = "127.0.0.1:8787")]
    api_bind: String,
    #[arg(short, long)]
    url: Option<String>,
    #[arg(short, long)]
    input: Option<PathBuf>,
    #[arg(short, long, default_value_t = 10)]
    num_clips: u32,
    #[arg(short, long, default_value = "./output")]
    output_dir: String,
    #[arg(long, default_value_t = 5.0)]
    min_duration: f32,
    #[arg(long, default_value_t = false)]
    captions: bool,
    #[arg(long, default_value_t = false)]
    crop: bool,
    #[arg(long, default_value = "center", value_parser = ["center", "subject", "face"])]
    crop_mode: String,
    #[arg(long, default_value_t = false)]
    accurate: bool,
    #[arg(long, default_value = "python", value_parser = ["binary", "python"])]
    whisper_mode: String,
    #[arg(long, default_value = "base")]
    whisper_model: String,
    #[arg(long, default_value = "local", value_parser = ["local", "openai"])]
    transcription_provider: String,
    #[arg(long, default_value = "whisper-1")]
    cloud_transcription_model: String,
    #[arg(long, default_value = "OPENAI_API_KEY")]
    transcription_api_key_env: String,
    #[arg(long, default_value_t = false)]
    laughter: bool,
    #[arg(long)]
    chat_log: Option<PathBuf>,
    #[arg(long, default_value_t = 1.0)]
    energy_weight: f32,
    #[arg(long, default_value_t = 0.35)]
    motion_weight: f32,
    #[arg(long, default_value_t = 0.75)]
    laughter_weight: f32,
    #[arg(long, default_value_t = 0.75)]
    chat_weight: f32,
    #[arg(long, default_value_t = 0.85)]
    transcript_weight: f32,
    #[arg(long, default_value_t = 0.50)]
    hook_weight: f32,
    #[arg(long)]
    subtitle_style_map: Option<PathBuf>,
    #[arg(long, default_value = "Monospace")]
    subtitle_font: String,
    #[arg(long, default_value_t = 24)]
    subtitle_size: u32,
    #[arg(long, default_value = "&H00FFFFFF")]
    subtitle_color: String,
    #[arg(long, default_value = "&H0000F6FF")]
    subtitle_highlight_color: String,
    #[arg(long, default_value = "&H00000000")]
    subtitle_outline_color: String,
    #[arg(long, default_value = "&H64000000")]
    subtitle_back_color: String,
    #[arg(long, default_value_t = 2)]
    subtitle_outline: u32,
    #[arg(long, default_value_t = 0)]
    subtitle_shadow: u32,
    #[arg(long, default_value_t = 1)]
    subtitle_border_style: u32,
    #[arg(long, default_value_t = false)]
    subtitle_bold: bool,
    #[arg(long, default_value_t = 2)]
    subtitle_alignment: u32,
    #[arg(long, default_value_t = 28)]
    subtitle_margin_v: u32,
    #[arg(
        long,
        default_value = "classic",
        value_parser = ["classic", "legendary", "creator_pro", "creator_neon", "creator_minimal", "creator_bold"]
    )]
    subtitle_preset: String,
    #[arg(long, default_value = "none", value_parser = ["none", "karaoke", "emphasis", "impact", "pulse", "creator_pro"])]
    subtitle_animation: String,
    #[arg(long, default_value_t = false, action = ArgAction::Set)]
    subtitle_emoji_layer: bool,
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    subtitle_beat_sync: bool,
    #[arg(long, default_value_t = false, action = ArgAction::Set)]
    subtitle_scene_fx: bool,
    #[arg(long, default_value_t = false)]
    motion: bool,
    #[arg(long, default_value_t = 0.4)]
    scene_threshold: f64,
    #[arg(long, default_value = "auto", value_parser = ["auto", "ass", "subtitles"])]
    subtitles_mode: String,
    #[arg(long, default_value = "./output/benchmark.csv")]
    csv_path: String,
    #[arg(long, default_value = "csv", value_parser = ["csv", "json", "human"])]
    csv_format: String,
    #[arg(long, default_value = "utc", value_parser = ["utc", "local"])]
    timestamp_mode: String,
    #[arg(long, default_value_t = false)]
    append: bool,
    #[arg(long, default_value_t = false)]
    llm_enable: bool,
    #[arg(long, default_value = "heuristic", value_parser = ["heuristic", "local", "openai", "openrouter", "groq", "huggingface", "anthropic", "gemini"])]
    llm_provider: String,
    #[arg(long, default_value = "gpt-4o-mini")]
    llm_model: String,
    #[arg(long, default_value = "OPENAI_API_KEY")]
    llm_api_key_env: String,
    #[arg(long)]
    llm_output: Option<String>,
    #[arg(long, default_value_t = false)]
    export_bundle: bool,
    #[arg(long)]
    export_bundle_path: Option<String>,
    #[arg(long, default_value_t = false)]
    proof_report: bool,
    #[arg(long)]
    proof_report_path: Option<String>,
    #[arg(long, default_value_t = false)]
    thumbnails: bool,
    #[arg(long)]
    thumbnails_dir: Option<String>,
    #[arg(long, default_value = "framed", value_parser = ["plain", "framed", "cinematic"])]
    thumbnail_style: String,
    #[arg(long, default_value_t = false)]
    thumbnail_collage: bool,
    #[arg(long)]
    thumbnail_collage_path: Option<String>,
    #[arg(long, default_value = "VIRALCLIP_API_KEY")]
    api_key_env: String,
    #[arg(long, default_value = "VIRALCLIP_API_TOKEN_SHA256")]
    api_token_sha256_env: String,
    #[arg(long, default_value_t = 1048576)]
    api_max_body_bytes: usize,
    #[arg(long, default_value_t = 60)]
    api_rate_limit_per_minute: u32,
    #[arg(long, default_value = "VIRALCLIP_API_CLIENTS_JSON")]
    api_clients_json_env: String,
    #[arg(long, default_value_t = false)]
    api_allow_url_input: bool,
    #[arg(long, default_value_t = 32)]
    api_max_queued_jobs: u32,
    #[arg(long, default_value = "./output/security_audit.log")]
    security_audit_log: String,
    #[arg(long, default_value = "./output/api_quota_state.json")]
    api_quota_store: String,
    #[arg(long, default_value_t = 200)]
    api_client_daily_quota_runs: u32,
    #[arg(long, default_value_t = 15)]
    api_read_timeout_secs: u64,
    #[arg(long, default_value_t = 15)]
    api_write_timeout_secs: u64,
    #[arg(long, default_value_t = 8192)]
    api_max_header_line_bytes: usize,
    #[arg(long, default_value = "")]
    api_url_allowlist: String,
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    api_url_dns_guard: bool,
    #[arg(long)]
    malware_scan_cmd: Option<String>,
    #[arg(long, default_value_t = 8589934592)]
    max_input_bytes: u64,
    #[arg(long, default_value = "mp4,mov,mkv,webm,m4v,avi")]
    allowed_input_exts: String,
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    secure_temp_cleanup: bool,
}

#[derive(Deserialize)]
struct SubtitleStylePatch {
    font: Option<String>,
    size: Option<u32>,
    color: Option<String>,
    highlight_color: Option<String>,
    outline_color: Option<String>,
    back_color: Option<String>,
    outline: Option<u32>,
    shadow: Option<u32>,
    border_style: Option<u32>,
    bold: Option<bool>,
    alignment: Option<u32>,
    margin_v: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct ConfigFile {
    api: Option<bool>,
    api_bind: Option<String>,
    url: Option<String>,
    input: Option<PathBuf>,
    num_clips: Option<u32>,
    output_dir: Option<String>,
    min_duration: Option<f32>,
    captions: Option<bool>,
    crop: Option<bool>,
    crop_mode: Option<String>,
    accurate: Option<bool>,
    whisper_mode: Option<String>,
    whisper_model: Option<String>,
    transcription_provider: Option<String>,
    cloud_transcription_model: Option<String>,
    transcription_api_key_env: Option<String>,
    laughter: Option<bool>,
    chat_log: Option<PathBuf>,
    energy_weight: Option<f32>,
    motion_weight: Option<f32>,
    laughter_weight: Option<f32>,
    chat_weight: Option<f32>,
    transcript_weight: Option<f32>,
    hook_weight: Option<f32>,
    subtitle_style_map: Option<PathBuf>,
    subtitle_font: Option<String>,
    subtitle_size: Option<u32>,
    subtitle_color: Option<String>,
    subtitle_highlight_color: Option<String>,
    subtitle_outline_color: Option<String>,
    subtitle_back_color: Option<String>,
    subtitle_outline: Option<u32>,
    subtitle_shadow: Option<u32>,
    subtitle_border_style: Option<u32>,
    subtitle_bold: Option<bool>,
    subtitle_alignment: Option<u32>,
    subtitle_margin_v: Option<u32>,
    subtitle_preset: Option<String>,
    subtitle_animation: Option<String>,
    subtitle_emoji_layer: Option<bool>,
    subtitle_beat_sync: Option<bool>,
    subtitle_scene_fx: Option<bool>,
    motion: Option<bool>,
    scene_threshold: Option<f64>,
    subtitles_mode: Option<String>,
    csv_path: Option<String>,
    csv_format: Option<String>,
    timestamp_mode: Option<String>,
    append: Option<bool>,
    llm_enable: Option<bool>,
    llm_provider: Option<String>,
    llm_model: Option<String>,
    llm_api_key_env: Option<String>,
    llm_output: Option<String>,
    export_bundle: Option<bool>,
    export_bundle_path: Option<String>,
    proof_report: Option<bool>,
    proof_report_path: Option<String>,
    thumbnails: Option<bool>,
    thumbnails_dir: Option<String>,
    thumbnail_style: Option<String>,
    thumbnail_collage: Option<bool>,
    thumbnail_collage_path: Option<String>,
    api_key_env: Option<String>,
    api_token_sha256_env: Option<String>,
    api_max_body_bytes: Option<usize>,
    api_rate_limit_per_minute: Option<u32>,
    api_clients_json_env: Option<String>,
    api_allow_url_input: Option<bool>,
    api_max_queued_jobs: Option<u32>,
    security_audit_log: Option<String>,
    api_quota_store: Option<String>,
    api_client_daily_quota_runs: Option<u32>,
    api_read_timeout_secs: Option<u64>,
    api_write_timeout_secs: Option<u64>,
    api_max_header_line_bytes: Option<usize>,
    api_url_allowlist: Option<String>,
    api_url_dns_guard: Option<bool>,
    malware_scan_cmd: Option<String>,
    max_input_bytes: Option<u64>,
    allowed_input_exts: Option<String>,
    secure_temp_cleanup: Option<bool>,
}

type JobSender = mpsc::SyncSender<(usize, Cli)>;

fn find_yt_dlp() -> Result<PathBuf> {
    which("yt-dlp").context("yt-dlp not found in PATH. Please install it.")
}

fn transcript_cleanup_regexes() -> (
    &'static Regex,
    &'static Regex,
    &'static Regex,
    &'static Regex,
    &'static Regex,
) {
    static TIMESTAMP_RE: OnceLock<Regex> = OnceLock::new();
    static INDEX_RE: OnceLock<Regex> = OnceLock::new();
    static ARROW_RE: OnceLock<Regex> = OnceLock::new();
    static LEADING_INDEX_RE: OnceLock<Regex> = OnceLock::new();
    static WS_RE: OnceLock<Regex> = OnceLock::new();
    (
        TIMESTAMP_RE.get_or_init(|| {
            Regex::new(r"\b\d{2}:\d{2}:\d{2},\d{3}\b").expect("timestamp cleanup regex is valid")
        }),
        INDEX_RE.get_or_init(|| Regex::new(r"^\d+$").expect("index cleanup regex is valid")),
        ARROW_RE.get_or_init(|| Regex::new(r"\s*-->\s*").expect("arrow cleanup regex is valid")),
        LEADING_INDEX_RE
            .get_or_init(|| Regex::new(r"^\d+\s+").expect("leading index regex is valid")),
        WS_RE.get_or_init(|| Regex::new(r"\s+").expect("whitespace cleanup regex is valid")),
    )
}

fn srt_parse_regexes() -> (&'static Regex, &'static Regex) {
    static TIMESTAMP_RE: OnceLock<Regex> = OnceLock::new();
    static INDEX_RE: OnceLock<Regex> = OnceLock::new();
    (
        TIMESTAMP_RE.get_or_init(|| {
            Regex::new(
                r"^(?P<start>\d{2}:\d{2}:\d{2},\d{3})\s*-->\s*(?P<end>\d{2}:\d{2}:\d{2},\d{3})$",
            )
            .expect("srt timestamp regex is valid")
        }),
        INDEX_RE.get_or_init(|| Regex::new(r"^\d+$").expect("srt index regex is valid")),
    )
}

fn transcript_token_regex() -> &'static Regex {
    static TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    TOKEN_RE.get_or_init(|| Regex::new(r"[A-Za-z0-9']+").expect("token regex is valid"))
}

fn signalstats_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"lavfi\.signalstats\.YAVG=([0-9.]+)").expect("signalstats regex is valid")
    })
}

fn face_center_regexes() -> &'static [Regex; 3] {
    static RE: OnceLock<[Regex; 3]> = OnceLock::new();
    RE.get_or_init(|| {
        [
            Regex::new(r"x[:=]\s*(\d+)\s*y[:=]\s*(\d+)\s*w[:=]\s*(\d+)\s*h[:=]\s*(\d+)")
                .expect("face regex 1"),
            Regex::new(r"face.*?x[:=]\s*(\d+).*?y[:=]\s*(\d+).*?w[:=]\s*(\d+).*?h[:=]\s*(\d+)")
                .expect("face regex 2"),
            Regex::new(
                r"left[:=]\s*(\d+).*?top[:=]\s*(\d+).*?width[:=]\s*(\d+).*?height[:=]\s*(\d+)",
            )
            .expect("face regex 3"),
        ]
    })
}

fn cropdetect_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"crop=(\d+):(\d+):(\d+):(\d+)").expect("cropdetect regex is valid")
    })
}

fn scene_pts_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"pts_time:([0-9.]+)").expect("scene pts regex is valid"))
}

fn chat_timestamp_regexes() -> (&'static Regex, &'static Regex) {
    static TIME_RE: OnceLock<Regex> = OnceLock::new();
    static NUMERIC_RE: OnceLock<Regex> = OnceLock::new();
    (
        TIME_RE.get_or_init(|| {
            Regex::new(r"^\[?(\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d+)?)]?")
                .expect("chat timestamp regex is valid")
        }),
        NUMERIC_RE.get_or_init(|| {
            Regex::new(r"^(\d+(?:\.\d+)?)").expect("numeric chat timestamp regex is valid")
        }),
    )
}

fn parse_allowed_extensions(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

#[derive(Clone, Debug)]
struct UrlTarget {
    scheme: String,
    host: String,
    port: u16,
}

fn parse_host_allowlist(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|value| value.trim().to_ascii_lowercase())
        .map(|value| value.trim_start_matches("*.").to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn is_supported_media_extension(path: &Path, allowed: &[String]) -> bool {
    let Some(ext) = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
    else {
        return false;
    };
    allowed.iter().any(|candidate| candidate == &ext)
}

fn parse_http_url_target(url: &str) -> Result<UrlTarget> {
    let trimmed = url.trim();
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        anyhow::bail!("URL must include scheme (http:// or https://)");
    };
    let normalized_scheme = scheme.to_ascii_lowercase();
    if !matches!(normalized_scheme.as_str(), "http" | "https") {
        anyhow::bail!("Only http:// and https:// URLs are allowed for API jobs");
    }

    let authority = rest.split(&['/', '?', '#'][..]).next().unwrap_or_default();
    if authority.contains('@') {
        anyhow::bail!("URLs with embedded credentials are not allowed");
    }
    let host_port = authority.trim();
    if host_port.is_empty() {
        anyhow::bail!("URL host is required");
    }
    if host_port.starts_with('[') {
        let Some(end) = host_port.find(']') else {
            anyhow::bail!("Invalid IPv6 URL host");
        };
        let host = &host_port[1..end];
        if host.is_empty() {
            anyhow::bail!("URL host is required");
        }
        let port = match host_port[end + 1..].strip_prefix(':') {
            Some(port) => port.parse::<u16>().context("parse IPv6 URL port")?,
            None => default_port_for_scheme(&normalized_scheme),
        };
        return Ok(UrlTarget {
            scheme: normalized_scheme,
            host: host.to_ascii_lowercase(),
            port,
        });
    }

    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) => {
            (host.trim(), port.parse::<u16>().context("parse URL port")?)
        }
        _ => (
            host_port.trim(),
            default_port_for_scheme(&normalized_scheme),
        ),
    };
    if host.is_empty() {
        anyhow::bail!("URL host is required");
    }
    Ok(UrlTarget {
        scheme: normalized_scheme,
        host: host.to_ascii_lowercase(),
        port,
    })
}

fn default_port_for_scheme(scheme: &str) -> u16 {
    match scheme {
        "http" => 80,
        _ => 443,
    }
}

fn host_matches_allowlist(host: &str, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    allowlist
        .iter()
        .any(|entry| host == entry || host.ends_with(&format!(".{entry}")))
}

fn ip_disallowed_for_url_target(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_private()
                || ipv4.is_loopback()
                || ipv4.is_link_local()
                || ipv4.is_broadcast()
                || ipv4.is_documentation()
                || ipv4.is_unspecified()
                || ipv4.octets()[0] == 0
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || ipv6.is_unique_local()
                || ipv6.is_unicast_link_local()
                || ipv6.is_multicast()
        }
    }
}

fn validate_api_input_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        anyhow::bail!("API requests may not use absolute input paths");
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        anyhow::bail!("API requests may not use parent directory traversal in input paths");
    }
    let normalized = path.to_string_lossy().replace('\\', "/");
    if !(normalized.starts_with("./input/") || normalized.starts_with("input/")) {
        anyhow::bail!("API local input paths must stay under ./input");
    }
    Ok(())
}

fn validate_api_aux_input_file(path: &Path, max_bytes: u64, label: &str) -> Result<()> {
    validate_api_input_path(path)?;
    if !path.is_file() {
        anyhow::bail!(
            "API {} path is not a regular file: {}",
            label,
            path.display()
        );
    }
    if path.to_string_lossy().starts_with("\\\\") {
        anyhow::bail!("UNC/network paths are not allowed for API {} files", label);
    }
    let metadata = path
        .metadata()
        .with_context(|| format!("read API {} metadata {}", label, path.display()))?;
    if metadata.len() > max_bytes {
        anyhow::bail!(
            "API {} file is too large (> {} bytes): {}",
            label,
            max_bytes,
            path.display()
        );
    }
    Ok(())
}

fn validate_api_url(url: &str, allowlist: &[String], dns_guard: bool) -> Result<()> {
    let target = parse_http_url_target(url)?;
    if !host_matches_allowlist(&target.host, allowlist) {
        anyhow::bail!("URL host is not in api_url_allowlist: {}", target.host);
    }
    let expected_port = default_port_for_scheme(&target.scheme);
    if target.port != expected_port {
        anyhow::bail!(
            "Custom URL ports are not allowed for API jobs (expected {} for {})",
            expected_port,
            target.scheme
        );
    }

    if !dns_guard {
        return Ok(());
    }

    if let Ok(ip) = target.host.parse::<IpAddr>() {
        if ip_disallowed_for_url_target(ip) {
            anyhow::bail!("URL IP target is not allowed: {ip}");
        }
        return Ok(());
    }

    let resolved: Vec<IpAddr> = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .with_context(|| format!("resolve URL host {}", target.host))?
        .map(|socket| socket.ip())
        .collect();
    if resolved.is_empty() {
        anyhow::bail!("URL host did not resolve to any IPs");
    }
    if resolved.iter().any(|ip| ip_disallowed_for_url_target(*ip)) {
        anyhow::bail!("URL host resolves to local/private IPs, blocked by DNS guard");
    }
    Ok(())
}

fn validate_input_file_path(path: &Path, max_bytes: u64, allowed_exts: &[String]) -> Result<()> {
    if !path.is_file() {
        anyhow::bail!(
            "Input file not found or not a regular file: {}",
            path.display()
        );
    }
    if path.to_string_lossy().starts_with("\\\\") {
        anyhow::bail!("UNC/network paths are not allowed for input files");
    }
    if !is_supported_media_extension(path, allowed_exts) {
        anyhow::bail!(
            "Unsupported input extension for {}. Allowed: {}",
            path.display(),
            allowed_exts.join(", ")
        );
    }
    let metadata = path
        .metadata()
        .with_context(|| format!("read input metadata {}", path.display()))?;
    if metadata.len() > max_bytes {
        anyhow::bail!(
            "Input file is too large (> {} bytes): {}",
            max_bytes,
            path.display()
        );
    }
    Ok(())
}

fn validate_api_bind_address(bind_addr: &str) -> Result<()> {
    let ip_part = bind_addr
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(bind_addr)
        .trim_matches(['[', ']']);
    if ip_part.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    let ip = ip_part
        .parse::<IpAddr>()
        .with_context(|| format!("parse API bind address host from {}", bind_addr))?;
    if !ip.is_loopback() {
        anyhow::bail!(
            "Refusing non-loopback API bind address without a reverse proxy/TLS layer: {bind_addr}"
        );
    }
    Ok(())
}

fn api_client_id(stream: &TcpStream) -> String {
    stream
        .peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn check_rate_limit(rate_limits: &RateLimitMap, client_id: &str, per_minute: u32) -> Result<bool> {
    let mut guard = rate_limits
        .lock()
        .map_err(|_| anyhow::anyhow!("rate limit map poisoned"))?;
    let now = Instant::now();
    let window = Duration::from_secs(60);
    if guard.len() > 4096 {
        guard.retain(|_, entries| {
            entries.retain(|instant| now.duration_since(*instant) <= window);
            !entries.is_empty()
        });
    }
    let entries = guard.entry(client_id.to_string()).or_default();
    entries.retain(|instant| now.duration_since(*instant) <= window);
    if entries.len() >= per_minute as usize {
        return Ok(false);
    }
    entries.push(now);
    Ok(true)
}

fn extract_api_token(headers: &HashMap<String, String>) -> Option<String> {
    if let Some(auth) = headers.get("authorization") {
        if let Some(token) = auth.strip_prefix("Bearer ") {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    headers
        .get("x-api-key")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn api_request_authorized(
    headers: &HashMap<String, String>,
    security: &ApiSecurityConfig,
) -> Option<ApiPrincipal> {
    let provided = extract_api_token(headers)?;
    let provided_hash = sha256_hex(&provided);

    for client in &security.clients {
        if constant_time_eq(&provided_hash, &client.token_sha256) {
            return Some(ApiPrincipal {
                client_id: client.client_id.clone(),
                scopes: client.scopes.clone(),
            });
        }
    }

    if let Some(expected_hash) = &security.token_sha256_hex {
        if constant_time_eq(&provided_hash, expected_hash) {
            return Some(ApiPrincipal {
                client_id: "shared-hash-token".to_string(),
                scopes: vec!["read".to_string(), "run".to_string()],
            });
        }
    }
    if let Some(expected_key) = &security.raw_api_key {
        if constant_time_eq(&provided, expected_key) {
            return Some(ApiPrincipal {
                client_id: "shared-raw-token".to_string(),
                scopes: vec!["read".to_string(), "run".to_string()],
            });
        }
    }
    None
}

fn principal_has_scope(principal: &ApiPrincipal, scope: &str) -> bool {
    principal
        .scopes
        .iter()
        .any(|value| value == scope || value == "admin")
}

fn validate_api_output_path(output_path: &Path) -> Result<()> {
    if output_path.is_absolute() {
        anyhow::bail!("API requests may not use absolute output paths");
    }
    if output_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        anyhow::bail!("API requests may not use parent directory traversal in output paths");
    }
    let normalized = output_path.to_string_lossy().replace('\\', "/");
    if !(normalized == "output"
        || normalized.starts_with("./output")
        || normalized.starts_with("output/"))
    {
        anyhow::bail!("API output paths must stay under ./output");
    }
    Ok(())
}

fn validate_api_job_request(cli: &Cli, security: &ApiSecurityConfig) -> Result<()> {
    if cli.api || cli.config.is_some() {
        anyhow::bail!("nested API/config execution is not allowed through the API");
    }
    if cli.url.is_some() && !security.allow_url_input {
        anyhow::bail!("URL input is disabled for API jobs");
    }
    if cli.num_clips == 0 || cli.num_clips > 20 {
        anyhow::bail!("num_clips must be between 1 and 20 for API jobs");
    }
    if !(1.0..=180.0).contains(&cli.min_duration) {
        anyhow::bail!("min_duration must be between 1 and 180 seconds for API jobs");
    }
    if cli.max_input_bytes > 8u64 * 1024 * 1024 * 1024 {
        anyhow::bail!("max_input_bytes may not exceed 8GB for API jobs");
    }
    if cli.malware_scan_cmd.is_some() {
        anyhow::bail!("malware_scan_cmd cannot be set by API requests");
    }
    let allowed_exts = parse_allowed_extensions(&cli.allowed_input_exts);
    if allowed_exts.is_empty() {
        anyhow::bail!("allowed_input_exts may not be empty");
    }
    match (&cli.input, &cli.url) {
        (Some(input), None) => {
            validate_api_input_path(input)?;
            validate_input_file_path(input, cli.max_input_bytes, &allowed_exts)?;
        }
        (None, Some(url)) => {
            validate_api_url(url, &security.url_allowlist, security.url_dns_guard)?
        }
        (Some(_), Some(_)) => anyhow::bail!("Provide either input or URL, not both for API jobs"),
        (None, None) => anyhow::bail!("API jobs must provide input or URL"),
    }
    validate_api_output_path(Path::new(&cli.output_dir))?;
    validate_api_output_path(Path::new(&cli.csv_path))?;
    if let Some(path) = &cli.llm_output {
        validate_api_output_path(Path::new(path))?;
    }
    if let Some(path) = &cli.export_bundle_path {
        validate_api_output_path(Path::new(path))?;
    }
    if let Some(path) = &cli.proof_report_path {
        validate_api_output_path(Path::new(path))?;
    }
    if let Some(path) = &cli.thumbnails_dir {
        validate_api_output_path(Path::new(path))?;
    }
    if let Some(path) = &cli.thumbnail_collage_path {
        validate_api_output_path(Path::new(path))?;
    }
    if let Some(path) = &cli.chat_log {
        validate_api_aux_input_file(path, cli.max_input_bytes, "chat_log")?;
    }
    if let Some(path) = &cli.subtitle_style_map {
        validate_api_aux_input_file(path, cli.max_input_bytes, "subtitle_style_map")?;
    }
    Ok(())
}

fn count_active_jobs(jobs: &JobMap) -> Result<usize> {
    let guard = jobs
        .lock()
        .map_err(|_| anyhow::anyhow!("job map poisoned"))?;
    Ok(guard
        .values()
        .filter(|job| matches!(job.status.as_str(), "queued" | "running"))
        .count())
}

fn append_security_audit_log(
    path: &Path,
    event: &str,
    client_id: &str,
    outcome: &str,
    detail: &str,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create audit log dir {}", parent.display()))?;
        }
    }
    let payload = serde_json::json!({
        "timestamp": Utc::now().to_rfc3339(),
        "event": event,
        "client_id": client_id,
        "outcome": outcome,
        "detail": detail,
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open audit log {}", path.display()))?;
    writeln!(file, "{payload}").context("write audit log entry")?;
    Ok(())
}

fn load_api_clients_from_env(env_name: &str) -> Result<Vec<ApiClientRecord>> {
    let Some(raw) = env::var(env_name)
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(Vec::new());
    };
    let mut clients: Vec<ApiClientRecord> = serde_json::from_str(&raw)
        .with_context(|| format!("parse API clients JSON from {}", env_name))?;
    for client in &mut clients {
        if client.client_id.trim().is_empty() {
            anyhow::bail!("API client_id may not be empty");
        }
        client.token_sha256 = normalize_sha256_hex(&client.token_sha256)
            .with_context(|| format!("invalid token_sha256 for client {}", client.client_id))?;
    }
    Ok(clients)
}

fn current_utc_day() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn load_quota_state(path: &Path, day_utc: &str) -> Result<ApiQuotaState> {
    if !path.exists() {
        return Ok(ApiQuotaState {
            day_utc: day_utc.to_string(),
            clients: HashMap::new(),
        });
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read quota state {}", path.display()))?;
    let mut state: ApiQuotaState = serde_json::from_str(&raw)
        .with_context(|| format!("parse quota state {}", path.display()))?;
    if state.day_utc != day_utc {
        state.day_utc = day_utc.to_string();
        state.clients.clear();
    }
    Ok(state)
}

fn save_quota_state(path: &Path, state: &ApiQuotaState) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create quota state dir {}", parent.display()))?;
        }
    }
    let payload = serde_json::to_string_pretty(state).context("serialize quota state")?;
    std::fs::write(path, payload)
        .with_context(|| format!("write quota state {}", path.display()))?;
    Ok(())
}

fn check_and_increment_client_quota(
    lock: &QuotaLock,
    path: &Path,
    client_id: &str,
    limit: u32,
) -> Result<bool> {
    let _guard = lock
        .lock()
        .map_err(|_| anyhow::anyhow!("quota lock poisoned"))?;
    let day = current_utc_day();
    let mut state = load_quota_state(path, &day)?;
    let entry = state.clients.entry(client_id.to_string()).or_insert(0);
    if *entry >= limit {
        return Ok(false);
    }
    *entry += 1;
    save_quota_state(path, &state)?;
    Ok(true)
}

fn parse_command_template(command: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quote != Some('\'') => escaped = true,
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                } else {
                    current.push(ch);
                }
            }
            ch if ch.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if escaped || quote.is_some() {
        anyhow::bail!("invalid malware_scan_cmd: unmatched quote or trailing escape");
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.is_empty() {
        anyhow::bail!("invalid malware_scan_cmd: command is empty");
    }
    Ok(tokens)
}

fn maybe_run_malware_scan(scan_cmd: Option<&str>, target: &Path) -> Result<()> {
    let Some(scan_cmd) = scan_cmd.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let target_str = target.to_string_lossy().to_string();
    let mut tokens = parse_command_template(scan_cmd)?;
    let executable = tokens.remove(0);
    let mut args = tokens
        .into_iter()
        .map(|token| token.replace("{path}", &target_str))
        .collect::<Vec<_>>();
    if !args.iter().any(|arg| arg.contains(&target_str)) {
        args.push(target_str.clone());
    }

    println!(
        "{}",
        format!("Running malware scan command for {}", target.display()).cyan()
    );
    let mut command = Command::new(executable);
    command.args(&args);
    let status = command.status().context("run malware scan command")?;
    if !status.success() {
        anyhow::bail!("malware scan failed for {}", target.display());
    }
    Ok(())
}

fn download_video(url: &str, output_dir: &Path) -> Result<PathBuf> {
    let yt_dlp = find_yt_dlp()?;
    let status = Command::new(&yt_dlp)
        .arg(url)
        .arg("-o")
        .arg(
            output_dir
                .join("%(title)s.%(ext)s")
                .to_string_lossy()
                .to_string(),
        )
        .arg("--format")
        .arg("bestvideo+bestaudio/best")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--no-progress")
        .status()
        .context("failed to run yt-dlp")?;
    if !status.success() {
        anyhow::bail!("yt-dlp failed with exit code: {:?}", status.code());
    }

    let mut files: Vec<_> = std::fs::read_dir(output_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "mp4"))
        .collect();
    files.sort_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok());
    files
        .last()
        .map(|entry| entry.path())
        .context("No mp4 file found after download")
}

fn has_audio_stream(video_path: &Path) -> Result<bool> {
    let ffprobe = which("ffprobe").context("ffprobe not found in PATH")?;
    let mut cmd = Command::new(&ffprobe);
    cmd.arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("a:0")
        .arg("-show_entries")
        .arg("stream=codec_type")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(video_path);

    match command_output_checked(&mut cmd, "probing audio stream") {
        Ok(output) => Ok(String::from_utf8_lossy(&output.stdout).contains("audio")),
        Err(_) => Ok(false),
    }
}

fn generate_silent_audio(video_path: &Path, output_wav: &Path) -> Result<()> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let duration =
        probe_video_duration(video_path).context("probe duration for silent audio fallback")?;
    warn!(
        "Input {} has no usable audio stream; generating {:.2}s of silent fallback audio",
        video_path.display(),
        duration
    );
    let status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("anullsrc=r=16000:cl=mono")
        .arg("-t")
        .arg(format!("{duration:.3}"))
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg(output_wav)
        .status()
        .context("failed to run ffmpeg for silent audio fallback")?;
    if !status.success() {
        anyhow::bail!("ffmpeg silent audio fallback failed");
    }
    Ok(())
}

fn extract_audio(video_path: &Path, output_wav: &Path) -> Result<()> {
    if !has_audio_stream(video_path)? {
        return generate_silent_audio(video_path, output_wav);
    }

    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-map")
        .arg("0:a:0")
        .arg("-vn")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg(output_wav)
        .status()
        .context("failed to run ffmpeg for audio extraction")?;
    if status.success() {
        return Ok(());
    }

    warn!(
        "Audio extraction failed for {}; falling back to silent audio",
        video_path.display()
    );
    generate_silent_audio(video_path, output_wav)
}

fn normalize_scores(values: &[f32]) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }

    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for value in values {
        min = min.min(*value);
        max = max.max(*value);
    }
    if (max - min).abs() < f32::EPSILON {
        return vec![0.0; values.len()];
    }
    values
        .iter()
        .map(|value| (value - min) / (max - min))
        .collect()
}

fn analyze_audio_windows(wav_path: &Path, window_secs: f32) -> Result<Vec<WindowMetrics>> {
    let reader = WavReader::open(wav_path).context("opening WAV for audio analysis")?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let window_size = ((window_secs * sample_rate as f32) as usize).max(1);
    let frame_size = ((sample_rate as f32 * 0.05) as usize).max(1);

    let samples: Vec<i16> = reader
        .into_samples::<i16>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to read WAV samples")?;

    let mut starts = Vec::new();
    let mut energy_raw = Vec::new();
    let mut zcr_raw = Vec::new();
    let mut modulation_raw = Vec::new();

    for (window_idx, chunk) in samples.chunks(window_size).enumerate() {
        if chunk.is_empty() {
            continue;
        }

        let sum_sq: f32 = chunk.iter().map(|&sample| (sample as f32).powi(2)).sum();
        let rms = (sum_sq / chunk.len() as f32).sqrt();

        let crossings = chunk
            .windows(2)
            .filter(|pair| (pair[0] >= 0 && pair[1] < 0) || (pair[0] < 0 && pair[1] >= 0))
            .count() as f32;
        let zcr = if chunk.len() > 1 {
            crossings / (chunk.len() - 1) as f32
        } else {
            0.0
        };

        let frame_rms: Vec<f32> = chunk
            .chunks(frame_size)
            .map(|frame| {
                let frame_sum_sq: f32 = frame.iter().map(|&sample| (sample as f32).powi(2)).sum();
                (frame_sum_sq / frame.len() as f32).sqrt()
            })
            .collect();
        let modulation = if frame_rms.is_empty() {
            0.0
        } else {
            let mean = frame_rms.iter().sum::<f32>() / frame_rms.len() as f32;
            let variance = frame_rms
                .iter()
                .map(|value| {
                    let delta = value - mean;
                    delta * delta
                })
                .sum::<f32>()
                / frame_rms.len() as f32;
            let std_dev = variance.sqrt();
            if mean > 0.0 {
                std_dev / mean
            } else {
                0.0
            }
        };

        starts.push(window_idx as f32 * window_secs);
        energy_raw.push(rms);
        zcr_raw.push(zcr);
        modulation_raw.push(modulation);
    }

    let energy_scores = normalize_scores(&energy_raw);
    let zcr_scores = normalize_scores(&zcr_raw);
    let modulation_scores = normalize_scores(&modulation_raw);

    let mut windows = Vec::with_capacity(starts.len());
    for idx in 0..starts.len() {
        windows.push(WindowMetrics {
            start_sec: starts[idx],
            energy: energy_scores[idx],
            laughter: (modulation_scores[idx] * 0.55)
                + (zcr_scores[idx] * 0.30)
                + (energy_scores[idx] * 0.15),
            motion: 0.0,
            chat: 0.0,
            transcript: 0.0,
            hook: 0.0,
            semantic: 0.0,
            speech_confidence: 0.0,
            face_score: 0.0,
            transcript_density: 0.0,
            transcript_text: String::new(),
            score: 0.0,
        });
    }
    Ok(windows)
}

fn extract_clip(
    video_path: &Path,
    start_sec: f32,
    duration_sec: f32,
    output_path: &Path,
    accurate: bool,
) -> Result<()> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let start_str = format!("{start_sec}");
    let dur_str = format!("{duration_sec}");

    let status = if accurate {
        Command::new(&ffmpeg)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-i")
            .arg(video_path)
            .arg("-ss")
            .arg(&start_str)
            .arg("-t")
            .arg(&dur_str)
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("fast")
            .arg("-crf")
            .arg("23")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("128k")
            .arg(output_path)
            .status()
            .context("failed to run ffmpeg (accurate mode)")?
    } else {
        Command::new(&ffmpeg)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-ss")
            .arg(&start_str)
            .arg("-i")
            .arg(video_path)
            .arg("-t")
            .arg(&dur_str)
            .arg("-c")
            .arg("copy")
            .arg("-avoid_negative_ts")
            .arg("make_zero")
            .arg(output_path)
            .status()
            .context("failed to run ffmpeg (fast mode)")?
    };

    if !status.success() {
        anyhow::bail!("ffmpeg clip extraction failed");
    }
    Ok(())
}

fn parse_clock_timestamp(raw: &str) -> Option<f32> {
    let value = raw.trim().trim_matches(['[', ']']);
    let parts: Vec<&str> = value.split(':').collect();
    match parts.as_slice() {
        [minutes, seconds] => {
            let minutes = minutes.parse::<f32>().ok()?;
            let seconds = seconds.replace(',', ".").parse::<f32>().ok()?;
            Some(minutes * 60.0 + seconds)
        }
        [hours, minutes, seconds] => {
            let hours = hours.parse::<f32>().ok()?;
            let minutes = minutes.parse::<f32>().ok()?;
            let seconds = seconds.replace(',', ".").parse::<f32>().ok()?;
            Some(hours * 3600.0 + minutes * 60.0 + seconds)
        }
        _ => None,
    }
}

fn parse_srt_timestamp(ts: &str) -> Option<f32> {
    let parts: Vec<&str> = ts.split([':', ',']).collect();
    if parts.len() != 4 {
        return None;
    }
    let hours = parts[0].parse::<f32>().ok()?;
    let minutes = parts[1].parse::<f32>().ok()?;
    let seconds = parts[2].parse::<f32>().ok()?;
    let millis = parts[3].parse::<f32>().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds + millis / 1000.0)
}

fn format_srt_timestamp(seconds: f32) -> String {
    let total_millis = (seconds.max(0.0) * 1000.0).round() as u64;
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis % 3_600_000) / 60_000;
    let secs = (total_millis % 60_000) / 1000;
    let millis = total_millis % 1000;
    format!("{hours:02}:{minutes:02}:{secs:02},{millis:03}")
}

fn extract_srt_segment(
    full_srt: &Path,
    start_sec: f32,
    end_sec: f32,
    output_srt: &Path,
) -> Result<()> {
    let entries = parse_srt_entries(full_srt)?;
    let mut clip_index = 1usize;
    let mut out_blocks = Vec::new();

    for entry in entries {
        let source_start = entry.start_sec;
        let source_end = entry.end_sec;
        if source_start >= end_sec || source_end <= start_sec {
            continue;
        }

        let shifted_start = (source_start - start_sec).max(0.0);
        let shifted_end = (source_end - start_sec).max(shifted_start);
        let mut rewritten = format!(
            "{}\n{} --> {}\n",
            clip_index,
            format_srt_timestamp(shifted_start),
            format_srt_timestamp(shifted_end)
        );
        for line in entry.text.split(" / ") {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
        out_blocks.push(rewritten.trim_end().to_string());
        clip_index += 1;
    }

    std::fs::write(output_srt, out_blocks.join("\n\n")).context("writing clip SRT")?;
    Ok(())
}

fn clean_transcript_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let (timestamp_re, index_re, arrow_re, leading_index_re, ws_re) = transcript_cleanup_regexes();

    let mut parts = Vec::new();
    for raw_line in normalized.lines() {
        let line = raw_line.trim();
        if line.is_empty() || index_re.is_match(line) {
            continue;
        }
        let line = arrow_re.replace_all(line, " ");
        let line = timestamp_re.replace_all(&line, " ");
        let line = leading_index_re.replace_all(&line, "");
        let line = ws_re.replace_all(line.trim(), " ");
        let line = line.trim();
        if !line.is_empty() {
            parts.push(line.to_string());
        }
    }

    let joined = parts.join(" ");
    ws_re.replace_all(joined.trim(), " ").trim().to_string()
}

fn parse_srt_entries(path: &Path) -> Result<Vec<TranscriptEntry>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read transcript {}", path.display()))?;
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut entries = Vec::new();
    let (timestamp_re, index_re) = srt_parse_regexes();

    let mut current_start = None;
    let mut current_end = None;
    let mut current_lines: Vec<String> = Vec::new();

    let flush_entry = |entries: &mut Vec<TranscriptEntry>,
                       current_start: &mut Option<f32>,
                       current_end: &mut Option<f32>,
                       current_lines: &mut Vec<String>| {
        if let (Some(start_sec), Some(end_sec)) = (*current_start, *current_end) {
            let joined = clean_transcript_text(&current_lines.join(" / "));
            if !joined.is_empty() {
                entries.push(TranscriptEntry {
                    start_sec,
                    end_sec,
                    text: joined,
                });
            }
        }
        *current_start = None;
        *current_end = None;
        current_lines.clear();
    };

    for raw_line in normalized.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if index_re.is_match(line) {
            continue;
        }

        if let Some(capture) = timestamp_re.captures(line) {
            flush_entry(
                &mut entries,
                &mut current_start,
                &mut current_end,
                &mut current_lines,
            );
            current_start = parse_srt_timestamp(&capture["start"]);
            current_end = parse_srt_timestamp(&capture["end"]);
            continue;
        }

        if current_start.is_some() {
            current_lines.push(line.to_string());
        }
    }

    flush_entry(
        &mut entries,
        &mut current_start,
        &mut current_end,
        &mut current_lines,
    );

    Ok(entries)
}

fn transcript_tokens(text: &str) -> Vec<String> {
    transcript_token_regex()
        .find_iter(text)
        .map(|m| m.as_str().to_ascii_lowercase())
        .filter(|token| token.len() > 1)
        .collect()
}

fn estimate_hook_signal(text: &str) -> f32 {
    let lower = text.to_ascii_lowercase();
    let hooks = [
        "how",
        "why",
        "secret",
        "mistake",
        "crazy",
        "wild",
        "unbelievable",
        "never",
        "best",
        "worst",
        "don't",
        "do not",
        "should",
        "hack",
        "truth",
        "exposed",
        "imagine",
        "wait",
    ];
    let hook_hits = hooks.iter().filter(|hook| lower.contains(**hook)).count() as f32;
    let punctuation_bonus =
        lower.matches('?').count() as f32 * 0.35 + lower.matches('!').count() as f32 * 0.25;
    let number_bonus = if lower.chars().any(|ch| ch.is_ascii_digit()) {
        0.35
    } else {
        0.0
    };
    hook_hits + punctuation_bonus + number_bonus
}

fn unique_token_ratio(tokens: &[String]) -> f32 {
    if tokens.is_empty() {
        return 0.0;
    }
    let unique = tokens
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len() as f32;
    unique / tokens.len() as f32
}

fn repeated_phrase_penalty(tokens: &[String]) -> f32 {
    if tokens.len() < 4 {
        return 0.0;
    }

    let mut repeated = 0usize;
    for window in tokens.windows(2) {
        if window[0] == window[1] {
            repeated += 1;
        }
    }

    let mut repeating_bigrams = 0usize;
    for idx in 0..tokens.len().saturating_sub(3) {
        if tokens[idx] == tokens[idx + 2] && tokens[idx + 1] == tokens[idx + 3] {
            repeating_bigrams += 1;
        }
    }

    ((repeated as f32 * 0.6) + repeating_bigrams as f32) / tokens.len() as f32
}

fn estimate_semantic_signal(text: &str) -> f32 {
    let lower = text.to_ascii_lowercase();
    let contrast_terms = [
        "but", "instead", "except", "until", "however", "finally", "suddenly", "then",
    ];
    let payoff_terms = [
        "because",
        "so",
        "therefore",
        "that's why",
        "this is why",
        "which means",
        "turns out",
    ];
    let direct_terms = ["you", "your", "here's", "watch", "listen", "look"];
    let stakes_terms = [
        "lose", "won", "risk", "mistake", "secret", "truth", "crazy", "never", "best", "worst",
    ];

    let count_matches = |terms: &[&str]| -> f32 {
        terms.iter().filter(|term| lower.contains(**term)).count() as f32
    };

    let punctuation =
        lower.matches('?').count() as f32 * 0.35 + lower.matches('!').count() as f32 * 0.20;
    let numeric = if lower.chars().any(|ch| ch.is_ascii_digit()) {
        0.35
    } else {
        0.0
    };

    (count_matches(&contrast_terms) * 0.55)
        + (count_matches(&payoff_terms) * 0.65)
        + (count_matches(&direct_terms) * 0.25)
        + (count_matches(&stakes_terms) * 0.50)
        + punctuation
        + numeric
}

fn estimate_speech_confidence(text: &str, density: f32) -> f32 {
    let tokens = transcript_tokens(text);
    if tokens.is_empty() {
        return 0.0;
    }

    let alpha_ratio = text.chars().filter(|ch| ch.is_ascii_alphabetic()).count() as f32
        / text.chars().count().max(1) as f32;
    let avg_len = tokens.iter().map(|token| token.len() as f32).sum::<f32>() / tokens.len() as f32;
    let lexical = unique_token_ratio(&tokens);
    let repetition_penalty = repeated_phrase_penalty(&tokens).clamp(0.0, 1.0);
    let density_fit = if density <= 0.0 {
        0.0
    } else if density < 1.0 {
        density / 1.0
    } else if density <= 5.5 {
        1.0
    } else {
        (1.0 - ((density - 5.5) / 4.0)).clamp(0.0, 1.0)
    };
    let avg_len_fit = ((avg_len - 2.0) / 3.0).clamp(0.0, 1.0);

    ((alpha_ratio * 0.30) + (lexical * 0.25) + (density_fit * 0.25) + (avg_len_fit * 0.20)
        - (repetition_penalty * 0.45))
        .clamp(0.0, 1.0)
}

fn estimate_music_noise_penalty(text: &str, density: f32) -> f32 {
    let lower = text.to_ascii_lowercase();
    let tokens = transcript_tokens(text);
    if tokens.is_empty() {
        return 1.0;
    }

    let repeated = repeated_phrase_penalty(&tokens);
    let non_alpha_ratio = 1.0
        - (text
            .chars()
            .filter(|ch| ch.is_ascii_alphabetic() || ch.is_whitespace())
            .count() as f32
            / text.chars().count().max(1) as f32);
    let lyric_markers = ["oh", "yeah", "la", "na", "woah", "ooh", "uh"];
    let lyric_hits = lyric_markers
        .iter()
        .filter(|term| lower.contains(**term))
        .count() as f32;
    let density_penalty = if density > 6.5 {
        ((density - 6.5) / 4.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    ((repeated * 0.50) + (non_alpha_ratio * 0.20) + (density_penalty * 0.20) + (lyric_hits * 0.12))
        .clamp(0.0, 1.0)
}

fn readability_from_density_and_confidence(words_per_sec: f32, speech_confidence: f32) -> f32 {
    (readability_from_density(words_per_sec) * (0.35 + (speech_confidence.clamp(0.0, 1.0) * 0.65)))
        .clamp(0.0, 1.0)
}

fn readability_from_density(words_per_sec: f32) -> f32 {
    let ideal_low = 1.8f32;
    let ideal_high = 4.2f32;
    if words_per_sec <= 0.0 {
        return 0.0;
    }
    if words_per_sec < ideal_low {
        return (words_per_sec / ideal_low).clamp(0.0, 1.0);
    }
    if words_per_sec <= ideal_high {
        return 1.0;
    }
    (1.0 - ((words_per_sec - ideal_high) / 4.0)).clamp(0.0, 1.0)
}

fn overlap_duration(start_a: f32, end_a: f32, start_b: f32, end_b: f32) -> f32 {
    (end_a.min(end_b) - start_a.max(start_b)).max(0.0)
}

struct TranscriptEntryStats<'a> {
    entry: &'a TranscriptEntry,
    word_count: usize,
}

fn analyze_transcript_window(
    entry_stats: &[TranscriptEntryStats<'_>],
    start: f32,
    end: f32,
    excerpt_words: usize,
) -> (f32, String, String) {
    let window_duration = (end - start).max(0.001);
    let mut effective_words = 0.0f32;
    let mut combined_text = String::new();
    let mut excerpt = String::new();
    let mut excerpt_count = 0usize;
    let mut last_text: Option<&str> = None;

    for stat in entry_stats {
        let entry = stat.entry;
        let overlap = overlap_duration(start, end, entry.start_sec, entry.end_sec);
        if overlap <= 0.0 {
            continue;
        }

        let entry_duration = (entry.end_sec - entry.start_sec).max(0.001);
        effective_words += stat.word_count as f32 * (overlap / entry_duration);

        if !combined_text.is_empty() {
            combined_text.push(' ');
        }
        combined_text.push_str(&entry.text);

        if last_text == Some(entry.text.as_str()) {
            continue;
        }
        last_text = Some(entry.text.as_str());

        if excerpt_count >= excerpt_words {
            continue;
        }
        for word in transcript_token_regex().find_iter(&entry.text) {
            let token = word.as_str().to_ascii_lowercase();
            if token.len() <= 1 {
                continue;
            }
            if !excerpt.is_empty() {
                excerpt.push(' ');
            }
            excerpt.push_str(&token);
            excerpt_count += 1;
            if excerpt_count >= excerpt_words {
                break;
            }
        }
    }

    (effective_words / window_duration, combined_text, excerpt)
}

fn transcript_excerpt_for_range(
    entries: &[TranscriptEntry],
    start: f32,
    end: f32,
    max_words: usize,
) -> String {
    let entry_stats = entries
        .iter()
        .map(|entry| TranscriptEntryStats {
            entry,
            word_count: transcript_token_regex()
                .find_iter(&entry.text)
                .filter(|token| token.as_str().len() > 1)
                .count(),
        })
        .collect::<Vec<_>>();
    analyze_transcript_window(&entry_stats, start, end, max_words).2
}

fn transcript_density_for_range(entries: &[TranscriptEntry], start: f32, end: f32) -> f32 {
    let entry_stats = entries
        .iter()
        .map(|entry| TranscriptEntryStats {
            entry,
            word_count: transcript_token_regex()
                .find_iter(&entry.text)
                .filter(|token| token.as_str().len() > 1)
                .count(),
        })
        .collect::<Vec<_>>();
    analyze_transcript_window(&entry_stats, start, end, 0).0
}

fn apply_transcript_scores(
    windows: &mut [WindowMetrics],
    entries: &[TranscriptEntry],
    window_secs: f32,
) {
    if windows.is_empty() || entries.is_empty() {
        return;
    }

    let mut transcript_raw = Vec::with_capacity(windows.len());
    let mut density_raw = Vec::with_capacity(windows.len());
    let mut hook_raw = Vec::with_capacity(windows.len());
    let mut semantic_raw = Vec::with_capacity(windows.len());
    let mut speech_raw = Vec::with_capacity(windows.len());
    let mut texts = Vec::with_capacity(windows.len());
    let entry_stats = entries
        .iter()
        .map(|entry| TranscriptEntryStats {
            entry,
            word_count: transcript_token_regex()
                .find_iter(&entry.text)
                .filter(|token| token.as_str().len() > 1)
                .count(),
        })
        .collect::<Vec<_>>();

    for window in windows.iter() {
        let start = window.start_sec;
        let end = start + window_secs;
        let (density, text, excerpt) = analyze_transcript_window(&entry_stats, start, end, 16);
        if text.is_empty() {
            transcript_raw.push(0.0);
            density_raw.push(0.0);
            hook_raw.push(0.0);
            semantic_raw.push(0.0);
            speech_raw.push(0.0);
            texts.push(String::new());
            continue;
        }

        let tokens = transcript_tokens(&text);
        let hook = estimate_hook_signal(&text);
        let lexical_variety = unique_token_ratio(&tokens);
        let semantic = estimate_semantic_signal(&text);
        let speech_confidence = estimate_speech_confidence(&text, density);
        let noise_penalty = estimate_music_noise_penalty(&text, density);
        let transcript_signal = ((density * 0.30) + (lexical_variety * 1.10) + (semantic * 0.90))
            * (0.25 + (speech_confidence * 0.75));

        transcript_raw.push((transcript_signal * (1.0 - (noise_penalty * 0.70))).max(0.0));
        density_raw.push(density);
        hook_raw
            .push((hook * (0.30 + (speech_confidence * 0.70))) * (1.0 - (noise_penalty * 0.55)));
        semantic_raw.push((semantic * (1.0 - (noise_penalty * 0.45))).max(0.0));
        speech_raw.push(speech_confidence);
        texts.push(excerpt);
    }

    let transcript_norm = normalize_scores(&transcript_raw);
    let hook_norm = normalize_scores(&hook_raw);
    let semantic_norm = normalize_scores(&semantic_raw);

    for (idx, window) in windows.iter_mut().enumerate() {
        window.transcript = transcript_norm[idx];
        window.hook = hook_norm[idx];
        window.semantic = semantic_norm[idx];
        window.speech_confidence = speech_raw[idx];
        window.transcript_density = density_raw[idx];
        window.transcript_text = texts[idx].clone();
    }
}

fn transcript_similarity(lhs: &str, rhs: &str) -> f32 {
    let left = transcript_tokens(lhs);
    let right = transcript_tokens(rhs);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let left_set: std::collections::HashSet<_> = left.iter().collect();
    let right_set: std::collections::HashSet<_> = right.iter().collect();
    let intersection = left_set.intersection(&right_set).count() as f32;
    let union = left_set.union(&right_set).count() as f32;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn probe_video_dimensions(video_path: &Path) -> Result<(u32, u32)> {
    let ffprobe = which("ffprobe").context("ffprobe not found in PATH")?;
    let mut cmd = Command::new(&ffprobe);
    cmd.arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=width,height")
        .arg("-of")
        .arg("default=noprint_wrappers=1")
        .arg(video_path);
    let output = command_output_checked(&mut cmd, "running ffprobe for dimensions")?;

    let stdout = String::from_utf8(output.stdout).context("ffprobe output not utf8")?;
    let mut width = None;
    let mut height = None;
    for line in stdout.lines() {
        if let Some(stripped) = line.strip_prefix("width=") {
            width = stripped.parse::<u32>().ok();
        } else if let Some(stripped) = line.strip_prefix("height=") {
            height = stripped.parse::<u32>().ok();
        }
    }

    match (width, height) {
        (Some(width), Some(height)) => Ok((width, height)),
        _ => anyhow::bail!("Could not parse video dimensions from ffprobe output: {stdout}"),
    }
}

fn probe_video_duration(video_path: &Path) -> Result<f32> {
    let ffprobe = which("ffprobe").context("ffprobe not found in PATH")?;
    let mut cmd = Command::new(&ffprobe);
    cmd.arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(video_path);
    let output = command_output_checked(&mut cmd, "running ffprobe for duration")?;

    let stdout = String::from_utf8(output.stdout).context("ffprobe duration output not utf8")?;
    stdout
        .trim()
        .parse::<f32>()
        .context("parse probed duration as float")
}

fn clamp_crop_axis(center: i64, size: u32, full_size: u32) -> u32 {
    if size >= full_size {
        return 0;
    }
    let half = (size / 2) as i64;
    let max = (full_size - size) as i64;
    (center - half).clamp(0, max) as u32
}

fn plan_center_crop(width: u32, height: u32) -> CropPlan {
    let target_width = ((height as f32) * 9.0 / 16.0).round() as u32;
    let crop_width = width.min(target_width.max(1));
    CropPlan {
        width: crop_width,
        height,
        x: (width.saturating_sub(crop_width)) / 2,
        y: 0,
    }
}

fn sample_candidate_crop_positions(width: u32, crop_width: u32, samples: usize) -> Vec<u32> {
    if crop_width >= width || samples == 0 {
        return vec![0];
    }

    let max_x = width - crop_width;
    if samples == 1 {
        return vec![max_x / 2];
    }

    let mut positions = Vec::with_capacity(samples);
    for idx in 0..samples {
        let ratio = idx as f32 / (samples - 1) as f32;
        let x = (max_x as f32 * ratio).round() as u32;
        positions.push(x.min(max_x));
    }
    positions.sort_unstable();
    positions.dedup();
    positions
}

fn estimate_crop_activity(
    video_path: &Path,
    crop_width: u32,
    height: u32,
    crop_x: u32,
) -> Result<f32> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let filter = format!(
        "fps=2,crop={}:{}:{}:0,tblend=all_mode=difference,signalstats,metadata=mode=print",
        crop_width, height, crop_x
    );
    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("info")
        .arg("-t")
        .arg("12")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(&filter)
        .arg("-frames:v")
        .arg("24")
        .arg("-f")
        .arg("null")
        .arg("-");
    let output = command_output_checked(
        &mut cmd,
        &format!("running ffmpeg motion probe for crop x={crop_x}"),
    )?;

    let stderr = String::from_utf8(output.stderr).context("ffmpeg motion probe output not utf8")?;
    let mut values = Vec::new();
    for capture in signalstats_regex().captures_iter(&stderr) {
        if let Ok(value) = capture[1].parse::<f32>() {
            values.push(value);
        }
    }

    if values.is_empty() {
        return Ok(0.0);
    }

    Ok(values.iter().sum::<f32>() / values.len() as f32)
}

fn detect_activity_center(
    video_path: &Path,
    width: u32,
    height: u32,
    crop_width: u32,
) -> Result<Option<u32>> {
    if crop_width >= width {
        return Ok(None);
    }

    let candidates = sample_candidate_crop_positions(width, crop_width, 5);
    let mut best_score = f32::NEG_INFINITY;
    let mut best_center = None;

    for crop_x in candidates {
        let score = estimate_crop_activity(video_path, crop_width, height, crop_x)?;
        if score > best_score {
            best_score = score;
            best_center = Some(crop_x + crop_width / 2);
        }
    }

    Ok(best_center)
}

fn parse_face_centers(stderr: &str, scale_ratio: f32) -> Vec<u32> {
    let mut centers = Vec::new();
    for line in stderr.lines() {
        for re in face_center_regexes() {
            if let Some(capture) = re.captures(line) {
                let x = capture[1].parse::<f32>().ok();
                let w = capture[3].parse::<f32>().ok();
                if let (Some(x), Some(w)) = (x, w) {
                    let center = ((x + (w / 2.0)) * scale_ratio).round();
                    if center.is_finite() && center >= 0.0 {
                        centers.push(center as u32);
                    }
                    break;
                }
            }
        }
    }
    centers
}

fn face_detection_count(stderr: &str) -> usize {
    parse_face_centers(stderr, 1.0).len()
}

fn estimate_face_presence_for_range(
    video_path: &Path,
    start_sec: f32,
    duration_sec: f32,
) -> Result<f32> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let probe_duration = duration_sec.clamp(0.6, 4.0);
    let output = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("info")
        .arg("-ss")
        .arg(format!("{start_sec:.3}"))
        .arg("-t")
        .arg(format!("{probe_duration:.3}"))
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg("fps=2,scale=480:-2,facedetect=mode=accurate:resize=320")
        .arg("-frames:v")
        .arg("8")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .context("running ffmpeg face presence probe")?;

    if !output.status.success() {
        return Ok(0.0);
    }

    let stderr =
        String::from_utf8(output.stderr).context("ffmpeg face presence output not utf8")?;
    let detections = face_detection_count(&stderr) as f32;
    Ok((detections / 4.0).clamp(0.0, 1.0))
}

fn detect_face_center(video_path: &Path, width: u32) -> Result<Option<u32>> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let scaled_width = width.clamp(160, 640);
    let scale_ratio = width as f32 / scaled_width as f32;
    let filter = format!("fps=2,scale={scaled_width}:-2,facedetect=mode=accurate:resize=320");
    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("info")
        .arg("-t")
        .arg("12")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(&filter)
        .arg("-frames:v")
        .arg("24")
        .arg("-f")
        .arg("null")
        .arg("-");
    let output = match command_output_checked(&mut cmd, "running ffmpeg face detection") {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };

    let stderr =
        String::from_utf8(output.stderr).context("ffmpeg face detection output not utf8")?;
    let mut centers = parse_face_centers(&stderr, scale_ratio);
    if centers.is_empty() {
        return Ok(None);
    }

    centers.sort_unstable();
    Ok(Some(centers[centers.len() / 2]))
}

fn detect_face_crop_plan(video_path: &Path, width: u32, height: u32) -> Result<Option<CropPlan>> {
    let target_width = ((height as f32) * 9.0 / 16.0).round() as u32;
    let crop_width = width.min(target_width.max(1));
    if crop_width >= width {
        return Ok(None);
    }

    let Some(face_center) = detect_face_center(video_path, width)? else {
        return Ok(None);
    };

    Ok(Some(CropPlan {
        width: crop_width,
        height,
        x: clamp_crop_axis(face_center as i64, crop_width, width),
        y: 0,
    }))
}

fn detect_subject_crop_plan(
    video_path: &Path,
    width: u32,
    height: u32,
) -> Result<Option<CropPlan>> {
    if let Some(face_plan) = detect_face_crop_plan(video_path, width, height)? {
        return Ok(Some(face_plan));
    }

    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("info")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg("cropdetect=limit=24:round=2:reset=0")
        .arg("-frames:v")
        .arg("180")
        .arg("-f")
        .arg("null")
        .arg("-");
    let output =
        command_output_checked(&mut cmd, "running ffmpeg cropdetect for subject-aware crop")?;

    let stderr = String::from_utf8(output.stderr).context("ffmpeg cropdetect output not utf8")?;
    let target_width = ((height as f32) * 9.0 / 16.0).round() as u32;
    let crop_width = width.min(target_width.max(1));
    if crop_width >= width {
        return Ok(None);
    }

    let mut cropdetect_centers = Vec::new();
    for capture in cropdetect_regex().captures_iter(&stderr) {
        let detected_width = capture[1].parse::<u32>().unwrap_or(width);
        let detected_x = capture[3].parse::<u32>().unwrap_or(0);
        if detected_width == 0 {
            continue;
        }
        cropdetect_centers.push(detected_x + detected_width / 2);
    }

    let activity_center = detect_activity_center(video_path, width, height, crop_width)?;

    if cropdetect_centers.is_empty() && activity_center.is_none() {
        return Ok(None);
    }

    let resolved_center = if cropdetect_centers.is_empty() {
        activity_center.unwrap_or(width / 2)
    } else {
        cropdetect_centers.sort_unstable();
        let cropdetect_center = cropdetect_centers[cropdetect_centers.len() / 2];
        if let Some(activity_center) = activity_center {
            (((cropdetect_center as f32) * 0.4) + ((activity_center as f32) * 0.6)).round() as u32
        } else {
            cropdetect_center
        }
    };

    Ok(Some(CropPlan {
        width: crop_width,
        height,
        x: clamp_crop_axis(resolved_center as i64, crop_width, width),
        y: 0,
    }))
}

fn crop_to_vertical(video_path: &Path, output_path: &Path, crop_mode: &str) -> Result<()> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let (width, height) = probe_video_dimensions(video_path)?;
    let crop_plan = match crop_mode {
        "face" => detect_face_crop_plan(video_path, width, height)?
            .unwrap_or_else(|| plan_center_crop(width, height)),
        "subject" => detect_subject_crop_plan(video_path, width, height)?
            .unwrap_or_else(|| plan_center_crop(width, height)),
        _ => plan_center_crop(width, height),
    };

    let status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!(
            "crop={}:{}:{}:{}",
            crop_plan.width, crop_plan.height, crop_plan.x, crop_plan.y
        ))
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("fast")
        .arg("-crf")
        .arg("23")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("128k")
        .arg(output_path)
        .status()
        .context("running ffmpeg for crop")?;
    if !status.success() {
        anyhow::bail!("Crop failed");
    }
    Ok(())
}

fn transcribe_with_local_whisper(audio_path: &Path, srt_path: &Path, cli: &Cli) -> Result<()> {
    if cli.whisper_mode == "python" {
        println!(
            "{}",
            "Using Whisper via Python (python -m whisper)...".green()
        );
        let python = which("python").context("python not found in PATH")?;
        let status = Command::new(&python)
            .arg("-m")
            .arg("whisper")
            .arg(audio_path)
            .arg("--model")
            .arg(&cli.whisper_model)
            .arg("--language")
            .arg("en")
            .arg("--word_timestamps")
            .arg("True")
            .arg("--output_format")
            .arg("srt")
            .arg("--output_dir")
            .arg(audio_path.parent().context("audio path has no parent")?)
            .status()
            .context("running python whisper")?;
        if !status.success() {
            anyhow::bail!("Whisper transcription failed");
        }
    } else {
        println!(
            "{}",
            "Using binary Whisper. For best word timing support, prefer --whisper-mode python."
                .yellow()
        );
        let whisper = which("whisper")
            .context("whisper not found. Install with: pip install openai-whisper")?;
        let status = Command::new(&whisper)
            .arg(audio_path)
            .arg("--model")
            .arg(&cli.whisper_model)
            .arg("--language")
            .arg("en")
            .arg("--output_format")
            .arg("srt")
            .arg("--output_dir")
            .arg(audio_path.parent().context("audio path has no parent")?)
            .status()
            .context("running whisper binary")?;
        if !status.success() {
            anyhow::bail!("Whisper binary transcription failed");
        }
    }

    let generated = audio_path.with_extension("srt");
    if !generated.exists() {
        anyhow::bail!("Whisper did not produce expected SRT at {:?}", generated);
    }
    std::fs::rename(&generated, srt_path).context("rename generated srt")?;
    Ok(())
}

fn parse_openai_transcription_srt(response: &Value) -> Option<String> {
    response
        .get("text")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn transcribe_with_openai(audio_path: &Path, srt_path: &Path, cli: &Cli) -> Result<()> {
    println!("{}", "Using OpenAI cloud transcription...".green());
    let api_key = std::env::var(&cli.transcription_api_key_env)
        .with_context(|| format!("missing API key env var {}", cli.transcription_api_key_env))?;
    let curl = which("curl").context("curl not found in PATH")?;
    let output = Command::new(&curl)
        .arg("-sS")
        .arg("-f")
        .arg("https://api.openai.com/v1/audio/transcriptions")
        .arg("-H")
        .arg(format!("Authorization: Bearer {api_key}"))
        .arg("-F")
        .arg(format!("file=@{}", audio_path.display()))
        .arg("-F")
        .arg(format!("model={}", cli.cloud_transcription_model))
        .arg("-F")
        .arg("response_format=srt")
        .output()
        .context("running OpenAI transcription request")?;

    if !output.status.success() {
        anyhow::bail!(
            "OpenAI transcription failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let body =
        String::from_utf8(output.stdout).context("OpenAI transcription response not utf8")?;
    let content = if body.trim_start().starts_with('{') {
        let parsed: Value =
            serde_json::from_str(&body).context("parse OpenAI transcription JSON")?;
        parse_openai_transcription_srt(&parsed)
            .or_else(|| parsed.as_str().map(|value| value.to_string()))
            .context("OpenAI transcription response did not include SRT text")?
    } else {
        body
    };
    std::fs::write(srt_path, content)
        .with_context(|| format!("write cloud SRT {}", srt_path.display()))?;
    Ok(())
}

fn transcribe_full_audio(audio_path: &Path, srt_path: &Path, cli: &Cli) -> Result<()> {
    match cli.transcription_provider.as_str() {
        "local" => transcribe_with_local_whisper(audio_path, srt_path, cli),
        "openai" => transcribe_with_openai(audio_path, srt_path, cli),
        other => anyhow::bail!("Unsupported transcription provider: {other}"),
    }
}

fn detect_scene_changes(video_path: &Path, threshold: f64) -> Result<Vec<f32>> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("info")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("select='gt(scene,{threshold})',showinfo"))
        .arg("-f")
        .arg("null")
        .arg("-");
    let output = command_output_checked(&mut cmd, "running ffmpeg for scene detection")?;

    let stderr = String::from_utf8(output.stderr).context("ffmpeg stderr not utf8")?;
    let mut timestamps = Vec::new();
    for line in stderr.lines() {
        if let Some(capture) = scene_pts_regex().captures(line) {
            if let Ok(timestamp) = capture[1].parse::<f32>() {
                timestamps.push(timestamp);
            }
        }
    }
    Ok(timestamps)
}

fn now_strings(mode: &str) -> (String, String) {
    match mode {
        "local" => {
            let dt = Local::now();
            (dt.to_rfc3339(), dt.format("%d %b %Y, %H:%M").to_string())
        }
        _ => {
            let dt = Utc::now();
            (dt.to_rfc3339(), dt.format("%d %b %Y, %H:%M").to_string())
        }
    }
}

fn parse_chat_log(path: &Path) -> Result<Vec<f32>> {
    let file = File::open(path).with_context(|| format!("open chat log {}", path.display()))?;
    let reader = BufReader::new(file);
    let (time_pattern, numeric_pattern) = chat_timestamp_regexes();

    let mut timestamps = Vec::new();
    for line in reader.lines() {
        let line = line.context("read chat log line")?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let timestamp = if let Some(capture) = time_pattern.captures(trimmed) {
            parse_clock_timestamp(&capture[1])
        } else if let Some(capture) = numeric_pattern.captures(trimmed) {
            capture[1].parse::<f32>().ok()
        } else {
            let first_field = trimmed.split([',', '\t']).next().unwrap_or("");
            parse_clock_timestamp(first_field).or_else(|| first_field.parse::<f32>().ok())
        };

        if let Some(timestamp) = timestamp {
            timestamps.push(timestamp.max(0.0));
        }
    }

    Ok(timestamps)
}

fn apply_motion_scores(windows: &mut [WindowMetrics], timestamps: &[f32]) {
    let mut counts = vec![0.0f32; windows.len()];
    for (idx, window) in windows.iter().enumerate() {
        for timestamp in timestamps {
            let distance = (window.start_sec - *timestamp).abs();
            if distance <= 3.0 {
                counts[idx] += 1.0 - (distance / 3.0);
            }
        }
    }
    let normalized = normalize_scores(&counts);
    for (window, score) in windows.iter_mut().zip(normalized) {
        window.motion = score;
    }
}

fn apply_chat_scores(windows: &mut [WindowMetrics], timestamps: &[f32]) {
    let mut counts = vec![0.0f32; windows.len()];
    for timestamp in timestamps {
        let idx = *timestamp as usize;
        if idx < counts.len() {
            counts[idx] += 1.0;
        }
    }
    let normalized = normalize_scores(&counts);
    for (window, score) in windows.iter_mut().zip(normalized) {
        window.chat = score;
    }
}

fn score_windows(windows: &mut [WindowMetrics], cli: &Cli) {
    for window in windows {
        let laughter = if cli.laughter { window.laughter } else { 0.0 };
        let motion = if cli.motion { window.motion } else { 0.0 };
        let chat = if cli.chat_log.is_some() {
            window.chat
        } else {
            0.0
        };
        let semantic_bonus = window.semantic * 0.35;
        let speech_bonus = window.speech_confidence * 0.15;
        window.score = (window.energy * cli.energy_weight)
            + (motion * cli.motion_weight)
            + (laughter * cli.laughter_weight)
            + (chat * cli.chat_weight)
            + (window.transcript * cli.transcript_weight)
            + (window.hook * cli.hook_weight)
            + semantic_bonus
            + speech_bonus;
    }
}

fn build_tasks(windows: &[WindowMetrics], min_duration: f32, num_clips: usize) -> Vec<ClipTask> {
    let mut ranked = windows.to_vec();
    ranked.sort_by(|lhs, rhs| rhs.score.partial_cmp(&lhs.score).unwrap_or(Ordering::Equal));

    let mut selected = Vec::new();
    let min_gap = min_duration.max(1.0);
    for candidate in ranked {
        let overlaps = selected
            .iter()
            .any(|existing: &ClipTask| (existing.start_sec - candidate.start_sec).abs() < min_gap);
        if overlaps {
            continue;
        }

        let duplicate = selected.iter().any(|existing: &ClipTask| {
            transcript_similarity(
                &existing.metrics.transcript_text,
                &candidate.transcript_text,
            ) >= 0.65
        });
        if duplicate {
            continue;
        }

        let clip_id = selected.len() + 1;
        selected.push(ClipTask {
            clip_id,
            start_sec: candidate.start_sec,
            end_sec: candidate.start_sec + min_duration,
            metrics: candidate,
        });
        if selected.len() >= num_clips {
            break;
        }
    }
    selected
}

fn rerank_selected_tasks(tasks: &mut [ClipTask]) {
    for task in tasks.iter_mut() {
        let face_bonus = task.metrics.face_score * 0.18;
        let semantic_bonus = task.metrics.semantic * 0.20;
        let speech_bonus = task.metrics.speech_confidence * 0.08;
        task.metrics.score += face_bonus + semantic_bonus + speech_bonus;
    }

    tasks.sort_by(|lhs, rhs| {
        rhs.metrics
            .score
            .partial_cmp(&lhs.metrics.score)
            .unwrap_or(Ordering::Equal)
    });
    for (idx, task) in tasks.iter_mut().enumerate() {
        task.clip_id = idx + 1;
    }
}

fn enrich_selected_tasks_with_multimodal_context(video_path: &Path, tasks: &mut [ClipTask]) {
    for task in tasks.iter_mut() {
        if let Ok(face_score) = estimate_face_presence_for_range(
            video_path,
            task.start_sec,
            task.end_sec - task.start_sec,
        ) {
            task.metrics.face_score = face_score;
        }
    }
    rerank_selected_tasks(tasks);
}

fn rerank_candidate_tasks(
    ai_options: &AiOptions,
    transcript_entries: &[TranscriptEntry],
    tasks: &mut [ClipTask],
) -> Result<()> {
    if tasks.is_empty() || transcript_entries.is_empty() {
        return Ok(());
    }

    let transcript_segments = transcript_entries
        .iter()
        .map(|entry| entry.text.clone())
        .collect::<Vec<_>>();
    let contexts = build_ai_clip_contexts(tasks, transcript_entries);
    let reranked = rerank_candidates(ai_options, &transcript_segments, &contexts)?;
    let score_map: HashMap<usize, f32> = reranked
        .into_iter()
        .map(|item| (item.clip_id, item.score))
        .collect();

    for task in tasks.iter_mut() {
        let rerank_score = score_map.get(&task.clip_id).copied().unwrap_or(0.0);
        task.metrics.score += rerank_score * 0.55;
        task.metrics.semantic =
            ((task.metrics.semantic * 0.7) + (rerank_score * 0.3)).clamp(0.0, 1.0);
    }

    tasks.sort_by(|lhs, rhs| {
        rhs.metrics
            .score
            .partial_cmp(&lhs.metrics.score)
            .unwrap_or(Ordering::Equal)
    });
    for (idx, task) in tasks.iter_mut().enumerate() {
        task.clip_id = idx + 1;
    }
    Ok(())
}

fn subtitle_style_from_cli(cli: &Cli) -> SubtitleStyle {
    let classic_defaults = SubtitleStyle {
        font: "Monospace".to_string(),
        size: 24,
        color: "&H00FFFFFF".to_string(),
        highlight_color: "&H0000F6FF".to_string(),
        outline_color: "&H00000000".to_string(),
        back_color: "&H64000000".to_string(),
        outline: 2,
        shadow: 0,
        border_style: 1,
        bold: false,
        alignment: 2,
        margin_v: 28,
    };
    let mut style = match cli.subtitle_preset.as_str() {
        "legendary" => SubtitleStyle {
            font: "Arial Black".to_string(),
            size: 34,
            color: "&H0000F6FF".to_string(),
            highlight_color: "&H0000FFFF".to_string(),
            outline_color: "&H00000000".to_string(),
            back_color: "&H96000000".to_string(),
            outline: 4,
            shadow: 1,
            border_style: 3,
            bold: true,
            alignment: 2,
            margin_v: 42,
        },
        "creator_pro" => SubtitleStyle {
            font: "Arial Black".to_string(),
            size: 30,
            color: "&H00FFFFFF".to_string(),
            highlight_color: "&H0000F6FF".to_string(),
            outline_color: "&H00000000".to_string(),
            back_color: "&H32000000".to_string(),
            outline: 3,
            shadow: 0,
            border_style: 1,
            bold: true,
            alignment: 2,
            margin_v: 84,
        },
        "creator_neon" => SubtitleStyle {
            font: "Arial Black".to_string(),
            size: 30,
            color: "&H00FFFFFF".to_string(),
            highlight_color: "&H0000FF66".to_string(),
            outline_color: "&H00000000".to_string(),
            back_color: "&H36000000".to_string(),
            outline: 4,
            shadow: 0,
            border_style: 1,
            bold: true,
            alignment: 2,
            margin_v: 84,
        },
        "creator_minimal" => SubtitleStyle {
            font: "Arial".to_string(),
            size: 28,
            color: "&H00FFFFFF".to_string(),
            highlight_color: "&H0000F6FF".to_string(),
            outline_color: "&H00000000".to_string(),
            back_color: "&H24000000".to_string(),
            outline: 2,
            shadow: 0,
            border_style: 1,
            bold: true,
            alignment: 2,
            margin_v: 84,
        },
        "creator_bold" => SubtitleStyle {
            font: "Impact".to_string(),
            size: 32,
            color: "&H00FFFFFF".to_string(),
            highlight_color: "&H0000F6FF".to_string(),
            outline_color: "&H00000000".to_string(),
            back_color: "&H42000000".to_string(),
            outline: 4,
            shadow: 0,
            border_style: 1,
            bold: true,
            alignment: 2,
            margin_v: 88,
        },
        _ => classic_defaults.clone(),
    };

    if cli.subtitle_preset == "classic" || cli.subtitle_font != classic_defaults.font {
        style.font = cli.subtitle_font.clone();
    }
    if cli.subtitle_preset == "classic" || cli.subtitle_size != classic_defaults.size {
        style.size = cli.subtitle_size;
    }
    if cli.subtitle_preset == "classic" || cli.subtitle_color != classic_defaults.color {
        style.color = cli.subtitle_color.clone();
    }
    if cli.subtitle_preset == "classic"
        || cli.subtitle_highlight_color != classic_defaults.highlight_color
    {
        style.highlight_color = cli.subtitle_highlight_color.clone();
    }
    if cli.subtitle_preset == "classic"
        || cli.subtitle_outline_color != classic_defaults.outline_color
    {
        style.outline_color = cli.subtitle_outline_color.clone();
    }
    if cli.subtitle_preset == "classic" || cli.subtitle_back_color != classic_defaults.back_color {
        style.back_color = cli.subtitle_back_color.clone();
    }
    if cli.subtitle_preset == "classic" || cli.subtitle_outline != classic_defaults.outline {
        style.outline = cli.subtitle_outline;
    }
    if cli.subtitle_preset == "classic" || cli.subtitle_shadow != classic_defaults.shadow {
        style.shadow = cli.subtitle_shadow;
    }
    if cli.subtitle_preset == "classic"
        || cli.subtitle_border_style != classic_defaults.border_style
    {
        style.border_style = cli.subtitle_border_style;
    }
    if cli.subtitle_preset == "classic" || cli.subtitle_bold != classic_defaults.bold {
        style.bold = cli.subtitle_bold;
    }
    if cli.subtitle_preset == "classic" || cli.subtitle_alignment != classic_defaults.alignment {
        style.alignment = cli.subtitle_alignment;
    }
    if cli.subtitle_preset == "classic" || cli.subtitle_margin_v != classic_defaults.margin_v {
        style.margin_v = cli.subtitle_margin_v;
    }
    style
}

fn merge_subtitle_style(default_style: &SubtitleStyle, patch: SubtitleStylePatch) -> SubtitleStyle {
    SubtitleStyle {
        font: patch.font.unwrap_or_else(|| default_style.font.clone()),
        size: patch.size.unwrap_or(default_style.size),
        color: patch.color.unwrap_or_else(|| default_style.color.clone()),
        highlight_color: patch
            .highlight_color
            .unwrap_or_else(|| default_style.highlight_color.clone()),
        outline_color: patch
            .outline_color
            .unwrap_or_else(|| default_style.outline_color.clone()),
        back_color: patch
            .back_color
            .unwrap_or_else(|| default_style.back_color.clone()),
        outline: patch.outline.unwrap_or(default_style.outline),
        shadow: patch.shadow.unwrap_or(default_style.shadow),
        border_style: patch.border_style.unwrap_or(default_style.border_style),
        bold: patch.bold.unwrap_or(default_style.bold),
        alignment: patch.alignment.unwrap_or(default_style.alignment),
        margin_v: patch.margin_v.unwrap_or(default_style.margin_v),
    }
}

fn subtitle_animation_preset(value: &str) -> SubtitleAnimationPreset {
    match value {
        "karaoke" => SubtitleAnimationPreset::Karaoke,
        "emphasis" => SubtitleAnimationPreset::Emphasis,
        "impact" => SubtitleAnimationPreset::Impact,
        "pulse" => SubtitleAnimationPreset::Pulse,
        "creator_pro" => SubtitleAnimationPreset::CreatorPro,
        _ => SubtitleAnimationPreset::None,
    }
}

fn load_subtitle_styles(cli: &Cli) -> Result<HashMap<usize, SubtitleStyle>> {
    let default_style = subtitle_style_from_cli(cli);

    let mut styles = HashMap::new();
    if !cli.captions {
        return Ok(styles);
    }

    if let Some(path) = &cli.subtitle_style_map {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read subtitle style map {}", path.display()))?;
        let patches: HashMap<String, SubtitleStylePatch> =
            serde_json::from_str(&raw).context("parse subtitle style map json")?;
        for (key, patch) in patches {
            let clip_id = key
                .parse::<usize>()
                .with_context(|| format!("invalid clip id '{key}' in subtitle style map"))?;
            styles.insert(clip_id, merge_subtitle_style(&default_style, patch));
        }
    }

    styles.entry(0).or_insert(default_style);
    Ok(styles)
}

fn subtitle_style_for_clip(
    styles: &HashMap<usize, SubtitleStyle>,
    clip_id: usize,
) -> Option<&SubtitleStyle> {
    styles.get(&clip_id).or_else(|| styles.get(&0))
}

fn ai_options_from_cli(cli: &Cli) -> AiOptions {
    AiOptions {
        enabled: cli.llm_enable,
        provider: cli.llm_provider.clone(),
        model: cli.llm_model.clone(),
        api_key_env: cli.llm_api_key_env.clone(),
        subtitle_preset: cli.subtitle_preset.clone(),
    }
}

fn build_ai_clip_contexts(
    tasks: &[ClipTask],
    transcript_entries: &[TranscriptEntry],
) -> Vec<AiClipContext> {
    tasks
        .iter()
        .map(|task| AiClipContext {
            clip_id: task.clip_id,
            start_sec: task.start_sec,
            end_sec: task.end_sec,
            score: task.metrics.score,
            energy: task.metrics.energy,
            laughter: task.metrics.laughter,
            motion: task.metrics.motion,
            chat: task.metrics.chat,
            transcript_excerpt: transcript_excerpt_for_range(
                transcript_entries,
                task.start_sec,
                task.end_sec,
                18,
            ),
            readability_score: readability_from_density_and_confidence(
                transcript_density_for_range(transcript_entries, task.start_sec, task.end_sec),
                task.metrics.speech_confidence,
            ),
            semantic_score: task.metrics.semantic,
            speech_confidence: task.metrics.speech_confidence,
            face_score: task.metrics.face_score,
        })
        .collect()
}

fn resolve_llm_output_path(cli: &Cli) -> PathBuf {
    cli.llm_output
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&cli.output_dir).join("ai_storyboard.json"))
}

fn resolve_export_bundle_path(cli: &Cli) -> PathBuf {
    cli.export_bundle_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&cli.output_dir).join("export_bundle.json"))
}

fn resolve_proof_report_path(cli: &Cli) -> PathBuf {
    cli.proof_report_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&cli.output_dir).join("proof_report.md"))
}

fn resolve_thumbnails_dir(cli: &Cli) -> PathBuf {
    cli.thumbnails_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&cli.output_dir).join("thumbnails"))
}

fn resolve_thumbnail_collage_path(cli: &Cli) -> PathBuf {
    cli.thumbnail_collage_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_thumbnails_dir(cli).join("collage.jpg"))
}

fn build_platform_exports(
    task: &ClipTask,
    ai_plan: Option<&viralclip_swarm::ai::AiClipPlan>,
) -> Vec<PlatformExport> {
    let fallback_title = format!("Moment #{:02}", task.clip_id);
    let base_title = ai_plan
        .map(|plan| plan.title.clone())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(fallback_title);
    let base_caption = ai_plan
        .map(|plan| plan.social_caption.clone())
        .filter(|caption| !caption.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "Clip {} starts at {:.1}s with a {:.2} score.",
                task.clip_id, task.start_sec, task.metrics.score
            )
        });

    vec![
        PlatformExport {
            platform: "youtube_shorts".to_string(),
            title: format!("{} | Shorts", base_title),
            caption: ai_plan
                .map(|plan| plan.youtube_shorts_caption.clone())
                .filter(|caption| !caption.trim().is_empty())
                .unwrap_or_else(|| format!("{} #shorts", base_caption)),
            hashtags: vec![
                "shorts".to_string(),
                "viralclip".to_string(),
                "creator".to_string(),
            ],
            aspect_ratio: "9:16".to_string(),
            recommended_duration_sec: (task.end_sec - task.start_sec).max(0.0).round() as u32,
        },
        PlatformExport {
            platform: "tiktok".to_string(),
            title: base_title.clone(),
            caption: ai_plan
                .map(|plan| plan.tiktok_caption.clone())
                .filter(|caption| !caption.trim().is_empty())
                .unwrap_or_else(|| format!("{} #tiktok #fyp", base_caption)),
            hashtags: vec!["tiktok".to_string(), "fyp".to_string(), "viral".to_string()],
            aspect_ratio: "9:16".to_string(),
            recommended_duration_sec: (task.end_sec - task.start_sec).max(0.0).round() as u32,
        },
        PlatformExport {
            platform: "instagram_reels".to_string(),
            title: format!("{} | Reel", base_title),
            caption: ai_plan
                .map(|plan| plan.instagram_reels_caption.clone())
                .filter(|caption| !caption.trim().is_empty())
                .unwrap_or_else(|| format!("{} #reels", base_caption)),
            hashtags: vec![
                "reels".to_string(),
                "creator".to_string(),
                "clips".to_string(),
            ],
            aspect_ratio: "9:16".to_string(),
            recommended_duration_sec: (task.end_sec - task.start_sec).max(0.0).round() as u32,
        },
    ]
}

fn write_export_bundle(
    path: &Path,
    timestamp_mode: &str,
    tasks: &[ClipTask],
    storyboard: Option<&viralclip_swarm::ai::AiStoryboard>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create export bundle directory {}", parent.display()))?;
        }
    }

    let plan_map: HashMap<usize, &viralclip_swarm::ai::AiClipPlan> = storyboard
        .map(|storyboard| {
            storyboard
                .clips
                .iter()
                .map(|plan| (plan.clip_id, plan))
                .collect()
        })
        .unwrap_or_default();
    let (generated_at, generated_at_human) = now_strings(timestamp_mode);

    let bundle = ExportBundle {
        generated_at,
        generated_at_human,
        clips: tasks
            .iter()
            .map(|task| ExportClipBundle {
                clip_id: task.clip_id,
                file_name: format!("clip_{}.mp4", task.clip_id),
                start_sec: task.start_sec,
                duration_sec: task.end_sec - task.start_sec,
                total_score: task.metrics.score,
                transcript_score: task.metrics.transcript,
                hook_score: task.metrics.hook,
                readability_score: readability_from_density_and_confidence(
                    task.metrics.transcript_density,
                    task.metrics.speech_confidence,
                ),
                platforms: build_platform_exports(task, plan_map.get(&task.clip_id).copied()),
            })
            .collect(),
    };

    let raw = serde_json::to_string_pretty(&bundle).context("serialize export bundle")?;
    std::fs::write(path, raw).with_context(|| format!("write export bundle {}", path.display()))?;
    Ok(())
}

fn build_proof_report(cli: &Cli, benchmark: &BenchmarkLog) -> ProofReport {
    let successful: Vec<&ClipTiming> = benchmark.clips.iter().filter(|clip| clip.success).collect();
    let success_rate = if benchmark.clips.is_empty() {
        0.0
    } else {
        successful.len() as f32 / benchmark.clips.len() as f32
    };
    let average_total_score = if successful.is_empty() {
        0.0
    } else {
        successful.iter().map(|clip| clip.total_score).sum::<f32>() / successful.len() as f32
    };
    let average_readability_score = if successful.is_empty() {
        0.0
    } else {
        successful
            .iter()
            .map(|clip| clip.readability_score)
            .sum::<f32>()
            / successful.len() as f32
    };
    let average_extract_ms = if successful.is_empty() {
        0.0
    } else {
        successful
            .iter()
            .map(|clip| clip.extract_ms as f32)
            .sum::<f32>()
            / successful.len() as f32
    };
    let average_total_ms = if successful.is_empty() {
        0.0
    } else {
        successful
            .iter()
            .map(|clip| clip.total_ms as f32)
            .sum::<f32>()
            / successful.len() as f32
    };
    let best_clip = successful
        .iter()
        .max_by(|lhs, rhs| {
            lhs.total_score
                .partial_cmp(&rhs.total_score)
                .unwrap_or(Ordering::Equal)
        })
        .copied();
    let (generated_at, generated_at_human) = now_strings(&cli.timestamp_mode);

    let mut highlights = Vec::new();
    highlights.push(format!(
        "Run processed {} clip candidates with {:.0}% success.",
        benchmark.summary.total_clips,
        success_rate * 100.0
    ));
    if let Some(best) = best_clip {
        highlights.push(format!(
            "Best clip was #{} with total score {:.3} and readability {:.2}.",
            best.clip_id, best.total_score, best.readability_score
        ));
    }
    highlights.push(format!(
        "Average successful clip latency was {:.0} ms end-to-end.",
        average_total_ms
    ));
    highlights.push(format!(
        "Transcript-aware readability averaged {:.2}.",
        average_readability_score
    ));

    ProofReport {
        generated_at,
        generated_at_human,
        benchmark_path: cli.csv_path.clone(),
        output_dir: cli.output_dir.clone(),
        success_rate,
        average_total_score,
        average_readability_score,
        average_extract_ms,
        average_total_ms,
        best_clip_id: best_clip.map(|clip| clip.clip_id),
        best_clip_score: best_clip.map(|clip| clip.total_score).unwrap_or(0.0),
        highlights,
    }
}

fn write_proof_report(cli: &Cli, benchmark: &BenchmarkLog) -> Result<()> {
    let report = build_proof_report(cli, benchmark);
    let path = resolve_proof_report_path(cli);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create proof report directory {}", parent.display()))?;
        }
    }

    let mut body = String::new();
    body.push_str("# ViralClip Swarm Proof Report\n\n");
    body.push_str(&format!(
        "- Generated: {} ({})\n- Benchmark path: `{}`\n- Output dir: `{}`\n\n",
        report.generated_at, report.generated_at_human, report.benchmark_path, report.output_dir
    ));
    body.push_str("## Summary\n\n");
    body.push_str(&format!(
        "- Success rate: {:.0}%\n- Average total score: {:.3}\n- Average readability score: {:.2}\n- Average extract time: {:.0} ms\n- Average total clip time: {:.0} ms\n- Best clip: {}\n\n",
        report.success_rate * 100.0,
        report.average_total_score,
        report.average_readability_score,
        report.average_extract_ms,
        report.average_total_ms,
        report
            .best_clip_id
            .map(|id| format!("#{} ({:.3})", id, report.best_clip_score))
            .unwrap_or_else(|| "none".to_string())
    ));
    body.push_str("## Highlights\n\n");
    for line in &report.highlights {
        body.push_str(&format!("- {}\n", line));
    }
    body.push_str("\n## Clip Table\n\n");
    body.push_str("| Clip | Success | Score | Readability | Extract ms | Total ms | Error |\n");
    body.push_str("|---|---:|---:|---:|---:|---:|---|\n");
    for clip in &benchmark.clips {
        body.push_str(&format!(
            "| {} | {} | {:.3} | {:.2} | {} | {} | {} |\n",
            clip.clip_id,
            if clip.success { "yes" } else { "no" },
            clip.total_score,
            clip.readability_score,
            clip.extract_ms,
            clip.total_ms,
            if clip.error.is_empty() {
                "-"
            } else {
                &clip.error
            }
        ));
    }

    std::fs::write(&path, body)
        .with_context(|| format!("write proof report {}", path.display()))?;
    Ok(())
}

fn escape_drawtext_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
        .replace('%', "\\%")
        .replace(',', "\\,")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn default_drawtext_font() -> Option<String> {
    let candidates = [
        PathBuf::from("C:/Windows/Fonts/arialbd.ttf"),
        PathBuf::from("C:/Windows/Fonts/arial.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ];

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| {
            path.to_string_lossy()
                .replace('\\', "/")
                .replace(':', "\\:")
        })
}

fn extract_single_frame_face_score(video_path: &Path, capture_at: f32) -> Result<f32> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("info")
        .arg("-ss")
        .arg(format!("{capture_at:.3}"))
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg("scale=480:-2,facedetect=mode=accurate:resize=320")
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("null")
        .arg("-");
    let output = match command_output_checked(&mut cmd, "running ffmpeg single-frame face probe") {
        Ok(output) => output,
        Err(_) => return Ok(0.0),
    };

    let stderr =
        String::from_utf8(output.stderr).context("single-frame face probe output not utf8")?;
    Ok((face_detection_count(&stderr) as f32).clamp(0.0, 2.0) / 2.0)
}

fn select_thumbnail_capture_time(video_path: &Path) -> Result<f32> {
    let duration = probe_video_duration(video_path).unwrap_or(0.0);
    if duration <= 0.3 {
        return Ok(0.0);
    }

    let candidates = [0.18f32, 0.34, 0.52, 0.72];
    let mut best = ((duration * 0.45).min((duration - 0.1).max(0.0)), 0.0f32);
    for (idx, ratio) in candidates.iter().enumerate() {
        let capture_at = (duration * ratio).min((duration - 0.1).max(0.0));
        let face_score = extract_single_frame_face_score(video_path, capture_at).unwrap_or(0.0);
        let timing_bias = match idx {
            0 => 0.06,
            1 => 0.10,
            2 => 0.14,
            _ => 0.08,
        };
        let score = face_score + timing_bias;
        if score > best.1 {
            best = (capture_at, score);
        }
    }
    Ok(best.0)
}

fn extract_thumbnail_with_style(
    video_path: &Path,
    output_path: &Path,
    style: &str,
    text: Option<&str>,
) -> Result<()> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let capture_at = select_thumbnail_capture_time(video_path).unwrap_or(0.0);
    let base_filter = match style {
        "cinematic" => {
            "scale=720:-2,eq=saturation=1.18:contrast=1.08,drawbox=x=0:y=0:w=iw:h=ih:color=black@0.25:t=24"
                .to_string()
        }
        "framed" => {
            "scale=720:-2,pad=iw+40:ih+80:20:20:color=0x111111,drawbox=x=8:y=8:w=iw-16:h=ih-16:color=0x00d7ff@0.9:t=10"
                .to_string()
        }
        _ => "scale=720:-2".to_string(),
    };

    let filter_with_text = text.map(str::trim).filter(|text| !text.is_empty()).map(|text| {
        let escaped = escape_drawtext_text(text);
        let font_clause = default_drawtext_font()
            .map(|font| format!(":fontfile='{font}'"))
            .unwrap_or_default();
        format!(
            "{base_filter},drawbox=x=40:y=ih-150:w=iw-80:h=92:color=black@0.46:t=fill,drawtext=text='{escaped}'{font_clause}:x=(w-text_w)/2:y=h-122:fontcolor=white:fontsize=40:borderw=3:bordercolor=black"
        )
    });

    for filter in filter_with_text
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(base_filter.as_str()))
    {
        let status = Command::new(&ffmpeg)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-ss")
            .arg(format!("{capture_at:.3}"))
            .arg("-i")
            .arg(video_path)
            .arg("-vf")
            .arg(filter)
            .arg("-frames:v")
            .arg("1")
            .arg("-q:v")
            .arg("2")
            .arg(output_path)
            .status()
            .with_context(|| format!("extract thumbnail from {}", video_path.display()))?;

        if status.success() {
            return Ok(());
        }
    }

    anyhow::bail!(
        "ffmpeg thumbnail extraction failed for {}",
        video_path.display()
    );
}

fn write_thumbnail_collage(cli: &Cli, images: &[PathBuf]) -> Result<()> {
    if images.is_empty() {
        return Ok(());
    }

    let collage_path = resolve_thumbnail_collage_path(cli);
    if let Some(parent) = collage_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create collage directory {}", parent.display()))?;
        }
    }

    if images.len() == 1 {
        std::fs::copy(&images[0], &collage_path)
            .with_context(|| format!("copy single thumbnail collage {}", collage_path.display()))?;
        return Ok(());
    }

    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y");

    let selected: Vec<&PathBuf> = images.iter().take(4).collect();
    for image in &selected {
        cmd.arg("-i").arg(image);
    }

    let filter = match selected.len() {
        2 => "[0:v][1:v]hstack=inputs=2[v]".to_string(),
        3 => "[0:v][1:v]hstack=inputs=2[top];[2:v]pad=iw*2:ih:iw/2:0:black[bottom];[top][bottom]vstack=inputs=2[v]".to_string(),
        _ => "[0:v][1:v]hstack=inputs=2[top];[2:v][3:v]hstack=inputs=2[bottom];[top][bottom]vstack=inputs=2[v]".to_string(),
    };

    let status = cmd
        .arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[v]")
        .arg("-frames:v")
        .arg("1")
        .arg(&collage_path)
        .status()
        .context("generate thumbnail collage with ffmpeg")?;

    if !status.success() {
        anyhow::bail!("ffmpeg thumbnail collage generation failed");
    }

    Ok(())
}

fn write_thumbnails(
    cli: &Cli,
    benchmark: &BenchmarkLog,
    storyboard: Option<&viralclip_swarm::ai::AiStoryboard>,
) -> Result<()> {
    let dir = resolve_thumbnails_dir(cli);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create thumbnails dir {}", dir.display()))?;
    let mut images = Vec::new();
    let plan_map: HashMap<usize, &viralclip_swarm::ai::AiClipPlan> = storyboard
        .map(|storyboard| {
            storyboard
                .clips
                .iter()
                .map(|plan| (plan.clip_id, plan))
                .collect()
        })
        .unwrap_or_default();

    for clip in benchmark.clips.iter().filter(|clip| clip.success) {
        let clip_path = PathBuf::from(&cli.output_dir).join(format!("clip_{}.mp4", clip.clip_id));
        if !clip_path.exists() {
            continue;
        }
        let thumb_path = dir.join(format!("clip_{}.jpg", clip.clip_id));
        let thumb_text = plan_map
            .get(&clip.clip_id)
            .map(|plan| plan.thumbnail_text.as_str())
            .filter(|text| !text.trim().is_empty());
        extract_thumbnail_with_style(&clip_path, &thumb_path, &cli.thumbnail_style, thumb_text)?;
        images.push(thumb_path);
    }

    if cli.thumbnail_collage {
        write_thumbnail_collage(cli, &images)?;
    }

    Ok(())
}

fn push_config_arg<T: ToString>(args: &mut Vec<String>, flag: &str, value: Option<T>) {
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}

fn push_config_flag(args: &mut Vec<String>, flag: &str, enabled: Option<bool>) {
    if enabled == Some(true) {
        args.push(flag.to_string());
    }
}

fn config_to_args(config: ConfigFile) -> Vec<String> {
    let mut args = Vec::new();

    push_config_flag(&mut args, "--api", config.api);
    push_config_arg(&mut args, "--api-bind", config.api_bind);
    push_config_arg(&mut args, "--url", config.url);
    push_config_arg(
        &mut args,
        "--input",
        config.input.map(|path| path.display().to_string()),
    );
    push_config_arg(&mut args, "--num-clips", config.num_clips);
    push_config_arg(&mut args, "--output-dir", config.output_dir);
    push_config_arg(&mut args, "--min-duration", config.min_duration);
    push_config_flag(&mut args, "--captions", config.captions);
    push_config_flag(&mut args, "--crop", config.crop);
    push_config_arg(&mut args, "--crop-mode", config.crop_mode);
    push_config_flag(&mut args, "--accurate", config.accurate);
    push_config_arg(&mut args, "--whisper-mode", config.whisper_mode);
    push_config_arg(&mut args, "--whisper-model", config.whisper_model);
    push_config_arg(
        &mut args,
        "--transcription-provider",
        config.transcription_provider,
    );
    push_config_arg(
        &mut args,
        "--cloud-transcription-model",
        config.cloud_transcription_model,
    );
    push_config_arg(
        &mut args,
        "--transcription-api-key-env",
        config.transcription_api_key_env,
    );
    push_config_flag(&mut args, "--laughter", config.laughter);
    push_config_arg(
        &mut args,
        "--chat-log",
        config.chat_log.map(|path| path.display().to_string()),
    );
    push_config_arg(&mut args, "--energy-weight", config.energy_weight);
    push_config_arg(&mut args, "--motion-weight", config.motion_weight);
    push_config_arg(&mut args, "--laughter-weight", config.laughter_weight);
    push_config_arg(&mut args, "--chat-weight", config.chat_weight);
    push_config_arg(&mut args, "--transcript-weight", config.transcript_weight);
    push_config_arg(&mut args, "--hook-weight", config.hook_weight);
    push_config_arg(
        &mut args,
        "--subtitle-style-map",
        config
            .subtitle_style_map
            .map(|path| path.display().to_string()),
    );
    push_config_arg(&mut args, "--subtitle-font", config.subtitle_font);
    push_config_arg(&mut args, "--subtitle-size", config.subtitle_size);
    push_config_arg(&mut args, "--subtitle-color", config.subtitle_color);
    push_config_arg(
        &mut args,
        "--subtitle-highlight-color",
        config.subtitle_highlight_color,
    );
    push_config_arg(
        &mut args,
        "--subtitle-outline-color",
        config.subtitle_outline_color,
    );
    push_config_arg(
        &mut args,
        "--subtitle-back-color",
        config.subtitle_back_color,
    );
    push_config_arg(&mut args, "--subtitle-outline", config.subtitle_outline);
    push_config_arg(&mut args, "--subtitle-shadow", config.subtitle_shadow);
    push_config_arg(
        &mut args,
        "--subtitle-border-style",
        config.subtitle_border_style,
    );
    push_config_flag(&mut args, "--subtitle-bold", config.subtitle_bold);
    push_config_arg(&mut args, "--subtitle-alignment", config.subtitle_alignment);
    push_config_arg(&mut args, "--subtitle-margin-v", config.subtitle_margin_v);
    push_config_arg(&mut args, "--subtitle-preset", config.subtitle_preset);
    push_config_arg(&mut args, "--subtitle-animation", config.subtitle_animation);
    push_config_arg(
        &mut args,
        "--subtitle-emoji-layer",
        config.subtitle_emoji_layer,
    );
    push_config_arg(&mut args, "--subtitle-beat-sync", config.subtitle_beat_sync);
    push_config_arg(&mut args, "--subtitle-scene-fx", config.subtitle_scene_fx);
    push_config_flag(&mut args, "--motion", config.motion);
    push_config_arg(&mut args, "--scene-threshold", config.scene_threshold);
    push_config_arg(&mut args, "--subtitles-mode", config.subtitles_mode);
    push_config_arg(&mut args, "--csv-path", config.csv_path);
    push_config_arg(&mut args, "--csv-format", config.csv_format);
    push_config_arg(&mut args, "--timestamp-mode", config.timestamp_mode);
    push_config_flag(&mut args, "--append", config.append);
    push_config_flag(&mut args, "--llm-enable", config.llm_enable);
    push_config_arg(&mut args, "--llm-provider", config.llm_provider);
    push_config_arg(&mut args, "--llm-model", config.llm_model);
    push_config_arg(&mut args, "--llm-api-key-env", config.llm_api_key_env);
    push_config_arg(&mut args, "--llm-output", config.llm_output);
    push_config_flag(&mut args, "--export-bundle", config.export_bundle);
    push_config_arg(&mut args, "--export-bundle-path", config.export_bundle_path);
    push_config_flag(&mut args, "--proof-report", config.proof_report);
    push_config_arg(&mut args, "--proof-report-path", config.proof_report_path);
    push_config_flag(&mut args, "--thumbnails", config.thumbnails);
    push_config_arg(&mut args, "--thumbnails-dir", config.thumbnails_dir);
    push_config_arg(&mut args, "--thumbnail-style", config.thumbnail_style);
    push_config_flag(&mut args, "--thumbnail-collage", config.thumbnail_collage);
    push_config_arg(
        &mut args,
        "--thumbnail-collage-path",
        config.thumbnail_collage_path,
    );
    push_config_arg(&mut args, "--api-key-env", config.api_key_env);
    push_config_arg(
        &mut args,
        "--api-token-sha256-env",
        config.api_token_sha256_env,
    );
    push_config_arg(&mut args, "--api-max-body-bytes", config.api_max_body_bytes);
    push_config_arg(
        &mut args,
        "--api-rate-limit-per-minute",
        config.api_rate_limit_per_minute,
    );
    push_config_arg(
        &mut args,
        "--api-clients-json-env",
        config.api_clients_json_env,
    );
    push_config_flag(
        &mut args,
        "--api-allow-url-input",
        config.api_allow_url_input,
    );
    push_config_arg(
        &mut args,
        "--api-max-queued-jobs",
        config.api_max_queued_jobs,
    );
    push_config_arg(&mut args, "--security-audit-log", config.security_audit_log);
    push_config_arg(&mut args, "--api-quota-store", config.api_quota_store);
    push_config_arg(
        &mut args,
        "--api-client-daily-quota-runs",
        config.api_client_daily_quota_runs,
    );
    push_config_arg(
        &mut args,
        "--api-read-timeout-secs",
        config.api_read_timeout_secs,
    );
    push_config_arg(
        &mut args,
        "--api-write-timeout-secs",
        config.api_write_timeout_secs,
    );
    push_config_arg(
        &mut args,
        "--api-max-header-line-bytes",
        config.api_max_header_line_bytes,
    );
    push_config_arg(&mut args, "--api-url-allowlist", config.api_url_allowlist);
    push_config_arg(&mut args, "--api-url-dns-guard", config.api_url_dns_guard);
    push_config_arg(&mut args, "--malware-scan-cmd", config.malware_scan_cmd);
    push_config_arg(&mut args, "--max-input-bytes", config.max_input_bytes);
    push_config_arg(&mut args, "--allowed-input-exts", config.allowed_input_exts);
    push_config_arg(
        &mut args,
        "--secure-temp-cleanup",
        config.secure_temp_cleanup,
    );

    args
}

fn config_path_from_args(args: &[std::ffi::OsString]) -> Result<Option<PathBuf>> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        let value = arg.to_string_lossy();
        if value == "--config" {
            let Some(path) = iter.next() else {
                anyhow::bail!("--config requires a path");
            };
            return Ok(Some(PathBuf::from(path)));
        }
        if let Some(path) = value.strip_prefix("--config=") {
            return Ok(Some(PathBuf::from(path)));
        }
    }
    Ok(None)
}

fn cli_from_args_with_config(args: Vec<std::ffi::OsString>) -> Result<Cli> {
    let Some(config_path) = config_path_from_args(&args)? else {
        return Cli::try_parse_from(args).context("parse CLI arguments");
    };

    let raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read config file {}", config_path.display()))?;
    let config: ConfigFile = serde_json::from_str(&raw)
        .with_context(|| format!("parse config json {}", config_path.display()))?;

    let mut merged_args = vec![args[0].to_string_lossy().to_string()];
    merged_args.extend(config_to_args(config));

    let mut skip_next = false;
    for arg in args.iter().skip(1) {
        let value = arg.to_string_lossy();
        if skip_next {
            skip_next = false;
            continue;
        }
        if value == "--config" {
            skip_next = true;
            continue;
        }
        if value.starts_with("--config=") {
            continue;
        }
        merged_args.push(value.to_string());
    }

    Cli::try_parse_from(merged_args).context("parse CLI arguments merged with config")
}

fn record_failure(
    task: &ClipTask,
    processing: &ProcessingOptions,
    started_at: Instant,
    extract_ms: u128,
    subtitles_ms: u128,
    crop_ms: u128,
    error: impl Into<String>,
) -> ClipTiming {
    let (timestamp, timestamp_human) = now_strings(&processing.timestamp_mode);
    ClipTiming {
        clip_id: task.clip_id,
        start_sec: task.start_sec,
        duration: processing.min_duration,
        energy_score: task.metrics.energy,
        laughter_score: task.metrics.laughter,
        motion_score: task.metrics.motion,
        chat_score: task.metrics.chat,
        transcript_score: task.metrics.transcript,
        hook_score: task.metrics.hook,
        transcript_density: task.metrics.transcript_density,
        readability_score: readability_from_density_and_confidence(
            task.metrics.transcript_density,
            task.metrics.speech_confidence,
        ),
        total_score: task.metrics.score,
        extract_ms,
        subtitles_ms,
        crop_ms,
        total_ms: started_at.elapsed().as_millis(),
        success: false,
        duplicate_of: None,
        error: error.into(),
        timestamp,
        timestamp_human,
    }
}

fn record_success(
    task: &ClipTask,
    processing: &ProcessingOptions,
    started_at: Instant,
    extract_ms: u128,
    subtitles_ms: u128,
    crop_ms: u128,
) -> ClipTiming {
    let (timestamp, timestamp_human) = now_strings(&processing.timestamp_mode);
    ClipTiming {
        clip_id: task.clip_id,
        start_sec: task.start_sec,
        duration: processing.min_duration,
        energy_score: task.metrics.energy,
        laughter_score: task.metrics.laughter,
        motion_score: task.metrics.motion,
        chat_score: task.metrics.chat,
        transcript_score: task.metrics.transcript,
        hook_score: task.metrics.hook,
        transcript_density: task.metrics.transcript_density,
        readability_score: readability_from_density_and_confidence(
            task.metrics.transcript_density,
            task.metrics.speech_confidence,
        ),
        total_score: task.metrics.score,
        extract_ms,
        subtitles_ms,
        crop_ms,
        total_ms: started_at.elapsed().as_millis(),
        success: true,
        duplicate_of: None,
        error: String::new(),
        timestamp,
        timestamp_human,
    }
}

fn finalize_output(source: &Path, destination: &Path) -> Result<()> {
    if let Err(rename_err) = std::fs::rename(source, destination) {
        std::fs::copy(source, destination)
            .with_context(|| format!("copy final output after rename failure: {rename_err}"))?;
    }
    Ok(())
}

fn synthesize_video_audio_track(input_path: &Path, output_path: &Path) -> Result<()> {
    let ffmpeg = which("ffmpeg").context("ffmpeg not found in PATH")?;
    let copy_status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(input_path)
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("anullsrc=r=48000:cl=stereo")
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("1:a:0")
        .arg("-c:v")
        .arg("copy")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("128k")
        .arg("-shortest")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output_path)
        .status()
        .context("failed to run ffmpeg for silent video audio mux")?;
    if copy_status.success() {
        return Ok(());
    }

    warn!(
        "Stream-copy audio mux failed for {}; retrying with video re-encode",
        input_path.display()
    );
    let reencode_status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(input_path)
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("anullsrc=r=48000:cl=stereo")
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("1:a:0")
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("fast")
        .arg("-crf")
        .arg("23")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("128k")
        .arg("-shortest")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output_path)
        .status()
        .context("failed to run ffmpeg for silent video audio re-encode")?;
    if !reencode_status.success() {
        anyhow::bail!("ffmpeg failed to add fallback audio track");
    }
    Ok(())
}

fn ensure_video_has_audio(input_path: &Path, output_path: &Path) -> Result<()> {
    if has_audio_stream(input_path)? {
        finalize_output(input_path, output_path)?;
        return Ok(());
    }

    warn!(
        "Output clip {} has no audio stream; muxing silent fallback track",
        input_path.display()
    );
    synthesize_video_audio_track(input_path, output_path)
}

fn process_clip(task: &ClipTask, context: &ProcessingContext) -> ClipTiming {
    let started_at = Instant::now();
    let mut extract_ms = 0u128;
    let mut subtitles_ms = 0u128;
    let mut crop_ms = 0u128;

    let raw_clip = context
        .temp_path
        .join(format!("clip_{}_raw.mp4", task.clip_id));
    let final_output =
        PathBuf::from(&context.processing.output_dir).join(format!("clip_{}.mp4", task.clip_id));

    let extract_started = Instant::now();
    if let Err(error) = extract_clip(
        &context.video_path,
        task.start_sec,
        context.processing.min_duration,
        &raw_clip,
        context.processing.accurate,
    ) {
        return record_failure(
            task,
            &context.processing,
            started_at,
            extract_ms,
            subtitles_ms,
            crop_ms,
            format!("extract failed: {error}"),
        );
    }
    extract_ms = extract_started.elapsed().as_millis();
    println!(
        "{}",
        format!(
            "Clip {} extracted in {:.2?}",
            task.clip_id,
            extract_started.elapsed()
        )
        .yellow()
    );

    let mut current = raw_clip;
    if let Some(full_srt_path) = context.full_srt.as_ref() {
        let clip_srt = context.temp_path.join(format!("clip_{}.srt", task.clip_id));
        if let Err(error) =
            extract_srt_segment(full_srt_path, task.start_sec, task.end_sec, &clip_srt)
        {
            return record_failure(
                task,
                &context.processing,
                started_at,
                extract_ms,
                subtitles_ms,
                crop_ms,
                format!("subtitle segment failed: {error}"),
            );
        }

        if clip_srt
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            > 0
        {
            let captioned = context
                .temp_path
                .join(format!("clip_{}_captioned.mp4", task.clip_id));
            let subtitle_started = Instant::now();
            let style = subtitle_style_for_clip(&context.subtitle_styles, task.clip_id);
            let animation = subtitle_animation_preset(&context.processing.subtitle_animation);
            let scene_score = if context.processing.subtitle_scene_fx {
                ((task.metrics.motion * 0.65) + (task.metrics.energy * 0.35)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let render_options = SubtitleRenderOptions {
                template: context.processing.subtitle_preset.clone(),
                emoji_layer: context.processing.subtitle_emoji_layer,
                beat_sync: context.processing.subtitle_beat_sync,
                scene_score,
            };
            let burn_result = match context.processing.subtitles_mode.as_str() {
                "ass" => burn_subtitles_via_ass(
                    &current,
                    &clip_srt,
                    &captioned,
                    style,
                    animation,
                    Some(&render_options),
                ),
                "subtitles" => match style {
                    Some(style) if animation == SubtitleAnimationPreset::None => {
                        burn_subtitles(&current, &clip_srt, &captioned, style)
                    }
                    Some(_) => burn_subtitles_via_ass(
                        &current,
                        &clip_srt,
                        &captioned,
                        style,
                        animation,
                        Some(&render_options),
                    ),
                    None => Err(anyhow::anyhow!("missing subtitle style for subtitles mode")),
                },
                "auto" => {
                    if animation != SubtitleAnimationPreset::None {
                        burn_subtitles_via_ass(
                            &current,
                            &clip_srt,
                            &captioned,
                            style,
                            animation,
                            Some(&render_options),
                        )
                    } else if let Some(style) = style {
                        burn_subtitles(&current, &clip_srt, &captioned, style).or_else(|_| {
                            burn_subtitles_via_ass(
                                &current,
                                &clip_srt,
                                &captioned,
                                Some(style),
                                SubtitleAnimationPreset::None,
                                Some(&render_options),
                            )
                        })
                    } else {
                        burn_subtitles_via_ass(
                            &current,
                            &clip_srt,
                            &captioned,
                            None,
                            SubtitleAnimationPreset::None,
                            Some(&render_options),
                        )
                    }
                }
                _ => burn_subtitles_via_ass(
                    &current,
                    &clip_srt,
                    &captioned,
                    style,
                    animation,
                    Some(&render_options),
                ),
            };

            if let Err(error) = burn_result {
                return record_failure(
                    task,
                    &context.processing,
                    started_at,
                    extract_ms,
                    subtitles_ms,
                    crop_ms,
                    format!("subtitle burn failed: {error}"),
                );
            }

            subtitles_ms = subtitle_started.elapsed().as_millis();
            current = captioned;
            println!(
                "{}",
                format!(
                    "Subtitles burned for clip {} in {:.2?}",
                    task.clip_id,
                    subtitle_started.elapsed()
                )
                .green()
            );
        }
    }

    if context.processing.crop {
        let cropped = context
            .temp_path
            .join(format!("clip_{}_cropped.mp4", task.clip_id));
        let crop_started = Instant::now();
        if let Err(error) = crop_to_vertical(&current, &cropped, &context.processing.crop_mode) {
            return record_failure(
                task,
                &context.processing,
                started_at,
                extract_ms,
                subtitles_ms,
                crop_ms,
                format!("crop failed: {error}"),
            );
        }
        crop_ms = crop_started.elapsed().as_millis();
        current = cropped;
        println!(
            "{}",
            format!(
                "Clip {} cropped in {:.2?}",
                task.clip_id,
                crop_started.elapsed()
            )
            .green()
        );
    }

    let final_source = if current == final_output {
        current.clone()
    } else {
        current
    };
    if let Err(error) = ensure_video_has_audio(&final_source, &final_output) {
        return record_failure(
            task,
            &context.processing,
            started_at,
            extract_ms,
            subtitles_ms,
            crop_ms,
            format!("finalize output failed: {error}"),
        );
    }

    record_success(
        task,
        &context.processing,
        started_at,
        extract_ms,
        subtitles_ms,
        crop_ms,
    )
}

fn write_csv_log(path: &Path, timings: &[ClipTiming], append: bool) -> Result<()> {
    let path_exists = path.exists();
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .with_context(|| format!("open benchmark csv {}", path.display()))?;
    let mut writer = WriterBuilder::new().has_headers(false).from_writer(file);

    if !append || !path_exists {
        writer.write_record([
            "clip_id",
            "start_sec",
            "duration",
            "energy_score",
            "laughter_score",
            "motion_score",
            "chat_score",
            "transcript_score",
            "hook_score",
            "transcript_density",
            "readability_score",
            "total_score",
            "extract_ms",
            "subtitles_ms",
            "crop_ms",
            "total_ms",
            "success",
            "duplicate_of",
            "error",
            "timestamp",
            "timestamp_human",
        ])?;
    }

    for timing in timings {
        writer.serialize(timing)?;
    }
    writer.flush()?;
    Ok(())
}

fn write_json_log(path: &Path, benchmark: &BenchmarkLog, append: bool) -> Result<()> {
    let payload = if append && path.exists() {
        let existing_raw = std::fs::read_to_string(path)
            .with_context(|| format!("read existing benchmark json {}", path.display()))?;
        let mut existing: Vec<BenchmarkLog> =
            serde_json::from_str(&existing_raw).context("parse existing benchmark json")?;
        existing.push(benchmark.clone());
        serde_json::to_string_pretty(&existing)?
    } else {
        serde_json::to_string_pretty(benchmark)?
    };
    std::fs::write(path, payload)
        .with_context(|| format!("write benchmark json {}", path.display()))?;
    Ok(())
}

fn write_human_log(path: &Path, benchmark: &BenchmarkLog, append: bool) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .with_context(|| format!("open benchmark human {}", path.display()))?;

    writeln!(
        file,
        "Run started: {} ({})",
        benchmark.summary.run_timestamp, benchmark.summary.run_timestamp_human
    )?;
    writeln!(
        file,
        "Total clips: {}, success: {}, failed: {}, total duration: {} ms",
        benchmark.summary.total_clips,
        benchmark.summary.successful_clips,
        benchmark.summary.failed_clips,
        benchmark.summary.total_duration_ms
    )?;
    for clip in &benchmark.clips {
        writeln!(
            file,
            "clip {}: start {:.1}s dur {:.1}s score {:.3} success {} error {}",
            clip.clip_id, clip.start_sec, clip.duration, clip.total_score, clip.success, clip.error
        )?;
    }
    writeln!(file)?;
    Ok(())
}

fn write_benchmark_log(cli: &Cli, benchmark: &BenchmarkLog) -> Result<()> {
    let path = Path::new(&cli.csv_path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create benchmark directory {}", parent.display()))?;
        }
    }

    match cli.csv_format.as_str() {
        "csv" => write_csv_log(path, &benchmark.clips, cli.append),
        "json" => write_json_log(path, benchmark, cli.append),
        "human" => write_human_log(path, benchmark, cli.append),
        other => anyhow::bail!("Unsupported benchmark format: {other}"),
    }
}

fn ensure_scoring_enabled(cli: &Cli) -> Result<()> {
    let total_weight = cli.energy_weight
        + if cli.motion { cli.motion_weight } else { 0.0 }
        + if cli.laughter {
            cli.laughter_weight
        } else {
            0.0
        }
        + if cli.chat_log.is_some() {
            cli.chat_weight
        } else {
            0.0
        }
        + cli.transcript_weight
        + cli.hook_weight;
    if total_weight <= 0.0 {
        anyhow::bail!("At least one enabled scoring weight must be positive");
    }
    Ok(())
}

fn print_no_args_guidance() {
    eprintln!("viralclip-swarm needs a video source before it can generate clips.");
    eprintln!();
    eprintln!("Use one of these:");
    eprintln!("  viralclip-swarm --input \"C:\\path\\to\\video.mp4\"");
    eprintln!("  viralclip-swarm --url \"https://www.youtube.com/watch?v=...\"");
    eprintln!();
    eprintln!("Optional example:");
    eprintln!("  viralclip-swarm --input \"video.mp4\" --num-clips 5 --min-duration 8");
}

fn read_prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().context("flush interactive prompt")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read interactive prompt response")?;
    Ok(input.trim().to_string())
}

fn looks_like_url(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")
}

fn normalize_prompt_value(value: String) -> String {
    value.trim().trim_matches('"').trim().to_string()
}

fn prompt_with_default(prompt: &str, default: &str) -> Result<String> {
    let value = normalize_prompt_value(read_prompt_line(&format!("{prompt} [{default}]: "))?);
    if value.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(value)
    }
}

fn prompt_bool(prompt: &str, default: bool) -> Result<bool> {
    let default_hint = if default { "Y/n" } else { "y/N" };
    let value = normalize_prompt_value(read_prompt_line(&format!("{prompt} [{default_hint}]: "))?);
    if value.is_empty() {
        return Ok(default);
    }

    match value.to_ascii_lowercase().as_str() {
        "y" | "yes" | "true" | "1" => Ok(true),
        "n" | "no" | "false" | "0" => Ok(false),
        _ => anyhow::bail!("Invalid yes/no response: {value}"),
    }
}

fn prompt_optional_path(prompt: &str) -> Result<Option<String>> {
    let value = normalize_prompt_value(read_prompt_line(prompt)?);
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn prompt_for_cli() -> Result<Cli> {
    println!(
        "{}",
        "No arguments provided. Choose a source to continue.".yellow()
    );
    println!("Enter one source and leave the other blank.");
    let input_path = normalize_prompt_value(read_prompt_line("Local video path: ")?);
    let url = normalize_prompt_value(read_prompt_line("YouTube URL: ")?);

    let mut args = vec!["viralclip-swarm".to_string()];

    match (input_path.is_empty(), url.is_empty()) {
        (false, true) => {
            if looks_like_url(&input_path) {
                args.push("--url".to_string());
                args.push(input_path);
            } else {
                let candidate = PathBuf::from(&input_path);
                if !candidate.is_file() {
                    anyhow::bail!(
                        "Local video path does not exist or is not a file: {}",
                        candidate.display()
                    );
                }
                args.push("--input".to_string());
                args.push(input_path);
            }
        }
        (true, false) => {
            args.push("--url".to_string());
            args.push(url);
        }
        (true, true) => {
            print_no_args_guidance();
            anyhow::bail!("missing input source")
        }
        (false, false) => {
            eprintln!("Provide either a local file path or a YouTube URL, not both.");
            anyhow::bail!("multiple prompted input sources")
        }
    }

    println!();
    println!("{}", "Interactive options".cyan());
    let num_clips = prompt_with_default("Number of clips", "10")?;
    let min_duration = prompt_with_default("Clip duration in seconds", "5.0")?;
    let output_dir = prompt_with_default("Output directory", "./output")?;
    let captions = prompt_bool("Enable captions", false)?;
    let crop = prompt_bool("Enable vertical crop", false)?;
    let accurate = prompt_bool("Use accurate cuts (slower)", false)?;
    let motion = prompt_bool("Enable motion detection", false)?;
    let laughter = prompt_bool("Enable laughter detection", false)?;
    let llm_enable = prompt_bool("Enable AI storyboard and metadata", false)?;
    let export_bundle = prompt_bool("Generate platform export bundle", true)?;
    let proof_report = prompt_bool("Generate proof report", true)?;
    let thumbnails = prompt_bool("Generate thumbnails", true)?;
    let chat_log = prompt_optional_path("Chat log path (optional, blank to skip): ")?;

    args.push("--num-clips".to_string());
    args.push(num_clips);
    args.push("--min-duration".to_string());
    args.push(min_duration);
    args.push("--output-dir".to_string());
    args.push(output_dir);

    if captions {
        args.push("--captions".to_string());
        let transcription_provider =
            prompt_with_default("Transcription provider (local/openai)", "local")?;
        args.push("--transcription-provider".to_string());
        args.push(transcription_provider.clone());
        if transcription_provider == "local" {
            let whisper_mode = prompt_with_default("Whisper mode (python/binary)", "python")?;
            let whisper_model = prompt_with_default("Whisper model", "base")?;
            args.push("--whisper-mode".to_string());
            args.push(whisper_mode);
            args.push("--whisper-model".to_string());
            args.push(whisper_model);
        } else {
            let cloud_model = prompt_with_default("Cloud transcription model", "whisper-1")?;
            let api_key_env =
                prompt_with_default("Transcription API key env var", "OPENAI_API_KEY")?;
            args.push("--cloud-transcription-model".to_string());
            args.push(cloud_model);
            args.push("--transcription-api-key-env".to_string());
            args.push(api_key_env);
        }
        let subtitle_preset = prompt_with_default(
            "Subtitle preset (classic/legendary/creator_pro/creator_neon/creator_minimal/creator_bold)",
            "legendary",
        )?;
        let subtitle_animation = prompt_with_default(
            "Subtitle animation (none/karaoke/emphasis/impact/pulse/creator_pro)",
            "none",
        )?;
        if prompt_bool("Enable emoji/sticker subtitle layer", true)? {
            args.push("--subtitle-emoji-layer".to_string());
            args.push("true".to_string());
        } else {
            args.push("--subtitle-emoji-layer".to_string());
            args.push("false".to_string());
        }
        if prompt_bool("Enable beat-sync subtitle timing", true)? {
            args.push("--subtitle-beat-sync".to_string());
            args.push("true".to_string());
        } else {
            args.push("--subtitle-beat-sync".to_string());
            args.push("false".to_string());
        }
        if prompt_bool("Enable scene-aware subtitle FX", true)? {
            args.push("--subtitle-scene-fx".to_string());
            args.push("true".to_string());
        } else {
            args.push("--subtitle-scene-fx".to_string());
            args.push("false".to_string());
        }
        let subtitles_mode = prompt_with_default("Subtitles mode (auto/ass/subtitles)", "auto")?;
        args.push("--subtitle-preset".to_string());
        args.push(subtitle_preset);
        args.push("--subtitle-animation".to_string());
        args.push(subtitle_animation);
        args.push("--subtitles-mode".to_string());
        args.push(subtitles_mode);
    }

    if crop {
        args.push("--crop".to_string());
        let crop_mode = prompt_with_default("Crop mode (center/subject/face)", "center")?;
        args.push("--crop-mode".to_string());
        args.push(crop_mode);
    }

    if accurate {
        args.push("--accurate".to_string());
    }

    if motion {
        args.push("--motion".to_string());
        let scene_threshold = prompt_with_default("Scene threshold", "0.4")?;
        args.push("--scene-threshold".to_string());
        args.push(scene_threshold);
    }

    if laughter {
        args.push("--laughter".to_string());
    }

    if let Some(chat_log) = chat_log {
        let chat_path = PathBuf::from(&chat_log);
        if !chat_path.is_file() {
            anyhow::bail!(
                "Chat log path does not exist or is not a file: {}",
                chat_path.display()
            );
        }
        args.push("--chat-log".to_string());
        args.push(chat_log);
    }

    if llm_enable {
        args.push("--llm-enable".to_string());
        let provider = prompt_with_default(
            "LLM provider (heuristic/openai/openrouter/groq/huggingface/anthropic/gemini)",
            "heuristic",
        )?;
        let model = if provider == "heuristic" || provider == "local" {
            "heuristic".to_string()
        } else {
            prompt_with_default("LLM model", "gpt-4o-mini")?
        };
        let api_key_env = if provider == "heuristic" || provider == "local" {
            "OPENAI_API_KEY".to_string()
        } else if provider == "openai" {
            prompt_with_default("LLM API key env var", "OPENAI_API_KEY")?
        } else if provider == "openrouter" {
            prompt_with_default("LLM API key env var", "OPENROUTER_API_KEY")?
        } else if provider == "groq" {
            prompt_with_default("LLM API key env var", "GROQ_API_KEY")?
        } else if provider == "huggingface" {
            prompt_with_default("LLM API key env var", "HF_TOKEN")?
        } else if provider == "anthropic" {
            prompt_with_default("LLM API key env var", "ANTHROPIC_API_KEY")?
        } else {
            prompt_with_default("LLM API key env var", "GEMINI_API_KEY")?
        };
        let output_path =
            prompt_with_default("AI storyboard output", "./output/ai_storyboard.json")?;
        args.push("--llm-provider".to_string());
        args.push(provider);
        args.push("--llm-model".to_string());
        args.push(model);
        args.push("--llm-api-key-env".to_string());
        args.push(api_key_env);
        args.push("--llm-output".to_string());
        args.push(output_path);
    }

    if export_bundle {
        args.push("--export-bundle".to_string());
    }
    if proof_report {
        args.push("--proof-report".to_string());
    }
    if thumbnails {
        args.push("--thumbnails".to_string());
        let thumbnail_style =
            prompt_with_default("Thumbnail style (plain/framed/cinematic)", "framed")?;
        args.push("--thumbnail-style".to_string());
        args.push(thumbnail_style);
        if prompt_bool("Generate thumbnail collage", true)? {
            args.push("--thumbnail-collage".to_string());
        }
    }

    Cli::try_parse_from(args).context("parse prompted interactive CLI arguments")
}

fn cli_from_env() -> Result<Cli> {
    let args: Vec<_> = env::args_os().collect();
    if args.len() > 1 {
        return cli_from_args_with_config(args);
    }
    prompt_for_cli()
}

fn ensure_input_source(cli: &Cli) -> Result<()> {
    let allowed_exts = parse_allowed_extensions(&cli.allowed_input_exts);
    if allowed_exts.is_empty() {
        anyhow::bail!("allowed_input_exts may not be empty");
    }
    match (&cli.input, &cli.url) {
        (Some(_), Some(_)) => anyhow::bail!("Provide either --input or --url, not both"),
        (None, None) => anyhow::bail!("Either --input or --url must be provided"),
        (Some(path), None) => validate_input_file_path(path, cli.max_input_bytes, &allowed_exts),
        _ => Ok(()),
    }
}

fn print_run_banner(cli: &Cli) {
    println!("{}", "ViralClip Swarm".bold().cyan());
    if let Some(url) = &cli.url {
        println!("{}", format!("URL: {url}").blue());
    } else if let Some(path) = &cli.input {
        println!("{}", format!("Input file: {}", path.display()).blue());
    }
    println!("{}", format!("Clips: {}", cli.num_clips).magenta());
    println!("{}", format!("Output: {}", cli.output_dir).magenta());
    println!(
        "{}",
        format!(
            "Weights -> energy: {:.2}, motion: {:.2}, laughter: {:.2}, chat: {:.2}, transcript: {:.2}, hook: {:.2}",
            cli.energy_weight,
            cli.motion_weight,
            cli.laughter_weight,
            cli.chat_weight,
            cli.transcript_weight,
            cli.hook_weight
        )
        .cyan()
    );

    if cli.captions {
        println!(
            "{}",
            format!(
                "Captions enabled (provider: {}, mode: {}, model: {})",
                cli.transcription_provider, cli.whisper_mode, cli.whisper_model
            )
            .green()
        );
        println!(
            "{}",
            format!(
                "Subtitle preset: {} | style: font={} size={} mode={} animation={} emoji_layer={} beat_sync={} scene_fx={}",
                cli.subtitle_preset,
                cli.subtitle_font,
                cli.subtitle_size,
                cli.subtitles_mode,
                cli.subtitle_animation,
                cli.subtitle_emoji_layer,
                cli.subtitle_beat_sync,
                cli.subtitle_scene_fx
            )
            .green()
        );
    }
    if cli.crop {
        println!("{}", format!("Crop enabled ({})", cli.crop_mode).magenta());
    }
    if cli.motion {
        println!(
            "{}",
            format!(
                "Motion detection enabled (scene threshold: {})",
                cli.scene_threshold
            )
            .cyan()
        );
    }
    if cli.laughter {
        println!("{}", "Laughter detection enabled".cyan());
    }
    if let Some(chat_log) = &cli.chat_log {
        println!(
            "{}",
            format!("Chat velocity enabled from {}", chat_log.display()).cyan()
        );
    }
    if cli.llm_enable {
        println!(
            "{}",
            format!(
                "AI storyboard enabled (provider: {}, model: {}, output: {})",
                cli.llm_provider,
                cli.llm_model,
                resolve_llm_output_path(cli).display()
            )
            .cyan()
        );
    }
    if cli.export_bundle {
        println!(
            "{}",
            format!(
                "Export bundle enabled ({})",
                resolve_export_bundle_path(cli).display()
            )
            .cyan()
        );
    }
    if cli.proof_report {
        println!(
            "{}",
            format!(
                "Proof report enabled ({})",
                resolve_proof_report_path(cli).display()
            )
            .cyan()
        );
    }
    if cli.thumbnails {
        println!(
            "{}",
            format!(
                "Thumbnail extraction enabled ({}) style={}",
                resolve_thumbnails_dir(cli).display(),
                cli.thumbnail_style
            )
            .cyan()
        );
    }
    if cli.thumbnail_collage {
        println!(
            "{}",
            format!(
                "Thumbnail collage enabled ({})",
                resolve_thumbnail_collage_path(cli).display()
            )
            .cyan()
        );
    }
    println!(
        "{}",
        format!(
            "Benchmark path: {} (format: {}, append: {})",
            cli.csv_path, cli.csv_format, cli.append
        )
        .cyan()
    );
}

fn run_with_cli(cli: &Cli) -> Result<BenchmarkLog> {
    ensure_input_source(cli)?;
    ensure_scoring_enabled(cli)?;
    let allowed_exts = parse_allowed_extensions(&cli.allowed_input_exts);

    let (run_ts_iso, run_ts_human) = now_strings(&cli.timestamp_mode);
    let start_total = Instant::now();

    print_run_banner(cli);

    let temp_dir: TempDir = tempfile::tempdir().context("create temp dir")?;
    let temp_path = temp_dir.path();

    let video_path = if let Some(input_path) = &cli.input {
        validate_input_file_path(input_path, cli.max_input_bytes, &allowed_exts)?;
        let file_name = input_path
            .file_name()
            .context("input path is missing a file name")?;
        let dest = temp_path.join(file_name);
        std::fs::copy(input_path, &dest).context("copy input to temp")?;
        dest
    } else if let Some(url) = &cli.url {
        println!("{}", "Downloading video...".cyan());
        download_video(url, temp_path)?
    } else {
        anyhow::bail!("Either --url or --input must be provided");
    };
    println!(
        "{}",
        format!("Video ready: {}", video_path.display()).green()
    );
    maybe_run_malware_scan(cli.malware_scan_cmd.as_deref(), &video_path)?;

    let wav_path = temp_path.join("audio.wav");
    let t_audio = Instant::now();
    println!("{}", "Extracting audio...".blue());
    extract_audio(&video_path, &wav_path)?;
    println!(
        "{}",
        format!("Audio extracted in {:.2?}", t_audio.elapsed()).green()
    );

    let full_srt = if cli.captions {
        let srt_path = temp_path.join("full.srt");
        let t_trans = Instant::now();
        println!("{}", "Transcribing audio with Whisper...".green());
        transcribe_full_audio(&wav_path, &srt_path, cli)?;
        println!(
            "{}",
            format!("Transcription finished in {:.2?}", t_trans.elapsed()).green()
        );
        Some(srt_path)
    } else {
        None
    };

    println!("{}", "Analyzing audio windows...".cyan());
    let mut windows = analyze_audio_windows(&wav_path, 1.0)?;
    let mut transcript_entries = Vec::new();
    if let Some(full_srt_path) = &full_srt {
        transcript_entries = parse_srt_entries(full_srt_path)?;
        apply_transcript_scores(&mut windows, &transcript_entries, 1.0);
        println!(
            "{}",
            format!(
                "Transcript-aware scoring enabled from {} entries",
                transcript_entries.len()
            )
            .cyan()
        );
    }

    if cli.motion {
        println!("{}", "Detecting scene changes...".cyan());
        let scene_timestamps = detect_scene_changes(&video_path, cli.scene_threshold)?;
        println!("Found {} scene changes", scene_timestamps.len());
        apply_motion_scores(&mut windows, &scene_timestamps);
    }

    if let Some(chat_log) = &cli.chat_log {
        println!("{}", "Analyzing chat velocity...".cyan());
        let chat_timestamps = parse_chat_log(chat_log)?;
        println!("Parsed {} chat timestamps", chat_timestamps.len());
        apply_chat_scores(&mut windows, &chat_timestamps);
    }

    score_windows(&mut windows, cli);
    let candidate_pool = ((cli.num_clips as usize).saturating_mul(3)).max(cli.num_clips as usize);
    let mut tasks = build_tasks(
        &windows,
        cli.min_duration,
        candidate_pool.min(windows.len()),
    );
    if tasks.is_empty() {
        anyhow::bail!("No clips selected");
    }
    let ai_options = ai_options_from_cli(cli);
    rerank_candidate_tasks(&ai_options, &transcript_entries, &mut tasks)?;
    tasks.truncate(cli.num_clips as usize);
    enrich_selected_tasks_with_multimodal_context(&video_path, &mut tasks);

    println!("{}", format!("Selected {} clips:", tasks.len()).bold());
    for task in &tasks {
        println!(
            "clip {}: {:.1}s - {:.1}s score {:.3} [energy {:.3}, laughter {:.3}, motion {:.3}, chat {:.3}, transcript {:.3}, hook {:.3}]",
            task.clip_id,
            task.start_sec,
            task.end_sec,
            task.metrics.score,
            task.metrics.energy,
            task.metrics.laughter,
            task.metrics.motion,
            task.metrics.chat,
            task.metrics.transcript,
            task.metrics.hook
        );
    }

    std::fs::create_dir_all(&cli.output_dir).context("create output dir")?;
    let subtitle_styles = load_subtitle_styles(cli)?;
    let storyboard = build_storyboard(
        &ai_options,
        &build_ai_clip_contexts(&tasks, &transcript_entries),
    )?;
    if let Some(storyboard) = storyboard.as_ref() {
        let storyboard_path = resolve_llm_output_path(cli);
        write_storyboard(&storyboard_path, storyboard)?;
        println!(
            "{}",
            format!("AI storyboard written to {}", storyboard_path.display()).cyan()
        );
    }
    if cli.export_bundle {
        let bundle_path = resolve_export_bundle_path(cli);
        write_export_bundle(
            &bundle_path,
            &cli.timestamp_mode,
            &tasks,
            storyboard.as_ref(),
        )?;
        println!(
            "{}",
            format!("Export bundle written to {}", bundle_path.display()).cyan()
        );
    }
    let context = ProcessingContext {
        video_path: video_path.clone(),
        temp_path: temp_path.to_path_buf(),
        full_srt,
        subtitle_styles,
        processing: ProcessingOptions {
            min_duration: cli.min_duration,
            accurate: cli.accurate,
            crop: cli.crop,
            crop_mode: cli.crop_mode.clone(),
            subtitles_mode: cli.subtitles_mode.clone(),
            subtitle_preset: cli.subtitle_preset.clone(),
            subtitle_animation: cli.subtitle_animation.clone(),
            subtitle_emoji_layer: cli.subtitle_emoji_layer,
            subtitle_beat_sync: cli.subtitle_beat_sync,
            subtitle_scene_fx: cli.subtitle_scene_fx,
            output_dir: cli.output_dir.clone(),
            timestamp_mode: cli.timestamp_mode.clone(),
        },
    };

    println!("{}", "Processing clips in parallel...".yellow());
    let mut timings: Vec<ClipTiming> = tasks
        .par_iter()
        .map(|task| process_clip(task, &context))
        .collect();
    timings.sort_by_key(|timing| timing.clip_id);

    let successful_clips = timings.iter().filter(|timing| timing.success).count();
    let failed_clips = timings.len().saturating_sub(successful_clips);
    let benchmark = BenchmarkLog {
        summary: RunSummary {
            run_timestamp: run_ts_iso,
            run_timestamp_human: run_ts_human,
            total_clips: timings.len(),
            successful_clips,
            failed_clips,
            total_duration_ms: timings.iter().map(|timing| timing.total_ms).sum(),
        },
        clips: timings.clone(),
    };

    write_benchmark_log(cli, &benchmark)?;
    if cli.proof_report {
        let proof_path = resolve_proof_report_path(cli);
        write_proof_report(cli, &benchmark)?;
        println!(
            "{}",
            format!("Proof report written to {}", proof_path.display()).cyan()
        );
    }
    if cli.thumbnails {
        let thumbnails_dir = resolve_thumbnails_dir(cli);
        write_thumbnails(cli, &benchmark, storyboard.as_ref())?;
        println!(
            "{}",
            format!("Thumbnails written to {}", thumbnails_dir.display()).cyan()
        );
    }

    if !cli.secure_temp_cleanup {
        let preserved_path = temp_dir.path().to_path_buf();
        std::mem::forget(temp_dir);
        println!(
            "{}",
            format!(
                "secure_temp_cleanup disabled; preserved temp workspace at {}",
                preserved_path.display()
            )
            .yellow()
        );
    }

    println!(
        "{}",
        format!("Completed run in {:.2?}", start_total.elapsed()).green()
    );

    if failed_clips > 0 {
        let failures = timings
            .iter()
            .filter(|timing| !timing.success)
            .map(|timing| format!("clip {}: {}", timing.clip_id, timing.error))
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("{} clip(s) failed: {}", failed_clips, failures);
    }

    Ok(benchmark)
}

fn cli_from_config(config: ConfigFile) -> Result<Cli> {
    let mut args = vec!["viralclip-swarm".to_string()];
    args.extend(config_to_args(config));
    Cli::try_parse_from(args).context("parse config into CLI")
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    body: &str,
    content_type: &str,
) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .context("write HTTP response")?;
    Ok(())
}

fn write_json_error(stream: &mut TcpStream, status: &str, message: &str) -> Result<()> {
    let body = serde_json::json!({
        "ok": false,
        "message": message,
    })
    .to_string();
    write_http_response(stream, status, &body, "application/json")
}

fn handle_api_connection(
    mut stream: TcpStream,
    jobs: &JobMap,
    job_tx: &JobSender,
    next_job_id: &Arc<Mutex<usize>>,
    rate_limits: &RateLimitMap,
    quota_lock: &QuotaLock,
    security: &ApiSecurityConfig,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(security.read_timeout_secs.max(1))))
        .context("set API read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(
            security.write_timeout_secs.max(1),
        )))
        .context("set API write timeout")?;

    let mut reader = BufReader::new(stream.try_clone().context("clone TCP stream")?);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .context("read request line")?;
    if request_line.trim().is_empty() {
        return Ok(());
    }
    if request_line.len() > security.max_header_line_bytes {
        let client_id = api_client_id(&stream);
        let _ = append_security_audit_log(
            &security.audit_log_path,
            "request_validation",
            &client_id,
            "rejected",
            "request line too large",
        );
        write_json_error(
            &mut stream,
            "414 URI Too Long",
            "request line exceeds configured size limit",
        )?;
        return Ok(());
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        write_json_error(&mut stream, "400 Bad Request", "invalid request line")?;
        return Ok(());
    }
    let method = parts[0];
    let path = parts[1];

    let client_id = api_client_id(&stream);
    if !check_rate_limit(rate_limits, &client_id, security.rate_limit_per_minute)? {
        let _ = append_security_audit_log(
            &security.audit_log_path,
            "rate_limit",
            &client_id,
            "rejected",
            "rate limit exceeded",
        );
        write_json_error(&mut stream, "429 Too Many Requests", "rate limit exceeded")?;
        return Ok(());
    }

    let mut content_length: Option<usize> = None;
    let mut headers = HashMap::new();
    let mut header_count = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).context("read header line")?;
        if line.len() > security.max_header_line_bytes {
            let _ = append_security_audit_log(
                &security.audit_log_path,
                "request_validation",
                &client_id,
                "rejected",
                "header line too large",
            );
            write_json_error(
                &mut stream,
                "431 Request Header Fields Too Large",
                "header line too large",
            )?;
            return Ok(());
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        header_count += 1;
        if header_count > 64 {
            let _ = append_security_audit_log(
                &security.audit_log_path,
                "request_validation",
                &client_id,
                "rejected",
                "too many headers",
            );
            write_json_error(
                &mut stream,
                "431 Request Header Fields Too Large",
                "too many headers",
            )?;
            return Ok(());
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let key = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if key == "content-length" {
                let parsed = value.parse::<usize>().ok();
                if parsed.is_none() {
                    write_json_error(&mut stream, "400 Bad Request", "invalid content-length")?;
                    return Ok(());
                }
                content_length = parsed;
            }
            headers.insert(key, value);
        }
    }

    if method == "POST" && content_length.is_none() {
        write_json_error(
            &mut stream,
            "411 Length Required",
            "content-length required",
        )?;
        return Ok(());
    }

    let content_length = content_length.unwrap_or(0);
    if content_length > security.max_body_bytes {
        let _ = append_security_audit_log(
            &security.audit_log_path,
            "request_validation",
            &client_id,
            "rejected",
            "request body too large",
        );
        write_json_error(
            &mut stream,
            "413 Payload Too Large",
            "request body too large",
        )?;
        return Ok(());
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        use std::io::Read;
        reader.read_exact(&mut body).context("read request body")?;
    }

    if method == "GET" && path == "/health" {
        write_http_response(
            &mut stream,
            "200 OK",
            "{\"ok\":true,\"status\":\"healthy\"}",
            "application/json",
        )?;
        return Ok(());
    }

    let principal = if let Some(principal) = api_request_authorized(&headers, security) {
        principal
    } else {
        let _ = append_security_audit_log(
            &security.audit_log_path,
            "auth",
            &client_id,
            "rejected",
            "missing or invalid API token",
        );
        write_json_error(
            &mut stream,
            "401 Unauthorized",
            "missing or invalid x-api-key",
        )?;
        return Ok(());
    };

    match (method, path) {
        ("POST", "/run") => {
            if !principal_has_scope(&principal, "run") {
                let _ = append_security_audit_log(
                    &security.audit_log_path,
                    "authz",
                    &principal.client_id,
                    "rejected",
                    "missing run scope",
                );
                write_json_error(&mut stream, "403 Forbidden", "missing run scope")?;
                return Ok(());
            }
            let content_type = headers
                .get("content-type")
                .map(String::as_str)
                .unwrap_or("");
            if !content_type
                .to_ascii_lowercase()
                .starts_with("application/json")
            {
                let _ = append_security_audit_log(
                    &security.audit_log_path,
                    "request_validation",
                    &principal.client_id,
                    "rejected",
                    "invalid content-type",
                );
                write_json_error(
                    &mut stream,
                    "415 Unsupported Media Type",
                    "content-type must be application/json",
                )?;
                return Ok(());
            }
            let config: ConfigFile = match serde_json::from_slice(&body) {
                Ok(config) => config,
                Err(error) => {
                    let _ = append_security_audit_log(
                        &security.audit_log_path,
                        "request_validation",
                        &principal.client_id,
                        "rejected",
                        "invalid JSON body",
                    );
                    write_json_error(
                        &mut stream,
                        "400 Bad Request",
                        &format!("invalid JSON body: {error}"),
                    )?;
                    return Ok(());
                }
            };
            let mut cli = match cli_from_config(config) {
                Ok(cli) => cli,
                Err(error) => {
                    let _ = append_security_audit_log(
                        &security.audit_log_path,
                        "request_validation",
                        &principal.client_id,
                        "rejected",
                        "invalid job config",
                    );
                    write_json_error(
                        &mut stream,
                        "400 Bad Request",
                        &format!("invalid job config: {error}"),
                    )?;
                    return Ok(());
                }
            };
            if let Err(error) = validate_api_job_request(&cli, security) {
                let _ = append_security_audit_log(
                    &security.audit_log_path,
                    "request_validation",
                    &principal.client_id,
                    "rejected",
                    &error.to_string(),
                );
                write_json_error(&mut stream, "400 Bad Request", &error.to_string())?;
                return Ok(());
            }
            cli.malware_scan_cmd = security.malware_scan_cmd.clone();
            if count_active_jobs(jobs)? >= security.max_queued_jobs as usize {
                let _ = append_security_audit_log(
                    &security.audit_log_path,
                    "queue_limit",
                    &principal.client_id,
                    "rejected",
                    "too many queued or running jobs",
                );
                write_json_error(&mut stream, "503 Service Unavailable", "job queue is full")?;
                return Ok(());
            }
            if !check_and_increment_client_quota(
                quota_lock,
                &security.quota_store_path,
                &principal.client_id,
                security.client_daily_quota_runs.max(1),
            )? {
                let _ = append_security_audit_log(
                    &security.audit_log_path,
                    "quota_limit",
                    &principal.client_id,
                    "rejected",
                    "daily run quota exceeded",
                );
                write_json_error(
                    &mut stream,
                    "429 Too Many Requests",
                    "daily run quota exceeded",
                )?;
                return Ok(());
            }
            let job_id = {
                let mut guard = next_job_id
                    .lock()
                    .map_err(|_| anyhow::anyhow!("job counter poisoned"))?;
                let id = *guard;
                *guard += 1;
                id
            };
            {
                let mut guard = jobs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("job map poisoned"))?;
                guard.insert(
                    job_id,
                    ApiJobStatus {
                        job_id,
                        status: "queued".to_string(),
                        message: "job queued".to_string(),
                        benchmark_path: Some(cli.csv_path.clone()),
                        output_dir: Some(cli.output_dir.clone()),
                        summary: None,
                        owner_client_id: principal.client_id.clone(),
                    },
                );
            }
            match job_tx.try_send((job_id, cli)) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    if let Ok(mut guard) = jobs.lock() {
                        guard.remove(&job_id);
                    }
                    let _ = append_security_audit_log(
                        &security.audit_log_path,
                        "queue_limit",
                        &principal.client_id,
                        "rejected",
                        "job queue is full",
                    );
                    write_json_error(&mut stream, "503 Service Unavailable", "job queue is full")?;
                    return Ok(());
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    if let Ok(mut guard) = jobs.lock() {
                        guard.remove(&job_id);
                    }
                    error!("API job queue disconnected");
                    write_json_error(
                        &mut stream,
                        "503 Service Unavailable",
                        "job queue unavailable",
                    )?;
                    return Ok(());
                }
            }
            let _ = append_security_audit_log(
                &security.audit_log_path,
                "job_queue",
                &principal.client_id,
                "accepted",
                &format!("job {} queued", job_id),
            );
            let response = ApiRunResponse {
                ok: true,
                message: format!("job {} queued", job_id),
                benchmark_path: String::new(),
                output_dir: String::new(),
                summary: None,
            };
            let body = serde_json::to_string(&response).context("serialize API response")?;
            write_http_response(&mut stream, "202 Accepted", &body, "application/json")?;
        }
        ("GET", "/jobs") => {
            if !principal_has_scope(&principal, "read") {
                let _ = append_security_audit_log(
                    &security.audit_log_path,
                    "authz",
                    &principal.client_id,
                    "rejected",
                    "missing read scope",
                );
                write_json_error(&mut stream, "403 Forbidden", "missing read scope")?;
                return Ok(());
            }
            let guard = jobs
                .lock()
                .map_err(|_| anyhow::anyhow!("job map poisoned"))?;
            let values: Vec<ApiJobStatus> = guard
                .values()
                .filter(|job| {
                    principal_has_scope(&principal, "admin")
                        || job.owner_client_id == principal.client_id
                })
                .cloned()
                .collect();
            let body = serde_json::to_string(&values).context("serialize jobs list")?;
            write_http_response(&mut stream, "200 OK", &body, "application/json")?;
        }
        ("GET", _) if path.starts_with("/jobs/") => {
            if !principal_has_scope(&principal, "read") {
                let _ = append_security_audit_log(
                    &security.audit_log_path,
                    "authz",
                    &principal.client_id,
                    "rejected",
                    "missing read scope",
                );
                write_json_error(&mut stream, "403 Forbidden", "missing read scope")?;
                return Ok(());
            }
            let job_id = path.trim_start_matches("/jobs/").parse::<usize>().ok();
            let guard = jobs
                .lock()
                .map_err(|_| anyhow::anyhow!("job map poisoned"))?;
            if let Some(job_id) = job_id {
                if let Some(job) = guard.get(&job_id) {
                    if !principal_has_scope(&principal, "admin")
                        && job.owner_client_id != principal.client_id
                    {
                        write_json_error(&mut stream, "404 Not Found", "job not found")?;
                        return Ok(());
                    }
                    let body = serde_json::to_string(job).context("serialize job status")?;
                    write_http_response(&mut stream, "200 OK", &body, "application/json")?;
                } else {
                    write_http_response(
                        &mut stream,
                        "404 Not Found",
                        "{\"ok\":false,\"message\":\"job not found\"}",
                        "application/json",
                    )?;
                }
            } else {
                write_http_response(
                    &mut stream,
                    "400 Bad Request",
                    "{\"ok\":false,\"message\":\"invalid job id\"}",
                    "application/json",
                )?;
            }
        }
        _ => {
            write_json_error(&mut stream, "404 Not Found", "not found")?;
        }
    }

    Ok(())
}

fn run_api_server_with_auth(bind_addr: &str, security: ApiSecurityConfig) -> Result<()> {
    validate_api_bind_address(bind_addr)?;
    let listener = TcpListener::bind(bind_addr)
        .with_context(|| format!("bind API server to {}", bind_addr))?;
    let jobs: JobMap = Arc::new(Mutex::new(HashMap::new()));
    let rate_limits: RateLimitMap = Arc::new(Mutex::new(HashMap::new()));
    let quota_lock: QuotaLock = Arc::new(Mutex::new(()));
    let next_job_id = Arc::new(Mutex::new(1usize));
    let queue_capacity = security.max_queued_jobs.max(1) as usize;
    let (job_tx, job_rx) = mpsc::sync_channel::<(usize, Cli)>(queue_capacity);
    let job_rx = Arc::new(Mutex::new(job_rx));
    let worker_count = std::thread::available_parallelism()
        .map(|count| count.get().clamp(1, 4))
        .unwrap_or(1);
    for worker_id in 0..worker_count {
        let jobs_for_worker = Arc::clone(&jobs);
        let job_rx = Arc::clone(&job_rx);
        let audit_log_path = security.audit_log_path.clone();
        thread::spawn(move || loop {
            let job = {
                let receiver = match job_rx.lock() {
                    Ok(receiver) => receiver,
                    Err(_) => {
                        error!("API job receiver lock poisoned");
                        return;
                    }
                };
                receiver.recv()
            };
            let Ok((job_id, cli)) = job else {
                warn!("API worker {worker_id} exiting because job queue closed");
                return;
            };

            if let Ok(mut guard) = jobs_for_worker.lock() {
                if let Some(job) = guard.get_mut(&job_id) {
                    job.status = "running".to_string();
                    job.message = format!("job running on worker {worker_id}");
                }
            }

            let result = run_with_cli(&cli);
            if let Ok(mut guard) = jobs_for_worker.lock() {
                if let Some(job) = guard.get_mut(&job_id) {
                    match result {
                        Ok(benchmark) => {
                            job.status = "completed".to_string();
                            job.message = "job completed".to_string();
                            job.benchmark_path = Some(cli.csv_path.clone());
                            job.output_dir = Some(cli.output_dir.clone());
                            job.summary = Some(benchmark.summary);
                            let _ = append_security_audit_log(
                                &audit_log_path,
                                "job_complete",
                                "worker",
                                "completed",
                                &format!("job {} completed", job_id),
                            );
                        }
                        Err(error) => {
                            job.status = "failed".to_string();
                            job.message = error.to_string();
                            job.benchmark_path = Some(cli.csv_path.clone());
                            job.output_dir = Some(cli.output_dir.clone());
                            job.summary = None;
                            let _ = append_security_audit_log(
                                &audit_log_path,
                                "job_complete",
                                "worker",
                                "failed",
                                &format!("job {} failed: {}", job_id, error),
                            );
                        }
                    }
                }
            }
        });
    }

    info!("API server listening on http://{}", bind_addr);
    info!("API endpoints enabled: GET /health, POST /run, GET /jobs, GET /jobs/{{id}}");
    if security.raw_api_key.is_some() || security.token_sha256_hex.is_some() {
        info!("API shared-token auth enabled");
    }
    if !security.clients.is_empty() {
        info!(
            "API client registry enabled for {} clients",
            security.clients.len()
        );
    }
    info!(
        "API hardening enabled: max_body={} bytes, rate_limit={}/min, daily_quota={} runs/client, queue_capacity={}, workers={}",
        security.max_body_bytes,
        security.rate_limit_per_minute,
        security.client_daily_quota_runs,
        queue_capacity,
        worker_count
    );
    println!(
        "{}",
        format!("API server listening on http://{}", bind_addr).cyan()
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let jobs = Arc::clone(&jobs);
                let job_tx = job_tx.clone();
                let next_job_id = Arc::clone(&next_job_id);
                let rate_limits = Arc::clone(&rate_limits);
                let quota_lock = Arc::clone(&quota_lock);
                let security = security.clone();
                thread::spawn(move || {
                    if let Err(error) = handle_api_connection(
                        stream,
                        &jobs,
                        &job_tx,
                        &next_job_id,
                        &rate_limits,
                        &quota_lock,
                        &security,
                    ) {
                        error!("API request failed: {error}");
                    }
                });
            }
            Err(error) => error!("API connection error: {error}"),
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = cli_from_env()?;
    if cli.api {
        let raw_api_key = env::var(&cli.api_key_env)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let token_sha256_hex = env::var(&cli.api_token_sha256_env)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| normalize_sha256_hex(&value))
            .transpose()?;
        let security = ApiSecurityConfig {
            raw_api_key,
            token_sha256_hex,
            max_body_bytes: cli.api_max_body_bytes,
            rate_limit_per_minute: cli.api_rate_limit_per_minute.max(1),
            clients: load_api_clients_from_env(&cli.api_clients_json_env)?,
            allow_url_input: cli.api_allow_url_input,
            max_queued_jobs: cli.api_max_queued_jobs.max(1),
            audit_log_path: PathBuf::from(&cli.security_audit_log),
            read_timeout_secs: cli.api_read_timeout_secs.max(1),
            write_timeout_secs: cli.api_write_timeout_secs.max(1),
            max_header_line_bytes: cli.api_max_header_line_bytes.max(256),
            url_allowlist: parse_host_allowlist(&cli.api_url_allowlist),
            url_dns_guard: cli.api_url_dns_guard,
            malware_scan_cmd: cli.malware_scan_cmd.clone(),
            quota_store_path: PathBuf::from(&cli.api_quota_store),
            client_daily_quota_runs: cli.api_client_daily_quota_runs.max(1),
        };
        run_api_server_with_auth(&cli.api_bind, security)
    } else {
        run_with_cli(&cli).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        api_request_authorized, build_proof_report, check_and_increment_client_quota,
        check_rate_limit, clean_transcript_text, cli_from_args_with_config, cli_from_config,
        estimate_music_noise_penalty, estimate_speech_confidence, format_srt_timestamp,
        parse_clock_timestamp, parse_command_template, parse_face_centers, parse_srt_entries,
        parse_srt_timestamp, readability_from_density, readability_from_density_and_confidence,
        sha256_hex, transcript_density_for_range, transcript_excerpt_for_range,
        transcript_similarity, validate_api_bind_address, validate_api_job_request,
        validate_api_url, ApiClientRecord, ApiSecurityConfig, BenchmarkLog, Cli, ClipTiming,
        ConfigFile, QuotaLock, RateLimitMap, RunSummary,
    };
    use clap::Parser;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::sync::{Arc, Mutex};
    use tempfile::{NamedTempFile, TempDir};
    use which::which;

    #[test]
    fn parses_clock_timestamps() {
        assert_eq!(parse_clock_timestamp("01:02:03"), Some(3723.0));
        assert_eq!(parse_clock_timestamp("[12:34]"), Some(754.0));
        assert_eq!(parse_clock_timestamp("bad"), None);
    }

    #[test]
    fn parses_and_formats_srt_timestamps() {
        let seconds = parse_srt_timestamp("00:01:02,250").unwrap();
        assert!((seconds - 62.25).abs() < 0.001);
        assert_eq!(format_srt_timestamp(seconds), "00:01:02,250");
    }

    #[test]
    fn transcript_similarity_detects_overlap() {
        let score = transcript_similarity(
            "This is the secret to going viral today",
            "The secret to going viral starts here",
        );
        assert!(score > 0.4, "expected overlap score, got {score}");
    }

    #[test]
    fn readability_prefers_moderate_density() {
        assert!(readability_from_density(2.8) > readability_from_density(0.5));
        assert!(readability_from_density(2.8) > readability_from_density(6.0));
    }

    #[test]
    fn noisy_repetitive_text_gets_lower_confidence() {
        let clean =
            estimate_speech_confidence("This is the secret to better hooks on short videos", 2.8);
        let noisy = estimate_speech_confidence("oh oh oh yeah yeah yeah la la la", 5.8);
        assert!(
            clean > noisy,
            "expected clean speech confidence > noisy confidence"
        );
        assert!(estimate_music_noise_penalty("oh oh oh yeah yeah yeah la la la", 5.8) > 0.4);
    }

    #[test]
    fn readability_uses_speech_confidence() {
        let high = readability_from_density_and_confidence(2.8, 0.95);
        let low = readability_from_density_and_confidence(2.8, 0.20);
        assert!(high > low);
    }

    #[test]
    fn sha256_auth_accepts_matching_token_hash() {
        let token = "super-secret-demo-token";
        let mut headers = HashMap::new();
        headers.insert("x-api-key".to_string(), token.to_string());
        let security = ApiSecurityConfig {
            raw_api_key: None,
            token_sha256_hex: Some(sha256_hex(token)),
            max_body_bytes: 1024,
            rate_limit_per_minute: 5,
            clients: Vec::new(),
            allow_url_input: false,
            max_queued_jobs: 4,
            audit_log_path: std::path::PathBuf::from("./output/test_audit.log"),
            read_timeout_secs: 15,
            write_timeout_secs: 15,
            max_header_line_bytes: 8192,
            url_allowlist: Vec::new(),
            url_dns_guard: true,
            malware_scan_cmd: None,
            quota_store_path: std::path::PathBuf::from("./output/test_quota.json"),
            client_daily_quota_runs: 10,
        };
        assert!(api_request_authorized(&headers, &security).is_some());
    }

    #[test]
    fn client_registry_auth_returns_scoped_principal() {
        let token = "client-token";
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {token}"));
        let security = ApiSecurityConfig {
            raw_api_key: None,
            token_sha256_hex: None,
            max_body_bytes: 1024,
            rate_limit_per_minute: 5,
            clients: vec![ApiClientRecord {
                client_id: "demo-client".to_string(),
                token_sha256: sha256_hex(token),
                scopes: vec!["read".to_string()],
            }],
            allow_url_input: false,
            max_queued_jobs: 4,
            audit_log_path: std::path::PathBuf::from("./output/test_audit.log"),
            read_timeout_secs: 15,
            write_timeout_secs: 15,
            max_header_line_bytes: 8192,
            url_allowlist: Vec::new(),
            url_dns_guard: true,
            malware_scan_cmd: None,
            quota_store_path: std::path::PathBuf::from("./output/test_quota.json"),
            client_daily_quota_runs: 10,
        };
        let principal = api_request_authorized(&headers, &security).unwrap();
        assert_eq!(principal.client_id, "demo-client");
        assert_eq!(principal.scopes, vec!["read".to_string()]);
    }

    #[test]
    fn rate_limit_blocks_after_threshold() {
        let rate_limits: RateLimitMap = Arc::new(Mutex::new(HashMap::new()));
        assert!(check_rate_limit(&rate_limits, "127.0.0.1", 2).unwrap());
        assert!(check_rate_limit(&rate_limits, "127.0.0.1", 2).unwrap());
        assert!(!check_rate_limit(&rate_limits, "127.0.0.1", 2).unwrap());
    }

    #[test]
    fn rate_limit_prunes_stale_clients_when_map_is_large() {
        let now = std::time::Instant::now();
        let stale = now.checked_sub(std::time::Duration::from_secs(61)).unwrap();
        let rate_limits: RateLimitMap = Arc::new(Mutex::new(HashMap::new()));
        {
            let mut guard = rate_limits.lock().unwrap();
            for idx in 0..4097 {
                guard.insert(format!("client-{idx}"), vec![stale]);
            }
        }

        assert!(check_rate_limit(&rate_limits, "fresh-client", 2).unwrap());
        let guard = rate_limits.lock().unwrap();
        assert!(
            guard.len() < 50,
            "expected stale clients to be pruned, got {}",
            guard.len()
        );
        assert!(guard.contains_key("fresh-client"));
    }

    #[test]
    fn quota_blocks_after_daily_limit() {
        let temp = TempDir::new().unwrap();
        let quota_file = temp.path().join("quota.json");
        let lock: QuotaLock = Arc::new(Mutex::new(()));
        assert!(check_and_increment_client_quota(&lock, &quota_file, "client-a", 2).unwrap());
        assert!(check_and_increment_client_quota(&lock, &quota_file, "client-a", 2).unwrap());
        assert!(!check_and_increment_client_quota(&lock, &quota_file, "client-a", 2).unwrap());
    }

    #[test]
    fn non_loopback_bind_is_rejected() {
        assert!(validate_api_bind_address("127.0.0.1:8787").is_ok());
        assert!(validate_api_bind_address("0.0.0.0:8787").is_err());
    }

    #[test]
    fn api_job_validation_rejects_url_when_disabled() {
        let cli =
            Cli::try_parse_from(["viralclip-swarm", "--url", "https://example.com/video"]).unwrap();
        let security = ApiSecurityConfig {
            raw_api_key: None,
            token_sha256_hex: None,
            max_body_bytes: 1024,
            rate_limit_per_minute: 5,
            clients: Vec::new(),
            allow_url_input: false,
            max_queued_jobs: 4,
            audit_log_path: std::path::PathBuf::from("./output/test_audit.log"),
            read_timeout_secs: 15,
            write_timeout_secs: 15,
            max_header_line_bytes: 8192,
            url_allowlist: Vec::new(),
            url_dns_guard: true,
            malware_scan_cmd: None,
            quota_store_path: std::path::PathBuf::from("./output/test_quota.json"),
            client_daily_quota_runs: 10,
        };
        assert!(validate_api_job_request(&cli, &security).is_err());
    }

    #[test]
    fn api_job_validation_rejects_malware_scan_override() {
        let cli = Cli::try_parse_from([
            "viralclip-swarm",
            "--url",
            "https://example.com/video",
            "--malware-scan-cmd",
            "echo unsafe",
        ])
        .unwrap();
        let security = ApiSecurityConfig {
            raw_api_key: None,
            token_sha256_hex: None,
            max_body_bytes: 1024,
            rate_limit_per_minute: 5,
            clients: Vec::new(),
            allow_url_input: true,
            max_queued_jobs: 4,
            audit_log_path: std::path::PathBuf::from("./output/test_audit.log"),
            read_timeout_secs: 15,
            write_timeout_secs: 15,
            max_header_line_bytes: 8192,
            url_allowlist: Vec::new(),
            url_dns_guard: false,
            malware_scan_cmd: None,
            quota_store_path: std::path::PathBuf::from("./output/test_quota.json"),
            client_daily_quota_runs: 10,
        };
        assert!(validate_api_job_request(&cli, &security).is_err());
    }

    #[test]
    fn api_url_validation_rejects_custom_ports_and_credentials() {
        assert!(validate_api_url("https://example.com:8443/video", &[], false).is_err());
        assert!(validate_api_url("https://user:pass@example.com/video", &[], false).is_err());
        assert!(validate_api_url("https://example.com/video", &[], false).is_ok());
    }

    #[test]
    fn malware_scan_template_parser_handles_quotes() {
        let parts =
            parse_command_template(r#"scanner --flag "{path}" --label "safe mode""#).unwrap();
        assert_eq!(
            parts,
            vec![
                "scanner".to_string(),
                "--flag".to_string(),
                "{path}".to_string(),
                "--label".to_string(),
                "safe mode".to_string()
            ]
        );
        assert!(parse_command_template(r#"scanner "unterminated"#).is_err());
    }

    #[test]
    fn cli_merges_json_config() {
        let config = NamedTempFile::new().unwrap();
        std::fs::write(
            config.path(),
            r#"{
                "input":"tests/fixtures/sample.mp4",
                "captions":true,
                "export_bundle":true,
                "num_clips":3
            }"#,
        )
        .unwrap();

        let cli = cli_from_args_with_config(vec![
            OsString::from("viralclip-swarm"),
            OsString::from("--config"),
            config.path().as_os_str().to_os_string(),
            OsString::from("--min-duration"),
            OsString::from("7"),
        ])
        .unwrap();

        assert_eq!(cli.num_clips, 3);
        assert!(cli.captions);
        assert!(cli.export_bundle);
        assert_eq!(cli.min_duration, 7.0);
        assert_eq!(
            cli.input.unwrap(),
            std::path::PathBuf::from("tests/fixtures/sample.mp4")
        );
    }

    #[test]
    fn parses_face_centers_from_ffmpeg_logs() {
        let stderr = "frame:0 face x:120 y:40 w:80 h:80\nface left=240 top=50 width=100 height=100";
        let centers = parse_face_centers(stderr, 2.0);
        assert_eq!(centers, vec![320, 580]);
    }

    #[test]
    fn transcript_helpers_preserve_density_and_deduplicate_excerpt() {
        let entries = vec![
            super::TranscriptEntry {
                start_sec: 0.0,
                end_sec: 4.0,
                text: "alpha beta gamma delta".to_string(),
            },
            super::TranscriptEntry {
                start_sec: 2.0,
                end_sec: 6.0,
                text: "alpha beta gamma delta".to_string(),
            },
            super::TranscriptEntry {
                start_sec: 3.0,
                end_sec: 5.0,
                text: "fresh insight lands here".to_string(),
            },
        ];

        let excerpt = transcript_excerpt_for_range(&entries, 1.0, 5.0, 6);
        assert_eq!(excerpt, "alpha beta gamma delta fresh insight");

        let density = transcript_density_for_range(&entries, 1.0, 5.0);
        assert!(
            density > 2.4 && density < 2.6,
            "unexpected density: {density}"
        );
    }

    fn build_video_only_fixture(dir: &TempDir) -> std::path::PathBuf {
        let ffmpeg = which("ffmpeg").unwrap();
        let video_path = dir.path().join("video-only.mp4");
        let status = std::process::Command::new(ffmpeg)
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg("testsrc=size=640x360:rate=25")
            .arg("-t")
            .arg("2")
            .arg("-an")
            .arg("-c:v")
            .arg("libx264")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg(&video_path)
            .status()
            .unwrap();
        assert!(status.success(), "expected ffmpeg to create video-only fixture");
        video_path
    }

    #[test]
    fn extract_audio_falls_back_to_silence_for_video_only_input() {
        if which("ffmpeg").is_err() || which("ffprobe").is_err() {
            return;
        }

        let dir = TempDir::new().unwrap();
        let video_path = build_video_only_fixture(&dir);
        let wav_path = dir.path().join("audio.wav");
        super::extract_audio(&video_path, &wav_path).unwrap();
        assert!(wav_path.is_file(), "expected fallback wav to be created");
        assert!(std::fs::metadata(&wav_path).unwrap().len() > 44);
    }

    #[test]
    fn ensure_video_has_audio_adds_silent_track_for_video_only_input() {
        if which("ffmpeg").is_err() || which("ffprobe").is_err() {
            return;
        }

        let dir = TempDir::new().unwrap();
        let video_path = build_video_only_fixture(&dir);
        let output_path = dir.path().join("with-audio.mp4");
        super::ensure_video_has_audio(&video_path, &output_path).unwrap();

        assert!(output_path.is_file(), "expected muxed mp4 to be created");
        assert!(super::has_audio_stream(&output_path).unwrap());
    }

    #[test]
    fn config_can_enable_api_mode_without_input_source() {
        let cli = cli_from_config(ConfigFile {
            api: Some(true),
            api_bind: Some("127.0.0.1:9999".to_string()),
            ..Default::default()
        })
        .unwrap();

        assert!(cli.api);
        assert_eq!(cli.api_bind, "127.0.0.1:9999");
        assert!(cli.input.is_none());
        assert!(cli.url.is_none());
    }

    #[test]
    fn proof_report_uses_successful_clips() {
        let cli = Cli::try_parse_from(["viralclip-swarm", "--input", "tests/fixtures/sample.mp4"])
            .unwrap();
        let benchmark = BenchmarkLog {
            summary: RunSummary {
                run_timestamp: "2026-03-25T00:00:00Z".to_string(),
                run_timestamp_human: "25 Mar 2026, 00:00".to_string(),
                total_clips: 2,
                successful_clips: 1,
                failed_clips: 1,
                total_duration_ms: 2000,
            },
            clips: vec![
                ClipTiming {
                    clip_id: 1,
                    start_sec: 0.0,
                    duration: 10.0,
                    energy_score: 0.8,
                    laughter_score: 0.0,
                    motion_score: 0.0,
                    chat_score: 0.0,
                    transcript_score: 0.9,
                    hook_score: 0.7,
                    transcript_density: 2.8,
                    readability_score: 1.0,
                    total_score: 1.6,
                    extract_ms: 400,
                    subtitles_ms: 100,
                    crop_ms: 200,
                    total_ms: 1000,
                    success: true,
                    duplicate_of: None,
                    error: String::new(),
                    timestamp: "2026-03-25T00:00:00Z".to_string(),
                    timestamp_human: "25 Mar 2026, 00:00".to_string(),
                },
                ClipTiming {
                    clip_id: 2,
                    start_sec: 30.0,
                    duration: 10.0,
                    energy_score: 0.4,
                    laughter_score: 0.0,
                    motion_score: 0.0,
                    chat_score: 0.0,
                    transcript_score: 0.1,
                    hook_score: 0.2,
                    transcript_density: 5.5,
                    readability_score: 0.1,
                    total_score: 0.5,
                    extract_ms: 600,
                    subtitles_ms: 0,
                    crop_ms: 0,
                    total_ms: 1000,
                    success: false,
                    duplicate_of: None,
                    error: "failed".to_string(),
                    timestamp: "2026-03-25T00:00:00Z".to_string(),
                    timestamp_human: "25 Mar 2026, 00:00".to_string(),
                },
            ],
        };

        let report = build_proof_report(&cli, &benchmark);
        assert_eq!(report.best_clip_id, Some(1));
        assert!(report.success_rate > 0.4 && report.success_rate < 0.6);
        assert!(report.average_readability_score > 0.9);
    }

    #[test]
    fn cleans_transcript_text_and_parses_stream_style_srt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sample.srt");
        std::fs::write(
            &path,
            "1\r\n00:00:11,440 --> 00:00:26,320\r\nI gave you right to die\r\n2\r\n00:00:28,260 --> 00:00:44,160\r\nI gave it all and you gave me shit\r\n3\r\n00:00:46,100 --> 00:00:52,140\r\nLove, what would you do if you couldn't get me back?\r\n",
        )
        .unwrap();

        let entries = parse_srt_entries(&path).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "I gave you right to die");
        assert_eq!(
            clean_transcript_text("2 00:00:28,260 --> 00:00:44,160 I gave it all"),
            "I gave it all"
        );
    }
}
