use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn run_command_capture(cmd: &mut Command, context_msg: &str) -> Result<()> {
    let output = cmd.output().with_context(|| context_msg.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}: {}", context_msg, stderr.trim());
    }
    Ok(())
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
    run_command_capture(&mut cmd, "converting srt to ass with ffmpeg")?;
    Ok(ass_path)
}

fn ass_bool(value: bool) -> i32 {
    if value { -1 } else { 0 }
}

fn ass_escape_text(value: &str) -> String {
    value.replace('\\', r"\\").replace('{', r"\{").replace('}', r"\}")
}

fn animate_ass_text(text: &str, preset: SubtitleAnimationPreset, style: &SubtitleStyle) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 1 {
        return ass_escape_text(text);
    }

    let total_cs = 90usize;
    let per_word = (total_cs / words.len().max(1)).max(8);
    let mut out = String::new();
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let escaped = ass_escape_text(word);
        match preset {
            SubtitleAnimationPreset::Karaoke => {
                out.push_str(&format!(r"{{\1c{}\k{}}}{}", style.highlight_color, per_word, escaped));
            }
            SubtitleAnimationPreset::Emphasis => {
                out.push_str(&format!(
                    r"{{\1c{}\k{}\fscx115\fscy115\t(0,120,\fscx100\fscy100)}}{}",
                    style.highlight_color, per_word, escaped
                ));
            }
            SubtitleAnimationPreset::Impact => {
                out.push_str(&format!(
                    r"{{\1c{}\k{}\bord5\fscx130\fscy130\t(0,140,\fscx100\fscy100)}}{}",
                    style.highlight_color, per_word, escaped
                ));
            }
            SubtitleAnimationPreset::Pulse => {
                out.push_str(&format!(
                    r"{{\1c{}\k{}\t(0,90,\alpha&H20&)\t(90,180,\alpha&H00&)}}{}",
                    style.highlight_color, per_word, escaped
                ));
            }
            SubtitleAnimationPreset::None => out.push_str(&escaped),
        }
    }
    out
}

fn apply_animation_to_ass(ass_path: &Path, preset: SubtitleAnimationPreset, style: &SubtitleStyle) -> Result<()> {
    if preset == SubtitleAnimationPreset::None {
        return Ok(());
    }
    let raw = std::fs::read_to_string(ass_path).with_context(|| format!("read ass file {}", ass_path.display()))?;
    let mut updated = Vec::new();
    for line in raw.lines() {
        if let Some(prefix_end) = line.rfind(',') {
            if line.starts_with("Dialogue:") {
                let prefix = &line[..=prefix_end];
                let text = line[prefix_end + 1..].trim();
                let animated = animate_ass_text(text, preset, style);
                updated.push(format!("{prefix}{animated}"));
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
    let raw = std::fs::read_to_string(ass_path).with_context(|| format!("read ass file {}", ass_path.display()))?;
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
) -> Result<()> {
    let ffmpeg = which::which("ffmpeg").context("ffmpeg not found in PATH")?;
    let ass_path = convert_srt_to_ass(srt_path).context("SRT->ASS conversion failed")?;
    let fallback_style = SubtitleStyle {
        font: "Arial".to_string(),
        size: 28,
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
    let style_ref = style.unwrap_or(&fallback_style);
    apply_style_to_ass(&ass_path, style_ref).context("apply subtitle style to ass")?;
    apply_animation_to_ass(&ass_path, animation, style_ref).context("apply subtitle animation to ass")?;

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
    let filter = format!("subtitles='{}':force_style='{}'", srt_escaped, force_style_escaped);

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
    use super::{animate_ass_text, SubtitleAnimationPreset, SubtitleStyle};

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
        let animated = animate_ass_text("hello world now", SubtitleAnimationPreset::Karaoke, &style());
        assert!(animated.contains(r"{\1c&H0000F6FF\k"));
    }
}
