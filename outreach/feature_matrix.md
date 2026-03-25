# Feature Matrix

| Area | Status | Evidence |
|---|---|---|
| Transcript-aware clip ranking | implemented | `src/main.rs`, `src/ai.rs` |
| LLM reranking + fallback | implemented | `src/ai.rs` |
| Creator subtitle presets and animation | implemented | `src/subtitles.rs` |
| Multi-theme subtitle templates | implemented | `creator_pro`, `creator_neon`, `creator_minimal`, `creator_bold` |
| Emoji/sticker subtitle layer | implemented | `--subtitle-emoji-layer` |
| Beat-sync word timing | implemented | `--subtitle-beat-sync` |
| Scene-aware subtitle transitions | implemented | `--subtitle-scene-fx` |
| API auth scopes + rate/body limits | implemented | `src/main.rs`, `SECURITY.md` |
| Persistent per-client daily quotas | implemented | `--api-quota-store`, `--api-client-daily-quota-runs` |
| Proof/export/thumbnails | implemented | `SHOWCASE.md`, `README.md` |
| Subtitle benchmark harness | implemented | `scripts/subtitle-benchmark.ps1` |
