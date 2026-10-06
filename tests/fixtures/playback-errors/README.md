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
