Subject: API-ready short-form clipping pipeline with security controls and benchmark evidence

We built a local-first Rust pipeline that converts long-form video into short-form clips with transcript-aware ranking, vertical exports, animated subtitles, and platform metadata.

Why this is relevant:

- API mode with auth scopes, request limits, audit logs, and per-client daily quota persistence
- Multi-provider LLM reranking and metadata generation with fallback paths
- Reproducible evidence package (`benchmark.json`, `proof_report.md`, `export_bundle.json`, thumbnails)
- Subtitle quality harness with measurable readability and timing metrics

Pilot offer:

- 7-day API pilot
- your sample videos in, benchmarked short-form outputs out
- shared report with latency, readability, and clip quality signals

Assets:

- Smoke package: `output/showcase-smoke/`
- Full package: `output/showcase-full/`
- Subtitle benchmark: `output/subtitle-benchmark/subtitle-benchmark-summary.md`
