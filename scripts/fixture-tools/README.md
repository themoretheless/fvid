# AV1 chroma deblocking fixture encoder

This encoder is a generation-only oracle tool. FVid and ordinary tests never
build, load or execute it. Stock encoders selected zero loopfilter levels on
these source patterns, so an enabled option was insufficient qualification.
The patch chooses legal nonzero directional/Y/UV levels and sharpness; unchanged
libaom and dav1d decoders independently verify the resulting stream pixels.

Use a separate clean libaom checkout pinned to
`44d0a57786f432d933ff64b653347c66f4d0fa1d` (v3.15.1):

```sh
git clone https://aomedia.googlesource.com/aom /tmp/fvid-aom-fixture-source
git -C /tmp/fvid-aom-fixture-source checkout --detach 44d0a57786f432d933ff64b653347c66f4d0fa1d
git -C /tmp/fvid-aom-fixture-source apply /path/to/fvid/scripts/fixture-tools/av1-chroma-forced-deblock.patch
cmake -S /tmp/fvid-aom-fixture-source -B /tmp/fvid-aom-fixture-build -DENABLE_TESTS=OFF -DENABLE_DOCS=OFF -DENABLE_TOOLS=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/fvid-aom-fixture-build --target aomenc -j 8
python3 /path/to/fvid/scripts/generate_av1_chroma_filter_samples.py --encoder /opt/homebrew/bin/aomenc --forced-deblock-encoder /tmp/fvid-aom-fixture-build/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
```

Replace `/path/to/fvid` and the stock CLI paths for the generation host. Generation
requires the source checkout and reference tools; test execution only reads the
checked-in synthetic OBU/WebM/YUV assets and uses native FVid APIs.

The same generator-only encoder also produces the odd chroma super-resolution
qualification streams (coefficients 9/12 and changing 16-to-9 coded widths):

```sh
python3 /path/to/fvid/scripts/generate_av1_chroma_superres_samples.py --encoder /tmp/fvid-aom-fixture-build/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
```

Two original noise patterns are used so restoration is actually selected in all
Y/U/V planes of each layout/depth group. The offline acceptance validates actual
header coefficients/loopfilter levels, coded grids, all pixels, filter-unit
statistics and WebM reset/rewind/seek behavior.

## Full-chroma film grain fixtures

These fixtures use the stock encoder, not the forced-deblocking build above:

```sh
python3 /path/to/fvid/scripts/generate_av1_chroma_grain_samples.py --tools --encoder /opt/homebrew/bin/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
python3 /path/to/fvid/scripts/generate_av1_chroma_grain_samples.py --custom --encoder /opt/homebrew/bin/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
python3 /path/to/fvid/scripts/generate_av1_chroma_grain_samples.py --show-existing --encoder /opt/homebrew/bin/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
```

The 156 owned streams cover 4:2:2/4:4:4 at 8/10/12 bits. Both grain-enabled
oracles must agree. Offline tests check all pixels, per-layout/depth header
coverage, reset and WebM timing/rewind/seek; generation remains separate.

## Full-chroma intrabc fixtures

Stock tools generate both acceptance and intrabc-disabled controls separately:

```sh
python3 /path/to/fvid/scripts/generate_av1_chroma_intrabc_samples.py --encoder /opt/homebrew/bin/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
python3 /path/to/fvid/scripts/generate_av1_chroma_intrabc_samples.py --control --encoder /opt/homebrew/bin/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
```

Each set has 24 one-frame streams at 769x257, SB64/128, two tile columns,
lossless/lossy, 4:2:2/4:4:4 and 8/10/12 bits. Both unchanged decoders must agree.
Offline acceptance includes exact pixels, actual copied blocks/chroma phases,
source displacement parity coverage, residuals, reset and WebM rewind/seek.

## Full-chroma inter prediction and reference resizing

```sh
python3 /path/to/fvid/scripts/generate_av1_chroma_inter_samples.py --encoder /opt/homebrew/bin/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
```

The stock encoder produces 24 owned eight-frame moving sequences. Both unmodified
oracles must agree on every raw pixel, including the 96x64 keyframe to 191x127
inter transition. Offline tests require actual compound variants, OBMC,
inter-intra, local/global warp and scaled-reference execution in each
4:2:2/4:4:4 and 8/10/12-bit group, plus reset/WebM timing/rewind/seek.

## Temporal SVC operating-point fixtures

Build the original generation-only C program against stock libaom (qualified
with 3.15.1), then compare all points with unchanged reference decoders:

```sh
cc /path/to/fvid/scripts/av1_temporal_operating_points_fixture.c $(pkg-config --cflags --libs aom) -o /tmp/fvid-av1-temporal-operating-points-fixture
python3 /path/to/fvid/scripts/generate_av1_temporal_operating_points_samples.py --encoder /tmp/fvid-av1-temporal-operating-points-fixture --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
```

Only generation uses libaom/dav1d. Ordinary acceptance reads the owned OBU/WebM
and three raw goldens, checking selection, inactive-reference invalidation,
configuration/reset, exact pixels, packetized input and WebM rewind/seek.

### Spatial SVC operating points

Generation only (stock libaom 3.15.1; no external codec in ordinary tests):

```sh
cc scripts/av1_spatial_operating_points_fixture.c $(pkg-config --cflags --libs aom) -o /tmp/fvid-av1-spatial-fixture
python3 scripts/generate_av1_spatial_operating_points_samples.py --encoder /tmp/fvid-av1-spatial-fixture --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
```

The generator requires exact agreement for both operating points and outputs all
spatial layers. See the fixture provenance for source pattern and qualification.

The spatial container wrappers are authored offline, without reference tools:

```sh
python3 scripts/generate_av1_spatial_container_samples.py
```

They group actual temporal delimiters into 20ms WebM blocks/MP4 samples and
produce the missing-last-upper acceptance and multiple-unit refusal variants.

### Combined spatial/temporal SVC

Original three-spatial/three-temporal-layer source (stock libaom 3.15.1 tools
run only during generation; tests read the committed results):

```sh
cc scripts/av1_spatiotemporal_operating_points_fixture.c $(pkg-config --cflags --libs aom) -o /tmp/fvid-av1-spatiotemporal-fixture
python3 scripts/generate_av1_spatiotemporal_operating_points_samples.py --encoder /tmp/fvid-av1-spatiotemporal-fixture --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
python3 scripts/generate_av1_spatiotemporal_container_samples.py
```

The raw generator requires agreement at all nine operating points. Containers
are authored offline; generalized size arguments preserve previous two-layer
fixture bytes. Actual interlayer/temporal prediction is checked in acceptance.
