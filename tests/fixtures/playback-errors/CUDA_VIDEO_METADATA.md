# Native CUDA video metadata

`avc-cuda-video-metadata.mp4` is generated from the immutable public synthetic
`control.mp4` seed. It replaces container colour with BT.709 limited-range,
adds 3:2 pixel aspect, a 90-degree track matrix, a synthetic title and a chapter. Codec packets and parameter
sets remain synthetic; no private media is used. `mdat` precedes changed `moov`,
so packet offsets remain valid.

Generate through `scripts/generate_playback_error_samples.py`; ordinary tests
only read the checked-in fixture and require neither FFmpeg nor network access.
The source/container regression asserts the specific nondefault colour,
rotation and aspect. Separate host tests cover transformed SPS crop under flips
and range mismatch refusal. The NVIDIA movie export acceptance test encodes this
fixture and the empty-edit MOV, checks container metadata/timestamps, then decodes
all output using FVid. Hardware execution remains deferred to NVIDIA qualification.
