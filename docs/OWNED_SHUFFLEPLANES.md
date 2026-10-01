# Owned plane shuffle

`DecodeTransform.shuffleplanes` and `LosslessTransform.shuffleplanes` now run
through `fvid-media::owned_shuffleplanes`, the owned decode pipeline and FFV1
export. CLI decode, lossless export and overlay processing accept
`--shuffleplanes`. Plans identify the owned filter. No FFmpeg invocation or
libav decoding/filtering is used by this path.

Three colour planes can be permuted or duplicated. Positional map indices and
`map0`/`map1`/`map2` names are accepted. `map3` syntax is accepted for compatible
requests, but these frames have no alpha; requesting alpha in an output colour
plane fails explicitly. RGB24 uses the planar GBR mapping convention and is
repacked into RGB24.

YUV chroma-only permutations retain subsampling and byte precision. If a mapping
crosses planes of different sizes, FVid promotes to 4:4:4 with nearest sampling
before applying the mapping. Source chroma samples are repeated without numeric
conversion. This sampling policy is explicit; no promise of a legacy backend's
default interpolation is made. Storage, dimensions, indices and allocation are
validated before publication of the mutated frame.

Synthetic two-frame fixtures cover YUV444/8, YUV420/10 chroma swapping,
YUV420/10 promotion, YUV444/16 plane duplication and RGB24. Generation lives in
`scripts/generate_shuffleplanes_samples.py`; FFmpeg creates FFV1 fixture files
and saved raw oracle bytes only during explicit generation. Ordinary tests
invoke only FVid. Reference generation for promotion explicitly selects nearest
sampling. The reference filter specification/source is available at
https://ffmpeg.org/ffmpeg-filters.html#shuffleplanes and
https://www.ffmpeg.org/doxygen/8.0/vf__shuffleplanes_8c_source.html .

Five tests pass both without default features and with `media`: exact oracle
bytes, shared API/CLI decode, owned plans and FFV1 decode after export, atomic
invalid-storage refusals, and a specific high-depth Y4M refusal. Native
headless/player/camera dependency checks contain no FFmpeg adapter.

The high-depth Y4M fixtures reproduce the existing reader's `supported pixel
formats: 8-bit 420, 422, 444` limitation. That test is a refusal reproduction,
not acceptance of high-depth Y4M. The high-depth shuffle acceptance cases use
owned FFV1/Matroska decoding. Remaining legacy media operations, high-depth
Y4M reading and codec gaps are not declared finished by this change.
