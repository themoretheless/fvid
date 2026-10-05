#Requires -Version 5.1
<#
.SYNOPSIS
  Optional external FFmpeg setup for explicit reference benchmarks.
  Production FVid media/player/CUDA builds do not require this script.
#>
param(
    [string]$Prefix = "C:\ffmpeg-shared",
    [string]$Url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip"
)

$ErrorActionPreference = "Stop"

Write-Host "Downloading $Url ..."
$zip = Join-Path $env:TEMP "ffmpeg-win64-gpl-shared.zip"
Invoke-WebRequest -Uri $Url -OutFile $zip -UseBasicParsing

$extract = Join-Path $env:TEMP "ffmpeg-win64-extract"
if (Test-Path $extract) { Remove-Item -Recurse -Force $extract }
Expand-Archive -Path $zip -DestinationPath $extract -Force

$root = Get-ChildItem $extract -Directory | Select-Object -First 1
if (-not $root) { throw "Unexpected archive layout" }
if (Test-Path $Prefix) { Remove-Item -Recurse -Force $Prefix }
New-Item -ItemType Directory -Path $Prefix | Out-Null
Copy-Item -Recurse -Force (Join-Path $root.FullName "*") $Prefix

foreach ($need in @("lib\avutil.lib", "bin\ffmpeg.exe", "bin\ffprobe.exe")) {
    $path = Join-Path $Prefix $need
    if (-not (Test-Path $path)) { throw "Missing expected path after extract: $path" }
}

$env:FVID_FFMPEG_PREFIX = $Prefix
$env:PATH = "$(Join-Path $Prefix 'bin');$env:PATH"

Write-Host "FVID_FFMPEG_PREFIX=$Prefix"
Write-Host "Reference benchmark PATH includes FFmpeg bin. Production FVid does not need these settings."
Write-Host "Optional benchmark settings:"
Write-Host "  [Environment]::SetEnvironmentVariable('FVID_FFMPEG_PREFIX','$Prefix','User')"
Write-Host "  # and append $Prefix\bin to User PATH"
