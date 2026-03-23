use anyhow::{Context, Result};
use chrono::{Local, Utc};
use clap::Parser;
use colored::*;
use csv::WriterBuilder;
use hound::WavReader;
use rayon::prelude::*;
use regex::Regex;
use serde::Serialize;
use serde_json;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tempfile::TempDir;
use which::which;

/// ViralClip Swarm CLI
#[derive(Parser)]
#[command(name = "viralclip-swarm")]
#[command(about = "AI‑powered viral clip generator", long_about = None)]
struct Cli {
    /// YouTube video URL
    #[arg(short, long)]
    url: Option<String>,

    /// Local input video file
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Number of clips to generate
    #[arg(short, long, default_value_t = 10)]
    num_clips: u32,

    /// Output directory for clips
    #[arg(short, long, default_value = "./output")]
    output_dir: String,

    /// Minimum clip duration in seconds
    #[arg(long, default_value_t = 5.0)]
    min_duration: f32,

    /// Add dynamic captions (requires Whisper)
    #[arg(long, default_value_t = false)]
    captions: bool,

    /// Crop to 9:16 vertical format
    #[arg(long, default_value_t = false)]
    crop: bool,

    /// Use accurate cuts (re‑encode) – slower but frame‑accurate
    #[arg(long, default_value_t = false)]
    accurate: bool,

    /// Whisper mode: binary or python
    #[arg(long, default_value = "python", value_parser = ["binary", "python"])]
    whisper_mode: String,

    /// Whisper model: tiny, base, small, medium, large
    #[arg(long, default_value = "base")]
    whisper_model: String,

    // Subtitle styling
    #[arg(long, default_value = "Monospace")]
    subtitle_font: String,

    #[arg(long, default_value = "24")]
    subtitle_size: u32,

    #[arg(long, default_value = "&H00FFFFFF")]
    subtitle_color: String,

    #[arg(long, default_value = "2")]
    subtitle_outline: u32,

    #[arg(long, default_value = "1")]
    subtitle_border_style: u32,

    // Motion detection
    #[arg(long, default_value_t = false)]
    motion: bool,

    #[arg(long, default_value_t = 0.4)]
    scene_threshold: f64,

    /// Subtitles mode: auto, ass, subtitles
    #[arg(long, default_value = "auto", value_parser = ["auto", "ass", "subtitles"])]
    subtitles_mode: String,

    /// Path to write benchmark (csv/json/human)
    #[arg(long, default_value = "./output/benchmark.csv")]
    csv_path: String,

    /// Format for benchmark output
    #[arg(long, default_value = "csv", value_parser = ["csv", "json", "human"])]
    csv_format: String,

    /// Timestamp mode: utc or local
    #[arg(long, default_value = "utc", value_parser = ["utc", "local"])]
    timestamp_mode: String,

    /// Append to existing log file instead of overwriting
    #[arg(long, default_value_t = false)]
    append: bool,
}

// -----------------------------------------------------------------------------
// Timing record and summary
#[derive(Serialize)]
struct ClipTiming {
    clip_id: usize,
    start_sec: f32,
    duration: f32,
    extract_ms: u128,
    subtitles_ms: u128,
    crop_ms: u128,
    total_ms: u128,
    success: bool,
    error: String,
    timestamp: String,
    timestamp_human: String,
}

#[derive(Serialize)]
struct RunSummary {
    run_timestamp: String,
    run_timestamp_human: String,
    total_clips: usize,
    total_duration_ms: u128,
}

// -----------------------------------------------------------------------------
// Utility: find yt-dlp
fn find_yt_dlp() -> Result<PathBuf> {
    which("yt-dlp").context("yt-dlp not found in PATH. Please install it.")
}

// -----------------------------------------------------------------------------
// Download video using yt-dlp
fn download_video(url: &str, output_dir: &Path) -> Result<PathBuf> {
    let yt_dlp = find_yt_dlp()?;
    let status = Command::new(&yt_dlp)
        .arg(url)
        .arg("-o")
        .arg(output_dir.join("%(title)s.%(ext)s").to_str().unwrap())
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

    let files: Vec<_> = std::fs::read_dir(output_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "mp4"))
        .collect();

    if files.is_empty() {
        anyhow::bail!("No mp4 file found after download");
    }

    Ok(files[0].path())
}

// -----------------------------------------------------------------------------
// Extract audio from video (16kHz mono PCM)
fn extract_audio(video_path: &Path, output_wav: &Path) -> Result<()> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;
    let status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
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

    if !status.success() {
        anyhow::bail!("ffmpeg audio extraction failed");
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Analyze energy (RMS) per window_secs
fn analyze_energy(wav_path: &Path, window_secs: f32) -> Result<Vec<f32>> {
    let reader = WavReader::open(wav_path).context("opening WAV for energy analysis")?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let window_size = (window_secs * sample_rate as f32) as usize;

    let samples: Vec<i16> = reader.into_samples::<i16>().collect::<Result<_, _>>().context("failed to read WAV samples")?;

    let mut energies = Vec::new();
    for chunk in samples.chunks(window_size) {
        if chunk.is_empty() {
            energies.push(0.0);
            continue;
        }
        let sum_sq: f32 = chunk.iter().map(|&s| (s as f32).powi(2)).sum();
        let rms = (sum_sq / chunk.len() as f32).sqrt();
        energies.push(rms);
    }
    Ok(energies)
}

// -----------------------------------------------------------------------------
// Clip extraction (fast copy by default, accurate re‑encode if requested)
fn extract_clip(
    video_path: &Path,
    start_sec: f32,
    duration_sec: f32,
    output_path: &Path,
    accurate: bool,
) -> Result<()> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;
    let start_str = format!("{}", start_sec);
    let dur_str = format!("{}", duration_sec);

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

// -----------------------------------------------------------------------------
// Parse SRT timestamp "HH:MM:SS,mmm" to seconds
fn parse_srt_timestamp(ts: &str) -> f32 {
    let parts: Vec<&str> = ts.split(|c| c == ':' || c == ',').collect();
    if parts.len() == 4 {
        let hours: f32 = parts[0].parse().unwrap_or(0.0);
        let minutes: f32 = parts[1].parse().unwrap_or(0.0);
        let seconds: f32 = parts[2].parse().unwrap_or(0.0);
        let millis: f32 = parts[3].parse().unwrap_or(0.0);
        hours * 3600.0 + minutes * 60.0 + seconds + millis / 1000.0
    } else {
        0.0
    }
}

// -----------------------------------------------------------------------------
// Extract a segment of the full SRT that preserves full blocks (index, timestamp, text, blank)
fn extract_srt_segment(
    full_srt: &Path,
    start_sec: f32,
    end_sec: f32,
    output_srt: &Path,
) -> Result<()> {
    let content = std::fs::read_to_string(full_srt).context("reading full SRT")?;
    let mut out_blocks = Vec::new();
    let mut lines = content.lines();

    loop {
        // Read index line
        let index = match lines.next() {
            Some(l) => l.to_string(),
            None => break,
        };

        // Read timestamp line
        let ts_line = match lines.next() {
            Some(l) => l.to_string(),
            None => break,
        };

        // Collect text lines until blank line or EOF
        let mut text_lines = Vec::new();
        for l in &mut lines {
            if l.trim().is_empty() {
                break;
            }
            text_lines.push(l.to_string());
        }

        // Parse timestamps and decide whether to keep block
        if ts_line.contains("-->") {
            let parts: Vec<&str> = ts_line.split("-->").collect();
            if parts.len() == 2 {
                let s = parse_srt_timestamp(parts[0].trim());
                let e = parse_srt_timestamp(parts[1].trim());
                if s < end_sec && e > start_sec {
                    // Keep the whole block
                    let mut block = String::new();
                    block.push_str(&index);
                    block.push('\n');
                    block.push_str(&ts_line);
                    block.push('\n');
                    for tl in text_lines.iter() {
                        block.push_str(tl);
                        block.push('\n');
                    }
                    block.push('\n');
                    out_blocks.push(block);
                }
            }
        }
    }

    std::fs::write(output_srt, out_blocks.join("")).context("writing clip SRT")?;
    Ok(())
}

// -----------------------------------------------------------------------------
// Burn subtitles using `ass` filter (requires SRT->ASS conversion)
fn burn_subtitles_via_ass(video_path: &Path, srt_path: &Path, output_path: &Path) -> Result<()> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;

    // Create a temporary ASS path next to the SRT
    let ass_path = srt_path.with_extension("ass");

    // Convert SRT -> ASS (ffmpeg will create an ASS file)
    let conv_status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(srt_path)
        .arg(&ass_path)
        .status()
        .context("converting srt to ass")?;

    if !conv_status.success() {
        anyhow::bail!("Failed to convert SRT to ASS");
    }

    // Use ass filter (path quoting is simpler)
    let ass_str = std::fs::canonicalize(&ass_path).context("canonicalize ass")?.display().to_string();
    let filter = format!("ass='{}'", ass_str.replace('\'', "\\'"));

    let status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(&filter)
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
        .context("running ffmpeg to burn ass subtitles")?;

    // Optionally remove the temporary ASS file
    let _ = std::fs::remove_file(&ass_path);

    if !status.success() {
        anyhow::bail!("Failed to burn subtitles (ass)");
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Burn subtitles using `subtitles` filter (with styling)
fn burn_subtitles(
    video_path: &Path,
    srt_path: &Path,
    output_path: &Path,
    style: &SubtitleStyle,
) -> Result<()> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;

    // Use absolute path and escape backslashes for Windows
    let srt_abs = std::fs::canonicalize(srt_path).context("canonicalize srt")?;
    let srt_str = srt_abs.display().to_string().replace('\\', "\\\\");

    // Use double quotes around filename for cross-platform robustness
    let filter = format!(
        "subtitles=\"{}\":force_style='FontName={},FontSize={},PrimaryColour={},Outline={},BorderStyle={}'",
        srt_str, style.font, style.size, style.color, style.outline, style.border_style
    );

    let status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(filter)
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
        .context("running ffmpeg to burn subtitles")?;

    if !status.success() {
        anyhow::bail!("Failed to burn subtitles");
    }
    Ok(())
}

// Helper struct to hold subtitle style
#[derive(Clone)]
struct SubtitleStyle {
    font: String,
    size: u32,
    color: String,
    outline: u32,
    border_style: u32,
}

// -----------------------------------------------------------------------------
// Crop to 9:16 (center crop) – robust version, no CSV parsing
fn crop_to_vertical(video_path: &Path, output_path: &Path) -> Result<()> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;
    let ffprobe = which::which("ffprobe").context("ffprobe not found in PATH")?;

    let output = Command::new(&ffprobe)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=width,height")
        .arg("-of")
        .arg("default=noprint_wrappers=1")
        .arg(video_path)
        .output()
        .context("running ffprobe for dimensions")?;

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

    let (width, height) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        _ => anyhow::bail!("Could not parse video dimensions from ffprobe output: {}", stdout),
    };

    let target_width = (height as f32 * 9.0 / 16.0) as u32;
    let crop_width = if width > target_width { target_width } else { width };
    let crop_x = (width - crop_width) / 2;

    let status = Command::new(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("crop={}:{}:{}:0", crop_width, height, crop_x))
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

// -----------------------------------------------------------------------------
// Whisper transcription of full audio (once)
fn transcribe_full_audio(audio_path: &Path, srt_path: &Path, cli: &Cli) -> Result<()> {
    if cli.whisper_mode == "python" {
        println!("{}", "📝 Using Whisper via Python (python -m whisper)...".green());
        let python = which::which("python").context("python not found in PATH")?;
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
            .arg(audio_path.parent().unwrap())
            .status()
            .context("running python whisper")?;
        if !status.success() {
            anyhow::bail!("Whisper transcription failed");
        }
    } else {
        println!("{}", "⚠️  Using binary Whisper – word‑level timestamps may not be available. For full dynamic captions, use `--whisper-mode python`.".yellow());
        let whisper = which::which("whisper").context("whisper not found. Install with: pip install openai-whisper")?;
        let status = Command::new(&whisper)
            .arg(audio_path)
            .arg("--model")
            .arg(&cli.whisper_model)
            .arg("--language")
            .arg("en")
            .arg("--output_format")
            .arg("srt")
            .arg("--output_dir")
            .arg(audio_path.parent().unwrap())
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

// -----------------------------------------------------------------------------
// Detect scene changes using ffprobe
fn detect_scene_changes(video_path: &Path, threshold: f64) -> Result<Vec<f32>> {
    let ffprobe = which::which("ffprobe").context("ffprobe not found in PATH")?;
    let output = Command::new(&ffprobe)
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("select='gt(scene,{})',showinfo", threshold))
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .context("running ffprobe for scene detection")?;

    let stderr = String::from_utf8(output.stderr).context("ffprobe stderr not utf8")?;
    let mut timestamps = Vec::new();
    let re = Regex::new(r"pts_time:([0-9.]+)").context("compile regex")?;

    for line in stderr.lines() {
        if let Some(caps) = re.captures(line) {
            if let Ok(t) = caps[1].parse::<f32>() {
                timestamps.push(t);
            }
        }
    }
    Ok(timestamps)
}

// -----------------------------------------------------------------------------
// Helper to produce ISO and human timestamps based on mode
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

// -----------------------------------------------------------------------------
// Main
fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let (run_ts_iso, run_ts_human) = now_strings(&cli.timestamp_mode);
    let start_total = Instant::now();

    println!("{}", "🎬 ViralClip Swarm".bold().cyan());

    if let Some(ref url) = cli.url {
        println!("{}", format!("URL: {}", url).blue());
    } else if let Some(ref path) = cli.input {
        println!("{}", format!("Input file: {}", path.display()).blue());
    }

    println!("{}", format!("Clips: {}", cli.num_clips).magenta());
    println!("{}", format!("Output: {}", cli.output_dir).magenta());

    if cli.captions {
        println!("{}", format!("✨ Captions enabled (mode: {}, model: {})", cli.whisper_mode, cli.whisper_model).green());
        println!("{}", format!("   Style: font='{}', size={}, color={}, outline={}, border_style={}",
            cli.subtitle_font, cli.subtitle_size, cli.subtitle_color,
            cli.subtitle_outline, cli.subtitle_border_style).bright_blue());
    }
    if cli.crop {
        println!("{}", "📱 Crop to 9:16 enabled".magenta());
    }
    if cli.accurate {
        println!("{}", "🎯 Accurate cuts enabled (slower)".yellow());
    } else {
        println!("{}", "⚡ Fast cuts enabled (copy stream)".yellow());
    }
    if cli.motion {
        println!("{}", format!("🎬 Motion detection enabled (scene threshold: {})", cli.scene_threshold).cyan());
    }
    println!("{}", format!("Subtitles mode: {}", cli.subtitles_mode).cyan());
    println!("{}", format!("Benchmark path: {} (format: {}, append: {})", cli.csv_path, cli.csv_format, cli.append).cyan());
    println!("{}", format!("Timestamps: {}", cli.timestamp_mode).cyan());

    // Temporary directory
    let temp_dir: TempDir = tempfile::tempdir().context("create temp dir")?;
    let temp_path = temp_dir.path();

    // Obtain video file (local or downloaded)
    let video_path = if let Some(ref input_path) = cli.input {
        let dest = temp_path.join(input_path.file_name().unwrap());
        std::fs::copy(&input_path, &dest).context("copy input to temp")?;
        dest
    } else if let Some(ref url) = cli.url {
        println!("{}", "📥 Downloading video...".cyan());
        download_video(url, temp_path)?
    } else {
        anyhow::bail!("Either --url or --input must be provided");
    };
    println!("{}", format!("✅ Video ready: {}", video_path.display()).green());

    // Extract full audio for energy analysis (and captions if needed)
    let wav_path = temp_path.join("audio.wav");
    let t_audio = Instant::now();
    println!("{}", "🎵 Extracting audio...".blue());
    extract_audio(&video_path, &wav_path)?;
    println!("{}", format!("✅ Audio extracted in {:.2?}", t_audio.elapsed()).green());

    // Optional: transcribe full audio once
    let full_srt = if cli.captions {
        let srt_path = temp_path.join("full.srt");
        let t_trans = Instant::now();
        println!("{}", "📝 Transcribing audio with Whisper...".green());
        transcribe_full_audio(&wav_path, &srt_path, &cli)?;
        println!("{}", format!("✅ Transcription finished in {:.2?}", t_trans.elapsed()).green());
        Some(srt_path)
    } else {
        None
    };

    // Build subtitle style if captions are enabled
    let subtitle_style = if cli.captions {
        Some(SubtitleStyle {
            font: cli.subtitle_font.clone(),
            size: cli.subtitle_size,
            color: cli.subtitle_color.clone(),
            outline: cli.subtitle_outline,
            border_style: cli.subtitle_border_style,
        })
    } else {
        None
    };

    // Analyze energy and optionally boost with scene changes
    println!("{}", "🔊 Analyzing audio energy...".cyan());
    let mut energies = analyze_energy(&wav_path, 1.0)?;

    if cli.motion {
        println!("{}", "🎬 Detecting scene changes...".cyan());
        let scene_timestamps = detect_scene_changes(&video_path, cli.scene_threshold)?;
        println!("   Found {} scene changes", scene_timestamps.len());

        // Boost energy at scene change seconds
        let window_secs = 1.0;
        for ts in scene_timestamps {
            let idx = (ts / window_secs) as usize;
            if idx < energies.len() {
                energies[idx] = energies[idx].max(1.0);
            }
        }
        println!("{}", "✅ Energy boosted at scene change positions".green());
    }

    // Select top windows
    let mut indexed: Vec<(usize, f32)> = energies.iter().enumerate().map(|(i, &e)| (i, e)).collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    let num_clips = cli.num_clips as usize;
    let mut selected = Vec::new();
    let min_gap = cli.min_duration as usize;
    for (idx, energy) in indexed {
        let mut overlap = false;
        for (sel_idx, _) in &selected {
            if (*sel_idx as i32 - idx as i32).abs() < min_gap as i32 {
                overlap = true;
                break;
            }
        }
        if !overlap {
            selected.push((idx, energy));
            if selected.len() >= num_clips {
                break;
            }
        }
    }

    if selected.is_empty() {
        anyhow::bail!("No clips selected");
    }

    println!("{}", format!("🎯 Selected {} clips:", selected.len()).bold());
    for (i, (idx, energy)) in selected.iter().enumerate() {
        let start_sec = *idx as f32;
        let end_sec = start_sec + cli.min_duration;
        println!("   clip {}: {:.1}s – {:.1}s (score: {:.2})", i + 1, start_sec, end_sec, energy);
    }

    // Create output directory
    std::fs::create_dir_all(&cli.output_dir).context("create output dir")?;

    // Prepare tasks for parallel processing
    let tasks: Vec<_> = selected
        .iter()
        .enumerate()
        .map(|(i, (idx, _))| {
            let start_sec = *idx as f32;
            let end_sec = start_sec + cli.min_duration;
            (start_sec, end_sec, i + 1)
        })
        .collect();

    // Use a Mutex to collect errors without panicking and to collect timings
    let errors = Arc::new(Mutex::new(Vec::new()));
    let timings = Arc::new(Mutex::new(Vec::<ClipTiming>::new()));

    // --- Prepare owned/cloned values for parallel workers ---
    let min_duration = cli.min_duration;
    let accurate_mode = cli.accurate;
    let crop_enabled = cli.crop;
    let subtitles_mode = cli.subtitles_mode.clone();
    let output_dir = cli.output_dir.clone();
    let timestamp_mode = cli.timestamp_mode.clone();

    let video_path = video_path.clone();
    let temp_path_buf = temp_path.to_path_buf();

    let full_srt_clone = full_srt.clone();
    let subtitle_style_clone = subtitle_style.clone();

    let errors_cloned = Arc::clone(&errors);
    let timings_cloned = Arc::clone(&timings);

    println!("{}", "⚡ Processing clips in parallel...".yellow());
    tasks.par_iter().for_each(|(start_sec, end_sec, i)| {
        let result = (|| -> Result<()> {
            let t_total_start = Instant::now();
            let raw_clip = temp_path_buf.join(format!("clip_{}_raw.mp4", i));
            let final_output = PathBuf::from(&output_dir).join(format!("clip_{}.mp4", i));

            // Extract raw clip
            let t0 = Instant::now();
            extract_clip(&video_path, *start_sec, min_duration, &raw_clip, accurate_mode)?;
            let extract_ms = t0.elapsed().as_millis();
            println!("{}", format!("🎬 Clip {} extracted in {:.2?}", i, t0.elapsed()).yellow());

            let mut current = raw_clip;
            let mut subtitles_ms = 0u128;
            let mut crop_ms = 0u128;
            let mut success = true;
            let mut error_msg = String::new();

            // If captions are enabled, burn them using the pre‑computed SRT segment
            if let (Some(ref full_srt_path), Some(ref style)) = (full_srt_clone.as_ref(), subtitle_style_clone.as_ref()) {
                let clip_srt = temp_path_buf.join(format!("clip_{}.srt", i));
                if let Err(e) = extract_srt_segment(full_srt_path, *start_sec, *end_sec, &clip_srt) {
                    println!("{}", format!("⚠️ Failed to extract SRT for clip {}: {}", i, e).yellow());
                } else {
                    if clip_srt.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
                        println!("{}", format!("⚠️ No subtitles for clip {}, skipping", i).yellow());
                    } else {
                        let captioned = temp_path_buf.join(format!("clip_{}_captioned.mp4", i));
                        let t1 = Instant::now();

                        // Choose subtitle burn method based on CLI flag
                        let burn_result = match subtitles_mode.as_str() {
                            "ass" => burn_subtitles_via_ass(&current, &clip_srt, &captioned),
                            "subtitles" => burn_subtitles(&current, &clip_srt, &captioned, style),
                            "auto" => match burn_subtitles(&current, &clip_srt, &captioned, style) {
                                Ok(_) => Ok(()),
                                Err(_) => burn_subtitles_via_ass(&current, &clip_srt, &captioned),
                            },
                            _ => burn_subtitles_via_ass(&current, &clip_srt, &captioned),
                        };

                        if let Err(e) = burn_result {
                            success = false;
                            error_msg = format!("Failed to burn subtitles: {}", e);
                            println!("{}", format!("⚠️ {}", error_msg).yellow());
                        } else {
                            subtitles_ms = t1.elapsed().as_millis();
                            println!("{}", format!("📝 Subtitles burned for clip {} in {:.2?}", i, t1.elapsed()).magenta());
                            let _ = std::fs::remove_file(&current);
                            current = captioned;
                        }
                    }
                }
            }

            // If crop is enabled, crop to 9:16
            if crop_enabled {
                let cropped = temp_path_buf.join(format!("clip_{}_cropped.mp4", i));
                let t2 = Instant::now();
                match crop_to_vertical(&current, &cropped) {
                    Ok(_) => {
                        crop_ms = t2.elapsed().as_millis();
                        println!("{}", format!("📱 Cropping finished for clip {} in {:.2?}", i, t2.elapsed()).magenta());
                        let _ = std::fs::remove_file(&current);
                        current = cropped;
                    }
                    Err(e) => {
                        println!("{}", format!("⚠️ Crop failed for clip {}: {}. Using original.", i, e).yellow());
                    }
                }
            }

            // Move final file to output
            if let Err(e) = std::fs::rename(&current, &final_output) {
                success = false;
                error_msg = format!("rename final clip failed: {}", e);
            } else {
                println!("{}", format!("✅ Clip {} saved to {}", i, final_output.display()).green());
            }

            let total_ms = t_total_start.elapsed().as_millis();
            let (ts_iso, ts_human) = now_strings(&timestamp_mode);

            // Record timing
            let record = ClipTiming {
                clip_id: *i,
                start_sec: *start_sec,
                duration: min_duration,
                extract_ms,
                subtitles_ms,
                crop_ms,
                total_ms,
                success,
                error: error_msg,
                timestamp: ts_iso,
                timestamp_human: ts_human,
            };

            {
                let mut guard = timings_cloned.lock().unwrap();
                guard.push(record);
            }

            // avoid unused warning
            let _ = total_ms;

            Ok(())
        })();

        if let Err(e) = result {
            let mut errors = errors_cloned.lock().unwrap();
            errors.push(format!("Clip {}: {}", i, e));
        }
    });

    // Write CSV/JSON/Human results with append support
    {
        let rows = timings.lock().unwrap();
        let path = Path::new(&cli.csv_path);

        let summary = RunSummary {
            run_timestamp: run_ts_iso.clone(),
            run_timestamp_human: run_ts_human.clone(),
            total_clips: rows.len(),
            total_duration_ms: start_total.elapsed().as_millis(),
        };

        match cli.csv_format.as_str() {
            "csv" => {
                // CSV: write a commented summary header (human readable) then consistent clip rows
                let exists = path.exists();

                // Ensure parent dir exists
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).context("create csv parent dir")?;
                }

                if !cli.append || !exists {
                    // Create/truncate file, write commented summary header, then CSV with headers
                    let mut header_file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(&path)
                        .context("create csv file")?;

                    let summary_text = format!(
                        "# Run started: {} ({})\n# Total clips: {}, Total duration: {:.2}s\n",
                        summary.run_timestamp,
                        summary.run_timestamp_human,
                        summary.total_clips,
                        summary.total_duration_ms as f64 / 1000.0
                    );
                    header_file.write_all(summary_text.as_bytes()).context("write csv summary header")?;

                    // Now create CSV writer that will append rows after the commented header
                    let mut wtr = WriterBuilder::new().has_headers(true).from_writer(header_file);
                    for r in rows.iter() {
                        wtr.serialize(r).context("serialize timing row")?;
                    }
                    wtr.flush().context("flush csv")?;
                } else {
                    // Append mode and file exists: open for append and do NOT write headers
                    let file = OpenOptions::new().append(true).open(&path).context("open csv for append")?;
                    let mut wtr = WriterBuilder::new().has_headers(false).from_writer(file);
                    for r in rows.iter() {
                        wtr.serialize(r).context("serialize timing row")?;
                    }
                    wtr.flush().context("flush csv")?;
                }

                println!("{}", format!("📊 CSV written to {}", path.display()).cyan());
            }

            "json" => {
                if cli.append && path.exists() {
                    // read existing array of runs or create new
                    let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
                    let mut all_runs: Vec<serde_json::Value> = serde_json::from_str(&existing).unwrap_or_default();
                    let mut bundle = serde_json::Map::new();
                    bundle.insert("summary".to_string(), serde_json::to_value(&summary).unwrap());
                    bundle.insert("clips".to_string(), serde_json::to_value(&*rows).unwrap());
                    all_runs.push(serde_json::Value::Object(bundle));
                    let json = serde_json::to_string_pretty(&all_runs).context("serialize to json")?;
                    std::fs::write(&path, json).context("write json file")?;
                } else {
                    let mut bundle = serde_json::Map::new();
                    bundle.insert("summary".to_string(), serde_json::to_value(&summary).context("summary to value")?);
                    bundle.insert("clips".to_string(), serde_json::to_value(&*rows).context("clips to value")?);
                    let json = serde_json::to_string_pretty(&bundle).context("serialize to json")?;
                    std::fs::write(&path, json).context("write json file")?;
                }
                println!("{}", format!("📊 JSON written to {}", path.display()).cyan());
            }

            "human" => {
                // Build human-readable table by appending formatted strings to a String
                let mut table = String::new();
                table.push_str(&format!("Run started: {} ({})", summary.run_timestamp, summary.run_timestamp_human));
                table.push('\n');
                table.push_str(&format!(
                    "Total clips: {}, Total duration: {:.2}s",
                    summary.total_clips,
                    summary.total_duration_ms as f64 / 1000.0
                ));
                table.push('\n');
                table.push_str("clip_id | start | dur | extract(s) | subs(s) | crop(s) | total(s) | success | timestamp | human_time | error");
                table.push('\n');

                for r in rows.iter() {
                    table.push_str(&format!(
                        "{:>7} | {:>5.1} | {:>3.1} | {:>10.3} | {:>8.3} | {:>7.3} | {:>8.3} | {:>7} | {} | {} | {}",
                        r.clip_id,
                        r.start_sec,
                        r.duration,
                        r.extract_ms as f64 / 1000.0,
                        r.subtitles_ms as f64 / 1000.0,
                        r.crop_ms as f64 / 1000.0,
                        r.total_ms as f64 / 1000.0,
                        r.success,
                        r.timestamp,
                        r.timestamp_human,
                        r.error
                    ));
                    table.push('\n');
                }

                if cli.append && path.exists() {
                    let mut file = OpenOptions::new().append(true).open(&path).context("open human file for append")?;
                    file.write_all(table.as_bytes()).context("append human table")?;
                } else {
                    std::fs::write(&path, table).context("write human table")?;
                }
                println!("{}", format!("📊 Human-readable table written to {}", path.display()).cyan());
            }

            other => {
                anyhow::bail!("Unknown csv-format: {}", other);
            }
        } // end match
    } // end logging block

    // Report errors if any
    let errors = errors.lock().unwrap();
    if !errors.is_empty() {
        println!("{}", "⚠️ The following clips encountered errors:".yellow());
        for err in errors.iter() {
            println!("   {}", err);
        }
        println!("{}", "Other clips were processed successfully.".yellow());
    }

    println!("{}", format!("⏱ Total pipeline time: {:.2?}", start_total.elapsed()).cyan());
    Ok(())
}//____end of main