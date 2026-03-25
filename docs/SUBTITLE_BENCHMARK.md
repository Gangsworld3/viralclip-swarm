# Subtitle Benchmark Harness

This project includes a lightweight benchmark harness for subtitle readability and timing quality.

## Inputs

Prepare a manifest JSON similar to:

- [subtitle-benchmark-manifest.example.json](/C:/Users/hp/Documents/New%20folder/viralclip-swarm/docs/subtitle-benchmark-manifest.example.json)

Each entry should point to an existing local media file.

## Run

```powershell
.\scripts\subtitle-benchmark.ps1 `
  -ManifestPath .\docs\subtitle-benchmark-manifest.example.json `
  -Preset creator_pro `
  -Animation creator_pro
```

## Outputs

- `output/subtitle-benchmark/subtitle-benchmark-summary.json`
- `output/subtitle-benchmark/subtitle-benchmark-summary.md`
- per-case clip outputs and benchmark logs

## Metrics

- average `readability_score`
- average subtitle render time (`subtitles_ms`)
- average total per-clip time (`total_ms`)
