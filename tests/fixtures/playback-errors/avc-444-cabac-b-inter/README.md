# Owned full-resolution 4:4:4 CABAC B/direct and biprediction

Generate: `python3 scripts/generate_avc_444_cabac_b_inter.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_cabac_b_inter`.

216 I/P/B/P videos cross Y/C depths 8/8,10/12,14/8, two moving references,
temporal direct, B skip, spatial direct, explicit bi partitions, all three
biprediction modes and cabac_init_idc 0/1/2. Common motion and mode syntax is
shared; Cb/Cr retain full-resolution six-tap prediction and their own explicit
weights. `witnesses.json` requires temporal motion to differ from zero-motion
averaging. The scalar pixels are calculated independently of native decoding.

Native decode order is I/P/B/P; MP4 presentation must be [0,2,1,3]. Acceptance
checks exact coded/cropped pixels, mixed component depths, CABAC declarations,
clocks, reset and rewind. The final P-skip must still sample the future reference
rather than the non-reference B frame. All samples and parameters are synthetic;
no FFmpeg, system decoder, private source or network is involved.

This matrix has zero B residuals. Nonzero B residual combinations, MBAFF inter and other unrepresented tool combinations need their own qualification.
Field/PAFF acceptance is covered separately.


Field/PAFF matrices: `avc-444-field-inter`, its B/filter companions and
`avc-444-paff-inter`. These extend admission without changing this matrix scope.
