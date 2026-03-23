# ViralClip Swarm 
A local first, AI powered pipeline that turns long form video (YouTube, Twitch VODs, podcasts, local files) into short, social ready clips automatically. It detects high engagement moments using audio, visual, and text signals, then trims, crops, captions, and exports clips optimized for TikTok, Instagram Reels, YouTube Shorts, and Twitter.
## Why it matters
###### •	Saves time: turns hours of footage into publishable clips in minutes.
###### •	Privacy first: runs locally; no forced uploads to third party services.
###### •	Customizable: open pipeline, adjustable scoring, and export options.
###### •	Extensible: add new detectors, models, or UI integrations.
# Quick Start
### Prerequisites
##### •	ffmpeg and ffprobe in PATH (recommended build with libx264, aac).
##### •	yt-dlp for remote downloads.
##### •	python (optional) for Whisper transcription mode.
##### •	Rust toolchain (if building the Rust CLI): cargo and rustc.
##### •	Recommended models: WhisperX or Whisper (python) for transcription; optional local LLM for sentiment.
## Install from source
``` 
git clone https://github.com/your-org/viralclip-swarm.git
cd viralclip-swarm
cargo build --release
```
## First run example
```
bash
# Process a local file, produce 6 clips, write benchmark CSV
./target/release/viralclip-swarm \
  --input /path/to/long_video.mp4 \
  --num-clips 6 \
  --captions \
  --crop \
  -csv-format csv \
  --csv-path ./output/benchmark.csv \
  --timestamp-mode utc
```
## Quick demo with YouTube URL
```
bash
./target/release/viralclip-swarm \
  --url "https://youtube.com/watch?v=..." \
  --num-clips 10 \
  --captions --whisper-mode python --whisper-model base \
  --subtitles-mode auto \
  --csv-format json --csv-path ./logs/run1.json
```
# CLI Reference
### Core flags
# | Flag | Purpose | Example |
#### | --input |	Local video file |	--input video.mp4 |
#### | --url |	YouTube/Twitch URL |	--url https://... |
#### | --num-clips |	Number of clips to generate |	--num-clips 10 |
#### | --min-duration |	Clip length in seconds |	--min-duration 5.0 |
#### | --captions |	Enable transcription + captions |	--captions |
#### | --whisper-mode |	python or binary |	--whisper-mode python |
#### | --whisper-model |	Whisper model name |	--whisper-model base |
#### | --subtitles-mode |	auto, ass, subtitles |	--subtitles-mode auto |
#### | --crop |	Crop to 9:16 vertical |	--crop |
#### | --accurate |	Re encode for frame accurate cuts |	--accurate |
#### | --csv-path |	Path to benchmark output |	--csv-path ./output/benchmark.csv |
#### | --csv-format |	csv, json, or human |	--csv-format json |
#### | --timestamp-mode |	utc or local timestamps |	--timestamp-mode local |
#### | --append |	Append to existing log file | 	--append |
# Subtitles modes
#### •	auto: try subtitles filter first, fallback to ASS conversion.
#### •	subtitles: use subtitles= filter (may require careful escaping on Windows).
#### •	ass: convert SRT → ASS then burn with ass= (most robust).
Benchmark logging
#### •	CSV: writes a run summary row then clip rows. Use --append to accumulate runs.
#### •	JSON: writes a run object; with --append it stores an array of runs.
#### •	Human: writes a readable table; with --append it appends new run blocks.
Timing fields recorded
#### •	clip_id, start_sec, duration, extract_ms, subtitles_ms, crop_ms, total_ms, success, error, timestamp (ISO), timestamp_human.
# Architecture and Roadmap
### High level pipeline
#### 1.	Ingest: local file or remote download via yt-dlp.
#### 2.	Audio extraction: ffmpeg → 16kHz mono WAV.
#### 3.	Transcription: WhisperX or Whisper (python) → SRT.
#### 4.	Signal analysis:
##### o	Audio: RMS peaks, laughter detection, music changes (librosa/pydub).
##### o	Visual: motion detection, face tracking, emotion heuristics (OpenCV, MediaPipe).
##### o	Text: transcript sentiment/excitement spikes (local LLM or rule based).
#### 5.	Scoring: weighted fusion of signals → top N segments.
#### 6.	Editing: extract clips, burn captions, crop to vertical, optional auto pan.
#### 7.	Export: MP4 clips + metadata + benchmark logs.
# Roadmap (phases)
#### •	Phase 1: Audio only detection (Whisper + energy peaks + FFmpeg).
#### •	Phase 2: Add motion detection (OpenCV) to avoid static shots.
#### •	Phase 3: Caption burning with animated ASS overlays.
#### •	Phase 4: Smart vertical crop with face tracking and auto pan.
#### •	Phase 5: Engagement scoring and user adjustable weights.
#### •	Phase 6: Web UI (React + FastAPI) and background jobs.
#### •	Phase 7: Twitch chat integration and hosted service options.
# Development Guide
### Recommended dev environment
#### •	OS: Linux or WSL for best ffmpeg compatibility; Windows supported but watch subtitle escaping.
#### •	Hardware: CPU with AVX2; GPU optional for faster model inference. For large models, 16–32GB RAM recommended.
#### •	Tools: Rust 1.70+, cargo, Python 3.10+ (if using Whisper python), ffmpeg, ffprobe, yt-dlp.
# Build and test
```
bash
# Build release binary
cargo build --release
# Run unit tests (if present)
cargo test
# Lint and fix
cargo clippy -- -D warnings
cargo fmt
```
# Configuration tips
#### •	Whisper mode: --whisper-mode python gives better word timestamps; requires pip install openai-whisper and models downloaded.
#### •	Subtitles: use --subtitles-mode ass on Windows if subtitles filter fails.
#### •	Performance: use --accurate only when you need frame accurate cuts; otherwise fast copy is much quicker.
#### •	Benchmarking: enable --csv-path and --csv-format to collect per clip timings and iterate on performance.
# Contribution and Governance
## How to contribute
#### •	Fork the repo, create a feature branch, open a PR with tests and changelog entry.
#### •	Add unit tests for new detectors and parsing utilities (e.g., parse_srt_timestamp, extract_srt_segment).
#### •	Follow the code style: cargo fmt and cargo clippy.
Issue triage
#### •	Label issues as bug, feature, performance, docs.
#### •	Provide reproducible steps and sample media when possible.
Roadmap governance
#### •	Core maintainers approve major architecture changes.
#### •	Community proposals accepted via RFCs in docs/rfcs.
# Troubleshooting and FAQ
## Q: ffmpeg fails to burn subtitles with original_size error on Windows
#### •	Use --subtitles-mode ass to convert SRT → ASS first. The ASS path avoids the original_size parsing issue.
## Q: Whisper produced no SRT
#### •	If using --whisper-mode python, ensure Python and the whisper package are installed and the model is downloaded. Check audio.wav.with_extension("srt") location.
## Q: Clips are boring or duplicate
#### •	Adjust --num-clips, --min-duration, and scoring weights (future config). Use --motion to boost scene changes.
## Q: I want local LLM sentiment but no GPU
#### •	Use a small LLM via llama.cpp or ONNX with CPU quantized models; provide a --lite mode in config.
# Common fixes
#### •	Ensure ffmpeg and ffprobe are in PATH.
#### •	On Windows, escape paths or use ASS mode for subtitles.
#### •	If yt-dlp fails, update it: pip install -U yt-dlp or use the binary.
Security Privacy and Licensing
Privacy
#### •	Default behavior is local processing. No data is uploaded unless you opt into a hosted service. Document this clearly in the README and UI.
License
####•	Use a permissive license for the core (MIT/Apache 2.0) and clearly mark any third party model licenses (Whisper, Llama, etc.).
# Model usage
####•	Document model sources and any license restrictions. Provide a models/README.md with download instructions and checksums.
# Growth and Monetization
## Open core strategy
#### •	Keep CLI and core detectors open source.
#### •	Offer a hosted service for convenience, priority support, and heavy processing.
# Sponsorship and partnerships
#### •	Add GitHub Sponsors, Patreon, or a donate button.
#### •	Offer consulting for MCNs and agencies.
# Value add
#### •	Provide curated music packs, templates, and premium auto caption styles as paid add ons.
# Appendix Examples and Tips
## Example: append JSON with local timestamps
```
bash
./target/release/viralclip-swarm \
  --input /path/to/video.mp4 \
  --captions --whisper-mode python --whisper-model small \
  --csv-format json --csv-path 
 ./logs/all_runs.json --append --timestamp-mode local
```
Tip: fast debugging
```
•	Run with --num-clips 2 --min-duration 3 --captions false 
to iterate quickly.
```
Tip: tune scoring
• Start with audio energy + scene changes. Add visual face motion only if you see false positives.


## Get involved
#### •	Open issues for bugs and feature requests.
#### •	Share example clips and workflows in the Discussions tab.
#### •	Submit PRs for new detectors, UI components, or model integrations.
# Maintainers
#### •	Add maintainer contact and code of conduct link here.

## License

[MIT](https://choosealicense.com/licenses/mit/)
