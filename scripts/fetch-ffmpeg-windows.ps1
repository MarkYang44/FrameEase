# Fetch the shared GPL build of FFmpeg 8.1 for Windows x64 (BtbN/FFmpeg-Builds) into
# src-tauri/resources/ffmpeg: ffmpeg.exe, ffprobe.exe and the FFmpeg DLLs beside them. The
# codec libraries (x264, x265, SVT-AV1, libaom, dav1d, libass...) are linked statically into
# those DLLs, so the codec set ships once; the static build embeds it in both programs.
# Windows loads a program's DLLs from the program's own folder first, and the app starts
# ffmpeg.exe by full path, so nothing depends on PATH. Used by CI and by developers building
# the Windows installer; the app itself never downloads anything.
#
#   ./scripts/fetch-ffmpeg-windows.ps1 [-Url <zip url of a shared build>]
param(
  [string]$Url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-gpl-shared-8.1.zip"
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue" # progress output slows large downloads
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root "src-tauri/resources/ffmpeg"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("frameease-ffmpeg-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null

Write-Host "Downloading $Url"
Invoke-WebRequest -Uri $Url -OutFile (Join-Path $tmp "ffmpeg.zip")
Expand-Archive (Join-Path $tmp "ffmpeg.zip") -DestinationPath $tmp

$bin = Get-ChildItem $tmp -Recurse -Filter ffmpeg.exe | Select-Object -First 1
if (-not $bin) { throw "ffmpeg.exe not found in archive" }
# The two programs the app runs and every DLL beside them (ffplay.exe is not used).
$dlls = @(Get-ChildItem $bin.DirectoryName -Filter *.dll | Sort-Object Name)
if ($dlls.Count -eq 0) { throw "no DLLs next to ffmpeg.exe: expected a shared build" }

Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $bin.DirectoryName "ffmpeg.exe") $out
Copy-Item (Join-Path $bin.DirectoryName "ffprobe.exe") $out
$dlls | Copy-Item -Destination $out
$license = Get-ChildItem $tmp -Recurse -Filter "LICENSE*" | Select-Object -First 1
if ($license) { Copy-Item $license.FullName (Join-Path $out "LICENSE.txt") }

# Start both programs the way the app does (full path into this folder) with only Windows
# itself on PATH, so a DLL missing here fails the build instead of the user's first job.
$savedPath = $env:PATH
$env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
try {
  foreach ($exe in "ffmpeg.exe", "ffprobe.exe") {
    & (Join-Path $out $exe) -hide_banner -version | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "$exe does not start from $out (exit code $LASTEXITCODE)" }
  }
  $version = @(& (Join-Path $out "ffmpeg.exe") -hide_banner -version)[0]
  $buildconf = @(& (Join-Path $out "ffmpeg.exe") -hide_banner -buildconf)
} finally {
  $env:PATH = $savedPath
}
$version | Set-Content (Join-Path $out "VERSION.txt")
$buildconf | Set-Content (Join-Path $out "BUILDCONF.txt")

# FrameEase's software encoders, AV1 decoding and subtitle burn-in come from these libraries.
$missing = @("libx264", "libx265", "libsvtav1", "libaom", "libdav1d", "libass") |
  Where-Object { -not ($buildconf -match ('^\s*--enable-' + $_ + '\s*$')) }
if ($missing) { throw "this FFmpeg build lacks $($missing -join ', ')" }

$files = @("ffmpeg.exe", "ffprobe.exe") + @($dlls | ForEach-Object { $_.Name })
@"
This folder contains ffmpeg.exe and ffprobe.exe from
$version
and the FFmpeg DLLs they load, copied unmodified from the shared GPL build for Windows x64
by BtbN/FFmpeg-Builds (https://github.com/BtbN/FFmpeg-Builds):
$Url
The other libraries FFmpeg uses (x264, x265, SVT-AV1, libaom, dav1d, libass and the rest
listed in BUILDCONF.txt) are linked statically into those DLLs.

FFmpeg is licensed under the GNU LGPL v2.1 or later. This build enables GPL components
(see BUILDCONF.txt: --enable-gpl / --enable-version3), so these FFmpeg binaries are
distributed under the GNU GPL v3 (LICENSE.txt). FrameEase runs FFmpeg as a separate
program and does not link to it.

Corresponding source code: FFmpeg at https://ffmpeg.org/releases/ (version above) and the
projects listed in BUILDCONF.txt, at the revisions pinned by the build scripts at
https://github.com/BtbN/FFmpeg-Builds.

Bundled files: $($files -join ", ")
"@ | Set-Content (Join-Path $out "LICENSE-NOTICE.txt")

Remove-Item $tmp -Recurse -Force
$size = (Get-ChildItem $out -File | Measure-Object Length -Sum).Sum
Write-Host ("FFmpeg ready in {0}: {1} files, {2:N1} MB" -f $out, @(Get-ChildItem $out -File).Count, ($size / 1e6))
Write-Host $version
