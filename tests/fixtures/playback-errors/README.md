# Playback failures: short synthetic reproductions

Every new media parsing, decoding, timestamp, seeking or playback error found
during an audit must get a short synthetic video and an automated regression
test. Never copy private video, frames, audio or codec parameter sets into the
repository. Reproduce the relevant metadata and encoding structure instead.

These eleven clips contain only FFmpeg `testsrc2`, 96×64, H.264 High, 12 fps,
12 pictures, no audio. The original six clips use independently decodable
pictures and take about 18 KB each; the inter-frame pair includes I/P/B pictures.
Both MP4 movie and video timescales are 12000. The media lasts one second;
the repeat fixture's movie timeline is 13/12 seconds.

| Clip | Reproduction | Intended movie frame sequence |
|---|---|---|
| `control.mp4` | Valid unmodified control | 0–11 |
| `edit-gap.mov` | Two edit ranges skip media frames 2–3 | 0, 1, 4–11 |
| `edit-three-ranges.mov` | Three ranges skip media frames 2–3 and 6–7 | 0, 1, 4, 5, 8–11 |
| `edit-repeat.mov` | First frame repeated by overlapping edit ranges | 0, 0–11 |
| `avcc-invalid-reserved.mp4` | Synthetic `avcC` extension ends in `7b f7 f7 00` | 0–11 with compatible parsing |
| `duplicate-pts.mp4` | Signed `ctts` offset makes frames 5 and 6 share PTS; DTS stays increasing | 0–4, 6–11; last picture at equal PTS wins |
| `duplicate-pts-run.mp4` | Five pictures share PTS 3000, with increasing DTS | 0–2, 7–11; picture 7 spans ticks 3000–8000 |
| `duplicate-pts-tail.mp4` | Final pair shares PTS 10000 | Last picture spans ticks 10000–12000, including seek near EOF |
| `duplicate-pts-all.mp4` | All twelve pictures share PTS zero | Picture 11 spans the entire second; decode remains bounded |
| `control-inter-frames.mp4` | Valid I/P/B control, GOP 4 | 0–11 |
| `edit-gap-inter-frames.mov` | Cuts inside two frames and seeks through dependent pictures | 0, half of 1, half of 3, 4–11 |

`tests/playback_error_samples.rs` checks the controls and the metadata that
causes each case, so unrelated malformed-container errors cannot count as a
reproduction. Edit-list regressions now require successful playback.

Active acceptance tests check exact pixel selection, frame count, continuous
movie timestamps, fractional frame clipping, dependent-picture decoding,
seeking at both sides of edit boundaries, duration, cache timeline and rewind.
All acceptance tests are active. Compatible AVC parsing ignores redundant
format hints/reserved bits in the optional extension, while retaining bounds,
NAL type and length checks. Equal-PTS pictures are all decoded for reference
dependencies, but only the last decoded picture at that time is presented.
The following distinct PTS remains unchanged. At EOF, the retained picture uses
the group's total nominal duration. Seeking starts before the whole equal-PTS
group, so it agrees with sequential playback. Pair/run/EOF regressions check
pixels, positive intervals, rewind, duration and seeking.

Chapter regressions use the existing four-second synthetic fixture
`tests/fixtures/chapters/chapters.mp4` in `tests/mp4.rs`. They retain its supported
QuickTime text track and verify video decoding, seek and rewind, including when
the `chpl` entry count exceeds the entries actually stored.

Run without FFmpeg, a network connection or a hardware decoder:

```sh
cargo test --locked --offline --no-default-features --test playback_error_samples
```

Run the chapter regressions:

```sh
cargo test --locked --offline --no-default-features --test mp4
```

Regenerate with FFmpeg/libx264:

```sh
python3 scripts/generate_playback_error_samples.py
```

The generator encodes a valid control, then changes only container metadata.
It keeps `mdat` before `moov`, preserving packet offsets when metadata grows.
Encoding versions can change bytes; review the regenerated fixtures and tests.

Audio audit fixtures (run with `--features player`):

- `control-aac.mp4`: one second of synthetic video and mono 48 kHz sine AAC.
- `aac-quicktime-v1.mov`: version-1 AAC sample entry with `esds` nested in `wave`.
- `aac-stale-channels.mp4`: stereo sample-entry declaration with mono AAC config.
  Both decode exactly the same presented PCM as the control.
- `opus-regression.webm`: synthetic VP9/Opus. The original refusal test is now
  an acceptance test that decodes and presents exactly 48000 mono samples.
- `opus-mono`, `opus-stereo`, `opus-silk`, `opus-hybrid`, `opus-surround`: one-second WebM videos
  with independently generated libopus `.f32` references. Tests verify the TOC
  mode, PCM error below 2e-5, encoder priming, end padding, exact sample count,
  repeated decoding and sample-exact suffixes after seeking. Surround uses six
  distinct tones to detect channel-order errors. Hybrid generation explicitly
  forces that mode through the reference libopus encoder; ordinary tests do not
  load libopus.
- `opus-positive-first.webm`: positive first audio PTS; header pre-skip must
  still remove encoder priming. Its PCM matches the independent reference and
  its 1.5 ms presentation offset is preserved.
- `delayed-video-start.mp4`: first video PTS is positive. Seeking zero legitimately
  returns the first future picture; the audit must not classify this as failure.

```sh
cargo test --locked --offline --no-default-features --features player --test playback_error_samples
```

- `subtitle-burn.y4m` / `subtitle-burn.srt`: three constant 96×64 YUV420
  frames at 2 fps, with a plain SRT cue active on `[0.5, 1.0)` seconds.
  Native burn-in acceptance verifies unchanged first/final pixels, changed middle
  pixels and preserved timestamps. Regenerate with
  `python3 scripts/generate_subtitle_burn_fixtures.py` (no external tools).

`gblur-impulse.y4m`: three 10x4 frames at 2 fps with a vertical luma impulse
and constant chroma. Generated by `scripts/generate_gblur_fixtures.py` without
external tools. Owned Gaussian-filter export tests check analytical impulse
samples, unchanged chroma, and exact packet timestamps in Y4M and FFV1 inputs.

`rotate-grid.y4m`: three synthetic 3x2 YUV444 frames at 2 fps with labelled
luma samples. Generated by `scripts/generate_rotate_fixtures.py`. Tests verify
clockwise sample ordering and CLI decode, planning and FFV1 export at 90 degrees.
Arbitrary-angle interpolation is covered by component tests; CLI equivalence
with the previous backend for every angle is not yet qualified.

`rotate-white420.y4m`: three synthetic limited-range white 8x8 YUV420
frames, generated by `scripts/generate_rotate_fixtures.py`. CLI acceptance
checks a 45-degree 11x11 canvas, independently rounded chroma planes, black
corners, and rotation followed by padding and scaling without libav.

`rotate-odd420.y4m`: three synthetic white 11x11 YUV420 frames with 6x6
chroma planes, generated by `scripts/generate_rotate_fixtures.py`. Reproduces
the former reader refusal of odd geometry emitted by native rotation export.
Acceptance checks exact frame storage, all three frames, and RGB edge pixels.
The CLI regression also reopens the actual 45-degree export in both owned readers.

Library rotation acceptance in `owned_lossless` exercises Y4M-to-FFV1 and
FFV1-to-FFV1 export using these fixtures, checking exact 90-degree sample
order, retained timestamps, and rounded chroma storage for the 45-degree canvas.

`bilateral-noise-edge.y4m`: three synthetic 8x4 YUV420 frames at 2 fps,
with alternating low-amplitude noise on either side of a strong luma edge.
Generated by `scripts/generate_bilateral_fixtures.py` without external tools.
Library and CLI acceptance tests check noise reduction without mixing the
regions, unchanged unselected chroma, FFV1 export and packet timestamps.

The HTTP input acceptance checks reuse `rotate-grid.y4m` and `ffv1-gray-8.mkv` to reproduce native
URL-as-local-file opening failures. Ordinary tests feed its bytes directly into
the temporary-input copier and owned decoder. Explicit ignored loopback tests
verify redirect/chunked/truncated HTTP responses and native player reopen/rewind;
network integration is separate from ordinary test execution.

`colorize-grid-8.y4m`, `colorize-grid-12.y4m`, and `colorize-grid-16.y4m`:
three 3×3 YUV420 frames with changing luma ramps and non-neutral chroma. Generate
with `python3 scripts/generate_colorize_fixtures.py`, without FFmpeg or network.
Owned `colorize` acceptance tests require luma preservation at mix=1, exact
constant red chroma at each depth, all three timestamps and FFV1 round trips.

`monochrome-grid-{8,12,16}.y4m`: three 3×3 YUV420 frames with changing luma
ramps and four different chroma cells, generated without FFmpeg by
`scripts/generate_monochrome_fixtures.py`. Acceptance tests check chroma-driven
luma changes, neutral output chroma, precision, all timestamps and Y4M/FFV1
export and decode through the owned library and CLI.

`deband-portable-{default,strong,no-blur,coupled,depth}.expected.raw`:
independent expected pixels for the four-frame synthetic `deband*.y4m` ramps.
`generate-deband.py` computes binary64 sine/cosine and explicitly rounds the
sampling-map arithmetic to binary32, without external programs. These reproduce
the Linux/macOS sampling-coordinate divergence at luma position (5, 1), where
platform `sinf` changed a random radius. Owned deband now uses the wider math on
all platforms. The older `deband-*.expected.raw` bytes are retained as historical
macOS-libm references, not used as portable acceptance oracles.

The existing `vibrance-grid-16.rgba` also reproduces Windows colorchannelmixer
power-preservation rounding at pixels 121 and 168. Dedicated acceptance checks
require the unchanged independent `colorchannelmixer-reference-16-11.raw` bytes;
the metric uses a binary64 cube root before binary32 quantization.

Fixture checkout preserves exact bytes on Windows as well: `.raw`, `.rgb`,
`.rgba` and `.pcm` are binary in `.gitattributes`, alongside Y4M/YUV. Text
`*.expected.txt` sidecars use LF even with `core.autocrlf=true`. This keeps the
bitplanenoise metadata oracle's exact newline bytes independent of checkout
settings, without changing its values or the synthetic video/pixel references.

Quoted curves path regression reuses `curves-gray-8.y4m`, its exact saved
negative pixels, and the synthetic `curves-negative.acv` parameters. The test
copies only that synthetic ACV into a temporary filename containing a colon on
POSIX; Windows supplies the drive colon. Quoted `psfile` and `plot` paths must
remain intact, export all four frames with exact expected pixels, and write the
plot file. The old delimiter split fails specifically with `unclosed curves
quote`; malformed incomplete quotes remain rejected. No private media or
parameter sets are used and no external programs generate/run this regression.

## HEVC long-term references

`hevc-long-term-rext8.mp4` contains three 64×64 8-bit I/P/P pictures. It is
derived exclusively from the owned synthetic explicit-RDPCM fixture
`hevc-rext-explicit-rdpcm-8-skip-disabled.mp4`: the SPS enables long-term
references with zero SPS entries; each P slice replaces its short-term set
with explicit used long-term entries and MSB cycles. CABAC data is retained.
No private content or codec parameter sets are used.

Before the fix, the second picture fails specifically with
`HEVC long-term motion prediction is not implemented`. The active acceptance
test requires used long-term entries, empty short-term sets, all three decoded
pictures, exact bytes from the independent HM 18.0 reconstruction in
`hevc-long-term-rext8.yuv`, and identical results after decoder reset.
This qualifies the contained low-delay RExt case, not every HEVC profile or
hardware submission.

Regeneration is separate from ordinary tests:

```sh
python3 scripts/generate_hevc_long_term_sample.py --hm-decoder /path/to/TAppDecoder
```

The script explicitly runs the ignored native syntax rewriter, the owned MP4
muxer, and HM for the pixel oracle. Ordinary tests use committed bytes only.

The same HEVC long-term fixture exercises the owned NVDEC scheduling/submission
adapter without a driver: `synthetic_long_term_submission_sets_driver_classification_and_current_set`
checks `IsLongTerm`, `RefPicSetLtCurr`, all current-set counts and bad/missing
slot classification; `long_term_fixture_retains_slots_and_aborted_submission_preserves_state`
checks DPB retention and aborted submission. Before support, the scheduler
refused the second picture with `HEVC NVDEC long-term reference submission is not implemented`.
These passing parameter/scheduler tests are not physical GPU playback acceptance.
`owned_hevc_long_term_submits_and_maps_on_nvidia` is separately ignored on
Linux/Windows until explicitly run on an NVIDIA device.

### Mixed and LSB-only HEVC long-term fixtures

`hevc-long-term-lsb-rext8.mp4` replaces explicit MSB cycles with LSB-only
long-term entries. `hevc-long-term-mixed-rext8.mp4` keeps POC zero short-term
through POC one; POC two uses a short-term POC zero and a long-term POC one.
Both entries have `used_by_curr_pic` set. Its active L0 list has one entry,
selecting the short-term picture; separate all-long-term fixtures select the
long-term picture. VPS/SPS DPB limits are raised to three for the mixed stream.
This avoids an invalid conversion of an already long-term picture into a
short-term reference, which HM would conceal by inserting a lost picture.

Both three-frame 64×64 variants have separate HM 18.0 YUV oracles. Active
acceptance tests assert their exact RPS structure, POCs, output flags and every
pixel before/after decoder reset. NVDEC tests check both driver current sets
and the mixed/LSB scheduler's live slots; no NVIDIA playback is claimed here.
The existing generator produces all three variants and rejects any HM log
reporting insertion of a lost POC before saving the reference output.

### HEVC persistent reference classification

`hevc-long-term-invalid-short-rext8.mp4` is an owned invalid three-picture
64×64 stream: POC one marks POC zero long-term, then POC two requests it as a
used short-term reference. The previous software decoder silently accepted
this because stored DPB references lost their classification between pictures.
The regression first verifies the exact short-term delta -2, successful first
two pictures, and then the specific classification refusal. Reset must allow
normal IDR decoding again. This is a passing refusal test for an invalid
stream, not playback acceptance. The NVDEC scheduler checks the same refusal
and unchanged POC, retained slots and long-term status after failure.

The long-term generator also produces this MP4, with no pixel oracle: HM
conceals its missing short-term reference, so its recovered frames cannot
serve as acceptance evidence. Valid long-term fixture oracles still must
decode without HM reference-loss concealment.

### SPS-selected HEVC long-term references

`hevc-long-term-sps-rext8.mp4` uses two long-term SPS entries, POC LSB zero
and one, both used. Its two P slices select entry zero and then entry one with
`lt_idx_sps`, one SPS-selected entry, zero explicit slice entries and explicit
MSB cycle zero. This is distinct from fixtures declaring no long-term SPS
entries and spelling out each slice's POC.

The active native acceptance test verifies both SPS entries, the selected POC
LSB per picture, empty short-term sets, exact HM 18.0 pixels and reset replay.
NVDEC parameter tests check `num_long_term_ref_pics_sps`, the resolved live slot
and long-term current set; scheduler tests include the same stream. The
ignored physical NVIDIA long-term test now includes all four valid variants.
No physical GPU result or B-picture/multiple-active-list qualification is
implied by these tests. The existing explicit generator produces this fourth
valid fixture and separate YUV oracle without FFmpeg.

### HEVC B pictures with active mixed L0/L1

`hevc-long-term-b-base-main8.mp4` is an owned four-picture 64×64 Main8 I/B/B/B
control. `hevc-long-term-b-mixed-main8.mp4` converts the older reference to
long-term and retains the nearest as short-term. POCs two and three each have
two active entries in both L0 and L1. The final owned source picture blends
the preceding two pictures to exercise bi-prediction.

`hevc-long-term-b-mixed-l1-main8.mp4` additionally enables PPS list
modification and reverses L1 for POC two, so L0 is [short, long] and L1 is
[long, short]. All three have independent HM 18.0 YUV oracles. Acceptance
requires exact pixels for every picture and reset replay. It also inspects
actual collocated motion: the unmodified-list mixed stream must use both
reference types in both lists; the L1-permuted stream must use a long-term L1
reference. Merely having an unused list entry cannot satisfy the test.

Regenerate explicitly with owned deterministic pixels/configuration:

```sh
python3 scripts/generate_hevc_long_term_b_sample.py --hm-encoder /path/to/TAppEncoder --hm-decoder /path/to/TAppDecoder
```

The generator first verifies encoder reconstruction against the independent
HM decoder, then rewrites only parameter/RPS/list syntax through the ignored
native generator and rejects concealed reference losses. Ordinary tests read
committed bytes without HM, FFmpeg or network access. These B pictures are
low-delay with ascending POCs, not reordered future-reference B pictures.
The owned NVDEC scheduler and ignored physical-device test include both mixed
variants; actual GPU execution remains unproven.

### Reordered HEVC B pictures with future and long-term past references

`hevc-long-term-reordered-base-main8.mp4` and
`hevc-long-term-reordered-mixed-main8.mp4` are owned three-picture 64×64
Main8 streams. HM decodes I/P/B in POC order 0,2,1; presentation remains 0,1,2.
The mixed B picture keeps future POC two short-term and converts past POC zero
to long-term, with two active references in each list. The fixture muxer takes
an explicit dense presentation-order permutation and writes signed version-1
`ctts`: offsets 0,+1,-1, giving DTS 0,1,2 and PTS 0,2,1.

Acceptance checks those container clocks, the used positive short-term delta,
long-term POC zero, actual B-motion use of future POC two, every HM pixel
sorted into presentation order, and complete reset/replay. The independently
saved YUV oracle is in presentation order, not decoder packet order. The own
NVDEC scheduler and ignored physical-device test include both streams. These
tests do not claim actual player seek/UI or physical GPU execution.

Regenerate separately from tests:

```sh
python3 scripts/generate_hevc_long_term_reordered_sample.py --hm-encoder /path/to/TAppEncoder --hm-decoder /path/to/TAppDecoder
```

The owned GOP config and deterministic pixels are generated without private
media. HM reconstruction must match decoder output; decoder logs must show
exactly POCs 0,2,1 and no concealed reference loss before saving oracles.
Ordinary tests read committed bytes without FFmpeg, HM or network access.

### HEVC CU chroma-QP list acceptance

`hevc-chroma-qp-list-active-rext8.mp4` is one owned 64×64 RExt8 I picture,
with group depth zero and PPS Cb/Cr entry [6,6].
`hevc-chroma-qp-list-groups-filtered-rext8.mp4` has three owned I/B/B pictures,
group depth one, and enabled SAO/deblocking. The deterministic YUV patterns
and configuration are owned by the generator; no private media is included.
Each saved YUV is independently decoded by HM 18.0 and checked against encoder
reconstruction. Regenerate explicitly with
`scripts/generate_hevc_chroma_qp_sample.py --hm-encoder /path/to/TAppEncoder --hm-decoder /path/to/TAppDecoder`.

The original raw PPS refusal and subsequent picture-selection refusal are now
replaced by enabled pixel/reset acceptance. Bounds tests cover group depth,
up to six pairs, signed offsets and CABAC selection/truncation. These cases
qualify software 4:2:0 RExt8 reconstruction; NVDEC configuration still refuses
unqualified lists. Ordinary tests read committed bytes without HM, FFmpeg or
network access.

Additional chroma-QP fixtures generated by the same script:

- `hevc-chroma-qp-list-wpp-rext8`: two WPP substreams per picture.
- `hevc-chroma-qp-list-slices-rext8`: four independent slice segments.
- `hevc-chroma-qp-list-dependent-rext8`: one independent and three dependent segments.
- `hevc-chroma-qp-list-high10-rext10` / `high12-rext12`: 10/12-bit reconstruction, YUV saved as little-endian 16-bit samples.

Each contains three owned I/B/B pictures, group depth one and enabled filters.
Tests validate entropy-path metadata and every saved HM sample after reset.

### HEVC PCM raw samples and CABAC restart

Owned `hevc-pcm-*.mp4` fixtures have matching saved HM `.yuv` files. Each has
one I picture, PCM block range 32x32 and no PPS chroma-QP list. The generator
owns its deterministic noise/constant pixels; no private media is copied.
The former SPS-PCM picture refusal reproduces with the old gate, and enabled
acceptance checks every pixel and reset replay under the implemented decoder.

- `active-rext8`: 64x64, all PCM.
- `mixed-rext8`: alternating PCM and normal CUs with enabled filters.
- `filtered-rext8`: PCM filtering permitted by the SPS.
- `parallel-rext8`: mixed 128x96 picture through row-worker reconstruction.
- `high10-rext10` / `high12-rext12`: eight-bit PCM scaled into higher-depth sequences.
- `full10-rext10` / `full12-rext12`: ten/twelve-bit PCM syntax.
- `wpp-rext8` / `dependent-rext8`: WPP and dependent-segment entropy restart.

Tests require actual PCM samples and, for mixed files, actual normal samples.
High-depth YUV uses little-endian 16-bit samples. Regenerate explicitly with
`scripts/generate_hevc_pcm_sample.py --hm-encoder /path/to/TAppEncoder --hm-decoder /path/to/TAppDecoder`.
Ordinary tests run without HM, FFmpeg or network access. Enabled filter syntax
alone is not exhaustive proof of nonzero filter effects on PCM boundaries.

PCM family extensions generated by the same owned script:

- `hevc-pcm-reference-rext8` / `hevc-pcm-reference-wpp-rext8`: I/B/B sequence, all-PCM I picture followed by motion-compensated non-PCM pictures, without/with two WPP substreams.
- `hevc-pcm-small8-rext8` / `hevc-pcm-small16-rext8`: all-PCM I picture constrained to 8x8/16x16 coding units.

Acceptance verifies actual PCM sample counts and actual motion references,
then compares every HM sample on original decode and reset replay. The same
explicit generator writes saved oracles; ordinary tests read committed bytes.

### HEVC single-slice tiles acceptance

Owned `hevc-tiles-*.mp4` fixtures contain three I/B/B pictures and matching
independent HM `.yuv` oracles. PCM/chroma-QP lists/WPP are disabled.
The original picture-tools refusal is replaced by enabled pixel/reset tests.

- `two-columns-rext8`: 64x64, two vertical tiles, CTU raster addresses visited as 0,2,1,3; filters disabled.
- `filtered-rext8` / `cross-filtered-rext8`: enabled SAO/deblocking, filtering respectively blocked/allowed across tile boundaries. Oracles differ in 1535 samples.
- `asymmetric-rext8`: 96x96, column widths [1,2] and row heights [2,1] in CTUs; four nonuniform tiles.
- `high10-rext10` / `high12-rext12`: two-column filtered 10/12-bit sequences, YUV saved as little-endian 16-bit samples.

Regenerate explicitly with `scripts/generate_hevc_tiles_sample.py --hm-encoder /path/to/TAppEncoder --hm-decoder /path/to/TAppDecoder`.
Tests validate geometry/entropy metadata, every decoded HM sample and reset
replay. Mutated metadata tests reject stream-count/bounds/truncation errors,
bad geometry and insufficient total decode budget. Ordinary tests read saved
bytes without external tools or network access. Multiple slice segments with
tiles and physical GPU decoding remain unqualified.

### HEVC tiled slice-segment address regression

`hevc-tiles-slices-rext8` / `hevc-tiles-dependent-rext8` contain three owned
I/B/B pictures with four independent/dependent segments per picture. Their
raster addresses are 0,2,1,3, correctly increasing in tile-scan order. Filters,
PCM, chroma-QP lists and WPP are disabled. Saved YUV is independent HM output.
The shared tile generator reproduces both MP4/YUV pairs.

Before the ordering fix, the independent case refused "slice addresses must
increase" and the dependent case refused "dependent segment address out of
range". Header admission now accepts both. Software picture reconstruction
still explicitly refuses multi-slice tile tools, and the HM pixel/reset
acceptance remains ignored until that implementation is complete. CPU-side
NVDEC tests preserve NAL submission order and reject raster-sorted segments;
no physical GPU acceptance is implied. Ordinary tests use committed bytes.

### HEVC tiled segment pixel acceptance

The former ignored tiled-segment acceptance is enabled. The shared HM generator
also produces `slices-filtered`, `dependent-filtered`, `cross-segments`,
`mixed-segments`, `spanning-segments` (all `rext8`) and
`mixed-segments-high10-rext10` / `mixed-segments-high12-rext12`.
Tests compare every sample of all three pictures and repeat after decoder reset.
The spanning case uses three vertical tiles and two segments, with two entropy
substreams in its first segment. Mixed cases contain both independent and
dependent segments and disable filtering across independent slice boundaries.
These fixtures replace the previous multi-segment reconstruction refusal;
ordinary tests consume committed MP4/YUV bytes without HM, FFmpeg or network.

### HEVC extended-precision staged acceptance

`hevc-tiles-extended-precision-high8-rext8` and
`hevc-tiles-extended-precision-high12-rext12` contain owned 64x64 I/B/B pictures
and saved HM pixel oracles. They reproduce the exact SPS range-tool refusal.
The pixel/reset acceptance test is explicitly ignored until full integration.
Generate separately with the tile generator's `--extended-precision-only` switch
and HM built with `HIGH_BITDEPTH=ON`; the default fixture set is unchanged.
No private source media or codec parameter sets were used.

Extended-precision acceptance is now enabled and replaces the SPS refusal
expectation above. Additional streams are `high10-rext10`, `mixed-high12-rext12`,
`wpp-high12-rext12` and `skip-rice-high12-rext12` under the same
`hevc-tiles-extended-precision-` prefix. Each contains three owned I/B/B pictures
and a saved HM pixel oracle. Every sample and decoder reset replay are checked.
The GPU configuration refusal is separately tested with the 8-bit fixture.

### HEVC CABAC aligned bypass staged regression

`hevc-cabac-alignment-444-rext12.mp4` contains three owned 64x64 4:4:4 I/B/B
pictures, 12-bit coding in the high-throughput 14-bit constrained profile, WPP,
extended precision and aligned coefficient bypass syntax. The paired YUV is
the independent HM decode oracle (also equal to encoder reconstruction).
The exact SPS refusal test passes; pixel/reset acceptance remains explicitly
ignored until 4:4:4 and alignment integration. Regenerate separately using
`scripts/generate_hevc_alignment_sample.py` with a HIGH_BITDEPTH HM build.
Ordinary tests invoke neither HM nor FFmpeg nor network access.

CABAC alignment syntax is now connected. The staged fixture test asserts active
alignment/extended-precision metadata and 4:4:4 12-bit SPS geometry, then checks
the exact remaining picture-tools refusal. The old SPS refusal expectation was
replaced; full pixel/reset acceptance remains ignored pending 4:4:4 geometry.

### HEVC full-resolution chroma acceptance

The aligned 4:4:4 acceptance is now enabled, replacing its old refusal.
The same explicit HM generator's `--full-chroma-suite` option also saves:

- `hevc-full-chroma-filtered-rext8/rext10/rext12`: SAO and deblocking.
- `hevc-full-chroma-high-qp-rext12`: QP=40 and linear chroma QP mapping.
- `hevc-full-chroma-wpp-rext12`: entropy row synchronization.
- `hevc-full-chroma-mixed-tiles-rext12`: independent/dependent tiled segments.
- `hevc-full-chroma-parallel-rext12`: 128x96 activates queued reconstruction.

Every fixture contains three owned I/B/B pictures and a paired decoded HM YUV
oracle; tests compare every sample and repeat after decoder reset. Generation
requires explicit HIGH_BITDEPTH HM paths. Tests invoke no external executables
or network. Separate-colour-plane, 4:2:2 and deeper sample formats remain open.

### HEVC 4:2:2 filtered acceptance

`hevc-chroma422-filtered-rext8/rext10/rext12.mp4` contain three owned
64x64 I/B/B pictures with SAO and deblocking. Paired YUV files contain
independently decoded HIGH_BITDEPTH HM reference samples (Y: 64x64,
Cb/Cr: 32x64). Regenerate explicitly with
`scripts/generate_hevc_alignment_sample.py --chroma422-only` and HM paths.
Tests invoke no external tools or network.

`tests/hevc_chroma422.rs` now enables complete pixel/reset acceptance and
software-player pixel/rewind acceptance for all three depths. The old exact
picture-tools refusal was replaced by metadata/first-picture acceptance.
Transform-tree syntax retains both vertically adjacent chroma blocks; motion,
PCM, SAO, prediction coordinates and deblocking use independent axis scales.
The corpus also includes `hevc-chroma422-wpp-rext12`,
`hevc-chroma422-mixed-tiles-rext12` and `hevc-chroma422-parallel-rext12`.
The last uses 128x96 WPP to activate queued reconstruction. The tiled stream
contains both independent and dependent slice segments across two columns.
Tests assert the active PPS tools, segment types and queue-enabling geometry,
then compare every pixel and replay decoder/player reset for all six streams.

### HEVC cross-component prediction acceptance

`hevc-cross-component-rext8/rext10/rext12.mp4` contain three owned 64x64
4:4:4 I/B/B pictures with cross-component prediction, SAO and deblocking.
`hevc-cross-component-parallel-rext12.mp4` uses 128x96 WPP and enables queued
reconstruction. Paired YUV files are decoded HIGH_BITDEPTH HM oracles,
checked against encoder reconstruction. Regenerate explicitly using
`generate_hevc_alignment_sample.py --cross-component-only` and HM paths.

`tests/hevc_cross_component.rs` now accepts PPS/pictures, compares every
sample with HM after decoder reset, and compares software-player frames after
rewind for all four streams. A codec-library test asserts nonzero alpha in
every stream, including the queued case. The old PPS refusal is replaced and
the formerly ignored pixel acceptance is enabled. Normal tests invoke no HM,
FFmpeg or network.

### HEVC monochrome filtered acceptance

`hevc-monochrome-filtered-rext8/rext10/rext12.mp4` contain three owned 64x64
monochrome I/B/B pictures with SAO/deblocking and no chroma QP adjustment.
Paired YUV files contain independently decoded HM luma, checked against
encoder reconstruction. Regenerate explicitly with `--monochrome-only`.
Ordinary tests invoke no external tools or network.

`tests/hevc_monochrome.rs` admits valid metadata and pictures, compares every
luma sample after reset, and tests software-player rewind with neutral chroma.
The former picture refusal and ignored acceptance were replaced. The software
adapter's former empty-chroma panic is reproduced by these streams and covered
by the playback test. Additional `hevc-monochrome-wpp-rext12`,
`hevc-monochrome-mixed-tiles-rext12` and `hevc-monochrome-parallel-rext12`
streams qualify WPP, two-column mixed independent/dependent segments and
128x96 queued reconstruction. Tests assert the active PPS tools and slice
headers, compare all luma samples and replay reset/rewind for all six streams.

### HEVC monochrome PCM acceptance

`generate_hevc_pcm_sample.py --monochrome-only` generates 14 owned streams with
prefix `hevc-pcm-mono-` and paired decoded HM luma oracles: active, small8,
small16, mixed, filtered, parallel, high10/high12, full10/full12, WPP, reference,
reference-WPP and dependent segments. These exercise 8x8/16x16/32x32 PCM,
8-bit PCM scaled to 10/12-bit reconstruction, full-depth PCM, both filter
policies, mixed PCM/non-PCM CUs and reference-picture reuse. The parallel stream
is 128x96. Normal tests invoke no external tools or network.

`monochrome_pcm_corpus_matches_hm_and_restarts` asserts the monochrome SPS and
active PCM samples (partial coverage in mixed streams), compares every luma
sample against HM and repeats after reset. Chroma planes remain absent.

### HEVC 4:2:2 and 4:4:4 PCM acceptance

The explicit PCM generator accepts `--chroma-format 422` or `444` and creates
14 streams per format with prefixes `hevc-pcm-422-` / `hevc-pcm-444-`.
The case matrix matches monochrome: active, small8/small16, mixed, filtered,
parallel, high10/high12, full10/full12, WPP, reference/reference-WPP and dependent.
Sources and saved HM oracles use the actual component geometry: half-width,
full-height chroma for 422; equal-sized planes for 444.

`subsampled_and_full_chroma_pcm_corpus_matches_hm_and_restarts` asserts active
PCM, partial PCM coverage in mixed CUs, exact chroma dimensions, every HM sample
and replay after reset across all 28 streams. Ordinary tests invoke no HM,
FFmpeg or network. Default 420 output naming and source generation are retained.

### HEVC mixed component depth acceptance

`hevc-mixed-depth-y8-c10`, `hevc-mixed-depth-y10-c8` and
`hevc-mixed-depth-y12-c10` contain three owned 64x64 4:4:4 I/B/B pictures with
SAO/deblocking. The generator's `--mixed-depth-only` mode sets independent
internal/output component depths and writes both depth fields to hvcC.
HM serializes all components as little-endian 16-bit samples in these oracles.
Native acceptance compares every sample and repeats after reset. Generation
checks HM decoder output against encoder reconstruction. Normal tests require
no external tools. Player/export component-depth transport is separately open.

Mixed-depth generation also saves `-wpp`, `-mixed-tiles`, `-parallel` and `-cross`
variants for each depth pair, giving 15 streams. Tests assert active entropy
sync, tiles, independent/dependent headers, queue-enabling 128x96 dimensions
and the cross-component PPS flag. All native samples match HM after reset.
The codec-library alpha counter separately proves nonzero alpha in all three
mixed-depth cross-component streams; this tests scaling both up and down.
Player/export mixed-depth transport remains a separate pending requirement.

Mixed-depth HEVC export acceptance: `tests/hevc_mixed_depth_export.rs` reuses
the fifteen `hevc-mixed-depth-*` synthetic MP4/YUV pairs. Before the export fix
the equal-depth spool gate returned unsupported and the public export fell
through to a misleading Y4M ASCII error. Acceptance compares all FFV1 output
samples to the independently depth-scaled HM oracle, checks scalar negate via
both public export APIs, and repeats output decoding after rewind. No external
codec or generator runs during these tests.

`hevc-deep-rext14{,-parallel,-inter,-inter-parallel}` and
`hevc-deep-rext16{,-parallel}` are owned three-frame 4:4:4 HM streams.
Generate explicitly with `generate_hevc_alignment_sample.py --deep-depth-only`
and supplied HM encoder/decoder. Ordinary tests use the MP4/YUV bytes only.
Before the fix `tests/hevc_deep_depth.rs` rejected the first picture as invalid
HEVC picture parameters. Acceptance verifies every oracle sample, actual
precision beyond 12 bits, I versus B syntax, WPP, software playback, both
FFV1 export APIs, negate, reset and rewind.

Deep PCM fixtures: `hevc-pcm-444-deep-{input8,full,mixed,dependent,parallel}-rext{14,16}`
are owned three-frame high-throughput intra streams. Generate explicitly with
`generate_hevc_pcm_sample.py --deep-depth-only --chroma-format 444` and supplied
HM tools. Full-depth source noise uses 16-bit little-endian sample words; input8
uses bytes. Minimum QP forces genuine PCM selection. The inline core acceptance
checks actual PCM sample counts, PCM depth, mixed blocks, filter policy,
dependent segments and WPP before comparing all HM samples and reset.
`tests/hevc_deep_pcm.rs` checks every pixel through playback and both FFV1 export
APIs after rewind. Ordinary tests only consume the saved bytes.

SCC base fixtures: `hevc-scc-base{,-parallel,-empty-initializers}-rext{8,10}`
are owned three-frame 4:4:4 I/B/B streams with extra SCC coding tools disabled.
`generate_hevc_scc_base_sample.py` explicitly invokes supplied HM tools for base
streams and reference pixels, then changes PTL and inert SCC SPS/PPS syntax;
entropy bytes stay unchanged. The acceptance test `tests/hevc_scc_base.rs` checks
profile 9, both SCC extensions, WPP, every decoded pixel, reset, software
playback, both FFV1 export APIs and rewind. Before the fix it failed at the
blanket unsupported SCC SPS extension gate.
`hevc-scc-reserved-motion.mp4` instead contains reserved SCC motion-resolution
value 3 and tests exact invalid-syntax refusal; it is not a playable acceptance
fixture. Ordinary tests invoke no external generators/codecs.

`hevc-scc-boundary-disabled{,-parallel}-rext{8,10}` covers SCC intra boundary
filtering disable with 4:4:4 I/B/B pictures, ordinary reconstruction and WPP.
Generate explicitly with `generate_hevc_scc_base_sample.py --boundary-disabled-only`
and HM18 HIGH_BITDEPTH encoder/decoder built with the saved
`scripts/hm_scc_boundary_oracle.patch`. The controlled reference disables both
angular and DC boundary correction. It is a patched oracle; unmodified HM does
not provide SCC support here. Encoder reconstruction and decoder bytes must
match before SCC syntax is added; entropy bytes stay unchanged. Tool/patch and
fixture hashes are in `hevc-scc-boundary-oracle.json`.
`tests/hevc_scc_boundary.rs` reproduces the previous unsupported boundary-disable
flag error and now checks every pixel through decoding, software playback and
both FFV1 export APIs, reset/rewind, and a deliberately ignored-flag mismatch.
Ordinary tests neither compile nor run reference tools.

SCC integer-motion fixtures (`hevc-scc-integer-*`) are three-frame owned
procedural 4:4:4 streams at 8 and 10 bits, encoded and decoded with official
HM-16.20+SCM-8.8 (revision and hashes in `hevc-scc-integer-oracle.json`).
They cover forced integer precision, WPP, adaptive integer and adaptive quarter
precision. `tests/hevc_scc_integer.rs` checks headers, all reconstructed samples,
reset, software playback, rewind and both native FFV1 export entrypoints.
Generate separately with `scripts/generate_hevc_integer_motion_sample.py` and
explicit SCM encoder/decoder paths; ordinary tests use only committed bytes.

SCC adaptive colour transform fixtures (`hevc-scc-act-*`) contain three owned
procedural 4:4:4 frames at 8 and 10 bits. Official SCM 8.8 encoder reconstruction
and decoder output are byte-identical; revision and binary/fixture hashes are
in `hevc-scc-act-oracle.json`. Variants exercise intra, inter, WPP, non-default
ACT QP offsets and transquant bypass. Core tests count active ACT blocks,
including inter blocks, and compare every sample twice with a decoder reset.
Integration tests cover software playback, rewind and both native FFV1 exports.
The original parser refuses these streams specifically at the ACT-enabled PPS.
Generate separately with `scripts/generate_hevc_act_sample.py` and explicit
SCM binary paths. No reference tools run during ordinary tests.

Additional ACT qualification (`hevc-scc-act-depth-*`, `hevc-scc-act-slice-*`)
uses three owned I/B/B frames, mixed 8/10 and 10/14 component depths, mandatory
WPP for profile 11, and non-zero slice offsets [3,-2,4] added to PPS offsets
[-5,-5,-3]. The slice-offset encoder patch enables existing reference settings;
the SCC14 patch corrects only the reference encoder's profile-validation gate.
Neither changes codec algorithms. All encoder reconstructions equal unmodified
SCM decoder output; JSON provenance records binary, patch and fixture hashes.
Core tests count active ACT for both intra and inter frames. Integration tests
compare every original-depth pixel, common-depth playback, rewind and both
FFV1 export APIs. Generation is `generate_hevc_act_depth_sample.py` with explicit
reference paths; use `--slice-offsets` for the separate offset fixture group.

Current-picture prediction has twenty-four owned 8/10-bit fixtures: ordinary and
WPP 4:4:4, weighted prediction, and 4:2:0 with odd luma vectors requiring
fractional chroma interpolation. `tests/hevc_scc_ibc.rs` compares every native
sample, reset, software playback, rewind and both FFV1 exports with SCM.
Current references read already reconstructed, unfiltered samples without a
second picture allocation; bounds, readiness, CU and WPP availability are checked.
Unmodified SCM supplies the oracle; a logging-only probe built with
`scm_ibc_probe.patch` proves active current-picture predictions and produces
identical samples. Counts and hashes are in `hevc-scc-ibc-oracle.json`.
Generate separately with `generate_hevc_ibc_sample.py`; ordinary tests require
no SCM, FFmpeg or network. The additional inter/inter-parallel streams mix current and completed
references. They reproduce a missing temporal IBC predictor: both reference
classes are used, and current-reference vectors transfer without POC scaling.
B-slice ordinary/WPP fixtures also require actual biprediction. Tiled and
dependent-segment fixtures accept every sample and validate their active
parameter/header flags. Additional depths and combined tool configurations
still need dedicated qualification.

`hevc-scc-current-capability{,-parallel}-rext{8,10}` separates an enabled SPS
capability from use of the tool: PPS current-picture prediction stays disabled.
Only inert owned parameter-set headers are rewritten; entropy stays unchanged.
Unmodified SCM decoder output of the rewritten stream equals encoder samples.
`tests/hevc_scc_current_capability.rs` accepts all pixels/reset/playback/rewind
and both FFV1 exports. Generate separately with the SCC base generator's
`--current-capability-only` option. These inert fixtures qualify capability admission separately from active
IBC acceptance above. The IBC test also verifies header parsing and the SPS/PPS dependency.

Palette SCC has twenty-eight owned `hevc-scc-palette-{intra,parallel,initializers,initializers-parallel,pps-initializers,pps-initializers-parallel,tiles,dependent}-rext{8,10}`
fixtures generated with SCM's pinned official intra-SCC configuration, overriding
ACT and IBC off. The unmodified decoder equals encoder reconstruction; a separate
logging-only `scm_palette_probe.patch` decoder counts actual palette blocks and
must return identical pixels. The JSON records configuration/binary/patch and
fixture hashes. Generate explicitly with `generate_hevc_palette_sample.py` and
reference binary paths; the SCM checkout must contain its official config.
`tests/hevc_scc_palette.rs` now accepts every decoded sample, reset, software
playback, rewind and both FFV1 export APIs. Palette CU syntax, reconstruction
and predictor state are connected; palette samples are excluded from deblocking.
The old refusal and ignored acceptance were replaced. Additional chroma/depth,
escape-heavy and combined-tool streams need dedicated coverage. The tiled and
dependent variants validate actual segment headers and every sample.

Palette SPS metadata accepts bounded maximum sizes and initializer tables.
The initializer variants set PalettePredInSPSEnabled and tests require nonempty
parsed tables. Palette admission and CU reconstruction are now enabled by the pixel acceptance
tests described above.

PPS initializer variants explicitly enable PalettePredInPPSEnabled. Tests require
nonempty tables and reject SPS capability/depth mismatches. A missing PPS table
inherits SPS initializers; an explicit empty table clears them. Tests keep the
legacy inert zero-entry PPS case separately from active palette admission.

Palette mono/420/422 ordinary/WPP 8/10-bit fixtures extend the 444 group.
Monochrome comparison keeps native luma unchanged and verifies neutral chroma
introduced for display/export; root export uses 420 and media-library export
uses 444. Both geometries are checked explicitly against reconstructed samples.

HEVC SCC palette escape qualification: eight additional owned 8/10-bit 4:4:4 streams cover lossy EG3 escapes and forced transquant-bypass fixed-width escapes, with and without WPP. The logging-only SCM probe records actual decoded luma escape samples (341–1043 lossy, 423–1272 bypass), and its output must equal the unmodified reference decoder. All 36 palette streams are accepted by the native codec, reset/rewind playback, and both lossless export APIs with exact oracle samples. Generation remains explicit and separate from offline tests.

2026-10-06: Palette escape qualification now includes 16 additional owned 4:2:0/4:2:2 streams (8/10-bit, lossy/bypass, ordinary/WPP). Reference logging proves 2763 lossy and 6738 bypass luma escape reads across the new streams. All 52 palette streams pass exact native pixels, reset, software playback/rewind and both lossless exports in the offline acceptance test (2 passed, 0 ignored). No additional depth or transpose coverage is inferred from these results.

2026-10-06: Eight owned monochrome palette-escape streams extend qualification to 60 total (8/10-bit, lossy/bypass, ordinary/WPP). Logging-only reference probe observed 1151 lossy and 3330 bypass luma escape reads in these eight. Offline native sample/reset/playback/rewind and both lossless-export acceptance checks pass (2 passed, 0 ignored). Display/export neutral chroma follows the existing monochrome API expectations; no chroma input is invented in the oracle. Other depths and tool combinations remain unqualified.

2026-10-06: Logging-only SCM probe now records actual transposed palette escape reads. All 32 escape fixtures (mono/420/422/444, 8/10-bit, lossy/bypass, ordinary/WPP) have nonzero counts. Generator rejects missing coverage, and an offline provenance regression verifies every saved escape case. All 60 streams again pass exact native pixels/reset/playback/rewind/both exports (3 tests passed, 0 ignored). This supersedes the earlier absence of observed transpose qualification for these specific fixtures; broader depths/tools remain unqualified.

2026-10-06: Added 28 owned 12-bit palette streams, bringing the total to 88. SCC high-throughput uses the legal 14-bit profile constraint with actual 12-bit component depths and mandatory WPP; encoder configuration validation uses the documented scm_scc14_config.patch, reference decoder remains unmodified. Tiles/dependent tile cases are excluded at 12-bit because SCM rejects tiles+WPP for this profile. Mono/420/422/444, predictor initializers, lossy/bypass/transposed escapes all pass exact samples, reset, playback/rewind and both exports (3 offline tests passed, 0 ignored). Input is owned 8-bit pattern promoted by the encoder, so arbitrary full-precision 12-bit input remains a separate qualification.

2026-10-06: Eight additional owned full-precision 12-bit palette/escape streams cover mono/420/422/444 lossy and forced bypass under mandatory WPP. Source uses 16-bit little-endian storage with nonzero low four bits, not promoted 8-bit samples. Generator proves low-bit retention in the oracle and source==oracle for bypass; offline provenance regression verifies retained low bits and actual transposed escape reads. All 96 palette streams pass exact native pixels, reset, playback/rewind and both lossless exports (3 tests passed, 0 ignored). This qualifies these full-precision patterns, not arbitrary 12-bit streams or all codec tools.

2026-10-06: 36 new owned 14-bit SCC palette streams extend acceptance to 132 total. Covers mono/420/422/444, SPS/PPS initializers, lossy/bypass/transposed escapes and eight full-precision input cases. High-throughput SCC uses 14-bit constraint and mandatory WPP; forbidden tiles+WPP combinations remain excluded. Generator confirms low-bit oracle retention and exact bypass input preservation. All 132 pass native sample/reset/playback/rewind/both lossless-export acceptance (3 offline tests passed, 0 ignored). Existing AVC field-reference and display-reordering refusals remain in source; this is not completion of the codec-gap objective.

2026-10-06 AVC audit correction: the earlier statement that display reordering is wholly unimplemented was too broad. AvcDecoder::decode_order accepts coded-order I/P/B pictures, and the MP4 playback reader performs presentation ordering using timestamps. AvcDecoder::decode intentionally requires increasing POC; its error now directs callers to decode_order. The owned avc-multislice-ipb fixture proves this specific API refusal, decode-order acceptance, reset, and exact presentation-order oracle playback/rewind in a dedicated regression. avc_multislice: 12 passed, 0 ignored, offline without FFmpeg. Field reference lists/marking remain unsupported; this audit does not remove those gaps.

2026-10-06 MBAFF gap reproduction: avc-mbaff-cabac.mp4 and avc-mbaff-cavlc.mp4 are owned three-picture 64x64 x264 CLI streams, generated explicitly by scripts/generate_avc_mbaff_sample.py (no FFmpeg). avc_mbaff verifies parsed MBAFF SPS and matching entropy mode, then the exact progressive-only reconstruction refusal, excluding unrelated parse failures. This is a passing refusal/reproduction test, NOT playback acceptance or an implemented interlaced decoder. Future support must replace this expectation with pixel acceptance against an independent oracle. Current offline reproduction: 1 passed, 0 ignored.

2026-10-06 MBAFF implementation foundation: src/codec/avc_mbaff.rs provides checked component sample addressing for progressive blocks and frame/field macroblock pairs (H.264 6.4.1). Supports mono/444 geometry and 422/420 subsampling with field row stride. Tests prove mixed frame/field pairs cover every component sample exactly once and reject out-of-picture/overflow/malformed geometry (2 passed offline). This helper is not yet connected to entropy, prediction or deblocking; the owned MBAFF playback refusal remains unchanged.

2026-10-06 MBAFF addressing continuation: added component sample-to-owner mapping for mixed frame/field pairs. Unknown pair mode or out-of-picture sample returns unavailable instead of guessing. Forward/inverse round trips cover each sample of mono/444, 422 and 420 layouts; progressive mapping is now used by existing intra reconstruction. Three geometry tests and 16 integration tests (MBAFF refusal, AVC multislice pixel/rewind/seek and parameter updates) passed offline. Entropy field flags, MBAFF prediction neighbours and deblocking remain incomplete; interlaced playback acceptance is not claimed.

2026-10-06 MBAFF field syntax foundation: PairMode reads mb_field_decoding_flag on even macroblock addresses or an odd address after skipped top, and otherwise inherits the known pair mode (H.264 7.3.4). Reader callback can supply a CAVLC bit or future CABAC decision; no CABAC context implementation is implied. State commits only after successful flag read; missing top mode is an error. Four addressing/state tests and two owned fixture tests passed offline. Actual CAVLC IDR fixture validates flag precedes I macroblock type. This is syntax-prefix qualification, not full coefficient parsing or playback acceptance; progressive-only reconstruction refusal remains.

2026-10-06 MBAFF oracle preparation: generator now saves exact YUV from the independent unmodified JM 19 decoder for both owned CABAC/CAVLC three-frame streams, with executable and oracle hashes. JM decoded all three frames in each. A separate pixel/rewind acceptance test is present but explicitly ignored until native MBAFF reconstruction/prediction/deblocking is connected. Ordinary offline tests: 2 passed, 1 ignored; the passing refusal test remains distinct from acceptance. Generation: python3 scripts/generate_avc_mbaff_sample.py --x264 /path/to/x264 --jm-decoder /path/to/ldecod.exe. No FFmpeg or reference executable is invoked by ordinary tests.

2026-10-06 MBAFF context foundation: component neighbour_location translates local sample offsets with the current field row stride, then resolves ownership in the neighbour pair mode. cavlc_context derives nC from available neighbour 4x4 blocks; missing slice/block counts remain unavailable and counts above 16 are rejected. Six geometry/flag/context tests pass offline, including frame-to-field and field-to-frame luma/chroma transitions. These context helpers are not yet connected to the production MBAFF entropy reader; JM pixel acceptance remains ignored until reconstruction/prediction/deblocking are implemented.

2026-10-06 CAVLC production integration: IntraCavlcReader now derives luma DC/AC and chroma AC nC through the checked component/macroblock neighbour context path. Existing raster coefficient grids provide decoded/slice availability; the duplicate old nc formula was removed and its availability test redirected to the production context helper. 105 AVC core tests and 17 integration tests passed offline; one MBAFF pixel acceptance test remains ignored. This connects the progressive path only: full MBAFF field flags, count-grid ownership, prediction and deblocking are still pending.

2026-10-06 CAVLC macroblock ownership: coefficient-count and intra-mode contexts now use address-major macroblock storage rather than global raster grids. Production mode prediction and nC resolve geometric neighbours through the shared addressing path; PCM/inter publication and count snapshots use the same ownership. Allocated context sizes remain unchanged. 105 AVC core tests and 17 pixel/rewind/parameter/MBAFF-prefix integration tests passed offline; one MBAFF playback acceptance remains ignored. This enables subsequent pair-mode addressing, but does not yet enable interlaced entropy/reconstruction.

2026-10-06 CAVLC intra MBAFF syntax reader: new_mbaff supports explicit intra frame slices with MBAFF. Counts include both macroblocks per map unit, first_mb is converted to pair address, field flags are read at pair starts and inherited below, coefficient contexts/mode neighbours use pair geometry, and luma/chroma 4x4 plus luma 8x8 inverse scans select field order. Incomplete terminal pairs are rejected. Embedded mixed P/B MBAFF dispatch remains unconnected. Added two owned alternating-row field-coded streams (CABAC/CAVLC), independently decoded by JM; x264 reports 100% field macroblocks, and native CAVLC test confirms every parsed IDR block field flag. Both all-frame and all-field CAVLC IDRs consume 16 blocks and exact RBSP trailer. 105 core AVC + 18 integration tests passed offline; one pixel playback acceptance covering all four MBAFF streams remains ignored. No interlaced pixel/reconstruction acceptance is claimed.

2026-10-06 MBAFF sample output foundation: checked write_samples scatters reconstructed component blocks into progressive or every-other-row field layouts. Complete footprint/sample count validation precedes any mutation; invalid/truncated/out-of-plane writes leave the plane unchanged. Existing AVC PCM and reconstructed block writes now use this common checked writer in progressive mode. 106 AVC core and 18 integration tests passed offline; one MBAFF playback pixel acceptance remains ignored. Field-capable writing does not yet connect MBAFF prediction or deblocking.

2026-10-06 MBAFF prediction-edge foundation: prediction_edges reads top/left/corner samples with frame or field row stride and caller-supplied per-sample availability, validates complete block geometry and avoids out-of-plane reads. Existing progressive AVC reconstruction now uses this checked edge gatherer. Test confirms exact field row samples and absent edges when a sample is unavailable. 107 AVC core and 18 integration tests pass offline, with one MBAFF pixel playback test still ignored. Remaining full MBAFF work includes pair-aware reconstruction readiness, prediction assembly, deblocking, CABAC and inter references.

2026-10-06 MBAFF reconstruction readiness: Readiness420 stores slice-local completed 4x4 masks separately for Y/Cb/Cr and each macroblock, with known pair modes and checked memory budget. Publishing inconsistent pair modes or invalid geometry does not expose samples. Combined prediction-edge test proves completed even field rows cannot make undecoded odd rows available; reset_slice clears all availability. Nine MBAFF geometry/syntax/readiness unit tests passed offline. This readiness map is not yet connected to full picture reconstruction; MBAFF pixel acceptance remains ignored and the codec-gap goal remains incomplete.

2026-10-06 first MBAFF pixel acceptance: avc_mbaff_picture implements CAVLC complete intra slice reconstruction with filtering disabled. Uses component frame/field views, slice-local readiness, existing owned intra transforms/prediction and checked sample scatter back into frame storage. Checked budget includes output planes, temporary view and entropy/readiness contexts. New owned avc-mbaff-field-intra-unfiltered-cavlc three-IDR fixture matches every JM sample both via direct reconstruction and native MP4 playback, including two rewind passes. Production intra dispatch enables this path; filtered CAVLC refusal now reports the filtering/complete-slice limit, while CABAC/inter/multi-slice MBAFF remain incomplete. 108 AVC core + 19 integration tests passed offline; one broader filtered/inter/CABAC acceptance test stays ignored. No complete MBAFF, mixed-pair/depth coverage or throughput claim follows from this fixture.

2026-10-06 MBAFF frame/mixed acceptance: added three owned three-IDR unfiltered CAVLC streams: all-frame pairs, field-right/frame-left pairs and field-left/frame-right pairs. Native syntax test verifies exactly 0/8/8 field macroblocks out of 16 in each frame (existing all-field fixture verifies 16/16), so names alone do not imply coverage. All four unfiltered intra streams match every JM sample via direct reconstruction and native MP4 playback, with rewind repeated. Target MBAFF suite: 4 passed, 1 ignored, offline; ignored case remains filtered/inter/CABAC playback. This establishes these mixed spatial patterns at 8-bit, not every pair topology, depth or tool combination.

2026-10-06 MBAFF multi-slice intra support: unfiltered CAVLC intra reconstruction now accepts ordered multiple slices, validates pair-address coverage and resets entropy/readiness for each slice. Added owned three-IDR two-slice field-coded fixture with independent JM oracle; native playback matches every sample and rewinds identically. Target MBAFF tests: 5 passed, 1 ignored (filtered/inter/CABAC). Deblocking/inter/CABAC remain incomplete; this qualifies the saved 8-bit two-slice pattern.

2026-10-06 MBAFF deblocking groundwork: filter_line applies the existing H.264 filter to a checked eight-sample strided line; horizontal field traversal can use twice plane stride. Invalid footprint/parameters fail before mutation. Existing progressive inter/intra traversal now uses this common primitive. Test verifies exact filtered values and untouched opposite-parity rows. 109 AVC core and 20 integration tests passed offline; broader MBAFF filtered/inter/CABAC acceptance still ignored. MBAFF boundary strengths, mixed frame/field edge topology and traversal order remain to implement; no filtered MBAFF acceptance is claimed.

2026-10-06 MBAFF intra boundary strength: added shared H.264 8.7.2.1 derivation for intra/intra edges: external vertical and external frame/frame horizontal bS=4, field-involving horizontal and internal bS=3. Progressive intra grid uses the shared rule. Tests compare the resulting normal versus strong sample outputs for both field modes and mixed flags. 110 AVC core plus 17 integration tests passed offline, with broader filtered/inter/CABAC MBAFF still ignored. Mixed edge topology and traversal remain incomplete; this does not enable filtered MBAFF playback.

2026-10-06 MBAFF filtered CAVLC intra acceptance: production reconstruction now retains pair-address component QPs, field mode, transform size and slice filter controls, and runs intra deblocking after all slices are reconstructed. Its checked memory budget includes deblocking metadata. Added five owned three-IDR 64x64 filtered streams: frame, field, both horizontal mixed layouts, and two field-coded slices, with independent unmodified JM oracle and generator/tool hashes. All match every sample through native playback and two rewind passes; single-slice cases also match direct reconstruction. A same-packet deblocking-disabled comparison proves all four single-slice fixtures actually exercise filtering. Original filtered CAVLC IP fixtures now accept/reset the first intra picture, and their old refusal test checks the still-unsupported inter picture instead. 114 AVC core and 21 integration tests passed offline (one inter/CABAC MBAFF acceptance test remains ignored). This qualifies the saved 8-bit CAVLC intra patterns; inter/CABAC MBAFF, broader pair topology, high depths and tool combinations remain unqualified. Ordinary tests consume saved fixtures and never invoke the generator, x264, JM or FFmpeg.

2026-10-06 MBAFF vertical mixed topology qualification: added two owned three-IDR filtered CAVLC streams with field pairs below frame pairs and the reverse. Syntax assertions verify the actual top/bottom transition and exactly eight field macroblocks per picture, rather than relying on fixture names. Direct reconstruction and native playback match every unmodified JM sample for all three pictures, with two rewind passes; disabling deblocking on the same first packet changes output. This covers the mixed horizontal boundary's two-parity frame-top case and field-top/frame-above case at 8-bit. MBAFF suite: 6 passed, 1 ignored (inter/CABAC); compatibility: 15 multislice/parameter-update tests passed offline; fixture/oracle manifest hashes verified. Inter/CABAC, higher depths and broader tool combinations remain incomplete.

2026-10-06 High10 MBAFF intra qualification: added seven owned filtered three-IDR CAVLC fixtures at actual 10-bit input/output depth (16-bit little-endian source storage, nonzero low bits), covering frame/field, horizontal and vertical mixed topologies in both directions, and field-coded two-slice pictures. Native playback matches every independent JM sample and repeats after rewind. Tests verify SPS depths, actual per-macroblock field flags/topology, all 16 blocks per picture, filtering enabled, at least one decoded 8x8 transform in the corpus, and retained nonzero low-bit precision. MBAFF suite: 7 passed, 1 ignored (inter/CABAC); compatibility: 15 tests passed offline. Fixture/oracle hashes verified. This qualifies these High10 intra streams; MBAFF inter/CABAC, additional chroma formats/depths and wider tool combinations remain incomplete.

2026-10-06 CABAC MBAFF field-flag syntax foundation: field_decoding_flag reads one regular arithmetic bin at context 70 + condTermFlagA + condTermFlagB (H.264 9.3.3.1.1.2). Neighbour availability and skipped-pair inference remain caller responsibilities. Scripted tests cover all neighbour combinations and both flag values; saved owned CABAC frame/field IDR fixtures validate the first pair's actual arithmetic decision, with PairMode bottom inheritance consuming no additional bin. 115 AVC core and 8 MBAFF integration tests passed offline; one full inter/CABAC MBAFF pixel acceptance test remains ignored. This is prefix syntax qualification, not an enabled CABAC macroblock reader or pixel/playback acceptance. CABAC spatial contexts, pair-aware termination and coefficient field dispatch remain to connect.

2026-10-06 CABAC block-context ownership: luma/chroma coded-block flags and intra prediction modes now use macroblock-address/local-cell storage. Progressive spatial lookups translate component raster coordinates into this storage; inter/skip, PCM, 4x4 and 8x8 publications use the same indexing. Allocation sizes are unchanged. Tests cover distinct cross-macroblock luma/chroma neighbours and intra versus inter unavailable coded-context conditions. 116 AVC core and 23 integration tests passed offline; existing CABAC I/P/B, multi-slice pixel/reset/seek and parameter-update behavior is preserved. MBAFF pair-aware spatial derivation, termination and residual field dispatch remain to connect; one full inter/CABAC MBAFF acceptance test remains ignored.

2026-10-06 Shared CABAC/CAVLC component neighbour geometry: avc_mbaff::block_neighbours resolves left/top 4x4 cells into macroblock-address/local-cell ownership with frame/field row steps and subsampling. CAVLC nC and production progressive CABAC coded/mode lookups now use this common geometry. Tests verify mixed vertical luma/chroma transitions, frame-top over field-pair ownership, unavailable unknown neighbouring pair and invalid component cell rejection. 117 AVC core and 23 integration tests passed offline, one inter/CABAC MBAFF acceptance remains ignored. CABAC macroblock-level spatial contexts, pair flags/termination and field residual dispatch remain to connect; this does not enable CABAC MBAFF reconstruction.

2026-10-06 CABAC macroblock neighbour geometry: shared avc_mbaff::macroblock_neighbours resolves left/top origin ownership for progressive and mixed frame/field storage, including unknown mode and picture-edge unavailability. Production progressive CABAC type, transform-size, chroma mode, coded pattern and DC context queries now use this path with checked geometry. Tests cover both blocks of a field pair next to frame storage, frame below field and field below frame, component subsampling and intra/inter unavailable-context conditions. 119 AVC core and 23 integration tests passed offline; one full inter/CABAC MBAFF pixel acceptance remains ignored. Actual MBAFF reader pair modes, pair-aware termination and field coefficient dispatch are not connected yet. Pattern-dependent mixed-boundary context derivation still needs acceptance qualification.

2026-10-06 CABAC MBAFF pair termination rule: shared end_of_slice_flag consumes a termination bin after progressive macroblocks or MBAFF bottom blocks only; top blocks infer continuation without any arithmetic read (H.264 7.3.4 slice_data). Scripted tests cover multiple top/bottom addresses, both termination values and progressive behavior. Existing production progressive end_mb now uses the shared helper; address advances after a successful decision. 120 AVC core and 23 integration tests passed offline, one full inter/CABAC MBAFF acceptance remains ignored. The MBAFF flag is not yet enabled in the macroblock reader, so this is syntax/control-flow foundation rather than MBAFF CABAC pixel acceptance.

2026-10-06 CABAC intra MBAFF syntax reader: explicit new_mbaff accepts I frame slices with MBAFF, doubles map-unit count, converts first_mb to pair address and tracks slice-local pair field modes. Top blocks decode contexts 70-72 from known left/top pairs, bottom blocks inherit; end_of_slice is consumed only after the bottom. Macroblock/component context neighbours use pair geometry, and 4x4/8x8 residual CABAC contexts and inverse scans select field order. Saved owned CABAC frame/field IDRs each parse exactly 16 blocks with expected field flags and precise final slice termination. 120 AVC core and 24 integration tests passed offline; one full inter/CABAC MBAFF pixel acceptance remains ignored. This enables explicit syntax parsing only; production picture reconstruction still refuses CABAC MBAFF. Mixed pair/pattern contexts, high-depth CABAC streams and exact pixel reconstruction remain to qualify before dispatch is enabled.

2026-10-06 CABAC intra MBAFF reconstruction acceptance: production intra dispatch now selects the explicit CABAC reader within the common MBAFF reconstruction/deblocking pipeline. Added seven owned filtered three-IDR CABAC fixtures (frame, field, horizontal/vertical mixed pairs both directions, field two-slice). Actual syntax tests verify per-macroblock pair topology and complete picture coverage; native playback matches every independent JM sample for all three pictures and two rewind passes. Direct first-picture reconstruction also matches JM; disabling deblocking on the same syntax changes every fixture's output. Original CABAC IP fixtures now accept/reset their first intra picture, while old refusal tests check the subsequent unsupported inter picture. 120 AVC core and 25 integration tests passed offline, one full inter MBAFF acceptance remains ignored. CABAC High10, broader pattern/tool combinations and MBAFF inter prediction remain unqualified/incomplete. Generation is explicit with owned source samples, x264 CLI and unmodified JM; ordinary tests need none of those tools or FFmpeg.

2026-10-06 CABAC High10 MBAFF intra qualification: added seven owned filtered three-IDR 10-bit CABAC streams, matching the frame/field, both horizontal/vertical mixed layouts and field two-slice CAVLC corpus. Shared High10 acceptance now checks both entropy modes, actual per-block topology, full 16-block picture coverage, depth and low-bit precision; separate per-entropy counters require actual decoded 8x8 transforms in both corpora. Native playback matches every unmodified JM sample for all three pictures and repeated rewind. 25 integration tests passed offline, one full MBAFF inter acceptance remains ignored; all 37 MBAFF fixture/oracle manifest hashes verified. No codec changes were needed for these samples. Wider tool/pattern combinations and MBAFF inter prediction remain incomplete; High10 intra acceptance is limited to the saved corpus, not universal profile coverage.

2026-10-06 MBAFF inter motion-storage foundation: MotionField now stores 4x4 cells by macroblock address/local raster cell rather than the picture raster. Publication, neighbour reads and transactional save/restore use the same storage indexing; allocation sizes remain unchanged. Persistent ReferenceMotionField snapshots explicitly export picture raster order, preserving co-located B-picture lookup and per-slice reference identities. New 2x2-macroblock test checks distinct vectors in every cell, address-owned storage boundaries and all snapshot positions/identities. 121 AVC core and 25 integration tests passed offline, including existing progressive I/P/B, multislice seek/reset and MBAFF intra coverage. MBAFF motion-neighbour geometry, field/frame vector/ref-index conversion, reference-list construction and inter reconstruction remain incomplete; full MBAFF inter acceptance stays ignored.

2026-10-06 MBAFF motion-neighbour geometry: shared motion_neighbours resolves A/B/C/D (left/top/top-right/top-left) partition positions through current frame/field layout into address-owned local 4x4 cells. Partition extent/alignment are checked; unknown mode and picture-edge samples stay unavailable. Existing progressive MotionField neighbour queries now use this path while retaining slice/decoded availability. Tests cover a field partition crossing between neighbouring frame blocks, a frame top below field pairs, all four neighbour locations and invalid footprints. 122 AVC core and 25 integration tests passed offline; full MBAFF inter acceptance remains ignored. Field/frame vector and reference-index normalization, pair-aware motion publication, reference lists and inter reconstruction remain incomplete.

2026-10-06 MBAFF spatial motion normalization foundation: avc_mv::normalize_neighbour converts frame/field vertical vector units and reference indices (H.264 8.4.1.3.2): frame-to-field y/2 and ref*2; field-to-frame y*2 and ref/2. Unavailable/NoPrediction remain unchanged; same-mode values retain their units. Tests cover negative odd vector division toward zero, every field reference index 0..63, source-list limits and signed-16-bit doubling boundaries/overflow refusal. 123 AVC core and 25 integration tests passed offline, full MBAFF inter acceptance still ignored. This helper is not yet connected to motion-neighbour publication/prediction. Existing progressive predictors still restrict references to 0..31; field-aware prediction, DPB reference-list expansion and complete inter reconstruction remain to implement.

2026-10-06 Field-aware AVC spatial predictor: predict_for_field accepts already-normalized field reference indices 0..63; ordinary predict and frame mode retain 0..31. Both paths share the existing partition preference, single matching reference, top-right fallback and component-median derivation. Tests connect frame-to-field neighbour normalization to predictor selection, exercise references 62/63, top/right partition preferences, missing top-right fallback, invalid reference refusal and progressive compatibility. 124 AVC core and 25 integration tests passed offline; full MBAFF inter acceptance remains ignored. This predictor is not yet called by an MBAFF inter reader. Pair-aware motion storage/publication, field-aware skip/direct paths, DPB list expansion and inter reconstruction remain incomplete.

2026-10-06 MBAFF motion publication/neighbour API: store_mbaff publishes address-local partitions with stored field mode and permits field reference indices 0..63. It validates pair mode consistency, duplicate publication, local footprint and prevents mixing progressive storage. neighbours_mbaff combines A/B/C/D geometry, decoded/slice availability and frame/field vector/reference normalization; conflicting reader/stored modes are rejected. Tests connect actual stored frame neighbours to a field predictor, verify reverse normalization, unused-list/slice isolation, reference 63, mode-change refusal and invalid local origins. Progressive snapshots/neighbour APIs refuse MBAFF storage instead of exporting incorrect co-located data; Cell sizing is included in the existing checked budget. 125 AVC core and 25 integration tests passed offline. No MBAFF inter reader invokes these APIs yet; skip/direct, pair-aware snapshots, field DPB references and motion compensation remain incomplete.

2026-10-06 MBAFF P-skip motion foundation: p_skip_for_field validates expanded field reference indices and applies the existing zero-neighbour/median rule to normalized candidates; ordinary p_skip remains the frame wrapper. MotionField::decode_p_skip_mbaff combines pair-aware lookup, normalization and predictor, then publishes reference-zero L0 and unused L1 across the complete address-local block. Tests cover zero detection after normalization, field reference 63, invalid reference rejection, nonzero median from three stored frame neighbours, all 16 published cells, duplicate publication and unknown pair mode without mutation. 127 AVC core and 25 integration tests passed offline. Slice-reader skipped-pair inference, inter entropy/residual dispatch, field reference lists, compensation and B-direct remain incomplete; full MBAFF inter acceptance stays ignored.

2026-10-06: the original `avc-mbaff-cavlc.mp4` and `avc-mbaff-field-cavlc.mp4` IP fixtures now have complete CAVLC P-slice syntax acceptance (both P pictures, 16 macroblock addresses, field modes and RBSP end, repeated fresh readers). This is distinct from complete decoded-pixel playback acceptance, which remains ignored until MBAFF inter reconstruction is connected. Ordinary tests consume saved owned fixture bytes and do not generate fixtures or launch FFmpeg.

2026-10-06 CAVLC MBAFF unfiltered P-picture pixel acceptance: two owned 64x64 three-picture I-P-P streams, frame-unfiltered-cavlc and field-unfiltered-cavlc, use one frame reference and disabled deblocking. Explicit pre-deblocking assembly matches every independent JM Y/Cb/Cr sample across all three pictures and two fresh decode passes. Syntax assertions require 16 macroblocks per P picture, exactly 0 or 16 field blocks respectively, and actual coded inter blocks. Multi-slice picture identity checks reject mismatched frame number, PPS, POC and reference status before assembly. Focused generation --only preserves other fixture manifest records; all 39 stream/oracle hashes verified. This acceptance does not enable production inter playback: inter deblocking, mixed topology/high-depth/multi-slice qualification, B-direct and CABAC inter remain incomplete. Ordinary tests use saved bytes without generator tools or FFmpeg.

2026-10-06 CAVLC MBAFF inter deblocking acceptance: the common pair-address component walker now accepts inter/intra metadata; strength_mbaff implements H.264 8.7.2.1 mixed-mode priority and field vertical-vector threshold. P assembly retains per-cell residual flags and resolved picture/field identities, component QPs and slice filtering controls within its checked allocation budget. decode_p_slices applies this filter to Y/Cb/Cr; its unfiltered counterpart remains explicit. Two new owned 64x64 frame/field three-picture I-P-P single-reference filtered fixtures match every independent JM sample and fresh restart, and same-header pre-filter comparisons prove each P picture exercises filtering. Existing 37 intra/original IP streams remain unchanged; all 41 manifest stream/oracle hashes verified. Walker tests preserve intra output for frame/field/mixed pair modes across all three components at 8/10/12/14-bit and reject missing inter motion before any sample mutation. 145 AVC core tests passed offline. Production decoder still refuses MBAFF inter: persistent MBAFF motion snapshots/DPB dispatch, mixed/high-depth/multi-slice inter qualification, B-direct and CABAC inter remain incomplete. Full inter playback acceptance remains ignored. Normative derivation: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items (8.7.2.1).

2026-10-06 CAVLC MBAFF native inter playback admission: persistent ReferenceMotionField now has explicit address-local MBAFF storage, checked pair-mode allocation and selected reference-field parity in ReferenceMotion. snapshot_mbaff_slices resolves each slice's frame-list indices into stable DPB identities, retains expanded field indices/vectors, rejects incomplete data, duplicate/missing mappings and insufficient budget. Physical-sample at_mbaff lookup retains source field mode; ordinary progressive at/colocated explicitly refuses MBAFF data, so incomplete B-direct conversions cannot silently treat field vectors as frame vectors. Unit tests cover every sample of all-field/mixed frame/field storage, both address parities, reference index 63, identity/vector preservation and invalid inputs. Native resolved inter dispatch enables CAVLC MBAFF P assembly/deblocking and stores its snapshot in the DPB. All six saved CAVLC I-P-P streams (original frame/field with default references plus new filtered/unfiltered single-reference frame/field cases) match every JM sample through native playback and rewind. Old CAVLC refusal expectations were replaced by enabled pixel acceptance; only CABAC inter acceptance remains ignored. Full codec unit suite: 467 passed, 1 ignored, offline; selected MBAFF/multislice/parameter-update integration coverage is retained. MBAFF B/direct, CABAC inter, mixed/high-depth/multi-slice inter corpus qualification and broader AVC/HEVC/AAC gaps remain incomplete. This is CAVLC P playback admission, not complete MBAFF/all-codec coverage.

2026-10-06 CAVLC MBAFF mixed/High10/multislice inter qualification: added 12 owned filtered 64x64 three-picture I-P-P single-reference streams: four mixed pair layouts (horizontal and vertical transitions in both directions) and field two-slice at 8-bit, plus frame/field, the same four mixed layouts and field two-slice at 10-bit. Automated syntax checks require the actual pair flags at every macroblock address in all three pictures, 0/8/16 field blocks as appropriate, actual coded P blocks, ordered complete slice coverage and deblocking enabled. High10 coverage requires real 8x8 transforms in the corpus and retained low-bit output precision. Every native playback Y/Cb/Cr sample matches independent JM for all three pictures and two rewind passes. No decoder change was needed for these cases. All 53 stored stream/oracle manifest hashes verified. Ordinary tests use saved bytes and no generator tools or FFmpeg. This qualifies the saved CAVLC patterns; changing pair topology across pictures, broader partition/reference combinations, CABAC inter and B-direct remain unqualified/incomplete.

2026-10-06 CABAC inter context storage foundation: CabacMotionContexts now stores reference-index/MVD condition cells by macroblock address and local 4x4 position; progressive origin, neighbour and partition update paths translate coordinates at the storage boundary. Allocation sizes and progressive syntax decisions remain unchanged. New 2x2-macroblock test assigns distinct context magnitudes to all 64 cells, checks contiguous macroblock ownership and actual cross-block left/top lookups, and confirms slice isolation. 147 AVC core tests passed offline. Pair-aware neighbour derivation/normalization, CABAC skipped-pair flags and the mixed MBAFF reader are still pending; CABAC inter playback acceptance remains ignored.

2026-10-06 CABAC MBAFF P motion context derivation: CabacMotionContexts::new_mbaff, store_non_inter_mbaff and read_prediction_mbaff use pair geometry for A/B neighbours with address-local cells, slice isolation and consistent stored pair modes. Reference-index conditions apply H.264 9.3.3.1.1.6's special field-neighbour ref 0/1 treatment for a current frame block; vertical absolute MVD magnitudes double field-to-frame and halve frame-to-field per 9.3.3.1.1.7. Explicit reference_index_for_field allows expanded lists up to 64 while the existing progressive entry point retains 32. Tests cover both mixed directions, same modes, condition/magnitude context decisions, index 63, actual MBAFF slice isolation, invalid mode/geometry/budget and poisoned entropy failure. The existing owned CABAC frame/field IP fixtures also validate each P picture's first coded skip/field/type/motion prefix with two fresh arithmetic/context readers. These are prefix syntax tests, not full P-picture pixel acceptance. 149 AVC core and 30 selected MBAFF/multislice/parameter-update integration tests passed offline; CABAC inter playback acceptance remains ignored. Complete skipped-pair dispatch, CABAC inter residual/context qualification and assembler admission are still pending; B/direct MBAFF and broader codec gaps remain incomplete. No new media failure class was discovered in this foundation step; saved owned CABAC IP reproducer streams remain the target. Normative source: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items (9.3.3.1.1.6 and 9.3.3.1.1.7).

2026-10-06 CABAC MBAFF P playback admission: IntraCabacReader::new_context_mbaff exposes shared mixed arithmetic/residual state; InterCabacSlice::new_mbaff dispatches complete P slices with pair-aware skip/type neighbours, field flags, expanded reference/motion syntax, embedded intra and field residual scans. For a skipped top, it probes the bottom skip/field prefix on a cloned arithmetic bank before publishing top reconstruction mode; the actual bottom later consumes the original bins and checks agreement. Fully skipped pairs infer same-slice left/above mode, defaulting to frame. Pair-field metadata is borrowed alongside arithmetic without per-macroblock grid copies. Existing macroblock pair-aware termination and poisoned-error behavior remain active. Common MBAFF P reconstruction now selects CABAC/CAVLC and budgets CABAC motion/context storage; native decoder dispatch admits both entropy modes and retains their DPB snapshots. Original CABAC frame/field three-picture IP streams now match every JM pixel through native playback and rewind. Their old refusal expectation became reset acceptance, and the previously ignored full CABAC playback test is enabled.

Added three owned 64x64 three-picture CABAC fixtures: frame-inter-skipped, field-inter-skipped and field-inter-topskip. Syntax gates verify actual fully skipped pairs and their left/above/default mode inference, plus skipped top preceding coded field bottom; native output matches every JM Y/Cb/Cr sample across all three pictures and two rewind passes. Static source patterns can still produce isolated coded blocks due to lossy encoder reconstruction, so tests require the intended skipped-pair class rather than claiming all macroblocks are skipped. The topskip generator disables scene cuts to preserve the intended P pictures. Bounded payload truncations prove a failed mixed CABAC dispatcher cannot resume. All 56 fixture/oracle manifest hashes verified. Full codec unit suite: 470 passed, 1 ignored; selected MBAFF/multislice/parameter-update integration tests: 34 passed, none ignored, offline without FFmpeg. CABAC mixed/High10/multislice inter corpus qualification, MBAFF B/direct and wider codec gaps remain incomplete. Normative skipped-pair handling: H.264 7.3.4, 7.4.4 and 9.3.3.1.1.1, https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items.

2026-10-06 Mixed CABAC MBAFF coded-block-pattern regression and inter qualification: new owned mixed-inter-filtered-cabac I-P-P fixture reproduced a CABAC context failure on the first P picture (after 16 blocks, end_of_slice was not recognized and the reader reported macroblock exceeds picture). JM independently decoded all three pictures. The luma CBP context previously reused one macroblock neighbour pattern for both 8x8 rows; mixed field/frame geometry can change the owning macroblock or select the same source 8x8 block for both rows. read_pattern now resolves each external 8x8 neighbour's actual address/local block via pair-aware sample geometry before selecting its CBP bit, preserving the progressive path. Synthetic context vectors independently verify both mixed directions, the current frame bottom and unavailable neighbours. The previously failing owned fixture now matches every JM sample through native playback and rewind, proving acceptance of the specific failure class rather than a replacement refusal.

Added 14 owned three-picture filtered CABAC I-P-P cases at 8/10-bit: frame, field, four horizontal/vertical mixed transitions in both directions, and field two-slice. Tests require actual field flags and complete address coverage in all three pictures, expected 0/8/16 field block counts, two slice headers when requested, real coded inter blocks and enabled filtering. High10 corpus assertions require real 8x8 transforms and retained low-bit output precision. Every native Y/Cb/Cr sample matches independent JM and repeats after rewind. All 70 stream/oracle manifest hashes verified; generation remains explicit and ordinary tests offline without FFmpeg or generator tools. 150 AVC core and 35 selected MBAFF/multislice/parameter-update integration tests passed. CABAC/CAVLC P support is qualified for this stored corpus; MBAFF B/direct, additional temporal topology/reference/partition combinations and wider AVC/HEVC/AAC gaps remain incomplete. Normative CBP derivation: H.264 6.4.11.2 and 9.3.3.1.1.4, https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items.

Owned MBAFF B-picture fixtures (2026-10-06): eight `avc-mbaff-{frame,field}-b-{spatial,temporal}-{cavlc,cabac}.mp4` files contain three 64x64 pictures in display order I-B-P (coded I-P-B), with saved independent JM YUV. Generation uses the existing owned pattern, x264 with one fixed B picture/no B pyramid/one frame reference and explicitly selected direct mode; no private media or FFmpeg. Automated syntax gates require actual MBAFF SPS, exactly one B picture, matching direct mode, real direct partitions, complete macroblock address coverage and field blocks in the field cases. Native software playback matches every display-order Y/Cb/Cr sample across all three pictures and two rewind passes. These qualify this 8-bit frame/field corpus; mixed layouts, High10, multislice, more partitions/references and changing pair topology need further B qualification. Generation remains separate from ordinary offline tests.

Extended MBAFF B corpus (2026-10-06): added 48 moving I-B-P streams for horizontal/vertical mixed frame/field transitions in both directions, field multislice, and frame/field/mixed/multislice High10, each with spatial/temporal direct and CAVLC/CABAC. Four additional static temporal field-multislice streams at 8/10-bit supply actual B-skip/direct where the moving encoder chooses only explicit biprediction. The moving cases remain checked independently; a direct-mode flag alone is never counted as direct acceptance. The 60 B fixtures together require direct syntax for every topology/mode/depth/entropy key. Syntax gates now verify exact pair modes/address coverage and slice counts in all three I/P/B coded pictures, implicit B weighting in PPS, genuine High10 B 8x8 transforms somewhere in the corpus and retained low-bit precision. Every byte matches saved JM display-order pixels, including rewind. Ordinary tests require no encoder, decoder executable, FFmpeg or network.

MBAFF B-pyramid/reference corpus (2026-10-06): 24 moving nine-picture streams (frame/field/mixed × 8/10-bit × spatial/temporal × CAVLC/CABAC) use fixed three-B groups, normal B-pyramid and three configured references. Syntax/DPB gates require I=1/P=2/B=6, two reference B pictures, multiple active references, actual nonzero frame indices in coded B partitions, and selection of a retained B as L1[0]. Every topology/depth/entropy/requested-mode category must actually invoke direct from that B's motion. The encoder can signal spatial on some B pictures even when temporal was requested; tests inspect each actual header rather than assuming the requested mode. Three additional mostly-static temporal cases (field High10 CABAC and mixed 8/10-bit CAVLC) provide actual direct-from-B coverage where the moving cases choose explicit predictors; moving cases remain independently checked. Static cases retain moving anchor pairs to signal the intended field modes. All output pixels, exact consecutive presentation ticks/unit durations and rewind match the saved nine-picture oracle/MP4 timeline. Generation is explicit, offline tests use only owned saved bytes.

2026-10-06 Changing MBAFF topology and owned temporal-direct qualification: eight nine-picture x264/JM streams switch pair modes across pictures at 8/10-bit with CAVLC/CABAC and spatial/temporal encoder requests. Every saved pixel, presentation timestamp and rewind is checked. Actual direct-from-B coverage is required for the changing spatial categories; temporal requests alone do not qualify the changing temporal categories.

Eight additional owned 16x32 five-picture CAVLC streams force nonzero temporal direct from retained B motion across both field-to-frame and frame-to-field transitions, at 8/10-bit with implicit or explicit B weights. The new generate_avc_mbaff_direct_samples.py writes SPS/PPS, PCM and P/B syntax itself; JM independently supplies saved pixel oracles. Tests verify reference identities, pair modes, motion syntax, actual direct partitions, weight tables, clipping, every output sample and rewind. Ordinary tests use saved bytes only. The combined manifests contain 173 stream/oracle pairs. Changing-mode CABAC temporal direct, separate field pictures and broader AVC/HEVC/AAC capabilities remain unqualified or incomplete.

2026-10-06 Owned CABAC cross-mode temporal direct qualification: the independent PCM/header generator now implements a short integer CABAC interval writer using normative probability/transition/context initialization tables. It keeps the previously qualified CAVLC I/P/reference-B anchors and selects a second CABAC PPS for the non-reference temporal-direct B target. Thirty-two new 16x32 five-picture streams cover frame-to-field/field-to-frame, 8/10-bit, implicit/explicit B weighting, cabac_init_idc 0/1/2, and a skipped top macroblock whose pair mode is signalled by the coded bottom. JM independently decodes every generated stream; ordinary acceptance uses saved bytes and no external codecs/network.

The CABAC syntax gate requires actual temporal B-direct partitions, exact target pair mode, complete pair termination and the expected skipped-top variant; metadata-only POC/DPB traversal requires retained B picture 2 as L1[0] and I picture 0 as L0[0]. Native playback matches every JM Y/Cb/Cr byte across all five display-order pictures, exact presentation ticks and rewind. All 205 combined stream/oracle manifest pairs verified. Generator debugging corrected its table parser and CABAC terminal bit placement; no new production decoder failure was found. This closes the previously unqualified owned cross-mode CABAC temporal-direct case, including bottom-signalled mode probing, but not universal AVC support: separate field pictures, FMO/ASO, mixed slice types, wider chroma/profiles and HEVC/AAC gaps remain.

2026-10-06 CABAC retained-B motion and higher-depth cross-mode qualification: the owned interval writer now emits B_L0_16x16 reference pictures with nonzero MVD (8,4), field reference index 1, pair-mode context selection, unary motion bins and bypass signs. Thirty-two new 8/10-bit streams retain that CABAC B motion before the CABAC temporal-direct target, covering all three cabac_init_idc banks, explicit/implicit weights and bottom-signalled pair mode after a skipped top. Source syntax gates now inspect actual entropy, L0 prediction/reference index, both MVDs, pair mode, exact coverage/termination and retained B identity, in addition to target direct and full pixel/rewind acceptance.

The same corpus was expanded to 12/14-bit equal-depth 4:2:0 using profile 244 (High 4:4:4 Predictive), with owned PCM gradients retaining lower-bit precision. These are 4:2:0 prediction/PCM qualifications under that profile, not 4:4:4-chroma or residual-tool acceptance. Combined owned direct manifest: 144 streams (16 CAVLC targets, 128 CABAC targets), 104 newly added since the previous 40-case step. All generated streams decoded independently with JM; every FVid output byte and rewind matches. Native tests also require profile/depth/chroma identity, in-range oracle samples and nonzero low-bit precision. All 309 combined stream/oracle manifest pairs verified. No production decoder change was required; separate field pictures, mixed slices, FMO/ASO, wider chroma/residual/profile tools and HEVC/AAC gaps remain.

2026-10-06 Native mixed MBAFF I/P slice admission: a new owned 32x32 three-picture reproducer first failed on picture 1 with "invalid MBAFF P reconstruction configuration" because the common assembler excluded I slices. After intra dispatch was connected, the same stream exposed a second refusal, "MBAFF P slices belong to different pictures": I/P slices had nal_ref_idc 2/3, which are both reference slices in one picture. H.264 7.4.1 and 7.4.1.2.4 distinguish reference identity only when one value is zero; the assembler now uses that same zero/nonzero rule already applied by access-unit preparation.

The common MBAFF reader now dispatches I slices through owned intra CAVLC/CABAC readers and publishes them through existing shared intra reconstruction, readiness, no-prediction motion and deblocking metadata alongside inter slices. The public resolved reconstruction facade routes MBAFF I/P slices to this common assembler. The new generate_avc_mbaff_mixed_samples.py writes owned PCM, nonzero P motion and slice headers; its explicit JM generation is separate from ordinary tests. CABAC generation supports I_PCM arithmetic restarts retaining adapted contexts and owned P_L0_16x16 motion. No private media/parameters or FFmpeg are used.

Forty-eight saved acceptance streams cover I/P and P/I order, horizontal frame/field and field/frame pairs, 8/10-bit, CAVLC/CABAC and deblocking idc 0/1/2. Syntax gates require actual I_PCM/P motion, pair modes, MVD/reference indices, complete slice address/termination and the intended filter mode; CAVLC cases require distinct nonzero NAL reference priorities. The mixed picture is retained and used by a subsequent P picture. Every JM output byte matches native display-order playback and rewind; the mixed output also matches through the public reconstruction API. All 357 combined manifest pairs verified. This is mixed I/P PCM/prediction acceptance, not mixed B/intra residual qualification or universal AVC coverage. Separate field pictures, FMO/ASO, additional chroma/profiles/residual tools and HEVC/AAC gaps remain.
Normative reference: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items.

Mixed I/P filter fixtures use P-slice QP 50 to activate the I/P boundary. The acceptance gate compares paired saved JM idc-0/idc-2 oracles: PCM anchor bytes must agree and the mixed picture itself must differ, for every topology/order/depth/entropy category. Differences only in a later picture do not qualify this path.


Mixed MBAFF I/B, B/I, P/B and B/P qualification adds 192 owned streams (240 mixed streams total), including retained motion, temporal direct, 8/10-bit, CAVLC/CABAC, explicit/implicit weighting and active cross-slice filtering. Every saved JM output byte matches native playback and rewind.

Frame-number gap work remains a foundation: DPB stores non-existing reference slots without allocating pixels, POC types 1/2 infer history, and type 0 excludes unknown-POC gaps from B lists. Nine new unit tests pass. The native decoder still explicitly refuses actual gaps until reference availability is connected; 24 owned gap fixtures reproduce that specific refusal, not playback acceptance. These fixtures and generators contain no private media and ordinary tests require neither FFmpeg nor JM.


Native frame-number gap admission now connects the owned DPB/POC foundations. SPS permission is required; every missing frame number occupies a non-existing sliding-window slot with no image allocation, receives a unique reference ID, and updates POC types 1/2. All 24 saved frame/field-MBAFF, 8/10-bit, CAVLC/CABAC and POC 0/1/2 streams now pass byte-exact JM acceptance for all three pictures over two decoder resets. Syntax gates require frame_num 0/3/4. The former refusal regression is replaced by this acceptance test.

Remaining gap limitation: active reference lists containing unused non-existing slots are still refused during view resolution. Prediction from non-existing pictures remains invalid; supporting unused slots requires optional reference availability in reconstruction and direct-motion paths. This change does not claim complete gap admission or separate field-picture support.


Unused gap reference admission: optional picture views preserve non-existing active slots through progressive/MBAFF reconstruction. Twenty-four additional owned streams retain two unused missing references alongside the real IDR (48 gap streams total). Syntax gates require three active L0 entries; every output pixel matches JM across two resets, 8/10-bit, CAVLC/CABAC, frame/field MBAFF and POC 0/1/2. A separate owned two-picture CAVLC fixture deliberately selects a missing frame and must fail specifically at prediction, then require reset. No synthetic pixels are substituted for missing references.

MBAFF B lists with missing entries still need stored inferred per-field POCs; co-located missing references are rejected instead of being treated as intra pictures. Separate field pictures, FMO/ASO and broader profile/tool gaps remain.


MBAFF B gap admission now retains exact inferred top/bottom field POCs in the DPB. Thirty-two owned I/B/P streams cover explicit L0 and temporal direct, 8/10-bit, CAVLC/CABAC, frame/field pairs and POC types 1/2. Both active lists include the real IDR and two non-existing slots; output matches every JM pixel over two resets. Direct/explicit output differs in all 16 paired categories. Missing co-located pictures are explicitly rejected; absent images are never treated as intra. A DPB test preserves asymmetric field POCs, rejects partial-frame metadata atomically and verifies eviction.

The new non-reference B transition also exposed and fixed PrevRefFrameNum history: inferred gap frames advance the previous reference number even when the current picture is non-reference, preventing duplicate inference before the subsequent P with the same frame_num. These fixtures reproduce that exact transition. Remaining codec coverage is not complete: separate field pictures, FMO/ASO and further profiles/tools still require implementation and acceptance.


ASO admission for single-group AVC: access-unit preparation validates all picture identities, normalizes slices by first_mb, rejects duplicate starts and missing macroblock zero, then computes reconstruction ranges before updating POC/DPB. Wire order is no longer required to be raster order. The former reversed-slice refusal test now verifies normalized ranges and byte-identical progressive I/P/B reconstruction.

Seventy-two new owned reversed-order MBAFF fixtures cover I/P, P/I, I/B, B/I, P/B, B/P, 8/10-bit, CAVLC/CABAC and deblocking idc 0/1/2. Every packet must contain addresses [1,0] on wire and normalize to [0,1]. All JM display-order pixels match native decode over two resets, including retained mixed pictures and temporal direct. Their JM outputs equal the corresponding independently saved raster-order outputs. The generator explicitly invokes JM separately; ordinary tests remain offline with no FFmpeg/JM. This does not implement FMO address maps or separate field pictures.


FMO CAVLC reader integration begins with an explicit syntax-only new_fmo constructor. It derives and expands the real PPS map and uses NextMbAddress after PCM and coded intra blocks, retaining coefficient/mode availability at physical addresses. Existing playback constructors still refuse FMO until reconstruction is connected. Two compact owned progressive explicit-map I_PCM fixtures (ordinary and reversed slice order) use groups [0,1,0,1]; readers must emit [0,2] and [1,3], terminate each slice, cover all four addresses exactly once and reproduce every saved JM PCM pixel. This is syntax acceptance plus a separate passing playback refusal, not FMO playback acceptance. Generation is separate and invokes JM explicitly; tests use only saved fixtures, no external decoder or private data.


Native progressive FMO I-picture reconstruction now tracks coverage and slice ownership by physical macroblock address, preserves per-slice prediction availability, and invokes the FMO reader. Deblocking receives explicit owners instead of inferring boundaries from first_mb ranges; idc 2 compares real slice ownership. Owner/map scratch storage is included in picture memory accounting. Slice identity validation now includes slice_group_change_cycle.

Twenty owned I_PCM fixtures cover map types 0–6, both changing-map directions and both slice wire orders. Every saved JM pixel matches AvcDecoder over two resets. The old native refusal in the explicit-map test is replaced with pixel acceptance; the legacy syntax constructor remains separately restricted. A missing-group regression verifies that no partial picture is returned, errors require reset and a valid frame succeeds after reset. This qualifies progressive 8-bit I_PCM FMO, not all intra prediction/residual tools, filtered FMO pixel cases, MBAFF FMO, or P/B FMO. These broader paths remain open and require their own fixtures.


Progressive FMO I16 DC/residual/filter qualification: 120 owned one-picture PCM/I16 streams cover all map types 0–6, both changing-map directions, forward/reversed slice order, zero/one positive luma DC coefficient and deblocking idc 0/1/2. Syntax gates require two PCM and two actual I16 DC blocks, exact DC coefficient count/sum, QP 50 and active +12 offsets. Every JM output pixel matches the native decoder over two resets. All 40 idc 0/2 paired categories require equal parsed PCM anchors but distinct saved output pixels, proving active cross-slice filtering. No production code changes were needed: the owned FMO reconstruction/filter-owner implementation passes this expanded acceptance.

This extends the former PCM-only qualification; it does not yet qualify arbitrary intra modes, AC/chroma residual combinations, higher depths, MBAFF FMO or P/B FMO. Ordinary tests consume saved fixtures only; the explicit JM generator uses no private media or FFmpeg.


FMO P/B CAVLC reader foundation: new_fmo derives the real map, skips foreign-group addresses after coded/intra/skip blocks, and validates skip run against a precomputed same-group suffix count. Remaining-group lookup is O(1), and added map/count context is charged to the reader budget. FMO mixed intra context uses the same physical address map. Ordinary constructors retain their previous behavior.

Two compact owned Extended-profile I/P/B fixtures (forward/reversed slice order) retain two references, include nonzero P/B MVDs and per-group skip runs; JM decodes all three pictures. Syntax acceptance requires correct picture types, coded MVD [8,4]/[4,0], zero coefficients, addresses [0,2]/[1,3], complete coverage and termination. A third owned malformed fixture requires the exact skip-run-out-of-group refusal and reader poison. A separate native refusal test documents that FMO P/B reconstruction remains unsupported; saved JM pixels are not yet counted as native playback acceptance. All 20 earlier PCM streams remain byte-identical after optional profile/POC/reference configuration was added to the generator.


Native progressive FMO P/B reconstruction now tracks unique coverage at physical addresses, writes loop-filter metadata into physical row/column slots and uses the FMO inter reader. FMO rows are published to reconstruction workers only after successful complete parsing; ordinary raster row pipelining remains enabled. Motion/prediction availability still follows each slice's identity, and replay preserves intra/inter decode order. The old playback refusal is replaced by full acceptance.

Six owned Extended-profile I/P/B streams cover forward/reversed order and deblocking idc 0/1/2. Every JM display-order pixel matches native decode over two resets. Filtered streams use QP 50/+12 offsets and distinct group MVDs; both idc 0/2 pairs require identical IDR pixels but different B and P top-row luma pixels. Initial identical-motion fixtures had zero boundary strength; final fixtures deliberately activate cross-slice filtering instead of weakening the gate. A malformed skip-run fixture must fail at group bounds through syntax and native decode, poison state, and allow a valid IDR after reset. This qualifies explicit-map progressive 8-bit motion/skip paths; other map types, residual/intra combinations, depths and MBAFF FMO still need acceptance.


FMO P/B map-type qualification now includes all map types 0–6 and both changing-map directions: 60 owned three-picture I/P/B streams cover both slice orders and filter idc 0/1/2. Groups may contain one, two or three macroblocks; skip runs derive from group length instead of assuming adjacent raster addresses. Syntax gates verify SPS Extended profile/two references, actual P/B types, each group's coded MVD, remaining skip addresses, unique complete four-macroblock coverage and termination. Every saved JM pixel matches native decode across two resets. All 20 filtered pairs require equal IDR but distinct B/P top-row luma output, proving active cross-slice filtering for every map topology.

Scope correction from normative H.264 A.2: Extended FMO uses 8-bit 4:2:0 and CAVLC; High-family profiles constrain num_slice_groups_minus1 to zero. Higher-depth/CABAC FMO combinations are not standard High-family bitstreams and should not be presented as missing FMO conformance. Valid MBAFF/field FMO, additional group counts, intra/residual tools and the broader non-FMO profile gaps still remain. Qualification here covers two-group progressive motion/skip FMO only.


Three-to-eight-group FMO qualification adds 144 owned 64×64 three-picture Extended-profile I/P/B fixtures for map types 0/1/2/6, every group count 3–8, forward/reversed group wire order and filter idc 0/1/2. Actual coded motion and all remaining skipped physical addresses are gated against each PPS map; every picture must cover all 16 addresses exactly once and terminate every slice. Every JM pixel matches native playback over two resets. All 48 filtered pairs require identical IDR pixels and distinct B/P luma planes. This exercises the highest group ID, non-power-of-two explicit maps, single-MB foreground groups and long skips in the remainder group. Changing map types 3–5 are defined only for two groups and retain their prior two-group acceptance.

No production fixes were required for these additional group counts. FMO residual/intra combinations and MBAFF/field pictures remain open, alongside broader codec/profile tools. Ordinary tests consume saved synthetic clips/oracles without FFmpeg, JM or network access.


Progressive mixed FMO inter/intra residual acceptance adds 60 owned Extended-profile I/P/B streams across map types 0–6, both changing-map directions, both slice orders and filter idc 0/1/2. P/B slices contain a coded motion block followed by an embedded I16 DC block with one positive luma DC coefficient (and remaining group skips where present). Syntax gates require actual P/B types, two coded inter blocks, nonzero I16 DC coefficients and unique complete physical coverage. All display-order JM pixels match native decode over two resets. All 20 filter pairs require identical PCM IDR anchors and distinct B/P luma output between idc 0 and 2.

Generation explicitly invokes restored official JM 19.0 outside ordinary tests; saved fixtures are synthetic and have verified hashes. No production fix was needed. This qualifies the specific mixed motion/I16 DC combination; arbitrary AC/chroma residuals, additional intra modes, changing-map rates/cycles, MBAFF/field FMO and broader codec/profile tools remain unqualified. Ordinary tests use saved fixtures without FFmpeg, JM or network.


Mixed progressive FMO AC/chroma acceptance adds 180 owned Extended-profile I/P/B streams covering all map types 0–6, changing-map directions, forward/reversed slice order and deblocking idc 0/1/2. Three residual modes exercise embedded I16 luma AC (16 nonzero coefficients), chroma DC (two nonzero coefficients), and combined luma/chroma AC (16 luma, two chroma DC and eight chroma AC coefficients). Luma/chroma AC signs alternate. Syntax checks require the exact coefficient counts, signed luma AC, real P/B slice types, coded inter blocks and complete unique physical coverage. Every display-order pixel matches independent saved JM output over two resets. All 60 filter pairs preserve the IDR anchor and change both B/P luma planes between idc 0 and 2.

No production fix was needed. The generator is owned and invokes JM explicitly only during generation; tests consume saved synthetic fixtures and require no FFmpeg, JM or network. This covers single nonzero first-AC levels within embedded I16 blocks, not arbitrary levels/scan runs, every intra prediction mode, residual-bearing inter blocks, changing-map rates/cycles, MBAFF/field FMO, or complete codec/profile conformance. Those remain open.


Progressive FMO inter residual acceptance adds 180 owned Extended-profile I/P/B streams with real motion and signed residuals in coded inter macroblocks, followed by group skip runs. Modes cover 16 nonzero luma 4x4 coefficients, two nonzero chroma DC coefficients, or their combination with eight nonzero chroma AC coefficients. Every map type 0–6, changing-map direction, slice wire order and filter idc 0/1/2 is exercised. Syntax gates require coded inter dispatch (no embedded intra), nonzero MVD, exact residual counts/signs and unique complete coverage. Every JM display-order sample matches native decoding across two resets. All 60 idc 0/2 pairs retain equal IDR anchors and change both B/P luma planes.

No production fixes were required. These use one nonzero first coefficient per selected block with alternating signs; arbitrary levels, scan positions/runs, additional prediction/partition modes, changing-map rates/cycles, MBAFF/field FMO and broad codec/profile conformance remain open. Generation invokes JM separately; ordinary saved-fixture tests require no FFmpeg, JM or network.


MBAFF FMO intra admission: an owned 32x64 explicit-map Extended-profile PCM stream reproduced the exact native refusal "intra CAVLC reader requires progressive 4:2:0 I slices without FMO". The MBAFF intra assembler now selects the FMO reader, verifies unique complete physical coverage and stores filter metadata by physical macroblock address instead of parse order. Coverage storage is charged to the picture budget. Ordinary raster range checks remain enabled outside FMO. The CAVLC reader records the last emitted address so field_decoding reports that pair's mode after a non-raster group jump rather than inferring it from next_address minus one.

Eighteen synthetic PCM streams exercise four pairs with map [0,1,0,1], frame/field/mixed pair modes, both wire orders and filter idc 0/1/2. Syntax gates require exact physical addresses [0,1,4,5]/[2,3,6,7], actual PCM, each pair's field mode, complete unique coverage and termination. Every JM output sample matches native decode over two resets. PCM-only streams qualify filter metadata/configuration, not active residual filtering. MBAFF FMO predicted/residual intra, P/B, other maps and separate field-picture admission still require acceptance. Generation explicitly invokes JM separately; ordinary tests require no external decoder, FFmpeg or network.


MBAFF FMO predicted intra qualification adds 144 owned 32x64 Extended-profile explicit-map streams. The first two pairs contain PCM anchors; the later two pairs use I16 DC/vertical luma and DC/vertical chroma prediction with zero or one positive luma DC coefficient at QP 50. Frame, field and mixed pair layouts, both wire orders and filter idc 0/1/2 are covered. Syntax checks require exact physical pair addresses/modes, PCM versus I16 dispatch, luma/chroma prediction modes, exact DC count/sum, QP and complete termination. Every saved JM sample matches native decoding over two resets. All 48 idc 0/2 pairs change output; 12 unfiltered pairs independently require luma and chroma prediction-mode changes to affect their respective planes.

No additional production fixes were required beyond the preceding physical-address MBAFF FMO admission. This qualifies DC/vertical prediction with first-DC residuals, not arbitrary intra modes/AC/chroma residuals, other maps, P/B FMO, or separate field pictures. Fixtures and saved references are synthetic; explicit JM generation remains separate from ordinary offline tests without FFmpeg/JM/network.


MBAFF FMO inter admission connects the common intra/inter assembler to FMO CAVLC readers. Owned three-picture streams first reproduced the common reader's FMO refusal. The assembler now stores deblocking metadata by physical address, validates unique complete coverage, retains ordinary raster checks outside FMO and includes additional FMO reader/coverage storage in its budget. InterCavlcSlice records its last emitted address for field_decoding across group jumps, matching the earlier intra-reader correction.

Eighteen Extended-profile I/P/B streams cover four explicit-map pairs [0,1,0,1], frame/field/mixed modes, both wire orders and filter idc 0/1/2. They retain PCM IDR and motion P references; B pictures use explicit L0 motion. Syntax gates require P/B dispatch, every physical address, MVDs, pair modes, zero coefficients, unique coverage and termination. Every JM display-order sample matches native decode across two resets. Six idc 0/2 pairs require equal IDR and distinct B/P luma output. This qualifies explicit motion without residual/direct/skip mixtures; remaining map types, MBAFF FMO residual/intra/direct combinations, separate field pictures and broader codec tools remain open. Ordinary tests use saved owned fixtures without FFmpeg/JM/network.


MBAFF FMO skip/direct qualification adds 18 owned Extended-profile I/P/B streams with coded nonzero-motion top macroblocks and P-skip/B-skip bottom macroblocks in each pair. Explicit map [0,1,0,1], frame/field/mixed pair modes, both wire orders and filter idc 0/1/2 are covered. Syntax gates require exact coded/skip parity and physical addresses, actual P/B slices, top MVDs, pair modes, complete coverage and termination. Every JM display-order sample matches native decoding over two resets. Six filter pairs preserve IDR anchors and change B/P luma; every B luma output differs from its corresponding previously saved explicit-motion stream, isolating active temporal direct behavior.

The initial writer incorrectly emitted a second mb_skip_run before the coded block following a skip. It was corrected before fixture acceptance; the final JM generation completes all pictures. No production fixes were needed beyond the preceding physical-address common-assembler and field-mode corrections. Top-skip/whole-pair skips, explicit/spatial direct, residual/intra mixes, other FMO maps and separate field pictures remain open. Ordinary saved-fixture tests require no FFmpeg/JM/network.


MBAFF FMO inter residual qualification adds 54 owned Extended-profile I/P/B streams for luma 4x4 residuals, chroma DC, and combined luma/chroma DC+AC. Every coded macroblock has the intended counts (16/0/0, 0/2/0 or 16/2/8), alternating residual signs, explicit motion and exact physical pair modes. Frame/field/mixed layouts, both slice wire orders and filter idc 0/1/2 are covered. Every saved JM display-order sample matches native decode over two resets, including B prediction from the retained residual-bearing P frame. Eighteen filter pairs require equal IDR anchors and distinct B/P luma planes.

No new production fixes were needed. This qualifies single nonzero first coefficients in each selected block and opposite signs across top/bottom blocks; arbitrary levels/runs, additional partitions, residual-bearing direct/skip/intra combinations, remaining maps and separate field-picture admission remain open. Generation invokes JM explicitly outside ordinary offline tests, which require no FFmpeg/JM/network.


Separate-field PCM reconstruction foundation: four owned 32x32 streams contain two complementary field pictures, 8/10-bit and top-/bottom-first order. The first field is IDR with POC zero; the second is a non-IDR I field of the same frame_num with POC one, preserving the pair rather than flushing the DPB with a second IDR. Native stateful playback specifically refuses them at the progressive-only intra reader.

The new owned avc_field_picture module reconstructs single-group CAVLC PCM into compact field planes and weaves complementary fields into a full frame with parameter/frame/parity/dimension validation and explicit output/coverage memory accounting. Every independent JM pixel matches the reconstructed/weaved fields in either argument order. Tests require actual field/parity/IDR syntax, exact memory boundaries, rejection of equal-parity pairs and explicit non-PCM refusal. This is reconstruction acceptance only: the test separately retains the specific passing AvcDecoder refusal. Field reference-list construction, DPB marking, pairing/display scheduling, non-PCM intra/inter reconstruction and full playback acceptance remain unfinished. No FFmpeg/private media are used; explicit JM generation is separate from offline tests.


Field reference-list foundation: the owned avc_field_references module builds P/B lists from complete or unpaired frame stores, sorting stores first and alternating current/opposite parity with independent cursors. P uses FrameNumWrap; B uses the store's minimum field POC, including the equal-current-POC partition, and swaps the first two L1 fields when both initial lists are equal. Field PicNum/LongTermPicNum use same-parity +1, doubled wrap/modification ranges and prefix-preserving list changes. Configuration is bounded to 16 stores/32 fields, validates IDs, available fields, frame numbers and long-term indices, and refuses missing selections.

Four tests cover asymmetric unpaired stores, frame-number wrap, frame POC versus individual-field order, equal B lists, same/opposite parity, long-term changes, intentional modified-prefix duplicates, modulo cycles and invalid/missing references. The four existing complementary PCM field fixtures also exercise parity list selection from their actual parameter/frame metadata. This is a list algorithm foundation, not field playback acceptance: DPB field marking and list integration remain disconnected. The API currently represents one shared short/long-term status per frame store; mixed marking within a pair must be added before complete conformance. Separate-field non-PCM reconstruction/display integration and broad codec gaps remain open.


Independent field marking is now represented in the field-list foundation: each parity has its own optional long-term index. Short-list expansion selects only short fields while preserving frame-store POC sorting; long lists independently sort each parity by its long-term index and alternate them. Duplicate long-term indices are rejected within the same parity, while opposite-parity reuse is retained. Modification lookup uses the selected field's own status/index. Two new tests cover mixed pairs, distinct long indices in one pair, asymmetric parity order, shared store POC, short/long changes, opposite-parity index reuse and absent marked fields.

Four additional owned complementary-field streams (8/10-bit, both first-field orders) place actual MMCO 3 in the second field header, converting the first opposite-parity field to long-term index 2 while retaining the current field as short-term. Syntax gates require that exact operation; reconstruction/weaving matches every JM sample and the list tests select the intended parity for short and long references. Eight saved field streams now qualify reconstruction. This does not implement DPB MMCO execution or full field playback: AvcDecoder's progressive-only reconstruction refusal remains separately asserted. The prior shared-marking-only restriction of the list module is superseded, but DPB integration, field prediction, non-PCM reconstruction and display pairing remain unfinished.


Field DPB foundation: avc_field_dpb stores reference fields independently, resolves field selections and executes MMCO 1–6 transactionally before committing references/limits. Complementary reference fields share a stable store ID; only the immediately pending store can receive its other parity, avoiding accidental pairing with an older wrapped frame_num. Short/long marking is per field. Sliding-window qualification covers complete short-term pairs and unpaired fields; full field/frame mixed storage and all mixed-store eviction cases still need conformance acceptance.

The eight PCM field fixtures now drive actual field DPB marking and list lookup. Their mixed-long headers were corrected: SPS max_num_ref_frames is three and MMCO 4 sets MaxLongTermFrameIdx before MMCO 3 converts the opposite field to index two. The previous JM-decodable sequence lacked that limit establishment and was not sufficient conformance evidence. Exact syntax gates now require MMCO 4+3. Tests exercise conversion, current-long replacement, parity-specific long/short forgetting, reset, pair completion without sliding eviction, eviction of an older complete short pair, refusal when only long references remain and preservation of all references after invalid operations. PCM reconstruction/weaving still matches JM.

This is field DPB/list execution acceptance, not AvcDecoder playback acceptance. Native field reconstruction admission, sample/motion reference integration, non-PCM prediction, mixed frame/field DPB streams, POC/display pairing and memory integration remain unfinished. Ordinary tests are offline without FFmpeg/JM/network; fixture generation invokes JM separately.

`avc-field-pcm-unpaired.mp4` is an owned single top PCM field generated by `generate_avc_field_pcm_samples.py`. It exercises an incomplete complementary pair at EOF, not container truncation. Playback must report `unpaired AVC field at end of MP4`; paired field fixtures now additionally require first-field PTS/sample identity and the combined duration in playback and owned media decode. Ordinary tests use saved fixtures only.

`avc-field-skip-{8,10}bit-{top,bottom}-first.mp4` contains two PCM I fields followed by two full-run P-skip fields. `generate_avc_field_skip_samples.py` owns all parameters and syntax; an explicitly supplied JM decoder generates the saved two-frame pixel oracle. Acceptance checks both native field references and software playback/rewind, first-field timestamps, and allocation boundaries. These fixtures do not qualify coded motion/residual, B, weighted or opposite-parity prediction.

`avc-field-motion-*` are sixteen owned complementary I/I/P/P field streams generated by `generate_avc_field_motion_samples.py` with explicit separate JM pixel oracles. Signed quarter-sample motion covers both parity orders and 8/10-bit precision; each P field contains two explicit 16x16 blocks, no residual and deblocking disabled. Tests gate actual syntax, require changed output, compare every sample after reset/rewind and check decoder memory boundaries. Generation requires the explicitly supplied JM decoder; ordinary tests use only saved files.

`avc-field-partition-*` are forty owned streams from `generate_avc_field_partition_samples.py`: P16x8/P8x16 plus P8x8/P8x8ref0 and every subdivision size, 8/10-bit, both field orders. Saved JM oracles qualify partition-specific motion/interpolation, native reset, playback rewind and presentation timing. Syntax gates require the intended partition count, dimensions and signed MVDs. Residual and filtering are disabled and are not qualified by these fixtures.

`avc-field-residual-*` from `generate_avc_field_residual_samples.py` contains twelve signed luma-AC/chroma-DC/all-AC acceptance streams and four zero-residual controls, 8/10-bit with both parity orders. Controls retain identical motion/skip syntax to isolate residual effects. Tests gate actual coefficient counts/signs and field scan positions, compare every saved JM sample after reset/rewind, and check media presentation. Filtering is disabled. Fixture generation is explicit and separate from offline tests.

`avc-field-transform-*` from `generate_avc_field_transform_samples.py` are forty-eight owned High/High10 streams: 4x4/8x8 transform, QP18/26/40 with mb_qp_delta -3/+3, both field orders and optional explicit PPS scaling lists. `-scale24` selects inter matrices of twenty-four versus flat-sixteen controls. Tests gate actual syntax/matrices and every JM pixel after reset/rewind; QP40 paired controls require visible transform/scaling effects. Generation explicitly uses JM, ordinary tests only read saved fixtures. Filtering remains disabled.

`avc-field-bypass-*` from `generate_avc_field_bypass_samples.py` comprises thirty-two owned QP-prime-zero inter-field streams and quantized controls. Profile244 uses 4:2:0; dimensions/parameters are synthetic. 8/10-bit, 4x4/8x8, both parity orders and flat/custom matrices qualify bypass selection and scaling independence with exact saved JM pixels after reset/rewind. Tests require enabled output to ignore matrix changes and differ from disabled-bypass controls. Generation and JM use remain separate from ordinary tests.

`avc-field-filter-*` from `generate_avc_field_filter_samples.py` supplies thirty-six owned QP50 streams: both parity orders, 8/10-bit, motion-only/luma-AC/all-AC and filter0/1/2. Enabled outputs must visibly differ from filter1 controls; filter2 must match filter0 within the single slice. The motion-only 16x8 macroblock differs vertically by two quarter-field samples, gating the field motion threshold. Every saved JM pixel is compared after native reset and playback rewind, including memory admission at the filtered field boundary. Broader multi-slice/multi-row and mixed intra/inter filtering remains to qualify.

`avc-field-multislice-*` from `generate_avc_field_multislice_samples.py` contains forty-eight owned two-slice P-field streams, 8/10-bit/both field orders, motion/all-AC and filter0/1/2. `-aso` reverses NAL order. The two slices use nal_ref_idc two/three; both remain reference slices. Tests gate actual wire addresses, slice-local motion/coefficient syntax, cross-slice filter suppression, ASO pixel invariance and every saved JM pixel after reset/rewind. Ordinary tests do not run the generator or JM.

`avc-field-rows-*` from `generate_avc_field_rows_samples.py` comprises one hundred twenty owned 32x64-frame/two-row-field streams: one whole-field slice, two row slices or four block slices; signed motion/all-AC; filter0/1/2; 8/10-bit and both field orders. Multi-slice variants reverse wire order. Tests gate dimensions/address ranges and context-dependent coefficient syntax, compare every JM pixel after reset/rewind/seek, require visible filter effects and cross-row slice suppression, and check ASO invariance. These sources are synthetic and do not contain private media or parameter sets.

`avc-field-refs-*` from `generate_avc_field_reference_samples.py` contains ninety-six owned six-field streams: two distinct PCM reference pairs, then slices independently selecting frame-zero/frame-one fields via short-term list modifications. Normal/swapped choices, skip/coded prediction, filters0/1/2, 8/10-bit, both parity orders and ASO prove per-slice reference selection, actual reference identity in boundary strengths and reset/rewind/seek behavior against saved JM pixels. No private parameters or pixels are used; ordinary tests do not generate fixtures.

`avc-field-opposite-*` from `generate_avc_field_opposite_samples.py` comprises ninety-six owned streams selecting opposite-parity frame-zero/frame-one references through field PicNum modifications Subtract(4)/Subtract(2). Skip/coded, swapped slices, filters0/1/2, ASO, both field orders and 8/10-bit exercise signed chroma parity adjustment and reference identity in filtering. Syntax gates and every saved JM sample are checked after reset/rewind/seek; ordinary tests remain offline and do not run JM.

`avc-field-multiref-*` from `generate_avc_field_multiref_samples.py` supplies two hundred eighty-eight owned six-field streams with four explicit L0 entries and per-partition ref_idx. Retained old/new frame pairs provide same/opposite parity references; rotated choices, 16x8/8x16/subdivisions to 4x4, filters0/1/2, ASO, both field orders and 8/10-bit test prediction and reference/parity identity in filtering. Tests gate active count/list modifications/every ref_idx, compare JM pixels after reset/rewind/seek and enforce memory admission boundaries. These tests use saved fixtures offline; generator/JM invocation is explicit.

`avc-field-weight-*` from `generate_avc_field_weight_samples.py` contains 168 owned six-field streams: identity/explicit four-entry weighting, whole-field skip, 16x8/8x8 partitions and signed residual; filters0/1/2, ASO, 8/10-bit and both field orders. Tests gate actual weights and syntax, require clipping and residual effects against matched controls, and compare saved JM samples after reset/rewind/seek. Fixture generation invokes JM explicitly; ordinary tests read saved synthetic fixtures without external decoders.

`avc-field-mixed-pcm-*` from `generate_avc_field_mixed_pcm_samples.py` supplies 48 owned I/I/P/P streams with I_PCM before/after skipped or explicitly moving inter macroblocks. Both parity orders, 8/10-bit and filter0/1/2 qualify PCM syntax, intra neighbours and deblocking against saved JM pixels after reset/rewind/seek. The prior implementation refuses specifically at the intra macroblock in P syntax. Sources and pixels are synthetic; explicit oracle generation remains separate from offline tests. These fixtures do not qualify non-PCM intra prediction.

`avc-field-mixed-intra4-*` from `generate_avc_field_mixed_intra4_samples.py` contains 48 owned P-field streams with Intra4x4 DC/zero residual before/after coded inter or skip. The actual wire intra type is five (Intra4x4). Tests explicitly gate that type, all sixteen predicted-mode flags, chroma DC and zero coded-block pattern, then compare saved JM pixels after reset/rewind/seek. Both depths/parity orders and filter0/1/2 are covered. Ordinary tests remain offline without generators/JM.

`avc-field-intra-residual-*` from `generate_avc_field_intra_residual_samples.py` contains 240 synthetic streams: Intra4x4 alternating-sign luma AC plus identical zero controls, and Intra16x16 positive/negative/zero DC. Every mode is placed before/after coded inter or skip in a P field, with filters0/1/2, both field orders and 8/10-bit. Gates require field scan position four, exact signs and visible residual effects without filtering; native/player reset/rewind/seek match saved JM samples. No private media is used and generation remains separate from offline tests.

`avc-field-intra-*` (excluding the separately documented `intra-residual` family) from `generate_avc_field_intra_samples.py` comprises 120 synthetic two-slice I-field pairs. Intra4x4 AC and Intra16x16 signed DC have matched zero controls; both depths/parities, filters0/1/2 and ASO gate syntax, field scan, pair timing and every saved JM pixel after reset/rewind/seek. The former PCM-only path refuses specifically at non-PCM syntax. Explicit generation is separate from ordinary offline tests.

`avc-field-intra-chroma-*` from `generate_avc_field_intra_chroma_samples.py` contains 120 owned two-slice I-field pairs with signed Cb/Cr DC and alternating chroma AC (plus zero controls), Intra4x4/Intra16x16, both depths/parities, filters0/1/2 and ASO. Syntax gates prove signs and field scan; matched luma-only controls isolate chroma effects with deblocking disabled. Native/player reset/rewind/seek and media timing compare saved JM pixels without executing external decoders during tests.

`avc-field-intra8-*` from `generate_avc_field_intra8_samples.py` contains 96 synthetic High/High10 I-field pairs: Intra8x8 DC with signed AC or zero coefficients, intra matrices8/16, two slices, ASO, filters0/1/2 and both field orders. Gates check transform8, explicit matrices and field scan positions9/24/32/17; independent saved JM pixels qualify reset/rewind/seek and pair presentation. Matrix controls isolate residual effects without filtering. Generator/JM invocation remains separate from ordinary offline tests.

`avc-field-intra-bypass-*` from `generate_avc_field_intra_bypass_samples.py` contains 224 synthetic profile244/4:2:0 I-field pairs, QP-prime zero at8/10-bit, Intra4x4/Intra8x8/Intra16x16 DC, signed luma/chroma or zero coefficients, matrices8/16, SPS enabled/control and ASO/both field orders. Exact saved JM pixels qualify reset/rewind/seek; controls require bypass scaling independence and a difference from quantized reconstruction. Filtering is disabled. No external decoder is invoked during ordinary tests.

`avc-field-fmo-*` from `generate_avc_field_fmo_samples.py` supplies 480 owned 32x64 PCM/I16 DC field streams for FMO map types0–6, dynamic directions, both depths/parities, zero/signed DC, filters0/1/2 and ASO. Exact address gates prove group traversal; saved JM pixels qualify reset/rewind/seek and pair timing. Type0/dc0/filter0 reproduces the horizontal intra-field boundary-strength mismatch; filter controls show actual filtering and slice suppression. Ordinary tests are offline and do not run fixture generation or JM.

`avc-field-cabac-*` from `generate_avc_field_cabac_samples.py` contains 72 owned High/High10 Intra16x16/DC I-field pairs. The owned arithmetic writer encodes zero or signed DC at field scan index one; independent JM output validates syntax. Tests require raster index four, exact sign, ASO invariance, visible residual effects without filtering and native/player reset/rewind/seek with pair timing. Both depths/parities and filters0/1/2 are covered. Ordinary tests only read saved fixtures and do not run external decoders.

`avc-field-cabac-p-*` from `generate_avc_field_cabac_p_samples.py` contains 144 owned I/I/P/P streams: nonuniform signed-DC references, skip or P16x16 fractional MVD[1,-1], CABAC init0/1/2, filters0/1/2, ASO,8/10-bit and both orders. Gates require actual skip/coded syntax and MVDs, init/ASO pixel invariance and visible motion versus unfiltered skip controls. Every saved JM sample matches native/player reset/rewind/seek; ordinary tests remain offline without external decoders.

`avc-field-cabac-p-residual-*` from `generate_avc_field_cabac_p_residual_samples.py` contains 144 owned P-field streams and zero controls with identical fractional motion/CBP15. Signed luma AC uses field CABAC significance/last contexts and field scan index one (raster four). Exact signs/counts/MVDs, init/ASO invariance and visible residual differences without filtering are required. Saved JM pixels qualify native/player reset/rewind/seek at8/10-bit, both orders and filters0/1/2. Fixture generation remains separate from ordinary offline tests.

`avc-field-cabac-p-chroma-*` from `generate_avc_field_cabac_p_chroma_samples.py` contains 144 synthetic CBP47 P-field streams with signed luma/Cb/Cr residual and zero controls. Exact DC signs, chroma counts and field AC raster position four are gated. Matched luma-only controls isolate chroma changes without filtering; every JM sample matches native/player reset/rewind/seek for all init banks, ASO, filters0/1/2, both orders and8/10-bit. Ordinary tests are offline and do not generate fixtures or invoke JM.

`avc-field-cabac-p-transform8-*` from `generate_avc_field_cabac_p_transform8_samples.py` supplies 288 owned P-field streams: signed8x8 residual at field raster position eight, inter matrices16/24 with invariant intra matrices, or CBP0 no-residual controls. Tests gate exact coefficients/transform selection, unchanged references, observable matrix effects and every saved JM pixel after reset/rewind/seek for init0/1/2, ASO, filters0/1/2 and both depths/orders. Filtered8-bit/AC/init0/scale16 reproduces the erroneous use of4x4 counts for CABAC8 boundary metadata; acceptance is enabled with the fix. Ordinary tests remain offline without generator/JM execution.

`avc-field-cabac-p-partition-*` from `generate_avc_field_cabac_p_partition_samples.py` supplies 432 owned streams covering 16x8 through 4x4 partition geometry with alternating signed MVD. Tests assert syntax, visible motion differences, exact saved JM pixels and repeatability after reset/rewind/seek. Generation is explicit; ordinary tests need neither JM nor FFmpeg.

`avc-field-cabac-p-mixed-*` from `generate_avc_field_cabac_p_mixed_samples.py` contains 432 synthetic single-slice P fields mixing I4 DC or positive/negative I16 field-scanned DC with skip or fractional-motion blocks, in both placements. Acceptance checks actual syntax and every saved JM pixel, observable residual/motion effects, reset/rewind/seek and representative owned visitor timing. Generation is explicit and separate; ordinary tests execute neither JM nor FFmpeg and never read private videos.

`avc-field-cabac-p-mixed-constrained-*` and `avc-field-cabac-p-mixed-prediction-control-*` use explicit `--constrained`/`--prediction-control` generation in `generate_avc_field_cabac_p_mixed_samples.py`: 432 matched pairs with signed DC mean plus row variation. Every unfiltered intra-last pair must visibly differ when inter neighbours are forbidden; intra-first pairs and reference pictures stay identical. Both sides match native/JM pixels, with constrained reset/rewind/seek and representative media visitor timing. Ordinary tests never invoke generators, JM, FFmpeg or private files.

`avc-field-cabac-p-multiref-*` from `generate_avc_field_cabac_p_multiref_samples.py` supplies 1728 synthetic four-reference P-field streams with partition sizes through4x4, default/permuted L0, rotated indices, signed fractional MVD, all CABAC init banks, ASO and filtering. Owned references differ by age/parity with DC mean and row variation. Acceptance gates exact syntax and saved JM pixels, visible reference/list effects, reset/rewind/seek and representative owned visitor timing. Ordinary tests remain offline with no generator, JM or FFmpeg invocation.

`avc-field-cabac-p-weight-*` from `generate_avc_field_cabac_p_weight_samples.py` contains 576 owned four-reference streams: skip,16x8,8x8 and signed full-component residual with identity/explicit weight tables. Tables cover negative/zero weights, separate component denominators, signed offsets and a default reference entry. Tests gate exact weights/reference indices/residual, observable matched weight and per-plane residual effects, every saved JM sample and reset/rewind/seek. Representative streams pass owned media visitor timing. Generation is explicit; tests execute neither JM nor FFmpeg and use no private media.

`avc-field-b-explicit-*` from `generate_avc_field_b_explicit_samples.py` supplies 288 synthetic six-sample streams with actual L0/L1/Bi B fields, CAVLC or all CABAC init banks, ASO, filtering, both field orders and8/10-bit. The first CABAC Bi packet reproduced the exact native reconstruction gate after four accepted I-field packets; that refusal expectation is replaced by acceptance. Tests gate reference indices/MVD, observed Bi differences from each unilateral control, every saved JM pixel and reset/rewind/seek. Representative B streams also gate owned media visitor timing. Explicit generation is separate; ordinary tests run offline without JM/FFmpeg or private media. Direct/skip, implicit weighting and broader B-field tools are not covered by this acceptance.

`avc-field-b-future-*` uses `generate_avc_field_b_explicit_samples.py --future`: 288 owned I–I–B field streams with decode POC/PTS0,1,4,5,2,3 and signed MP4 composition offsets. Native output is checked in decode order against reordered saved JM slices; presentation paths require sample identities0,4,2, PTS0,2,4, duration2 and exact JM display pixels. Reset, rewind, seek into the B interval, ASO, filters, entropy banks, depths/orders and owned visitor timing are gated. The ordered API's deliberate refusal of decreasing POC is tested separately from successful presentation acceptance and requires reset before recovery. Explicit generation invokes JM only when requested; ordinary tests are offline and never invoke JM/FFmpeg or private media.

`avc-field-b-implicit-*` uses explicit `generate_avc_field_b_explicit_samples.py --implicit` generation: 288 future-reference streams with PPS weighted_bipred_idc2. Matched `b-future` controls require unchanged references and L0/L1 output but actual Bi weight effects. Acceptance checks field POC, absence of explicit weight tables, exact JM decode/display pixels, reset/rewind/seek and owned visitor timing. The original sample4 missing-POC-context refusal is replaced by acceptance with the fix; existing PCM/MMCO cases separately assert selected parity POC and short-to-long marking. Ordinary tests require neither JM nor FFmpeg/network/private files.

`avc-field-b-direct-*`: 768 owned separate-field AVC streams generated by `scripts/generate_avc_field_b_direct_samples.py` with an explicitly supplied JM decoder. Two I pairs precede a future inter-coded P pair and a reordered B pair. P list modification selects older I fields for valid temporal mapping; CAVLC P16 and CABAC P4x4 exercise retained identities/parities and nonzero motion. Spatial/temporal, skip/coded, direct8 on/off, both field orders, 8/10-bit, CABAC init banks, filters and ASO have saved JM pixels and hashes in `avc-field-b-direct-generated.json`. Ordinary tests consume these files offline: JM and FFmpeg are not invoked. Native decode and presentation are acceptance tests; the old reconstruction entrypoint lacking direct context has a distinct contract-refusal reproducer. Control pairs gate actual motion/inference effects and skip/direct equality. No private samples or parameter sets are used.

`avc-field-b-mixed-*`: 2976 owned eight-sample CAVLC streams from `scripts/generate_avc_field_b_mixed_samples.py`. One B slice contains direct beside L0/L1/Bi16 or I_PCM, or mixes direct with every explicit B8x8 subtype1..12 at each group position. Saved JM pixels and manifest hashes accompany all field orders, depths8/10, direct modes, inference flags and filters. Unfiltered vector controls distinguish spatial neighbour use from temporal co-located motion. Native decode, presentation, reset/rewind/seek and media visitor checks are acceptance tests.

`avc-field-b-residual-*`: 624 owned CAVLC B-direct streams from `scripts/generate_avc_field_b_residual_samples.py`, with signed luma AC, chroma DC or combined AC in either macroblock and zero controls. Gates cover field-scan coefficient placement, sign, CBP, per-plane pixel effects and full saved JM output. Both families use only owned writers and synthetic samples; generation requires an explicit JM executable, while ordinary tests invoke neither JM nor FFmpeg and need no network. No private videos, pixels or parameter sets are copied.

`avc-field-b-cabac-residual-*`: 2304 owned CABAC B-direct field streams from `scripts/generate_avc_field_b_cabac_residual_samples.py`. CBP0 and empty-CBP47 controls accompany signed luma AC, chroma DC and combined AC across spatial/temporal modes, inference flags, CABAC init0/1/2, ASO, filters, field orders and8/10-bit. Gates cover field coefficients, sign/plane effects, filtering, JM pixels and reset/rewind/seek/presentation.

`avc-field-b-transform8-*`: 864 owned CABAC B-direct field streams from `scripts/generate_avc_field_b_transform8_samples.py`, with direct8 inference, transform8, signed field-scanned AC, inter matrices16/24 and no-residual controls. Intra matrices remain16; reference and chroma controls isolate the inter8 luma effect. Both generators require explicitly supplied JM only during generation. Saved oracle files and manifest hashes are consumed offline; ordinary tests call neither JM nor FFmpeg and copy no private media or codec parameter sets.

`avc-field-b-cabac-mixed-*`: 5184 owned eight-sample streams from `scripts/generate_avc_field_b_cabac_mixed_samples.py`. One CABAC B slice contains L0/L1/Bi16 or I4 DC / signed I16 DC before/after direct-coded or skipped blocks. The matrix covers every init bank, both field orders, depths8/10, direct modes, inference flags and filters. Syntax and saved-JM acceptance gates accompany explicit-MVD, intra-sign and skip/coded controls; reset, rewind, display timing, seek and selected media visitor cases are checked. Manifest hashes match every saved synthetic video and oracle. Generation alone uses an explicitly supplied JM executable; ordinary tests run offline with no JM/FFmpeg invocation. No private media or codec parameter sets are copied.

`avc-field-b-longterm-*`: 768 qualified owned streams from `scripts/generate_avc_field_b_longterm_samples.py`, with source/co-located long-term marking and matched short-term controls. Saved JM pixels are consumed offline. The separately generated `--mixed-parity` family adds 768 accepted streams with a long-term first parity and short-term complementary parity. Tests gate mixed list modifications, selected-parity DPB status, full saved JM output and isolated direct-row effects. Both families have media visitor timing/pixel acceptance; ordinary tests require neither JM nor FFmpeg.

`avc-field-b-cabac-sub-*` comes from `scripts/generate_avc_field_b_cabac_sub_samples.py`: 6912 owned CABAC separate B-field streams mix direct in each 8x8 group with every explicit B subtype1..12. Both fields contain two independent slices; syntax gates partition geometry, prediction modes, zero references, signed MVD and entropy termination. All init banks, depths8/10, field orders, spatial/temporal modes, inference flags and filters match saved JM pixels after reset, rewind, presentation and seek. Selected media visitors gate exact pixels and reordered first-field timing. Generation uses explicitly supplied JM only; ordinary tests consume saved synthetic fixtures offline, without JM/FFmpeg/network/private media. This qualifies existing reconstruction; no production decoder fix was needed. Cross-slice-independent B8x8 acceptance does not qualify cross-macroblock CABAC contexts, mixed residual/transform/bypass or directional intra. Manifest hashes were verified for every saved video and oracle.

`avc-field-b-cabac-sub-joined-*` uses `generate_avc_field_b_cabac_sub_samples.py --joined`: 6912 additional owned CABAC B8x8 streams put both macroblocks in one slice. The writer retains per-list 4x4 MVD magnitudes across the boundary and emits neighbour-dependent skip, mb_type and CBP contexts. Tests decode both macroblocks, gate every partition/reference/MVD and entropy termination, then accept all saved JM decode/display pixels, reset, rewind and seek across subtype1..12, direct positions, depths, field orders, init banks, inference flags, direct modes and filters. Unfiltered independent-slice controls require invariant reference frames and left macroblocks, while at least one right macroblock must change through neighbour prediction. Selected media visitors accept pixels and timing; every fixture/oracle hash is verified. No production decoder change was needed. Positive reference-index context transitions, richer mixed residuals, transform bypass and directional intra remain unqualified. Generation requires an explicitly supplied JM executable; ordinary acceptance consumes owned saved fixtures offline without JM/FFmpeg/network/private media.

`avc-field-b-cabac-sub-references-*` uses `generate_avc_field_b_cabac_sub_samples.py --joined --references`: 6912 additional joined CABAC B8x8 streams cycle reference indices0..3 by group, list and macroblock. The owned writer maintains per-list reference grids and emits contexts54..57 with unary continuation58/59; direct groups remain unavailable for positive-index context derivation. Syntax gates exact selected indices, partition geometry, signed MVD and termination for both macroblocks. All saved JM pixels, reset, rewind, presentation and seek pass across all B subtype1..12, direct positions, init banks, depth/order, inference, spatial/temporal and filters. Zero-index controls require unchanged reference frames and observable B prediction changes. Selected media visitors and all video/oracle hashes pass. Generation alone uses explicit JM; ordinary tests need no JM/FFmpeg/network/private files. Existing production reconstruction satisfies the matrix. Transform bypass, richer mixed residuals, directional intra and mixed frame/field or frame-number-gap storage remain open.

`avc-field-b-bypass8-*` uses `scripts/generate_avc_field_b_bypass8_samples.py`: 1728 owned CABAC B-direct 8x8 streams use High444Predictive with 4:2:0 geometry and minimum QP_Y (0 at8-bit, -12 at10-bit), paired transform_bypass enabled/disabled controls, inter matrices16/24, signed field-scanned AC and zero residuals. CABAC initialization clamps the negative QP to0 while reconstruction retains the actual QP. Syntax gates minimum slice QP, bypass flag, transform8, CBP and signed raster-position8 coefficients. Every saved JM decode/display pixel, reset, rewind and seek passes across field orders, spatial/temporal, init banks, ASO and filters. Enabled bypass must ignore scaling matrices exactly, preserve reference frames relative to controls and visibly change B reconstruction. Selected media visitors accept timing/pixels. All new video/oracle hashes were checked; regenerated existing864 transform8 streams remain unchanged. Generation alone requires explicit JM; ordinary acceptance is offline without JM/FFmpeg/network/private media. Existing production decoding satisfies these cases; B4x4/chroma bypass and mixed residual combinations remain unqualified.

`avc-field-gap-*` uses `scripts/generate_avc_field_gap_samples.py`, with explicit JM generation only: The native decoder now inserts pixel-free non-existing field stores for allowed frame_num gaps, advances shared POC/frame history and applies field sliding-window eviction. Field storage separates pixel ownership from reference metadata; unknown type0 POC slots participate in P PicNum ordering but are excluded from B POC ordering. Non-existing fields cannot be promoted to long-term, and failed inferred insertion/MMCO preserves the buffer. 48 owned CAVLC I0–P2–P3 streams reorder prediction to real retained fields, with both depths8/10, field orders, skip/coded, filters and ASO. They pass exact saved JM pixels, reset, rewind, sample/PTS/duration and seek; a separate gaps-forbidden SPS fixture refuses sample2 with the exact SPS error and requires reset. Metadata regression gates pixel-free slots, missing POC exclusion, valid inferred POC and transactional MMCO rejection. Representative media visitors pass. Ordinary tests consume saved synthetic files offline without JM/FFmpeg/network/private media. CABAC, POC1/2 end-to-end gap qualification, wraparound and long-term gap eviction controls remain to be expanded; mixed frame/field storage remains unsupported. All48 video/oracle hashes were verified. `avc-field-gap-forbidden.mp4` is a contract refusal, distinct from allowed-gap playback acceptance.

`avc-field-gap-wrap-*` and `avc-field-gap-wrap-long-*` use `generate_avc_field_gap_samples.py --wrap` with optional `--long-term`: 48 owned wrap streams use I0–I14–P0 with inferred frames1..13 and15, exercising eviction across MaxFrameNum16 while P0 explicitly selects real frame14 fields. A further48 streams retain I0 as long-term index0 and declare four reference stores. The initial three-store long-term generator draft was rejected by JM for a missing selected reference and corrected before decoder reproduction. The valid long-term fixture reproduced native sample4 failure "AVC field DPB capacity or identity conflict": old validation incorrectly conflated reused frame_num across short/long-term marking. Field insertion and pixel-free gap insertion now reject duplicate short-term frame numbers while permitting coexistence with long-term stores; stable IDs and long-term-index uniqueness remain separately validated. Both matrices accept saved JM pixels, reset, rewind, presentation and seek, and matched short/long oracle pixels are identical. An automated metadata test cycles inferred frame numbers through0/1, retains both long-term parities and proves coexistence with pixel-free frame0. Media visitor checks cover ordinary and long-term wraps; all96 video/oracle hashes pass. Ordinary tests remain offline without JM/FFmpeg/network/private files. CABAC and POC1/2 gap end-to-end qualification, B gaps, MMCO-reset gap combinations and mixed frame/field storage remain open.

`avc-field-gap-cabac-*`, `avc-field-gap-wrap-cabac-*` and `avc-field-gap-wrap-long-cabac-*` use `generate_avc_field_gap_samples.py --cabac` with optional `--wrap --long-term`: 432 owned CABAC I/P gap streams extend ordinary I0–P2–P3, wrapped I0–I14–P0 and retained-long-term wrapped sequences (144 each). All init banks0/1/2, depths8/10, field orders, skip/P16, filters and ASO pass saved JM pixels, reset, rewind, presentation and seek. Syntax gates CABAC init, single active L0, exact skip/P16 dispatch, zero MVD and entropy termination. Matched skip/P16 controls and short/long marking controls require identical pixels. Selected media visitors pass all three timing contracts. Every video/oracle hash is verified; regenerated existing144 CAVLC gap streams remain unchanged. Existing production gap handling satisfies the new matrix with no further decoder change. Generation uses explicitly supplied JM only; ordinary tests are offline with no JM/FFmpeg/network/private samples. POC1/2 gap playback, B gaps and MMCO-reset gap combinations remain unqualified.

`avc-field-gap-*-poc1-*` / `*-poc2-*` (and base `avc-field-gap-poc1-*` / `poc2-*`) use `generate_avc_field_gap_samples.py --poc-type 1|2` with optional `--cabac --wrap --long-term`: 1152 owned POC1/2 I/P gap streams extend ordinary, wrapped and retained-long-term cases with CAVLC and every CABAC init bank, depths8/10, field orders, skip/P16, filters and ASO. POC1 uses explicit per-field delta0/1, zero top-bottom offset, a single cycle offset2 and non-reference offset-1; POC2 uses inferred decode order with no POC syntax. Acceptance independently gates each inferred frame order, absence of POC LSB syntax,14 inferred frames for wraps (one for ordinary gaps) and FrameNumOffset0→16 before accepting every saved JM pixel, reset, rewind, presentation and seek. Matched CABAC skip/P16 and short/long controls stay identical. Selected media visitors pass; all1152 video/oracle hashes and the separate refusal hash are checked. Existing576 POC0 gap streams regenerate unchanged. The initial POC1 bottom-first draft used top-bottom offset1 and zero delta, producing invalid nonzero IDR bottom POC; JM accepted that draft but native normative validation correctly refused it. A compact two-field owned reproducer is generated separately as avc-field-poc1-invalid-idr-bottom.mp4 and tested only for the exact IDR refusal/reset contract. The corrected matrix uses zero IDR POC and explicit complementary-field delta1; no production validation was weakened. Generation alone needs explicitly supplied JM; ordinary tests are offline with no JM/FFmpeg/network/private media. More complex POC cycles/deltas, B gaps, MMCO-reset gap combinations and mixed frame/field storage remain unqualified.

`avc-field-b-gap-longterm-*` / `avc-field-b-gap-longterm-mixed-*` use `generate_avc_field_b_longterm_samples.py --gap` with optional `--mixed-parity`: 1536 owned eight-sample streams insert frame_num1 between I0 and I2, then decode future P3 and reordered non-reference B4. P/B lists explicitly select real I0 source fields and real P3 co-located fields; five declared reference stores retain the pixel-free inferred entry throughout the sequence. Complete and mixed-parity long-term source/co-located controls cover CAVLC/CABAC init0/1/2, depths8/10, field orders, spatial/temporal direct, skip/coded and filters. Syntax gates frame-number sequence, updated PicNum modifications, selected identities/parities and MMCO; manual DPB insertion proves the gap slot owns no pixels and never enters the active B lists. Every saved JM decode/display pixel, reset, rewind and seek passes, including causal long-term direct-row controls and mixed-parity row isolation. All1536 video/oracle hashes pass; regenerated previous1536 long-term streams are unchanged. Representative media visitors accept reordered sample/PTS/duration. The five relevant long-term/gap regressions and media visitor passed both in an isolated snapshot and in the current primary checkout. Generation alone needs explicit JM; ordinary acceptance remains offline without JM/FFmpeg/network/private media. POC1/2 B gaps, wrap/MMCO-reset B combinations and mixed frame/field storage remain unqualified.

`avc-field-b-gap-longterm-poc1-*`, `poc2-*` and corresponding `mixed-poc1-*` / `mixed-poc2-*` are generated with `generate_avc_field_b_longterm_samples.py --gap --poc-type 1|2` and optional `--mixed-parity`. Separate-field B-gap POC1/2 acceptance: 3072 owned eight-sample streams extend complete and mixed-parity long-term source/co-located cases, CAVLC/CABAC init0/1/2, 8/10-bit fields, both field orders, spatial/temporal direct, skip/coded and filters. POC1 uses an inferred missing frame at POC2/2 and explicit deltas for reordered B fields; POC2 uses no POC syntax, P3 reference fields at POC6/6 and non-reference B3 fields at POC5/5 with the same frame_num as the reference. Acceptance gates frame-number sequences, exact inferred/decoded POCs, modulo32 PicNum reordering, selected reference identities/parities, pixel-free gap storage and MMCO. Every saved JM pixel passes native decode, reset, presentation, rewind and seek; matched skip/coded and causal complete/mixed-parity long-term controls pass. All four 768-stream manifest/video/oracle hash sets are verified. Eight selected media visitors pass reordered frame timing. No decoder change was needed. Generation requires explicit JM only; ordinary tests remain offline without JM/FFmpeg/network/private media. General B gap wraps/MMCO resets, richer POC1 cycles/deltas and mixed frame/field storage remain unqualified.

Separate-field B-gap frame-number wrap acceptance: 1536 owned POC0 streams decode I0, I14, future reference P0 and reordered non-reference B1 (complete and mixed-parity long-term source/co-located families, 768 each). Thirteen pixel-free frames are inferred before I14 and frame15 before P0; selected L0 is real I14 and selected L1 is real P0. Modulo32 modifications select I14 with wrapped PicNum-3/-4 and P0 with PicNum1/0. Explicit MMCO1 releases gap9/10 before I14 and gap11/12 before P0, leaving room for both complementary fields and mixed short/long marking. Source-long cases initially mark the complete I0 pair long-term, then MMCO2/6 replace it with complete or mixed I14; this avoids filling a gap sequence while the initial pair is mixed. Initial drafts exceeded JM reference capacity at gap15 or mixed P0 and were corrected in fixture marking without weakening decoder validation. All 1536 saved JM outputs pass exact native/display pixels, reset, rewind, seek, skip/coded equivalence and causal long-term/parity-row controls; four representative media visitors pass reordered timing. Both 768-record video/oracle manifest hash sets are verified. Existing decoder handles the accepted matrix without production changes. Shared intra generators gain optional explicit short/long forgetting, with unchanged defaults. Generator command is generate_avc_field_b_longterm_samples.py --gap --wrap with optional --mixed-parity and explicit --jm-decoder; ordinary tests require no JM, FFmpeg, network or private media. POC1/2 B wraps, B MMCO5/reset-gap combinations and mixed frame/field storage remain unqualified.

Separate-field B-gap POC1/2 wrap acceptance: 3072 owned eight-sample streams extend the wrap complete/mixed long-term source/co-located matrix across CAVLC/CABAC init0/1/2, depths8/10, field orders, spatial/temporal direct, skip/coded and filters. POC1 I14 fields use POC28/29, reference P0 uses POC36/37 and reordered non-reference B1 uses POC34/35. POC2 uses no explicit POC syntax: I14 POC28/28, reference P0 POC32/32 and same-frame_num non-reference B0 POC31/31. Acceptance independently checks all14 inferred frame POCs (2*frame_num), FrameNumOffset0→16, exact decoded POCs, active wrapped PicNum references/parities and MMCO1/2/4/6. Every saved JM pixel passes native and display decode, reset, rewind and seek, matched skip/coded equivalence and causal complete/mixed long-term controls. All four768-record video/oracle manifest hash sets and eight selected media timing visitors pass. The existing production decoder satisfies these streams without changes; ordinary acceptance is offline without JM/FFmpeg/network/private media. Generation alone requires explicit JM and --gap --wrap --poc-type1|2 plus optional --mixed-parity. B MMCO5/reset-gap combinations, general POC1 cycles/deltas and mixed frame/field storage remain unqualified.

Separate-field MMCO5 reset-gap/B acceptance: 1152 owned ten-sample streams (384 each POC0/1/2) decode an initial I0 pair, infer frames1..14 before I15, then wrap to a new I0 whose first field carries MMCO5. Its complementary second field has no reset. After MMCO5 a fresh inferred frame1 precedes reference P2 and reordered B3 (POC0/1) or same-frame_num B2 (POC2). The matrix covers 8/10-bit, both field orders, CAVLC/CABAC init0/1/2, spatial/temporal direct, skip/coded, filters and initially short/long I0 controls. Acceptance gates15 inferred records, exact pre/post-marking POCs and FrameNumOffset16 on the reset field then0, MMCO5 syntax on every slice of only the first reset field, cleared old I0/I15 identities and selection of only the new reset I0/P2 references. Every JM output passes exact native decode-order/display pixels, reset, rewind, timestamps and seek to the reordered B frame. Initial short/long status cannot change any output; skip/coded outputs are identical. All three384-record video/oracle hash sets and six representative media visitor timing checks pass. Existing production decoder satisfies this matrix without changes. Shared intra writers gain an optional MMCO5 command; 96 PCM and576 CABAC default encodings remain byte-identical to HEAD. Generation uses generate_avc_field_b_reset_gap_samples.py with optional --poc-type1|2 and explicit --jm-decoder; ordinary acceptance needs no JM, FFmpeg, network or private media. This qualifies reset on the first field of a coded frame_num0 pair after wrap, not arbitrary non-paired-field/reset arrangements, combined residual/FMO/reset tools, general POC1 cycles or mixed frame/field reference storage. Those remain open. Fixture constraints were checked against H.264 (02/2016), clauses3.36,7.4.1.2.4,7.4.3 and8.2.1; a reset field's FrameNumOffset is calculated before marking resets the state for later fields.

Mixed-reference storage first integration: ReferenceBuffer can transfer initialized state, reference IDs/frame numbers, long-term index limit, exact field POCs and pixel-free gap records into FieldBuffer through an owned conversion callback. AvcDecoder uses this transfer before the first non-IDR separate field, after checking field storage/scratch budget. Complete intra-coded frame references are split into top/bottom compact planes; any empty MBAFF intra motion snapshot is recognized as intra, while non-empty co-located motion conversion remains an explicit unsupported case. This enables MBAFF PCM frames (all macroblocks coded as frames) followed by separate CAVLC P fields without an intervening IDR. Forty-eight owned streams cover 8/10-bit, both field orders, skip/coded, filters and short/long IDR frame references; all JM pixels, output shape, native reset, presentation timing, rewind and seek pass. Long/short controls require identical output and syntax gates require preserved long-term vs short-term list selection. Four representative media visitors pass. Migration unit coverage checks stable identities, exact parity POCs, unknown/known pixel-free gap POCs, preserved long-term limit, initialized marking and atomic failure. All48 video/oracle hashes pass. Production frame-to-field pixel conversion retains original frame_num and takes a checked per-field allocation budget.

The initial PAFF PCM frame fixture failed earlier in IntraCavlcReader, so it could not demonstrate the mixed-storage failure. generate_avc_frame_field_samples.py --paff preserves one owned valid JM two-frame reproducer separately; its passing refusal test gates PAFF frame syntax, the exact reader error and reset/poison behavior. It is not PAFF acceptance. The corrected MBAFF fixture reproduced the precise mixed-storage refusal at sample1 before the fix and now passes acceptance. Inter-coded frame co-located motion conversion, the reverse field-to-frame path, broader PAFF frame geometry and arbitrary mixed/non-paired reference arrangements remain open. Fixture generation alone needs explicit JM; ordinary tests are offline without JM/FFmpeg/network/private media. A stale shared-target codec artifact initially reported the removed refusal; cargo clean -p fvid-codecs forced a current-source rebuild before acceptance and broad validation.

Mixed-reference follow-up verification: the owned --inter-source reproducer adds an accepted full-frame skipped P picture before the separate field. The passing refusal test decodes both initial frame pictures first, then gates the precise unconnected frame-to-field co-located motion error, poisoning and reset. This is not inter-motion acceptance; its one video/oracle hash pair is verified. After a clean current-source codec rebuild, broad AVC validation passed29 MBAFF+95 multislice+3 parameter tests (127 total); full core passed506 with1 ignored. Added long-term/migration checks separately pass three PCM/refusal tests, one marking migration unit and the media visitor. During verification another edit in owned_yuv_rgb.rs introduced an E0308 return-type error; only the LUT getter return was corrected to select its matrix entry, retaining the other draft changes. A fresh current-source inter-motion refusal test then compiled and passed. Full arbitrary mixed-reference support remains open.

PAFF CAVLC frame geometry integration: IntraCavlcReader now admits non-MBAFF frame pictures in an interlaced SPS, reserves width_mbs*height_map_units*2 contexts for frame pictures (one map-unit height for separate fields), and uses the same full frame height for raster neighbour lookup. Calling the ordinary reader for an actual MBAFF frame still refuses; MBAFF dispatch remains required. The existing owned PAFF PCM reproducer no longer fails at frame sample0: the old exact-reader-refusal test was removed and replaced by playback acceptance. Forty-eight PCM frame-to-field streams (24 each short/long IDR marking) cover 8/10-bit, both field orders, skip/coded and filters, with exact JM native/display pixels, reset, rewind, seek and list syntax gates.

PAFF intra context qualification adds72 four-slice frame streams (I4 zero/symmetric AC/biased AC and I16 zero/positive/negative DC, depths8/10, filters, normal/ASO wire order), plus36 joined single-slice frame streams spanning all four raster macroblocks. Syntax gates full four-MB context budget, exact addresses in both rows, frame rather than field scan (AC position1, position4 zero), signed I16 DC, four-block joined parsing and termination. All saved JM pixels, native reset, presentation, rewind and seek pass; ASO stays pixel-identical. Joined vs split controls stay equal for neutral/symmetric patterns and require a prediction difference for biased I4 and nonzero I16 at disabled filtering. The initial symmetric-I4 inequality expectation was incorrect; a separate biased owned pattern provides causal neighbour evidence without changing decoder output. All156 PAFF video/oracle hash pairs are verified. Ordinary tests remain offline without JM/FFmpeg/network/private media; generators alone require explicit JM. Core AVC190 tests, MBAFF29, parameter updates3 and existing FMO18 regressions pass after the geometry change. Wider PAFF CABAC/inter-frame/residual/FMO/tool combinations and inter-motion or reverse field-to-frame reference conversion remain open; these fixtures do not claim universal PAFF support.

`avc-paff-cabac-*` contains 36 owned PAFF full-frame CABAC I16 fixtures (8/10-bit, zero/signed DC, filter 0/1/2, normal/ASO order). Regenerate separately with `scripts/generate_avc_paff_cabac_samples.py --jm-decoder /path/to/ldecod.exe`. These reproduced the old CABAC PAFF context refusal; `paff_cabac_intra_raster_rows_and_aso_match_jm` now requires acceptance, four raster macroblocks, exact oracle pixels, reset/rewind/seek. This is not universal PAFF CABAC/inter support. Ordinary tests use only checked-in MP4/YUV files.

`avc-paff-inter-*`: 72 owned PAFF CABAC P-frame streams following signed I16 references. Generate separately using `scripts/generate_avc_paff_inter_samples.py --jm-decoder /path/to/ldecod.exe`. `paff_cabac_inter_frames_motion_skip_and_aso_match_jm` requires actual P syntax, both raster rows, skip and nonzero MVD controls, exact JM pixels, reset/rewind/seek. The pre-fix failure was the PAFF frame reconstruction refusal. No FFmpeg/JM/network is needed by ordinary tests; this is not universal PAFF inter-tool acceptance.

`avc-paff-b-*`: 144 owned reordered PAFF CABAC B streams. Regenerate separately with `scripts/generate_avc_paff_b_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `paff_cabac_b_direct_skip_spatial_temporal_and_aso_match_jm` verifies B syntax, temporal/spatial direct, skip/direct, raster rows, exact JM pixels, decode/display order, reset/rewind/seek and causal mode controls. Source/reference pictures are owned signed I16 plus motion P frames. Tests need no JM/FFmpeg/network. Joined-neighbour and broader PAFF B tools are still unqualified.

`avc-paff-inter-joined-*` (36) and `avc-paff-b-joined-*` (72) are generated by the corresponding PAFF inter/B scripts with `--joined --jm-decoder /path/to/ldecod.exe`. `joined_paff_cabac_p_and_b_neighbours_match_jm` requires one P/B slice with four macroblocks, first-block nonzero P MVD and subsequent zero MVD, exact JM reconstruction, reorder/reset/rewind/seek, and filter-disabled equality with independent vectors. The generator's corrected context uses neighbouring MVD magnitudes, not reconstructed vectors. No external decoder is used during ordinary tests.

`avc-paff-cavlc-*`: 108 owned PAFF CAVLC P/B frame streams. Generate separately with `scripts/generate_avc_paff_cavlc_inter_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `paff_cavlc_p_b_raster_rows_joined_and_aso_match_jm` verifies both raster rows, actual P/B/skip/direct syntax, joined MVD neighbours, ASO controls, exact JM pixels, reordered playback/reset/rewind/seek. Before the geometry fix, a lower-row slice reproduced `AVC first macroblock outside picture`. PAFF FMO and broader inter tools are still unqualified. Ordinary tests read checked-in fixtures only.

`avc-paff-fmo-*`: 480 owned full-frame PAFF FMO streams, generated separately with `scripts/generate_avc_paff_fmo_samples.py --jm-decoder /path/to/ldecod.exe`. The SPS has two map-unit rows and four frame macroblock rows (eight macroblocks); group expansion follows ordinary PAFF raster geometry. Acceptance `paff_fmo_full_frame_maps_p_motion_and_b_direct_match_jm` gates all map types 0..6, changing-map directions/cycle, actual eight-block group traversal, PCM I / motion P / direct-or-skip B, ASO, exact JM pixels, reorder/reset/rewind/seek and equal skip/direct controls. Tests use checked-in MP4/YUV only, without external codecs or network. This does not qualify FMO residual or weighted tools.

`avc-paff-fmo-residual-*`: 480 owned PAFF FMO P/B signed coefficient streams and no-residual controls. Generate separately with `scripts/generate_avc_paff_fmo_residual_samples.py --jm-decoder /path/to/ldecod.exe`. The first inter macroblock in each group carries signed luma AC at frame-scan position 1, chroma DC and/or chroma AC; the remainder are skipped. `paff_fmo_signed_luma_chroma_residuals_match_jm_and_controls` checks actual coefficients/signs and group coverage, exact JM native/playback pixels, reset/rewind/seek/reorder, ASO equality and causal no-residual controls. Existing non-residual FMO bytes remain unchanged. Ordinary tests invoke no external codec or network. This does not qualify transform8/bypass, weighted or richer partitions.

`avc-paff-weight-*`: 192 owned PAFF CAVLC weighted P / explicit Bi B fixtures. Regenerate separately with `scripts/generate_avc_paff_weight_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `paff_weighted_multireference_p_b_partitions_match_jm_and_controls` verifies actual PPS/slice weights and denominators, two selected B references, 16x16/16x8/8x16/8x8 partitions, motion, exact JM pixels, reorder/reset/rewind/seek and ASO. Identity weights and alternate reference indices provide causal pixel controls. Ordinary tests use only checked-in MP4/YUV files, without FFmpeg/JM/network. CABAC, implicit weighting, joined/residual/FMO combinations remain separately unqualified.

`avc-paff-implicit-*`: 384 owned PAFF CAVLC implicit bipred / ordinary average controls with asymmetric POCs (references 0/8, B at 2/6). Regenerate separately with `scripts/generate_avc_paff_implicit_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `paff_implicit_bipred_asymmetric_poc_matches_jm_and_average_controls` gates PPS bipred 2/0, actual POCs, absence of explicit B weight tables, two reference indices and four partition shapes, exact JM pixels, reorder/reset/rewind/seek and nonuniform PTS. I/P controls remain identical while implicit B differs from average. The explicit weighted generator's default fixture bytes are unchanged. Ordinary tests require no external codec or network.

`avc-paff-cabac-weight-*`: 576 owned PAFF CABAC weighted P / two-reference explicit Bi B fixtures, generated separately with `scripts/generate_avc_paff_cabac_weight_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `paff_cabac_weighted_multireference_partitions_match_jm_and_controls` checks init 0/1/2, actual weights, selected references, split/sub partitions and MVD syntax, exact JM pixels, reset/reorder/rewind/seek, identity/reference/ASO controls and CAVLC-oracle equivalence. Source PCM frames and every codec parameter are synthetic. Ordinary tests require no external codec or network. Implicit CABAC and joined/residual/FMO combinations remain separate.

`avc-paff-cabac-implicit-*`: 1,152 owned CABAC implicit/average streams (three init banks, four Bi partition shapes, two references, asymmetric POCs 0/8 with B 2/6). Generate separately with `scripts/generate_avc_paff_cabac_implicit_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `paff_cabac_implicit_bipred_matches_jm_cavlc_and_average_controls` checks actual syntax/init/POC/weights/reference indices, exact JM pixels and CAVLC-oracle equality, causal average controls, ASO, reset/reorder/rewind/seek/nonuniform PTS. All previous 576 explicit CABAC files remain byte-identical. Ordinary tests require no external codec or network.

2026-10-07 frame-to-field motion acceptance: the existing `avc-frame-to-field-inter-8bit-top-first-coded-filter0` refusal is replaced by exact JM/reset acceptance. `scripts/generate_avc_frame_field_direct_samples.py --jm-decoder /path/to/local/JM/ldecod.exe` additionally produces 128 owned `avc-frame-field-direct-*` videos/oracles. PAFF, MBAFF frame-coded, field-coded and mixed pair sources retain nonzero P motion before reordered separate B fields; 8/10-bit, top/bottom-first, spatial/temporal, skip/direct and zero-motion controls are covered. Native/software acceptance checks exact JM pixels, reset, display sample/PTS order, rewind and seek, plus causal P/temporal-B motion differences. Unit tests independently index the FLD/FRM and FLD/AFRM co-located table and verify shared motion allocation/reset release. All 128 video/oracle hash pairs were verified. Generation is explicit and separate from offline tests; no private source content or FFmpeg is used. This acceptance does not cover reverse field-to-frame migration or all CABAC/residual/weighted mixed-reference tools.

2026-10-07 native field→frame migration: `scripts/generate_avc_field_frame_samples.py --jm-decoder /path/to/local/JM/ldecod.exe` produces 258 owned `avc-field-frame-*` video/oracle pairs. The original broad storage refusal was reproduced specifically at full-P sample2 after the initial pair matched JM. 192 acceptance cases cover native I/P fields followed by full P or spatial/temporal B, PAFF/MBAFF, 8/10-bit, both parity orders, short/long marking, skip/coded and zero/nonzero motion. Distinct signed source-field vectors expose incorrect parity selection. Another 64 acceptance cases exercise fields→frame→fields→frame and inter reference retention. Native JM pixels/reset and software display samples/PTS, rewind/seek, causal motion controls and selected media visitor durations pass. All 258 hashes were verified. The two `partial-refusal`/`mixed-refusal` fixtures first verify native field pixels, then gate the exact incomplete/mixed reference-store rejection and reset requirement. These two are explicitly not playback acceptance; unified storage is still needed. Generation is separate from offline tests and invokes local JM explicitly, without FFmpeg or private source content.


`avc-unified-field-*`: 64 owned retained-field fixtures, generated separately by `scripts/generate_avc_unified_field_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `individual_reference_fields_remain_usable_after_full_frame_marking` verifies the selected individual reference, marking, exact saved pixels, reset/rewind/seek, motion and ASO controls. The earlier `avc-field-frame-partial-refusal` and `avc-field-frame-mixed-refusal` filenames are historical; their test now requires full playback acceptance. Ordinary tests invoke no external codec or network. See the dated codec progress entry for remaining unsupported combinations.


`avc-unified-cabac-*`: 96 owned CABAC streams retain one non-paired old I field through full-frame P prediction and select that individual field again. Generate separately with `scripts/generate_avc_unified_cabac_field_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `cabac_retained_nonpaired_fields_cross_full_frames_and_match_jm` gates actual reference marking/list selection and verifies every saved pixel, reset/rewind/seek, field order, init bank, skip/coded and ASO controls. Selected media visitors check sample identity, presentation time and duration. Ordinary tests need no external codec or network.

Scope correction for historical mixed-long fixtures: H.264 7.4.3.3 requires both retained fields to share short/long marking after all commands, with the same index for a long-term pair. Acceptance of one-short/one-long saved shapes is a robustness check for non-conforming input, not valid-stream codec coverage. Partial-reference fixtures and the CABAC non-paired matrix qualify valid individual-field retention separately.


`avc-unified-residual-*`: 1,152 owned signed CABAC frame/field reference crossings. Regenerate separately with `scripts/generate_avc_unified_cabac_residual_samples.py --jm-decoder /path/to/ldecod.exe`. The `-fromframe` branch selects the intervening full frame for later field prediction; the other branch selects the surviving non-paired old I field. Acceptance `signed_cabac_residual_survives_canonical_field_frame_crossings` gates actual coefficients/scan positions, signs, CBP, reference selection/marking and checks every saved pixel, reset/rewind/seek, propagation and no-residual/empty/init/ASO controls. No external codec is invoked by ordinary tests. Scope: PAFF P, split slices, 8/10-bit 4:2:0, QP50, no filtering, 4x4 transforms; broader residual tools are not implied.


`avc-unified-weight-*`: 1,296 owned weighted P canonical frame/field crossings. Generate separately with `scripts/generate_avc_unified_weight_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `weighted_cabac_references_survive_field_frame_crossings_and_seek` gates actual weights/offsets, syntax and reference selection/marking; it checks saved pixels, reset/rewind/seek and identity/init/ASO controls. `-fromframe` selects the intervening full frame for subsequent fields; the other branch selects the old surviving I field. Scope: CABAC PAFF P16, one active reference, 8/10-bit 4:2:0, QP50, no filtering, split slices, skip/coded/4x4 signed residual. Ordinary tests use no external decoder or network.


`avc-unified-b-*`: 576 owned CABAC full B-direct pictures with non-paired field references retained across the frame and used again by later P fields. Generate separately with `scripts/generate_avc_unified_b_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `cabac_b_direct_crosses_nonpaired_field_storage_and_preserves_later_references` gates syntax and checks every saved JM picture in native/playback order, timing, reset/rewind/seek and skip/residual/motion/init/ASO controls. The generator also writes `avc-unified-b-invalid-temporal-reference.mp4`; `nonpaired_temporal_colocated_reference_is_refused_after_valid_field_prefix` qualifies only its exact invalid-reference refusal and reset contract after successful prefix decoding. Tests invoke no external decoder or network. Scope: PAFF 8/10-bit 4:2:0, split single-MB slices, QP50, no filtering, direct/skip with optional combined 4x4 signed residual; explicit/weighted B and other tools are separate.


`avc-unified-explicit-b-*`: 2,304 owned CABAC full B pictures through canonical non-paired field storage. Generate separately with `scripts/generate_avc_unified_explicit_b_samples.py --jm-decoder /path/to/ldecod.exe`. Acceptance `explicit_weighted_b_crosses_canonical_nonpaired_fields_and_matches_jm` gates weights, active references, partitions/MVD and marking, then verifies saved pixels, reordered native/playback output, timing, reset/rewind/seek and identity/reference/init/ASO controls. Includes L0/L1 and Bi16x16/16x8/8x16/8x8, explicit and implicit weighting. Scope: PAFF full B frames, QP50, no filtering, zero residual, single-MB slices and 8/10-bit 4:2:0; B-field and additional transform/residual combinations remain separate. Ordinary tests invoke no external decoder or network.

### AV1 show-existing timing and frame IDs

`python3 scripts/generate_av1_show_existing_samples.py` writes 26 owned OBU
acceptance streams, seven malformed refusal streams and 24 WebM variants.
The WebM blocks mark coded hidden frames invisible. The manifest records hashes.
No source video or external codec is needed for generation or ordinary tests.
For optional independent reference validation, compile
`scripts/av1_show_existing_oracle.c` against libaom and pass its executable
with `--oracle`; this reference helper is not used by ordinary tests.
Acceptance checks flat reconstructed pixels, timing, rewind and seek; refusal
checks truncated metadata, invalid IDs, trailing bits and OBU kind.

### AV1 inter-frame IDs

`python3 scripts/generate_av1_inter_id_samples.py` writes 175 owned flat
acceptance OBU streams and ten malformed refusals with a hash manifest.
It covers every legal delta/ID-width combination, exact reference-window edges,
modular wrap, both boundary reference slots, each of the seven bad deltas,
repeated current IDs, half-modulus jumps and backward progression.
`--oracle` optionally uses the independent libaom helper described above.
Ordinary native regression tests do not generate fixtures or invoke an oracle.

The owned flat inter entropy can be independently regenerated with the existing
libaom reference helper `scripts/av1_fixture.c`: run the compiled helper with
`32 0 0 0 4 8 0 0 1 1`. Its third coded frame has tile bytes `8c 70`.
The final argument selects reference-ID generation with disabled order hints
and CDF updates, error resilience and disabled automatic keyframe placement.
This optional reference generation is separate from the pure-Python fixtures.

### AV1 short reference signaling

`python3 scripts/generate_av1_short_reference_samples.py` writes 298 owned
acceptance streams, two malformed anchor refusals and ten WebM variants.
The JSON manifest stores hashes and independent expected reference maps.
Short and explicit reference signaling are paired across order-hint widths
1–8, frame IDs, tie ordering, future/past references, wrap and anchor slots.
`--oracle` optionally validates reconstructed flat pixels through the independent
libaom helper; ordinary acceptance/refusal/playback tests use saved files only.
No private media, codec parameters, FFmpeg or network access is used.

### AV1 inherited segmentation with unchanged zero maps

`python3 scripts/generate_av1_segmentation_inheritance_samples.py` writes 224
owned acceptance OBU streams, 16 WebM variants and three refusal/reproduction
streams with a hash manifest. The acceptance streams exercise all reference
slots/primary roles, inherited tables, signed clipping, disable/reset and
explicit clearing. Their six shown frames are uniformly 128 in all planes.

Two map-update streams are valid according to the optional libaom oracle but
remain native unsupported-behavior reproductions (`acceptance: false`). They
must become acceptance checks when native map reconstruction is implemented.
The former active ALT_Q reproduction is now in `active_acceptances` and has
a native acceptance test. The malformed refusal is truncated inherited feature data. Fixture generation and
optional `--oracle` reference checking are separate from ordinary offline tests.

### Active AV1 ALT_Q and independent residual pixels

`python3 scripts/generate_av1_alt_q_samples.py` writes 25 owned OBU streams and
six WebM variants from saved entropy templates. Saved independent YUV pixels
are used by ordinary native tests; generation and external reference execution
are never part of ordinary testing. `--oracle` invokes the optional reference
helper, whose third path argument writes the decoded 8-bit 32x32 planar output.

For complete regeneration, compile `scripts/av1_fixture.c` with libaom and
create three owned inputs using `32 1 Q 0 4 8 0 0 1 1`, with Q = 0, 16, 63.
Run `cargo run --offline --manifest-path crates/fvid-codecs/Cargo.toml
--no-default-features --example av1_alt_q_templates -- INPUT0 INPUT16 INPUT63`
and save its stdout as `av1-alt-q-owned-entropy.json`. The helper verifies
template headers and extracts the second key and first inter entropy payloads.
Then run the Python generator with the compiled
`scripts/av1_show_existing_oracle.c` helper passed through `--oracle`. Only
owned synthetic inputs are allowed; do not extract templates from private media.

The acceptance matrix covers negative/positive ALT_Q and clipping at 0/255;
all 25 shown pictures are compared pixel by pixel, including nonzero residuals.
The old flat active-ALT_Q gap fixture now accepts. Map-update streams remain
explicit unsupported-behavior reproductions until map decoding is implemented.

## AV1 active ALT_LF

`generate_av1_alt_lf_samples.py` writes 82 owned OBU streams using the committed
owned ALT_Q entropy templates. The manifest records hashes for streams and
saved independent YUV references; ten cases also have WebM variants. Optional
`--oracle` points to the separately compiled libaom reference helper. Ordinary
`av1_alt_lf` tests execute neither the generator nor an external decoder.

Acceptance checks exact pixels, reset, WebM timestamps, rewind and sync seek.
The matrix covers all four segment-zero ALT_LF features, signed clipping,
reference/mode deltas, level-32 scaling and nominal zero plane levels. It uses
unchanged implicit zero maps; updated segment maps remain unsupported.

## AV1 segmentation maps

`generate_av1_segmentation_map_samples.py --writer WRITER --oracle ORACLE`
regenerates 144 OBU/YUV/WebM triples and `av1-seg-map-generated.json`. Build
`scripts/av1_fixture_symbol_writer.c` with the same optional libaom static
library as `av1_symbol_oracle.c`. ORACLE is the independently compiled
`av1_show_existing_oracle.c` helper. External tools are generation-only.

`av1-seg-map-owned-symbols.json` records the nonadaptive key/inter symbol
sequence of the owned `av1-alt-q-target64-base64-delta0.obu` input; block
markers identify the point after skip syntax where map symbols are inserted.
No private inputs or parameters are used. The generator calculates spatial
prediction and negative deinterleaving independently in Python and stores
expected raster maps along with independent reference pixel hashes.

Acceptance checks segment IDs, all reconstructed shown pixels, decoder reset,
WebM replay and seek for spatial/temporal updates, unchanged maps and reset.
The existing temporal0/temporal1 flat map refusals are now listed in
`map_acceptances`; their bytes remain unchanged. Forced segment tools and
multitile qualification remain separate work.

The map matrix includes 48 nonadaptive streams, 48 `-adapt` variants and 48
`-adapt-publish` variants. Typed CDF IDs/contexts in the owned symbol records
allow Python to encode the same symbols with independently adapted models.
Frame-end publication variants retain distributions across primary references
and reset adaptation counts when loading each new frame. Header tests verify
the adaptation/publication flags, while acceptance compares maps and pixels
for all modes. No production change was necessary for these qualification
streams; freezing only the native segment CDF updates makes acceptance fail.

## AV1 mixed lossless/lossy blocks

`generate_av1_mixed_lossless_samples.py --writer WRITER --oracle ORACLE`
regenerates 504 OBU/YUV/WebM triples and their manifest. The optional tools are
the same standalone range writer and independent pixel oracle used for maps.
Python reads the committed normative default CDF arrays, writes its own block,
segment and coefficient symbols, and optionally adapts the models. It accepts
no source media. Ordinary `av1_mixed_lossless` tests read only saved fixtures.

Each 32x32 input has four 16x16 intra blocks with one lossless and one lossy
segment assigned in all fourteen mixed patterns. Qindices 1/64/255, largest
and SELECT transforms, both CDF modes, and zero/positive/negative DC residuals
are covered. Signed residuals exercise WHT reconstruction in Y, Cb and Cr.
The old decoder's invalid CDF access is reproduced specifically on the SELECT
lossless block; acceptance now verifies actual pixels, maps and WebM replay.

## AV1 mixed lossless/lossy inter blocks

`generate_av1_mixed_inter_samples.py --writer WRITER --oracle ORACLE`
regenerates 672 owned OBU/YUV/WebM triples and their SHA-256 manifest.
The optional standalone range writer and independent libaom pixel oracle
are generation-only tools. The generator accepts no source media.

Two hidden contrasting references seed physical slots 0 and 7. Each of seven
logical reference roles selects slot 7; shown frames refresh that slot and
change the segment map. Four 16x16 identity GLOBALMV blocks cover mixed
lossless/lossy segments, qindices 1/64/255, largest/SELECT transforms, CDF
adaptation and positive/negative DC residuals in every plane.
`av1_mixed_inter` reads saved fixtures offline and checks maps, exact pixels,
reference routing, WebM timing, rewind and seek. Selecting slot 0 instead
of the declared reference causes a real pixel mismatch.

## AV1 mixed-block nonzero motion

`generate_av1_mixed_motion_samples.py --writer WRITER --oracle ORACLE`
generates 384 OBU/YUV/WebM triples. Optional libaom tools run only during
generation; no input media is accepted. Signed magnitude-14 DC in hidden
references makes +/-1, +/-1/2 and +/-1/4 pixel horizontal NEWMV observable
on the first 16x16 block. An independent stationary control must produce
different pixels for every fixture. Remaining blocks use identity GLOBALMV.

The matrix covers LAST/ALTREF routing, two mixed lossless/lossy maps and
their complements, qindices 1/64, largest/SELECT, adaptation and signed
residuals. `av1_mixed_motion` verifies saved exact pixels, maps, WebM timing,
rewind and seek offline. It does not establish vertical/compound motion or
complete motion-tool conformance.

## AV1 mixed-block vertical and diagonal motion

`generate_av1_mixed_axes_samples.py --writer WRITER --oracle ORACLE`
creates 768 owned OBU/YUV/WebM triples. Both NEWMV components and MV_JOINT
are written explicitly. Vertical and opposite-sign diagonal vectors cover
quarter, half and full pixels in both directions, LAST/ALTREF, qindices
1/64, mixed segment maps, largest/SELECT, adaptation and signed residuals.
Magnitude-14 reference detail makes movement observable; independent
stationary controls must differ. No source media is accepted.

`av1_mixed_axes` reads saved fixtures only and verifies exact pixels, maps,
timestamps, rewind and seek. Optional libaom tools run during generation only.

## AV1 forced segment reference

`generate_av1_forced_reference_samples.py --writer WRITER --oracle ORACLE`
creates 256 owned OBU/YUV/WebM triples with SEG_LVL_REF_FRAME. Segment IDs
precede skip, and forced-reference syntax omits is_inter/reference-selection
symbols. The matrix covers forced INTRA and every inter reference role, mixed maps and their
complements, qindices 1/64, largest/SELECT, adaptation and signed residuals.
The optional independent oracle accepts every generated stream.

`av1_forced_reference` enables native acceptance for a former specific
“active segmentation features not implemented” refusal. It compares exact
pixels/maps and WebM timing, rewind and seek offline; generation remains
separate and accepts no source media. Skip/global segmentation tools remain
outside this fixture matrix.

## AV1 forced skip/global segments

`generate_av1_forced_tools_samples.py --writer WRITER --oracle ORACLE`
creates 160 owned OBU/YUV/WebM triples with skip/global/both and opposite
assignments across two segments. It writes pre-skip segment IDs, omits
forced mode/reference/skip symbols where required, and handles skipped
neighbors' transform contexts. The independent oracle accepts all inputs.
No source media is accepted; optional libaom tools run only during generation.

`av1_forced_tools` replaces the old active-feature refusal with acceptance
for exact pixels, maps, WebM clock, rewind and seek. Fixtures use identity
GLOBALMV, qindices 1/64, mixed maps and complements, largest/SELECT, both
CDF modes and signed residuals on unskipped lossless blocks. Nonidentity
global motion remains outside this matrix.

## AV1 nonidentity global motion qualification

Native parsing now accepts translation, rotation/zoom and affine global models,
including differential parameters inherited from the primary reference. Prediction
uses projected motion vectors and the owned affine warp implementation; invalid
shear falls back to projected motion. Translation also consumes switchable filter
symbols, fixing the reproduced truncated-entropy failure.

`tests/av1_global_motion.rs` checks 768 owned global-motion streams and four
GLOBALMV/NEARMV streams against saved independent pixels. Coverage includes header
parameters, mixed segment maps, reset, WebM timestamps, rewind and sync seek.
Generators are separate from tests; ordinary offline tests use no FFmpeg, libav
or libaom. Optional libaom is only a generator-side range writer and pixel oracle.

This matrix covers 32x32 8-bit pictures, mixed 16x16 segments and LAST/ALTREF
routing. It does not establish full AV1 conformance: high-precision motion,
scaled references, OBMC, inter-intra and general compound profiles remain outside
this qualification. Earlier nonidentity-global-motion limitations above are
superseded only within this documented scope.

## AV1 inter-intra prediction and wedge masks

The former `AV1 inter-intra blending not implemented` refusal is reproduced
by an owned 32x32 stream and replaced with native prediction. DC, vertical,
horizontal and smooth intra predictors blend with the inter predictor using
the normative mode weights. All 16 wedge masks are constructed from the master
profiles and shape codebook; 4:2:0 chroma uses rounded four-sample mask averaging.
Inter-intra suppresses overlapped/local-warp mode syntax, and size-group contexts
follow the block-size table rather than the shorter dimension alone.

`generate_av1_interintra_samples.py` writes 1152 synthetic OBU/WebM/YUV triples.
The matrix covers all four intra modes and all 16 wedge indices, LAST/ALTREF,
contrasting hidden references, mixed lossless/lossy segmentation, adaptive CDFs
and signed residuals. `tests/av1_interintra.rs` compares every displayed pixel
against saved oracle output, resets the decoder and checks WebM timestamps,
rewind and sync seek. Generation stays separate from ordinary offline tests;
production and tests do not load FFmpeg/libav/libaom. Optional libaom is used
only during generation for range writing and independent oracle pixels.

The pixel matrix qualifies 8-bit 16x16 inter-intra blocks in 32x32 pictures.
Rectangular block masks and other bit depths use the native implementation but
are not established by this matrix. OBMC, general masked inter-inter compound
and scaled-reference prediction remain separate unresolved AV1 gaps.

Validation: all 23 regression tests in ten AV1 suites and all 35 AV1 core
tests pass offline. The integration executable has no libav, FFmpeg or libaom
dynamic linkage. Existing sequence and entropy generator defaults remain
byte-identical in the checked compatibility cases.

## AV1 masked compound prediction

The owned reproducer for `AV1 masked compound prediction not implemented`
now decodes through native reconstruction. Two inter predictors blend using all
16 wedge indices with either sign, or using a difference-weighted mask with
either inversion. Difference weights are computed before clipping predictors,
with bit-depth/post-round normalization; chroma reuses the rounded subsampled
luma mask. Neighbor compound-group state contributes to later CDF contexts,
and joint distance-weight syntax is read only for the unmasked group.

`generate_av1_masked_compound_samples.py` produces 1056 owned OBU/WebM/YUV
triples. Coverage includes two distinct physical references selected as the
unidirectional LAST/LAST2 pair, all-masked and alternating masked/average
blocks, adaptive CDFs, mixed lossless/lossy segments, signed residuals and
strong hidden-reference contrast. The stronger difference cases exercise
spatially varying weights instead of only the constant base weight.
`tests/av1_masked_compound.rs` checks saved independent pixels, all segment maps,
decoder reset, displayed WebM timestamps, rewind and exact pixels after seek.
Ordinary tests require no encoder, external decoder, FFmpeg/libav or network.
The generator optionally uses libaom only as range writer and pixel oracle.

This matrix establishes 8-bit 16x16 blocks in 32x32 pictures and LAST/LAST2
routing. High-depth and rectangular compound blocks, alternate reference pairs
and interaction with global/local affine models still need further fixture
qualification. OBMC and scaled-reference prediction remain unresolved gaps.

Validation: 24 regression tests in eleven AV1 suites and 35 AV1 core tests
pass offline. A controlled fixed-weight counterfactual fails the strong-contrast
fixture with a pixel mismatch; restoring the difference-weighted implementation
passes. The integration executable links no FFmpeg, libav or libaom. Checked
legacy sequence/frame/entropy generator defaults remain byte-identical.

## AV1 overlapped motion compensation (OBMC)

The owned stream previously refused with `AV1 overlapped motion compensation
not implemented` now reconstructs natively. Above-neighbor blending precedes
left-neighbor blending, with normative masks, candidate stepping and neighbor
limits. Each overlap uses the neighbor's first reference, stored vector and
interpolation filters; neighboring warp and compound blends are excluded.
Tile boundaries and the small-plane restriction on the above pass are retained.

`generate_av1_obmc_samples.py` writes 2048 synthetic OBU/WebM/YUV triples.
Coverage includes left-only, above-only and both-pass blocks, two segment
reference assignments, four segment maps, adaptive CDFs, selected transform
sizes, signed residuals and contrasting hidden references, with zero and fractional global translation
vectors. Quarter- and half-pixel motion exercises overlap interpolation. Additional maps
make the top and left candidates use different references, so pass ordering
changes the output. `tests/av1_obmc.rs` checks saved independent pixels, segment
maps, decoder reset, WebM timestamps, rewind and exact pixels after sync seek.
Ordinary tests need no FFmpeg/libav, external decoder, encoder or network.
Optional libaom is limited to generator-side range writing and pixel oracle.

This matrix establishes 8-bit 16x16 blocks in 32x32 pictures, with 8-pixel luma
and 4-pixel chroma overlaps, regular interpolation and LAST/ALTREF routing. Additional block sizes,
high depth, small chroma planes, tile boundaries, compound/local-warp neighbors
and multiple candidates still need wider fixture qualification. Scaled reference
prediction remains an explicit unresolved AV1 gap.

Validation: 25 regression tests across twelve AV1 suites and all 35 AV1 core
tests pass offline, including the final expanded 2048-stream matrix. Controlled
counterfactuals disabling OBMC or reversing the above/left passes each fail with
a pixel mismatch; restoring the canonical implementation passes. Checked legacy
generator defaults remain byte-identical.
