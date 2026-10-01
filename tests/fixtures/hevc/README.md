HEVC integration fixtures generated locally from FFmpeg's synthetic `testsrc2`.
No external copyrighted video is included. FFmpeg/libx265 are oracle/generation
tools only; Rust tests use embedded parameter sets, IDR payload and expected YUV.

Generation:

```sh
ffmpeg -v error -f lavfi -i testsrc2=s=32x32:r=25 -frames:v 1 \
  -c:v libx265 -x265-params 'log-level=error:pools=none:frame-threads=1:ctu=32:aq-mode=0:qp=24:no-sao=1:no-deblock=1' \
  -f hevc detail.hevc
ffmpeg -v error -i detail.hevc -frames:v 1 -f rawvideo -pix_fmt yuv420p detail-oracle.yuv
```

The `.bin` files contain NAL types 33, 34 and 20 respectively, including the
two-byte NAL header and excluding Annex B start codes. SEI/VPS are not needed by
this isolated SPS/PPS/IDR integration test. The oracle contains 1,536 samples:
1,024 Y, 256 Cb and 256 Cr, without padding. Loop filters are disabled explicitly;
this fixture verifies prediction/transform/entropy integration, not deblocking or
SAO. It exercises 10 CUs, 31 prediction blocks and 47 coded residual blocks.

The `detail10-*` files use the same generation command with
`-vf format=yuv420p10le` before the encoder, and the oracle decoder uses
`-pix_fmt yuv420p10le`. Its 1,536 samples are stored as little-endian u16 values
(3,072 bytes). The same Rust reconstruction harness compares every 10-bit value;
no truncation to eight bits occurs in this comparison.

The `multi-*` fixture uses `testsrc2=s=64x64:r=25` and adds `wpp=0` to the
8-bit encoder options above. Its four 32x32 CTUs exercise continuous CABAC state,
cross-CTU sample prediction and the top-mode restriction at a CTU-row boundary.
The expected planar YUV file contains 6,144 bytes.

The `sao-*` fixture uses the multi-CTU command with `qp=30:sao=1` in place of
`qp=24:no-sao=1`. Deblocking remains disabled. The test asserts nonzero resolved
SAO offsets before comparing all 6,144 filtered samples against the oracle.

## I/P/B playback sequences

`main-ipb.mp4` and `main10-ipb.mp4` contain 17 frames of
`testsrc2=s=128x128:r=30`, encoded with libx265 using:

```
log-level=error:pools=2:frame-threads=1:ctu=32:wpp=1:aq-mode=2:crf=28:temporal-mvp=0:weightp=0:weightb=0:bframes=3:b-adapt=0:ref=2:keyint=8:min-keyint=8:scenecut=0:open-gop=0
```

Use `-c:v libx265 -tag:v hvc1 -movie_timescale 30`. Main10 additionally uses
`-vf format=yuv420p10le`. CRF mode (not fixed QP) is intentional: it enables
adaptive CU QP deltas. Both in-loop filters remain enabled.

`weighted-tmvp.mp4` uses `testsrc2=s=128x96:r=30`,
`-vf fade=t=in:st=0:d=0.4`, `-tag:v hev1`, and the same options except
`temporal-mvp=1:weightp=1:weightb=1:open-gop=1`. It exercises nondefault weights,
collocated motion, CRA and leading RASL pictures. The integration test checks
that these features actually occur in the bitstream.

The corresponding `.yuv` files are decoded with:

```
ffmpeg -v error -i INPUT.mp4 -pix_fmt yuv420p -f rawvideo OUTPUT.yuv
```

Use `yuv420p10le` for Main10. Samples are in presentation order; tests compare
all samples, then repeat after rewind. No private video is part of the fixtures.

## `hdr10.mp4` — a signal stated only in the parameter set

```sh
ffmpeg -v error -y -f lavfi -i testsrc2=size=64x64:rate=24:duration=0.2 \
  -c:v libx265 -pix_fmt yuv420p10le \
  -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc \
  -x265-params 'colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:master-display=G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(10000000,1):max-cll=1000,400' \
  -frames:v 5 hdr10.mp4
```

Five 64x64 Main10 pictures, `hev1`, 7 226 bytes, sha256
`5c58c83bb1b03b6cf65012a80bd58a6b274b23250fa931ef3e9ad7eaf2e39113`. Two runs of
the command above reproduce it byte for byte.

What it is for is measured rather than assumed: FFmpeg 9.0.2's mov muxer writes
**no** `colr`, `mdcv` or `ccll` atom for this file (counted in its bytes: zero of
each), so the BT.2020 / PQ / BT.2020-NCL triple and the limited range a reader
reports come out of the `hvcC`'s VUI and the in-band SPS and nowhere else. The
mastering display and the 1 000/400 cd/m² light levels are likewise written
nowhere in the container: they travel as SEI messages 137 and 144, which
`ffprobe` surfaces as per-frame side data. So this fixture is the case where a
container says nothing about colour and the coding says everything, and the
in-band half of it is what a reader reports: the HEVC decoder caches SEI 137
and 144 as it walks an access unit, and a container with no box of its own has
nothing to override them with. The same encoder writes those two messages into
the `hvcC`'s NAL unit array as well, and the decoder reads that array as it is
built, so `bitstream_hdr` already states this volume and these light levels at
open — before a single packet has been decoded.

## `hlg.mp4` — a curve stated with no light at all

```sh
ffmpeg -v error -y -f lavfi -i testsrc2=size=64x64:rate=24:duration=0.2 \
  -c:v libx265 -pix_fmt yuv420p10le \
  -color_primaries bt2020 -color_trc arib-std-b67 -colorspace bt2020nc \
  -x265-params 'colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc' \
  -frames:v 5 hlg.mp4
```

Five 64x64 Main10 pictures, `hev1`, 4 571 bytes, sha256
`f8dd6121bb1a7897adbd09eb78bfdcea6e93d1329b328fd9df339853e2581ef6`. Two runs of
the command above reproduce it byte for byte. `ffprobe` reports
`color_primaries=bt2020`, `color_transfer=arib-std-b67`, `color_space=bt2020nc`.

Like the HDR10 file, this one has no `colr`, `nclx`, `mdcv` or `ccll` atom —
counted in its bytes, zero of each — so the BT.2020 / HLG / BT.2020-NCL triple
again comes out of the parameter set alone. Unlike it, nothing states any light:
this encoder writes no mastering-display or content-light message for HLG, in the
container or in the bitstream, and the reader's `hdr()` stays empty at open and
after every picture. That absence is the point. HLG's scene light is normalised
to whatever panel shows it, so a grade for this file has no headroom to compress
and takes the panel's own peak — the case
`an_hlg_picture_is_scaled_to_the_panel_that_reads_it`,
`a_real_hlg_files_signal_reaches_the_caller_that_grades_it` and
`an_hlg_item_is_graded_at_the_panels_own_peak` hold to.

## `mdcv-only.mp4` — a volume with no content light beside it

```sh
ffmpeg -v error -y -f lavfi -i testsrc2=size=64x64:rate=24:duration=0.2 \
  -c:v libx265 -pix_fmt yuv420p10le \
  -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc \
  -x265-params 'colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:master-display=G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(10000000,1)' \
  -frames:v 5 mdcv-only.mp4
```

The `hdr10.mp4` command with `max-cll` left off, which is how an encoder is
usually driven: `--master-display` is what people remember and `--max-cll` is
optional. Five 64x64 Main10 pictures, `hev1`, 7 218 bytes, sha256
`7832f5f3f55e0e4c0053be7c6247166068e86b50e29c7ace327d4fc671de014b`, again byte
for byte across two runs. `ffprobe` reports `color_primaries=bt2020`,
`color_transfer=smpte2084`, `color_space=bt2020nc`.

Again no `colr`, `nclx`, `mdcv` or `ccll` atom (counted in the file's bytes: zero
of each), so everything is in-band. What the band says is the point: SEI 137
states the BT.2020 volume and its 1 000 cd/m² authored peak, and SEI 144 *is*
written — with `max_content=0, max_average=0`. BT.2408 reads a zero MaxCLL as
"nothing stated", so this file has a mastering volume and no content light,
which is the state most real HDR10 files are in and the one where a reader that
only asked `ccll` silently tone maps from the panel it happens to be drawing
on. `a_volume_without_a_content_light_grades_against_its_own_peak` holds that the
grade compresses from the authored 1 000 cd/m² rather than the 100-nit panel's
fallback.



`../playback-errors/hevc-multislice-main.mp4` is a three-picture 128x128 Main stream with two
independent slices per picture (CTU addresses 0 and 8). Generated with:

```sh
ffmpeg -v error -f lavfi -i 'testsrc2=size=128x128:rate=30:duration=0.1' -c:v libx265 -pix_fmt yuv420p -x265-params 'pools=1:frame-threads=1:ctu=32:slices=2:log-level=error' -an ../playback-errors/hevc-multislice-main.mp4
ffmpeg -v error -i ../playback-errors/hevc-multislice-main.mp4 -pix_fmt yuv420p -f rawvideo ../playback-errors/hevc-multislice-main.yuv
```

`hevc_multislice` verifies owned independent-slice header collection, picture
identity and monotonic CTU addresses, rejects duplicated picture starts and
checks truncations. Native Main/Main10 reconstruction is byte-equal to the
saved YUV oracles, including rewind; partial pictures are never published.
The Main10 fixture uses the same commands with yuv420p10le for encoding and
reference output and `hevc-multislice-main10` filenames. Both contain I/B/P
pictures. Dependent slice inheritance is verified by the HM fixtures below. Reference-index remapping
is tested with reordered/overlapping POC maps, but these encoded fixtures do
not establish differing per-slice reference-list behavior; the comparison does not prove unrestricted HEVC conformance.

`../playback-errors/hevc-multislice-temporal.mp4` extends coverage to 18 frames,
temporal MVP, multiple active references and B-frame reordering:

```sh
ffmpeg -v error -f lavfi -i 'testsrc2=size=128x128:rate=30:duration=0.6' -c:v libx265 -pix_fmt yuv420p -x265-params 'pools=1:frame-threads=1:ctu=32:slices=2:ref=4:bframes=3:keyint=30:log-level=error' -an ../playback-errors/hevc-multislice-temporal.mp4
ffmpeg -v error -i ../playback-errors/hevc-multislice-temporal.mp4 -pix_fmt yuv420p -f rawvideo ../playback-errors/hevc-multislice-temporal.yuv
```

The owned decoder matches the saved oracle byte for byte over two passes.
Header checks require temporal MVP and multiple active references; sync seek
rebuilds references after EOF and reproduces frames 10, 4, 17, 0 and 13 from
sequential decoding. This does not establish differing reference-list order
between slices.

Dependent segments: `hevc-dependent-main`, `hevc-dependent-wpp` and
`hevc-dependent-inter` under `../playback-errors`. Synthetic testsrc2 input,
reference HM encoder/decoder at commit 382e0a040c7f0189670c4aad9716d45db977b319
from https://github.com/listenlink/HM . The temporary compiler makefile only
replaces -Werror with -Wno-error for current Clang; codec source is unchanged.
Build with `make -C build/linux -j4 release`. HM is an offline oracle,
not a production dependency. Generation (paths abbreviated):

```sh
ffmpeg -v error -f lavfi -i 'testsrc2=size=128x128:rate=30:duration=0.1' -pix_fmt yuv420p -f rawvideo input.yuv
TAppEncoderStatic -c cfg/encoder_intra_main.cfg -i input.yuv -b dependent-main.hevc -o hm.yuv -wdt 128 -hgt 128 -f 3 -fr 30 --SliceSegmentMode=1 --SliceSegmentArgument=2 --WaveFrontSynchro=0
ffmpeg -v error -r 30 -i dependent-main.hevc -c copy hevc-dependent-main.mp4
ffmpeg -v error -i dependent-main.hevc -pix_fmt yuv420p -fps_mode passthrough -f rawvideo hevc-dependent-main.yuv
```

WPP variant: segment argument 1 and WaveFrontSynchro 1, producing four segments
at addresses 0,1,2,3. Inter variant: eight input frames (duration 0.2666667),
`cfg/encoder_lowdelay_main.cfg`, eight encoded frames, segment argument 1,
WPP 1. Save the raw HEVC decode as its YUV reference: MP4 timing/edit handling
can omit its leading presentation frame; reconstruction tests compare all
eight coded pictures. `TAppDecoderStatic -b dependent-inter.hevc -o hm.yuv`
and FFmpeg raw decode are byte-identical. Production tests read saved bytes
and never execute HM/FFmpeg. Ordinary independent-segment tests remain enabled.

### In-band PPS update oracle

`../playback-errors/hevc-pps-update.packet` is the first synthetic
`hevc-multislice-main.mp4` access unit prefixed with a PPS that toggles
`constrained_intra_pred_flag`. Its construction is reproduced and asserted in
`tests/hevc_parameter_updates.rs`. The picture is intra, so the changed restriction
on inter-coded neighbours does not change its samples.

Run `python3 scripts/generate_hevc_pps_oracle.py` from the repository root to
regenerate `hevc-pps-update.yuv`. The generator converts configuration NALs and
the length-prefixed packet to Annex B, decodes one picture with FFmpeg as an
independent reference, and requires equality with the original saved first frame.
Ordinary tests read the saved oracle; they never invoke FFmpeg or the generator.

### SPS resolution-change sequence

`../playback-errors/hevc-sps-resize.mp4` contains three synthetic 96x64
Main-profile I/P/P pictures. `tests/hevc_parameter_updates.rs` feeds its parameter
sets and packets to the decoder after the existing 128x128 I/B/P fixture, compares
all samples to the saved YUV output, and restores the original configuration.
Generation commands, run in `tests/fixtures/hevc`:

```sh
ffmpeg -v error -f lavfi -i 'testsrc2=size=96x64:rate=30:duration=0.1' -c:v libx265 -pix_fmt yuv420p -x265-params 'pools=1:frame-threads=1:ctu=32:bframes=0:log-level=error' -an ../playback-errors/hevc-sps-resize.mp4
ffmpeg -v error -i ../playback-errors/hevc-sps-resize.mp4 -pix_fmt yuv420p -f rawvideo ../playback-errors/hevc-sps-resize.yuv
```
