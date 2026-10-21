# Zero-bit coupling acceptance

448 owned cases: exponents 8/12, independent/converted streams, coupling
ranges 37..73 without phase inversion and 61..85 with phase flags, all eight
AHT plane masks, and seven per-block/per-channel dither patterns.

Regenerate separately: `python3 scripts/generate_eac3_coupling_zero.py`.
The generator needs neither FFmpeg nor network access. Fixtures use three
4096-byte audio frames and a synthetic VP9/Matroska video variant.

`tests/eac3_coupling_zero.rs` checks disabled-dither silence after overlap,
noise re-enabling, exact 16x exponent scaling, stereo correlation, raw and
Matroska rewind, and worker PCM/PTS with a 20 ms partial-packet seek.
The private decoder regression checks actual zero allocation in the coupling
range and per-channel spectra before the inverse transform; disabled channels
must be exactly zero and enabled channels must not share one noise vector.
This prevents uncoupled low-band noise from hiding broken coupling behavior.

These are passing acceptance tests with the current reconstruction, not a new
decoder fix or proof of all coupling tools. AHT/LFE zero-bit policy, enhanced
coupling noise and mixed high-allocation/noise spectra remain unqualified.
