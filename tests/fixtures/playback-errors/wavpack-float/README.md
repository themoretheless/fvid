# Owned WavPack IEEE-754 restoration

Generate separately with `scripts/generate_wavpack_float.py`. Ordinary tests read
checked-in artifacts only. No external codec/process/network, private media or
previous generated fixture is used. The shared owned helpers construct literal
zero-run entropy and power-of-two predictor history; expected IEEE words use an
independent closed-form exponent/mantissa calculation and explicit bit patterns.

180 accepted 60 ms float videos cover ordinary unequal stereo and mono-coded
stereo (with separate raw mono controls), maximum exponents 0/1/8/24/25/127/150/254,
float shifts 0/8, zero/one/same/literal mantissa restoration and legacy/optimized
WVX streams. Optimized cases vary minimum shifted zeros and maximum shifted ones.
Explicit vectors include signed zero, minimum/maximum subnormals, small normal
values, signed Inf, quiet/signaling NaNs and distinct NaN payloads. Normalization
hints vary; the lossless decoder preserves original IEEE words, not an optional
normalized output. Comparisons use bytes, not floating-point equality/tolerance.

Four further videos qualify optimized INT32 maximum-width parsing, reading all,
some or none of the literal low bits, with/without duplicate-bit restoration.
This reader shares the new WVX metadata dispatch with floating point.

All 184 accepted videos check every raw and Matroska PCM byte through reset,
rewind, EOF and sample-exact 35 ms seek. Neutral owned VP9 is decoded in the same
container. The headless audio worker verifies finite float PCM, PTS and endpoint
through playback/rewind/seek. Non-finite values are qualified at the decoder and
container PCM interfaces; physical device output is not asserted.

Ten malformed packets and matching short videos preserve specific CRC,
truncation, padding, trailing-word, metadata length/presence, conflicting storage
flags, unsafe shift and mixed-WVX-format refusals. Error handling is checked
before a subsequent good block and after reset.

On pre-fix source two acceptance probes fail with the intended reasons:
`floating-point block is not supported` and `sends bits it has no stream for`.
The final source passes four new integration tests. All 1,217 generated files
(6,110,212 bytes), excluding this README, regenerate identically with PATH empty
and external commands/network disabled.

This finite matrix does not prove complete WavPack: hybrid/correction, DSD,
multichannel, other unqualified tools/metadata and combinations remain open.

Primary references:
- https://github.com/dbry/WavPack/blob/master/src/unpack_floats.c
- https://github.com/dbry/WavPack/blob/master/src/pack_floats.c
- https://github.com/dbry/WavPack/blob/master/src/open_utils.c
- https://github.com/dbry/WavPack/blob/master/include/wavpack.h
