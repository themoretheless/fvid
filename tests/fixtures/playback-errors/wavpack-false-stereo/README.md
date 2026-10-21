# WavPack mono-coded stereo and Matroska mapping

Generate separately with `scripts/generate_wavpack_false_stereo.py`.
Ordinary tests read checked-in artifacts only. No external codec, subprocess,
network, private samples/parameters or existing encoded fixture is used.

99 accepted 60 ms videos contain owned neutral VP9 and nonzero WavPack audio.
Zero-run residuals are combined with exactly represented power-of-two predictor
history. Independent scalar recurrences and integer fixups generate every PCM
sample, including signed values, rather than reading decoder output.

Coverage: 8/16/24/32-bit storage, terms 1/2/8/17/18, positive/negative weights,
standard/custom rates (8 kHz, 48 kHz, 37.5 kHz), header shifts, larger magnitudes,
INT32 zero/one/duplicate restoration, explicit low bits and WVX CRC/padding.
Each false-stereo packet has an independently authored mono control; a separate
stream switches false stereo -> unequal ordinary stereo -> false stereo.
The latter confirms that coded-channel count belongs to the block, not stream
state. PCM duplication happens only after mono CRC and integer reconstruction.

Matroska packets use the specified 12-byte compact header and two-byte version
in CodecPrivate, not a private full-wvpk shortcut. Tests decode video and audio,
check every PCM byte, EOF, rewind and sample-exact seek at 35 ms. Audio worker
capture checks PCM, timestamps and endpoint through playback, rewind and seek.
Six malformed packets and corresponding videos check contradictory mono flags,
main/WVX CRC corruption, non-one padding, extra trailing words and truncation.
Wrong output channels, setup version/length and track rate are refused too.

Pre-fix source fails three independent acceptance probes with the intended
reasons: unsupported dual-mono, unread extra bits, and unavailable Matroska
A_WAVPACK4 track. The final source passes five new automated tests.
802 generated files (2,838,791 bytes), excluding this README, regenerate with
PATH empty and external commands/network denied.

This qualifies the stated integer lossless mono/stereo matrix. Hybrid, float,
DSD, multichannel, additional metadata/tools and arbitrary signal combinations
remain outside acceptance. It does not prove complete WavPack or all codecs.

Primary references:
- https://www.wavpack.com/WavPack5FileFormat.pdf
- https://github.com/dbry/WavPack/blob/master/src/unpack.c
- https://github.com/dbry/WavPack/blob/master/src/pack.c
- https://www.matroska.org/technical/codec_specs.html#wavpack
