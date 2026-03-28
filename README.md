# ViralClip Swarm

![ViralClip Swarm Logo](assets/logo.svg)

Rust CLI for turning long videos into short clips with optional captions, vertical crop, AI metadata, thumbnails, and a local API.

## What It Does

- accepts a local video file or YouTube URL
- scores candidate moments
- cuts clips
- optionally adds subtitles
- optionally crops to 9:16
- writes benchmark and export metadata

## Requirements

- Rust
- `ffmpeg` and `ffprobe`
- `yt-dlp` for `--url`
- one transcription option:
  - Python + `openai-whisper`, or
  - `whisper` binary in `PATH`
- `curl` for cloud AI or cloud transcription features

## Install

```powershell
git clone https://github.com/Gangsworld3/viralclip-swarm.git
cd viralclip-swarm
cargo build --release
```

## Quick Start

Run on a local file:

```powershell
cargo run -- --input "C:\path\to\video.mp4"
```

Run with captions:

```powershell
cargo run -- --input "C:\path\to\video.mp4" --captions
```

Run with captions and vertical crop:

```powershell
cargo run -- --input "C:\path\to\video.mp4" --captions --crop --crop-mode face
```

Run from a config file:

```powershell
cargo run -- --config ".\showcase-config.json"
```

Run the local API:

```powershell
cargo run -- --api --api-bind "127.0.0.1:8787"
```

## Output

By default, output is written under `./output/`.

Common files:

- `clip_*.mp4`
- `benchmark.csv` or configured benchmark file
- `ai_storyboard.json`
- `export_bundle.json`
- `proof_report.md`
- `thumbnails/`

## Useful Flags

- `--input <PATH>`
- `--url <URL>`
- `--num-clips <N>`
- `--min-duration <SECONDS>`
- `--captions`
- `--crop`
- `--crop-mode <center|subject|face>`
- `--llm-enable`
- `--export-bundle`
- `--proof-report`
- `--thumbnails`

## Config

`--config` accepts JSON using CLI field names in `snake_case`.

Example:

```json
{
  "input": "tests/fixtures/sample.mp4",
  "num_clips": 3,
  "min_duration": 8,
  "captions": true,
  "crop": true,
  "crop_mode": "face",
  "export_bundle": true
}
```

## Development

Format, lint, and test:

```powershell
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## More Info

- [SECURITY.md](SECURITY.md)
- [deploy/README.md](deploy/README.md)
- [SHOWCASE.md](SHOWCASE.md)

## License

MIT
