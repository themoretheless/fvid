# Canonical LD-SBR analysis phase regression

Eight authored eight-AU programmes cover 480/512 cores, mono/stereo, CRC and
single/dual-rate LD-SBR in MP4 and Matroska. All core spectra and codec parameters
are generated locally. No private or ISO media is included.

The nominal CLDFB analysis used 2n-96 instead of the canonical 2n-95 in
ISO/IEC 14496-3:2009, 4.6.19.4.1. The centered factor-one bank already used 2n-95.
Before the fix both new tests fail: a complex analysis-bin mismatch and a
0.00013408763 maximum PCM error on n480-c1-x1. The PCM test is acceptance, not
an expected refusal. Its independent direct-convolution/cosine oracle uses
canonical analysis, HF reconstruction, noise and CLDFB synthesis.

Regenerate separately with `python3 scripts/generate_aac_eld_cldfb_phase_fixtures.py`.
Ordinary tests consume frozen assets, without generation, FFmpeg or network.
The actual worker test also checks playback, seeks, rewind and timestamps.
