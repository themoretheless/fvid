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

The original high-depth Y4M refusal fixtures now have enabled acceptance:
reading preserves 10/16-bit sample bytes, rewind is exact and RGB preview works.
The same filter and FFV1 export cases run directly from Y4M and from Matroska.
An additional synthetic sample test checks all declared 9/10/12/14/16-bit
420/422/444 reader profiles and byte-size calculations. The old byte transform
API explicitly rejects high-depth input; use the owned sample-plane geometry
pipeline for those transforms.

The preceding commit's refusal test reproduced the specific `supported pixel
formats: 8-bit 420, 422, 444` gap; it was replaced with acceptance in the fix.
Remaining legacy media operations and codec gaps are not declared finished.

Validation of the Y4M extension: six regression tests without default features
passed; the core library suite passed 652 tests with three ignored.
The same six regression tests passed with `media`, and the player builds.
High-depth Y4M also stays on the owned probe path; fixture timing assertions
confirm two frames and 80,000 microseconds without legacy demuxing.
