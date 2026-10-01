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
The shader runs on RGB and 8..16-bit planar GPU video draws after colour management/LUT and
picture adjustment, before framebuffer sRGB encoding. RGB values are display
codes in 0..1, and the result is clamped to that range. `uv` is the source
texture coordinate, including the current crop. This display effect currently
does not alter snapshots or exports.

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

- Qualified Metal players import VideoToolbox NV12/P010 surfaces directly;
  CPU consumers explicitly download them.
- Native decode/export APIs and most built-in filters still use CPU processing.
- General resident shader pipelines do not yet import decoder surfaces or
  export encoder surfaces. CUDA `media hw-filter` has a dedicated NVDEC/NVENC
  path, limited to its built-in NV12 geometry filters.
- RGB player frames upload their display codes and take the same picture
  adjustment and display shader as source-depth planar frames. An immutable
  shared RGB frame uploads once; redraws reuse its texture, and crop or picture
  settings update only the uniform. HEVC Main10 hardware decode requests P010
  output and preserves its source precision.
- This implementation does not add NVIDIA RTX Video enhancement SDK features.
- NVIDIA/CUDA/DX12 runtime verification requires an NVIDIA host; WGSL
  translation checks alone are not runtime verification.

`cargo test --features player --test gpu_shader` translates the complete player
shader with grayscale and UV-dependent custom effects to Metal 1.2, HLSL 5.1,
SPIR-V 1.0, desktop GLSL 330 and GLES 300. This includes RGB, planar and native
surface sampling plus sequential 1D/3D grading. Resource bindings are explicit,
including HLSL sampler indirection; these checks do not exercise platform drivers.

## Source-precision playback

`fvid_vt::Encoder` requires and verifies VideoToolbox hardware compression for
H.264 or HEVC. Its synchronous `encode` accepts a retained CoreVideo surface,
without locking or copying pixels; only compressed bytes reach host storage.
Output is a length-prefixed access unit with owned SPS/PPS (AVC) or VPS/SPS/PPS
(HEVC), the NAL length-prefix size and the exact owned avcC/hvcC payload from
CoreMedia format extensions. The physical hardware test encodes
three NV12 frames per codec after destroying the originating decoder, then
creates a new hardware decoder from the exported parameter sets and decodes
all three access units, verifying geometry and depth. It also writes the units
and codec-private configuration through the existing Matroska PacketWriter and
decodes all frames using NativeReader. This is
an encoder foundation: export command wiring, video MP4 muxing, encoder quality
controls and Metal shader output surfaces remain pending.
Compression failure stops reuse of that session.
Encoded frames expose CoreMedia's sync flag for muxer indexing.
`encode_with_keyframe` can request a random-access frame at a segment boundary;
the hardware test decodes that requested frame using a fresh decoder, then
muxes each frame with its reported sync flag.
`new_with_depth` selects HEVC Main10 for ten-bit input and rejects unsupported
codec/depth combinations. Each submitted surface must match the configured
depth. The physical Main10 test verifies hvcC profile 2 and ten-bit luma, then
decodes three encoded units into P010 with unchanged geometry. This verifies
ten-bit transport and codec profile, not lossless reconstruction or quality.
`new_with_bitrate` sets VideoToolbox's target average rate before encoder
preparation and returns an error if the property is rejected. Rates must be
positive and fit a signed 32-bit value. The hardware round-trip test uses
1,000,000 bits/s for H.264 and HEVC; this verifies configuration and decodability,
not that a three-frame stream achieves that average rate or a quality target.
Encoded output includes rational PTS and duration read from CMSampleBuffer,
rejecting invalid, indefinite or non-positive durations. Tests use unequal
frame durations and preserve the resulting timestamps through Matroska muxing
and NativeReader decode; callers no longer need to infer output timing from
frame count or nominal FPS.
`EncodedTime::nanoseconds` converts signed rational timestamps with checked
overflow and rejects zero timescales. Sub-nanosecond fractions truncate toward
zero. Negative timestamps remain signed so a container caller can apply an
explicit timeline offset rather than accidentally wrapping into a huge value.

Headless decode/encode/export requires only the `videotoolbox` feature; it does
not depend on the player, wgpu or CUDA. Qualification command:
`cargo test --no-default-features --features videotoolbox --test hardware_decode -- --include-ignored`.
All seven tests available in this configuration pass on the physical Mac.

`hardware_export::write_video` joins native surfaces, the hardware encoder and
Matroska PacketWriter as a streaming video-track API. Inputs specify rational
PTS/duration and optional forced keyframes; codec and target bitrate are
explicit. It rejects changing codec configuration, non-increasing output PTS,
zero durations and unrepresentable timebases. Native full/limited range must
remain consistent: a range change stops the export
before that surface reaches the encoder. Hardware tests construct real NV12
surfaces in both ranges and verify this failure for H.264 and HEVC.
Compressed frames are written
one at a time. The caller owns destination publication and additional audio,
subtitle, colour, HDR and display metadata tracks. Physical tests round-trip
the API through H.264 and HEVC, including unequal frame durations.
`write_video_with_options` carries explicit output colour/HDR declarations,
crop, pixel aspect and rotation into the Matroska track. These describe the
encoded result and must be updated when grading or shader processing changes
the signal. The hardware export test verifies BT.709 declarations, a 2:1 coded
pixel aspect and 90-degree rotation after NativeReader reopening (the reader
reports the resulting 1:2 display-space pixel aspect).
Main10 export qualification also carries BT.2020/PQ declarations, mastering
primaries/white point/luminance and MaxCLL/MaxFALL through Matroska. NativeReader
reopening returns identical container HDR metadata and decodes three frames
at ten-bit source precision. This checks signalling transport; it does not
assert that arbitrary input pictures contain PQ-encoded HDR content.

`Surface::metal_black` allocates an IOSurface-backed NV12/P010 output and
initializes luma/chroma through Metal render attachments, without a host pixel
upload. Writable plane imports stay private until GPU completion; public
surface imports remain read-only. Physical qualification verifies limited/full
NV12 black codes and direct submission of the resulting surface to the hardware
H.264 encoder. P010 initialization now verifies exact ten-bit black/chroma
codes in both ranges and direct HEVC Main10 compression. P010 render output
requires TEXTURE_FORMAT_16BIT_NORM plus TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
missing features return an error before allocation.
`Surface::metal_render` records caller-provided render commands into the luma
and interleaved chroma planes, while keeping writable texture handles private.
The returned immutable surface waits for GPU completion. Physical tests write
non-black NV12/P010 codes with WGSL, verify exact plane values (including P010
low bits), and submit the surfaces directly to H.264 or HEVC Main10 encoders.
Export CLI wiring remains unfinished. Initialization clears and shader rendering share one submit
and one completion wait. A partial-draw test verifies exact black/chroma codes
outside the scissor region for both NV12 and P010.
`ColorShader::encoder_source` adds native luma/chroma fragment entries to the
display pipeline. The caller supplies matrix coefficients, output depth and
range and disables framebuffer sRGB conversion. The renderer supplies output
dimensions in the 160-byte Params uniform. Chroma averages four display-RGB
samples at output pixel centers before 4:2:0 conversion;
P010 codes are rounded and MSB-aligned. WGSL validation covers both depths and
ranges. Physical render comparison and connection to the export pipeline are
covered by `MetalEncoderRenderer` for RGB or native
surface input. It shares grading resources and converts display codes into
NV12/P010 render targets with framebuffer sRGB conversion disabled. Physical
tests compare all three planes against an independent RGB-to-YUV calculation
for 8/10-bit full/limited range, compare RGB and native surface shader paths,
and submit the native output directly to H.264/HEVC Main10 encoders. CLI export
wiring remains pending. `render_rgb_sized` and `render_surface_sized` accept
explicit output dimensions, reject zero/device-limit violations and resample
on the GPU. Native surface output supports
0/90/180/270-degree rotation, swapping output dimensions for quarter turns.
Chroma qualification now also
covers a spatially varying nonlinear UV effect on 8×6 and 7×5 frames across
both depths/ranges. Each output chroma texel samples an aligned 2×2 source
block, replicating the last row/column at odd edges. All planes agree with an
independent CPU calculation within one code. A separate physical rotation test
compares output geometry and all planes with an upright RGB reference for both
depths and even/odd sizes, using a spatial UV effect. It also exercises switching
from RGB to native input with the same frame serial. This test uses a constant
native source, so it does not independently qualify varying source orientation.
A scaling test compares the spatial effect with an output-sized reference for
up/down scaling, even/odd output dimensions, all rotations and both depths/ranges.
This qualifies the output sample grid and chroma alignment. An additional
varying RGB8 source test independently computes bilinear source sampling,
YUV conversion and 2×2 chroma averaging for up/down scaling, even/odd sizes
and both depths/ranges. All output codes agree within one code on physical
Metal. RGB sampling uses four textureLoad reads and float32 interpolation:
hardware normalized-byte sampler filtering exceeded that tolerance for P010.
This verifies bilinear correctness, not anti-aliasing quality for downscaling;
varying native-source scaling still needs independent qualification.

`hardware_export::write_shader_video` connects a lazy stream of `ShaderFrame`
native surfaces to `MetalEncoderRenderer`, VideoToolbox and the Matroska muxer.
Each frame supplies source colour, optional grade, rotation, timing and forced
keyframe state. The caller selects output dimensions and output track metadata;
the function renders and encodes one frame at a time without downloading pixel
planes. A physical end-to-end test retains a decoded MP4 surface after closing
the decoder, applies a custom shader and 90-degree rotation into 32×48 output,
encodes H.264/NV12 and HEVC/Main10, and reopens both Matroska streams. It verifies
three decoded frames, dimensions and variable PTS. Existing renderer/encoder
tests separately qualify pixel codes and forced keyframes. `write_shader_file`
and `write_video_file` stage output beside the destination, sync the completed
file, then publish with a no-overwrite hard link. Failure removes the staged
file; a destination appearing during encoding is preserved. Publication tests
cover partial-write failure, competing destination creation and success. A
filesystem without hard-link support returns an error without rename fallback.
The macOS player-enabled binary exposes this route as:

```sh
cargo run --features player -- shader-export input.mp4 output.mkv \
  --video-only --shader shaders/grayscale.wgsl --codec hevc --depth 10 \
  --size 1920x1080 --bitrate 8000000 --device 0
```

`shader-export --help` lists options. This CLI currently accepts native
VideoToolbox input, uncompressed 8-bit Y4M input and shaders producing SDR
Rec.709 display codes. Y4M planes are uploaded once; subsequent grading, LUT,
shader, scaling and native output remain on GPU until compressed packets reach
the muxer. Compressed inputs still require native VideoToolbox surfaces.
`MetalEncoderRenderer` also exposes planar8/packed input rendering APIs. It grades
the source signal (including HDR10/PQ and HLG) into that output before running
the shader. `--sdr-nits` sets target peak (100 by default), and `--tonemap`
selects linear/gamma/clip/reinhard/hable/mobius; omitting it uses Grade's
automatic HDR shoulder selection. The grade is cached until source signal/HDR
metadata changes. It requires `--video-only` while copying audio/subtitle
tracks remains pending. The stream API accepts explicit output signal metadata.
Input rotation is
applied to pixels, not repeated in output metadata. Output publication refuses
existing files. `--lut FILE` loads the existing .cube/.3dl/.dat/.spi1d/.spi3d
formats; `--log CURVE` and `--gamut NAME` override source interpretation.
`--grid 2..128` and `--interp nearest|trilinear|tetrahedral` control the conversion
grid. The order is source conversion → authored LUT → shader → encoder YUV.
`--display pq|hlg --hdr-nits N` selects HDR BT.2020 output (1000 nits by
default), requiring `--codec hevc --depth 10`. The GPU conversion writes the
selected transfer before LUT/shader, and encoder YUV uses BT.2020 coefficients.
Matroska records primaries 9, transfer 16/18 and matrix 9. Shaders must preserve
the selected signal. Output mastering display and MaxCLL/FALL default to unspecified:
source values are not valid evidence for transformed output. Encoder colour
properties and attachments on a separate CVPixelBuffer over the same IOSurface
set bitstream VUI without mutating the input Surface or copying pixels. HEVC
automatic HDR metadata insertion is disabled before session preparation; no
unmeasured mastering/content-light or vendor metadata is requested. Explicit
output HDR SEI is supported by the stream API: explicitly supplied output
MDCV/CLLI is serialized as HEVC prefix SEI on keyframes, with emulation
prevention and the encoder's NAL length prefix. Content light levels must be
integer nits within 0..65535. Physical Main10 export tests read HDR from the
bitstream independently of Matroska, comparing the exact quantized MDCV
payload and CLLI levels. CLI `--max-cll N --max-fall N` sets explicit result
content-light values; `--mastering-display Rx,Ry,Gx,Gy,Bx,By,Wx,Wy,max_nits,min_nits`
sets its mastering volume. These require HDR output; integer content levels
must fit 16 bits and MaxFALL must not exceed a known MaxCLL. Input metadata is
never implicitly copied through the shader. Physical CLI tests compare MDCV
payload/CLLI in container and bitstream and reject inconsistent light values
before publication. Additional source formats remain pending.
The physical CLI integration test executes H.264/8-bit and HEVC/10-bit exports,
uses SDR, HDR10 and HLG source fixtures, decodes every output frame at the
requested 32×48 size, checks SDR Rec.709 metadata, verifies grayscale
channels within four codes after lossy encoding, and repeats each command to
verify the existing file is preserved byte-for-byte with no staged leftovers.
It runs both without a LUT and with a constant 1D LUT, asserting the latter's
output remains within four 8-bit codes of 0.25 after lossy encoding.
Y4M cases additionally verify three frames and their exact 25-fps PTS with
H.264/HEVC, both with and without LUT.
`--crop X:Y:WIDTH:HEIGHT` selects an upright pixel rectangle after container
rotation and before `--size` scaling. With no size override, output dimensions
match the rectangle. The GPU renderer uses a validated normalized window for
luma and every chroma subsample; malformed/out-of-bounds crops fail before
publication. A physical RGB crop test compares all encoder planes with an
independently cropped CPU source for odd/even rectangles and both depths/ranges
within one code. It covers original crop dimensions, enlargement and reduction.
Encoder RGB sampling clamps interpolation to crop edge pixel centers; this
prevents surrounding pixels leaking into enlarged output. The change is scoped
to encoder rendering; player sampling retains its existing behavior. Native
encoder luma/chroma sampling likewise clamps upright UVs to the crop's edge
centers before rotation. A physical test downloads varying NV12/P010 sources,
rotates and crops their planes independently, then compares enlarged encoder
output with the native surface route within one code for all four rotations.
That test uses even-aligned 4:2:0 crops; odd native crop/chroma phase still needs
independent qualification.
Separate physical CLI cases export an HDR10 source to PQ and HLG with an identity
shader, decode every frame as Main10 and verify BT.2020 and selected transfer
metadata. They also read HEVC bitstream colour independently of container
metadata and assert primaries 9, transfer 16/18 and matrix 9. They do not
independently measure HDR luminance or qualify mastering/content-light SEI.

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
normalized texture coordinates. Conversion grids followed by 1D or 3D looks stay
on the GPU. Snapshots explicitly download the held surface.
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
looks retain their float nodes and authored domains in a tiled texture, with
linear interpolation matching the CPU. Tests cover one-node tables, row and
layer boundaries, and the parser maximum of 300,000 nodes; GPU output differs
from the CPU reference by at most one output code. `is_shader_look` checks eligibility
without copying the grid on every frame.

The current renderer regression run (`cargo test --features player,cuda --lib
player_gpu::tests -- --include-ignored`) passes all 30 tests on physical Metal,
including native encoder surfaces, source precision, geometry, chained grading
and muxed additional tracks. The eight `gpu_shader` translation tests also pass.
Translation tests qualify shader generation only; they do not establish driver
execution on Vulkan, DirectX, OpenGL or NVIDIA devices.
The physical `metal_player_presents_shared_main10_and_can_snapshot_it` test also
passes: hardware Main10 decoding reaches the player's presentation state as a
10-bit `Pixels::Surface`, with no CPU video/packed texture populated, and its
snapshot produces PNG data. It exercises the player integration with an egui
context; it does not constitute inspection of a visible OS window.
An additional visible macOS run with `--backend metal --shader
shaders/grayscale.wgsl --no-audio` was inspected through the native UI: the
Main10 fixture rendered a grayscale frame and displayed VideoToolbox status.
The short clip retained its final frame after playback. Paused startup now
transfers the pending one-frame step through background opening. A fresh
visible `--start-paused` run displayed grayscale frame 0 (00:00:00.000) without
starting playback. The physical integration test uses background opening,
checks PTS 0 and verifies that repeated presentation polls retain that frame
while paused.

CUDA C point shaders are exposed separately as `fvid_cuda::ByteShader` and
`CudaPipeline::with_shaders`. Supply one optional shader per planar transform.
The native CUDA media filter selects NV12 or P010 from probed 4:2:0 video.
It checks the hardware context's component format before filtering or NVENC
passthrough, and rejects mismatched layouts rather than interpreting P010
as NV12. Eight-bit output uses H.264 NVENC; ten-bit output uses HEVC NVENC
with the Main10 profile. Colour range, primaries, transfer, matrix and sample
aspect ratio are carried into the encoder configuration. HDR mastering and
content-light metadata preservation is not yet qualified for this CUDA path.
The CUDA library now separately exposes `P010View` and `P010Processor` for
native ten-bit crop, reflections and optional component shaders. Pitches are
bytes; samples passed to `process_byte` and `sample` are ten-bit codes. Shader
results keep their low ten bits and are stored in bits 15..6, with low bits
cleared. The media CLI uses this path for ten-bit crop, reflections and shader
processing with no full-frame host copies. P010 `--host-bounce` is rejected.
The real Main10 fixture is probed in a host test and selects P010/HEVC NVENC;
the complete decode/filter/encode test remains ignored pending NVIDIA hardware.
Admission checks word alignment, byte footprints, overflow and plane overlap.
The generated sampling kernel passes an independent C++ host reference for
all four reflection combinations, crop, neighborhood blur, both chroma
components, padding and extreme signed sampler coordinates. This checks
algorithmic behavior, not NVRTC compilation or NVIDIA execution. Linux and
Windows compile checks include the ignored physical P010 test; that test
still requires an NVIDIA GPU and CUDA NVRTC to execute.
The trusted source defines `__device__ unsigned int process_byte(unsigned int
value, unsigned int plane, unsigned int x, unsigned int y)`; coordinates refer
to the output plane before crop/reflection sampling. The result is narrowed to
its low eight bits. `shaders/negate.cu` is an example. NVRTC compiles the wrapper
once when constructing the pipeline; each shader executes in its existing
crop/reflection pass, retaining one upload and one download for the whole chain.
Identical generated CUDA sources share a single compiled module within that
pipeline, even when their stages have different geometry. Cache ownership ends
with the pipeline; there is no unbounded process-global custom-shader cache.
This CUDA C API does not execute WGSL.
`GpuStage::cuda_shader` connects it to the resident pipeline. The Y4M CLI accepts
`--backend cuda --cuda-shader shaders/negate.cu`; combine stages with `--then`.
Each stage accepts one WGSL or CUDA C shader, and incompatible backend/language
selections are rejected before device initialization. The ignored NVIDIA test
`resident_cuda_shaders_fuse_geometry_and_keep_one_host_roundtrip` compares two
shader stages against CPU values and checks transfer counters. Its actual GPU
execution still requires NVIDIA hardware and CUDA 13 NVRTC.

`Nv12Processor::with_shader` compiles the point-shader signature into the pitched
NV12 transform kernel. It filters luma and separate U/V components as planes
0/1/2, with coordinates in each component's output plane. A shader forces kernel
execution even when crop/flip is identity; it cannot be bypassed by the crop-only
device copy optimization. Existing external-stream ordering and module lifetime
ownership remain in effect. This API works directly on `Nv12View` device pointers
without host pixel staging. Sampling shaders also work on native NV12: each
sampler reads its Y/U/V component using the correct pitched address and chroma
subsampling, clamping coordinates before crop/reflections.
NV12 admission rejects overlapping source/destination plane address footprints,
including padding between visible rows, before launching a kernel or device
copy. Callers still own allocation validity and distinct virtual-address aliases;
the API cannot inspect actual allocation lengths from a device pointer.
The NVDEC/NVENC media CLI exposes this constructor with
`fvid media hw-filter INPUT.mp4 OUTPUT.mp4 --cuda-shader shaders/negate.cu`
in a `media-cuda` build. It accepts a trusted point shader of at most 64 KiB.
Use `--cuda-sampling-shader shaders/boxblur.cu` for neighborhood reads. Shader runs disable
passthrough, direct crop copying and the parallel-session shortcut,
retaining a single native NVDEC→CUDA kernel→NVENC pipeline. The media adapter
requests FFmpeg's primary CUDA context for every shader run and kernel-based
crop/vflip run, matching `fvid-cuda`'s device context. Shader source admission
occurs before CUDA device creation. JSON export statistics identify the filter
as `cuda-point-shader` or `cuda-sampling-shader`, alongside existing host-copy
and device-pass counters. The media adapter
uses its existing libavformat/libavcodec integration. The `media-cuda` CLI build
passes on macOS, but actual NVIDIA execution has not been qualified.
Full-frame vertical reflection now uses a CUDA filter for both NV12 and P010;
the former implicit NV12 download/CPU reflection/upload path was removed.
Explicit NV12 `--host-bounce` performs one real download and upload per frame
before the device filter and counts both transfers. Host routing tests pass;
the end-to-end test asserting zero ordinary host copies and two diagnostic
copies per frame remains ignored until NVIDIA execution is available. No
throughput comparison is claimed for this changed reflection path.
The native NV12 sampler and generated blur kernel pass a host C++ reference
check over pitched planes, crop, all reflections, separate U/V reads and extreme
signed coordinates. This checks address/algorithm behavior, not CUDA execution.
`native_nv12_sampling_shader_preserves_pitches_and_matches_cpu` is the matching
ignored NVIDIA test: it allocates real device buffers with row padding, executes
the NVRTC shader through `Nv12Processor`, and compares both complete output
allocations against CPU results across all reflection combinations. Padding must
remain zero and chroma must remain unchanged. The test compiles for Linux and
Windows; it has not been executed on NVIDIA in this environment.

```sh
cargo test --manifest-path crates/fvid-cuda/Cargo.toml native_nv12_sampling_shader_preserves_pitches_and_matches_cpu -- --ignored
```

For neighborhood filters, construct `ByteShader::with_sampling` or use
`--cuda-sampling-shader shaders/boxblur.cu`. Its `process_byte` receives a fifth
`FvidSampler sampler` argument; `sample(sampler, x, y)` reads the transformed
input plane. Signed 64-bit coordinates clamp at the current output plane edges
before applying crop and reflections. Each pass reads its input buffer and writes
a distinct output, so neighborhood reads never see partially filtered pixels.
The CUDA blur uses the same integer division as `boxblur.wgsl`.
The actual C++ sampler and generated blur kernel have passed a host reference
check covering crop, all reflection combinations and signed-coordinate extremes;
NVRTC compilation and NVIDIA execution remain unverified for this sampling path.
The ignored test `sampling_shader_matches_independent_crop_and_reflection_reference`
executes NVRTC and the real resident pipeline on NVIDIA, compares all four
reflection combinations to a CPU crop/blur reference, preserves chroma, and
asserts one upload/download pair. Run it on Linux or Windows with:

```sh
cargo test --manifest-path crates/fvid-cuda/Cargo.toml sampling_shader_matches_independent_crop_and_reflection_reference -- --ignored
```

`hardware_export::write_video_with_tracks` merges pre-encoded additional tracks
with hardware video in presentation-time order. `AdditionalPacket::track` indexes
the additional track list; video occupies track zero in the resulting Matroska.
The caller supplies a globally sorted packet iterator and maps source edits,
codec delay, padding and timestamps onto the output timeline. Payloads, durations
and packet options are forwarded, and packets after the last video frame are
retained. The mux holds one additional packet ahead. Export statistics count
encoded video frames and video bytes only.

`write_video_file_with_tracks` uses the same temporary-file publication as the
video-only API: failed exports leave no destination, and an existing destination
is never overwritten. The physical Metal/VideoToolbox test
`hardware_video_mux_interleaves_subtitles_and_preserves_tail` verifies ASS payloads,
timestamps, durations, packet ordering and a subtitle beyond the video end.
It also copies a real AAC packet from an MP4 fixture and verifies its payload,
AudioSpecificConfig, timestamp, duration and discard padding after demuxing the
hardware export. Full audio playback and source edit-list mapping through this
API are not yet qualified by this packet-level test.

`write_video_file_with_mp4_audio` copies all AAC tracks from eligible MP4 inputs
using the existing native edit/gapless planner, including CodecDelay and final
discard padding. It merges packets with a heap while reading compressed payloads
one at a time. Timing plans and source sample indices remain in memory. Source
video frames must already use the same edited presentation timeline. Inputs with
unrepresented tracks, non-AAC audio or complex edits are rejected. The physical
`hardware_export_copies_mp4_audio_with_gapless_timing` test checks every packet in
two-track AAC and single-edit fixtures against the native AAC remux result.
It also decodes every copied track through the owned MP4 and Matroska PCM paths,
both in full and over a 10–20 ms interval. Output sample counts and float PCM
bytes match exactly, including edit-list delay and final padding. This qualifies
the owned PCM export path; real-time player scheduling is a separate check.

The `shader-export` CLI accepts exactly one of `--video-only` or `--copy-audio`.
The latter opens a separate MP4 demuxer and copies all eligible AAC tracks while
Metal shaders and VideoToolbox process video. Automatic subtitle copying remains
pending. Real-time audio playback through this new export path remains unqualified.
In `--copy-audio` mode, video frames are intersected with the single source video
edit and rebased to presentation time before encoding. Frames outside the edit
are decoded as needed for references but are not rendered or exported; crossing
frames have their durations clipped. This matches the AAC planner's edited
timeline. `--video-only` retains its existing source media-timeline behavior.
