# Owned full-resolution P field component filtering

Generate: `python3 scripts/generate_avc_444_field_inter_deblock.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_field_inter_deblock`.

432 four-field videos cross Y/C depths 8/8,10/12,14/8, CAVLC/CABAC init 0/1/2,
both arrival orders, all/Cb-only/Cr-only nonzero residuals, C QP offsets 0/0 or
-6/+6 and deblocking modes 0/1/2. The common CBP does not imply common nonzero
coefficients or QPs. Each component uses full-resolution edges in compact field
coordinates. Exact scalar bS=2 deblocking supplies independently calculated
woven samples. Enabled-filter witnesses require changes to active components;
inactive components must remain unmodified.

Native and software MP4 acceptance checks pixels, crop, precision, field state,
clocks, sample indexes, CABAC declarations, reset and rewind. All parameter sets
and samples are synthetic. No FFmpeg, system codec, network or private data is
needed. The two compact macroblocks share one slice; modes 0 and 2 agree here.
This is not a claim of cross-slice filtering or every motion-boundary strength.
