# Owned full-resolution 4:4:4 CABAC P pictures

Generate separately: `python3 scripts/generate_avc_444_cabac_inter.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_cabac_inter`.

324 three-frame videos cross Y/C depths 8/8,10/12,14/8, integer/signed fractional
motion, independent explicit component weights, skip, motion-only, residual
4x4/8x8, inter-to-I16 and skip-to-I16, and all cabac_init_idc values 0/1/2.
Each is an owned nonseparate High 4:4:4 Predictive level-4.1 stream, coded 32x16
and cropped [1;4]. Source I_PCM also exercises arithmetic restart between full
three-component raw macroblocks. P prediction uses one shared motion field and
independent full-resolution residual contexts, depths and mapped component QPs.

The fixture-only integer CABAC writer uses the owned normative context tables;
pixel oracles use independent scalar prediction/transform formulas shared with
the CAVLC matrix. The first baseline was refused by the CAVLC-only gate. After
enabling the full context bank, the valid 8x8 stream exposed a second failure:
current-macroblock neighbours still looked unavailable because its inter type
was published after coefficient reading. Classification now precedes reading
explicit 4:4:4 8x8 coded-block flags. Skip-to-intra also requires actual zeroed
Cb/Cr DC/CBF history, rather than unavailable-intra inference.

Acceptance compares all coded samples, checks the declared CABAC init in every
inter slice, then software MP4 pixels, crop, depth packing, clocks, reset and
rewind. A P-skip echo must sample the reconstructed reference. No system codec,
FFmpeg, network or private media is used. This is progressive qualification;
field/PAFF acceptance is covered separately; MBAFF and combinations outside this matrix remain separate work.


Field/PAFF matrices: `avc-444-field-inter`, its B/filter companions and
`avc-444-paff-inter`. These extend admission without changing this matrix scope.
