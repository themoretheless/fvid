# Fvid CUDA adapter

Real CUDA compute for the current fused planar-byte crop/horizontal-reflection/vertical-reflection operation. This is an optional backend, not an NVENC/NVDEC implementation or a codec library. Kernels ship as multi-arch PTX under [`ptx/`](ptx/); NVRTC is only a fallback. It does not shell out to FFmpeg or substitute CPU execution.

## Runtime requirements

- Linux or Windows with an NVIDIA GPU and a compatible NVIDIA driver.
- **CUDA driver** (`nvcuda.dll` / `libcuda.so.1`) for device execution.
- Optional **CUDA 13 NVRTC** for fallback compile when no embedded PTX matches the GPU. Regenerate PTX with:
  ```sh
  # Windows: . ../../scripts/use_cuda_windows.ps1
  cargo run --example emit_ptx --release
  ```
- On Windows: if you need NVRTC/tools, ensure
  `C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.*\bin\x64` is on `PATH` (`nvrtc64_130_0.dll`). The driver alone is enough for `--list-devices` and for running precompiled PTX.
- MSVC Build Tools are required to build the Rust crate on Windows; no `nvcc` is required at Rust build time.
- macOS builds the same public API without the CUDA dependency and returns a clear unsupported-platform error. Use the Metal backend there.

No CUDA SDK linker or `nvcc` is required at Rust build time. Dependencies are target-scoped and loaded at runtime. Missing driver libraries return errors. As with cudarc itself, malformed vendor installations with missing entry points can still panic; this does not isolate driver faults in another process.

## Context pool and transfers

A **process-wide device pool** caches one `CudaContext` and loaded modules per ordinal. Creating another `CudaProcessor` / `Nv12Processor` / `CudaPipeline` in the same process reuses them. **One-shot CLI** (`fvid.exe` per invocation) cannot keep a context across processes — that would need a long-lived worker (e.g. MCP). Warm path = stay in-process.

Each processor allocates **two** device I/O + pinned staging slots and runs a depth-2 pipeline (`submit`/`flush`): while stream B processes frame N+1, stream A can complete frame N. Single-frame `apply` still synchronizes immediately. Resident `upload`/`download` use slot 0.

## Ownership and synchronization

`CudaPipeline` adds a preallocated resident chain: `upload` once, `process` launches every stage on one CUDA stream with predecessor buffers passed directly to the next kernel, and `download` explicitly returns the final bytes. No intermediate host transfers or device-to-device copy commands are issued between stages. The checked phase machine rejects processing before upload and download before processing. All stage allocations and boundary buffers are admitted to a fixed payload budget. Root Fvid exposes borrowed frame tokens and `--then`; see [resident contract](../../docs/GPU_RESIDENT.md).

After a GPU operation error the processor is poisoned and must be recreated (the shared context/modules stay valid).

The parameter ABI is documented on `CudaProcessor::new`. Validation rejects invalid flags, reserved parameters, gaps/overlaps in output, escaped input planes, division by zero and overflow before any driver calls. Unsafe operations: fixed-name vendor library probes, pinned host alloc/free, and the documented kernel launch.

## Verification

```sh
cargo test --manifest-path crates/fvid-cuda/Cargo.toml
cargo check --manifest-path crates/fvid-cuda/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check --manifest-path crates/fvid-cuda/Cargo.toml --target x86_64-pc-windows-msvc
```

On an NVIDIA machine, put CUDA 13 `bin` on `PATH`, then explicitly execute the smoke test (failure to initialize CUDA is a test failure, never a silent success):

```sh
cargo test --manifest-path crates/fvid-cuda/Cargo.toml -- --ignored --nocapture
```

The hardware tests cover crop, all reflection combinations, short-buffer rejection, reuse with changed inputs, padded input strides, all three nontrivial planes, unaligned plane boundaries and output spanning multiple workgroups. A macOS host cannot verify this CUDA kernel's runtime correctness or performance. Cross-target Rust checks prove compilation only.

Implementation references: [cudarc driver API](https://docs.rs/cudarc/0.19.9/cudarc/driver/index.html), [NVRTC compilation](https://docs.rs/cudarc/0.19.9/cudarc/nvrtc/safe/fn.compile_ptx_with_opts.html), [CUDA kernel execution](https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__EXEC.html).
