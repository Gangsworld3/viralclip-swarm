param(
    [string]$ManifestPath = ".\docs\subtitle-benchmark-manifest.example.json",
    [string]$OutputDir = ".\output\subtitle-benchmark",
    [string]$Preset = "creator_pro",
    [string]$Animation = "creator_pro"
)

$ErrorActionPreference = "Stop"

if (!(Test-Path $ManifestPath)) {
    throw "Manifest not found: $ManifestPath"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$manifest = Get-Content $ManifestPath -Raw | ConvertFrom-Json
$rows = @()

foreach ($item in $manifest) {
    if (!(Test-Path $item.input)) {
        Write-Host "Skipping missing input: $($item.input)"
        continue
    }

    $runDir = Join-Path $OutputDir $item.id
    New-Item -ItemType Directory -Force -Path $runDir | Out-Null
    $bench = Join-Path $runDir "benchmark.json"

    $args = @(
        "run", "--",
        "--input", $item.input,
        "--num-clips", "3",
        "--min-duration", "8",
        "--captions",
        "--subtitle-preset", $Preset,
        "--subtitle-animation", $Animation,
        "--subtitle-emoji-layer", "true",
        "--subtitle-beat-sync", "true",
        "--subtitle-scene-fx", "true",
        "--subtitles-mode", "ass",
        "--output-dir", $runDir,
        "--csv-format", "json",
        "--csv-path", $bench
    )

    Write-Host "Running benchmark for $($item.id)..."
    cargo @args | Out-Null

    if (!(Test-Path $bench)) {
        Write-Host "No benchmark output for $($item.id), skipping."
        continue
    }
    $json = Get-Content $bench -Raw | ConvertFrom-Json
    $clips = @($json.clips | Where-Object { $_.success -eq $true })
    if ($clips.Count -eq 0) {
        continue
    }
    $avgReadability = ($clips | Measure-Object -Property readability_score -Average).Average
    $avgSubtitleMs = ($clips | Measure-Object -Property subtitles_ms -Average).Average
    $avgTotalMs = ($clips | Measure-Object -Property total_ms -Average).Average

    $rows += [PSCustomObject]@{
        id = $item.id
        input = $item.input
        clips = $clips.Count
        avg_readability = [Math]::Round($avgReadability, 3)
        avg_subtitles_ms = [Math]::Round($avgSubtitleMs, 1)
        avg_total_ms = [Math]::Round($avgTotalMs, 1)
        note = $item.note
    }
}

$jsonOut = Join-Path $OutputDir "subtitle-benchmark-summary.json"
$mdOut = Join-Path $OutputDir "subtitle-benchmark-summary.md"
$rows | ConvertTo-Json -Depth 5 | Set-Content -Path $jsonOut

$md = @()
$md += "# Subtitle Benchmark Summary"
$md += ""
$md += "Preset: $Preset | Animation: $Animation"
$md += ""
$md += "| Case | Clips | Avg Readability | Avg Subtitle ms | Avg Total ms | Note |"
$md += "|---|---:|---:|---:|---:|---|"
foreach ($r in $rows) {
    $md += "| $($r.id) | $($r.clips) | $($r.avg_readability) | $($r.avg_subtitles_ms) | $($r.avg_total_ms) | $($r.note) |"
}
$md -join "`n" | Set-Content -Path $mdOut

Write-Host "Wrote:"
Write-Host " - $jsonOut"
Write-Host " - $mdOut"
