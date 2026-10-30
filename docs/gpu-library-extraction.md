# Shared GPU extraction boundary

Only reusable Metal shared memory moved to Forma's `gpu-compute` library:
`metal_memory::MetalSharedBuffer`, its allocation lifetime, 16 KiB page alignment,
host copies, bounds checks, and import into native wgpu 30. Enable `native-wgpu`.
The module has no dependency on video, fvid, CoreVideo, YUV, codec parameters,
color conversion or a UI. Native wgpu is optional and restricted to Metal;
Forma's existing gpu-runtime and CUDA computation interfaces remain unchanged.
The original source license notices accompany the library.

All application and media policy stays in fvid: frame transforms and shaders,
YUV textures, LUT and color processing, presentation and encoder rendering,
CoreVideo texture cache and pixel-buffer lifetime, CUDA kernels, NVDEC and NVENC.
The existing fvid-vt UmaSharedBuffer API delegates only allocation/storage to
Forma while preserving its original result/error types. No other fvid API changes.

Check out fvid and forma as siblings; dependencies use relative Cargo paths.
The earlier broad video/CUDA extraction was withdrawn following the clarified
scope. Logs under docs/improvements/evidence/gpu-extraction-20261029 document
that earlier experiment, not the current architecture. Current qualification is
under docs/improvements/evidence/gpu-common-20261029.

This structural move is not counted toward the 1000 verified media fixes.
