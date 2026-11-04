# Check the Windows installers built by `tauri build`: each must carry every file of
# src-tauri/resources/ffmpeg (the FFmpeg DLLs included) in one folder, and ffmpeg.exe and
# ffprobe.exe must start from that folder with only Windows itself on PATH, as they do in the
# installed app. The packages are unpacked, not installed: 7-Zip for NSIS, an administrative
# install for MSI. Ends with a notice giving the installer sizes.
#
#   ./scripts/check-windows-installers.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$bundle = Join-Path $root "target/release/bundle"
$ffmpegDir = Join-Path $root "src-tauri/resources/ffmpeg"
$want = @(Get-ChildItem $ffmpegDir -File | ForEach-Object { $_.Name } | Sort-Object)
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("frameease-installers-" + [guid]::NewGuid())

function Test-Unpacked([string]$label, [string]$dir) {
  $ff = Get-ChildItem $dir -Recurse -Filter ffmpeg.exe | Select-Object -First 1
  if (-not $ff) { throw "${label}: no ffmpeg.exe inside" }
  $got = @(Get-ChildItem $ff.DirectoryName -File | ForEach-Object { $_.Name } | Sort-Object)
  if (($got -join "|") -ne ($want -join "|")) {
    throw "${label}: its ffmpeg folder holds [$($got -join ', ')], expected [$($want -join ', ')]"
  }
  # The app's About section points to this list next to the ffmpeg folder.
  if (-not (Test-Path (Join-Path (Split-Path $ff.DirectoryName) "THIRD_PARTY_LICENSES.md"))) {
    throw "${label}: THIRD_PARTY_LICENSES.md is missing next to the ffmpeg folder"
  }
  $savedPath = $env:PATH
  $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
  try {
    foreach ($exe in "ffmpeg.exe", "ffprobe.exe") {
      & (Join-Path $ff.DirectoryName $exe) -hide_banner -version | Out-Null
      if ($LASTEXITCODE -ne 0) { throw "${label}: $exe does not start (exit code $LASTEXITCODE)" }
    }
  } finally {
    $env:PATH = $savedPath
  }
  Write-Host "${label}: all $($want.Count) FFmpeg files present; ffmpeg.exe and ffprobe.exe start"
}

$nsis = @(Get-ChildItem (Join-Path $bundle "nsis") -Filter *.exe)
$msi = @(Get-ChildItem (Join-Path $bundle "msi") -Filter *.msi)
if ($nsis.Count -eq 0 -or $msi.Count -eq 0) { throw "expected NSIS and MSI installers in $bundle" }
$sizes = @()
foreach ($f in $nsis) {
  $dir = Join-Path $tmp $f.BaseName
  & 7z x -y "-o$dir" $f.FullName
  if ($LASTEXITCODE -ne 0) { throw "7-Zip could not unpack $($f.Name) (exit code $LASTEXITCODE)" }
  Test-Unpacked $f.Name $dir
  $sizes += "{0} {1:N1} MB" -f $f.Name, ($f.Length / 1e6)
}
foreach ($f in $msi) {
  $dir = Join-Path $tmp $f.BaseName
  $log = Join-Path $tmp "$($f.BaseName).log"
  $p = Start-Process msiexec.exe -ArgumentList "/a `"$($f.FullName)`" /qn TARGETDIR=`"$dir`" /l*v `"$log`"" -Wait -PassThru
  if ($p.ExitCode -ne 0) {
    $tail = (Get-Content $log -Tail 15 -ErrorAction SilentlyContinue) -join " | "
    throw "msiexec /a $($f.Name) failed (exit code $($p.ExitCode)): $tail"
  }
  Test-Unpacked $f.Name $dir
  $sizes += "{0} {1:N1} MB" -f $f.Name, ($f.Length / 1e6)
}
Remove-Item $tmp -Recurse -Force

$ffmpegMB = (Get-ChildItem $ffmpegDir -File | Measure-Object Length -Sum).Sum / 1e6
Write-Host ("::notice title=Windows installers::{0}; bundled FFmpeg folder {1:N1} MB uncompressed" -f ($sizes -join ", "), $ffmpegMB)
