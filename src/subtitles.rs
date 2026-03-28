use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::runtime::command_output_checked;

#[derive(Clone, Debug)]
pub struct SubtitleStyle {
    pub font: String,
    pub size: u32,
    pub color: String,
    pub highlight_color: String,
    pub outline_color: String,
    pub back_color: String,
    pub outline: u32,
    pub shadow: u32,
    pub border_style: u32,
    pub bold: bool,
    pub alignment: u32,
    pub margin_v: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubtitleAnimationPreset {
    None,
    Karaoke,
    Emphasis,
    Impact,
    Pulse,
    CreatorPro,
}

#[derive(Clone, Debug)]
pub struct SubtitleRenderOptions {
    pub template: String,
    pub emoji_layer: bool,
    pub beat_sync: bool,
    pub scene_score: f32,
}

impl Default for SubtitleRenderOptions {
    fn default() -> Self {
        Self {
            template: "classic".to_string(),
            emoji_layer: false,
            beat_sync: false,
            scene_score: 0.0,
        }
    }
}

fn ffmpeg_filter_escape<S: AsRef<OsStr>>(s: S) -> String {
    let mut s = s.as_ref().to_string_lossy().into_owned();
    s = s.replace('\\', "/");
    s = s.replace('\'', r"\'");
    if s.len() >= 2 {
        let mut chars = s.chars();
        let first = chars.next().unwrap();
        let second = chars.next().unwrap();
        if first.is_ascii_alphabetic() && second == ':' {
            s = format!("{}\\:{}", &s[..1], &s[2..]);
        }
    }
    s
}

fn normalize_windows_extended_prefix(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else if let Some(stripped) = path.strip_prefix("//?/") {
        stripped.to_string()
    } else {
        path.to_string()
    }
}

fn convert_srt_to_ass(srt_path: &Path) -> Result<PathBuf> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;
    let ass_path = srt_path.with_extension("ass");
    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(srt_path)
        .arg(&ass_path);
    command_output_checked(&mut cmd, "converting srt to ass with ffmpeg")?;
    Ok(ass_path)
}

fn ass_bool(value: bool) -> i32 {
    if value {
        -1
    } else {
        0
    }
}

fn ass_escape_text(value: &str) -> String {
    value
        .replace('\\', r"\\")
        .replace('{', r"\{")
        .replace('}', r"\}")
}

fn estimate_syllables(word: &str) -> usize {
    let lowered = word
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    if lowered.is_empty() {
        return 1;
    }
    let vowels = lowered
        .chars()
        .fold((0usize, false), |(count, prev_vowel), ch| {
            let is_vowel = matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
            let mut next = count;
            if is_vowel && !prev_vowel {
                next += 1;
            }
            (next, is_vowel)
        })
        .0;
    vowels.max(1)
}

fn beat_duration_cs(word: &str, preset: SubtitleAnimationPreset, beat_sync: bool) -> usize {
    if !beat_sync {
        return if preset == SubtitleAnimationPreset::CreatorPro {
            11
        } else {
            10
        };
    }
    let syllables = estimate_syllables(word);
    let punctuation_bonus = if word.ends_with([',', ';', ':']) {
        2
    } else if word.ends_with(['.', '!', '?']) {
        4
    } else {
        0
    };
    (6 + (syllables * 3) + punctuation_bonus).clamp(6, 24)
}

fn line_break_indices(words: &[&str], preset: SubtitleAnimationPreset) -> HashSet<usize> {
    let max_words_per_line = match preset {
        SubtitleAnimationPreset::CreatorPro => 5usize,
        SubtitleAnimationPreset::Impact => 4usize,
        SubtitleAnimationPreset::Emphasis => 5usize,
        _ => 6usize,
    };
    let mut breaks = HashSet::new();
    let mut current_count = 0usize;
    for (index, word) in words.iter().enumerate() {
        if index == 0 {
            current_count = 1;
            continue;
        }
        let punctuation_break = word.ends_with([',', ';', ':']) || word.ends_with(['.', '!', '?']);
        if current_count >= max_words_per_line || punctuation_break {
            breaks.insert(index);
            current_count = 1;
        } else {
            current_count += 1;
        }
    }
    breaks
}

fn emoji_for_text(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("money") || lower.contains("revenue") || lower.contains("sale") {
        Some("💸")
    } else if lower.contains("secret") || lower.contains("truth") {
        Some("🤫")
    } else if lower.contains("warning") || lower.contains("mistake") || lower.contains("risk") {
        Some("⚠️")
    } else if lower.contains("crazy") || lower.contains("insane") || lower.contains("viral") {
        Some("🔥")
    } else if lower.contains("win") || lower.contains("best") || lower.contains("growth") {
        Some("🚀")
    } else {
        None
    }
}

fn transition_tag(dialogue_index: usize, scene_score: f32) -> &'static str {
    if scene_score > 0.7 {
        match dialogue_index % 4 {
            0 => r"{\fad(10,45)\t(0,90,\fscx118\fscy118)\t(90,180,\fscx100\fscy100)}",
            1 => r"{\fad(20,50)\t(0,90,\alpha&H18&)\t(90,190,\alpha&H00&)}",
            2 => r"{\fad(15,40)\t(0,110,\bord7)\t(110,190,\bord5)}",
            _ => r"{\fad(12,45)\t(0,90,\blur2)\t(90,170,\blur1)}",
        }
    } else {
        match dialogue_index % 3 {
            0 => r"{\fad(28,65)\t(0,120,\fscx105\fscy105)\t(120,220,\fscx100\fscy100)}",
            1 => r"{\fad(24,60)\t(0,120,\alpha&H20&)\t(120,220,\alpha&H00&)}",
            _ => r"{\fad(30,70)\t(0,120,\bord5)\t(120,220,\bord4)}",
        }
    }
}

fn scene_overlay_tag(options: &SubtitleRenderOptions) -> String {
    let mut tags = String::new();
    if options.scene_score > 0.6 {
        tags.push_str(r"\t(0,120,\fscx108\fscy108)");
    }
    if options.scene_score > 0.8 {
        tags.push_str(r"\t(0,100,\1c&H0000F6FF&)\t(100,220,\1c&H00FFFFFF&)");
    }
    tags
}

fn animate_ass_text(
    text: &str,
    preset: SubtitleAnimationPreset,
    style: &SubtitleStyle,
    options: &SubtitleRenderOptions,
) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 1 {
        return ass_escape_text(text);
    }

    let breaks = line_break_indices(&words, preset);
    let mut out = String::new();
    for (index, word) in words.iter().enumerate() {
        let should_break = breaks.contains(&index);
        if should_break {
            out.push_str(r"\N");
        }
        if index > 0 && !should_break {
            out.push(' ');
        }
        let escaped = ass_escape_text(word);
        let beat_cs = beat_duration_cs(word, preset, options.beat_sync);
        match preset {
            SubtitleAnimationPreset::Karaoke => {
                out.push_str(&format!(
                    r"{{\1c{}\k{}}}{}",
                    style.highlight_color, beat_cs, escaped
                ));
            }
            SubtitleAnimationPreset::Emphasis => {
                out.push_str(&format!(
                    r"{{\1c{}\k{}\fscx115\fscy115\t(0,120,\fscx100\fscy100)}}{}",
                    style.highlight_color, beat_cs, escaped
                ));
            }
            SubtitleAnimationPreset::Impact => {
                out.push_str(&format!(
                    r"{{\1c{}\k{}\bord5\fscx130\fscy130\t(0,140,\fscx100\fscy100)}}{}",
                    style.highlight_color, beat_cs, escaped
                ));
            }
            SubtitleAnimationPreset::Pulse => {
                out.push_str(&format!(
                    r"{{\1c{}\k{}\t(0,90,\alpha&H20&)\t(90,180,\alpha&H00&)}}{}",
                    style.highlight_color, beat_cs, escaped
                ));
            }
            SubtitleAnimationPreset::CreatorPro => {
                out.push_str(&format!(
                    r"{{\1c{}\k{}\bord4\shad0\blur1\fscx95\fscy95\t(0,70,\fscx122\fscy122)\t(70,160,\fscx100\fscy100)}}{}",
                    style.highlight_color, beat_cs, escaped
                ));
            }
            SubtitleAnimationPreset::None => out.push_str(&escaped),
        }
    }
    out
}

fn apply_animation_to_ass(
    ass_path: &Path,
    preset: SubtitleAnimationPreset,
    style: &SubtitleStyle,
    options: &SubtitleRenderOptions,
) -> Result<()> {
    if preset == SubtitleAnimationPreset::None {
        return Ok(());
    }
    let raw = std::fs::read_to_string(ass_path)
        .with_context(|| format!("read ass file {}", ass_path.display()))?;
    let mut updated = Vec::new();
    let mut dialogue_index = 0usize;
    for line in raw.lines() {
        if let Some(prefix_end) = line.rfind(',') {
            if line.starts_with("Dialogue:") {
                let prefix = &line[..=prefix_end];
                let text = line[prefix_end + 1..].trim();
                let animated = animate_ass_text(text, preset, style, options);
                let emoji = if options.emoji_layer {
                    emoji_for_text(text)
                        .map(|icon| format!(r"{{\fscx95\fscy95\alpha&H08&}}{icon} "))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let transition = transition_tag(dialogue_index, options.scene_score);
                let scene_overlay = scene_overlay_tag(options);
                let template_boost = if options.template.contains("neon") {
                    r"{\blur2\bord7}"
                } else if options.template.contains("minimal") {
                    r"{\blur0\bord3}"
                } else if options.template.contains("bold") {
                    r"{\blur1\bord8}"
                } else {
                    ""
                };
                let scene_block = if scene_overlay.is_empty() {
                    String::new()
                } else {
                    format!("{{{scene_overlay}}}")
                };
                updated.push(format!(
                    "{prefix}{transition}{scene_block}{template_boost}{emoji}{animated}"
                ));
                dialogue_index += 1;
                continue;
            }
        }
        updated.push(line.to_string());
    }
    std::fs::write(ass_path, updated.join("\n"))
        .with_context(|| format!("write animated ass file {}", ass_path.display()))?;
    Ok(())
}

fn apply_style_to_ass(ass_path: &Path, style: &SubtitleStyle) -> Result<()> {
    let raw = std::fs::read_to_string(ass_path)
        .with_context(|| format!("read ass file {}", ass_path.display()))?;
    let mut updated = Vec::new();
    let mut replaced = false;
    for line in raw.lines() {
        if line.starts_with("Style: Default,") {
            updated.push(format!(
                "Style: Default,{},{},{},{},{},{},{},0,0,0,100,100,0,0,{},{},{},{},20,20,{},1",
                style.font,
                style.size,
                style.color,
                style.highlight_color,
                style.outline_color,
                style.back_color,
                ass_bool(style.bold),
                style.border_style,
                style.outline,
                style.shadow,
                style.alignment,
                style.margin_v
            ));
            replaced = true;
        } else {
            updated.push(line.to_string());
        }
    }
    if !replaced {
        anyhow::bail!("ASS file is missing a default style line");
    }
    std::fs::write(ass_path, updated.join("\n"))
        .with_context(|| format!("write styled ass file {}", ass_path.display()))?;
    Ok(())
}

pub fn burn_subtitles_via_ass(
    video_path: &Path,
    srt_path: &Path,
    output_path: &Path,
    style: Option<&SubtitleStyle>,
    animation: SubtitleAnimationPreset,
    render_options: Option<&SubtitleRenderOptions>,
) -> Result<()> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;
    let ass_path = convert_srt_to_ass(srt_path).context("SRT->ASS conversion failed")?;
    let fallback_style = SubtitleStyle {
        font: "Arial".to_string(),
        size: 24,
        color: "&H00FFFFFF".to_string(),
        highlight_color: "&H0000F6FF".to_string(),
        outline_color: "&H00000000".to_string(),
        back_color: "&H38000000".to_string(),
        outline: 2,
        shadow: 0,
        border_style: 1,
        bold: false,
        alignment: 2,
        margin_v: 72,
    };
    let style_ref = style.unwrap_or(&fallback_style);
    let fallback_render = SubtitleRenderOptions::default();
    let render_ref = render_options.unwrap_or(&fallback_render);
    apply_style_to_ass(&ass_path, style_ref).context("apply subtitle style to ass")?;
    apply_animation_to_ass(&ass_path, animation, style_ref, render_ref)
        .context("apply subtitle animation to ass")?;

    let ass_canon = std::fs::canonicalize(&ass_path).context("canonicalize ass path")?;
    let mut ass_str = ass_canon.display().to_string();
    ass_str = normalize_windows_extended_prefix(&ass_str);
    let ass_str = ffmpeg_filter_escape(ass_str);

    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("ass='{}'", ass_str))
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
        .arg(output_path);

    let output = cmd
        .output()
        .with_context(|| "running ffmpeg to burn ass subtitles".to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to burn subtitles (ass): {}", stderr.trim());
    }
    let _ = std::fs::remove_file(&ass_path);
    Ok(())
}

pub fn burn_subtitles(
    video_path: &Path,
    srt_path: &Path,
    output_path: &Path,
    style: &SubtitleStyle,
) -> Result<()> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;
    let srt_abs = std::fs::canonicalize(srt_path).context("canonicalize srt path")?;
    let mut srt_str = srt_abs.display().to_string();
    srt_str = normalize_windows_extended_prefix(&srt_str);
    let srt_escaped = ffmpeg_filter_escape(srt_str);

    let force_style = format!(
        "FontName={},FontSize={},PrimaryColour={},OutlineColour={},BackColour={},Outline={},Shadow={},BorderStyle={},Bold={},Alignment={},MarginV={}",
        style.font,
        style.size,
        style.color,
        style.outline_color,
        style.back_color,
        style.outline,
        style.shadow,
        style.border_style,
        ass_bool(style.bold),
        style.alignment,
        style.margin_v
    );
    let force_style_escaped = force_style.replace('\'', r"\'");
    let filter = format!(
        "subtitles='{}':force_style='{}'",
        srt_escaped, force_style_escaped
    );

    let mut cmd = Command::new(&ffmpeg);
    cmd.arg("-hide_banner")
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
        .arg(output_path);

    let output = cmd
        .output()
        .with_context(|| "running ffmpeg to burn subtitles".to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to burn subtitles: {}", stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{animate_ass_text, SubtitleAnimationPreset, SubtitleRenderOptions, SubtitleStyle};

    fn style() -> SubtitleStyle {
        SubtitleStyle {
            font: "Arial".to_string(),
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
        }
    }

    #[test]
    fn karaoke_animation_injects_k_tags() {
        let animated = animate_ass_text(
            "hello world now",
            SubtitleAnimationPreset::Karaoke,
            &style(),
            &SubtitleRenderOptions::default(),
        );
        assert!(animated.contains(r"{\1c&H0000F6FF\k"));
    }

    #[test]
    fn creator_pro_animation_injects_transform_tags() {
        let animated = animate_ass_text(
            "this is a strong hook",
            SubtitleAnimationPreset::CreatorPro,
            &style(),
            &SubtitleRenderOptions {
                template: "creator_pro".to_string(),
                emoji_layer: true,
                beat_sync: true,
                scene_score: 0.85,
            },
        );
        assert!(animated.contains(r"\t(0,70,\fscx122\fscy122)"));
        assert!(animated.contains(r"\k"));
    }
}
