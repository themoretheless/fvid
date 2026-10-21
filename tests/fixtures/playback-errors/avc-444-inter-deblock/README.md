# Nonseparate 4:4:4 inter component filtering

Generate: `python3 scripts/generate_avc_444_inter_deblock.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_inter_deblock`.

54 synthetic three-frame videos cross Y/C depths 8/8,10/12,14/8, all-component,
Cb-only and Cr-only nonzero coefficients, C QP offsets 0/0 and -6/+6, and filter
modes 0/1/2. The shared coded-block pattern does not imply equal nonzero flags:
Y may carry all-zero residual syntax while Cb or Cr requires bS=2 filtering.
Chroma QPs are mapped independently (20/31 for offsets -6/+6 at QPY=26).
All three planes use full-resolution luma-style edges and sample precision.

An independent scalar edge filter supplies exact pixels. `witnesses.json`
requires changed samples for each active component and enabled filter mode;
inactive components must remain unchanged. Native decode and software MP4,
crop, timestamps, reset and rewind are acceptance gates. The following P-skip
samples the filtered reference to qualify DPB publication. The two macroblocks
share a slice, so modes 0 and 2 agree here; this is not cross-slice qualification.

Only owned samples and parameter sets are stored. No private media, external
codec, FFmpeg or network is used during generation or ordinary tests.
