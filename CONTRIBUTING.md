# Contributing

## Scope

Contributions should improve one of these areas:

- clip ranking quality
- creator-facing output quality
- API reliability
- evaluation and benchmark coverage
- setup and packaging

## Development Setup

1. Install Rust.
2. Install `ffmpeg` and `ffprobe`.
3. Optionally install `yt-dlp` for URL input.
4. Optionally install local Whisper or configure cloud credentials.
5. Build and test:

```powershell
cargo build
cargo test
```

## Pull Request Expectations

- Keep changes focused.
- Add or update tests when behavior changes.
- Do not commit generated `output/`, `.vs/`, `target/`, or IDE artifacts.
- Document new flags, config fields, or API changes in `README.md`.
- If you change showcase behavior, refresh the demo instructions or packaging docs when needed.

## Code Style

- Prefer simple, explicit Rust over clever abstractions.
- Keep CLI/config field names aligned.
- Preserve local-first behavior unless there is a clear reason not to.
- Add short comments only where the code is not self-explanatory.

## Reporting Issues

When filing an issue, include:

- operating system
- ffmpeg version
- exact command or config used
- error output
- whether local or cloud transcription/LLM mode was enabled
