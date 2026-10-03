# Owned constant hue controls

Regenerate with `python3 scripts/generate_hue_fixtures.py`. These one-frame
2x2 YUV420 controls contain only literal synthetic samples at 8 and 10 bits.
The export acceptance test requests a 90-degree rotation, writes FFV1 through
the public lossless API, then decodes its packets with the owned decoder and
compares exact expected samples. No generator or external codec runs in tests.

`ffmpeg_hue_reference` is an explicit optional benchmark: ten combinations
cover identity, degrees/radians, positive/negative saturation and brightness.
The implementation uses a centered chroma rotation with fixed-point rounding;
reference semantics are documented by
https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/vf_hue.c.

Owned production admission covers constant numeric parameters and 8/10-bit
planar YUV. Expressions involving timestamps or other variables, RGB,
monochrome, and other bit depths still require implementation. This does not
claim full replacement of the expression-based legacy filter.

The native MP4 frontend and `media decode --hue` also use this shared filter.
`native_hue` verifies the shared request and CLI with the committed synthetic
MP4 control, and checks hue-before-negate ordering with literal pixel values.
Ordinary tests run with no external codec executable.

Native MP4-to-FFV1 lossless configuration also retains `hue`. The CLI export
regression compares every one of the synthetic MP4 control's 25 decoded
frames with independently filtered source frames, checks the owned backend
and output EOF, and verifies that a hue request is not classified as identity.
