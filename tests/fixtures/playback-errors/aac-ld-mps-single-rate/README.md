# AAC LD-SBR/MPS single-rate synthesis regression

Eight wholly authored eight-AU programmes: 480/512, single/dual rate,
constant gain / in-band gain-reset epochs. Each has MP4 synthetic video and
Matroska audio. No private or ISO media/frames/audio/codec parameter sets.

Explicit generation: `python3 scripts/generate_aac_ld_mps_single_rate_fixtures.py`.
Ordinary tests use frozen files without FFmpeg, C compilers, foreign decoders or
network. Golden PCM uses own core cosine/window superposition, LD analysis,
HF/noise/gain equations and absolute-time synthesis convolution. Own tree212
CLD0dB/ICC1 gives two equal outputs; M1/M2 initial interpolation yields alpha².
Gain epochs at AU3/6 clear only synthesis history while the SBR signal continues.

The shared AAC single-rate LD-QMF synthesis uses phase2n-31, while the generic
standalone MPS32 PCM bank uses2n-127. Dual-rate64 uses2n-255 in both contexts.
The archived AAC reference InitSbrSynFilterbank independently establishes this
context distinction. No phase/delay/gain is fitted to source media.

Before: all eight videos decode successfully; four dual-rate controls match,
four single-rate cases fail the specific waveform acceptance. After: all eight
match independent PCM byte-for-byte. Tests also cover both transports, sample
seek/rewind, checkpoints/reset, both renderers/frontends and gain reconfiguration.
