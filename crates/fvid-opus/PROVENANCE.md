# Native Opus core provenance

Source: https://github.com/stephenberry/opus-pure
Upstream revision: cf954a76381ae8968b2bc35bb5d578b3a2311d53 (0.2.2)
Imported on 2026-10-01. Original copyrights and BSD-3-Clause terms are retained
in LICENSE and ATTRIBUTION.md. This is adapted upstream code, not original FVid
codec authorship. The upstream README describes its full API; FVid exposes only
its own playback adapter.

This local crate has no C, libopus, FFmpeg, system library or network dependency.
SIMD cfg branches are disabled to retain the scalar safe
Rust path under unsafe_code=forbid. Runtime playback uses the decoder only.

FVid adaptations also expose the OpusHead parser, check equal durations across
multistream packets, adjust the scalar TDAC test dispatch, and update Rust crate
paths in API examples. The original code and upstream API documentation are
retained alongside these changes.
