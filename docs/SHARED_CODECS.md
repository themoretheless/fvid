# Shared owned compressed-video kernels

`fvid-codecs` compiles the canonical AVC, HEVC, VP9 and AV1 source modules once. The root crate reexports these modules; `fvid-media::owned_codecs` exposes the same public types. Shared error and fallible allocation primitives live in `fvid-control`. Canonical source files remain in this monorepo; this change does not qualify standalone package publication. The player feature retains HEVC worker priority through `fvid-platform`.

Plain WebM VP9/AV1 requests now dispatch through these kernels in both native-only and legacy-enabled fvid-media builds. AV1 configuration OBUs initialize the decoder before packets, including files whose first packet has no sequence header. Malformed packets propagate errors; explicitly unsupported tools remain capability refusals. WebM transforms now use the shared owned streaming pipeline; unsupported container workflows remain explicit capability refusals.

The library and production feature graphs no longer depend on FFmpeg/libav, including CUDA and the historical reference marker. Native functional coverage is still incomplete: remaining exports and codec/profile/tool gaps require implementation and acceptance. Presence in the capability inventory denotes linked packet-decoder components, not acceptance of every container workflow or profile. Earlier migration sections below record historical state; the current production and retired-backend sections describe the active API.

Verification: shared-kernel unit tests, root and standalone media unit tests, native deband/perspective regressions, eleven shared-codec integration tests and the offline native dependency guard. Ordinary tests require neither FFmpeg nor network access.

## Owned MP3 player route (2026-10-16)

The production player `A_MPEG/L3` factory uses `src/codec/mp3_decoder.rs` and
the owned `crates/fvid-codecs/src/mp3.rs` reconstruction path. MPEG reference
decoder features are dev-dependencies only. Acceptance covers raw/Matroska PCM,
all nine sample rates, mono/MS, protected frames, free-format spacing through
640 kbit/s, exact sample seek and rewind. Owned Xing/Info plus trusted revision-zero
LAME metadata now supplies gapless head/tail trimming across all nine rates.
Metadata frame reservoir bytes survive factory resets and checkpoint restores.
Invalid or untrusted trim metadata preserves full audio. This does not establish
every MP3 profile combination or all library export workflows.

Visited raw/Matroska positions can restore owned reservoir, overlap/synthesis
and sample-clock checkpoints. Capture tests verify unchanged PCM and fewer
decoded packets; cold seeks still rebuild history. Backwards PTS retain full
replay because timestamp-only restoration could lose an earlier occurrence.
Independent PCM, compressed fixtures and checkpoint evidence are recorded in
[NATIVE_PLAYBACK.md](NATIVE_PLAYBACK.md).

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

Aggregate controlled-allocation admission for Opus audio remains unsupported. Noncanonical mappings and Ogg file export are not covered by this adapter. The public decoder inventory now reports the linked owned Opus component; this does not imply support for every mapping or container. Production features no longer select the retired FFmpeg backend.

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


## Owned AVC NVDEC reference scheduling

`AvcNvdecDecoder` combines the owned slice adapter with FVid's existing POC and
DPB marking logic. It computes staged state before submission and commits only
after the driver accepts the picture. Malformed input leaves the previous POC/
DPB unchanged. A driver error requires reopening because its internal state is
no longer assumed recoverable.

Reference entries and retained decoded-picture tickets hold decode slots.
Mappings additionally pin their picture until successful unmap. Slots become
reusable only after all owners release them; exhaustion applies backpressure.
Foreign tickets refuse and duplicate mappings refuse before calling the driver.
Caller code remains responsible for display order, packet timestamps and
synchronizing unsafe CUDA-pointer consumers before release.

Ordinary tests parse every packet of existing synthetic control and multislice
I/P/B MP4 fixtures, validate reference scheduling and ensure retained display
tickets prevent slot reuse even after DPB eviction. Another test drops staged
work and checks that malformed input does not advance committed state. An ignored
NVIDIA test decodes/maps/releases the complete I/P/B fixture without libav; it has
not executed here and checks mapping metadata rather than pixel equivalence.
The first scheduler keeps the adapter's progressive eight-bit 4:2:0/single-group
scope and explicitly refuses frame-number gaps requiring inferred references.
Production CUDA filter wiring and other codec adapters remain unfinished.


## Owned NV12 allocation and direct codec/filter chain

`Nv12Buffer` allocates pitched CUDA Y/UV storage through FVid's shared primary
context, with checked dimensions, plane extents and device addresses. It exposes
a borrowed raw view and an output stream for ordering filter work. The allocation
must remain live until external codec registrations and unsafe pointer users
are finished; borrowing its view does not transfer allocation ownership.

The existing CUDA qualification runner now additionally requires three native
AVC tests, including a synthetic first-IDR NVDEC -> GPU reflection -> NVENC chain.
The test orders filter completion through the owned output stream, unmaps the
decoder surface, registers the retained filtered allocation, drains encoder EOS
and closes the encoder before freeing its input. It checks packet existence and
timestamp/duration, not pixel-equivalence or 60 fps. This test is compiled on
Linux/Windows but has not run on a physical NVIDIA device here.

The chain remains an explicit ignored hardware qualification test; production
`cuda-hw` still uses the legacy backend pending actual filter/container wiring
and complete format coverage.


## Owned AVC encoder-to-container framing

`owned_avc_annexb::convert` converts three/four-byte Annex B start codes to
four-byte NAL lengths, derives validated `avcC` from bounded SPS/PPS sets,
preserves in-band parameter NALs and identifies IDR packets. Missing/invalid NAL
framing, incomplete parameter sets, conflicting parameter IDs and byte-limit
overflow refuse explicitly. SPS/PPS validation uses FVid's owned syntax parsers.

An ordinary synthetic control-MP4 test repackages its first access unit as
encoder-style Annex B, converts it, writes/reads the own Matroska container and
compares decoded pixels with the original access unit through FVid's AVC decoder.
No FFmpeg, network or GPU is required. The ignored direct codec/filter-chain test
now also muxes actual NVENC output, reopens it and decodes it with FVid, checking
the output dimensions. The latter remains unexecuted on NVIDIA here and does
not yet establish pixel-equivalence of the filtered/reencoded result.

Production file filtering still needs container/timestamp integration beyond
this framing bridge; the current native implementation has not replaced every
legacy CUDA workflow.


## Owned MP4 input for direct AVC NVDEC

`owned_nvdec_mp4::AvcMp4Input` opens the owned MP4 reader, selects its single AVC
video track and parses its active SPS/PPS pair before creating a native decoder.
It reuses caller packet storage, feeds packets in sample-table decode order and
returns retained GPU tickets alongside original sample index, PTS, DTS, duration,
sync flag and media timescale. Failed decoding makes the source terminal rather
than skipping a packet and continuing with incomplete references.

The input exposes a stable raw-media presentation-order index and preserves
movie timescale and edit lists separately. It does not silently reinterpret raw
media PTS as movie time or normalize variable durations to CFR. Repeated ranges,
empty edits and timeline filtering still need the higher-level presentation loop.

Ordinary tests read every packet of the existing synthetic multislice I/P/B MP4,
compare payload and timing with the demux index, decode it with FVid software,
check backwards decode-order PTS and ordered presentation indices. A repeated-edit
fixture verifies edit preservation. Both tests require no NVIDIA or FFmpeg.
An ignored NVIDIA test connects this source to the native decoder and maps each
decoded output; qualification now requires this test too. It has not run here.
Production file filtering and remaining format coverage are still unfinished.


## Native MP4 movie presentation plan

The shared video timeline now exposes an export-clock edit mapper retaining
leading and interior empty spans while reusing cumulative timescale rounding.
The established playback mapper keeps its existing trim-leading-empty policy
and explicit interior-empty refusal; changing export planning does not claim
that the player renders empty spans yet.

Native MP4 input builds bounded presentation occurrences from this movie map:
B pictures are visited in PTS order, overlapping frames are clipped to media
ranges, repeated ranges produce repeated source-frame appearances, and explicit
empty edits become blank events. Coordinates remain in media-timescale ticks
on the movie clock. A future rendering/export loop must realize these events.

The new 18 KiB synthetic `edit-empty-spans.mov` fixture comes from the public
control seed, with two empty spans surrounding repeated media. Native plan
acceptance checks the two blanks, duplicated source frame, half-second endpoint
and event-count refusal. Four shared-clock tests and three native-input tests
pass without hardware or external tools. Hardware rendering/export remains
unexecuted and production CUDA migration is incomplete.


## Device black frames for empty movie spans

Owned NV12 outputs can now fill black directly with stream-ordered CUDA memset:
limited range uses Y=16, full range Y=0, and both use neutral interleaved UV=128.
Both complete pitched planes, including padding, are covered. The operation
synchronizes before returning and performs no host pixel allocation/upload.

An ignored NVIDIA test downloads the synthetic result only for qualification,
checking every Y/UV/padding byte in both ranges. The existing qualification
runner now requires this test. Three host layout/admission tests and Linux/
Windows test compilation pass; the physical fill test remains unexecuted here.

This supplies the black-frame primitive needed by the existing synthetic
empty-edit timeline fixture. The movie event rendering/export loop still needs
to connect it, and production CUDA still has the legacy backend.


## Native AVC movie reader

`AvcMovieReader` now drives own MP4 packet input, timeline occurrences and direct
NVDEC. Its bounded queue accepts decode-order samples while delivering movie
events in presentation order. It caches only pictures needed before the next
replay, discards unnecessary preroll display tickets, and restarts both packet
iteration and decoder at source edit boundaries and repeated occurrences rather than retaining an entire
movie on GPU. Returned picture tickets remain owned while callers filter them.
All mapped/raw pointer consumers must finish before advancing across a replay.

The staged queue refuses unexpected decode indices and insufficient B-picture
cache before requesting another decode. Decoder/input errors make the reader
terminal. Blank events return no decoded ticket and can be rendered using the
owned GPU black-fill primitive.

Host tests cover reordered B output, repeated-range restart, long preroll without
retaining future-repeat frames, blanks and cache admission. An ignored NVIDIA
test runs the existing empty-edit synthetic MOV through the renderer, flips
pictures into its owned output and fills both blank events on GPU; qualification requires it. This
hardware test has not run here. Production encode/mux publication and remaining
codec profiles/formats still require integration.

`AvcMovieRenderer` owns the reader, output allocation and filter stream. It validates
an even crop before allocation, renders each picture to NV12, waits before
unmapping decoder input, and fills empty spans using the selected black range.
Callers must finish using its borrowed output before advancing. Any render error
makes it terminal; cleanup retains owners if GPU completion cannot be established.
This adds the reusable render stage; production encoding/export integration is
still incomplete and `cuda-hw` still enables the legacy backend.

## Native movie encode and Matroska stage

`AvcMovieEncoder` connects the owned AVC movie renderer to direct NVENC using a
bounded pool of 1..64 registered GPU input/output slots. A slot stays reserved
until its accepted output drains, including delayed encoder submissions. An
extra device-to-device copy separates the renderer's reusable output from
in-flight encoder inputs. This does not upload/download decoded pixel planes.
The default driver preset remains active; the caller supplies its nominal rate,
pool capacity and positive wait timeout. A pool smaller than driver delay can
time out and makes the encoder terminal, without reusing pending allocations.
EOS drains accepted output and cleanup closes the encoder before releasing its
registered buffers; failed cleanup retains GPU owners.

`write_avc_matroska` uses the own Annex B converter and packet writer. It carries
track-clock timestamps/durations into nanoseconds, checks stable AVC setup and
writes explicit black-span events as video pictures. It currently exports video
only; audio and production
CLI wiring still need integration. HEVC packets can be pulled through the encoder
but this Matroska helper currently requires AVC.

Host tests check bounded FIFO input ownership and timestamp arithmetic. The
ignored `synthetic_movie_encodes_and_muxes_without_libav` qualification test uses
the public empty-edit MOV, verifies every output timestamp/duration, and decodes
all muxed AVC pictures using FVid. It has only been cross-compiled here; NVIDIA
execution remains unverified. `cuda-hw` still enables the legacy production path.

Native CUDA movie export now carries source track name/language, rotation,
coded pixel aspect, SPS display crop and colour/HDR tags into the owned Matroska
writer. Container colour takes precedence over AVC VUI; VUI supplies otherwise
unstated colour/aspect. Crop is intersected with the filter window and its edges
are swapped by flips. A window with no visible image is refused before CUDA
allocation. The black-fill range must agree with stated source pixels; this
path does not silently convert range. Unknown colour uses unspecified H.273
codes with the selected range. NVENC receives matching colour VUI in H.264 and
HEVC configuration. HDR tags are preserved; no tone mapping is implied.

The synthetic `avc-cuda-video-metadata.mp4` regression proves metadata reading
and owned-container preservation. GPU acceptance now includes this fixture,
but only host tests and cross compilation have run here. Audio handling and production CLI integration remain unfinished.

`export_avc_matroska` now reuses the owned atomic publisher: it refuses existing
output, syncs a temporary file, closes NVENC before publication, and emits `done`
only after the final path is created. Packet progress is emitted while writing;
cancellation is checked between render/encode iterations and inside driver waits.
File tags and chapters are copied from the own MP4 reader to the Matroska writer.
The metadata fixture now carries an explicit synthetic title and chapter, whose
round trip is verified without CUDA. NVIDIA acceptance exercises actual file
publication, completed progress, and existing-output preservation, but has not
run here. This API remains video-only; production `hw-filter` still uses legacy
libav while its remaining codec/audio/options are being ported.

The native reader now accepts a half-open track-tick movie interval, clips each
frame/blank occurrence, rebases timestamps and trims/rebases chapters. Reference
preroll still decodes through the original packet stream. Empty or invalid
intervals are refused; variable durations remain intact. Legacy `hw-filter`
currently rebases selected frames to CFR, so CLI behavior is not yet switched.

`AvcMovieRenderer::new_with_shader` supports trusted point and sampling shaders
through the same CUDA processor. Empty spans get an owned source-sized black
surface and pass through the shader/transform into the reusable output; shaders
therefore apply consistently to pictures and blanks. GPU acceptance now includes
a uniform-white shader and checks every software-decoded output plane, including
both originally blank spans. NVIDIA execution remains unverified.

## Production CUDA routing

`media-cuda` and `cuda-hw` now enable owned CUDA code without `legacy-ffmpeg`.
The default dependency guard audits both graphs, including in CI. Earlier
migration notes in this document describe the legacy dependency before this
feature change; they are not the current production graph. The historical `legacy-ffmpeg` flag is now an empty compatibility marker for
reference benchmarks; it cannot switch the public API or library linkage. No unsupported production input is automatically retried with libav.

The existing `hw-filter` entrypoint uses the owned CUDA route for
AVC or qualified Main/Main10 HEVC MP4 input and `.mkv` output. CPU qualification parses every access unit using
the same owned POC/DPB/parameter adapter before GPU selection. Accepted files run
through owned MP4 input, movie reader, filter/shader, retained NVENC slots and
atomic Matroska publication. No libav retry occurs after native execution starts.
The standalone `native-cuda` feature exports the same operation contracts and
`hw_filter` API without enabling any legacy dependency.

The native route supports explicit host bounce through owned source-sized
NV12/P010 staging before filtering, without libav. Main10 uses retained P010 encoder pools/copies, Main10 NVENC registration and
submission, and owned HEVC Annex B/hvcC/Matroska export. Its hardware acceptance
test remains ignored until run on NVIDIA; host tests qualify source syntax,
clock and bit-exact Annex B conversion. Other codecs/containers return an
explicit unsupported-route error until owned support is implemented. This
feature change removes the build dependency; it does not establish complete
native codec/container/option equivalence. The owned operation exports video
only while retaining metadata and exact movie
occurrences. Its nominal rate is reduced from source sample count/duration.

The shared movie renderer selects a typed `Yuv420Buffer`/`Yuv420Processor` pair:
NV12 for eight-bit and P010 for ten-bit, retaining byte pitches and code depth.
Owned P010 black fill includes padding, with limited Y=64 and neutral UV=512
stored MSB-aligned in 16-bit words. P010 processors now expose the same explicit
launch synchronization boundary as NV12 before NVDEC unmap or cleanup. The
Main10 repeated-edit fixture is accepted by own software/clock tests; physical
CUDA black fill and movie/shader rendering remain ignored hardware tests.

`MovieSource` dispatches owned AVC/HEVC inputs into a shared `MovieReader` and
NV12 `MovieRenderer`; the original AVC names remain aliases. HEVC movie
qualification filters suppressed RASL/non-output pictures from presentation
events, while those packets still establish references during decoder preroll.
Field sequences, in-band parameter changes and mid-stream prior-output
suppression remain unqualified. The synthetic HEVC repeated-edit fixture has
two blank spans and two occurrences of the same six B-ordered pictures.

Host admission tests run without a GPU/FFmpeg; Linux/Windows compile the ignored
production-route qualification test. It asserts the owned backend, zero host
pixel transfers, every muxed packet and FVid decoding. Hardware execution remains
unverified. Existing CUDA option/stat types are shared between both backends.

Native CUDA statistics count successful filter/copy calls directly. An unshaded
black fill does not increment `device_filter_passes`; a blank passed through a
shader does. This is an operation counter, not measured GPU throughput.

CLI interval routing now selects whole movie events whose starts lie in `[from,
to)`, then rebases their timestamps. Unlike overlap clipping, a frame beginning
before `from` is excluded and a selected frame keeps its full duration even when
it extends past `to`. Boundaries in microseconds must map exactly to track ticks,
as the legacy interface requires. The general native reader retains its separate
`Clip` policy for overlap-based editing.

Native intervals now use the container movie clock directly, preserving edit
blanks and repeated ranges. They no longer normalize against the earliest
demuxed stream or reject the movie simply because it has blank spans or other
tracks. Video-only export keeps its existing track scope; audio is not remuxed. The duplicate-PTS synthetic fixture checks whole
frame selection, repeated timestamps and preserved durations. GPU production
acceptance now requests a half-second cut from the one-second control fixture
and verifies six output packets using FVid; execution remains unverified.

Native production AVC routing now accepts SPS display crop. CLI crop coordinates
refer to the visible image: the source SPS left/top offsets are added before the
CUDA transform, and right/bottom padding is excluded from its default output.
The renderer therefore removes source padding rather than encoding it as visible
content. A further crop is bounded against visible dimensions, and metadata crop
is cleared when the output window lies wholly inside the visible source.

`avc-display-crop.mp4` is a 62x46 synthetic I/P/B sequence with coded 64x48 and
right/bottom crop 2. Own software decoding and production admission accept all
8 pictures; host tests reject a crop that includes padding. NVIDIA qualification
adds default visible-size export and nested crop with both flips, then checks
mux dimensions, absent container crop and every FVid-decoded output picture.
These GPU checks have only been cross-compiled here.

## Production CUDA decode-only routing

`decode --device` now admits qualified AVC and HEVC Main/Main10 MP4 inputs to
owned MP4 demux, POC/DPB scheduling and direct NVDEC submission. Each visible
picture is mapped/unmapped to wait for device reconstruction without copying
pixels to CPU. Statistics report visible dimensions after SPS cropping and `cuda/nv12` or `cuda/p010`.
The standalone `native-cuda` feature exports `decode_video_cuda` without libav.
The production `cuda-hw` entrypoint uses this route without a legacy fallback;
unsupported admission happens before device creation, and execution errors never
retry through libav. CPU tests prove synthetic source admission and rewind.
An ignored NVIDIA acceptance test covers AVC crop, HEVC Main and Main10 output
counts/dimensions/formats. It has not run here. Other codecs/containers still
need owned device adapters for full functional coverage. The feature edge is
removed; this is not yet universal CUDA codec/container support.

Native CUDA decode now propagates container and codec qualification errors
verbatim before device creation. The old `try_decode` legacy fallback adapter
and libav decode-only implementation have been removed; even an explicit
legacy reference build uses the owned CUDA decode entrypoint. The existing
synthetic corrupt-NAL fixture verifies the specific `invalid NAL payload length`
error instead of a generic unsupported-input message. NVIDIA execution remains
unverified and unsupported codec tools are still incomplete native work.

## Owned CUDA host bounce

Explicit `host_bounce` copies the entire coded source into a retained typed
staging surface, then performs GPU-to-host and host-to-GPU transfers before
crop/shader processing. NV12/P010 pitches, padding and code depth are preserved
by the transfer. The staging host allocation is fallible and reused across
frames; synchronization precedes host
release, and a failed completion wait retains host storage. Defaults allocate
no host pixel buffer. Blank spans also receive the requested roundtrip, and
statistics count two full-frame host transfers per movie occurrence. The extra
identity staging kernel counts as a device pass for nonblank occurrences.
The old host-bounce refusal expectation is now an admission test. Ignored
NVIDIA tests check byte-exact NV12/P010 transfers and Main10 shader/crop export
including blanks, repeated ranges, original movie timestamps and all decoded
pixel planes. These tests are cross-compiled here but have not run on NVIDIA.

The production empty-edit interval refusal has been replaced by host admission
and movie-clock selection acceptance for AVC, HEVC Main and Main10 synthetic
fixtures. Interval preflight selects actual events before opening GPU resources;
an empty selection returns its specific error. The Main10 NVIDIA host-bounce
acceptance now runs both full-movie and 0.2–0.5 second selections, comparing all
packet timestamps/durations and software-decoded shader output, including a
blank span and both repeated ranges. Physical execution remains unverified.

## Legacy library backend retired

The media library no longer includes `legacy.rs`, gates native public exports
on `legacy-ffmpeg`, generates libav bindings, or links any FFmpeg library. Even
an explicit reference-marker build uses the same owned public API. Its build
script may add a reference library search directory; only the explicit
`ffmpeg_time_reference` benchmark declares a direct `avutil` link. Other
reference benchmarks invoke external FFmpeg, outside ordinary library/test runs.
The reverse/shuffle reference cases now compare owned results with external
level-1 FFV1 oracle files, rather than invoking a retired library adapter. This
comparison checks pixels and PTS and does not claim native FFV1 level-1
encoding support. Owned durations remain independently checked against the
synthetic source clock; FFmpeg derives its Matroska DefaultDuration from the
filtered nominal rate, so that additional oracle does not compare durations.
The default dependency guard checks the reference-marker library as well and
refuses resurrection of legacy source inclusion or libav header/link directives.
Tests use an invalid FFmpeg prefix to verify the marker does not need headers
or native FFmpeg libraries. Native codec/profile/tool coverage and actual NVIDIA
qualification remain incomplete; retirement does not imply universal parity.

The dependency audit also requires the root `--all-features` normal/build graph,
so optional production integrations cannot escape the per-feature FFmpeg gate.
A policy regression injects `ffmpeg-sys-next` into that graph and requires the
default audit to fail. This source/dependency proof remains distinct from full
native codec equivalence and physical NVIDIA/performance acceptance.

## Owned Matroska MP3 library export (2026-10-16)

`fvid-media::owned_matroska_mp3::decode_matroska_mp3_pcm` now decodes
`A_MPEG/L3` through the shared owned Layer III core. The existing sample timeline
handles CodecDelay, signed padding, gaps and ceil-rounded requested intervals.
An initial history-only Info packet contributes its container-counted silent
priming duration while preserving reservoir bytes for the first audio frame.
Coded audio that happens to begin with `Info` remains audio.

The public read-only audio plan and WAVE decode/transform dispatcher accept this
route. Atomic spool/export retains interval, resampling, channel conversion,
volume, metadata and publication policies. Decoder/Huffman staging admission is
included alongside container index and packet storage. Packet limits include
priming packets; cancellation during decode leaves no published destination.

Standalone acceptance covers 110 authored gapless indexed/free Matroska cases
at all nine rates in mono/MS, including Info-reservoir dependencies, exact PCM
windows and fractional-rate interval boundaries. A public primed-free export
matches PCM and WAVE-reference DSP; budget/packet limits, existing destinations,
mid-operation cancellation and coded-marker PCM have separate assertions.
Raw MP3 library file export is covered by the following shared-parser route.
Other unqualified codec/container workflows remain gaps; neither route silently
substitutes an external decoder.

## Owned raw MP3 library export and shared MPEG parser (2026-10-16)

`fvid-media::owned_mpeg_audio` now owns the canonical bare Layer II/III frame
parser. The player reexports its existing public parser types and adapts them to
AudioStream. Free-format inference directly invokes shared owned MP2/MP3 kernels,
so no player, external decoder or duplicate framing implementation is required.
The consuming parser avoids a second compressed-file copy. Input and growing
header/frame indexes use fallible allocation and aggregate admission.

`owned_raw_mp3::decode_raw_mp3_pcm` and public audio plans/WAVE exports accept
raw Layer III, including ID3, trusted Xing/LAME gapless windows, free-format and
Info-reservoir priming. Presentation intervals ceil-round to samples; full
preceding frames reconstruct reservoir and overlap. The Info prefix counts
against the timed decode packet limit. Packet-byte bounds also constrain
free-format inference probes, including padding bytes. Plans remain read-only;
framing probes may decode candidate packets without publishing output.

Acceptance covers 115 raw gapless/control cases and 39 pure free-format cases,
independent coded-marker PCM, public WAVE/DSP equivalence, packet-prefix output,
input/index budget admission and mid-operation cancellation. Saved short neutral
video companions remain the fixture provenance/regression artifacts. Ordinary
tests run no generators or external media processes. This path currently indexes
the complete compressed file in memory and rebuilds the prefix for interval
exports; it does not claim streaming-memory use or optimized distant seeks.
Layer I is added by the following section. Other unqualified codec/profile/export
workflows remain active gaps.

## Owned MPEG Layer I and shared MPEG library workflows (2026-10-16)

`fvid-codecs::mp1` reconstructs Layer I through owned allocation/scalefactor
parsing, joint intensity stereo, requantization, protected-field CRC and shared
polyphase synthesis. Failed packets leave live synthesis/free-spacing state
unchanged. The header accepts nine rates, including the MPEG-2.5 extension;
Layer I frame padding uses four-byte slots. The player factory exposes
`A_MPEG/L1`, its bare MPEG reader performs exact sample seek with synthesis
preroll, and Matroska playback normalizes known frame samples beneath rounded
container timestamps. The playlist/file dialog includes `.mp1`.

The shared bare parser handles all three layers. Layer I free-format probes
require a confirmed header chain, preserve EOF-bounded single frames and prevent
bad packets disappearing inside a longer candidate. Header-like ancillary bytes
are exercised after the real payload, including dense frames with all 32 bands
allocated. Ordinary tests use saved authored bitstreams and video companions.

`owned_raw_mpeg::decode_raw_mpeg_pcm` and
`owned_matroska_mpeg::decode_matroska_mpeg_pcm` expose all three owned MPEG
kernels to the library. Layer II now reaches public WAVE/DSP export alongside
Layers I/III. The old raw-MP3 entry point remains restricted to Layer III.
Read-only plans report the actual codec. Bare MPEG `probe` and MPEG format hints
use the same index, audible sample duration and geometry; a successful index
probe does not prove all compressed payloads decode. Matroska inspection names
all three MPEG codecs. Capability inventory denotes linked components, not
universal workflow/profile admission.

Layer I acceptance covers 258 authored indexed/free cases: nine rates, four
channel modes and all four joint boundaries, every allocation code 1–14,
scalefactor index 0–63, all 32 subbands and six dense cases. Independent
signed-fraction/convolution PCM is checked for every case. A dev-only independent
decoder agrees on 129 indexed cases; its explicit free-format refusal is tested
separately from owned free-format acceptance. Captured workers cover 36 baseline
cases and nine rounded-delay cases through playback, rewind and partial seek.

Thirty-six extra video fixtures exercise floor/ceil nanosecond representations
of whole-sample CodecDelay and terminal padding at every rate. Layer I restores
the delay to samples before seek/PTS/duration projection; this fixes the saved
one-nanosecond drift and ceil-rounded extra-sample trim. Library interval PCM
matches the same audible windows. Layer II acceptance reuses 174 indexed and
57 free-format authored cases, with Matroska companions for the free set.
Exact Layer II/III navigation and metadata clocks are qualified below. This
section does not mark all codec gaps closed or claim a streaming-memory/60 fps
benchmark.

Numerical/syntax reference: ISO/IEC 11172-3 sections 2.4.2.5 and 2.4.3.2:
https://ossrs.net/lts/zh-cn/assets/files/ISO_IEC_11172-3-MP3-1993-7e218e3ec360c25149d36152c4e02f08.pdf

### MPEG Layer II/III sample-clock qualification (2026-10-16)

Layer II seeks now replay synthesis history and trim to the exact requested
sample instead of returning the packet boundary. A cold second-packet decode
regression demonstrates the waveform error that this replay prevents. Layer II
currently replays from the beginning; warm-seek performance is not qualified.
Matroska Layers I/II/III share sample coordinates for PTS, CodecDelay, padding,
duration and seek. Whole-sample metadata is recovered before rational projection,
including floor/ceil nanosecond encodings; packet timestamp assertions remain
exact rather than accepting a widened tolerance.

The authored MPEG II/III corpus contains 18 raw streams, 18 control videos and
72 delay/padding videos across nine rates. Player PCM/seek/rewind and standalone
library intervals match saved independent PCM oracles. Captured playback workers
cover raw and rounded-metadata containers, including partial seek packets.
Ordinary tests read saved fixtures without FFmpeg, generators or network access.
All 109 generated files reproduce byte-for-byte; 108 artifact hashes match.

### Owned FLAC Matroska library path (2026-10-16)

The shared Matroska audio timeline now accepts `A_FLAC` through the owned
`fvid-codecs::flac` kernel. `owned_matroska_flac::decode_matroska_flac_pcm`
exposes float32 output and audible interval decoding. Public audio plans,
WAVE export and shared DSP use this route without a player dependency.
STREAMINFO geometry must agree with the track. Codec and presentation storage
use conservative aggregate admission; packet/index bytes are charged separately.
Cancellation and tiny-budget refusal leave no published file.

The saved authored corpus supplies 1460 short FLAC/neutral VP9 Matroska videos,
covering 1–8 channels and 4/8/12/16/20/24/32-bit source samples. Full and 2–6 ms
interval PCM match independent saved integer oracles exactly. Video companions
are actually decoded in the player regression. Public DSP agrees with a WAVE
control. Four-channel export uses the quad speaker mask from RFC 9639 section
9.1.3 rather than the generic four-channel FC/BC layout. Complete 1461-file
regeneration is deterministic; ordinary tests never run the generator.

This closes the Matroska FLAC library dispatch gap, not every FLAC container,
metadata-layout override, encoder or codec profile. Bare FLAC library export
and other remaining codec/workflow gaps still need separate qualification.
Reference: https://www.rfc-editor.org/rfc/rfc9639.html#section-9.1.3

FLAC milestone verification: 3 FLAC library tests, 7 MPEG integration controls,
8 shared Matroska unit tests and 1 companion-video test pass offline. Player and
standalone media-library checks pass; all 1460 saved artifact hashes agree.

### Bare FLAC library workflows (2026-10-16)

The player and library now share `owned_flac::Flac`, the canonical STREAMINFO
and CRC-delimited frame index. The library can consume an owned input buffer
without making a second encoded-file copy. Index/input/decoder admission and
encoded packet caps are applied before reconstruction. Existing parser and
player tests continue to exercise the same implementation through the adapter.

`owned_raw_flac::decode_raw_flac_pcm` supplies full and ceil-rounded sample
interval output. Bare `.flac` now reaches public audio plans, atomic WAVE export,
shared DSP and signature/hinted metadata inspection. Previously a public plan
failed in the WAVE parser; the saved authored corpus now enables acceptance.
All 1460 authored cases match independent integer-to-float PCM oracles for full
and 2–6 ms windows. Metadata inspection is checked against their sample counts.
Public workflows cover all eight channel counts, including the FLAC quad mask;
DSP matches a WAVE control. Tiny allocation budgets, encoded packet caps,
invalid stream selection and cancellation do not publish an output.

The existing `flac-matroska` corpus supplies the matching short neutral VP9
video companions; tests decode those pictures and never generate fixtures or
invoke FFmpeg/network operations. Default FLAC channel layouts are qualified;
metadata channel-layout overrides and other container/profile/workflow gaps
remain separate work. This is not a claim that the overall codec goal is done.

### FLAC comment speaker masks (2026-10-16)

The canonical bare FLAC index and library Matroska FLAC adapter now parse
`WAVEFORMATEXTENSIBLE_CHANNEL_MASK` from Vorbis comments. Field names and hex
values are case insensitive; padded hex, zero and partial masks retain their
meaning. Length/UTF-8/syntax/overflow and conflicting-mask checks use borrowed
metadata slices. Public plans and WAVE export preserve the selected assignment
rather than silently applying the default FLAC layout. WAVE inspection and
writers accept partial masks, retaining unassigned trailing channels.

Six authored examples reproduce missing LFE, FC/BC, height, center-only,
unassigned and stereo-prefix layouts. Each has a saved neutral VP9 Matroska
companion. Bare and Matroska WAVE output masks agree exactly, and all interleaved
PCM samples match the pre-existing independent integer oracles. The generator
requires no external encoder/network; all 13 files regenerate identically and
all 12 artifact hashes match. Existing default-layout FLAC cases remain enabled.

This qualifies layout preservation in library export. General spatial downmix
for nonstandard/height/partial masks and speaker-aware device playback remain
open; acceptance here must not be read as those runtime features being complete.
References: https://www.rfc-editor.org/rfc/rfc9639.html#section-8.6.2 and
https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ksmedia/ns-ksmedia-waveformatextensible

### Explicit speaker-mask downmix (2026-10-16)

The shared float32/float64 PCM mixers now use bounded source/destination
coefficient matrices instead of inferring speaker positions from channel count.
All 18 named mask positions are supported, with partial masks omitting trailing
unassigned channels. The defined fold policy keeps FL/FR at unity, folds other
named positions by lateral side at -3 dB, and omits LFE/unassigned channels.
A single center source retains the established unity dual-mono behavior. Mono
output averages the stereo fold. Expansion routes matching speakers first and
folds absent positions to available fronts; it does not invent surround/bass.
Equal channel geometry retains interleaved PCM and its original mask.

Explicit DirectOut mask zero is distinct from classic mono/stereo WAVE without
an explicit layout. WAVE inspection records this distinction and the exporter
normalizes only the implicit classic layout before mixing. Explicit zero remains
unassigned, including one/two-channel FLAC cases. Default-layout control tests
and older quad/surround tests continue to pass in both precisions.

The authored layout corpus now has 26 raw FLAC files and 26 neutral VP9
companions: all 18 individual named positions plus compound, partial and zero
layouts. Public raw/Matroska plans and 2–6 ms mono/stereo exports agree with
independent integer PCM plus an explicit speaker-weight oracle. Floating error
bounds scale with source amplitude, so low-level 32-bit fixtures still detect
incorrect routing. Direct impulse tests cover both precisions, fragmented frames,
partial/zero masks and speaker-preserving expansion. All 53 generated files
reproduce identically; all 52 artifact hashes match. Ordinary tests invoke no
FFmpeg, fixture generation or network operation.

This closes shared-library spatial fold admission for these layouts. Device
playback still needs explicit source/destination speaker negotiation; this gate
must not be presented as device routing, all codec profiles or performance proof.

### FLAC source layout through device playback (2026-10-16)

`AudioStream::speaker_mask` now carries a fallible explicit source assignment,
and the worker passes it through `AudioBackend::start_with_layout`. Existing
non-spatial backends retain their original stream contract. Bare and Matroska
FLAC expose both normative defaults and explicit comment masks, including zero.
Matroska setup now strips exactly the 34-byte STREAMINFO body: the previous
unbounded suffix accidentally included comments and reproduced `FLAC requires
STREAMINFO` before playback. The matching saved video corpus is acceptance.

CPAL routes explicit FLAC assignments to a known FL/FR stereo output before its
ring, using the owned bounded `StereoRouter`. It admits output bytes before
allocation and rejects partial/nonfinite PCM. Equal source/destination channel
counts still route rear-only or unassigned stereo correctly, rather than taking
file-export's layout-preserving identity shortcut. The packet sample-frame count,
PTS and output clock remain unchanged. Flush clears queued samples without
losing the stateless source routing assignment.

Captured workers exercise raw/Matroska playback and rewind for all 27 authored
layouts. Independent integer PCM plus literal speaker-weight oracles check
actual CPAL push, ring and callback output, including flush at a one-second base.
Core router controls cover same-count routing, bounded admission and malformed
PCM. Existing CPAL, worker and Matroska suites remain enabled. Every neutral VP9
companion actually decodes; all 55 generated files and 54 artifact hashes match.
Ordinary tests require no hardware, FFmpeg, generator or network access.

A separately invoked ignored hardware smoke test successfully opened the default
physical output at 48 kHz in a paused state for a six-channel source folded to
two output channels, then closed it. It emitted no sound. This proves the local
paused-open path, not audible end-to-end output, mono-only devices, native
multichannel-device layouts or every codec/profile. Explicit device routing for
other codec families and the broader codec/workflow goal remain open.

### AAC device routing and exact seek boundaries (2026-10-16)

AAC ADTS, MP4 and Matroska readers now negotiate an owned `StereoRouter` through
`AudioStream` and `AudioBackend`. CPAL receives coefficients in emitted PCM order,
not merely a channel count or a WAVE mask. The AAC helper retains explicit PCE
height/position/group descriptions, including bottom layers, repeated positions
and layouts whose WAVE mask is zero. Indexed layouts retain their declared mask;
explicit/negotiated PS uses FL/FR. The bounded router accepts up to 64 explicit
coefficients, with actual compressed ten-channel PCE acceptance beyond the old
eight-channel matrix limit. This is not a claim of compressed 64-channel coverage.

The defined PCE fold uses -3 dB for vertical/side/back groups, omits LFE, and
routes the final normal-front pair at unity. A lone normal-front center keeps
dual-mono unity. This is an explicit rendering policy, not a reconstruction of
spatial geometry or a claim of every possible PCE mixdown instruction.

Seek acceptance exposed old packet-boundary landing in both MP4 and Matroska
AAC. They now retain complete decoder preroll and trim to the requested sample.
Positive MP4 edits add priming once; MP4 EOF/beyond-end requests clamp to the audible
endpoint. Nanosecond packet rounding no longer discards an extra sample: the
saved 50 ms request previously landed at 50.020832 ms. Quantized source points
are recovered on their sample grid, while a request 1 ns beyond a boundary still
uses strict ceil. Whole-sample CodecDelay rounding is restored where identifiable;
coarse/off-grid packet timestamps retain their declared timing. This does not
claim a general rewrite of every coarse/discontinuous Matroska clock.

`aac-device-layout` contains 18 authored layouts, 36 MP4/Matroska companions and
two extra videos with a 48-sample positive edit and two floor/ceil CodecDelay
companions. Original independent cosine PCM
oracles supply literal-weight stereo oracles. All 54 raw/container combinations
pass full audio decode; all 40 companion videos are actually decoded. Captured workers verify full/rewind,
2 ms and 50 ms seeks with exact PCM/sample-clock assertions; positive-edit tests
also cover partial/last samples, EOF and beyond EOF. Actual CPAL push/ring/callback
code matches the independent oracles. Existing packet-boundary assertions were
replaced with exact PCM suffix acceptance; no timestamp tolerance was widened.
Floor/ceil CodecDelay videos preserve exactly 17 trimmed samples and exact
0/2/50 ms boundaries. All 77 generated files reproduce identically and all
76 artifact hashes match.
Previous height artifacts remain byte-identical; only the authored ten-channel
case was added. Ordinary tests invoke no generators, FFmpeg, network or hardware.

Additional metadata admission covers all 72 saved ER-AAC/SBR indexed layouts.
Existing LD, LTP, PS, MP4, Matroska, worker and CPAL regressions remain enabled.
Separately invoked ignored physical smoke tests successfully opened and closed
paused default output for both six-channel mask FLAC and ten-channel explicit
AAC PCE at 48 kHz; neither emitted sound. Production player and standalone media
library compile offline. Audible hardware playback, arbitrary midstream layout
renegotiation, PCE-aware library spatial export and other codec/workflow gaps
remain separate work; the overall codec goal remains active.

### PCE-aware AAC mono/stereo export (2026-10-16)

The public ADTS, MP4 and Matroska audio-export paths preserve explicit AAC
positions across their private decoded-WAVE boundary. Requested mono/stereo
folds use the same owned emitted-order `StereoRouter` as playback before WAVE
interval, gain and resampling processing. Identity channel-count exports retain
the original geometry and mask. Ten-channel PCE is accepted without pretending
that its duplicate front positions fit a standard WAVE mask. Private PS spools
now finalize their channel count and mask from negotiated decoded output, and
validate written PCM bytes against returned sample geometry.

`crates/fvid-media/tests/aac_spatial_export.rs` qualifies all 18 authored layouts
across ADTS/MP4/Matroska using independent literal-weight PCM oracles, including
mono, intervals, gain and resampling. Separate cases cover immediate/late PS
negotiated stereo, packet-prefix limits, cancellation, controlled admission,
existing-output preservation, quantized clocks and gap silence. Backwards
presentation overlap remains an explicit export refusal: no WAVE is published;
that refusal is not acceptance of overlapping timeline export. Ordinary tests
need no FFmpeg, network or fixture generation.

General PCE upmix to three or more channels, PCE mixdown instruction rendering,
and arbitrary midstream geometry/layout changes remain unqualified. This closes
mono/stereo spatial export only, not the overall codec goal.

Additional saved AAC clock videos qualify public spatial-export tail padding:
6131 audible samples after a 13-sample trim, independent stereo PCM, and no
publication on oversized packet padding. The player independently qualifies
source-AU padding identity for delayed PS; file-export overlap refusal and
player discontinuity acceptance remain deliberately distinct policies.

### Shared AAC coded-clock quantization and signed windows (2026-10-16)

AAC Matroska export now advances a coded-output sample clock independently of
audible trims. Container PTS are resolved against that clock, then exact signed
padding, negative preroll, gaps and requested intervals are applied. AAC no
longer rebases its first packet or subtracts leading padding from later sample
positions. The root native-media include and library share this implementation;
the root reader and export share the owned `quantized_sample_matches` helper.
The other codec branches retain their existing policies pending independent
qualification; no AAC result proves those branches correct.

Saved acceptance covers five signed-window/offset videos, actual CPAL callback
placement, public WAVE publication and delayed immediate/late PS with different
internal and terminal trims. The full-tick gap fixture preserves exactly 48
samples of silence. Existing PS negative-preroll export oracles now address
actual source PTS; 18 media artifacts remain byte-identical. All fixture
regeneration is explicit and owned; ordinary tests use saved inputs only.

### Owned scaled AAC block grids (2026-10-16)

The shared WebM/Matroska reader retains compact original block timing only for
non-unit AAC tracks. `PacketGrid` contains the cluster, signed relative tick and
lace displacement; the original factor is exposed without changing the track's
sample rate. Retained capacity participates in `estimated_index_payload_bytes`.
Both root and library reader wrappers use this implementation.

`owned_matroska_grid_clock` performs exact IEEE dyadic comparisons in bounded
integer limbs, without a foreign bigint or codec dependency. AAC export carries
the original packet index through delayed decoder queues, and playback retains
the grid epoch in its paired seek checkpoint. Twelve saved videos exercise LC
PCE and Main/PS, fractional/near-unit scales, cluster shifts, lacing, actual gaps,
EOF and warm seek. The existing unit-scale clock/window gates remain enabled.
The math and metadata results do not imply conformance of untested compressed
profiles or of scaled timing in other codecs.

### PCM presentation windows share the coded clock (2026-10-16)

The shared Matroska scheduler now applies exact coded-window timing to AAC and
the three PCM codec IDs, including its f64 PCM instantiation. Signed padding and
CodecDelay are applied after quantization recovery; PCM no longer rebases its
first timestamp or leading trim. Original grids are retained for non-unit audio
tracks and charged in the index estimate, but this does not qualify their use by
all other decoder adapters. Ordinary unit-scale tracks retain no extra records.

The PCM source stride comes from the shared decoder implementation. Library
Matroska setup now uses the same integer16/float32 compatibility defaults as the
player when BitDepth is absent. The root PCM seek walks indexed restored lengths
and clock metadata, selecting an exact audible window without decoding history.
A metered raw source proves it reads no PCM preroll payload; compressed container
indexing can still require decompression to establish restored lengths.

Forty-eight saved format/timeline combinations verify independent PCM, exact
seek, f32/f64 interval output, public export and plans; a separate malformed
sample-frame video verifies explicit refusal and no partial WAVE publication.
The additional negative-preroll scaled PS cases preserve original source identity
through lookahead/EOF. These qualifications do not close the remaining codec,
profile, layout or workflow gaps.


## Native HEVC texture-motion qualification

The shared layered HEVC owner now admits monochrome depth `tex_mc` and connects
current-AU texture selection, VPS POC/ViewIdx matching, extended merge selection,
per-sub-block native reconstruction and exact motion publication. The 96 owned I/P/B witnesses now explicitly select enhancement PTL 8 with the
I.11.1.1 conformance-indication flags, a separate Main base, and automated
profile/role/geometry/offset/tool checks. Common software entry, packed output,
rewind and precise seek are covered as well. These are acceptance checks for the
admitted subset, not certification of complete 3D Main. The owned
I/P/B playback witnesses cover on/off, CQT on/off, skip/DC-only, unavailable T,
full positive rounding range, direct and queued reconstruction, and rewind.
These are eight-bit monochrome depth, L0-derived T acceptance witnesses.
A separate coded I0/I2/B1 witness now qualifies unweighted bidirectional T with
past POC 0 and future POC 2, distinct authored depth patterns, opposite per-list
AMP motion, exact per-pixel averaging, both published motion lists and reference
identities, and rewind. Three additional coded bi-T witnesses qualify signed
weights/offsets at luma denominators 0, 1 and 7, including lower/upper clipping.
They retain the distinct reference identities and motion checks. Complete
Annex I/profile conformance and full filter-combination qualification are not implied.
A further 24 coded P witnesses qualify weighted unidirectional T at 32×32 and
128×128, direct/queued reconstruction, skip and DC-only residual. The four
weight/offset tables cover signed weights, denominators 0/1/7, both clipping
directions and prediction clipping before DC addition, through rewind.
Four additional on/off deblocking movies qualify true CU/PB edges versus T's
internal 8-row motion boundaries with exact nonzero weak-filter pixel changes,
in direct and queued 128×128 reconstruction, through rewind. Eighteen further
coded I/P/B witnesses qualify SAO band positions 0/8/31, signed offsets, wrap,
clipping, left/up parameter merges, skip/DC residual ordering and reuse of the
filtered P reference by B, in direct/queued paths and through rewind.
A further 24 coded witnesses qualify all four SAO edge directions, all five
sample categories, unavailable picture-edge neighbours, merged CTU parameters,
DC ordering and filtered reference reuse, in direct/queued paths through rewind.
Another 168 coded P witnesses qualify all four owned weight tables combined
with band positions 0/8/31 and all four edge directions, skip/DC residuals,
32×32 and 128×128, direct/queued reconstruction and rewind. Their independent
oracle checks weighting, prediction clipping, residual addition/output clipping,
then SAO using immutable reconstructed neighbours. Every combination has an
observable SAO effect. Sixty-three weighted B/SAO witnesses additionally combine
three signed weight tables with all seven SAO modes at 32×32 and 128×128,
including direct/queued reconstruction. Eight further 128×128 unfiltered controls
qualify the distinct-reference bi path itself. Both reference identities and
per-list motion remain checked. Strong deblocking, additional boundary/residual
combinations and other unimplemented tools are not qualified by these cases. Integer-domain depth motion resampling is implemented as a component utility.
3D Main forbids reference offsets/unequal reference dimensions for depth layers
(I.11.1.1), so spatially resampled depth is not a 3D Main playback requirement.
Coded scalable reference preparation is now covered by six three-layer
Scalable Main 4:4:4-signalled movies carrying monochrome enhancement layers,
including temporal MVP, sample resampling, signed MV limits and rewind.
Complete profile/tool conformance and other active inter-view 3D tools remain
open. See [NATIVE_PLAYBACK.md](NATIVE_PLAYBACK.md) for gate details.


Annex I inter-view temporal MV scaling now has owned arithmetic
(`hevc_motion::inter_view::temporal_vector`, I-125..I-134), checked numeric VPS
view identifiers (I-11), and native temporal merge context propagation.
Explicit AMVP retains base collocated rules and copies long-term source motion.
The active `iv_mv_scal_enabled_flag` is admitted. Thirty-two owned three-view
texture movies qualify on/off flags with unchanged explicit-AMVP controls and
scaled skip/temporal merge: positive/negative scale, equal/zero view distances,
signed MV limits and scale-factor clipping. Actual source/target pixels, saved vectors and reference identities,
ordinary player layer selection, packed output, rewind and exact seek are checked.
The VPS explicitly authors its 3D extension with no camera parameter pairs.
The earlier specific parameter refusal is replaced by coded acceptance; texture-side
inter-view disparity and other unimplemented tools remain refused. A separate 120-movie
four-layer matrix now qualifies depth-view temporal scaling at 32×32 and
128×128, direct/queued, explicit-AMVP copy controls and skip-merge scaling,
on/off, signed scale reversal, zero/equal distances, integer rounding and full
signed-16 depth MV saturation in merge mode.
Every 8×8 saved motion entry is checked in the shared quarter storage domain,
along with exact pixels, player output and seek. Additional geometries and
camera-dependent tools are not qualified by these matrices.
These checks do not certify full Annex I or 3D Main conformance. Existing Annex A/F
paths retain their ordinary predictor semantics.


Annex I-signalled VPS tails now have a bounded owned `ThreeDExtension` reader
rather than remaining opaque. It retains cp_precision, sparse ViewOIdxList camera
pairs, static signed values or explicit slice-header deferral, with memory
accounting and cursor rollback. Malformed precision, duplicate references,
out-of-range references and truncation have authored MP4 regressions. Static
unused camera metadata preserves pixels/timing/rewind. Unrelated profile namespaces
retain their opaque extension semantics. Per-slice camera values are now read
before entry points/header extensions, retained in bound slice syntax and charged
to header storage. Independent I-only camera movies verify signed limits, pixels,
timestamps and rewind. Disabled illumination-compensation flags are consumed in
Annex I texture/inter-view headers; active illumination compensation remains a
specific refusal. Texture-motion merge syntax now follows I-35/I-36, supports six
candidates and selects the sixth zero candidate or texture candidate in coded
controls, including unavailable texture motion and direct/queued reconstruction.
Camera-dependent prediction tools still require implementation.


Camera depth-to-disparity arithmetic (I-37..I-43) is now owned in
`hevc_motion::disparity::CameraTransform`, reachable from parsed bound
`SliceCamera::disparity_transform`. Signed inverse differences and evaluation
use i64, preserving normative unclipped outputs beyond i32. Direct sampling
requires no allocation; optional tables enforce explicit ownership budgets.
Independent exact-fraction fixtures cover 270 parameter/precision/depth cases,
30 complete eight-bit tables and selected wider-domain points. This is arithmetic
and parsed-header binding acceptance, not yet camera-dependent reconstruction;
those SPS tools remain refused.

Annex I.8.3.1 disparity candidate-picture selection is now owned alongside the
camera arithmetic. It preserves collocated-list ordering and duplicate list
slots, prioritizes same-view IRAP pictures, then minimum temporal ID and strict
POC distance below 255. POC differences are widened before subtraction. Unit
coverage includes list reversal, ties, repeated references, unavailable
candidates and extreme POCs. The current HEVC unit gate passes 266 tests (one
explicit fixture-regeneration helper ignored), offline without FFmpeg. This
metadata component is not yet wired into inter-view disparity reconstruction;
it does not remove the remaining SPS tool refusals or establish full codec
coverage.

Annex I.8.3.2 default disparity reference selection now chooses the lowest earlier
ViewIdx with a same-POC active reference, respecting P versus B active lists.
An exhaustive 2,160-case small-domain comparison uses the normative nested-loop
search independently of the implementation's minimum reduction. The selection
can bind to `SliceCamera::default_disparity_transform`; an unavailable default
view needs no camera pair, while a selected view requires its parameters.
The HEVC unit gate passes 267 tests (one regeneration helper ignored). This
remains component work toward inter-view disparity reconstruction, not playback
acceptance of that SPS tool.
The parsed-camera and exact-fraction disparity integration suites also pass
without FFmpeg/network. The I-only camera movies qualify parsed-header binding,
pixels, timing and rewind; their active-list inputs for the default-view bridge
are authored test metadata, not coded P/B disparity playback evidence.

Owned I.8.5.7 / I-300..I-312 sample derivation now covers partIdc 0, 1 and 2:
rounded quarter-sample reference coordinates, edge clipping, four-corner maxima,
horizontal/vertical 8×4 or 4×8 partition decisions, raw depth and backward
camera disparity outputs. The raw-depth picture entry point needs no camera
parameters. Output allocation is bounded before reserve and actual retained
capacity is checked; widened coordinate math also handles extreme public API
inputs without overflowing. A reconstructed monochrome picture bridge validates
precision and completion; the caller remains responsible for resolving the
current-AU depth-role layer of the selected view.
An owned short four-layer depth movie plus 378 independent per-output-sample
Fraction oracles qualify the component on decoded depth, including both split
directions, signed rounding, clipping and exact/insufficient budgets. Generating
all three artifacts is offline and byte-reproducible, separate from ordinary
tests. The HEVC unit gate passes 269 tests with one regeneration helper ignored.
These outputs are not yet consumed by inter-view disparity/VSP reconstruction;
the corresponding production tool refusals remain. Extreme arithmetic API cases
are not claims of legal coded motion-vector ranges or full profile acceptance.

Owned inter-view predicted merge derivation (I.8.5.3.2.10 / I-164..I-172) now
maps disparity plus offset to the exact aligned reference-view PB, then copies
same-POC saved motion into the current active lists. Same-list source precedence,
opposite-list fallback even for current P slices, first matching target slot and
canonical source-list indices are covered. Source motion is neither temporally
scaled nor converted from quarter to integer depth units by this process.
Widened coordinates preserve signed rounding and alignment-before-clipping.
A reconstructed-picture bridge uses the retained exact 8×8 motion map, including
an owned AMP movie whose opposite vectors meet on row 8. The coded three-view
source also drives reference mapping through native sample prediction with exact
pixel and rewind checks. The 272-test HEVC unit gate (one regeneration helper
ignored) and both new integration tests pass offline without FFmpeg.
This is a tested candidate-to-sample component pipeline with explicitly supplied
reference view/disparity, not admission of coded inter-view disparity merge or
full Annex I playback. Production tool refusals remain until candidate ordering,
disparity derivation and reconstruction dispatch are connected and qualified.

Owned DI/shifted-DI and VSP candidate derivation (I-181..I-194) now preserves
wide camera vectors. DI requires current POC plus reference view and shifts
horizontal motion by four quarter samples without wrapping. VSP selects one
first matching view across L0/L1, independently of POC, and binds current-AU
reference-depth sampling to horizontal/vertical 8×4/4×8 motion publication.
No available reference means no camera/depth allocation is required. Retained
VSP sample ownership includes the enclosing metadata and actual Vec capacity.
An offline exact-rational/scalar eight-tap oracle verifies 48 candidate-to-luma
prediction cases using two owned coded source movies, both split directions,
both selected lists, positive/negative camera scale, all four horizontal
fractional phases, edge extension and exact/insufficient memory budgets.
Both generated oracle artifacts are byte-reproducible. The 274-test HEVC unit
gate (one explicit regeneration helper ignored) and the new integration test
pass without FFmpeg or network. This is luma component-pipeline evidence with
explicitly supplied view/disparity/camera inputs. Weighted VSP qualification,
full candidate ordering, coded-tool admission and decoder dispatch remain open;
production SPS tool refusals are retained until those paths are implemented.

The 8-bit 4:2:0 VSP component pipeline now also has nonconstant Cb/Cr
qualification. An owned three-view PCM movie carries distinct spatial chroma
patterns; 72 independent rational/four-tap oracle cases verify both color planes,
all eight horizontal eighth-sample phases, signed camera scales, both split
directions and either selected reference list. The existing 48-case luma gate
remains enabled. The new source's complete six-picture playback checks exact
Y/Cb/Cr samples, timestamps, durations and rewind. Camera/disparity/view inputs
are still supplied explicitly by the component test, and weighted VSP and full
coded-tool dispatch remain unqualified. Fixture creation is offline, separate
from tests; three new artifacts regenerate byte-identically and four prior
movies remain byte-identical under the shared author's default parameters.

Annex I extended merge ordering now has a shared owned engine (I-135/I-136),
with T/IV precedence, depth-dependent A1/B1 comparisons, left-VSP insertion and
inheritance, DI pruning against the available spatial entries, IVShift/DIShift
availability rules, bounded nonspatial fill, original-small-PB selection
(I-137/I-138), and selected-candidate flags (I-139). The existing production
texture-only merge path delegates to this engine while retaining one bounded
output allocation. An independent unbounded-list pseudocode oracle verifies
3,072 metadata combinations across both depth roles, capacities one through
six, B2/A0 layouts, availability/duplicate cases and left-VSP inheritance.
The oracle regenerates byte-identically offline. The final 277-test HEVC unit
gate (one regeneration helper ignored) and 19 integration tests across ordering,
CQT and coded six-candidate texture merge pass without FFmpeg/network.
Depth T and depth IV now connect to decoder dispatch. Texture-side IV/DI/VSP
still require current-picture disparity state and selected-candidate
reconstruction dispatch. Metadata selection acceptance
must not be treated as coded IV/DI/VSP playback acceptance.

Current-AU reference-view binding is now owned in
`hevc_picture::select_current_view_component`. It resolves explicit ViewIdx and
texture/depth role through I-9, excluding dependency/auxiliary representations
and retaining the final matching layer. The existing same-view component lookup
uses this shared mapping. Source selection requires its pinned active binding
and the current picture/slice identity, checks VPS snapshot agreement, source
SPS/OLS identity, same-resolution luma precision, AU/POC, unique completed layer,
and complete Y/Cb/Cr geometry and actual plane precision. It shares the source
Arc rather than copying pixels or searching temporal DPB history.
Four new integration tests use owned decoded texture/depth view movies, including
equal numeric view_id_val across distinct ViewIdx, role separation, stale or
ambiguous sources, changed VPS snapshots, incomplete chroma/luma and forged
precision metadata. All ten integration tests across view-component selection,
existing TexturePic handoff and VSP pass offline without FFmpeg/network. This
qualifies the source-binding API and shared graph lookup, not automatic IV/DI/VSP
tool admission: the decoder owner still must combine it with disparity derivation
and selected-candidate reconstruction dispatch.

Owned texture disparity derivation now covers I.8.5.5: 16-aligned temporal
CU-centre lookup through saved canonical reference layers, different-view/active-
view temporal qualification, temporal-before-spatial priority, A1/B1 direct
same-POC vectors, later-direct-before-earlier-saved-skip fallback, parallel-merge
exclusion, B1 CTU-row restrictions on saved skip disparity, and default-view zero
fallback. Z-scan/slice/tile/intra availability remains an explicit callback
contract for the reconstruction owner. Optional backward-depth refinement keeps
DispVec intact while replacing DispRefVec horizontally; the depth-layer midpoint
default follows I-297..I-299 and preserves unclipped camera arithmetic.
An independent priority/outcome model qualifies 1,152 metadata combinations and
regenerates byte-identically offline. A coded three-view regression derives its
candidate order, active lists and collocated source from parsed headers and
native saved motion rather than injecting the resulting disparity vector.
Separate owned-depth checks qualify refinement, signed/extreme coordinates and
exact/insufficient budgets. The 280-test HEVC unit gate (one regeneration helper
ignored) and six integration tests across derivation, arithmetic, sampled depth
and parsed cameras pass without FFmpeg/network. These are derivation and source-
bridge components; current picture-cell state publication and automatic active
IV/DI/VSP reconstruction dispatch are still required before coded-tool admission.

Inter-view sub-block motion (I-150..I-163) now uses an owned common partition/
default-inheritance engine shared with the existing production texture-motion
path. It retains unshifted target rectangles until source-specific lookup,
chooses the normative central default, stops when that default is inactive,
and inherits the full default only when both sub-block lists are inactive.
IV applies disparity and 8-grid alignment exactly once through its source
bridge. Caller-budgeted tables preflight checked ownership, reserve fallibly and
check actual capacity; the bounded texture specialization retains its existing
reconstruction-owner budget contract and one table allocation.
Two owned nonconstant-Y/Cb/Cr AMP sources qualify quarter-sample magnitudes one
and four across the row-8 boundary. Independent Fraction/scalar 8/4-tap oracles
check 200 IV sub-block component cases: layouts, whole-PB fallback, signed/extreme
disparity, default motion, exact per-sub-block motion, uni/bi prediction, all three
sample planes, unavailable defaults, exact/insufficient budgets and source rewind.
All four generated artifacts are byte-reproducible and four older movies remain
unchanged under generator defaults. The 282-test HEVC unit gate (one regeneration
helper ignored) and 19 integration tests across CQT, IV sub-blocks and coded six-
candidate texture merge pass offline without FFmpeg/network.
IV inputs are supplied explicitly in this component pipeline; these sources do
not enable coded IV/DI/VSP merge admission. Decoder cell-state publication and selected-candidate dispatch still need
implementation/qualification; explicit AMVP follows its base normative route.


An AMVP audit found and corrected a production decoding error: the common
layered temporal helper had applied I-125..I-134 view-distance scaling to
explicit AMVP as well as merge. I.8.5.3.2.5 explicitly invokes base 8.5.3.2.9
for non-merge prediction, where long-term motion is copied unchanged. The HTM
reference likewise gates view scaling with bMRG. Explicit AMVP now selects the
base route; merge still selects the Annex I route. No extra IV/DI AMVP candidate
insertion is prescribed or introduced by this correction.
Eight new owned short texture/depth movies isolate scaling on/off and explicit
AMVP versus skip merge. Before the fix the acceptance test failed specifically
at target MV [0,16] versus expected [0,4] after validating the source MV; it now
passes exact source/target motion, pixels, timing and rewind. Earlier AMVP
expectations that incorrectly demanded view scaling were corrected in the
32-texture/120-depth movie matrices and source-candidate tests. Those 152 movies
remain byte-identical; their manifests now state merge-only scaling. The nine
new fixture outputs also regenerate byte-identically, offline and separately
from tests. Final gates: 282 HEVC unit tests pass (one regeneration helper
ignored) and 36 integration tests across ten suites pass without FFmpeg/network,
including direct/queued depth, normal player output, seek and retained IV/VSP/T
component coverage. Texture-side picture-state publication and automatic
IV/DI/VSP reconstruction dispatch remain open; this fix does not close full codec coverage.


Native monochrome depth IV is now connected from coded SPS/header admission to
ordinary reconstruction, not only exposed as a component. The decoder derives
DefaultRefViewIdx from same-POC active depth references, requires its bound
camera parameters, evaluates the midpoint backward disparity (I-297..I-299),
selects the canonical current-AU source already validated/prepared by the AU
owner, and maps its motion into current lists by POC. The shared extended merge
selector combines depth T/IV and initial candidates; existing reconstruction
publishes the selected motion in the normal cells and saved motion maps.
Source completion/camera/list preparation is cached per slice, not repeated per
CU, and uses fixed-size active-list scratch without new frame-sized allocation.
The depth iv_di_mc flag is admitted for monochrome depth; context-free active
reconstruction is refused. Texture-side IV/DI/VSP admission remains closed.

Twenty-six owned coded acceptance movies cover IV on/off, indices 0/1/2/4,
inter/intra sources, 32×32, 128×128 queued/direct, static camera parameters,
normal software-player highest-layer selection, exact packed pixels, rewind and
40 ms seek. Two source AMP controls give opposite left/right motion: changing
camera offset selects +1 versus -1 pixel, proving camera-driven source lookup
rather than metadata-only parsing. Missing required camera parameters have a
separate specific refusal witness. Before implementation the acceptance failed
at the active-3D-SPS refusal after the disabled controls decoded successfully;
the old depth-disparity parameter refusal test now expects acceptance.
All 28 new fixture artifacts regenerate byte-identically offline, with the
camera author's existing defaults unchanged. Final gates pass 283 HEVC unit
tests (one explicit regeneration helper ignored) and 46 integration tests across
12 suites without FFmpeg/network. The coded matrix qualifies these P-slice
scenarios, not full Annex I/profile conformance or all tool combinations.
Texture-layer disparity state/IV/DI/VSP dispatch and further codec gaps remain.

Depth-IV B-picture acceptance now adds 64 synthetic coded movies with an
independent scalar pixel oracle: bi/L0-only/L1-only source motion, target
weighted biprediction with distinct per-reference weights, opposite-list POC
mapping, merge indices 0/1/2, enabled/disabled and intra-source controls,
32/128 geometry and queued/direct paths. Tests distinguish encoded decode
order 0/2/1 from presentation order 0/1/2, check source/target motion and
pixels, normal-player output, rewind, and eight repeated forward/backward
native seeks. All 66 generated artifacts are byte-reproducible offline.
The focused four-suite gate passes 13 tests without FFmpeg/network. These
cases disable temporal MVP; texture-side dispatch and general Annex I tool
combinations remain unfinished.

The Annex I initial merge list now retains the five-candidate combination
threshold even when `tex_mc` enables six total entries (I.8.5.3.2.2). Previously
four distinct spatial candidates plus temporal MVP incorrectly admitted a
combined bi-predictive sixth entry instead of the zero candidate. The owned
`initial-merge-six-*` movies isolate this boundary at a 16x16 PB in a 64x64
picture: A1/B1/B0/A0 carry distinct motion and the fifth temporal candidate has
opposite L0/L1 motion. All six indices have queued/direct controls, with an
intra texture source making T unavailable. The regression compares exact
pixels, stored motion, reference indices and reordered output. The generator
and manifest are offline; tests never generate movies or invoke FFmpeg.
All 13 artifacts regenerate byte-identically. The pre-fix regression reached
indices 0 through 4 correctly and failed at index 5 (L0 vertical motion 4
quarter units instead of zero). The fixed gate passes 20 integration tests
across `hevc_cqt`, `hevc_depth_iv_b`, `hevc_initial_merge_six` and
`hevc_six_merge`, offline and without FFmpeg. Full codec coverage remains open.

Annex I temporal merge now implements alternative target-reference selection
(I.8.3.4, I-120..I-124): when the collocated source and index-zero target have
different short/long-term classifications, the first active opposite-class
target is used for scaling. Its index is published only when Col itself is
selected (I.8.5.3.2.7 step 9); initial combination derivation retains its
original indices. Explicit AMVP retains its coded reference and the base
classification refusal. No extra picture storage is allocated.

Twelve owned `temporal-alt-ref-*` movies cover default/swapped mixed ST/IL-LT
lists, skip merge/explicit AMVP, 32/128 geometry, queued/direct reconstruction
and rewind. Exact pixels and saved motion/reference indices are checked at
every target sample. The default-list control passed before the fix, while
the swapped-list merge failed at pixel (0,0), 91 instead of 97. All thirteen
artifacts (including the manifest) regenerate byte-identically offline;
ordinary tests never generate fixtures or invoke FFmpeg/network. This matrix
does not qualify every combined-candidate or alternate-list arrangement, nor
texture IV/DI/VSP dispatch or complete codec coverage.
Final gates pass 283 HEVC unit tests (one explicit regeneration helper ignored)
and seven integration tests across `hevc_temporal_alt_ref`,
`hevc_initial_merge_six`, `hevc_inter_view_scaling`,
`hevc_depth_view_scaling` and `hevc_amvp_merge`, offline without FFmpeg.

The additional `hevc_temporal_alt_combined` matrix qualifies the publication
boundary with 15 owned movies at 64/128 geometry and queued/direct decoding.
The first 32x32 CU selects Col and publishes references [1,1]; the next CU
selects A1, Col, A1L0+ColL1, ColL0+A1L1, or Zero0. The two combined entries
retain reference pairs [1,0] and [0,1], respectively, while selected Col uses
[1,1]. Every first/probe-CU sample and saved motion/reference is checked against
a scalar labeled-picture oracle. Explicit distinguishing-pixel assertions
ensure the oracle detects both premature alternate publication and failure to
publish selected Col. Rewind and five successive native seeks qualify the
normal software-player probe. All 16 artifacts regenerate byte-identically
offline without FFmpeg/network. This does not qualify every possible initial
list or extended IV/DI/VSP combination, and full codec coverage remains open.
The final focused gate passes all four tests in `hevc_temporal_alt_combined`,
`hevc_temporal_alt_ref`, `hevc_initial_merge_six` and `hevc_amvp_merge`.

The immutable native coding map now preserves the coded CU skip flag in its
existing byte (bit 5), alongside CU geometry and exact motion. Geometry parsing
masks this flag and retains its prior sizes/partitions; no frame allocation or
budget increase is required. `Picture::prediction_is_skipped_at` exposes the
saved flag. `disparity::derivation::neighbour_from_picture` binds a spatial
neighbour to that flag and the picture's canonical saved POC/layer lists,
mapping layer IDs through the caller's bound view resolver. Intra/no-motion
sources return unavailable; invalid positions, missing identities and IV-skip
without supplied saved disparity are refused.

`hevc_disparity_spatial_binding` uses existing owned coded texture AMVP/skip
movies to check every sample's saved skip/motion/reference identity and rewind,
then supplies explicit IV/refinement state to test fallback priority. A map
unit checks every CU size/partition, shared storage and identical heap costs
with skip on/off. This is a prerequisite bridge: production texture IV still
needs native IV/refined-state publication and reconstruction dispatch. The
component test's supplied IV/refinement flags do not establish coded texture-IV
playback acceptance or remove its SPS refusal.
Final gates pass 284 HEVC unit tests (one regeneration helper ignored) and
23 integration tests across `hevc_cqt`, `hevc_depth_iv`,
`hevc_disparity_derivation` and `hevc_disparity_spatial_binding`, offline and
without FFmpeg/network.

Native IV selection now remains distinguishable as `Resolved::InterView` until
publication, rather than becoming ordinary motion immediately. Selected depth
IV publishes its camera-derived base/refined disparity on the exact aligned
8x8 motion grid. The optional map is moved into immutable shared storage;
inactive pictures allocate none and all-empty maps are dropped. Its retained
heap is counted, clone/storage identity includes it, and reconstruction
admission includes the Vec-to-Arc conversion peak. Narrow 4/12 AMP publication
uses aligned sample ownership, preserving neighbouring PB state.

`Picture::inter_view_disparity_at` and
`disparity::derivation::published_neighbour_from_picture` read native selected
IV/skip/disparity state without external labels. `hevc_iv_publication` checks
90 existing owned P/B scenarios, all eight-aligned positions, on/off/intra/index
controls, source camera offsets, queued/direct reconstruction and rewind. Only
selected IV carries state; ordinary candidates with equal pixels or motion do
not. The saved state drives the existing spatial fallback component. Map and
budget units check optional/shared/retained storage, malformed state and narrow
AMP boundaries. This is active depth-IV publication and a prerequisite for
texture IV. Native texture IV/DI admission and dispatch are now connected below;
VSP and additional tool combinations remain unfinished.
Final validation passes 287 HEVC unit tests (one explicit regeneration helper
ignored) and 23 integration tests across `hevc_iv_publication`, `hevc_depth_iv`,
`hevc_depth_iv_b`, `hevc_disparity_spatial_binding`, `hevc_cqt` and
`hevc_temporal_alt_combined`, offline without FFmpeg/network.

Native pictures retain the first slice's NAL header through separate-plane
assembly and POC metadata translation. `Reference::disparity_metadata` derives
ViewIdx, temporal ID and IRAP classification from that decoded source and its
bound layer identity; missing pictures and relabelled NAL/reference layers are
refused. POC remains the reference owner's identity, including rebasing.

`disparity::derivation::PreparedTexture` binds a slice's active reference lists
with fixed-size metadata/view scratch, selects up to two temporal sources and
DefaultRefViewIdx, and checks selected-source completion/representation once
at preparation. Per-CU `derive` combines native temporal source lookup with
spatial/default priority and optional caller-bound depth/camera refinement.
The caller still enforces spatial availability and current-AU refinement
binding. Existing coded three-view texture fixtures check the prepared path
against the independent component results, temporal-before-spatial priority,
bounds and missing/mislabelled sources. Ninety owned P/B movies verify retained
NAL metadata on decoded pictures. The component pipeline now feeds native
texture IV/DI dispatch below; its earlier component checks alone did not prove
playback. VSP and depth-reference refinement admission remain unfinished.
Final gates pass 287 HEVC unit tests (one regeneration helper ignored) and
19 integration tests across `hevc_disparity_derivation`, `hevc_iv_publication`,
`hevc_poc_reset` and `hevc_separate_colour_planes`, offline without FFmpeg/network.

`texture-iv-*` now has thirty owned 32x32 coded dispatch witnesses: IV on/off,
inter/intra reference source, and five merge indices. Only the source view-1
component is active in the target's IL set; its other active reference is the
target's own POC0 picture. The inter source has +1/-1 vertical AMP motion at an
8/24-row boundary. Independent scalar luma oracles distinguish IV subblocks,
DI, DIShift and ordinary zero candidates. The native disabled controls decode
and rewind with exact source/target pixels; the component pipeline matches the
enabled IV/DI oracle. Available IVShift equal to the IV default is also checked,
including its suppression of DIShift.

All fifteen enabled streams now pass native playback acceptance, together with
fifteen disabled controls. The former specific SPS refusal expectation is
replaced by parameter/reference-list admission and an enabled acceptance test.
Both layered and standard software-player paths check pixels, saved IV state,
motion/reference indices, rewind and repeated seek. Source AMP layouts 8/24 and
24/8 distinguish sub-block IV from a flattened default vector, exercise a
separately selected IVShift, and check suppression of DIShift when IVShift is
available but equal to IV. All 32 artifacts regenerate byte-identically offline,
separately from tests. These witnesses have neutral chroma and qualify P slices
at single-CU 32x32 geometry, not arbitrary chroma/B/multi-CU combinations or full
Annex I conformance.

Texture IV can need a current-AU view component outside same-POC active RPL
membership; the native owner bridge supplies validated completed AU components
independently of active lists.

The AU-component owner bridge is now present. `ViewComponentPool` indexes
completed same-role sources by ViewIdx independently of active RPL membership,
retaining shared picture Arcs and the pinned current parameter binding. Source
NAL/status, role, AU, POC, completion, representation and source snapshots are
validated; missing/duplicate bindings, ambiguous views and later layers are
refused. Metadata is bounded and budgeted, with no copied frame planes.
`select_current_view_component` also checks the decoded picture's retained NAL
against its supplied status, preventing relabelled completion records.

The AU/DPB reconstruction owner retains Arc binding snapshots when texture IV
needs them, constructs the pool from completed AU pictures, deducts pool/index
metadata and passes it through `decode_layered_with_components` into the native
decoder. The entry rejects stale AU/POC and mismatched VPS/SPS/PPS snapshots.
Ordinary entry points retain their prior source behavior. Seven view-component
tests include exact Arc identity for components outside empty I-picture RPLs,
native reconstruction through the new entry, tight budgets, stale/duplicate/
missing/forged sources and changed current bindings.

Native texture IV/DI dispatch now uses this bridge. SPS admission enables
8-bit 4:2:0 texture `iv_di_mc` while depth-reference/VSP/residual/DBBP tools retain
their separate gates. Slice-bound `PreparedTexture` state feeds CU disparity
once per CU, using available native neighbours and saved IV fallback state.
The shared merge selector inserts IV/IVShift/DI/DIShift in normative order;
selected IV reconstructs its individual subblocks and publishes base/refined
state, while IVShift publishes the CU disparity with its shifted motion.
Fixed-size list scratch and bounded per-PB subblock scratch do not add another
frame store; retained IV-state allocation remains budgeted and conditional.
The pool indexes canonical non-auxiliary components only.

I-19 also keeps raw base-layer IV flags inert when NumRefListLayers is zero.
Both 8/10-bit base fixtures, including mid-GOP SPS updates, now preserve all
baseline pixels and timestamps; no-update SEI still forbids parameter changes.
The old parameter-refusal controls have been updated, with separate VSP refusal
fixtures retaining unsupported-tool coverage. All 45 SPS fixture artifacts are
offline reproducible; default AMP helper output remains byte-identical.

Final gates pass 287 HEVC unit tests (one explicit regeneration helper ignored)
and 44 integration tests across ten suites, with the texture-IV acceptance test
enabled. The expanded 30-movie texture-IV matrix also passes its three tests
without ignored cases. All gates are offline without FFmpeg/network. VSP,
refinement admission, additional profiles and broad tool-combination coverage
remain incomplete; this is not full codec coverage.

Native texture-IV B qualification adds 88 owned 32x32 movies with I0/I2/B1
decode order and I0/B1/I2 presentation. Sources are bi, L0-only, L1-only or
intra; target IV is enabled/disabled, merge indices span the initial/IV/DI/
DIShift cases, and weighted targets use distinct per-reference signed weights
and offsets. Independent scalar source/target luma oracles check every sample;
tests also check stored eight-aligned motion/reference/IV state, MP4 DTS/PTS,
layered and normal software output, rewind and five repeated native seeks.
Eight movies explicitly declare six merge candidates and select index 5.
All 90 artifacts regenerate byte-identically offline without test-time
generation, external codecs or private media.

This matrix exposed missing IV/DI NumExtraMergeCand handling. I-35/I-36 require
five_minus_max_num_merge_cand to be subtracted from 5+NumExtraMergeCand. The
decoder previously added the extra only for depth T. The corrected native
limit derives IV availability from potential NumRefListLayers, includes both
texture/depth IV and T (and the gated VSP condition), and preserves five for
ordinary/no-3D slices. Native reconstruction admission uses the same bound
limit. A specific pre-fix witness reads four candidates instead of the authored
five; after the fix it reads five, while the eight six-entry streams decode
their actual sixth candidate. P/depth-IV fixture authors now write the
normative offset; corresponding coded headers were regenerated while pixel
oracles remain unchanged. This corrects the previous author's shared limit
assumption rather than treating its old bytes as conformance evidence.

Texture B list remapping and weighted playback are now qualified by this
single-CU, neutral-chroma matrix. Multi-CU/queued texture IV, nonconstant
chroma combinations, depth-reference refinement, VSP and broader profile/tool
coverage remain unfinished.
The final targeted gate passes 34 integration tests across eight suites without
FFmpeg/network, alongside 287 HEVC unit tests (one regeneration helper ignored).

Texture-IV chroma qualification now adds 132 owned colored B movies at 32x32.
Source and target views have distinct nonconstant Cb/Cr patterns, including
wraps and picture-edge extension, so copying the source-view B picture cannot
stand in for predicting the target's own references. An independent integer
oracle implements H.265 8-241..8-261 filtering and final sample prediction;
actual motions exercise chroma phases 0/4 in the horizontal and vertical axes.
It checks all source/target Y/Cb/Cr samples, complete standard software packed
output, display order, rewind and five repeated native seeks per movie.

Forty-four cases signal explicit per-reference Cb/Cr weights (including zero
and negative weights) and distinct offsets. Parsed weights and midpoint-
corrected offsets are checked independently against the authored table (7-58),
and final pixels against 8-277. The other 88 cases retain default chroma weights
with unweighted or signed-weighted luma. Six-candidate, IV on/off, inter/intra
and bi/L0/L1 source controls remain present. The oracle includes 240 concrete
source positions distinguishing late-round/clip from prematurely rounding or
clipping each predictor. All 134 artifacts regenerate byte-identically offline,
without external codecs, private media or test-time generation.

This verifies nonconstant 8-bit 4:2:0 chroma for the specified single-CU B
combinations; arbitrary phase pairs, multi-CU/queued texture prediction,
refinement/VSP and full codec coverage remain open.
The final focused gate passes all five tests across the texture-IV P, B and
colored-B suites without ignored cases, offline and without FFmpeg/network.
