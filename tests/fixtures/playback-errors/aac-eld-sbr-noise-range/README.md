# LD-SBR inverse filtering and explicit strict noise-range refusal

Generate explicitly with `python3 scripts/generate_aac_eld_sbr_noise_range_fixtures.py`.
Ordinary tests never run generators or use FFmpeg/network access.

Two eight-AU stereo480/512 controls use dual-rate LD-SBR/CRC and inverse-filter
modes0/1/2/3. Their AAC/SBR bits equal the first8 AUs of existing own
`aac-eld-sbr/n{480,512}-c2-invf` programmes;frozen independent direct CLDFB/HF/noise
PCM is reused. Counterexamples change fourth-frame noise absolute values from7
to31 and regenerate CRC. Both have MP4 and Matroska movies. No private/ISO
media,frames,audio or codec parameter sets are copied.

Current behavior:these unchanged Q31 movies are accepted by default LD-SBR.
The sibling `aac-ld-sbr-q31` acceptance test compares both containers with its
independent Q31 PCM oracle. This directory's `strict_errors` metadata instead
reproduces the explicitly selected `NoisePolicy::Strict` refusal. Repeated
late strict rejection retains histories for a valid retry;valid controls also
check playback/live-history seek/rewind and timestamps. The previous generic
refusal expectation has been updated rather than counting it as acceptance.

ISO/IEC14496-3:2009 section4.6.18.3.6 constrains reconstructed noise to0..30.
Default LD-SBR additionally reconstructs original ISO streams' Q31 exactly as
an interoperability policy,without calling it normative or clamping it. The
strict standalone/legacy SBR helpers retain their original bounds;coupled
balance restrictions are also unchanged. Q32 refusal is in the sibling corpus.
