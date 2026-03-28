# Packaging

## Repo Packaging Goals

The repository should be easy to evaluate in under five minutes.

Required pieces:

- a clear README
- reproducible demo configs in the repo
- reproducible configs in the repo root
- contribution guidance
- release notes or tag history

## Recommended Release Layout

- `README.md`: product overview and quick start
- `demo/`: lightweight demo metadata or small proof artifacts
- `docs/GITHUB_ABOUT.md`: GitHub about metadata
- `SHOWCASE.md`: how to reproduce demo runs
- `RELEASE_SUMMARY.md`: concise release narrative
- `CONTRIBUTING.md`: contribution rules

## Binary Packaging

Current packaging path:

- build with `cargo build --release`
- distribute the binary with a short setup guide for `ffmpeg`

Future improvements:

- GitHub Actions build artifacts for Windows/Linux/macOS
- checksummed release bundles
- one-command installer scripts
- optional Docker image for API mode
