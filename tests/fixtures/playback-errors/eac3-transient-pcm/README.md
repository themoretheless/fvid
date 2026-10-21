# Standalone E-AC-3 transient PCM correction

Regenerate separately: `python3 scripts/generate_eac3_transient_pcm.py`.
A 6144-sample bounded synthetic waveform is saved as f32 and as the audio of a
neutral VP9 Matroska video. Twenty-four independent Python scalar outputs cover
two absolute buffer origins (including retained history), three coding-block
phases and processing lengths 0/1/128/255. The oracle snapshots source PCM before
crossfades and uses complementary half-Hann windows with exact endpoint weights.
Its f64 arithmetic is independently rounded once to f32; Rust uses f32 weights
and operations. The fixed 1e-6 absolute tolerance is a conservative rounding
allowance for the input bounded by 0.5, not fitted to measured output errors.

`tests/eac3_transient_pcm.rs` reads the actual video PCM, compares every corrected
sample with the scalar oracle, requires a real change, and checks all samples
outside the corrected interval bit-exactly. Missing history/future samples and
clock overflow must fail without any mutation. Overlapping source/destination
ranges require snapshotting and are included in the valid cases.

27 artifacts / 672,079 bytes excluding README reproduce exactly with process
and socket construction denied. No private media, encoder, FFmpeg or network;
generation stays separate from ordinary tests. Normative basis: ETSI TS 102 366
V1.4.1 E.2.7.2:
https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.04.01_60/ts_102366v010401p.pdf

This qualifies ONLY the standalone single-channel PCM operation in
`eac3_decoder::transient::apply_window`. `decoder_integration_accepted=false` in
the manifest is intentional. The E-AC-3 decoding pipeline still does not call
this operation. Streaming history/withheld output, frame-spanning events,
priming/window timestamps, reset/seek and correction before channel folding
remain required before claiming decoder feature support.
