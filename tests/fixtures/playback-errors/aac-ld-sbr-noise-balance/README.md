# Coupled LD-SBR noise-balance regression

Four wholly authored eight-AU stereo programmes: 480/512 core samples,
centered controls and asymmetric noise balance. MP4 contains a short synthetic
video; Matroska is an audio transport control. No private or ISO media,
frames, audio or codec parameter sets were copied.

Run `python3 scripts/generate_aac_ld_sbr_noise_balance_fixtures.py` explicitly.
Ordinary tests use frozen files and require neither FFmpeg nor network access.

AAC 4.6.18.3.5 defines noise left = 2^(7-Q0)/(1+2^(12-Q1)) and
right = 2^(7-Q0)/(1+2^(Q1-12)). The authored asymmetric case uses Q0=7,
Q1=8: left 1/17, right 16/17. The old decoder reversed these ratios.
The centered control passes before and after; asymmetric PCM fails before
(error 0.000047067264) and passes after. This is an acceptance test, not a
successful refusal. Expected PCM comes from direct cosine/window and CLDFB
convolution plus directly evaluated gain/noise fractions, without a decoder.
