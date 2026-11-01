# LD-MPS unequal HRTF-grid reproduction coverage

The remaining signalled 3D-stereo HRTF-grid refusal is now reproduced for
18 pairs: spatial grids4/9/23 crossed with each of the seven HRTF grids
4/5/7/9/12/15/23, excluding the three equal pairs. These are passing refusal
reproducers, not acceptance tests for unequal-grid playback. Conversion remains
unimplemented and must not be reported as complete codec coverage.

The three equal pairs are real playback controls. Their one-AU PCM is
byte-identical to the first AU of the existing full authored 3D-stereo movie.
All21 SSCs parse with their intended independent grid sizes and pass profile
validation. Every accompanying spatial frame parses fully before native movie
admission is checked, so an unrelated syntax or entropy error cannot satisfy
the refusal regression. Unequal pairs must emit the specific HRTF-grid error
without producing downmix PCM as a substitute.

The one-AU movies use the existing authored two-channel ELD/downscaled core,
24kHz output,32-bin spatial bank,tree525 and own spatial/HRTF values. No
private or original ISO media, audio, frames or codec parameters are copied.
This is not coverage of all spatial grids, bank sizes, trees or core profiles.
The existing540-programme equal-grid corpus is unchanged.

## Why the gap stays open

ISO/IEC14496-26:2010/Amd3:2014,7.26.1.2.1.1 and7.26.1.2.1.4, gives independent
nonzero restrictions for the spatial and HRTF frequency-resolution fields;
the published restriction table does not require them to be equal:
https://cdn.standards.iteh.ai/samples/62645/53b3f02e4abe4a959b9ddb1e24312b41/ISO-IEC-14496-26-2010-Amd-3-2014.pdf

That table establishes an admission gap, not the transformation algorithm.
The inspected pinned ISO source archive
`b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`
stores `hrtfParameterBands` in `sac_bitdec.c` and dequantizes only those entries.
`Calculate3Dinv` in `sac_calcM1andM2.c` then indexes directional HRTF arrays
directly by the spatial band. No grid-conversion implementation was found in
that source path. It is insufficient to infer averaging, interpolation,
parameter expansion or zero extension as the normative conversion rule.
The existing refusal is therefore retained pending qualified semantics and
independent waveform/matrix evidence. This does not block work on other codecs.

The eventual fix must replace the unequal-grid refusal expectations with
playback acceptance, add independent numerical references and multi-AU
playback/seek/rewind checks, and cover both decorrelation modes. The remaining
spatial grids5/7/12/15 and other bank/core geometries must be qualified as well;
the present18 pairs do not define a smaller completion target.

## Reproduce

Generation is explicit and separate from ordinary tests:

```sh
python3 scripts/generate_aac_ld_mps_hrtf_grid_fixtures.py
cargo test --offline --no-default-features --features player \
  --test aac_ld_mps_hrtf_grids
```

The targeted test passed with21 cases (three playback controls,eighteen refusal
reproducers). Regeneration produced identical bytes for all23 files. The JSON
report pins those artifacts. Ordinary tests read checked-in files; they do not
generate media or invoke FFmpeg/network services. No production decoder math
changes and no fresh broad regression/device gate are claimed.
