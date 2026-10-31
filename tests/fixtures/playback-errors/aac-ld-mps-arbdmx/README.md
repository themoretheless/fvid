# Mono arbitrary-downmix gain into native LD-MPS

Generate movies explicitly with
`python3 scripts/generate_aac_ld_mps_arbdmx_fixtures.py`.
Generate the separate component oracle explicitly with
`python3 scripts/generate_aac_ld_mps_arbdmx_reference.py <saoc-reference.zip>`.
Ordinary Rust tests use frozen outputs and need no generation, compiler for the
external C reference, network or FFmpeg.

60 own MP4/MKV pairs cover mono trees0/1 (5.1),23/9/4 parameter bands,unity and
time/frequency-varying gains. Five independent saved PCM controls come from own
qualified plain short ELD,downscaled LD-SBR and nominal48kHz-core LD-SBR corpora.
The spatial PCM interface uses32/64/128-band banks at12/24/48/96kHz.
All programmes contain12 source packets;no private media/parameters are copied.

Before the fix all five bare controls pass;three spatial tests refuse specifically
at arbitrary-downmix matrix construction. The fix applies decoded dB gains to M1
coefficients before temporal smoothing/interpolation. M2 and rendered PCM must
not be scaled as a substitute. The new matrix test compares1,209,600 endpoints
against original ISO5151/5152 C routines plus80-digit Decimal smoothing. The
separate input oracle and qualified spatial stages provide compositional PCM
acceptance,not an independent whole-decoder/profile conformance oracle.

Tests exercise timed/untimed native APIs,exports,MP4/MKV playback,packet/spatial
edge seeks,rewind/EOF,checkpoints and suffix-error retry. Truncated gain data
must retain the assembler and input cursor before valid retry. AudioWorker
checks source/queue stamps and live-history navigation.

This enables ordinary mono arbitrary-downmix gains only. TTT/stereo/multichannel
gain composition,HRTF,arbitrary-tree/downmix-residual extensions and full profile
conformance remain outside this acceptance. Existing ordinary paths retain
regression coverage. Shared-QMF LD-SBR ingress is not newly qualified here.
