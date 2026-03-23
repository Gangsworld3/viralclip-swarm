# 🎬 ViralClip Swarm
> A local first, AI powered pipeline that turns long form video (YouTube, Twitch VODs, podcasts, local files) into short, social ready clips automatically. It detects high engagement moments using audio, visual, and text signals, then trims, crops, captions, and exports clips optimized for TikTok, Instagram Reels, YouTube Shorts, and Twitter.

[![GitHub stars](https://img.shields.io/github/stars/Gangsworld3/viralclip-swarm)](https://github.com/Gangsworld3/viralclip-swarm/stargazers)
[![License](https://img.shields.io/github/license/Gangsworld3/viralclip-swarm)](LICENSE)

**See it in action:**  
![Demo](assets/demo.gif)

## ✨ Features

- **Fast extraction** – default `-c copy` is lightning fast.
- **Parallel processing** – uses `rayon` to process clips simultaneously.
- **Smart clipping** – detects the loudest segments (audio energy).
- **Motion detection** – identifies scene changes with `ffprobe` (optional).
- **Dynamic captions** – word‑level subtitles using Whisper (optional).
- **Stylable subtitles** – choose font, size, colour, outline (optional).
- **Optional 9:16 crop** – perfect for TikTok/Reels/Shorts.
- **Frame‑accurate cuts** – use `--accurate` to re‑encode if needed.
- **Works with YouTube or local files** – powered by `yt-dlp`.
- **Benchmark logging** – CSV, JSON, or human‑readable timings.

## 🚀 Quick Start

### Prerequisites

- [Rust](https://rustup.rs/)
- [FFmpeg](https://ffmpeg.org/) (with `ffprobe`)
- [yt-dlp](https://github.com/yt-dlp/yt-dlp)
- [Whisper](https://github.com/openai/whisper) (optional, for captions)

Install Whisper:
```bash
pip install openai-whisper
Install from source
bash
git clone https://github.com/Gangsworld3/viralclip-swarm.git
cd viralclip-swarm
cargo build --release
The binary will be in target/release/viralclip-swarm.exe (Windows) or target/release/viralclip-swarm (Linux/macOS).
```
Usage
```
bash
# Basic usage (fast copy, 5 clips, 8 seconds each)
viralclip-swarm --url "https://youtu.be/..." --num-clips 5 --min-duration 8
# Local file with captions (small model) and crop
viralclip-swarm --input video.mp4 --captions --model tiny --crop
# Motion detection + captions + crop
viralclip-swarm --input video.mp4 --motion --captions --crop
# Styled subtitles
viralclip-swarm --input video.mp4 --captions --subtitle-font Arial --subtitle-size 32 --subtitle-color "&H0000FF00" --subtitle-outline 3
# Accurate cuts (slower but frame‑exact)
viralclip-swarm --url "..." --accurate
All clips are saved in ./output/ by default (change with --output-dir).
```
# 📊 How It Works
##### Download / load video.
##### Extract audio (16kHz mono PCM).
##### Analyze energy – splits audio into 1‑second windows, computes RMS.
##### Optional motion detection – detects scene changes with ffprobe and boosts energy at those timestamps.
##### Select top N windows with a gap of at least min_duration seconds.
##### Process clips in parallel:
##### Extract raw clip (fast copy or re‑encode).
##### (Optional) Burn captions using pre‑computed Whisper SRT.
##### (Optional) Crop to 9:16.
##### Save final clip.
##### Log performance – records timings in CSV/JSON/human format.

# ⚙️ Command Line Options
Option	Description
--url	YouTube video URL
--input	Local input video file
--num-clips	Number of clips to generate (default: 10)
--min-duration	Clip duration in seconds (default: 5.0)
--output-dir	Output directory (default: ./output)
--captions	Enable dynamic captions (requires Whisper)
--crop	Crop to 9:16 vertical format
--accurate	Use accurate cuts (re‑encode, slower)
--motion	Enable motion detection (scene changes)
--scene-threshold	Threshold for scene change detection (0.0–1.0, default: 0.4)
--whisper-mode	Whisper backend: python (recommended) or binary
--whisper-model	Whisper model: tiny, base, small, medium, large (default: base)
--subtitle-font	Font name (default: Monospace)
--subtitle-size	Font size in pixels (default: 24)
--subtitle-color	Colour in ASS hex (e.g., &H00FFFFFF)
--subtitle-outline	Outline width (default: 2)
--subtitle-border-style	1 = outline, 3 = opaque box
--subtitles-mode	Subtitle filter: auto, ass, subtitles (default: auto)
--csv-format	Benchmark output: csv, json, human (default: csv)
--csv-path	Path to benchmark file (default: ./output/benchmark.csv)
--append	Append to benchmark file instead of overwriting
--timestamp-mode	utc or local (default: utc)
# 📈 Benchmarking
The tool logs performance for each clip, including extraction time, subtitle burning time, crop time, and total time. Output can be:
CSV – machine‑readable, with a commented header.
JSON – structured, with summary and clip details.
Human – formatted table for quick inspection.
### Example (human mode):
```
text
Run started: 2025-03-23T10:30:45Z (23 Mar 2025, 10:30)
Total clips: 3, Total duration: 18.32s
clip_id | start | dur | extract(s) | subs(s) | crop(s) | total(s) | success | timestamp | human_time | error
      1 |   5.0 | 5.0 |      0.123 |    0.456 |   0.789 |    1.368 |    true | 2025-03-23T10:30:45Z | 23 Mar 2025, 10:30 |
```
# 🤝 Contributing
Issues and pull requests are welcome! See CONTRIBUTING.md (coming soon).

# 📄 License
MIT License – see LICENSE.

# 🙌 Acknowledgements
yt-dlp
FFmpeg
Whisper
Rayon
colored
clap

### Built with ❤️ for creators and engineers.
### Star this repo to follow the journey!
