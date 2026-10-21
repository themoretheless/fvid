# Enhanced coupling zero-bit noise acceptance

224 owned cases: independent/converted streams, conventional/AHT coupling,
exponents 8/12, enhanced begin codes 0/8, explicit/default band grouping, and
seven per-channel/block dither patterns. Amplitude codes cover all 32 values.
Each case includes three raw audio frames, a short owned VP9/Matroska video,
and a one-frame companion with every coupling amplitude set to unity.

Regenerate separately: `python3 scripts/generate_eac3_enhanced_noise.py`.
No FFmpeg, subprocess or network is required. Private source media is absent.

`tests/eac3_enhanced_noise.rs` accepts silence after overlap, re-enabling,
exact exponent scaling, independent stereo noise, raw/Matroska rewind and
worker PCM/PTS with 20 ms partial-packet seek. The private decoder regression
checks actual enhanced geometry and zero coupling allocation. For every
coupled bin the decoded coefficient must equal the unity companion times the
authored amplitude. Low bands must remain identical; disabled dither and
amplitude code 31 must produce true zero. Correlation is checked before
transform on the unity companions, avoiding low-band noise masking a shared
coupling vector. The separate normative amplitude unit uses literal Q12 values.

Current production reconstruction passes without a new decoder fix. This
qualifies fixed-geometry stereo enhanced noise; it does not qualify all begin
codes, enabled-noise mixtures with coded bins, all tool transitions, dependent
presentations, or the separately open AHT/LFE zero-bit policy.
