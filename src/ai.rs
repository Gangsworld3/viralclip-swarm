use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct AiOptions {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub api_key_env: String,
    pub subtitle_preset: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiClipContext {
    pub clip_id: usize,
    pub start_sec: f32,
    pub end_sec: f32,
    pub score: f32,
    pub energy: f32,
    pub laughter: f32,
    pub motion: f32,
    pub chat: f32,
    pub transcript_excerpt: String,
    pub readability_score: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiClipPlan {
    pub clip_id: usize,
    pub title: String,
    pub hook: String,
    pub social_caption: String,
    pub subtitle_preset: String,
    pub thumbnail_text: String,
    pub call_to_action: String,
    pub youtube_shorts_caption: String,
    pub tiktok_caption: String,
    pub instagram_reels_caption: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiStoryboard {
    pub provider: String,
    pub model: String,
    pub clips: Vec<AiClipPlan>,
}

pub fn build_storyboard(options: &AiOptions, clips: &[AiClipContext]) -> Result<Option<AiStoryboard>> {
    if !options.enabled || clips.is_empty() {
        return Ok(None);
    }

    let storyboard = match options.provider.as_str() {
        "heuristic" | "local" => heuristic_storyboard(options, clips),
        "openai" | "anthropic" | "gemini" => match cloud_storyboard(options, clips) {
            Ok(storyboard) => storyboard,
            Err(error) => {
                eprintln!(
                    "AI provider {} failed: {}. Falling back to heuristic plan.",
                    options.provider, error
                );
                heuristic_storyboard(options, clips)
            }
        },
        other => {
            eprintln!("Unknown AI provider '{}', using heuristic plan instead.", other);
            heuristic_storyboard(options, clips)
        }
    };

    Ok(Some(storyboard))
}

pub fn write_storyboard(path: &Path, storyboard: &AiStoryboard) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create AI storyboard directory {}", parent.display()))?;
        }
    }
    let raw = serde_json::to_string_pretty(storyboard).context("serialize ai storyboard")?;
    std::fs::write(path, raw).with_context(|| format!("write ai storyboard {}", path.display()))?;
    Ok(())
}

fn normalize_copy(text: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for ch in text.chars() {
        let keep = ch.is_ascii_alphanumeric() || ch.is_whitespace() || matches!(ch, '\'' | '?' | '!' | ',' | '.');
        if !keep {
            continue;
        }
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

fn words(text: &str) -> Vec<String> {
    normalize_copy(text)
        .split_whitespace()
        .map(|word| word.to_string())
        .collect()
}

fn limit_words(text: &str, max_words: usize) -> String {
    words(text).into_iter().take(max_words).collect::<Vec<_>>().join(" ")
}

fn sentence_case(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut chars = trimmed.chars();
    let first = chars.next().unwrap().to_uppercase().to_string();
    let rest = chars.as_str();
    format!("{}{}", first, rest)
}

fn clean_excerpt(text: &str) -> String {
    let cleaned = normalize_copy(text);
    if cleaned.is_empty() {
        return String::new();
    }

    let limited = limit_words(&cleaned, 16);
    sentence_case(&limited)
}

fn punchy_title(intensity: &str, excerpt: &str, clip_id: usize) -> String {
    let excerpt_title = limit_words(excerpt, 4);
    if excerpt_title.is_empty() {
        format!("{intensity} moment #{clip_id:02}")
    } else {
        sentence_case(&excerpt_title)
    }
}

fn heuristic_storyboard(options: &AiOptions, clips: &[AiClipContext]) -> AiStoryboard {
    let clips = clips
        .iter()
        .map(|clip| {
            let intensity = if clip.score >= 1.4 {
                "Unhinged"
            } else if clip.score >= 0.9 {
                "High Voltage"
            } else {
                "Clean Hit"
            };
            let angle = if clip.motion > 0.55 {
                "scene flip"
            } else if clip.laughter > 0.55 {
                "laugh break"
            } else if clip.chat > 0.55 {
                "chat spike"
            } else if clip.readability_score > 0.7 {
                "clean quote"
            } else {
                "audio peak"
            };
            let excerpt = clean_excerpt(&clip.transcript_excerpt);
            let title = punchy_title(intensity, &excerpt, clip.clip_id);
            let hook = if excerpt.is_empty() {
                format!("Start at {:.1}s and lean into the {angle}.", clip.start_sec)
            } else {
                format!("{}.", sentence_case(&limit_words(&excerpt, 8)))
            };
            let caption_core = if excerpt.is_empty() {
                format!("Clip {} lands at {:.1}s.", clip.clip_id, clip.start_sec)
            } else {
                excerpt.clone()
            };
            let thumbnail_text = limit_words(&excerpt, 4).to_uppercase();
            AiClipPlan {
                clip_id: clip.clip_id,
                title,
                hook,
                social_caption: format!(
                    "{} {}",
                    sentence_case(&caption_core),
                    "Watch to the end and drop your take."
                ),
                subtitle_preset: options.subtitle_preset.clone(),
                thumbnail_text: if thumbnail_text.is_empty() {
                    intensity.to_ascii_uppercase()
                } else {
                    thumbnail_text
                },
                call_to_action: "Watch to the end and drop your take.".to_string(),
                youtube_shorts_caption: format!(
                    "{} #shorts #creator",
                    if excerpt.is_empty() { "Big moment." } else { &excerpt }
                ),
                tiktok_caption: format!(
                    "{} #fyp #viral",
                    if excerpt.is_empty() { "Wait for it." } else { &excerpt }
                ),
                instagram_reels_caption: format!(
                    "{} #reels #contentcreator",
                    if excerpt.is_empty() { "Saved this moment." } else { &excerpt }
                ),
            }
        })
        .collect();

    AiStoryboard {
        provider: options.provider.clone(),
        model: options.model.clone(),
        clips,
    }
}

fn cloud_storyboard(options: &AiOptions, clips: &[AiClipContext]) -> Result<AiStoryboard> {
    let api_key = std::env::var(&options.api_key_env)
        .with_context(|| format!("missing API key env var {}", options.api_key_env))?;
    let prompt = format!(
        "Return strict JSON with shape {{\"clips\":[{{\"clip_id\":1,\"title\":\"...\",\"hook\":\"...\",\"social_caption\":\"...\",\"subtitle_preset\":\"{}\",\"thumbnail_text\":\"...\",\"call_to_action\":\"...\",\"youtube_shorts_caption\":\"...\",\"tiktok_caption\":\"...\",\"instagram_reels_caption\":\"...\"}}]}}. Keep each title under 6 words, use transcript context, and make the tone bold, creator-style.",
        options.subtitle_preset
    );
    let payload = match options.provider.as_str() {
        "openai" => json!({
            "model": options.model,
            "messages": [
                {"role": "system", "content": "You create viral clip metadata and return strict JSON only."},
                {"role": "user", "content": format!("{}\n\nClip data:\n{}", prompt, serde_json::to_string_pretty(clips)?)}
            ],
            "response_format": {"type": "json_object"}
        }),
        "anthropic" => json!({
            "model": options.model,
            "max_tokens": 1200,
            "messages": [
                {"role": "user", "content": format!("{}\n\nClip data:\n{}", prompt, serde_json::to_string_pretty(clips)?)}
            ]
        }),
        "gemini" => json!({
            "contents": [{
                "parts": [{
                    "text": format!("{}\n\nClip data:\n{}", prompt, serde_json::to_string_pretty(clips)?)
                }]
            }],
            "generationConfig": {"responseMimeType": "application/json"}
        }),
        other => anyhow::bail!("unsupported AI provider {}", other),
    };

    let response = match options.provider.as_str() {
        "openai" => run_curl_json(
            "https://api.openai.com/v1/chat/completions",
            &[("Authorization", &format!("Bearer {}", api_key))],
            &payload,
        )?,
        "anthropic" => run_curl_json(
            "https://api.anthropic.com/v1/messages",
            &[
                ("x-api-key", &api_key),
                ("anthropic-version", "2023-06-01"),
            ],
            &payload,
        )?,
        "gemini" => run_curl_json(
            &format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                options.model, api_key
            ),
            &[],
            &payload,
        )?,
        _ => unreachable!(),
    };

    parse_cloud_storyboard(options, response)
}

fn run_curl_json(url: &str, headers: &[(&str, &str)], payload: &serde_json::Value) -> Result<serde_json::Value> {
    let curl = which::which("curl").context("curl not found in PATH")?;
    let mut cmd = Command::new(curl);
    cmd.arg("-sS").arg(url).arg("-H").arg("Content-Type: application/json");
    for (name, value) in headers {
        cmd.arg("-H").arg(format!("{name}: {value}"));
    }
    cmd.arg("-d").arg(payload.to_string());

    let output = cmd.output().context("running curl for AI provider")?;
    if !output.status.success() {
        anyhow::bail!("curl AI request failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }

    serde_json::from_slice(&output.stdout).context("parse AI provider json response")
}

fn parse_cloud_storyboard(options: &AiOptions, response: serde_json::Value) -> Result<AiStoryboard> {
    let raw = match options.provider.as_str() {
        "openai" => response["choices"][0]["message"]["content"]
            .as_str()
            .context("missing OpenAI message content")?
            .to_string(),
        "anthropic" => response["content"][0]["text"]
            .as_str()
            .context("missing Anthropic content text")?
            .to_string(),
        "gemini" => response["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .context("missing Gemini content text")?
            .to_string(),
        _ => anyhow::bail!("unsupported AI provider {}", options.provider),
    };

    let parsed: serde_json::Value = serde_json::from_str(&raw).context("parse AI content json")?;
    let clips: Vec<AiClipPlan> = serde_json::from_value(parsed["clips"].clone()).context("parse AI clip plans")?;
    Ok(AiStoryboard {
        provider: options.provider.clone(),
        model: options.model.clone(),
        clips,
    })
}

#[cfg(test)]
mod tests {
    use super::{build_storyboard, AiClipContext, AiOptions};

    #[test]
    fn heuristic_storyboard_includes_platform_fields() {
        let options = AiOptions {
            enabled: true,
            provider: "heuristic".to_string(),
            model: "heuristic".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            subtitle_preset: "legendary".to_string(),
        };
        let clips = vec![AiClipContext {
            clip_id: 1,
            start_sec: 10.0,
            end_sec: 18.0,
            score: 1.2,
            energy: 0.8,
            laughter: 0.1,
            motion: 0.2,
            chat: 0.0,
            transcript_excerpt: "this is the secret to getting better clips fast".to_string(),
            readability_score: 0.9,
        }];

        let storyboard = build_storyboard(&options, &clips).unwrap().unwrap();
        let clip = &storyboard.clips[0];
        assert!(!clip.thumbnail_text.is_empty());
        assert!(clip.youtube_shorts_caption.contains("#shorts"));
        assert!(clip.tiktok_caption.contains("#fyp"));
        assert!(clip.instagram_reels_caption.contains("#reels"));
        assert!(clip.title.split_whitespace().count() <= 4);
        assert!(clip.social_caption.split_whitespace().count() < 20);
    }
}
