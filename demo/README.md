# Demo Assets

This folder contains lightweight tracked demo metadata, not full media artifacts.

Current contents:

- `presets/<theme>/benchmark.json`: benchmark output for subtitle preset comparison runs

Large generated media is intentionally not tracked in the repository. Regenerate showcase output locally if you need preview images, clips, or collages.

Useful commands:

```powershell
cargo run -- --config ".\showcase-smoke.json"
.\scripts\generate-preset-demos.ps1
```
