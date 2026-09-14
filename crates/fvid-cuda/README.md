# Fvid CUDA adapter

Real CUDA compute for the current fused planar-byte crop/horizontal-reflection/vertical-reflection operation. This is an optional backend, not an NVENC/NVDEC implementation or a codec library. It uses a CUDA C kernel compiled by NVRTC and launched through cudarc; it does not shell out to FFmpeg or substitute CPU execution.

## Runtime requirements

- Linux or Windows with an NVIDIA GPU and a compatible NVIDIA driver.
- CUDA NVRTC runtime, including its vendor dependencies, available to the system dynamic library loader. The bindings target CUDA 12.0; the compiler must support the selected GPU's actual compute capability. Driver/compiler incompatibility is returned as an error.
- macOS builds the same public API without the CUDA dependency and returns a clear unsupported-platform error. Use the Metal backend there.

No CUDA SDK, CUDA linker or `nvcc` is required at Rust build time. Dependencies are target-scoped and loaded at runtime. Device enumeration requires the driver only; constructing a processor additionally requires NVRTC. Missing driver/compiler libraries return errors. As with cudarc itself, malformed vendor installations with missing entry points can still panic; this does not isolate driver faults in another process.

## Ownership and synchronization

`CudaPipeline` adds a preallocated resident chain: `upload` once, `process` launches every stage on one CUDA stream with predecessor buffers passed directly to the next kernel, and `download` explicitly returns the final bytes. No intermediate host transfers or device-to-device copy commands are issued. The checked phase machine rejects processing before upload and download before processing. All stage allocations and boundary buffers are admitted to a fixed payload budget. Root Fvid exposes borrowed frame tokens and `--then`; see [resident contract](../../docs/GPU_RESIDENT.md).

One processor allocates input/output/parameter GPU buffers once and retains one nonblocking CUDA stream. `apply` requires exact buffer lengths, copies CPU input to its device buffer, dispatches one kernel, copies the result back, and explicitly synchronizes before returning. Host transfer time belongs in end-to-end benchmarks. The pipeline does not yet offer pinned host buffers, overlapping frames, external GPU textures, or zero-copy Metal/DirectX/CUDA interop. After a GPU operation error the processor is poisoned and must be recreated.

The parameter ABI is documented on `CudaProcessor::new`. Validation rejects invalid flags, reserved parameters, gaps/overlaps in output, escaped input planes, division by zero and overflow before any driver calls. The only unsafe operations in this crate are fixed-name dynamic vendor library probes and the documented kernel launch.

## Verification

```sh
cargo test --manifest-path crates/fvid-cuda/Cargo.toml
cargo check --manifest-path crates/fvid-cuda/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check --manifest-path crates/fvid-cuda/Cargo.toml --target x86_64-pc-windows-msvc
```

On an NVIDIA machine, explicitly execute the smoke test (failure to initialize CUDA is a test failure, never a silent success):

```sh
cargo test --manifest-path crates/fvid-cuda/Cargo.toml -- --ignored --nocapture
```

The hardware tests cover crop, all reflection combinations, short-buffer rejection, reuse with changed inputs, padded input strides, all three nontrivial planes, unaligned plane boundaries and output spanning multiple workgroups. A macOS host cannot verify this CUDA kernel's runtime correctness or performance. Cross-target Rust checks prove compilation only.

Implementation references: [cudarc driver API](https://docs.rs/cudarc/0.19.9/cudarc/driver/index.html), [NVRTC compilation](https://docs.rs/cudarc/0.19.9/cudarc/nvrtc/safe/fn.compile_ptx_with_opts.html), [CUDA kernel execution](https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__EXEC.html).
