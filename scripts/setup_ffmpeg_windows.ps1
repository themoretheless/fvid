#Requires -Version 5.1
<#
.SYNOPSIS
  Download a pinned Windows shared FFmpeg 9.0 build (headers + import libs + DLLs)
  and optionally locate LLVM for bindgen/libclang.
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

foreach ($need in @("include\libavformat\avformat.h", "lib\avformat.lib", "bin")) {
    $path = Join-Path $Prefix $need
    if (-not (Test-Path $path)) { throw "Missing expected path after extract: $path" }
}

$env:FVID_FFMPEG_PREFIX = $Prefix
$env:PATH = "$(Join-Path $Prefix 'bin');$env:PATH"

$llvmCandidates = @(
    "${env:ProgramFiles}\LLVM\bin",
    "${env:ProgramFiles(x86)}\LLVM\bin"
) + @(Get-ChildItem "${env:ProgramFiles}\LLVM*" -Directory -ErrorAction SilentlyContinue | ForEach-Object { Join-Path $_.FullName "bin" })
$libclang = $llvmCandidates | Where-Object { Test-Path (Join-Path $_ "libclang.dll") } | Select-Object -First 1
if ($libclang) {
    $env:LIBCLANG_PATH = $libclang
    Write-Host "LIBCLANG_PATH=$libclang"
} else {
    Write-Warning "libclang.dll not found. Install LLVM (winget install LLVM.LLVM) and set LIBCLANG_PATH to its bin directory."
}

Write-Host "FVID_FFMPEG_PREFIX=$Prefix"
Write-Host "Session PATH includes FFmpeg bin. Persist with:"
Write-Host "  [Environment]::SetEnvironmentVariable('FVID_FFMPEG_PREFIX','$Prefix','User')"
Write-Host "  # and append $Prefix\bin to User PATH"
