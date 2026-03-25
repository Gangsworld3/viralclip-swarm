# Showcase

This repository includes a reproducible evidence path, not just feature code.

## What To Show

- `output/clip_*.mp4` for generated clips
- `output/benchmark.csv` or `output/benchmark.json` for run metrics
- `output/export_bundle.json` for platform packaging metadata
- `output/proof_report.md` for human-readable evidence

## How To Produce A Fresh Showcase

Fast smoke showcase:

```powershell
cargo run -- --config ".\showcase-smoke.json"
```

This writes a compact evidence package to `output/showcase-smoke/`.

Full local run:

```powershell
cargo run -- --input ".\gang.mp4" --num-clips 5 --min-duration 10 --captions --subtitle-preset legendary --subtitle-animation emphasis --crop --crop-mode face --llm-enable --llm-provider heuristic --export-bundle --proof-report
```

Example reproducible config run:

```powershell
cargo run -- --config ".\showcase-config.json"
```

## What Evaluators Should Look For

- Whether the chosen clips avoid duplicates
- Whether the caption styling is readable on vertical exports
- Whether the face-aware crop keeps the subject framed well
- Whether the AI metadata matches the actual spoken moment
- Whether the proof report metrics align with the generated files

## Recommended Demo Package

For outreach to AI companies or large creators, package these together:

- 3 to 5 generated clips
- the matching `proof_report.md`
- the matching `export_bundle.json`
- one source video reference
- one short note explaining why those moments were selected

## Current Repo Evidence

The repository now includes a generated smoke package under `output/showcase-smoke/` with:

- `benchmark.json`
- `ai_storyboard.json`
- `export_bundle.json`
- `proof_report.md`
- a generated clipped video
- `thumbnails/clip_1.jpg`
- `thumbnails/collage.jpg`

That package is the fastest artifact set to hand to someone evaluating the project quickly.
