# Build a no-install ZIP from an existing Windows release build. -SmokeTest is
# intended for a fresh CI user profile: it starts the real GUI, never an installer.
param([switch]$SmokeTest)
$ErrorActionPreference = "Stop"
if (-not $IsWindows) { throw "The portable ZIP must be packaged and checked on Windows" }
$root = Split-Path -Parent $PSScriptRoot
$json = & node (Join-Path $PSScriptRoot "stage-windows-portable.mjs")
if ($LASTEXITCODE -ne 0) { throw "Could not stage the portable distribution" }
$package = $json | ConvertFrom-Json
Compress-Archive -LiteralPath $package.directory -DestinationPath $package.zip -CompressionLevel Optimal -Force

# Read back the actual archive from a Unicode/spaced path and compare every byte.
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("FrameEase 解压测试 " + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  Expand-Archive -LiteralPath $package.zip -DestinationPath $tmp
  $unpacked = Join-Path $tmp (Split-Path $package.directory -Leaf)
  $want = @(Get-ChildItem -LiteralPath $package.directory -File -Recurse | ForEach-Object {
    [IO.Path]::GetRelativePath($package.directory, $_.FullName)
  } | Sort-Object)
  $got = @(Get-ChildItem -LiteralPath $unpacked -File -Recurse | ForEach-Object {
    [IO.Path]::GetRelativePath($unpacked, $_.FullName)
  } | Sort-Object)
  if (($want -join "|") -ne ($got -join "|")) { throw "The ZIP's file list differs from the staged package" }
  foreach ($name in $want) {
    $sourceHash = (Get-FileHash -LiteralPath (Join-Path $package.directory $name) -Algorithm SHA256).Hash
    $zipHash = (Get-FileHash -LiteralPath (Join-Path $unpacked $name) -Algorithm SHA256).Hash
    if ($sourceHash -ne $zipHash) { throw "The ZIP changed $name" }
  }

  $savedPath = $env:PATH
  $app = $null
  try {
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
    foreach ($tool in "ffmpeg.exe", "ffprobe.exe") {
      & (Join-Path $unpacked "ffmpeg/$tool") -hide_banner -version | Out-Null
      if ($LASTEXITCODE -ne 0) { throw "$tool could not start from the extracted ZIP" }
    }
    if ($SmokeTest) {
      $identifier = (Get-Content (Join-Path $root "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json).identifier
      $data = Join-Path ([Environment]::GetFolderPath("ApplicationData")) $identifier
      if (Test-Path -LiteralPath $data) { throw "Native smoke test requires a fresh user profile; refusing to open an existing queue" }
      $app = Start-Process -FilePath (Join-Path $unpacked "FrameEase.exe") -WorkingDirectory $tmp -PassThru
      $deadline = [DateTime]::UtcNow.AddSeconds(90)
      $ready = $false
      do {
        Start-Sleep -Milliseconds 250
        $app.Refresh()
        if ($app.HasExited) { throw "FrameEase exited during portable startup (code $($app.ExitCode))" }
        $capsPath = Join-Path $data "capabilities.json"
        if ($app.MainWindowHandle -ne 0 -and (Test-Path -LiteralPath $capsPath)) {
          $caps = Get-Content -LiteralPath $capsPath -Raw | ConvertFrom-Json
          $expected = [IO.Path]::GetFullPath((Join-Path $unpacked "ffmpeg/ffmpeg.exe"))
          $actual = [IO.Path]::GetFullPath($caps.ffmpeg.ffmpeg)
          # Tauri canonicalizes its executable path, which adds the Win32 long-path prefix.
          if ($actual.StartsWith('\\?\')) { $actual = $actual.Substring(4) }
          if (-not [String]::Equals($actual, $expected, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Portable startup selected an FFmpeg outside the extracted folder"
          }
          $ready = $true
        }
      } until ($ready -or [DateTime]::UtcNow -gt $deadline)
      if (-not $ready) { throw "Portable startup did not create a native window and detect its bundled FFmpeg within 90 seconds" }
      Write-Host "Native portable startup passed: a real window opened and the extracted FFmpeg was detected"
    }
  } finally {
    $env:PATH = $savedPath
    if ($null -ne $app -and -not $app.HasExited) {
      & "$env:SystemRoot\System32\taskkill.exe" /PID $app.Id /T /F | Out-Null
    }
  }
} finally {
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
$size = (Get-Item -LiteralPath $package.zip).Length / 1e6
Write-Host ("::notice title=Windows no-install ZIP::{0} ({1:N1} MB); all files and FFmpeg startup verified" -f (Split-Path $package.zip -Leaf), $size)
