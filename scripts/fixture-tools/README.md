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


### AV1 external camera tile lists

Generation/reference tooling only (stock libaom required):

```sh
cc scripts/av1_tile_list_fixture.c $(pkg-config --cflags --libs aom) -o /tmp/fvid-av1-tile-list-fixture
python3 scripts/generate_av1_tile_list_samples.py --generator /tmp/fvid-av1-tile-list-fixture
```

Ordinary offline acceptance uses checked-in artifacts:

```sh
cargo test --release --locked --offline --no-default-features --test av1_tile_list
```

The tile-list generator also writes a distinct two-anchor list and oracle.
The second external image adds 9 to the clipping-safe owned anchor pattern;
entries alternate anchors 0/1. Python independently verifies both outputs
before replacing the checked-in artifacts and their SHA-256 manifest.

The same command generates all 18 tile-list cases: SB64/SB128, depth 8/10/12,
and chroma 420/422/444. The original 8-bit 420 filenames remain stable; other
cases use `d<depth>-c<chroma>-sb<sb>` suffixes. Each has a SHA-256 manifest,
explicit camera header and one/two-anchor goldens independently validated
before admission. High-depth goldens contain little-endian 16-bit samples.

Tile-list generation includes Q32 camera variants, for 36 total cases.
Anchors stay lossless; lossy source images add an owned spatial residual.
Temporary individual camera-tile reconstructions must assemble to the exact
stock tile-list oracle before artifacts are admitted. Lossy output must differ
from authored source. Ordinary tests read only admitted fixtures.

Generation also covers inherited adapted anchor CDF: 72 total camera cases.
The `cdf-` variants enable CDF adaptation/save for the normal anchor and omit
large-scale priming, preserving its frame context for camera LAST. Six offline
test functions exercise the 72 cases and missing-side-information refusal.
The C generator's final optional integer selects adapted-anchor mode (0/1);
the Python driver enumerates both modes and writes explicit manifest metadata.

The camera generator's final mode now accepts 0 (default anchor context),
1 (adapted anchor CDF inherited through LAST), or 2 (adapted anchor retained
but camera encodes PRIMARY_REF_NONE). The Python driver covers all three:
108 cases total, with `none-` tags for genuine fresh-CDF camera syntax.
Seven offline test functions cover these cases and missing external context.

The driver now enumerates moving camera variants too (216 total cases).
Use `--motion 0` or `--motion 1` to regenerate one family independently.
The C generator's final optional motion flag shifts by 4x2 luma pixels and
uses cpu-used 0, with properly extended external-reference image borders.
Native moving acceptance requires actual nonzero coded motion; matching
pixels alone is insufficient. Motion/depth/chroma/SB/context details and
artifact hashes are recorded in each `mv-` manifest.

Tile-list manifests explicitly record positive coded-feature requirements.
All 108 moving cases must decode nonzero and border-crossing motion; the
pinned 42 fractional cases must also decode fractional luma displacement.
Regenerating with different stock encoder settings must retain these properties
or update fixtures and qualification deliberately; matching pixels alone does
not satisfy the required motion coverage.

The driver now enumerates motion -1/0/1 (324 total cases). `--motion -1`
regenerates the 108 `mvneg-` cases with clamped -4/-2 luma shifts.
Directional counters and `require_motion_edges` use left/top/right/bottom order.
Each positive/reverse pair requires all four coded edges jointly; source shift
alone cannot prove coded vector direction. False requirement flags mean not
required, not absent. Nine offline tile-list test functions cover the matrix.

### AAC ancillary fill elements

Run `python3 scripts/generate_aac_fill_samples.py` explicitly to regenerate nine
owned AAC derivatives, their six-frame Y4M companion and hash manifest. No
external codec is needed. Ordinary acceptance uses committed files:
`cargo test --release --locked --offline --no-default-features --test aac_fill_extensions`.
Audio-tool refusals are kept separate from ancillary-data playback acceptance.

### AAC height PCE and explicit channel placements

`python3 scripts/generate_aac_height_samples.py` explicitly generates seventeen
original six-frame AAC streams, independent cosine PCM, two malformed headers,
a Y4M companion and hashes. No external encoder is required. Run ordinary
acceptance with `cargo test --release --locked --offline --no-default-features
--test aac_height_layouts`. Invalid CRC/layer and changed-layout tests are
separate from accepted nonzero per-channel playback.

### AAC pulse/band interactions

`python3 scripts/generate_aac_pulse_band_samples.py` explicitly generates
original spectral/uncoded/zero/noise/intensity pulse cases, pulse-free controls,
independent spectral PCM and a video companion. No external codec is needed.
Run ordinary acceptance with `cargo test --release --locked --offline
--no-default-features --test aac_pulse_bands`. Invalid bands/offsets remain
refusal tests, separate from nonzero playback acceptance.

### Indexed AAC 7.1 Top

`python3 scripts/generate_aac_top_config_samples.py` explicitly generates eight
original six-frame AAC-LC Matroska streams with channelConfiguration 14,
equivalent explicit PCE controls, independent cosine PCM, ASC and Y4M companions.
No external codec is required. Rates 44100/48000, frame sizes 960/1024 and
common/separate ICS are covered. Ordinary offline acceptance:
`cargo test --release --locked --offline --no-default-features --test aac_top_configuration`.

### SBR frequency geometry (not playback qualification)

`python3 scripts/generate_aac_sbr_frequency_oracles.py` explicitly creates
1,152 original 80-digit Decimal geometry vectors. It invokes no external codec
or network. Ordinary tests read the checked-in JSON only:
`cargo test --manifest-path crates/fvid-media/Cargo.toml --release --locked
--offline --no-default-features --lib aac_sbr_bands`.
These are exact DSP geometry checks, not encoded-stream/PCM acceptance.


SBR Huffman protocol constants: `scripts/generate_aac_sbr_huffman_tables.py`
takes a locally saved UTF-8 GOST HTML and ISO draft PDF text. It performs no
network/process codec calls, validates ten complete prefix-free tables, applies
explicit documented translation errata and cross-checks all 604 symbols against
the separate ISO text layout before writing Rust constants and JSON vectors.
Do not run generation during ordinary tests. See fixture PROVENANCE.md.


SBR dequantization oracles: explicitly run
`python3 scripts/generate_aac_sbr_dequant_oracles.py` to generate 530 Decimal
numerical cases. It uses only the Python standard library, no encoder or network.
Ordinary tests read the checked-in JSON; see fixture PROVENANCE.md.


SBR analysis QMF: explicitly run `scripts/generate_aac_sbr_qmf_oracles.py`
with local GOST HTML and ISO draft text. It cross-checks all 640 normative
window constants, then writes original PCM and direct-convolution complex
traces with hashes. No FFmpeg/network; ordinary tests use saved traces.


SBR synthesis QMF: explicitly run `scripts/generate_aac_sbr_synthesis_oracles.py`
with local ISO draft text after generating the analysis traces. It writes four
original complex input/reference PCM pairs and hashes using a direct prior-slot
calculation. Ordinary tests use saved traces and require no codec processes.


Use `scripts/generate_aac_sbr_synthesis_oracles.py --bands 32` with local ISO
draft text for downsampled synthesis references. Published 6.18.4.3 specifies
the half-sample phase; references also enforce the exact full-rate decimation
identity. Default `--bands 64` retains the existing byte-identical output.

SBR inverse-filter bandwidth references: explicitly run
`python3 scripts/generate_aac_sbr_chirp_oracles.py`. The offline 80-digit Decimal
generator writes every five-frame mode sequence plus an eight-frame decay tail.
Ordinary tests consume saved numeric traces without running the generator.

SBR covariance references: explicitly run
`python3 scripts/generate_aac_sbr_predictor_oracles.py`. The offline generator
uses original signals and independent 80-digit Decimal covariance equations.
Tests read saved traces; they do not run generators or external codecs.

SBR HF references: explicitly run `python3 scripts/generate_aac_sbr_hf_oracles.py`.
The offline 80-digit Decimal generator creates original complex QMF input and
independent covariance/predictor/HF output. Ordinary tests read saved traces.
