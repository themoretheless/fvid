# Prepend CUDA 13 bin (+ bin\x64 on Windows) for NVRTC DLL load / tools.
$cudaRoot = if ($env:CUDA_PATH) { $env:CUDA_PATH } else {
    Get-ChildItem "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA" -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "v13*" } |
        Sort-Object Name -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $cudaRoot) {
    Write-Error "CUDA 13 toolkit not found. Install CUDA Toolkit 13.x or set CUDA_PATH."
    exit 1
}
$bin = Join-Path $cudaRoot "bin"
$binX64 = Join-Path $bin "x64"
# Toolkit 13 lays out nvrtc64_*.dll under bin\x64; nvcc stays in bin.
$env:PATH = if (Test-Path $binX64) { "$binX64;$bin;$env:PATH" } else { "$bin;$env:PATH" }
Write-Host "PATH prepended: $bin$(if (Test-Path $binX64) { " + $binX64" })"
$nvrtc = @($binX64, $bin) | ForEach-Object { Join-Path $_ "nvrtc64_130_0.dll" } | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $nvrtc) {
    Write-Error "nvrtc64_130_0.dll not found under $bin or $binX64"
    exit 1
}
Write-Host "nvrtc OK: $nvrtc"
