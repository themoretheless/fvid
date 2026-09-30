# Programmable GPU filters

Y4M processing accepts `--shader FILE.wgsl` on Metal, Vulkan, Direct3D 12
and OpenGL/GLES. Each `--then` starts a new stage. Crop and reflections run
before that stage's shader; coordinates refer to its output. A chain uploads
and downloads each frame exactly once; intermediate frames stay on the GPU.
CUDA does not execute WGSL and explicitly rejects shader stages.

```sh
cargo build --release
fvid input.y4m output.y4m --backend metal --hflip \
  --shader shaders/negate.wgsl --then --shader shaders/boxblur.wgsl
```

Implement this function, without entry points or additional global variables:

```wgsl
fn process_byte(value: u32, plane: u32, x: u32, y: u32) -> u32 {
    if plane == 0u { return 255u - value; }
    return value;
}
```

Planes are Y=0, Cb=1, Cr=2. Chroma coordinates use the chroma plane's size,
including subsampling. Returned values saturate at 255. For spatial effects,
`sample(plane: u32, x: i32, y: i32) -> u32` reads a neighbour in the current
stage's transformed input, clamping at the plane edges. Reading neighbours
never sees writes from the same pass. A following pass sees the previous
pass's complete output. Padding is zeroed and excluded from Y4M payloads.

Rust callers use `resident::ByteShader::new`, `resident::GpuStage`,
`resident::GpuPipeline::with_stages` or `process_gpu_stages`. Existing
`GpuPipeline::new` and `process_gpu_chain` retain transform-only behavior.
Repeated stage programs reuse a compiled pipeline within the chain.

## Player display shader

```sh
cargo run --features player -- play input.mp4 --backend metal \
  --shader shaders/grayscale.wgsl
```

Implement `process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32>`.
The shader runs on 8..16-bit planar GPU video draws after colour management/LUT and
picture adjustment, before framebuffer sRGB encoding. RGB values are display
codes in 0..1, and the result is clamped to that range. `uv` is the source
texture coordinate, including the current crop. This display effect currently
does not alter snapshots, exports or CPU-rendered pictures.

The player accepts `--backend auto|metal|vulkan|dx12|gl` and `--device N`.
An explicit API cannot fall back to another API. Software adapters are excluded.
CUDA handles video processing/codec work and is not a window renderer.

## Validation

Source is limited to 64 KiB and parsed/validated with Naga before GPU setup.
Additional resources and entry points are rejected. wgpu's normal shader
validation and runtime bounds checks remain enabled. User programs can still
run indefinitely; source validation is not an execution-time sandbox.
Backend runtime/driver compilation errors are returned to the caller.

```sh
cargo test --test byte_shaders --test gpu_shader --test gpu_integration --test resident
FVID_SHADER_BACKEND=metal cargo test --test byte_shaders \
  hardware_shader_chain_matches_independent_cpu_reference -- --ignored
# On each target host: FVID_SHADER_BACKEND=vulkan|dx12|gl
```

Physical GPU tests compare a negate → 3x3 blur chain with an independent CPU
reference across 420/422/444, odd geometry, texture row boundaries and reused
frames. They also verify budget rejection and boundary-transfer counts.
These tests do not establish hardware codec surface interop or CUDA/RTX
runtime support on a machine without NVIDIA hardware.

## Hardware qualification and remaining integration

VideoToolbox session creation now sets
`kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder` and
queries `kVTDecompressionPropertyKey_UsingHardwareAcceleratedVideoDecoder`.
A session is accepted only when the property confirms hardware use. The native
reader can then explicitly fall back to FVid software decoding if hardware
initialization fails; it no longer labels a VideoToolbox software session as
hardware. The test `hardware_decode` requires actual hardware H.264 and HEVC
sessions and decodes their frames; it must run outside an environment that
hides the platform media engine.

The complete acceleration goal remains open. In particular:

- VideoToolbox frames still copy into host planar buffers before GPU upload.
- Native decode/export APIs and most built-in filters still use CPU processing.
- General resident shader pipelines do not yet import decoder surfaces or
  export encoder surfaces. CUDA `media hw-filter` has a dedicated NVDEC/NVENC
  path, limited to its built-in NV12 geometry filters.
- CPU RGB player frames do not yet take this display shader. Source-depth
  planar frames now retain 8..16-bit samples for GPU colour conversion, LUT and
  the display shader. HEVC Main10 hardware decode requests P010 output;
  decoder-surface transport still copies samples into host storage.
- This implementation does not add NVIDIA RTX Video enhancement SDK features.
- NVIDIA/CUDA/DX12 runtime verification requires an NVIDIA host; WGSL
  translation checks alone are not runtime verification.

## Source-precision playback

Packed Y/Cb/Cr planes keep their original 8..16-bit samples between the decoder
and the GPU renderer. 8-bit planes use R8Unorm textures; wider samples use
RG8Unorm with the low and high byte in separate channels. The fragment shader
reconstructs exact source codes with textureLoad before bilinear interpolation
in float32. This avoids independently rounded low/high-byte interpolation and
preserves low bits without requiring optional normalized 16-bit texture formats.
Imported P010 uses the same interpolation after recovering its ten-bit codes.
The matrix, full/limited range and table lookup execute on the GPU. Identity
picture settings avoid clamping source samples before range conversion.
Existing 8-bit and source-depth frames share the same renderer and tables;
changing depth rebuilds the plane textures and bindings.

HEVC Main10 VideoToolbox sessions request x420/xf20 P010 rather than an 8-bit
output format. P010 has ten bits in the MSBs and interleaved Cb/Cr; the adapter
currently unpacks it into source-depth planar host bytes. This preserves the
coded precision but the player still uses a CPU copy. The macOS adapter also
provides `Session::decode_surface` and, with its `metal` feature,
`Surface::import_metal`: NV12 and P010 images map directly into the same wgpu
Metal device. `Session::new_avc_surface` and `Session::new_hevc_surface` request
Metal-compatible bi-planar output. NV12 uses R8Unorm/Rg8Unorm and needs no
optional device feature; P010 uses R16Unorm/Rg16Unorm and requires
`TEXTURE_FORMAT_16BIT_NORM`. The legacy planar decode constructors remain
available for CPU consumers.
Both the CVMetalTexture and its pixel buffer stay retained through the HAL
resource lifetime, preventing decoder-pool reuse while GPU work is pending.
The strict hardware test maps all 17 Main10 frames, releases the decoder and
source Surface handles, and verifies every GPU-read sample against the retained
CPU reference. The NV12 test verifies all ten H.264 frames in both full and limited range on
a device with no optional features. `SurfaceVideoCallback` can draw these imported planes through the same
conversion, crop window, picture settings, LUT and custom display shader as
host-backed planes. Metal player devices enable normalized 16-bit formats when
available. `NativeReader::enable_shared_surfaces` now opts an existing hardware AVC/HEVC
reader into shared output before decoding begins. MP4 presentation reordering
retains surfaces and charges their actual plane strides to its memory budget;
RawFrame::Surface preserves depth, colour and ownership across the reader
boundary. The hardware test verifies source planes, presentation order,
timestamps, seek and retained-frame validity after reader destruction.
On Metal devices supporting normalized 16-bit textures, the player enables
shared surfaces before opening the first video frame. `Playback::start_from_frame`
preserves that first raw frame and subsequent frames, including seek targets.
The converter passes surfaces with identity or shader-compatible grades
straight to SurfaceVideoCallback, preserving the container rotation as metadata.
All four quarter-turn orientations execute in GPU sampling coordinates. P010
rotates integer texel coordinates before interpolation so the sampling order and
exact source codes agree with physically rotated reference planes. NV12 rotates
normalized texture coordinates. A conversion grid followed by a 1D look still uses an explicit host fallback
when it cannot be folded into per-channel byte tables. Snapshots explicitly download the held surface.
The queue sizes slots against the larger of the source-depth plane bound and the
first hardware surface's actual padded storage, and rejects an individual surface
larger than its payload budget.
Snapshots explicitly convert from the retained source-depth frame.

`source_precision_planes_use_gpu_conversion_and_display_shader` compares GPU
readback with the CPU conversion across 8/10/12/16 bits, full/limited ranges,
420/422/444/440, odd dimensions, depth changes, LUT and a custom display shader.
The high-depth inputs contain low bits that distinguish them from narrowed
8-bit images. `hardware_main10_keeps_source_precision` compares hardware
Main10 samples with the software decoder over the fixture's full frame sequence.

The presentation queue now reserves for three 16-bit planes (six bytes per
visible pixel) rather than assuming three RGB bytes. Under its 64 MiB payload
budget it may use one queue slot for large source-depth frames. This bound is
conservative for subsampled/8-bit sources; decoder working storage, the frame
on screen, GPU textures and driver allocations are separate.

`hardware_surfaces_render_with_colour_and_custom_shader` compares shared
NV12/P010 rendering to host-backed rendering over H.264, HEVC Main and Main10
fixtures, including all four container rotations, crop windows, picture settings, LUT, custom shaders and
switching storage layouts while retaining the same frame serial.

`Grade::shader_stages` exposes borrowed output-code tables or the original
conversion LUT followed by the optional authored look. Each stage keeps its
original domain and resolution; intermediate float values must not be clamped
before the next stage. This is the resource contract for extending the renderer
to chained grading. The renderer now binds a second 3D texture and its original input domain.
Conversion and the authored 3D look execute in order without intermediate
clamping or rounding. This keeps hardware surfaces on the GPU for chained 3D
grading. The sequential LUT test covers all interpolation modes, different sizes
and non-unit per-channel domains against the CPU reference. One-dimensional
looks after a mixing conversion grid remain a separate pending shader path. `is_shader_look` now checks eligibility
without copying the grid on every frame.
