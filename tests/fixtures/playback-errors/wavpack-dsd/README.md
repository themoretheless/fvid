# Owned WavPack DSD playback fixtures

Generate separately with `python3 scripts/generate_wavpack_dsd.py`.
The generator reads no private source, generated artifact, external codec,
process or network. Ordinary tests consume these checked-in vectors directly.

256 accepted short VP9/WavPack videos cover all three defined DSD coding modes:
raw bytes, fast byte-context range coding and high adaptive bitwise coding.
Fast cases include verbatim and sparse RLE probability tables and 0/1/5 history
bits. High cases include initial table rates 0/7/255 and signed factors.
Mono/stereo/false stereo and six-channel frames have literal interleaved MSB-first
DSD-byte controls plus independent scalar 56-tap PCM controls. The matrix uses
byte-frame rates 44,100/352,800/705,600 Hz (eight DSD bits per byte frame),
silence/square/ramp/noise signals and block lengths 1/7/15/16/128. The 64 extra
short-block videos exercise the startup extrapolation threshold and filter state
across three packets.

Tests check raw DSD bytes and rate, exact float PCM, compact Matroska, neutral
video, EOF, reset/rewind, partial-block seek, presentation and headless audio
worker windows. A failed final component must preserve both the uncommitted
speaker layout and the already-advanced FIR history. Fourteen malformed videos
exercise metadata/flags, multiplier/timebase overflow, sample count, high-mode
rate/truncation, fast-mode context/table bounds and CRC refusal. This is a finite
matrix, not exhaustive malformed-input or physical-device qualification.

There are 2,335 generated artifacts totaling 8,227,770 bytes, excluding this
README. The pre-change raw and container acceptance probes both reached the
specific `WavPack DSD block is not supported` gate.

## Independent reference

The optional `benches/wavpack_dsd_reference.c` executable is compiled against a
separate WavPack checkout at `c9515833d1867d9ec03774d954369107f2e88a73` with DSD
enabled. `benches/check_wavpack_dsd_reference.py` takes `--oracle`,
`--reference-commit` and `--report`. It compares native DSD and 24-bit PCM output
with reference CRC checking enabled. All 256 cases match byte-for-byte in both
modes with zero CRC errors.

PCM calls use the same packet-sized windows as the FVid tests. The reference
startup extrapolation depends on the first call's visible frame count; a single
large call across several tiny packets is not the same startup policy. Floating
point operation widths, including double precision before the final startup
integer cast, are retained explicitly.

DSD seek currently replays from the beginning to restore exact filter history.
Bounded-preroll acceleration, physical DSD/device output, arbitrary block/window
combinations and all other WavPack tools remain outside this acceptance matrix.
The external reference is not a production, generator or ordinary-test dependency.
