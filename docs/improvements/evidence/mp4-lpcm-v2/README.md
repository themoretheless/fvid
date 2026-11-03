# Packed QuickTime LPCM version 2 — qualification

Base: ff7cbc844604c8653450a37e9ffdba1766d1b46a.

The parser previously skipped every `lpcm` sound description. The final unchanged five-test stand on the original implementation passes legacy PCM and actual neutral-video controls, then fails four LPCM acceptance/header/export tests. `final-baseline.json` records the original/candidate/test source hashes and guarded restoration; `final-baseline.log` contains the specific missing-audio refusals. Earlier before/after logs document smaller authoring stages and are not the final qualification. test-authoring-error.log records an incorrect new-test API name, corrected without changing product behavior or counting an improvement.

Apple defines version 2 with an explicit floating-point sample rate, channel count, bits, layout flags and constant packet geometry:
https://developer.apple.com/documentation/quicktime-file-format/sound_sample_description_version_2

The implementation validates the structure extent, marker, finite/in-range rate, channels and constant byte/frame geometry. It resolves supported packed/interleaved LPCM into existing PCM decoder descriptions: signed8/16/24/32, unsigned8, float32/64, both byte orders. Integral high rates are retained without the old 16.16 limit. Packet payload bytes are unchanged. Grouped three-frame packets and one-frame packets use the existing PCM index/window/seek paths.

Public Track/SampleDescription codec values for newly supported LPCM are normalized to sowt/twos/raw/fl32/fl64, with existing float byte-order metadata. Consumers requiring the original source fourcc must inspect the original stsd; previously these entries were omitted. Existing supported formats/interfaces are unchanged. Fractional rates, planar/unpacked layouts, unknown flags and other unsupported sample representations remain skippable descriptions, not accepted playback. Packed full-width aligned-high/nonmixable flags do not alter sample storage.

The generator separately authors 112 short cases, 28 legacy controls, analytic f32/f64 waveforms and malformed/unsupported headers. Float64 includes low bits absent from float32. Ordinary regression tests use no external encoder or network. They verify native PCM and rational clocks, precise seek/rewind/EOF/MAX,336 actual software-worker sessions per run, exact owned f32/f64 full/interval exports, typed specific malformed refusals and preserved unsupported-layout handling. A separate optional local FFmpeg reference matches all224 f32/f64 outputs exactly. It is not invoked by tests.

No performance, physical audio-device, general LPCM-layout or broad MP4-format completion claim. Accepted as FVID-0534, batch545:156 passing Rust test executions, two old physical-device ignores, core/player checks and three5-test external-tool-free repeats. All262 files regenerate byte-identically and4656 source/fixture hashes remain stable. See docs/improvements/batch-545-validation.json.
