# Shared owned compressed-video kernels

`fvid-codecs` compiles the canonical AVC, HEVC, VP9 and AV1 source modules once. The root crate reexports these modules; `fvid-media::owned_codecs` exposes the same public types. Shared error and fallible allocation primitives live in `fvid-control`. Canonical source files remain in this monorepo; this change does not qualify standalone package publication. The player feature retains HEVC worker priority through `fvid-platform`.

Plain WebM VP9/AV1 requests now dispatch through these kernels in both native-only and legacy-enabled fvid-media builds. AV1 configuration OBUs initialize the decoder before packets, including files whose first packet has no sequence header. Malformed packets propagate errors; explicitly unsupported tools remain capability refusals. WebM transforms now use the shared owned streaming pipeline; unsupported container workflows remain explicit capability refusals.

This is a migration step, not complete FFmpeg independence. MP4 interior-empty-edit, rotation and multitrack workflows, remaining exports and codec/profile gaps still require migration. Root media and CUDA features still enable legacy-ffmpeg. Presence in the capability inventory denotes linked packet-decoder components, not acceptance of every container workflow or profile.

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
