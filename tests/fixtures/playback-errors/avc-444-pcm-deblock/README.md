# Nonseparate 4:4:4 PCM loop filtering

Generate separately with `python3 scripts/generate_avc_444_pcm_deblock.py`.
Run `cargo test --offline --no-default-features --test avc_444_pcm_deblock`.

54 owned progressive 32x32 High 4:4:4 Predictive level 4.1 videos cover PCM
macroblocks in CAVLC and CABAC, Y/C depths 8/8, 10/12 and 14/8, independent
Cb/Cr QP offsets +/-12 and filter offsets +12, and filter IDC 0/1/2. PCM's
QPY=0 keeps Y unchanged, while mapped chroma QP and positive offsets make
internal Cb/Cr edges require real filtering. Each component has 20064 changed
sample witnesses across the matrix; IDC=1 controls must remain unchanged.

The baseline decoder returned unfiltered Cb for `y8-c8-o12-12-m0-e0`; the
regression compares actual sample arrays and fails on that specific mismatch.
Acceptance checks all coded planes, cropped native-depth software MP4 bytes,
PTS/duration/index, decoder reset and reader rewind. A separate resource test
requires filter metadata to be charged beyond the plane-only decode budget,
while preserving that budget for disabled filtering.

The scalar oracle is separate from native Rust filtering. No FFmpeg, network,
private media, frames, audio or copied parameter sets are used. This matrix
qualifies progressive PCM, not compressed CABAC or field/MBAFF filtering.

Final verification on 2026-10-21 passed all 54 CAVLC/CABAC samples plus the
plane-only budget regression. Prior exact PCM budgets with filtering disabled
continue to pass. Mixed shared-slice PCM/I16 cases are in
`../avc-444-mixed-deblock`.

Reauthoring verification on 2026-10-21 reproduced every generated artifact
byte for byte. Ordinary tests read these files without invoking the generator.
