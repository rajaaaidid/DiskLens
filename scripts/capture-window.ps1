# Capture a top-level window (default: HaveSpace) to PNG with PrintWindow(PW_RENDERFULLCONTENT),
# which includes GPU/WebView2 content even when the window is partly covered.
# Usage: scripts\capture-window.ps1 -Out docs\screens\app.png [-Title HaveSpace]
param(
  [Parameter(Mandatory = $true)][string]$Out,
  [string]$Title = "HaveSpace"
)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
"@
[Win]::SetProcessDPIAware() | Out-Null
$h = [Win]::FindWindow($null, $Title)
if ($h -eq [IntPtr]::Zero) {
  $p = Get-Process | Where-Object { $_.MainWindowTitle -eq $Title -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if ($p) { $h = $p.MainWindowHandle }
}
if ($h -eq [IntPtr]::Zero) { throw "window '$Title' not found" }
$r = New-Object Win+RECT
[Win]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Right - $r.Left; $hgt = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap $w, $hgt
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[Win]::PrintWindow($h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc); $g.Dispose()
$outPath = [IO.Path]::GetFullPath($Out)
New-Item -ItemType Directory -Force (Split-Path $outPath) | Out-Null
$bmp.Save($outPath, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
"{0}  {1}x{2}" -f $outPath, $w, $hgt
