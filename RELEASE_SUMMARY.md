# Release Summary

## What Changed

This pass moved ViralClip Swarm from a strong CLI prototype into a more credible product package.

Implemented:

- transcript-aware ranking and hook scoring
- duplicate suppression
- face-aware crop mode
- animated ASS captions with multiple presets
- richer AI metadata from transcript context
- export bundles for Shorts, TikTok, and Reels
- config-file runs
- local API mode with queueing and optional auth
- proof report generation
- thumbnail extraction and collage generation
- showcase configs and demo docs

## Validation

- `cargo test` passes
- smoke showcase package generated successfully
- full showcase package generated successfully from `showcase-config.json`

## Current Demo Assets

- `output/showcase/`
- `output/showcase-smoke/`
- `SHOWCASE.md`
- `PITCH_AI_COMPANIES.md`
- `PITCH_CREATORS.md`

## Remaining Risks

- semantic ranking is still heuristic-heavy without stronger model integration
- local Whisper on CPU is slow for longer videos
- API server is intentionally lightweight and not production-hardened
- thumbnail styling is functional but not a full design system
