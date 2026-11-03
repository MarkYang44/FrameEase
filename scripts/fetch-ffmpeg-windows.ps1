# Fetch a static GPL FFmpeg build for Windows x64 (BtbN/FFmpeg-Builds) into
# src-tauri/resources/ffmpeg. Used by CI and by developers building the Windows
# installer; the app itself never downloads anything.
#
#   ./scripts/fetch-ffmpeg-windows.ps1 [-Url <zip url>]
param(
  [string]$Url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-gpl-8.1.zip"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root "src-tauri/resources/ffmpeg"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("frameease-ffmpeg-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null

Write-Host "Downloading $Url"
Invoke-WebRequest -Uri $Url -OutFile (Join-Path $tmp "ffmpeg.zip")
Expand-Archive (Join-Path $tmp "ffmpeg.zip") -DestinationPath $tmp

$bin = Get-ChildItem $tmp -Recurse -Filter ffmpeg.exe | Select-Object -First 1
if (-not $bin) { throw "ffmpeg.exe not found in archive" }

Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $bin.DirectoryName "ffmpeg.exe") $out
Copy-Item (Join-Path $bin.DirectoryName "ffprobe.exe") $out
$license = Get-ChildItem $tmp -Recurse -Filter "LICENSE*" | Select-Object -First 1
if ($license) { Copy-Item $license.FullName (Join-Path $out "LICENSE.txt") }

& (Join-Path $out "ffmpeg.exe") -hide_banner -version | Select-Object -First 1 | Set-Content (Join-Path $out "VERSION.txt")
& (Join-Path $out "ffmpeg.exe") -hide_banner -buildconf | Set-Content (Join-Path $out "BUILDCONF.txt")
@"
FFmpeg for Windows from $Url
Static GPL build by BtbN/FFmpeg-Builds (https://github.com/BtbN/FFmpeg-Builds), distributed
under the GNU GPL v3. Source: https://ffmpeg.org/releases/ and the component projects listed
in BUILDCONF.txt. FrameEase runs FFmpeg as a separate program and does not link to it.
"@ | Set-Content (Join-Path $out "LICENSE-NOTICE.txt")

Remove-Item $tmp -Recurse -Force
Write-Host "FFmpeg ready in $out"
Get-Content (Join-Path $out "VERSION.txt")
