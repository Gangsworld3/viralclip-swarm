# v0.4.0 - Showcase Automation, Subtitle Benchmark, API Quota Hardening

## Highlights

- Added one-command showcase automation with:
  - full run generation
  - preset demo generation
  - subtitle benchmark execution
- Added creator subtitle theme stack:
  - `creator_pro`
  - `creator_neon`
  - `creator_minimal`
  - `creator_bold`
- Added subtitle quality benchmark harness and summary outputs.
- Added outreach pack templates for AI companies and influencers.
- Added API hardening follow-up:
  - persistent per-client daily run quotas
  - configurable quota state store
  - token rotation helper script

## Key Commands

```powershell
.\scripts\run-full-showcase.ps1
.\scripts\generate-preset-demos.ps1
.\scripts\subtitle-benchmark.ps1
.\scripts\rotate-api-token.ps1
```

## Evidence Links (Repo Paths)

- `demo/`
- `demo/presets/`
- `output/showcase-full/proof_report.md`
- `output/showcase-full/export_bundle.json`
- `output/subtitle-benchmark/subtitle-benchmark-summary.md`
