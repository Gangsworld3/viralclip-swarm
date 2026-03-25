param(
    [string]$InputPath = ".\demo\sample-clip.mp4",
    [string]$OutputRoot = ".\demo\presets"
)

$ErrorActionPreference = "Stop"

if (!(Test-Path $InputPath)) {
    throw "Input video not found: $InputPath"
}

$presets = @(
    @{ name = "creator_pro"; animation = "creator_pro" },
    @{ name = "creator_neon"; animation = "impact" },
    @{ name = "creator_minimal"; animation = "emphasis" }
)

foreach ($preset in $presets) {
    $outDir = Join-Path $OutputRoot $preset.name
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    Copy-Item -Force $InputPath (Join-Path $outDir "before.mp4")
    $bench = Join-Path $outDir "benchmark.json"

    Write-Host "Generating preset demo: $($preset.name)"
    cargo run -- `
      --input $InputPath `
      --num-clips 1 `
      --min-duration 8 `
      --captions `
      --subtitle-preset $preset.name `
      --subtitle-animation $preset.animation `
      --subtitle-emoji-layer true `
      --subtitle-beat-sync true `
      --subtitle-scene-fx true `
      --subtitles-mode ass `
      --output-dir $outDir `
      --csv-format json `
      --csv-path $bench `
      --thumbnails `
      --thumbnails-dir (Join-Path $outDir "thumbnails")

    $after = Join-Path $outDir "clip_1.mp4"
    if (Test-Path $after) {
        Copy-Item -Force $after (Join-Path $outDir "after.mp4")
    }
}

Write-Host "Preset demos generated in $OutputRoot"
