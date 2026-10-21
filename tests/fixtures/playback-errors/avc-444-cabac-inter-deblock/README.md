# Owned full-resolution 4:4:4 CABAC component filtering

Generate: `python3 scripts/generate_avc_444_cabac_inter_deblock.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_cabac_inter_deblock`.

162 three-frame videos cross component depths 8/8,10/12,14/8, all-component,
Cb-only and Cr-only nonzero CBFs, mapped C QP offsets 0/0 or -6/+6, filter modes
0/1/2 and cabac_init_idc 0/1/2. Shared CBP=15 does not imply equal coefficients:
zero Y residual flags must not suppress filtering required by Cb or Cr. All
components use full-resolution edges, their own actual nonzero flags and QPs.

The author invokes the independent scalar source/transform/filter oracle from
the CAVLC matrix in a temporary directory; it uses the scalar samples only and
writes complete CABAC packets separately. No external decoder is an oracle.
`witnesses.json` requires visible changes for each active component/filter mode
and no changes to inactive components. A following P-skip samples the filtered
reference. Native and software MP4 output, crop, precision, clocks, reset and
rewind are checked offline without FFmpeg, network or private data.

Both macroblocks share a slice, so modes 0 and 2 agree here. This does not claim
cross-slice, field/MBAFF or every possible nonzero B-residual combination.
