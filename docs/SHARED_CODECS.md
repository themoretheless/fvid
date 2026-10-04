# Shared owned compressed-video kernels

`fvid-codecs` compiles the canonical AVC, HEVC, VP9 and AV1 source modules once. The root crate reexports these modules; `fvid-media::owned_codecs` exposes the same public types. Shared error and fallible allocation primitives live in `fvid-control`. Canonical source files remain in this monorepo; this change does not qualify standalone package publication. The player feature retains HEVC worker priority through `fvid-platform`.

Plain WebM VP9/AV1 requests now dispatch through these kernels in both native-only and legacy-enabled fvid-media builds. AV1 configuration OBUs initialize the decoder before packets, including files whose first packet has no sequence header. Malformed packets propagate errors; explicitly unsupported tools remain capability refusals. The native-only API refuses transformed compressed sources rather than silently dropping options.

This is a migration step, not complete FFmpeg independence. MP4 interior-empty-edit and rotation workflows, transformed compressed-video workflows, remaining exports and codec/profile gaps still require migration. Root media and CUDA features still enable legacy-ffmpeg. Presence in the capability inventory denotes linked packet-decoder components, not acceptance of every container workflow or profile.

Verification: shared-kernel unit tests, root and standalone media unit tests, native deband/perspective regressions, eleven shared-codec integration tests and the offline native dependency guard. Ordinary tests require neither FFmpeg nor network access.

## MP4 dispatch

Plain MP4 AVC/HEVC without rotation now uses the shared packet decoders in fvid-media. AVC decode-order output includes B pictures; HEVC counts only pictures marked for output and reports cropped dimensions and source depth. No FFmpeg API is called on this admitted path. Interior empty edits and transforms remain explicit refusals in native-only builds, and retain their existing legacy route in legacy-enabled builds. This preserves the scope boundary without silently discarding presentation timing or requested filters.

Four short synthetic MP4 fixtures exercise AVC baseline/B pictures and HEVC Main/Main10. The pure Python generator replaces complete edit containers with same-size free boxes, preserving every packet offset. Tests compare native root and standalone frame count, dimensions and pixel format; edit-list acceptance tests replace the old refusal, while interior empty ranges and transforms remain explicitly unadmitted.

## Shared MP4 presentation mapping

`owned_video_timeline` owns the mapping used by both the root player and standalone media decoder. Rate-one media edits use cumulative ceil rounding in track ticks, preserving fractional endpoints without accumulating per-edit rounding drift. Leading empty edits retain the root player's existing start-at-first-picture policy. An empty edit inside playback remains an explicit unsupported capability until a blank-frame policy is implemented.

The stats decoder preserves codec reference reconstruction by decoding access units in decode order, counting a visible picture once for every overlapping media edit. Repeated and disjoint ranges therefore preserve their displayed frame count, with partial frames included at half-open boundaries. It reconstructs the source once rather than retaining the decoded pixel sequence or decoding each repeated range again. This change does not provide a compressed-frame transform/export visitor or qualify universal profile support.

Equal-PTS pictures remain codec references but yield one displayed picture, using the latest decoded picture metadata. Presentation intervals follow sorted PTS; the terminal duplicate group retains the sum of its nominal durations. The stats decoder keeps only presentation metadata, not the decoded frame sequence, and visits edits in movie order. Existing synthetic duplicate-PTS fixtures reproduce the old incorrect count (12 instead of 11); updated acceptance tests cover single duplicates, duplicate runs and a group spanning the entire clip.
