# Build a portable DiskLens folder: dist\DiskLens\DiskLens.exe (the app, UI embedded, no dev server
# needed) plus dist\DiskLens\cli\disklens.exe. Windows file names are case-insensitive, so the app and
# the CLI can't both be called "disklens.exe" in one folder; the app is built as disklens-app.exe and
# renamed here.
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts\package.ps1 [-Installer]
#   -Installer  also build the NSIS installer (the Tauri bundler downloads NSIS the first time).
param([switch]$Installer)
$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
Push-Location "$root\app\ui"
try {
  if ($Installer) { npx tauri build } else { npx tauri build --no-bundle }
  if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
} finally { Pop-Location }
Push-Location $root
try {
  cargo build --release -p disklens-cli
  if ($LASTEXITCODE -ne 0) { throw "CLI build failed" }
} finally { Pop-Location }

$dist = Join-Path $root "dist\DiskLens"
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force "$dist\cli" | Out-Null
Copy-Item "$root\target\release\disklens-app.exe" "$dist\DiskLens.exe"
Copy-Item "$root\target\release\disklens.exe" "$dist\cli\disklens.exe"
Copy-Item "$root\README.md" "$dist\README.md"
Get-ChildItem $dist -Recurse -File | Select-Object @{n='File';e={$_.FullName.Substring($dist.Length + 1)}}, @{n='MB';e={[math]::Round($_.Length / 1MB, 1)}}
if ($Installer) { Get-ChildItem "$root\target\release\bundle" -Recurse -Include *.exe, *.msi | Select-Object FullName }
