# Nonseparate 4:4:4 CAVLC B/direct and biprediction

Generate: `python3 scripts/generate_avc_444_b_inter.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_b_inter`.

72 owned I/P/B/P videos cross depths Y/C = 8/8,10/12,14/8, two signed moving
reference vectors, temporal direct, B skip, spatial direct, explicit bi partitions,
and unweighted/explicit/implicit biprediction. One shared motion field serves
three full-resolution component predictors; each component uses its own depth
and explicit weighting values. The scalar oracle performs nested six-tap
interpolation from the original and previously reconstructed future reference.
`witnesses.json` requires temporal motion to differ from zero-motion averaging.

Native decode order is I/P/B/P; MP4 presentation is samples [0,2,1,3]. Pixel
acceptance includes component depth conversion, crop, timestamps, decoder reset
and reader rewind. A final P-skip must still sample the future reference after
the non-reference B picture. All parameter sets and samples are synthetic;
no external decoder, FFmpeg, network or private source is involved.

This is progressive CAVLC motion/weight acceptance. CABAC motion/weights now have separate acceptance in `avc-444-cabac-b-inter`;
field/PAFF acceptance is covered separately; MBAFF and nonzero B residual combinations need further qualification.


Field/PAFF matrices: `avc-444-field-inter`, its B/filter companions and
`avc-444-paff-inter`. These extend admission without changing this matrix scope.
