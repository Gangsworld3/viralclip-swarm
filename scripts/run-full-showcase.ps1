param(
    [string]$ConfigPath = ".\showcase-config.json",
    [string]$OutputDir = ".\output\showcase-full"
)

$ErrorActionPreference = "Stop"

if (!(Test-Path $ConfigPath)) {
    throw "Config not found: $ConfigPath"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$fullConfig = Join-Path $OutputDir "showcase.generated.json"
$raw = Get-Content $ConfigPath -Raw | ConvertFrom-Json

function Set-JsonProp {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)]$Value
    )
    if ($Object.PSObject.Properties.Name -contains $Name) {
        $Object.$Name = $Value
    } else {
        $Object | Add-Member -NotePropertyName $Name -NotePropertyValue $Value
    }
}

Set-JsonProp -Object $raw -Name "output_dir" -Value $OutputDir
Set-JsonProp -Object $raw -Name "csv_format" -Value "json"
Set-JsonProp -Object $raw -Name "csv_path" -Value (Join-Path $OutputDir "benchmark.json")
Set-JsonProp -Object $raw -Name "proof_report" -Value $true
Set-JsonProp -Object $raw -Name "proof_report_path" -Value (Join-Path $OutputDir "proof_report.md")
Set-JsonProp -Object $raw -Name "export_bundle" -Value $true
Set-JsonProp -Object $raw -Name "export_bundle_path" -Value (Join-Path $OutputDir "export_bundle.json")
Set-JsonProp -Object $raw -Name "llm_enable" -Value $true
Set-JsonProp -Object $raw -Name "thumbnails" -Value $true
Set-JsonProp -Object $raw -Name "thumbnails_dir" -Value (Join-Path $OutputDir "thumbnails")
Set-JsonProp -Object $raw -Name "thumbnail_collage" -Value $true
Set-JsonProp -Object $raw -Name "thumbnail_collage_path" -Value (Join-Path $OutputDir "collage.jpg")
Set-JsonProp -Object $raw -Name "subtitle_preset" -Value "creator_pro"
Set-JsonProp -Object $raw -Name "subtitle_animation" -Value "creator_pro"
Set-JsonProp -Object $raw -Name "subtitle_emoji_layer" -Value $true
Set-JsonProp -Object $raw -Name "subtitle_beat_sync" -Value $true
Set-JsonProp -Object $raw -Name "subtitle_scene_fx" -Value $true

$configuredInput = if ($raw.PSObject.Properties.Name -contains "input") { $raw.input } else { $null }
if ([string]::IsNullOrWhiteSpace($configuredInput) -or !(Test-Path $configuredInput)) {
    $fallbackInputs = @(
        ".\gang.mp4",
        ".\demo\sample-clip.mp4",
        ".\output\clip_1.mp4"
    )
    $resolved = $fallbackInputs | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($resolved)) {
        throw "No valid input found. Expected one of: $($fallbackInputs -join ', ')"
    }
    Write-Host "Using fallback input: $resolved"
    Set-JsonProp -Object $raw -Name "input" -Value $resolved
}
$raw | ConvertTo-Json -Depth 10 | Set-Content -Path $fullConfig

Write-Host "Running full showcase..."
cargo run -- --config $fullConfig

Write-Host "Generating preset demos..."
& ".\scripts\generate-preset-demos.ps1" -InputPath ".\demo\sample-clip.mp4" -OutputRoot ".\demo\presets"

Write-Host "Running subtitle benchmark..."
& ".\scripts\subtitle-benchmark.ps1" -OutputDir ".\output\subtitle-benchmark"

Write-Host "Full showcase done."
Write-Host "Key outputs:"
Write-Host " - $OutputDir"
Write-Host " - .\output\subtitle-benchmark\subtitle-benchmark-summary.md"
Write-Host " - .\demo\presets"
