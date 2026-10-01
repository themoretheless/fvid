# Native AVC development fixtures

All inputs are synthetic. No private user video is included. FFmpeg appears only
when a fixture is built; FVid's runtime has no FFmpeg or x264 dependency.
Normative reference: ITU-T H.264 section 7.4.3 (sequence parameter set) and
7.4.3.1 (video usability information).

## `hlg-vui-only.mp4` — a triple stated only in the parameter set's VUI

```sh
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=64x64:rate=3:duration=0.35" \
  -c:v libx264 -preset ultrafast -profile:v high10 -pix_fmt yuv420p10le \
  -x264-params "colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc:threads=1" \
  -fflags +bitexact -f mp4 tests/fixtures/avc/hlg-vui-only.mp4
```

Two 64x64 High 10 pictures at `Lavc63.1.102`, 4 867 bytes, sha256
`42b60abd2003fc576c8091d8efc76cada2a862b8a623469748e91f4331afa274`; two runs of
the command above reproduce it byte for byte, which `cmp` confirms. `ffprobe`
reports `color_primaries=bt2020`, `color_transfer=arib-std-b67`,
`color_space=bt2020nc`, `color_range=tv`.

The curve was asked of the encoder rather than of the muxer, and x264 writes what
it is told into the sequence set's VUI alone: this file carries no `colr`, `nclx`,
`mdcv`, `cclt` or `uuid` box — counted in its bytes, zero of each — so the track's
own `ColourDescription` is empty and the whole statement is the 27-byte SPS whose
length field sits at offset 0x1205 inside the `avcC` record that begins at 0x11f8.

What the file is for is the moment the answer arrives. An H.264 reader parses the
parameter sets in that record while it builds the decoder, before any picture is
decoded, so the reader states `9/18/9` over a studio range at open and a player
can bake a grade from it; a picture's own selection still wins once it exists, and
since this stream has one set the answer is the same before and after the first
picture. `an_avc_parameter_set_in_the_record_states_its_signal_at_open` and
`an_avc_records_vui_is_graded_at_open` hold to that.

No light is stated for this file, in a box or in the bitstream, and the reader's
`hdr()` stays empty at open and after every picture — HLG material as an encoder
really writes it. So the grade is HLG → BT.709 clipped to the panel's own peak,
the same shape `../hevc/hlg.mp4` pins.

### In-band AVC parameter sets and resolution changes

`../playback-errors/avc-inband-resize.mp4` is a six-frame Main/CABAC `avc3`
stream with repeated SPS/PPS and an IDR change from 64x64 to 96x64. Each
sequence has three I/P/P frames. Regenerate from synthetic test patterns with
`python3 scripts/generate_avc_parameter_sample.py` from the repository root.
The two sequences are independently decoded at their native dimensions before
concatenating the saved YUV reference, avoiding an implicit resize in a reference
filter graph. Ordinary tests use saved bytes and never invoke the generator.

`tests/avc_parameter_updates.rs` verifies every decoded sample, parameter-only
packets, invalid non-IDR/late transitions, reset, software native playback, and
fixed-size camera output across the change and backward seek.

### Scaling matrices

`../playback-errors/avc-scaling-jvt.mp4` and `avc-scaling-custom.mp4` each
contain eight synthetic 64x64 High/CABAC I/P/B pictures. Their `.yuv` files are
saved independent-decoder outputs. Regenerate both with
`python3 scripts/generate_avc_scaling_samples.py` from the repository root.
The script records all encoder parameters and deterministic custom weights.

`tests/avc_scaling.rs` requires non-flat 4x4 and 8x8 matrices and compares every
sample twice with rewind. The custom fixture additionally proves a nonzero 8x8
intra residual is actually present. The JVT fixture proves its decoded stream,
not that every advertised transform is used. Separate tests check raster scan,
flat/default differences, SPS fallback A, PPS fallback B and malformed lists.
Ordinary tests never invoke FFmpeg or the generation script.

### Lossless transform bypass

`../playback-errors/avc-bypass-lossless.mp4` (CABAC) and
`avc-bypass-cavlc.mp4` each contain eight synthetic 64x64 yuv420p I/P pictures
at QP 0. The encoder labels them High 4:4:4 Predictive (profile 244), but their
SPS explicitly states 4:2:0. Regenerate with
`python3 scripts/generate_avc_bypass_samples.py` from the repository root.
The generator independently decodes both and requires exact equality with the
original synthetic YUV before saving references. x264 disables B pictures in
this lossless mode; these fixtures therefore do not prove lossless B decoding.

`tests/avc_bypass.rs` verifies CABAC/CAVLC, profile extension parsing, QP 0,
nonzero directional intra residuals (DPCM), every output sample, and rewind.
Unit tests verify checked DPCM accumulation and joining DC/AC across 4x4 tile
boundaries. Ordinary tests never run the generator or reference decoder.
