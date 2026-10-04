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
