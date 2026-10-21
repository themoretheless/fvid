# HEVC received zero CLL across independent sequences

Regenerate explicitly with `python3 scripts/generate_hevc_hdr_zero.py`.
The generator extends the owned `hevc-hdr-seek` layout with MaxCLL/MaxFALL 0/0
in the second independent coded sequence. Mastering display declarations remain
distinct and nonzero in all three sequences. Each sequence retains the original
three synthetic pictures from `hevc-pcm-reference-rext8`; no private media,
frames, audio or parameter sets are copied. All 21 artifacts regenerate
byte-identically with empty PATH and subprocess/socket creation denied.
Ordinary Rust tests only read saved artifacts and need no FFmpeg/network.

Nine movies cover hvc1, hev1 and Matroska, with inline, combined-prefix and
split-prefix SEI declarations. Complete static HDR is declared before each
sequence's first IDR and is unchanged within that sequence. CLL messages for
one CLVS have identical content; zero means no stated upper bound, not reuse
of the prior CLVS's bound (ITU-T H.274 8.10.2):
https://www.itu.int/rec/T-REC-H.274-202309-I/en

Baseline decoder priming/configuration override and both container readers
retained MaxCLL 1000 instead of 0 for the second sequence. The acceptance tests
require all exact source pixels, mastering display fields and light values on
every output, including lookahead, sequential/rewind EOF and repeated
forward/backward seeks. Separate controls verify public NAL extraction, the
observation path used by hardware adapters, zero recorded in hvcC/reset,
absence-versus-zero fallback, and atomic refusal after incomplete PPS.
Testing observation does not constitute hardware playback acceptance.

The fix preserves received CLL presence separately from numeric values, updates
all three decoder SEI ingestion paths, and prevents fallback over received zero
light. Parsing a valid zero CLL preserves unrelated mastering metadata.
This is static received-CLL qualification, not complete codec/HDR coverage.
