# LD-SBR inverse filtering and strict noise-range refusal

Generate explicitly with `python3 scripts/generate_aac_eld_sbr_noise_range_fixtures.py`.
Ordinary tests never run generators or use FFmpeg/network access.

Two eight-AU stereo controls use480/512 core samples,dual-rate LD-SBR/CRC and
inverse-filter modes0/1/2/3. Their AAC/SBR bytes are regenerated and asserted
identical to the first8 AUs of existing own `aac-eld-sbr/n{480,512}-c2-invf`
programmes. Frozen independent direct CLDFB/HF/noise PCM is reused unchanged.
Each control has a counterexample changing only fourth-frame absolute noise
scalefactors from7 to31 and regenerating CRC. Both have MP4 and Matroska movies.
No public ISO/private audio,frames or codec parameter sets are copied.

These are strict refusal regressions, not Q31 acceptance. ISO/IEC14496-3:2009
section4.6.18.3.6 constrains reconstructed level noise scalefactors to0..30.
The own Q31 inputs reach that specific dequantization error after valid CRC,
core and inverse-filter syntax. Repeated late rejection retains all histories;
retrying the corresponding valid AU reproduces the uninterrupted PCM exactly.
Valid controls additionally check frozen PCM,playback,live-history seeks,rewind
and timestamp progression in both containers.

The optional public ISO audit finds Q31 in all six `er_eld_sbr_i` streams.
Their unresolved strict-admission failures must not be counted as acceptance
or as proof of a supported missing valid noise range. Independently pinned FDK
limits reconstructed noise levels to0..35; that is a different robustness policy,
not proof that0..35 is the normative range. No clamp or guessed widening is
introduced in this milestone.
