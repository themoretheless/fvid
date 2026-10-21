# Owned sample-clock movie construction

Run `python3 scripts/generate_matroska_audio_sample_clock_regressions.py` to
regenerate the six movies and manifest offline, without FFmpeg, encoders or
network. The generator writes twelve 16x16 neutral VP9 pictures at 20 ms
intervals using the owned bit writer and constructs Matroska elements directly.

Audio comes from independent frozen nonzero synthetic reference vectors in
`tests/fixtures/audio/matroska-sample-clock-references`. Coded packets and codec
configuration bytes remain identical to the original fixtures, as do original
packet clocks, CodecDelay, SeekPreRoll, DefaultDuration, block duration, padding
and reference metadata. No audio case was replaced with silence or PCM, and all
six original codecs/rates/channel layouts are retained. Only video/container
construction changed. This does not claim native audio encoding support.

`scripts/test_matroska_audio_sample_clock_regressions.py` checks reproducibility
with an absent output directory, empty PATH and forbidden subprocess/network,
plus byte-for-byte preservation of every encoded audio packet and clock field.
`tests/matroska_audio_sample_clocks.rs` exercises native audio decoding, exact
sample clocks, rewind and worker playback, including known ALAC frame sizes and
quantized container times. It also checks all twelve neutral RGB pictures,
their intervals, finite audio and retained nonzero signal.

FFmpeg reference encoding has moved to the explicit benchmark helper
`benches/generate_matroska_audio_sample_clock_reference.py`; ordinary tests and
fixture generation never execute it. Its default output is ignored benchmark
data, leaving the checked-in reference vectors intact.
