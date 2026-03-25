# ViralClip Swarm Pitch For AI Companies

## One-Line Positioning

ViralClip Swarm is a local-first Rust clipping engine that turns long-form video into ranked short clips, vertical exports, captions, and platform-ready metadata with reproducible benchmark artifacts.

## Why It Matters

- It is not just a demo model wrapper; it is an end-to-end media pipeline.
- It produces structured outputs that are useful for evaluation and orchestration.
- It now includes a local API mode with queueing and optional auth via `x-api-key`.
- It ships with reproducible showcase configs and proof artifacts.

## What Is Already Working

- transcript-aware ranking
- face-aware crop fallback path
- animated captions
- platform export bundles
- proof reports
- thumbnail extraction and collage generation
- queued local API server

## Evidence In Repo

Smoke package:

- `output/showcase-smoke/benchmark.json`
- `output/showcase-smoke/export_bundle.json`
- `output/showcase-smoke/proof_report.md`
- `output/showcase-smoke/thumbnails/collage.jpg`

Full showcase package:

- `output/showcase/benchmark.json`
- `output/showcase/export_bundle.json`
- `output/showcase/proof_report.md`
- `output/showcase/thumbnails/`

## Current Showcase Metrics

Full showcase run on `gang.mp4`:

- 5 clips selected
- 100% clip success rate
- average total score: 1.175
- best clip score: 1.986
- average end-to-end clip processing time: about 54s after selection

## Partnership Angle

This is a strong fit for:

- local media AI workflows
- creator tooling backends
- clip-ranking experiments
- evaluation pipelines for transcript-aware or multimodal ranking models

## What To Ask For

- model partnerships for stronger semantic clip ranking
- transcript or vision APIs for better hook detection and subject tracking
- infra support for turning the local API server into a production worker service
