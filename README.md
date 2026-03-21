# ViralClip Swarm

> AI‑powered tool that turns long videos into viral shorts – fast copy, parallel processing, optional captions & crop.

[![GitHub stars](https://img.shields.io/github/stars/Gangsworld3/viralclip-swarm)](https://github.com/Gangsworld3/viralclip-swarm/stargazers)
[![License](https://img.shields.io/github/license/Gangsworld3/viralclip-swarm)](LICENSE)

**See it in action:**  
![Demo](assets/demo.gif)

## ✨ Features

- **Fast extraction** – default `-c copy` is lightning fast.
- **Parallel processing** – uses `rayon` to process clips simultaneously.
- **Smart clipping** – detects the loudest segments (energy‑based).
- **Optional captions** – add word‑level subtitles with a single Whisper pass.
- **Optional 9:16 crop** – perfect for TikTok/Reels/Shorts.
- **Frame‑accurate cuts** – use `--accurate` to re‑encode if needed.
- **Works with YouTube or local files** – powered by `yt-dlp`.

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

Usage
bash
# Basic usage (fast copy, 5 clips, 8 seconds each)
viralclip-swarm --url "https://youtu.be/..." --num-clips 5 --min-duration 8

# Local file with captions (small model) and crop
viralclip-swarm --input video.mp4 --captions --model tiny --crop

# Accurate cuts (slower but frame‑exact)
viralclip-swarm --url "..." --accurate
All clips are saved in ./output/ by default (change with --output-dir).

📊 How It Works
Download / load video.

Extract audio (16kHz mono PCM).

Analyze energy – splits audio into 1‑second windows, computes RMS.

Select top N windows with a gap of at least min_duration seconds.

Process clips in parallel:

Extract raw clip (fast copy or re‑encode).

(Optional) Burn captions using pre‑computed Whisper SRT.

(Optional) Crop to 9:16.

Save final clip.

⚙️ Performance Tips
Use --accurate only when you need exact frame cuts.

For captions, start with --model tiny to test; larger models are slower but more accurate.

For local files, avoid --url overhead.

🤝 Contributing
Issues and pull requests are welcome! See CONTRIBUTING.md (soon).

📄 License
MIT License – see LICENSE.

🙌 Acknowledgements
yt-dlp

FFmpeg

Whisper

Rayon

