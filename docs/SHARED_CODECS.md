# Shared owned compressed-video kernels

`fvid-codecs` compiles the canonical AVC, HEVC, VP9 and AV1 source modules once. The root crate reexports these modules; `fvid-media::owned_codecs` exposes the same public types. Shared error and fallible allocation primitives live in `fvid-control`. Canonical source files remain in this monorepo; this change does not qualify standalone package publication. The player feature retains HEVC worker priority through `fvid-platform`.

Plain WebM VP9/AV1 requests now dispatch through these kernels in both native-only and legacy-enabled fvid-media builds. AV1 configuration OBUs initialize the decoder before packets, including files whose first packet has no sequence header. Malformed packets propagate errors; explicitly unsupported tools remain capability refusals. WebM transforms now use the shared owned streaming pipeline; unsupported container workflows remain explicit capability refusals.

This is a migration step, not complete FFmpeg independence. MP4 interior-empty-edit, rotation and multitrack workflows, remaining exports and codec/profile gaps still require migration. Root CUDA features still enable legacy-ffmpeg; ordinary media now uses the owned backend. Presence in the capability inventory denotes linked packet-decoder components, not acceptance of every container workflow or profile.

Verification: shared-kernel unit tests, root and standalone media unit tests, native deband/perspective regressions, eleven shared-codec integration tests and the offline native dependency guard. Ordinary tests require neither FFmpeg nor network access.

## MP4 dispatch

Plain MP4 AVC/HEVC without rotation now uses the shared packet decoders in fvid-media. AVC decode-order output includes B pictures; HEVC counts only pictures marked for output and reports cropped dimensions and source depth. No FFmpeg API is called on this admitted path. Interior empty edits remain explicit refusals in native-only builds, and retain their existing legacy route in legacy-enabled builds. Single-video-track transforms now use the owned presentation bridge described below. This preserves the scope boundary without silently discarding presentation timing or requested filters.

Four short synthetic MP4 fixtures exercise AVC baseline/B pictures and HEVC Main/Main10. The pure Python generator replaces complete edit containers with same-size free boxes, preserving every packet offset. Tests compare native root and standalone frame count, dimensions and pixel format; edit-list acceptance tests replace the old refusal, while interior empty ranges remain explicitly unadmitted.

## Shared MP4 presentation mapping

`owned_video_timeline` owns the mapping used by both the root player and standalone media decoder. Rate-one media edits use cumulative ceil rounding in track ticks, preserving fractional endpoints without accumulating per-edit rounding drift. Leading empty edits retain the root player's existing start-at-first-picture policy. An empty edit inside playback remains an explicit unsupported capability until a blank-frame policy is implemented.

The stats decoder preserves codec reference reconstruction by decoding access units in decode order, counting a visible picture once for every overlapping media edit. Repeated and disjoint ranges therefore preserve their displayed frame count, with partial frames included at half-open boundaries. It reconstructs the source once rather than retaining the decoded pixel sequence or decoding each repeated range again. The plain stats path retains only presentation metadata. The separate presentation visitor spools pixels only for filtering/export; neither path qualifies universal profile support.

Equal-PTS pictures remain codec references but yield one displayed picture, using the latest decoded picture metadata. Presentation intervals follow sorted PTS; the terminal duplicate group retains the sum of its nominal durations. The stats decoder keeps only presentation metadata, not the decoded frame sequence, and visits edits in movie order. Existing synthetic duplicate-PTS fixtures reproduce the old incorrect count (12 instead of 11); updated acceptance tests cover single duplicates, duplicate runs and a group spanning the entire clip.

## WebM streaming transforms and export

VP9 and AV1 packets now feed the existing owned filter, interval, step, reverse and shuffle pipeline. The same visitor writes atomic Matroska/FFV1 exports without libav. Visible dimensions are cropped from padded codec strides, including odd high-depth VP9 pictures. Hidden reference packets reconstruct dependencies but do not establish the visible interval origin. Multiple visible pictures per packet still require distinct timestamps and are rejected.

AV1 configuration initialization and rewind/reset share one kernel constructor, retaining configuration-only sequence headers and static HDR metadata. Export preserves decoded range/matrix, declared primaries/transfer and merged static HDR metadata; fixed format/colour/HDR is required for the admitted streaming path. FFV1 inputs retain their existing container metadata policy. Synthetic acceptance compares every output sample with native source samples, independent negate arithmetic and expected step/reverse/shuffle frame order. HDR tests establish metadata transport, not HDR image quality.

Remaining boundaries include compressed multitrack exports, isolated untimed pictures or nonincreasing untimed display timestamps, crop/rotation, RGB or studio-range monochrome streaming and unsupported codec tools. These changes do not remove the remaining production legacy feature edges.

## Missing WebM durations

Owned compressed exports now infer missing durations from the next actually shown picture, not the next coded block. A separate decoder pass retains only presentation indices/PTS, including reconstruction of hidden references. Explicit positive BlockDuration/DefaultDuration wins; the terminal picture uses declared Segment end when available, otherwise repeats the preceding visible interval. An isolated picture without timing and nonpositive inferred intervals remain unadmitted. This terminal cadence policy is an estimate when the container omits its end, not recovery of unknowable original timing. Decoder cancellation and input packet limits apply during the inference pass too.

Synthetic VP9 and AV1 derivatives remove DefaultDuration and BlockDuration using equal-sized Void elements. Acceptance checks source pixels, every displayed PTS and every emitted duration; the AV1 source includes hidden and show-existing pictures. Ordinary tests use no external backend.

## MP4 filtering and lossless export

AVC/HEVC MP4 with a single video track now feeds the established owned FFV1 filter/export pipeline. Shared packet decoding writes cropped source-precision planes to a private fixed-record spool. Presentation metadata selects the last equal-PTS picture, reorders B pictures and maps/clips repeated, disjoint, fractional and leading-empty edits before replay. A private Matroska/FFV1 intermediate carries pixel aspect, file tags/chapters, track title/language, VUI colour and merged static HDR. Undefined MP4 language is normalized to `und`, preserving its meaning instead of rejecting the Matroska description.

The bridge is transitional: it incurs temporary disk storage and an extra own FFV1 encode/decode pass, rather than retaining all decoded frames in RAM. It is not a 60 fps performance qualification. RAII removes both temporary stores; only the established final exporter publishes user output atomically. Codec/presentation corruption remains an owned error even in legacy-enabled dispatch. Explicit codec or changing-format capability refusals remain distinguishable.

All-pixel and exact timeline acceptance covers eleven committed synthetic AVC/HEVC clips, including Main10, B pictures, duplicate PTS, repeated/disjoint/fractional/leading edits. Independent negate arithmetic and step/reverse order verify actual transformation. A corrupt-NAL derivative verifies the specific invalid length error and absence of output in native-only and `media` builds. Audio/other tracks, refused tracks, rotation, interior empty edits and unsupported codec tools remain unadmitted by this bridge; no tracks are silently dropped. The production legacy feature edges are still present.

## Video-only decode selection in MP4 with audio

Transformed video decode now selects the first supported video track even when an MP4 also carries audio, matching the video-only API's contract and its plain-decode path. The internal presentation bridge has a separate video-only admission mode. Export additionally admits one AVC/HEVC video track with AAC companion tracks as described below.

A Python-only fixture combines the existing synthetic AVC baseline with the complete packet timeline of the native AAC edit fixture. Tests prove both track types and nonempty AAC samples, video pixel equivalence with the single-track control, native transformed-decode acceptance and audio-preserving multitrack export acceptance.

## Owned MP4 filtering with AAC companions

For an MP4 containing exactly one AVC/HEVC video and one or more AAC tracks, the owned exporter filters video through FFV1 and copies every AAC packet without re-encoding. Original track order is preserved, including audio-first input. The canonical owned MP4 audio planner supplies timestamps, durations, codec delay and discard padding. Global and scoped metadata and stream metadata edits survive the final owned Matroska mux. Only final publication emits completion; packet limits and cancellation leave no partial destination.

Synthetic acceptance covers negate, reverse and frame step, two AAC tracks, exact compressed audio bytes and timing, every video pixel, metadata, and identical decoded PCM after filtering. This admission permits an optional interval and explicit stream selection containing the video track, but no complex AAC edit list. Multiple video tracks, other companion codecs and broader codec tools remain outside this MP4 scope. WebM companions have separate acceptance below. The CUDA feature graphs still include `legacy-ffmpeg`; these container acceptance checks do not establish complete FFmpeg independence or 60 fps performance.

Selected MP4 intervals now remain on the owned multitrack exporter. AAC bounds use ceil-rounded sample positions relative to the existing edit origin. The packet prefix is retained to reconstruct overlap history, CodecDelay removes all samples before the selected origin, and terminal DiscardPadding removes samples beyond the endpoint. Video keeps the established frame-start selection contract and rebases PTS; it does not split pictures at fractional frame boundaries. Synthetic acceptance compares exact PCM against direct interval decode, independently checks sample counts, and verifies selected video pixels and timestamps for one/two AAC tracks and a nonzero AAC edit origin. Retaining the full AAC prefix is correct but does not optimize long-range seek.

MP4 companion export also supports explicit original stream indices in the requested output order, including video-only selection from an audio-first source. Only selected AAC tracks are planned and copied. Scoped tags are remapped to the resulting track UIDs, while metadata mutations retain original source-index semantics. The source packet budget counts selected tracks; intermediate video filtering normalizes its own stream index to zero. Duplicate/out-of-range selections and a selection without video fail in the owned path before publication. Synthetic selection acceptance checks reversed order, omitted audio, sole video, exact AAC bytes and decoded PCM, all video pixels, scoped metadata and a packet limit equal to the selected input count. Interval/priming acceptance additionally permutes the audio and video tracks, comparing PCM via the resulting output index.

## Owned WebM/Matroska video filtering with AAC/Opus

One FFV1/VP9/AV1 video plus selected AAC/Opus companions now uses the existing owned frame pipeline and a final owned Matroska packet mux. Video-only selection, audio-first track order and reordered stream indices are supported. Companion codec setup, packet bytes, PTS, explicit durations, CodecDelay and signed DiscardPadding survive unchanged. Scoped tags follow original source indices to new output UIDs; global metadata edits use the existing video exporter. Private staging is shared with MP4 and emits no public completion. Cancellation and packet caps preserve atomic publication.

Three Python-only fixtures join committed synthetic VP9/AV1 with AAC and Opus packets. Acceptance checks negate/reverse/step pixels, all/reordered/video-only tracks, metadata, exact companion packet tuples, AAC presentation PCM and raw Opus PCM decoded by `fvid-opus`. Opus transport presentation is checked separately through identical clocks, delay and padding. These companion-copy tests use the own Opus packet decoder directly. The standalone file-export adapter is described below.

This admission currently excludes intervals, multiple video tracks, unrepresented metadata, nondefault dispositions, conflicting legacy/IETF languages, negative block PTS and backwards audio packet clocks, other audio codecs, video crop/rotation and unsupported pixel/codec tools. Remaining workflows and production legacy feature edges still require removal. The private filtered-video stage incurs disk I/O; no 60 fps claim is made.

Missing companion durations no longer force fallback: AAC uses the declared frame-sample count/rate, and Opus validates transport framing and infers its duration from TOC. An additional synthetic file omits both audio BlockDuration and DefaultDuration while preserving delay/padding. Acceptance compares decoded PCM with the source and verifies that only the missing duration metadata is reconstructed.


## Owned Matroska Opus audio export

`decode_audio` now routes selected Matroska/WebM Opus through `fvid-opus`, the existing presentation timeline and WAVE DSP/export pipeline. CodecDelay, signed DiscardPadding, intervals, gain, channel conversion and resampling are covered by native acceptance tests. Planning and container loudness use the same adapter. The player shares its PCM decoding, output gain, channel permutation and reset implementation; its SILK, hybrid, stereo and surround regressions pass.

Opus Matroska output always declares the 48 kHz codec clock. OpusHead's informational input rate may be 44.1 kHz or zero and is preserved without changing playback timing. Two synthetic video derivatives cover both values and compare round-trip presentation PCM. Cancellation, packet caps and completion preserve atomic file publication.

Aggregate controlled-allocation admission for Opus audio remains unsupported. Noncanonical mappings, Ogg file export and broader legacy production workflows are not covered by this adapter; production features still have legacy FFmpeg edges.

Standalone Opus WAVE export is also accepted against committed PCM references for SILK, hybrid, mono/stereo CELT, 5.1 family-1 and a positive first timestamp. Each fixture produces exactly 48000 presentation frames with maximum absolute sample error below 0.00002; tests execute no reference encoder or decoder process.


## Owned compressed audio mix and merge

The standalone mix/merge entrypoints now decode inputs through the owned audio file-export pipeline, retaining the direct float32 WAVE fast path. Other admitted formats are decoded into a private RAII-cleaned WAVE spool before the existing PCM mix/channel merge. Legacy dispatch adopts this path for inputs admitted by the owned exporter; unsupported inputs still retain the legacy fallback.

Planning shares the audio exporter's decoder configuration descriptor and reads container/codec headers without decoding PCM or creating a temporary spool. Packet payload validity remains an execution check. Output publication and geometry checks remain those of the existing mix/merge implementation. Native Opus acceptance compares both mixed and channel-merged PCM exactly against predecoded WAVE inputs. Existing weighted/shortest WAVE regression also passes. This closes the prior float32-WAVE-only input restriction for admitted audio, but does not remove all legacy production features.


## Production media feature

Root `media` enables the owned media backend and HTTP input without enabling `legacy-ffmpeg`. The dependency guard now checks this production graph on every normal run. Root `mcp` inherits that backend through `media`. Unsupported owned codecs, packet tools or workflows return their explicit errors instead of invoking libav in this graph; completing those gaps remains migration work, not proof of universal compatibility. `media-cuda` and library `cuda-hw` still enable the legacy adapter and remain outside the completed removal.


## Direct CUDA codec device ownership

`fvid-cuda::CodecDevice` now obtains the existing native CUDA primary-context pool and owns a separate stream for direct codec integrations. It binds the context before exposing borrowed handles and supports explicit stream synchronization. It creates no AVHWDeviceContext and requires no libav headers. Host refusal/ordinal tests pass and the Linux implementation cross-checks successfully. The NVIDIA device test is explicit and ignored without hardware; no live NVDEC/NVENC claim follows from these checks. The legacy hardware filter still needs direct codec session implementations and wiring before CUDA feature dependencies can be removed.


## NVENC driver preflight

`fvid-cuda::NvencApi` directly loads the fixed NVIDIA driver library name on Linux/Windows, retains its lifetime, and queries `NvEncodeAPIGetMaxSupportedVersion` with the SDK calling convention. Driver errors, zero versions and incompatible requested versions are explicit refusals. Tests cover those checks without requiring the driver; Linux/Windows cross-checks compile the actual loader ABI. This is not an encoder session: the compatibility function-table/session owner is implemented below, while encoder initialization, registration and bitstream output still need implementation and NVIDIA hardware acceptance.

ABI/reference: [NVIDIA NVENC programming guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/nvenc-video-encoder-api-prog-guide/index.html), and the [official NVIDIA loader example](https://github.com/NVIDIA/video-sdk-samples/blob/master/Samples/NvCodec/NvEncoder/NvEncoder.cpp). No FFmpeg headers or library are used by this preflight.


## Direct NVENC CUDA session ownership

`NvencSession` creates the NVIDIA function table, opens a CUDA session and owns encoder/device/library lifetimes. Explicit close exposes driver errors, retains a failed handle for retry and is idempotent after success. Drop attempts closure; if the driver refuses, CUDA/library resources are intentionally retained until process exit rather than unloaded under a live encoder.

The implemented compatibility ABI is SDK 8.1, from NVIDIA's MIT-licensed interface header at commit `aa3544dcea2fe63122e4feb83bf805ea40e58dbe`. It predates AV1 and is not qualification of modern GPU encoding. Function-table/session sizes and offsets were matched against C compilation of that pinned header: table 2552 bytes (destroy/open offsets 224/240), open parameters 1552 bytes (API/reserved-pointer offsets 24/1040). Four host tests pass; Linux/Windows implementation and tests cross-check. The actual session test remains explicitly hardware-only/ignored. Frame encoding, modern SDK tools, NVDEC and the production CUDA pipeline remain unfinished.

Direct NVENC sessions now expose the driver codec GUID list through typed SDK entrypoints. Count/query errors and oversized returned counts refuse explicitly; zero counts avoid a second call and unknown GUIDs remain intact. A synthetic driver-response test covers these cases without hardware; the explicit NVIDIA session test queries this list before close and verifies refusal after close. This is capability negotiation, not initialized frame encoding.


## NVENC encoder initialization

The direct session can now initialize synchronous H.264 from the NVIDIA default preset, validating even geometry and a nonzero frame-rate ratio. It refuses repeated initialization and marks a failed mutating initialize call as requiring session closure. Config/init structures are checked-in bindgen 0.72.1 output from the pinned NVIDIA SDK 8.1 interface, with the NVIDIA MIT notice retained and system calling conventions. Normal compilation requires neither SDK headers nor bindgen/libclang. C verification gave config/preset/init sizes 3584/5128/1808, preset config offset 8 and init config-pointer offset 88; host tests enforce those values. Linux/Windows cross-checks compile the actual driver calls. The explicit hardware test initializes 128×72 at 60/1 and rejects a second initialization; live NVIDIA verification is left to the user. HEVC/configuration options, modern SDK support, frame registration/submission/output, NVDEC and CUDA production wiring remain unfinished.


## NVENC output slot ownership

Initialized direct sessions now allocate driver bitstream slots and track them before exposing their indices. Every slot is destroyed before encoder closure; a partially failed cleanup keeps remaining handles live for retry and never destroys the encoder early. Ownership storage is reserved before driver allocation, preventing a post-allocation host bookkeeping failure. SDK output-buffer size/handle offset and function-table offsets were C-verified as 776/16 and 120/128. Seven host tests and Linux/Windows test cross-checks pass; the explicit NVIDIA test creates two outputs and closes them. No frame has yet been submitted or read back through this path; CUDA surface registration, submission, output locking and production wiring remain unfinished.


## Direct CUDA NV12 registration

Initialized NVENC sessions now register and map caller-owned CUDA NV12 allocations, checking pitch, pointer range and the full Y+UV extent. The raw pointer API is explicitly unsafe: the caller must retain and synchronize the allocation until successful session closure, including cleanup retries. Session records retain registrations even if mapping fails. Cleanup unmaps before unregistering, then releases output buffers and encoder; failures retain handles for retry. SDK register/map sizes and function-table offsets were C-verified as 1536/1544 and 248/256/208/216. Nine host tests and Linux/Windows test cross-checks pass. The explicit NVIDIA test now allocates a synthetic CUDA surface from the same primary-context pool, registers/maps it and closes the session before freeing the allocation. It has not been executed on hardware here. NVDEC and production CUDA wiring remain unfinished.


## Direct NVENC frame submission

The direct session accepts mapped NV12 inputs and output slots, distinguishes
buffered frames from unaccepted busy submissions, and drains packets in submission
order. EOS makes pending outputs eligible; close waits through blocking output
locks before releasing CUDA inputs. Failed unlocks retain their handles for retry.
Packets copy driver bytes into owned Rust storage before unlocking and reject
hardware errors or empty output.

The ignored NVIDIA test now submits four synthetic CUDA frames, drains EOS and
checks packet timestamps and nonempty payloads. It does not yet prove decoder
acceptance or production filter integration. Hardware execution remains deferred
to the user's NVIDIA machine. Host tests and Linux/Windows compilation do not
establish GPU performance or SDK compatibility on a particular driver.


Direct eight-bit NV12 initialization now selects H.264 or HEVC with the
respective SDK codec GUID and the driver's default preset. HEVC has its own
ignored NVIDIA submission/drain test using the same synthetic surfaces and
resource lifecycle. No HEVC hardware execution is claimed; P010/Main10 and
native NVDEC remain separate unfinished work.


## Direct NVDEC capability negotiation

`fvid-cuda::NvdecApi` loads the fixed NVIDIA driver library and queries
`cuvidGetDecoderCaps` with the selected `CodecDevice` context bound to the
calling thread. The typed request covers H.264, HEVC, VP8 and VP9, chroma
formats and 8/10/12-bit inputs. Driver errors and inconsistent supported
limits refuse explicitly; unsupported capabilities remain ordinary results.
Geometry admission checks minimum/maximum coded dimensions and macroblock
capacity with overflow-safe arithmetic before future surface allocation.

The layout comes from the pinned NVIDIA
[cuviddec.h](https://github.com/NVIDIA/video-sdk-samples/blob/aa3544dcea2fe63122e4feb83bf805ea40e58dbe/Samples/NvCodec/NvDecoder/cuviddec.h):
C verification gives size/alignment 88/4 and support/max-width/min-width/
reserved-tail offsets 24/28/40/44. Four host tests cover the ABI, initialized
requests, error handling and geometry admission. Linux/Windows test compilation
passes. An ignored NVIDIA test queries actual H.264 device capabilities without
libav. It has not been executed here. This creates no decoder, processes no
compressed packets and does not yet replace production NVDEC.


## Direct NVDEC decoder ownership

`NvdecSession` creates a driver decoder after querying the selected device's
4:2:0 codec/depth/geometry limits. Its request explicitly carries coded size,
parser-provided reference-surface count and mapped-output capacity. NV12 is
selected for eight-bit input and P016 for higher bit depth. Reference count must
come from the sequence parser; this allocation API does not infer it from a file.

The hand-written SDK layout uses platform C `unsigned long`, not a fixed Rust
integer: pinned-header C probes give 176-byte Linux and 112-byte Windows x64
creation storage, with display/output-format/context-lock/reserved-tail offsets
80/88/120/136 and 44/52/72/88 respectively. Rust regression checks cover both
layouts. Driver function pointers use CUDAAPI's system calling convention.

Closure binds the owning context, synchronizes its stream and destroys the
decoder. Failure retains the handle for retry; a failed final Drop intentionally
retains its context and driver library rather than unloading live resources.
Host tests exercise request admission and destruction retries. An ignored NVIDIA
test creates and closes an actual H.264 decoder without libav; it has not run here.
Compressed-packet parsing, picture submission, surface mapping and production
CUDA integration remain unfinished.


## Direct NVDEC mapped outputs

Sessions now resolve the 64-bit CUDA-pointer map/unmap entrypoints directly.
The unsafe progressive-picture mapping API requires an already submitted picture
whose decode slot remains reserved. It returns a borrowed device-pointer/pitch
description, not a retained allocation. All consumers must finish before unmap
or session closure. Native packet parsing and picture submission are still pending,
so real mapped-frame acceptance has not been exercised yet.

Mapping bookkeeping is bounded by simultaneous output capacity; released entries
are removed and monotonically increasing slot identifiers prevent a stale release
from freeing a later mapping. Invalid pointers, pitches, high-bit-depth alignment
and overflowing extents refuse. Returned mappings remain owned even when mapping
reports an error. Cleanup retains failed unmaps for retry and unmaps all tracked
surfaces before destroying the decoder.

The pinned-header C probes verify 264-byte x64 processing storage and stream/
reserved-pointer offsets 56/248 on both Linux and Windows. Host regression tests
cover output extent refusal, unmap retry and ABI. No GPU mapping execution or
production CUDA/libav replacement is claimed by these tests.


## Direct NVDEC picture submission

The session now resolves `cuvidDecodePicture` and submits typed picture
parameters without libav. A checked-in generated SDK data module includes
H.264/HEVC/VP8/VP9 codec-specific layouts; the developer generator is separate
from ordinary builds and emits no linked functions.

The unsafe boundary requires readable bitstream/slice-offset arrays and correct
codec/reference parameters from a parser. Admission checks progressive coded
geometry, picture-slot bounds, nonempty byte/offset storage and increasing slice
offsets. A mapped picture cannot be overwritten. Only successfully submitted
indices may be mapped; failed submissions invalidate their previous mapping
eligibility. These checks do not validate the codec-specific union or references.

Pinned-header C probes verify 4280-byte x64 picture storage and byte-pointer/
slice-pointer/codec-union offsets 32/48/184 on Linux and Windows. Regeneration is
byte-identical. Host tests cover malformed submission admission and slice ordering;
Linux/Windows test compilation passes. No compressed-stream hardware acceptance
has been executed: parser callbacks, reference-slot scheduling and end-to-end
production CUDA integration remain necessary before removing the legacy backend.


## FVid AVC syntax to NVDEC

The FFmpeg-free `fvid-media/native-cuda` feature now exposes
`owned_nvdec_avc::AvcPicture`. It converts FVid-parsed SPS/PPS/access-unit slices
and scaling matrices into the SDK picture description, owns Annex-B slice bytes
and offsets, and initializes unused DPB entries to invalid slots. Caller-supplied
reference descriptors carry decode slots, frame/long-term indices and field POC.
Submission still requires correct live reference ownership in the matching session.

This first adapter admits progressive eight-bit 4:2:0 with one slice group;
interlaced, FMO and higher-depth AVC require further translation. The existing
synthetic control MP4 exercises the owned MP4/config/slice parsers and parameter
assembly in an ordinary host test without NVIDIA or external tools. A separate
ignored NVIDIA test submits its first IDR, maps output and releases it; it is not
pixel-equivalence proof and has not executed here.

The ordinary dependency guard includes the new native CUDA adapter graph.
Legacy `cuda-hw` still activates libav while production filter integration,
reference scheduling and remaining codec adapters are unfinished.
