# Native AV1 development fixtures

All inputs are synthetic. No private user video is included. External encoders
and decoders produce test fixtures only; FVid's runtime has no FFmpeg, libaom,
libvpx or dav1d dependency. Normative reference:
https://aomediacodec.github.io/av1-spec/av1-spec.pdf

## Picture oracles

`.obu` files are low-overhead AV1. `.yuv` files contain independently decoded,
cropped, planar 4:2:0 samples (8-bit bytes or little-endian 16-bit samples).
`.webm` files contain the same encoded video, remuxed with `-c copy`.

Build the external synthetic generator (libaom 3.15):

```sh
cc -I/opt/homebrew/include scripts/av1_fixture.c \
  -L/opt/homebrew/lib -laom -o /tmp/fvid-av1-fixture
```

Arguments are `size pattern q full-intra frames depth filters tile-cols inter`.
The deterministic ramp includes spatial, chroma and temporal variation.
`inter=1` allows P frames and disables temporal MV fields, global/warped/OBMC,
inter-intra and asymmetric partitions. Other coding decisions remain adaptive.

| Stem | Generator arguments | Oracle format |
| --- | --- | --- |
| flat | `32 0 0 0` | all samples equal 128 |
| ramp | `32 1 0 0` | yuv420p |
| lossy | `32 1 35 0` | yuv420p |
| intra64 | `64 1 0 1` | yuv420p |
| tiles | `128 1 30 1 3 8 1 1` | yuv420p |
| odd10 | `70 1 28 1 2 10 1` | yuv420p10le |
| lossless12 | `48 1 0 1 2 12 0` | yuv420p12le |
| inter-lossless | `32 1 0 1 3 8 0 0 1` | yuv420p |
| inter10 | `128 1 30 1 8 10 1 1 1` | yuv420p10le |

Example regeneration, from repository root:

```sh
/tmp/fvid-av1-fixture 128 1 30 1 8 10 1 1 1 > tests/fixtures/av1/inter10.obu
ffmpeg -v error -i tests/fixtures/av1/inter10.obu -pix_fmt yuv420p10le \
  -f rawvideo -y tests/fixtures/av1/inter10.yuv
```

`sequence.obu` is the original three-frame 64x64 SVT-AV1 4.2.0 sequence:

```sh
ffmpeg -v error -f lavfi -i 'testsrc2=size=64x64:rate=3:duration=1' \
  -c:v libsvtav1 -preset 12 -svtav1-params 'lp=1' -g 3 -crf 35 -b:v 0 \
  -f obu sequence.obu
ffmpeg -v error -i sequence.obu -frames:v 1 -pix_fmt yuv420p \
  -f rawvideo sequence-first.yuv
```

`random-access.obu` exercises 24 displayed frames, hidden/show-existing frames,
compound prediction, local warp, variable transforms, filtering and CDF refresh:

```sh
ffmpeg -v error -f lavfi -i 'testsrc2=size=192x128:rate=12:duration=2' \
  -c:v libsvtav1 -preset 12 -svtav1-params 'lp=1' -crf 35 \
  -f obu random-access.obu
ffmpeg -v error -i random-access.obu -pix_fmt yuv420p -f rawvideo random-access.yuv
ffmpeg -v error -r 12 -i random-access.obu -c copy random-access.webm
```

SVT-AV1 4.2.0 maps preset 12 to preset 11. These are finite pixel-oracle tests,
not a claim of full AV1 conformance; see `docs/NATIVE_PLAYBACK.md` for limits.

## Independent primitive oracles

`symbols.bin`: 30 libaom streams, 1024 symbols each; alphabet sizes 2–16 with
adaptation on/off. Each stream starts with a four-byte little-endian length.
Regenerate with `scripts/av1_symbol_oracle.c`; all 30,720 symbols and entropy
termination are checked by Rust tests.

`transforms.bin`: deterministic inverse-transform results from libaom, covering
19 square/rectangular sizes, valid DCT/ADST/flipped/identity combinations,
8/10/12-bit and four coefficient patterns each. libaom uses column-major input;
the oracle generator explicitly converts from FVid's row-major coefficients.

```sh
cc scripts/av1_transform_oracle.c /opt/homebrew/lib/libaom.a \
  -lm -lpthread -o /tmp/fvid-av1-transform-oracle
/tmp/fvid-av1-transform-oracle > tests/fixtures/av1/transforms.bin
```

The table generators extract normative numeric tables from the official AV1
specification and record SHA-256 hashes of their input sections in Rust output.
No external decoder source is compiled into FVid.
