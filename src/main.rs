use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Context, Result};
use which::which;
use hound::WavReader;
use rayon::prelude::*;

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

    /// Whisper model: tiny, base, small, medium, large
    #[arg(long, default_value = "base")]
    model: String,
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
        .status()?;

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
        .status()?;

    if !status.success() {
        anyhow::bail!("ffmpeg audio extraction failed");
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Analyze energy (RMS) per window_secs
fn analyze_energy(wav_path: &Path, window_secs: f32) -> Result<Vec<f32>> {
    let reader = WavReader::open(wav_path)?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let window_size = (window_secs * sample_rate as f32) as usize;

    let samples: Vec<i16> = reader.into_samples().map(|s| s.unwrap()).collect();
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
    let ffmpeg = which::which("ffmpeg")?;
    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-i")
        .arg(video_path)
        .arg("-ss")
        .arg(start_sec.to_string())
        .arg("-t")
        .arg(duration_sec.to_string());

    if accurate {
        cmd.arg("-c:v").arg("libx264").arg("-c:a").arg("aac");
    } else {
        cmd.arg("-c").arg("copy");
    }
    cmd.arg(output_path);

    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("ffmpeg clip extraction failed");
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Whisper transcription of full audio (once)
fn transcribe_full_audio(audio_path: &Path, srt_path: &Path, model: &str) -> Result<()> {
    let whisper = which::which("whisper").context("whisper not found. Install with: pip install openai-whisper")?;
    let status = Command::new(&whisper)
        .arg(audio_path)
        .arg("--model")
        .arg(model)
        .arg("--language")
        .arg("en")
        .arg("--word_timestamps")
        .arg("True")
        .arg("--output_format")
        .arg("srt")
        .arg("--output_dir")
        .arg(audio_path.parent().unwrap())
        .status()?;

    if !status.success() {
        anyhow::bail!("Whisper transcription failed");
    }
    let generated = audio_path.with_extension("srt");
    std::fs::rename(&generated, srt_path)?;
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
// Extract a segment of the full SRT that overlaps with [start, end]
fn extract_srt_segment(
    full_srt: &Path,
    start_sec: f32,
    end_sec: f32,
    output_srt: &Path,
) -> Result<()> {
    let content = std::fs::read_to_string(full_srt)?;
    let mut out_lines = Vec::new();
    let mut in_segment = false;

    for line in content.lines() {
        if line.contains("-->") {
            // Parse the timestamp line
            let parts: Vec<&str> = line.split("-->").collect();
            if parts.len() == 2 {
                let start_str = parts[0].trim();
                let end_str = parts[1].trim();
                let line_start = parse_srt_timestamp(start_str);
                let line_end = parse_srt_timestamp(end_str);
                if line_start < end_sec && line_end > start_sec {
                    in_segment = true;
                    out_lines.push(line);
                } else {
                    in_segment = false;
                }
            } else {
                out_lines.push(line);
            }
        } else if in_segment {
            out_lines.push(line);
        }
    }

    // Write the segment (may need to renumber entries; but burning works with any order)
    std::fs::write(output_srt, out_lines.join("\n"))?;
    Ok(())
}

// -----------------------------------------------------------------------------
// Burn subtitles into video (re‑encodes)
fn burn_subtitles(video_path: &Path, srt_path: &Path, output_path: &Path) -> Result<()> {
    let ffmpeg = which::which("ffmpeg")?;
    let status = Command::new(&ffmpeg)
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("subtitles={}", srt_path.display()))
        .arg("-c:v")
        .arg("libx264")
        .arg("-c:a")
        .arg("aac")
        .arg(output_path)
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to burn subtitles");
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Crop to 9:16 (center crop)
fn crop_to_vertical(video_path: &Path, output_path: &Path) -> Result<()> {
    let ffmpeg = which::which("ffmpeg")?;
    let ffprobe = which::which("ffprobe").context("ffprobe not found in PATH")?;

    let output = Command::new(&ffprobe)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=width,height")
        .arg("-of")
        .arg("csv=p=0")
        .arg(video_path)
        .output()?;

    let dims = String::from_utf8(output.stdout)?;
    let dims: Vec<&str> = dims.trim().split(',').collect();
    if dims.len() != 2 {
        anyhow::bail!("Could not parse video dimensions");
    }
    let width: u32 = dims[0].parse()?;
    let height: u32 = dims[1].parse()?;

    let target_width = (height as f32 * 9.0 / 16.0) as u32;
    let crop_width = if width > target_width { target_width } else { width };
    let crop_x = (width - crop_width) / 2;

    let status = Command::new(&ffmpeg)
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("crop={}:{}:{}:0", crop_width, height, crop_x))
        .arg("-c:v")
        .arg("libx264")
        .arg("-c:a")
        .arg("aac")
        .arg(output_path)
        .status()?;

    if !status.success() {
        anyhow::bail!("Crop failed");
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Main
fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    println!("🎬 ViralClip Swarm");
    if let Some(ref url) = cli.url {
        println!("URL: {}", url);
    } else if let Some(ref path) = cli.input {
        println!("Input file: {}", path.display());
    }
    println!("Clips: {}", cli.num_clips);
    println!("Output: {}", cli.output_dir);
    if cli.captions {
        println!("✨ Captions enabled (model: {})", cli.model);
    }
    if cli.crop {
        println!("📱 Crop to 9:16 enabled");
    }
    if cli.accurate {
        println!("🎯 Accurate cuts enabled (slower)");
    } else {
        println!("⚡ Fast cuts enabled (copy stream)");
    }

    // Temporary directory
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.path();

    // Obtain video file (local or downloaded)
    let video_path = if let Some(input_path) = cli.input {
        let dest = temp_path.join(input_path.file_name().unwrap());
        std::fs::copy(&input_path, &dest)?;
        dest
    } else if let Some(url) = cli.url {
        println!("📥 Downloading video...");
        download_video(&url, temp_path)?
    } else {
        anyhow::bail!("Either --url or --input must be provided");
    };
    println!("✅ Video ready: {}", video_path.display());

    // Extract full audio for energy analysis (and captions if needed)
    let wav_path = temp_path.join("audio.wav");
    println!("🎵 Extracting audio...");
    extract_audio(&video_path, &wav_path)?;

    // Optional: transcribe full audio once
    let full_srt = if cli.captions {
        let srt_path = temp_path.join("full.srt");
        println!("📝 Transcribing audio with Whisper (model: {})...", cli.model);
        transcribe_full_audio(&wav_path, &srt_path, &cli.model)?;
        Some(srt_path)
    } else {
        None
    };

    // Analyze energy and select top windows
    println!("🔊 Analyzing audio energy...");
    let energies = analyze_energy(&wav_path, 1.0)?;

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

    println!("🎯 Selected {} clips:", selected.len());
    for (i, (idx, energy)) in selected.iter().enumerate() {
        let start_sec = *idx as f32;
        let end_sec = start_sec + cli.min_duration;
        println!("   clip {}: {:.1}s – {:.1}s (energy: {:.2})", i+1, start_sec, end_sec, energy);
    }

    // Create output directory
    std::fs::create_dir_all(&cli.output_dir)?;

    // Prepare tasks for parallel processing
    let tasks: Vec<_> = selected.iter().enumerate().map(|(i, (idx, _))| {
        let start_sec = *idx as f32;
        let end_sec = start_sec + cli.min_duration;
        (start_sec, end_sec, i+1)
    }).collect();

    // Use rayon to process clips in parallel
    println!("⚡ Processing clips in parallel...");
    tasks.par_iter().try_for_each(|(start_sec, end_sec, i)| -> Result<()> {
        let raw_clip = temp_path.join(format!("clip_{}_raw.mp4", i));
        let final_output = PathBuf::from(&cli.output_dir).join(format!("clip_{}.mp4", i));

        // Extract raw clip
        extract_clip(&video_path, *start_sec, cli.min_duration, &raw_clip, cli.accurate)?;

        let mut current = raw_clip;

        // If captions are enabled, burn them using the pre‑computed SRT segment
        if let Some(ref full_srt) = full_srt {
            let clip_srt = temp_path.join(format!("clip_{}.srt", i));
            extract_srt_segment(full_srt, *start_sec, *end_sec, &clip_srt)?;
            let captioned = temp_path.join(format!("clip_{}_captioned.mp4", i));
            burn_subtitles(&current, &clip_srt, &captioned)?;
            std::fs::remove_file(&current)?;
            current = captioned;
        }

        // If crop is enabled, crop to 9:16
        if cli.crop {
            let cropped = temp_path.join(format!("clip_{}_cropped.mp4", i));
            crop_to_vertical(&current, &cropped)?;
            std::fs::remove_file(&current)?;
            current = cropped;
        }

        // Move final file to output
        std::fs::rename(&current, &final_output)?;
        println!("✅ Clip {} saved", i);
        Ok(())
    })?;

    println!("✅ All clips saved in {}", cli.output_dir);
    Ok(())
}