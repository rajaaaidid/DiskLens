# Full-resolution screenshot of the running UI dev server via headless Edge.
# Usage: scripts\screenshot.ps1 -Query "theme=light&hover=pagefile.sys" -Out docs\mockup\light.png
param(
  [string]$Query = "",
  [Parameter(Mandatory = $true)][string]$Out,
  [int]$Width = 1600,
  [int]$Height = 980,
  [string]$Base = "http://localhost:1420/"
)
$edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
if (-not (Test-Path $edge)) { $edge = "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe" }
$profileDir = Join-Path $env:TEMP "disklens-shot-profile"
$outPath = [IO.Path]::GetFullPath($Out)
New-Item -ItemType Directory -Force (Split-Path $outPath) | Out-Null
$url = if ($Query) { "$Base`?$Query" } else { $Base }
& $edge --headless=new --use-angle=d3d11 --hide-scrollbars --force-device-scale-factor=1 `
  --user-data-dir="$profileDir" --window-size="$Width,$Height" --virtual-time-budget=4000 `
  --screenshot="$outPath" $url 2>$null | Out-Null
Get-Item $outPath | Select-Object FullName, Length
