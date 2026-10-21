# SPX noise reset, rewind and seek

Generate separately with `python3 scripts/generate_eac3_spx_noise.py`.
The generator uses owned bit writers, CRC and VP9/Matroska envelopes. It does not
read private media, run external codecs or access the network. Ordinary tests
read the checked-in artifacts and never generate them.

The 15 stereo cases cross blend codes 31/0/8/16/24 with absent, ordinary and
edge notch attenuation. Each is three identical six-block 48 kHz syncframes,
as elementary EC3 and a short VP9/Matroska video. Blend 31 is the signal-only
positive control; the other four blends introduce noise in at least one band.

`tests/eac3_spx_noise.rs` checks reset against a fresh decode, raw/Matroska
rewind, and audio-worker PCM/PTS with rewind and a 20 ms partial-packet seek on
all 15 cases. Stationary signal-only frames must repeat exactly, while noisy
frames must differ: this guards against reseeding at every normal frame boundary
or accidentally authoring noise-free fixtures. All paths require nonzero PCM.

Before the fix, the signal-only test passed but noisy reset, reader rewind and
worker rewind failed. `none-blend0` differed by up to 0.07922463 after reset;
the worker failed on generation 1, packet 0. The decoder reset ordinary dither
but left the separate SPX random generator advanced. Reset now restores its
initial seed while normal inter-frame playback retains its advancing sequence.

All 31 artifacts / 284,824 bytes regenerate identically with PATH empty and
process/socket creation denied (excluding this README). The shared authoring
helper's default blend remains 31; all 83 prior BSID-matrix artifacts regenerate
byte-identically, including the three existing SPX attenuation variants.

This is deterministic FVid reset/transport acceptance, not an assertion that
another decoder must choose the same random samples. It does not establish the
statistical energy/correlation of every SPX or zero-bit dither mode, or qualify
SPX+AHT/coupling combinations, other geometries/rates, hardware audio or speed.
