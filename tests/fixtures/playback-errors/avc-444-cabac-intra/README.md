# Nonseparate 4:4:4 CABAC compressed intra

Author separately: `python3 scripts/generate_avc_444_cabac_intra.py`.
Test offline: `cargo test --offline --no-default-features --test avc_444_cabac_intra`.

486 owned progressive High 4:4:4 Predictive level 4.1 videos cover I16 zero/DC,
I4 zero/DC/Cb-only and I8 zero/DC/Cb-only/Cr-only, depths Y/C=8/8, 10/12, 14/8,
independent chroma QP offsets 0/-6/+6 and deblocking IDC 0/1/2. All modes include isolated and shared-slice macroblocks. I8 cases exercise
independent explicit Y/Cb/Cr 8x8 coded-block flags, including Y=0 with only Cb
or Cr nonzero, left-neighbour transform flags and filtered DC prediction
between macroblocks. Mixed transform-size transitions are qualified separately
in `../avc-444-cabac-transform-switch`; intra fields are qualified in `../avc-444-fields`. MBAFF intra is qualified in `../avc-444-mbaff-intra`; progressive and field/PAFF
inter have dedicated matrices. Adaptive MBAFF inter remains open.

The arithmetic writer uses owned interval encoding and normative probability
tables. Reconstruction/filter expectations use separate scalar Python helpers,
not the Rust decoder. Acceptance compares every coded Y/Cb/Cr sample, component
depths/crop, cropped packed software MP4 bytes, PTS/duration/index, reset and
rewind. No FFmpeg, network, private samples or private codec parameters are used.

Baseline on 2026-10-21 failed the first sample with the specific CABAC non-PCM
refusal. The initial 378 cases passed after connecting the full-resolution CABAC reader.
A historical one-component/incomplete-coverage refusal in `../avc-444-pcm` is
replaced by a valid two-macroblock/three-component acceptance sample; the four historical MBAFF refusals likewise now have complete acceptance
fixtures in that directory.

The extra 460..1023 CABAC context pairs come from pinned official H.264
(06/2019) tables 9-25..9-33, extracted by
`scripts/generate_avc_cabac_444_tables.py` using a local PDF and pdftotext.
PDF SHA-256: `4d18ee180e1760565289e25d886e0228bb0aa8e3669074f7d340db80b1118687`.
Generated bank SHA-256: `145460a95bd4f19e0e43ebb0bbf1ec69b12cc68c895e794c595c98a79eb5f813`.
The extraction explicitly normalizes Table 9-26's printed right index 641 beside
497 to 541, inferred from that table's declared 484..571 range and the 540/542
row sequence. The real 641 from Table 9-27 stays separate. This printed typo
also appears in the 2024 edition. Base 0..459 tables and legacy fixture writers
are unchanged; ordinary decoding leaves extended contexts unavailable.

The expanded 486-case matrix passed on 2026-10-21. All 1512 pre-existing
generated artifacts stayed byte-identical when shared I8 coverage was added.

Final 2026-10-21 verification passed all 215 AVC unit tests and 14 selected
AVC integration targets, including ordinary monochrome/422 CABAC, inter
residuals, MBAFF loop filtering, component fields, separate-plane 8x8 scaling,
prior 444 CAVLC/PCM/filter matrices, and these CABAC cases. Reauthoring every
artifact in this directory independently reproduced identical bytes.
