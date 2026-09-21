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
