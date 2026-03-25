use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
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
    pub semantic_score: f32,
    pub speech_confidence: f32,
    pub face_score: f32,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RerankResult {
    pub clip_id: usize,
    pub score: f32,
    pub reason: String,
}

pub fn build_storyboard(options: &AiOptions, clips: &[AiClipContext]) -> Result<Option<AiStoryboard>> {
    if !options.enabled || clips.is_empty() {
        return Ok(None);
    }

    let storyboard = match options.provider.as_str() {
        "heuristic" | "local" => heuristic_storyboard(options, clips),
        "openai" | "openrouter" | "groq" | "huggingface" | "anthropic" | "gemini" => match cloud_storyboard(options, clips) {
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

pub fn rerank_candidates(
    options: &AiOptions,
    transcript_segments: &[String],
    clips: &[AiClipContext],
) -> Result<Vec<RerankResult>> {
    if clips.is_empty() {
        return Ok(Vec::new());
    }

    match options.provider.as_str() {
        "openai" | "openrouter" | "groq" | "huggingface" | "anthropic" | "gemini" if options.enabled => match cloud_rerank(options, transcript_segments, clips) {
            Ok(scores) => Ok(scores),
            Err(error) => {
                eprintln!(
                    "AI reranker {} failed: {}. Falling back to local embedding-style rerank.",
                    options.provider, error
                );
                Ok(local_embedding_rerank(transcript_segments, clips))
            }
        },
        _ => Ok(local_embedding_rerank(transcript_segments, clips)),
    }
}

fn is_openai_compatible_provider(provider: &str) -> bool {
    matches!(provider, "openai" | "openrouter" | "groq" | "huggingface")
}

fn provider_chat_url(options: &AiOptions, api_key: &str) -> Result<String> {
    match options.provider.as_str() {
        "openai" => Ok("https://api.openai.com/v1/chat/completions".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api/v1/chat/completions".to_string()),
        "groq" => Ok("https://api.groq.com/openai/v1/chat/completions".to_string()),
        "huggingface" => Ok("https://router.huggingface.co/v1/chat/completions".to_string()),
        "anthropic" => Ok("https://api.anthropic.com/v1/messages".to_string()),
        "gemini" => Ok(format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={api_key}",
            options.model
        )),
        other => anyhow::bail!("unsupported provider {}", other),
    }
}

fn provider_headers<'a>(options: &AiOptions, api_key: &'a str) -> Vec<(&'static str, String)> {
    match options.provider.as_str() {
        "openai" | "groq" | "huggingface" => vec![("Authorization", format!("Bearer {api_key}"))],
        "openrouter" => vec![
            ("Authorization", format!("Bearer {api_key}")),
            ("HTTP-Referer", "https://github.com/Gangsworld3/viralclip-swarm".to_string()),
            ("X-OpenRouter-Title", "ViralClip Swarm".to_string()),
        ],
        "anthropic" => vec![
            ("x-api-key", api_key.to_string()),
            ("anthropic-version", "2023-06-01".to_string()),
        ],
        "gemini" => Vec::new(),
        _ => Vec::new(),
    }
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

fn weighted_tokens(text: &str) -> HashMap<String, f32> {
    let mut map = HashMap::new();
    for token in words(text) {
        *map.entry(token).or_insert(0.0) += 1.0;
    }
    map
}

fn cosine_similarity(lhs: &HashMap<String, f32>, rhs: &HashMap<String, f32>) -> f32 {
    if lhs.is_empty() || rhs.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut lhs_norm = 0.0f32;
    let mut rhs_norm = 0.0f32;

    for value in lhs.values() {
        lhs_norm += value * value;
    }
    for value in rhs.values() {
        rhs_norm += value * value;
    }

    for (token, lhs_value) in lhs {
        if let Some(rhs_value) = rhs.get(token) {
            dot += lhs_value * rhs_value;
        }
    }

    if lhs_norm <= 0.0 || rhs_norm <= 0.0 {
        0.0
    } else {
        dot / (lhs_norm.sqrt() * rhs_norm.sqrt())
    }
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

fn keyword_priority(word: &str) -> usize {
    match word {
        "secret" | "truth" | "mistake" | "crazy" | "never" | "best" | "worst" | "exposed"
        | "hack" | "warning" | "proof" | "viral" | "million" | "money" | "insane" => 3,
        "why" | "how" | "what" | "when" | "wait" | "because" | "before" | "after"
        | "finally" | "actually" | "everyone" => 2,
        _ => usize::from(word.chars().any(|ch| ch.is_ascii_digit())),
    }
}

fn stopword(word: &str) -> bool {
    matches!(
        word,
        "a"
            | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "but"
            | "by"
            | "for"
            | "from"
            | "get"
            | "got"
            | "had"
            | "has"
            | "have"
            | "he"
            | "her"
            | "him"
            | "his"
            | "i"
            | "if"
            | "in"
            | "into"
            | "is"
            | "it"
            | "its"
            | "just"
            | "me"
            | "my"
            | "of"
            | "on"
            | "or"
            | "our"
            | "out"
            | "she"
            | "so"
            | "that"
            | "the"
            | "their"
            | "them"
            | "there"
            | "they"
            | "this"
            | "to"
            | "up"
            | "was"
            | "we"
            | "were"
            | "what"
            | "when"
            | "why"
            | "with"
            | "you"
            | "your"
    )
}

fn semantic_phrase_weight(text: &str) -> f32 {
    let lower = text.to_ascii_lowercase();
    let phrases = [
        "this is why",
        "the secret",
        "what happened",
        "here's why",
        "watch this",
        "i can't believe",
        "the mistake",
        "turns out",
        "you need to",
    ];
    phrases.iter().filter(|phrase| lower.contains(**phrase)).count() as f32
}

fn build_segment_centroid(transcript_segments: &[String]) -> HashMap<String, f32> {
    let segments = transcript_segments
        .iter()
        .map(|segment| normalize_copy(segment))
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return HashMap::new();
    }

    let mut doc_freq: HashMap<String, usize> = HashMap::new();
    let mut segment_maps = Vec::new();
    for segment in &segments {
        let token_map = weighted_tokens(segment);
        let token_set: HashSet<String> = token_map.keys().cloned().collect();
        for token in token_set {
            *doc_freq.entry(token).or_insert(0) += 1;
        }
        segment_maps.push(token_map);
    }

    let total_docs = segments.len() as f32;
    let mut centroid = HashMap::new();
    for (segment, token_map) in segments.iter().zip(segment_maps.iter()) {
        let segment_weight = 1.0 + semantic_phrase_weight(segment) * 0.35;
        for (token, tf) in token_map {
            let df = *doc_freq.get(token).unwrap_or(&1) as f32;
            let idf = ((1.0 + total_docs) / (1.0 + df)).ln() + 1.0;
            *centroid.entry(token.clone()).or_insert(0.0) += tf * idf * segment_weight;
        }
    }

    centroid
}

fn local_embedding_rerank(transcript_segments: &[String], clips: &[AiClipContext]) -> Vec<RerankResult> {
    let centroid = build_segment_centroid(transcript_segments);
    let mut results = clips
        .iter()
        .map(|clip| {
            let clip_vec = weighted_tokens(&clip.transcript_excerpt);
            let similarity = cosine_similarity(&centroid, &clip_vec);
            let score = ((similarity * 0.42)
                + (clip.semantic_score * 0.28)
                + (clip.speech_confidence * 0.15)
                + (clip.face_score * 0.08)
                + (clip.readability_score * 0.07))
                .clamp(0.0, 1.0);
            let reason = if similarity > 0.55 {
                "strong transcript centroid match"
            } else if clip.semantic_score > 0.7 {
                "high semantic payoff"
            } else if clip.speech_confidence > 0.7 {
                "clear spoken segment"
            } else {
                "baseline rerank support"
            };
            RerankResult {
                clip_id: clip.clip_id,
                score,
                reason: reason.to_string(),
            }
        })
        .collect::<Vec<_>>();

    results.sort_by(|lhs, rhs| rhs.score.partial_cmp(&lhs.score).unwrap_or(std::cmp::Ordering::Equal));
    results
}

fn pick_thumbnail_text(clip: &AiClipContext, fallback: &str) -> String {
    let mut ranked = words(&clip.transcript_excerpt)
        .into_iter()
        .filter(|word| !stopword(word) && word.len() > 2)
        .map(|word| {
            let priority = keyword_priority(&word);
            let len_score = word.len().min(12);
            (word, priority, len_score)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|lhs, rhs| rhs.1.cmp(&lhs.1).then_with(|| rhs.2.cmp(&lhs.2)));

    let mut selected = Vec::new();
    for (word, _, _) in ranked {
        if selected.iter().any(|existing: &String| existing == &word) {
            continue;
        }
        selected.push(word.to_ascii_uppercase());
        if selected.len() >= 3 {
            break;
        }
    }

    if selected.is_empty() {
        fallback.to_ascii_uppercase()
    } else {
        selected.join(" ")
    }
}

fn platform_caption(base: &str, tags: &str, fallback: &str) -> String {
    let chosen = if base.trim().is_empty() { fallback } else { base };
    format!("{} {}", limit_words(chosen, 14), tags)
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
            let intensity = if clip.score >= 1.55 || clip.semantic_score >= 0.78 {
                "Unhinged"
            } else if clip.score >= 0.95 || clip.face_score >= 0.55 {
                "High Voltage"
            } else {
                "Clean Hit"
            };
            let angle = if clip.face_score > 0.65 && clip.motion > 0.45 {
                "face-first scene turn"
            } else if clip.motion > 0.55 {
                "scene flip"
            } else if clip.laughter > 0.55 {
                "laugh break"
            } else if clip.chat > 0.55 {
                "chat spike"
            } else if clip.semantic_score > 0.65 {
                "strong payoff line"
            } else if clip.readability_score > 0.7 {
                "clean quote"
            } else {
                "audio peak"
            };
            let excerpt = clean_excerpt(&clip.transcript_excerpt);
            let title = punchy_title(intensity, &excerpt, clip.clip_id);
            let hook = if excerpt.is_empty() {
                format!("Start at {:.1}s and lean into the {angle}.", clip.start_sec)
            } else if clip.semantic_score > 0.72 {
                format!("{}.", sentence_case(&limit_words(&excerpt, 7)))
            } else {
                format!("{}.", sentence_case(&limit_words(&excerpt, 6)))
            };
            let caption_core = if excerpt.is_empty() {
                format!("Clip {} lands at {:.1}s.", clip.clip_id, clip.start_sec)
            } else {
                excerpt.clone()
            };
            let thumbnail_text = pick_thumbnail_text(clip, intensity);
            let call_to_action = if clip.speech_confidence < 0.35 {
                "Watch the full turn before you judge it.".to_string()
            } else if clip.semantic_score > 0.7 {
                "Watch the payoff and drop your take.".to_string()
            } else {
                "Watch to the end and drop your take.".to_string()
            };
            AiClipPlan {
                clip_id: clip.clip_id,
                title,
                hook,
                social_caption: format!("{} {}", sentence_case(&caption_core), call_to_action),
                subtitle_preset: options.subtitle_preset.clone(),
                thumbnail_text,
                call_to_action: call_to_action.clone(),
                youtube_shorts_caption: platform_caption(&excerpt, "#shorts #creator", "Big moment."),
                tiktok_caption: platform_caption(&excerpt, "#fyp #viral", "Wait for it."),
                instagram_reels_caption: platform_caption(
                    &excerpt,
                    "#reels #contentcreator",
                    "Saved this moment.",
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

fn cloud_rerank(options: &AiOptions, transcript_segments: &[String], clips: &[AiClipContext]) -> Result<Vec<RerankResult>> {
    let api_key = std::env::var(&options.api_key_env)
        .with_context(|| format!("missing API key env var {}", options.api_key_env))?;
    let transcript_segments = transcript_segments.iter().take(12).cloned().collect::<Vec<_>>();
    let prompt = "Return strict JSON with shape {\"scores\":[{\"clip_id\":1,\"score\":0.0,\"reason\":\"...\"}]}. Score each candidate between 0 and 1 for semantic short-form potential using transcript payoff, clarity, face presence, and creator hook strength. Prefer clips with quotable statements, contrast, reveal, or direct viewer relevance.";

    let payload = match options.provider.as_str() {
        provider if is_openai_compatible_provider(provider) => json!({
            "model": options.model,
            "messages": [
                {"role": "system", "content": "You rerank short-form clip candidates and return strict JSON only."},
                {"role": "user", "content": format!(
                    "{}\n\nTranscript segments:\n{}\n\nCandidate clips:\n{}",
                    prompt,
                    serde_json::to_string_pretty(&transcript_segments)?,
                    serde_json::to_string_pretty(clips)?
                )}
            ],
            "response_format": {"type": "json_object"}
        }),
        "anthropic" => json!({
            "model": options.model,
            "max_tokens": 1400,
            "messages": [
                {"role": "user", "content": format!(
                    "{}\n\nTranscript segments:\n{}\n\nCandidate clips:\n{}",
                    prompt,
                    serde_json::to_string_pretty(&transcript_segments)?,
                    serde_json::to_string_pretty(clips)?
                )}
            ]
        }),
        "gemini" => json!({
            "contents": [{
                "parts": [{
                    "text": format!(
                        "{}\n\nTranscript segments:\n{}\n\nCandidate clips:\n{}",
                        prompt,
                        serde_json::to_string_pretty(&transcript_segments)?,
                        serde_json::to_string_pretty(clips)?
                    )
                }]
            }],
            "generationConfig": {"responseMimeType": "application/json"}
        }),
        other => anyhow::bail!("unsupported rerank provider {}", other),
    };

    let response = match options.provider.as_str() {
        provider if is_openai_compatible_provider(provider) || provider == "anthropic" => {
            let url = provider_chat_url(options, &api_key)?;
            let headers = provider_headers(options, &api_key);
            let header_refs = headers.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>();
            run_curl_json(&url, &header_refs, &payload)?
        }
        "gemini" => run_curl_json(&provider_chat_url(options, &api_key)?, &[], &payload)?,
        _ => unreachable!(),
    };

    parse_cloud_rerank(options, response)
}

fn cloud_storyboard(options: &AiOptions, clips: &[AiClipContext]) -> Result<AiStoryboard> {
    let api_key = std::env::var(&options.api_key_env)
        .with_context(|| format!("missing API key env var {}", options.api_key_env))?;
    let prompt = format!(
        "Return strict JSON with shape {{\"clips\":[{{\"clip_id\":1,\"title\":\"...\",\"hook\":\"...\",\"social_caption\":\"...\",\"subtitle_preset\":\"{}\",\"thumbnail_text\":\"...\",\"call_to_action\":\"...\",\"youtube_shorts_caption\":\"...\",\"tiktok_caption\":\"...\",\"instagram_reels_caption\":\"...\"}}]}}. Keep each title under 6 words, each thumbnail_text under 4 words, use transcript context plus semantic_score, speech_confidence, and face_score, and make the tone bold, creator-style.",
        options.subtitle_preset
    );
    let payload = match options.provider.as_str() {
        provider if is_openai_compatible_provider(provider) => json!({
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
        provider if is_openai_compatible_provider(provider) || provider == "anthropic" => {
            let url = provider_chat_url(options, &api_key)?;
            let headers = provider_headers(options, &api_key);
            let header_refs = headers.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>();
            run_curl_json(&url, &header_refs, &payload)?
        }
        "gemini" => run_curl_json(&provider_chat_url(options, &api_key)?, &[], &payload)?,
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
    let raw = parse_cloud_content(options, response)?;

    let parsed: serde_json::Value = serde_json::from_str(&raw).context("parse AI content json")?;
    let clips: Vec<AiClipPlan> = serde_json::from_value(parsed["clips"].clone()).context("parse AI clip plans")?;
    Ok(AiStoryboard {
        provider: options.provider.clone(),
        model: options.model.clone(),
        clips,
    })
}

fn parse_cloud_content(options: &AiOptions, response: serde_json::Value) -> Result<String> {
    match options.provider.as_str() {
        provider if is_openai_compatible_provider(provider) => response["choices"][0]["message"]["content"]
            .as_str()
            .context("missing OpenAI-compatible message content")
            .map(|value| value.to_string()),
        "anthropic" => response["content"][0]["text"]
            .as_str()
            .context("missing Anthropic content text")
            .map(|value| value.to_string()),
        "gemini" => response["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .context("missing Gemini content text")
            .map(|value| value.to_string()),
        _ => anyhow::bail!("unsupported AI provider {}", options.provider),
    }
}

fn parse_cloud_rerank(options: &AiOptions, response: serde_json::Value) -> Result<Vec<RerankResult>> {
    let raw = parse_cloud_content(options, response)?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).context("parse rerank json")?;
    let mut scores: Vec<RerankResult> =
        serde_json::from_value(parsed["scores"].clone()).context("parse rerank scores")?;
    for score in &mut scores {
        score.score = score.score.clamp(0.0, 1.0);
    }
    Ok(scores)
}

#[cfg(test)]
mod tests {
    use super::{build_storyboard, pick_thumbnail_text, rerank_candidates, AiClipContext, AiOptions};

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
            semantic_score: 0.85,
            speech_confidence: 0.95,
            face_score: 0.6,
        }];

        let storyboard = build_storyboard(&options, &clips).unwrap().unwrap();
        let clip = &storyboard.clips[0];
        assert!(!clip.thumbnail_text.is_empty());
        assert!(clip.youtube_shorts_caption.contains("#shorts"));
        assert!(clip.tiktok_caption.contains("#fyp"));
        assert!(clip.instagram_reels_caption.contains("#reels"));
        assert!(clip.title.split_whitespace().count() <= 4);
        assert!(clip.social_caption.split_whitespace().count() < 20);
        assert!(clip.thumbnail_text.split_whitespace().count() <= 3);
    }

    #[test]
    fn thumbnail_text_prefers_strong_keywords() {
        let clip = AiClipContext {
            clip_id: 1,
            start_sec: 0.0,
            end_sec: 8.0,
            score: 1.0,
            energy: 0.8,
            laughter: 0.0,
            motion: 0.0,
            chat: 0.0,
            transcript_excerpt: "this is the secret money mistake nobody notices".to_string(),
            readability_score: 0.8,
            semantic_score: 0.7,
            speech_confidence: 0.9,
            face_score: 0.2,
        };

        let text = pick_thumbnail_text(&clip, "fallback");
        assert!(text.contains("SECRET") || text.contains("MISTAKE") || text.contains("MONEY"));
    }

    #[test]
    fn local_reranker_prefers_semantic_candidate() {
        let options = AiOptions {
            enabled: false,
            provider: "heuristic".to_string(),
            model: "heuristic".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            subtitle_preset: "legendary".to_string(),
        };
        let transcript_segments = vec![
            "here's the secret to going viral without wasting time".to_string(),
            "most people make this mistake before they post".to_string(),
            "you only need one clear hook and one payoff".to_string(),
        ];
        let clips = vec![
            AiClipContext {
                clip_id: 1,
                start_sec: 0.0,
                end_sec: 8.0,
                score: 0.7,
                energy: 0.7,
                laughter: 0.0,
                motion: 0.0,
                chat: 0.0,
                transcript_excerpt: "random loud noise and crowd reaction".to_string(),
                readability_score: 0.5,
                semantic_score: 0.2,
                speech_confidence: 0.3,
                face_score: 0.2,
            },
            AiClipContext {
                clip_id: 2,
                start_sec: 10.0,
                end_sec: 18.0,
                score: 0.6,
                energy: 0.5,
                laughter: 0.0,
                motion: 0.0,
                chat: 0.0,
                transcript_excerpt: "the secret is one clear hook and one payoff".to_string(),
                readability_score: 0.9,
                semantic_score: 0.9,
                speech_confidence: 0.9,
                face_score: 0.4,
            },
        ];

        let results = rerank_candidates(&options, &transcript_segments, &clips).unwrap();
        assert_eq!(results[0].clip_id, 2);
        assert!(results[0].score > results[1].score);
    }
}
