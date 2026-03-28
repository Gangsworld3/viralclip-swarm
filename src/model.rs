use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::subtitles::SubtitleStyle;

#[derive(Clone, Debug)]
pub struct WindowMetrics {
    pub start_sec: f32,
    pub energy: f32,
    pub laughter: f32,
    pub motion: f32,
    pub chat: f32,
    pub transcript: f32,
    pub hook: f32,
    pub semantic: f32,
    pub speech_confidence: f32,
    pub face_score: f32,
    pub transcript_density: f32,
    pub transcript_text: String,
    pub score: f32,
}

#[derive(Clone, Debug)]
pub struct ClipTask {
    pub clip_id: usize,
    pub start_sec: f32,
    pub end_sec: f32,
    pub metrics: WindowMetrics,
}

#[derive(Clone, Debug)]
pub struct CropPlan {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Debug)]
pub struct ProcessingOptions {
    pub min_duration: f32,
    pub accurate: bool,
    pub crop: bool,
    pub crop_mode: String,
    pub subtitles_mode: String,
    pub subtitle_preset: String,
    pub subtitle_animation: String,
    pub subtitle_emoji_layer: bool,
    pub subtitle_beat_sync: bool,
    pub subtitle_scene_fx: bool,
    pub output_dir: String,
    pub timestamp_mode: String,
}

#[derive(Clone, Debug)]
pub struct ProcessingContext {
    pub video_path: PathBuf,
    pub temp_path: PathBuf,
    pub full_srt: Option<PathBuf>,
    pub subtitle_styles: HashMap<usize, SubtitleStyle>,
    pub processing: ProcessingOptions,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClipTiming {
    pub clip_id: usize,
    pub start_sec: f32,
    pub duration: f32,
    pub energy_score: f32,
    pub laughter_score: f32,
    pub motion_score: f32,
    pub chat_score: f32,
    pub transcript_score: f32,
    pub hook_score: f32,
    pub transcript_density: f32,
    pub readability_score: f32,
    pub total_score: f32,
    pub extract_ms: u128,
    pub subtitles_ms: u128,
    pub crop_ms: u128,
    pub total_ms: u128,
    pub success: bool,
    pub duplicate_of: Option<usize>,
    pub error: String,
    pub timestamp: String,
    pub timestamp_human: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunSummary {
    pub run_timestamp: String,
    pub run_timestamp_human: String,
    pub total_clips: usize,
    pub successful_clips: usize,
    pub failed_clips: usize,
    pub total_duration_ms: u128,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BenchmarkLog {
    pub summary: RunSummary,
    pub clips: Vec<ClipTiming>,
}

#[derive(Clone, Debug)]
pub struct TranscriptEntry {
    pub start_sec: f32,
    pub end_sec: f32,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportBundle {
    pub generated_at: String,
    pub generated_at_human: String,
    pub clips: Vec<ExportClipBundle>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportClipBundle {
    pub clip_id: usize,
    pub file_name: String,
    pub start_sec: f32,
    pub duration_sec: f32,
    pub total_score: f32,
    pub transcript_score: f32,
    pub hook_score: f32,
    pub readability_score: f32,
    pub platforms: Vec<PlatformExport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlatformExport {
    pub platform: String,
    pub title: String,
    pub caption: String,
    pub hashtags: Vec<String>,
    pub aspect_ratio: String,
    pub recommended_duration_sec: u32,
}

#[derive(Serialize)]
pub struct ApiRunResponse {
    pub ok: bool,
    pub message: String,
    pub benchmark_path: String,
    pub output_dir: String,
    pub summary: Option<RunSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApiJobStatus {
    pub job_id: usize,
    pub status: String,
    pub message: String,
    pub benchmark_path: Option<String>,
    pub output_dir: Option<String>,
    pub summary: Option<RunSummary>,
    #[serde(skip_serializing)]
    pub owner_client_id: String,
}

pub type JobMap = Arc<Mutex<HashMap<usize, ApiJobStatus>>>;
pub type RateLimitMap = Arc<Mutex<HashMap<String, Vec<Instant>>>>;
pub type QuotaLock = Arc<Mutex<()>>;
#[derive(Clone, Debug)]
pub struct ApiSecurityConfig {
    pub raw_api_key: Option<String>,
    pub token_sha256_hex: Option<String>,
    pub max_body_bytes: usize,
    pub rate_limit_per_minute: u32,
    pub clients: Vec<ApiClientRecord>,
    pub allow_url_input: bool,
    pub max_queued_jobs: u32,
    pub audit_log_path: PathBuf,
    pub read_timeout_secs: u64,
    pub write_timeout_secs: u64,
    pub max_header_line_bytes: usize,
    pub url_allowlist: Vec<String>,
    pub url_dns_guard: bool,
    pub malware_scan_cmd: Option<String>,
    pub quota_store_path: PathBuf,
    pub client_daily_quota_runs: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApiClientRecord {
    pub client_id: String,
    pub token_sha256: String,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ApiPrincipal {
    pub client_id: String,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiQuotaState {
    pub day_utc: String,
    pub clients: HashMap<String, u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProofReport {
    pub generated_at: String,
    pub generated_at_human: String,
    pub benchmark_path: String,
    pub output_dir: String,
    pub success_rate: f32,
    pub average_total_score: f32,
    pub average_readability_score: f32,
    pub average_extract_ms: f32,
    pub average_total_ms: f32,
    pub best_clip_id: Option<usize>,
    pub best_clip_score: f32,
    pub highlights: Vec<String>,
}
