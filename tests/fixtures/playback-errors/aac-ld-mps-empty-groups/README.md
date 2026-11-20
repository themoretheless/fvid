# Empty grouped LD-MPS parameter regions

252 independently authored short MP4/MKV programmes and 28 unchanged empty
unit-stride PCM controls. Trees0/1 exercise an empty LFE OTT CLD region with
strides2/5/23, paired/unpaired fine/coarse PCM and 1D/2D Huffman data. LD/ELD
480/512 cores cover PCM32/64/128 and shared LD-SBR QMF32/64 ingress. Every
positive programme must preserve its control waveform exactly.

The empty grouped range is [0,0), but original C signed division yields one
entropy datum. It consumes bits and does not update visible bands or history.
864 saved CLD/ICC/CPC component units are checked against unmodified ISO
getStrideMap/ecDataDec plus its entropy reader. Frozen oracle records include
bit position, dummy values, range and unchanged zero history. These are
component semantics, not an independent whole-codec PCM oracle. Nonempty
regions and spatial DSP are shared by controls and cases.

28 malformed movies encode unused fine CLD PCM codeword31 in a late spatial
frame with a valid recomputed envelope CRC. Tests require this exact error,
then retry the correct packet and compare the entire waveform. They do not
stand in for positive playback acceptance. Empty unit-stride Huffman remains
invalid and has a refusal in the older frame corpus.

The former split-TTT refusal is now an acceptance case named
`split-ttt-empty-history-grouped.mp4`. Its movie payload and the packet blob
are unchanged. The owned parser consumes but does not reconstruct discarded
numeric indices; the wider split-TTT corpus in `aac-ld-mps-discard` qualifies
nonzero shared-history and out-of-range dummy results.

Tests cover both containers, sample-boundary seek, rewind, opaque source IDs,
100 Hz media-clock stamps, checkpoint/reset and worker playback. No private
media, source frames/audio or codec parameter sets are copied.

Explicit offline generation, separate from ordinary tests:

```sh
python3 scripts/generate_aac_ld_mps_empty_groups_fixtures.py
```

Optional original-C oracle generation needs the pinned external ISO reference
archive and a C compiler. It is never called by ordinary tests:

```sh
python3 scripts/validate_aac_ld_mps_empty_groups_reference.py /path/to/saoc-reference.zip
```

Ordinary tests only read the committed fixtures and need no FFmpeg or network:

```sh
cargo test --offline --no-default-features --features player --test aac_ld_mps_empty_groups
```

This does not qualify full LD-MPS profiles, unequal HRTF grids, active arbitrary
trees, changed-rate/layout SSCs or nonzero time alignment.
