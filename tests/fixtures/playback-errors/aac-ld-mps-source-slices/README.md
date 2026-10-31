# Spatial frames crossing source windows

448 authored twelve-core-packet programmes in MP4 and Matroska,112 unchanged
fixed-frame controls and42 saved input references (30 analytical PCM,12
independently authored spatial-bank LD-SBR QMF). Four late malformed movies
exercise failure/retry with existing partial output or a batch of queued frames.
No private source media,frames,audio or codec parameter sets.

All eight trees,480/512 LD/ELD cores,PCM32/64/128 and shared-QMF32/64 ingress:

- staggered headers leave part of an original source window across calls;
- batched headers render many partial windows in a single transaction;
- initial SSCs include both sub-core and multi-core frames;
- straddling frames contain a partial head,several whole sources and a tail.

Each source span records its offset,full source size and original unsliced
stamp. Native AAC assembles a complete submitted source window before exposing
it to container edits/padding. The ordinary Session opaque-ID API retains its
strict whole-chunk contract;the native reassembler explicitly enables slices.
The new metadata fields describe caller-submitted chunks,not inferred IDs.

Direct component composition of saved PCM/QMF bypasses the native scheduler
and reassembler. The remaining DSP is the same owned implementation,so this is
component/association evidence,not independent whole-codec PCM truth. Staggered,
batched and straddling streams also match independently decodable unchanged
fixed-frame controls exactly after their common initial frame. Initial-SSC
streams use direct composition because their initial interpolation differs.

Shared LD-SBR bank selection now depends on actual rate/mode rather than SSC
frame length. Matched-clock long initial SSCs therefore retain the LD-QMF
interface instead of the former PCM-interface fallback. Frozen spatial-bank
SBR QMF qualifies that selection;unmatched clocks keep the PCM interface.

Checks include source windows in media clocks and opaque IDs near i64::MAX,
zero-tick slices at100 Hz,checkpoint replay,reset with partial PCM retained,
MP4/MKV seek/rewind and audio-worker navigation. A library unit checks exact
retained allocation capacity,completion and reset of the partial-window buffer.
The two older frame-length movies that omit final parameters now fail at EOF,
not at a codec geometry refusal. Their binary movies remain unchanged.

Generate explicitly:
`python3 scripts/generate_aac_ld_mps_source_slices_fixtures.py`.
Tests read frozen artifacts without generation,FFmpeg,network or external
reference executables. The old and new generators are audited separately from
ordinary test execution. No physical audio,realtime performance or complete
ISO23003-2 Annex B/profile conformance claim.
