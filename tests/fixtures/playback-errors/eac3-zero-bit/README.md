# Zero-bit dither flags in conventional and AHT streams

Generate separately with `python3 scripts/generate_eac3_zero_bit.py`.
The generator authors packets, CRC and short VP9/Matroska envelopes in process,
without private media, generated inputs, external codecs or network access.
Ordinary tests consume the checked-in media and never regenerate it.

There are 112 zero-bit cases crossing native/converted streams, exponents 8/12,
four full-band AHT masks and seven dither-flag patterns: off/on, left/right,
late/early enable and alternating channels. All SNR offsets are zero, forcing
zero allocation without estimating a masking threshold. Sixteen high-allocation
DC controls prove that disabling dither must not mute coded mantissas. Each case
has three six-block 48 kHz stereo frames, elementary and VP9/Matroska versions.

`tests/eac3_zero_bit.rs` requires exact zero after two adjacent disabled blocks;
the first disabled block must retain the previous block's overlap tail. Enabled
blocks/tails must remain nonzero. It also checks headers, raw/Matroska duration
and rewind, exact exponent scaling (four exponent steps reduce amplitude 16x),
and a fixed stereo correlation guard against accidentally sharing noise between
channels. The latter is a finite regression guard, not a universal statistical
certification. All 128 cases on both readers exercise audio-worker PCM/PTS,
speaker assignment, rewind and a 20 ms partial-first-packet seek.

Before the fix, conventional zero-bit handling, coded-signal controls, scaling,
correlation and transport passed, but AHT ignored disabled dither:
`a0-e8-t0-m1-off` had peak 0.08947197 where exact silence was required.
The decoder now retains first-allocation zero-bit markers for full-band AHT
planes and applies each block's dither flag when copying coefficients from the
six-block cache. It preserves the cache for later re-enabling and leaves coded
coefficients and ordinary overlap untouched. The marker storage is bounded by
the codec's five full-band planes and 256 coefficient bins.

All 257 artifacts / 4,830,852 bytes regenerate identically with PATH empty and
process/socket creation denied (excluding this README).

The optional `benches/check_eac3_zero_bit_reference.py` audit remains explicitly
non-green against FFmpeg 9.0.2: FVid is silent in 16/16 disabled-dither cases;
the reference is silent only in the four conventional cases. Its twelve AHT
cases emit noise, maximum peak 0.1175970658659935. No PCM tolerance or PRNG
alignment explains a disabled-noise violation, so the audit reports this
discrepancy and exits nonzero. FVid follows the dither-flag silence requirement
in [ETSI TS 102 366 6.3.4 and E.1.2.4](https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.03.01_60/ts_102366v010301p.pdf).
No reference process is part of ordinary tests or production playback.

This matrix covers uncoupled full-band zero allocation and scalar coded controls.
Mixed zero/VQ/GAQ allocation, zero-bit coupling/LFE, SPX interactions, other
rates/windows, hardware audio and performance remain separately unqualified.
