# Synthetic HEVC native-input edit timeline

`hevc-cuda-edit-repeat.mp4` derives only from the checked-in public synthetic
`tests/fixtures/hevc/main-ipb.mp4`. No private source bytes or parameters are used.
`scripts/generate_playback_error_samples.py` regenerates it without FFmpeg or
network access. Ordinary tests use the checked-in file and never run a generator.

The movie clock is 30 Hz and media clock 15360 Hz. Four edits contain a
0.1-second blank, a six-frame range starting at media tick 1024, a second
0.1-second blank, then the same range again. Output has 14 occurrences, ends at
media-clock tick 9216 (0.6 seconds), and each displayed picture lasts 512 ticks.

`owned_nvdec_hevc_mp4::tests::synthetic_hevc_edits_keep_blanks_and_repeated_b_picture_occurrences`
accepts the own input's exact timeline and qualifies every packet with the own
HEVC scheduler. This proves input/clock support; it does not prove NVIDIA pixel
output or HEVC movie-render/export support.
