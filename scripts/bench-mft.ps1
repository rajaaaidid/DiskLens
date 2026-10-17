# Benchmark the MFT fast path against the directory walk on every NTFS drive. Must run elevated.
# Usage (admin terminal): scripts\bench-mft.ps1 [-Out results.txt]
param([string]$Out = "$PSScriptRoot\..\docs\bench-mft.txt")
$exe = Join-Path $PSScriptRoot "..\target\release\disklens.exe"
$env:DISKLENS_PROFILE = "1"
$lines = @("DiskLens MFT benchmark  $(Get-Date -Format s)", "")
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$lines += "elevated: $admin"
foreach ($v in Get-Volume | Where-Object { $_.DriveLetter -and $_.FileSystem -eq 'NTFS' -and $_.DriveType -eq 'Fixed' } | Sort-Object DriveLetter) {
  $drive = "$($v.DriveLetter):\"
  Write-Host "Benchmarking $drive (MFT x3, then MFT vs walk)..."
  $lines += ""
  $lines += "=== $drive  ($([math]::Round(($v.Size - $v.SizeRemaining) / 1GB)) GB used) ==="
  $lines += "--- MFT, 3 runs ---"
  $lines += (& $exe bench $drive --runs 3 --method mft 2>&1 | ForEach-Object { "$_" })
  $lines += "--- MFT vs walk ---"
  $lines += (& $exe compare $drive 2>&1 | ForEach-Object { "$_" })
}
$lines | Out-File -FilePath $Out -Encoding utf8
"done" | Out-File -FilePath "$Out.done" -Encoding utf8
Write-Host "Done. Results written to $Out"
