# Playback failures: short synthetic reproductions

Every new media parsing, decoding, timestamp, seeking or playback error found
during an audit must get a short synthetic video and an automated regression
test. Never copy private video, frames, audio or codec parameter sets into the
repository. Reproduce the relevant metadata and encoding structure instead.

Container I/O regressions use `ebml-interrupted-skip.webm`,
`y4m-interrupted-header.y4m` and `matroska-interrupted-copy.webm`.
Their tests inject a temporary read interruption at saved byte offsets, compare
against valid controls and ensure permanent failures still propagate. Regenerate
these authored clips with `python3 scripts/generate_container_io_regressions.py`;
no external encoder or network is needed. See
[`docs/improvements`](../../../docs/improvements/README.md) for checks and status.

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

## AV1 prediction from scaled references

The owned stream formerly refused with `AV1 scaled reference prediction not
implemented` now reconstructs natively. Reference/current ratios use 14-bit
precision; signed sample-center mapping, phase rounding and independent X/Y
steps use 10-bit coordinates. Horizontal/vertical filters vary their phases
per output sample, with a correctly sized intermediate buffer and reference
border clipping. Unscaled prediction retains its existing path. Local-warp
mode syntax is gated by the computed reference scale, and scaled prediction
uses the motion-vector path rather than the affine warp kernel. Ratios outside
the normative reference/current size limits fail as invalid input.

`generate_av1_scaled_reference_samples.py` writes 768 owned OBU/WebM/YUV
triples and one invalid-ratio OBU. Twelve size pairs cover enlargement,
reduction, crossed horizontal/vertical scaling and odd reference dimensions.
The matrix varies LAST/ALTREF, all four fixed interpolation filters, zero and
fractional translation, adaptive CDFs and segment maps. The generator also
handles hidden 16x16/rectangular references and explicit frame-size overrides;
legacy defaults remain byte-identical in the checked compatibility cases.
`tests/av1_scaled_reference.rs` compares every displayed sample, reference and
output dimensions, segment maps, decoder reset, WebM dimensions/timestamps,
rewind and exact pixels after seek. The invalid-ratio test is a refusal test,
separate from the successful playback acceptance test. Ordinary offline tests
need no FFmpeg/libav, external decoder, encoder or network; optional libaom
is limited to fixture-generation range writing and independent pixel oracle.

This matrix establishes 8-bit single-reference prediction in 16x16/32x32 and
rectangular pictures. High depth, larger pictures, more extreme valid ratios
and scaled references combined with compound/OBMC/inter-intra tools still
need additional fixture qualification. This does not establish full AV1
conformance or close the remaining codec gaps.

Validation: 27 tests across 13 AV1 integration suites and 35 AV1 core tests passed offline, including the scaled-reference pixel, map, dimension, rewind, timestamp and seek acceptance tests.

## Scaled AV1 masked compound qualification

The scaled-reference implementation now also has independent pixel acceptance
coverage for two-reference masked compound prediction. The 544 owned fixtures
exercise all 16 wedge indices, both signs, difference masks, adaptive tile CDFs,
four interpolation filters, opposite synthetic residuals and four configurations
of differently sized LAST/ALTREF references. Sizes include odd dimensions,
rectangular axes, upscale, downscale, and a scaled/unscaled reference combination.
The two references stay distinct: shown frames refresh no reference slots.

`tests/av1_scaled_compound.rs` checks all hidden/shown dimensions, segment maps,
all displayed Y/Cb/Cr pixels against saved independent oracle output, decoder
reset, WebM rewind, timestamps and seek. All 544 cases passed offline. All 1,632
fixture hashes were verified. Deliberately replacing the sampling step with unity
or adding one filter phase made the pixel test fail; the restored decoder passed.
The optional generator uses libaom only to write entropy and save reference pixels;
ordinary tests read checked-in files and need no FFmpeg, libav, libaom or network.

This extends the preceding single-reference qualification; it does not establish
full AV1 conformance. Scaled OBMC/inter-intra, distance-weighted compound,
higher depths, chroma formats and extreme valid size ratios still need coverage.

## Scaled AV1 inter-intra qualification

The native scaled-reference predictor is independently qualified with inter-intra
blending in 1,632 owned streams. The matrix covers DC, vertical, horizontal and
smooth modes; all 16 wedge masks; LAST and ALTREF logical references; 12 pairs of
reference/current dimensions including odd dimensions, downscale, upscale and
unequal horizontal/vertical factors. Interpolation filters and adaptive CDFs vary
across the matrix. Hidden reference pixels have a synthetic spatial pattern.
Shown frames refresh no references, preserving the intended size difference.

`tests/av1_scaled_interintra.rs` compares all displayed Y/Cb/Cr pixels with saved
independent oracle output and checks sizes, segment maps, decoder reset, WebM
rewind, timestamps and seek. All 1,632 cases passed offline, and all 4,896 hashes
were verified. Disabling inter-intra blending in the isolated verification copy
caused a pixel mismatch; restored source is rechecked before qualification.
Fixtures are generated separately with optional libaom entropy/oracle helpers;
ordinary tests require no FFmpeg, libav, libaom or network and contain no private
media. This extends the preceding scaled-reference coverage; scaled OBMC,
distance-weighted compound, higher depths/chroma and full AV1 conformance remain
unproven. The explicit film-grain, temporal motion-field and separate frame-header
limitations are still implementation gaps, not passing playback acceptance.

## Native AV1 separate frame-header and tile-group OBUs

The decoder now accepts coded OBU_FRAME_HEADER followed by ordered OBU_TILE_GROUP
units, including groups delivered in separate decode calls. It parses the header's
trailing-one bit and zero padding separately from OBU_FRAME byte alignment.
Header/CDF/reference state is published only after all tiles validate. Compatible
OBU_REDUNDANT_FRAME_HEADER copies are accepted, including extra zero padding;
layer mismatches, different header contents, missing/repeated/out-of-order groups
and a new header or delimiter before frame completion are specific errors.
Pending compressed data shares the decoder budget with retained reference pictures.
Reset drops pending state. A configuration record cannot hide an unfinished coded
frame. `Decoder::finish()` validates transport EOF, and native MP4/WebM readers
call it so an unfinished frame cannot silently become successful EOF.

The owned generator `scripts/generate_av1_separate_frame_samples.py` saves 96
scaled-reference/inter streams (12 size pairs, LAST/ALTREF, fixed/adaptive CDFs,
with/without redundant headers) and four lossless synthetic tile layouts with
2, 4 or 8 groups covering 4 or 8 tiles. Ten invalid streams exercise exact parser
failures; four partially delivered tile layouts reproduce incomplete EOF in raw
OBU, WebM and MP4. All fixture parameters and pixels are owned synthetic data.
No private source media or codec parameter sets are used.

`tests/av1_separate_frame.rs` checks exact oracle Y/Cb/Cr pixels, segment maps,
dimensions, publication at packet boundaries, reset, WebM/MP4 clocks, rewind,
seek, specific malformed-input failures, redundant-header padding, pending-byte
limits, configuration refusal and incomplete EOF. The pre-fix acceptance test
reproduced `AV1 separate frame header/tile groups not implemented`, rather than
an unrelated entropy error. Ordinary tests use only saved data and need no
FFmpeg, libav, libaom or network. Optional libaom tools are generator/reference
helpers; the generator's input images are synthesized internally. The generator
also preserves existing key/inter-header and WebM bytes for checked defaults.

This removes the separate-header refusal, not the remaining AV1 temporal-motion,
film-grain, palette, intrabc, super-resolution, restoration/profile/chroma and
layering gaps. Full codec conformance remains unproven.

Final validation: all 47 tests across 21 checked-in AV1 integration suites and 35 AV1 core tests passed offline on the final source. The six new tests passed; all 326 fixture/container hashes were verified. The regression executable has no FFmpeg/libav/libaom linkage. Verification and canonical AV1/MP4/WebM sources match.

## Native AV1 palette prediction

The native intra decoder now reads Y/UV palettes (2–8 colors), merges and deduplicates
eligible above/left palette caches, decodes palette color deltas and signed/modular
V deltas, builds diagonal color-index maps with normative context ordering and
extends map edges before predicting visible samples. Palette prediction replaces
ordinary intra prediction while retaining transform/residual reconstruction.
Filter-intra syntax is omitted when a Y palette is present. Neighbor palette state
is stored with the block grid; image accounting includes the added state.
`Picture::palette_counts` and `palette_cache_hits` expose actual coded use rather
than inferring palette coverage from source images.

The owned generator saves 168 streams: 8/10/12-bit 4:2:0, all palette sizes 2–8,
constant/varied chroma, matching/opposite U/V patterns, 64x64 and odd 125x117
sizes. Saved pixels agree byte for byte between independent libaom and dav1d
references; normal tests require neither oracle, FFmpeg/libav nor network.
All ordinary sources are synthesized internally, including palette parameters.
The fixture manifest records original source hashes separately from decoded hashes:
18 opposite-chroma cases produce reconstructed samples differing from the source
image with these generator settings, despite lossless encoding requested. Both
reference decoders and native FVid agree on the coded reconstruction. These cases
are decoder acceptance, not a claim of exact source-preserving encoding.

`tests/av1_palette.rs` checks visible sample cropping, all sample bits, sizes,
actual palette sizes in both plane groups for every depth, cached-color use,
reset/finish, WebM raw depth, timestamps, rewind and seek. The pre-fix stream
reproduced the exact native palette refusal. Deliberately replacing palette map
lookup by color zero or dropping the V-delta sign caused pixel mismatches; the
restored decoder passed. The WebM fixture wrapper now recognizes implicit visibility
in reduced headers. Saved existing wrappers remained identical for checked cases.

This qualifies the saved lossless-coded palette tools and 4:2:0 profiles; lossy
palette/residual combinations, further layouts and full AV1 conformance are still
unproven. Temporal motion fields, film grain, intrabc/super-resolution, restoration,
quantization matrices and additional layering/chroma behavior remain codec gaps.

Final validation: all 48 tests in 22 checked-in AV1 integration suites and 35 AV1 core tests passed offline. The 168-case palette acceptance also passed in the canonical checkout. All 504 fixture hashes were verified. The palette regression executable has no FFmpeg/libav/libaom/dav1d linkage. All canonical AV1 sources match the verification snapshot after restoration of counterfactual mutations.

## PS aligned stereo PCM reference

`aac-ps-dsp-oracles.json` and `aac-ps-dsp-reference.bin` are original
numeric QMF and stereo PCM references from
`scripts/generate_aac_ps_dsp_oracles.py`. Generation is explicit and offline.
They reuse the original PS matrix synthetic MP4 packets and syntax sequences;
no private media or codec parameters are used. Decimal direct convolution and
direct per-lag QMF synthesis qualify both output rates and lookahead alignment.
Native encoded PS refusal remains; these accept the composed component DSP.

## Streamed SBR/PS 960/1024 regression references

`scripts/generate_aac_sbr_ps_fixtures.py` explicitly generates two original
960-sample SCE/SBR/PS MP4s, `he-aac-sbr-ps-960-packets.bin`,
`aac-sbr-ps-30-oracles.json` and `aac-sbr-ps-30-coefficients.bin`.
The MP4s reuse only authored video/container seeds and own codec parameter sets.
`scripts/generate_aac_ps_dsp_oracles.py` extends saved stereo PCM references
with independently computed silent-core SBR noise QMF for both 960 and 1024.
Tests decode original SCE bits natively and compare the streamed SBR/PS bridge
against both PCM channels, real lookahead, zero EOF and rollback. Native combined
PS packet playback remains refused until its buffering/timeline integration.
No ordinary test runs generation, FFmpeg or network access.

## Native PS complete packet API regressions

`generate_aac_ps_native_fixtures.py` adds `aac-ps-native-oracles.json`,
`he-aac-ps-native-malformed-packets.bin` and six original three-packet MP4s.
They derive only from prior authored SCE/SBR/PS packets and video/container seeds.
The acceptance cases decode full original packets for both output rates and
explicit/sync signalling; the saved independent DSP references supply stereo
PCM expectations. Malformed MP4 cases separately require exact END/FIL/element
errors and unchanged subsequent audio. Full packet acceptance is distinct from
production factory/timestamp/seek support and nonzero-core PS conformance.
Generation is explicit, offline and absent from ordinary test execution.

## Delayed PS playback timing and stack regression

`generate_aac_ps_playback_fixtures.py` explicitly creates
`he-aac-ps-timing-960-synthetic.mp4`, `he-aac-ps-timing-1024-synthetic.mp4`
and `aac-ps-playback-oracles.json`. Original authored SCE/SBR/PS packets are
unchanged; `stts` windows differ so using lookahead packet metadata is detectable.
The adapter tests match independent stereo PCM and original PTS/durations through
EOF, checkpoint/reset and signed preroll. The same fixtures reproduce the native
checkpoint/adapter stack overflow before heap ownership; a dedicated 2 MiB
thread now accepts complete timing/recovery. Production worker integration remains
separate. Generation is offline/explicit and never run by ordinary tests.

## Production PS worker edit/EOF regressions

`generate_aac_ps_worker_fixtures.py` creates `aac-ps-worker-oracles.json` and
four original PS MP4s with silence/repeated ranges or an incomplete source range.
Only authored SCE/SBR/PS/video/container data is reused. The 960 repeat needs the
last queued packet at EOF before the next decoder reset; completed edits must
not present an out-of-range lookahead frame into silence. Worker tests exercise
the production factory, source-window trim, EOF, checkpoint, seek/rewind and
exact expected PCM slices. Source-gap files assert the exact error after drain,
not playback acceptance. No test invokes generation, FFmpeg or network.


## Absent and late PS regressions

`generate_aac_ps_absence_fixtures.py` creates eight original three-packet
`he-aac-ps-{all-mono,late-ps,missing-middle,trailing-mono}-{960,1024}-synthetic.mp4`
fixtures, `he-aac-ps-absence-packets.bin`, `aac-ps-absence-pcm.bin` and
`aac-ps-absence-oracles.json`. Original silent SCE and authored SBR noise/PS
parameters contain no private media or parameter sets. Independent Decimal
noise/decorrelation/matrix and direct synthesis references cover both output
rates. The normative mono mapping follows GOST R 53556.8-2013 6.5.1 and A.1;
decorrelation reset on returning PS follows A.3. Tests accept absent/late PS,
compare all PCM, retain synthesis tails, and replay checkpoint/reset/EOF.
Generation is separate, offline and never invoked by ordinary tests.

The same generator also creates six `he-aac-ps-{headerless-start,header-only-start,
dependent-start}-{960,1024}-synthetic.mp4` fixtures. They transmit a PS element
before a usable independent header, including zero envelopes or same-mode time
deltas. The first frame must emit nonzero dual mono; subsequent independent
headers must yield distinct stereo. The shared independent PCM oracle includes
all 14 videos, both output rates, continuous synthesis and replay.


## Matroska PS negative presentation preroll

`generate_aac_ps_matroska_fixtures.py` writes two short original AVC+PS MKVs,
`he-aac-ps-negative-preroll-{960,1024}-synthetic.mkv`, and
`aac-ps-matroska-oracles.json`. Original authored AVC and SCE/SBR/PS packets
are muxed with a -10 ms first audio timestamp; no private data or external
codec is used. References point to the independent absence/late-PS PCM oracle.
The worker regression first failed on the exact untrimmed preroll length and
now checks every surviving PCM sample, nanosecond PTS, EOF and bit-exact
seek/rewind recovery. Video decoding validates the authored AVC component.
Generation stays offline and separate from ordinary tests.


## Matroska PS export gaps and delayed EOF

`generate_aac_ps_matroska_export_fixtures.py` creates
`he-aac-ps-export-gaps-{960,1024}-synthetic.mkv` and
`aac-ps-matroska-export-oracles.json`, using only the original authored AVC
and SCE/SBR/PS data from the player regression. The last audio timestamp
is changed to 90 ms so both frame sizes have non-overlapping source intervals.
The independent PCM reference comes from the saved absence/late-PS oracle;
gap positions derive directly from original timestamps and first-source origin.
`he_aac_ps_matroska_export` reproduces the specific pre-fix PS synthesis refusal
and accepts every PCM sample, gap, interval and accepted-packet tail. It also
checks root reader byte equality, public WAVE export and admission before output.
Ordinary tests invoke neither generation, codec executables nor network.


## MP4 PS export repeated source and queued checkpoints

`generate_aac_ps_mp4_export_fixtures.py` writes
`he-aac-ps-export-repeat-{960,1024}-synthetic.mp4` and
`aac-ps-mp4-export-oracles.json`. Original authored AVC/SCE/SBR/PS packets
are reused with silence and two 3200-sample ranges starting at source 1920;
these ranges require the last queued packet at source EOF. References point
to the independent DSP stereo oracle, not decoder-generated PCM.
`he_aac_ps_mp4_export` reproduces the pre-fix PS synthesis refusal and accepts
all output PCM, selected intervals, accepted packet-limit tail and public WAV.
Repeated ranges exercise retained decoder/pending-window checkpoints. Existing
original unequal-window fixtures additionally prove source duration/PTS and
real lookahead beyond early interval endpoints. Root reader output must match
owned export byte-for-byte. Generation is offline and never run by tests.

The MP4 export suite also reuses the original worker source-gap MP4s to assert
the exact invalid-range refusal only after pending EOF PCM is written to the
caller-owned raw output. This is a refusal test for an invalid timeline, distinct
from the valid edited/unequal-window playback and export acceptance tests.


## Native in-band PS with LC/SBR ASC

`generate_aac_ps_inband_fixtures.py` creates four
`he-aac-ps-inband-{lc,sbr}-{960,1024}-synthetic.mp4` videos and
`aac-ps-inband-oracles.json`. Original authored late-PS packets and AVC seeds
are unchanged; only original ASC signalling is generated without PS metadata.
Saved independent PCM from the absence/late-PS oracle covers core/double output.
`he_aac_ps_inband` accepts the native candidate API, checks actual PS presence,
full PCM and atomic checkpoint/reset/EOF state. Known trailing-byte malformed
syntax additionally verifies that a rejected packet cannot latch PS presence.
Explicit disabled tools remain rejected; an ordinary no-PS mono stream rejects
the PS candidate at EOF while the normal owned mono decoder accepts it.
This does not assert automatic player/export container discovery. Generation
is offline and separate from ordinary tests, without external codec programs.

Layered HEVC output ordering witnesses (`generate_hevc_layered_reconstruction.py`):
`hevc-layered-output-reorder.mp4` has complete two-layer PCM AUs in coded POC
order 0,2,1,4,3. The owned post-reconstruction queue must emit POCs 0..4 and,
for each POC, layer 0 followed by layer 3. This reproduces decode-order output
without requiring private media or external codecs. The latency witness has
coded order 0,6,1,2,3,4,5: at HighestTid=2, POC6's latency reaches five and forces
additional bumping even after the pending-AU count is within the reorder limit.
Tests verify actual decoding, the former fixture's exact original PCM pixels,
output identities, repeated epochs, and atomic budget/identity refusals.
These standalone queue tests qualify F.13.5.2.3 additional bumping. The native
queued entry also covers output epochs and sub-DPB fullness with the witnesses
below; automatic player dispatch remains separate.

The queued native packet entry (`LayeredDecoder::decode_queued_packet`) stages
reference storage, parameter activations, random-access flags and output together.
`hevc-layered-output-bla-flush.mp4` and `-bla-discard.mp4` differ in the coded
prior-output flag: pending POCs 0/1 must respectively be emitted or discarded
before the two-layer BLA at POC2. `-cra-after-eos.mp4` uses flag zero but CRA
following base EOS still discards pending output. `-bla-flush-eob.mp4` also flushes
POC2 at EOB. All use original authored PCM entropy and exact saved pixel oracles.
`-bla-flush-truncated.mp4` removes 256 unescaped entropy bytes from the last
layer-3 PCM NAL while keeping valid framing and complete slice headers. This is
an intentional reconstruction refusal/rollback witness, not a playback
acceptance stream. The test requires successful AU parsing, preserved POC and
pending frames, and a successful retry using the valid BLA fixture.

Sub-DPB pressure witnesses use four storage slots per layer and a three-AU
reorder allowance. `hevc-layered-output-dpb-fullness.mp4` retains hidden reference
POC0 while POCs 1/2/3 await output. Before hidden nonreference POC4 can decode,
POC1 (both layers) must be bumped; additional bumping alone cannot detect this.
`-shared-reference.mp4` keeps POC0 both referenced and pending: count its storage
once, so the final hidden picture must not cause early output. `-rps-release.mp4`
drops the hidden reference in the final RPS and must not bump either. The passing
additional-only test is explicitly a reproduction of incomplete DPB accounting;
the queued native entry is the actual acceptance path. All pixel comparisons
use the original authored PCM oracle, across repeated resets.
`-layer-phase.mp4` has independent layers and layer-3 EOS before the final AU.
Base-layer pressure must output both pictures at POC1 before the subsequent
layer-3 CRA discards remaining pending pictures in layer 3. Applying all IRAP
output actions at AU entry would incorrectly discard layer-3 POC1 too early.
`-fullness-truncated.mp4` retains the same pressure condition but truncates the
second-layer PCM entropy of the final AU. It must parse successfully, refuse
reconstruction, preserve all three pending AUs and POC state, and then emit the
proper early AU when retried with the valid packet. This is a refusal/rollback
witness, separately tested from successful fullness playback acceptance.

`hevc-layered-output-reorder-ctts.mp4` adds an original signed composition-offset
map to the owned POC permutation witness. `LayeredMp4VideoReader` must preserve
each delayed result's original sample index, PTS and duration, including both
output layers at equal PTS. The indexed source also accepts all ten original
layered reconstruction witnesses and converts their own immutable colour/chroma
bindings into the common packed-planar player payload with exact saved pixels.
Tests cover rewind, later-IRAP seek (decoder-relative AU zero maps to source
sample two), global extraction across a metadata-only sample, error/reset, and
pre-publication VPS/OLS/temporal-selection refusals. The explicit layered
source API exposes all selected output layers. The common MP4/player entry now
automatically selects a native output-layer set and displays its highest eligible
output layer per access unit; it preserves ordinary single-layer codec dispatch.

`hevc-layered-random-access-alternative-radl-ctts.mp4` adds original signed
composition offsets to the alternative-output RADL witness. Its base picture
at POC1 is a permitted alternative when the primary layer is suppressed. The
common reader must display layers 0,3,3,3,3 at increasing presentation times,
with original source samples 1,0,2,4,3. These are coded playback acceptance
tests, including the ordinary native player source, rewind and seek; generation
uses the owned Python generator and ordinary tests need neither FFmpeg nor network.

`hevc-layered-hdr-reorder-ctts.mp4` carries original layer-0 prefix CLLI
(600/120 nits) and layer-3 suffix CLLI (1800/350 nits) on its first AU. Later
pictures omit SEI. Native layered output must preserve each layer's persistent
metadata on every picture through POC reordering and rewind. The companion
`hevc-layered-hdr-clvs-reset.mp4` starts a new CLVS with BLA: layer 0 states
900/200 nits and layer 3 states none. Metadata must reset per layer without
changing already queued old-CLVS pictures. These are coded-picture acceptance
fixtures generated with the owned PCM generator, independent of FFmpeg/network.
The common single-view player reports the displayed layer's immutable HDR
snapshot, including old queued pictures after a later CLVS has already decoded.
Rewind clears the displayed snapshot until a new picture is selected.

`hevc-layered-hdr-later-cra.mp4` has an ordinary later CRA without repeated SEI:
seeking there must preserve the original CLVS's layer-local 600/120 and
1800/350-nit statements. A BLA seek in the reset companion must instead clear
or replace them. Header/SEI priming keeps original metadata initialization
history separate from fresh pixel-decoder random access, discards skipped POC
history before decoding the indexed target, and preserves source sample/AU
identity. The native player source verifies HDR after seek and rewind.

`hevc-layered-hdr-later-cra-skipped-entropy.mp4` truncates only original layer-3
PCM entropy of the first AU. The full-playback refusal test verifies the exact
entropy error. Separate acceptance seeks to the later independent CRA and
restores both layers' original HDR without reconstructing skipped pictures.
The old parameter-only priming API reproduces missing HDR on the same target;
the metadata-aware path passes. Tests also require explicit completion before
decode, forbid mixing priming modes, reject priming an active pixel decoder,
and check that malformed header input leaves the staged POC state unchanged.

`hevc-layered-hdr-alternative-radl-ctts.mp4` combines original distinct layer HDR
with alternative-output RADL suppression and signed composition offsets. The
common player must display base layer 0 with 600/120-nit guidance first, then
layer 3 with 1800/350-nit guidance. This explicitly rules out metadata leaking
from the suppressed primary layer or the later packet that releases the frame.

### Layered HEVC VPS epochs

`hevc-layered-vps-epoch-{eos,eob,split,before-first,bla}.mp4` use original
PCM pixels to change output policy at legal sequence boundaries, including a
base-layer BLA without an explicit EOS. The split variant places the new VPS
in a separate packet; before-first replaces an unactivated configuration VPS.
The multiple variant overwrites an unactivated same-ID definition inside one
packet and finally activates the original VPS. Its extraction inventory must
remain global across EOS because the activated VPS is unchanged. The short
variant reduces the selected OLS temporal limit while old frames remain queued;
its metadata-only packet must preserve that queue until the new picture.

`hevc_layered_vps_epochs` checks exact pixels, immutable old bindings, original
sample timestamps, common-player selection, rewind and seek. The illegal variant
changes VPS on an ordinary non-IRAP picture and must refuse activation. Truncated
and bla-truncated cut enhancement-layer entropy: failed decoding must preserve
old parameters and queued output, and retry with the complete packet must work.
These tests require neither FFmpeg nor generation during test execution. The
following fixtures separately qualify referenced VPS IDs and graph changes.

`hevc-layered-vps-epoch-graph.mp4` replaces dependent layer 3 with dependent
layer 4 on a base BLA, preserving original PCM entropy and introducing a layer-4
PPS binding. Before the fix it specifically refused with `HEVC layered POC
configuration requires reset`. Acceptance checks exact saved pixels, old-layer
output snapshots, new POC history, common-player layer selection, rewind and seek.
The graph-truncated variant cuts only new enhancement-layer entropy and verifies
transactional parameter/POC/queue rollback followed by successful complete retry.
This qualifies a BLA graph transition; other VPS activation boundaries and
nonidentity inter-layer prediction tools remain separate requirements.


`hevc-layered-vps-epoch-ids.mp4` receives VPS ID 1 and then VPS ID 0 in one
packet, but activates ID 1 through the slice PPS/SPS chain at BLA. The ids-split
variant receives both IDs in a metadata-only packet before the new SPS/pictures;
ids-unused receives ID 1 without ever activating it and continues ordinary
pictures under ID 0. Before the fix the first two specifically refused with
`multiple VPS IDs in one source sample require activation selection`.
Acceptance checks exact PCM pixels, original timestamps, temporal extraction,
immutable old bindings, rewind and common-player seek. A header-only lookup test
also verifies that resolving the VPS reference does not activate or mutate the
parameter bank. Fixtures are authored without FFmpeg and generated separately.

`hevc-layered-vps-epoch-idr.mp4` replaces VPS at an AU containing IDR for every
necessary target layer, with no preceding EOS and no coded prior-output discard.
The idr-graph variant also replaces dependent layer 3 with layer 4. Both originally
refused with `HEVC active VPS changed before sequence end`. Acceptance compares
original PCM pixels and sample timestamps, preserves queued old-layer output,
and checks common-player seek/rewind across the new IDR epoch. The idr-truncated
variant cuts enhancement entropy and verifies atomic parameter/POC/queue rollback
and successful retry. The idr-missing and idr-mixed variants deliberately omit a
target IDR or replace it with an ordinary picture: VPS activation must still be
refused. These fixtures qualify the complete-target-IDR rule independently of
BLA; coded cross-layer BLA with an incomplete IDR target remains a separate case.

`hevc-layered-vps-epoch-cross-bla.mp4` codes cross_layer_bla_flag in a base IDR
and activates a new VPS without EOS. The target enhancement layer is absent in
that AU and enters in the next AU through CRA. The cross-bla-graph variant also
replaces dependent layer 3 with layer 4. Before the fix both specifically refused
with `HEVC active VPS changed before sequence end`. Acceptance checks original
PCM pixels, original timestamps, immutable queued old-layer output and
common-player rewind/seek to the base IDR before the delayed output-layer entry.
Cross-bla-unset carries the same parameter changes with the flag zero and must
still refuse activation. Cross-bla-truncated cuts only base-IDR entropy and
checks transactional parameter/POC/queue rollback and complete-packet retry.
These cases qualify a coded cross-layer restart with a later enhancement CRA;
same-AU mixed IDR/non-IDR pictures with POC reset require separate qualification.

### Mixed IDR/non-IDR POC reset and prior output

`hevc-layered-vps-epoch-mixed-reset-active.mp4` uses base IDR and enhancement
TRAIL_R in the same AU with full reset IDC 2/period 7, retaining the active VPS
and already initialized enhancement layer. It reproduced `HEVC output POC
precedes an already emitted picture`: the queue restored the old POC after
starting the new period. It also exposed incomplete-layer seek initialization
at a container sync sample; the source now prerolls from a complete layer entry
and hides preparation frames while preserving the requested sync PTS.

Mixed-cross-reset and mixed-cross-reset-graph combine the same reset with coded
cross-layer BLA/VPS activation. Their ordinary enhancement picture is correctly
ignored until its later CRA. Mixed-cross-reset-discard additionally sets the
prior-output discard flag and reproduced four leaked old pictures: reset drained
them before applying discard. Acceptance now expects only the later enhancement
CRA. Mixed-cross-reset-truncated cuts base entropy and verifies that a failed
reset/discard preserves old bindings, POC and queue until a successful retry.
Mixed-cross-reset-badidc omits the required full reset and must retain its precise
mixed-base-IDR refusal. Tests compare original PCM pixels and timestamps and
exercise rewind and common-player seek without FFmpeg or fixture generation.

### Received but unactivated same-ID VPS

`hevc-layered-vps-epoch-deferred-vps.mp4` ends the old CVS with EOS, receives a
same-ID VPS with a different output policy in a metadata-only sample, then
replaces it with the original definition before the next BLA picture. The
deferred-vps-first variant receives these unused definitions before any picture. Both
reproduced `HEVC extraction VPS disagrees with active picture` on the unused
metadata, rather than actual activation. Selected decoding now checks the VPS
bound to coded pictures; receiving an unactivated definition does not select it.
Existing direct/queued mismatch tests still verify refusal at real activation.

Acceptance compares original PCM pixels and sample timestamps, checks all
selected layers and temporal limits, and exercises direct decoding plus common
player seek/rewind. The truncated variant cuts enhancement entropy in the next
BLA: previously received parameters and queued old pictures must survive the
failure, and a complete retry must preserve old binding identities. Generation
uses only owned seeds and remains separate from ordinary offline tests.

### Monochrome inter-layer reference resampling

`generate_hevc_inter_layer_resampling.py` adds thirteen original monochrome
Y4M cases to the existing rational FIR atlas: 8/10/16-bit grid, identity and
phase operations plus depth increase, signed reference offsets and unused-chroma depth changes. Cb/Cr are
absent, not full-size placeholder arrays. All 58 prior oracle cases remain
unchanged. Acceptance checks luma coordinates/samples, empty chroma planes,
motion maps, repeated operations and memory/input validation. Two 16-bit
identity cases change only the absent chroma depth; they reproduced unwanted
FIR quantization (65535 became 65532) and now preserve every luma bit.

`generate_hevc_monochrome_inter_layer.py` authors two coded two-frame MP4s from
the documented `hevc-pcm-mono-reference-rext8` seed, without external codecs.
The identity control shares samples; the region variant enables same-AU base
prediction with reference-region offsets [2,2,2,2]. It specifically reproduced
`HEVC inter-layer resampling requires three sample planes`. Its saved oracle
uses the independent rational FIR implementation and differs from the identity
control. Syntax acceptance requires the enabled direct layer 0 and empty
short/long temporal RPS, so plain temporal prediction cannot satisfy the test.
Native playback compares every luma sample, original timing, empty decoded
chroma, packed neutral chroma, rewind and common/native seek. Region-truncated
retains complete headers but only one CABAC byte for the enhancement P picture:
failed prediction must preserve POC, parameter bindings and queued output and
allow a successful complete retry. Ordinary tests generate nothing and require
neither FFmpeg nor network. Coded acceptance covers an 8-bit base and 8/10-bit
enhancement; the standalone resampling atlas additionally covers 16-bit.
Scalable Monochrome profiles require matching chroma_format_idc for the layers
(H.265 V11, H.11.1.2); mismatches now produce a profile-conformance refusal.


`hevc-layered-mono-inter-depth10-{identity,region}.mp4` qualify coded bit-depth
scalability from an 8-bit base to a 10-bit enhancement. The dependent SPS inherits
its representation from VPS and keeps the owned 8-bit PCM sample syntax. Saved
16-bit little-endian oracles include PCM expansion and direct 10-bit rational FIR
rounding. Tests compare every sample, validate effective SPS depths and enabled
inter-layer prediction without temporal RPS, and check packed neutral chroma,
rewind and common/native seek. Depth10-region-truncated retains complete headers
but one CABAC byte: failure must preserve parameter bindings, POC and queued
output, followed by successful exact-pixel retry. Existing 8-bit files remain
byte-identical. Two Y4M cases separately require Invalid for mono/chroma mixing
under the scalable profile constraint; these are conformance refusals rather
than acceptance for a conversion algorithm. The sample atlas now has 73 cases.

Normative profile constraint: [ITU-T H.265 V11 (01/2026), H.11.1.2](https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-202601-I%21%21PDF-E&lang=e&type=items).

### AV1 codec configuration and MP4 initialization

`generate_av1_configuration_samples.py` reuses the original separate-frame /
tile-group 128x128 seed and wraps unchanged coded payloads in small MP4s.
Configuration-only sequence cases have an empty sync-sample list, with the
sequence header in av1C. They specifically reproduced `AV1 frame precedes
sequence header`: the MP4 source constructed an unseeded decoder and ignored
the configuration OBUs. The source now initializes and resets the native decoder
from av1C. In-band and duplicate-inband controls keep ordinary sync samples;
their repeated sequence is legal across configuration and sample boundaries.
The records use matching profile/level/colour hints and include nclx colour data.

Acceptance compares every saved Y/Cb/Cr sample, original PTS/duration, direct
codec decoding, decoder reset and player rewind/seek. Delay-present values 0 and
15 are structurally accepted. Nonzero reserved bits, missing marker, empty or
truncated container records, repeated config sequence and a sequence after
another config OBU are exact refusal cases; unknown versions remain Unsupported.
The old unchecked-reserved/sequence-order acceptance was reproduced separately
before these expectations were enabled. Ordinary tests neither generate files
nor invoke FFmpeg, an external codec or network.

[AOM AV1-ISOBMFF v1.3, sections 2.3 and 2.4](https://aomediacodec.github.io/av1-isobmff/)
define the record fields, configOBU ordering and sync-sample requirements.
Comparison of every av1C profile/level/colour hint against sequence headers and
full presentation-delay model validation remain separate work; these fixtures
qualify initialization and structural checks only.

2026-10-13 AV1 codec configuration summary validation: owned MP4 fixtures in
`av1-configuration.json` independently toggle profile, operating-point-zero
level/tier, high-bitdepth, twelve-bit, monochrome, subsampling X/Y and chroma
position. Both configOBUs and later in-band sequence headers require the exact
summary match; reset/rewind retain the validation. The valid controls still
compare native decoded pixels against the saved separate-tile oracle.

`av1_fixture_configuration.py` derives fixture summary bytes with the native
Rust sequence parser via the offline `av1_configuration_fields` example. It runs
only during explicit generation, never ordinary tests. The owned container
corpus formerly used zero placeholders. `repair_av1_fixture_configuration.py`
(dry run by default, `--apply` to write) updates just bytes 1/2 of av1C/AV1
CodecPrivate and corresponding saved manifest hashes, preserving packet bytes,
lengths, offsets and pixel oracles. Deliberately malformed configuration fixtures
are excluded; regenerate those with `generate_av1_configuration_samples.py`.

2026-10-13 AV1 MP4/Matroska implicit final OBU sizes: `av1-implicit-size.json`
contains owned two-tile-group MP4 and WebM samples whose last tile group omits its size
field, with and without extension headers. Additional samples end in an implicit
padding OBU or an owned HDR CLL metadata OBU (1234/567). All four variants in each container reproduced
`AV1 low-overhead OBU requires a size field` before the fix. Enabled acceptance
compares saved exact pixels, timestamps, sync seek, rewind, direct decoder reset
and metadata available at open. The ordinary raw low-overhead API remains strict.

A separate bounded sample iterator/API permits the final OBU to fill the sample
remainder without copying bytes or guessing subsequent headers inside payloads.
Malformed extension headers still fail and poison the decoder until reset;
configOBUs still require explicit sizes. Three saved negative samples in each
container verify those refusals. WebM playback and the fvid-media stats decoder
use the same bounded framing. Regenerate explicitly with
`python3 scripts/generate_av1_implicit_size_samples.py`; no ordinary test invokes
it, external codecs, FFmpeg or the network.

2026-10-13 AV1 media-library MP4 admission: `av1-library.json` wraps the owned
8-picture temporal-SVC stream into plain/repeated/disjoint/leading-edit MP4
files, retaining the independent saved oracle. The old media-library decode
refused the plain MP4 with `owned video decode does not yet support this
container, codec or transform`. The enabled acceptance checks counts, exact
presentation ordering/timestamps, negate, framestep, reverse and FFV1 export.
The shared WebM streaming adapter also accepts bounded implicit final sizes and
selects the highest displayed spatial layer, matching native playback.

10/12-bit synthetic wrappers retain existing independently decoded oracles.
The 70x70 AV1 wrapper exposed padded-row aliasing in MP4 playback: byte 140
(the first byte of row 1) differed from the saved 10-bit reference. Visible rows
now use each AV1 plane's stride. Additional 191x127 8/10-bit wrappers qualify
ceil-divided chroma geometry through playback, filters and lossless export.
Generate explicitly with `python3 scripts/generate_av1_library_samples.py`;
ordinary tests only read saved fixtures. No private input or external codec is
used by the generator or tests.
The same AV1 library acceptance now includes owned 4:2:2 (8/10-bit) and 4:4:4
(8/12-bit) wrappers, retaining their original saved chroma-plane oracles. Invalid
configuration and in-band header failures must retain the precise owned error
and leave a pre-existing export destination unchanged. RGB AV1 streaming export
remains a separate capability gap, not counted as covered by these YUV cases.

2026-10-13 AV1 identity RGB playback: `av1-rgb.json` contains owned 8/10/12-bit
MP4, WebM and low-overhead samples. The generator authors the conforming
1/13/0 identity color description in existing synthetic 4:4:4 headers and
reinterprets their independently saved G/B/R plane samples as interleaved
R/G/B, without a color conversion or external codec. Native sequence parsing
and direct decode prove the intended color configuration and exact samples.
Both container players previously refused with
`unsupported AV1 RGB matrix coefficients`.

Enabled RGB playback acceptance checks source-depth samples, explicit display
normalization, timestamps, rewind, seek, byte-exact rotation, flip and black
padding. Interleaved RGB uses GeometryFrame subsampling=None; sample depth stays
8/10/12, with LE word storage above eight bits. YUV shaders never consume RGB
bytes. The owned arbitrary-angle rotation adapter also supports RGB LE words;
the 10-bit sample previously refused with `RGB rotation requires 8-bit samples`.

RGB lossless export is NOT an enabled acceptance case yet: the separate passing
refusal test records missing native FFV1 RGB support. Compressed MP4/Matroska
capability refusals now stay explicit rather than falling through to Y4M ASCII
parsing. Replace that refusal with exact export acceptance when RGB FFV1 is
implemented. Regenerate explicitly with
`python3 scripts/generate_av1_rgb_samples.py`; ordinary tests are offline and do
not generate fixtures or call FFmpeg/external codecs.
Rotated RGB MP4 variants set tkhd rotation=90 and retain the coded 64x48 planes.
They qualify upright 48x64 display codes and source-depth raw/geometry APIs.
The playback queue originally lost rotation while taking the RGB presentation
path (the depth-8/90-degree runtime regression failed before the fix); it now
rotates before explicit display conversion. Native raw samples stay unrotated.

2026-10-13 Native FFV1 v1 RGB core: `ffv1-rgb.json` stores independently authored
Python range-coder packets, 4x2 channel-distinct/extreme sample oracles, complete
RCT plane traces and two-frame Matroska videos at 8/10/12/16 bits. RFC 9043
requires a distinct blue-based RCT for 9..15-bit RGB without alpha; 8/16 bits
use the ordinary green-based RCT. Coding is line-interleaved with n+1 residual
bits and positively offset chroma differences. The old decoder refused these
packets with `unsupported FFV1 capability: FFV1 RGB decoding is not implemented`.

Enabled acceptance requires exact independently generated packet bytes from the
Rust encoder, exact samples from the Rust decoder, reset/keyframe recovery,
explicit memory-budget refusal and bounded truncation handling. Native Matroska
playback also verifies RGB precision, display codes, timing, rewind and seek.
This qualifies v1 range-coded RGB without alpha; version 3, Golomb Rice and
alpha remain separate gaps. RGB AV1 media-pipeline export still needs wiring to
this RGB core; its existing refusal test is not export acceptance. Generate
explicitly with `python3 scripts/generate_ffv1_rgb_fixtures.py`; ordinary tests
only read saved fixtures and require no FFmpeg, external codec or network.
Three malformed RGB videos/packets require exact failures for missing chroma,
subsampled RGB and out-of-depth reconstructed channels, before publication.
The existing threaded YUV-rotation test also now verifies the current GPU
contract: coded 12-bit planes plus Frame rotation metadata, then exact rotated
samples/display codes. Its old expectation failed identically on baseline HEAD
9f426958b, independently of the RGB codec implementation.

### Delayed cloud startup

`cloud-startup.y4m` is a three-frame 2×2 synthetic clip generated by
`scripts/generate_cloud_startup_fixture.py`. The player regression gates source
delivery to reproduce a cloud open that has not returned yet, checks loading
state while polling stays responsive, then requires successful playback setup.
All inputs use a streaming buffer instead of whole-file preloading or a cold speed probe.
Tests do not require FFmpeg, a cloud mount, or network access.

`streaming-block-boundary.y4m` is a generated 0.48-second synthetic clip.
The single-worker regression pins playback at the start, requests an uncached
tail, then reads the entire clip across block boundaries and compares every byte.
Shared-source regression checks that video/audio cursors remain independent
and reuse an already open descriptor without reopening the source path.

The retained-RAM regression loads the synthetic block-boundary clip, invalidates
its source, seeks to the middle and back, and requires every cached byte to
remain available until the item is dropped. A subsequent open sees the new source.

### VP9 software long-GOP seeking

`vp9-long-gop-seek.webm` contains 10.2 seconds of synthetic moving test pattern
with 3.4-second keyframe spacing. Generate with
`scripts/generate_vp9_seek_preroll.py`; ordinary tests require no FFmpeg/network.
`vp9_long_gop_seek` forces the owned software decoder and compares forward and
backward seek timestamps and all RGB pixels with sequential playback, including
targets near the end of a GOP. Performance is measured separately on the player
build; this acceptance test verifies correct pre-roll rather than a flaky wall-time limit.

### Owned compressed MP3 frame chains

`mp3-decoder/` contains eighteen authored four-frame Layer III chains, neutral VP9
Matroska companions and independent PCM oracles. They cover MPEG-1/2/2.5 at
all nine MP3 rates, mono and MS stereo, protected frame CRCs, padding, explicit/implicit
Huffman regions and mixed start/short/stop transitions. Rebuild separately with
`scripts/generate_mp3_decoder_fixtures.py`; no FFmpeg, network or private source
is used. `mp3_synthesis_oracle.py` evaluates absolute subband timelines and
polyphase convolution independently of the decoder's persistent overlap state.
Core acceptance lives in `crates/fvid-codecs/tests/mp3_decoder.rs`; video/audio
container acceptance and the saved synthetic tone reference comparison live in
`tests/mp3_native_reference.rs`. Production factory and navigation acceptance
live in `tests/mp3_owned_seek.rs`.

The MP3 chain set was expanded to twelve cases with 44.1/22.05/11.025 kHz
companions. `tests/mp3_owned_seek.rs` exercises the production owned factory,
exact sample suffixes, quantized Matroska timestamps, EOF and worker navigation.
The production MP3 factory now uses the owned decoder; older milestone notes
above describe its pre-integration state.

### Free-format Layer III framing

`mp3-free/` contains 39 authored free-format cases with neutral VP9 Matroska
companions and independent PCM: all nine rates, mono/MS, padding, 640 kbit/s spacing,
false ancillary sync words and three single-frame files. It also contains a
four-frame second-CRC corruption regression for raw and bounded-container paths.
Rebuild separately with `scripts/generate_mp3_free_fixtures.py`; run acceptance
with `tests/mp3_free_player.rs`. No private media, encoder, network or FFmpeg is
used. The single-frame fake-header fixture is retained after fixing its initial
framing refusal.

### MP3 checkpoint navigation

`mp3-checkpoint/` contains authored 160-frame raw/Matroska chains at 48/44.1 kHz
and a backwards-audio-clock Matroska variant; all video is neutral 16x16 VP9.
Generate separately with `scripts/generate_mp3_checkpoint_fixtures.py` and test
with `tests/mp3_checkpoints.rs`. Acceptance compares restored state to continuous
PCM and counts encoded packets after seek, including the quantized clock path.
These fixtures qualify navigation; independent codec PCM oracles live in the
other MP3 fixture sets. No FFmpeg, encoder, network or private media is used.

### Native MP4 video clock precision

`native-video-clock/clock.mp4` reuses twelve authored AVC pictures from
`control.mp4` with an adversarial integer clock (7 ticks/second) and a version-1
media duration beyond the u64 nanosecond horizon. The generator
`scripts/generate_native_video_clock_regressions.py` reads only that saved
synthetic control. `tests/native_video_clock.rs` verifies exact cached packet
endpoints, declared duration, actual decoding, and cache stability after rewind.
Ordinary tests run neither the generator nor an external media tool.

### MP3 gapless metadata and priming history

`mp3-gapless/` is generated offline by `generate_mp3_gapless_fixtures.py` from
FVid-authored compressed chains, independent source PCM and neutral VP9 pictures.
It contains 115 indexed/free gapless cases at all nine rates, metadata refusal
controls, two Info-reservoir priming cases and 864 tag-layout vectors.
`coded-marker` preserves actual coded audio beginning with `Info`; its independent
convolution PCM oracle includes subsequent mixed start/short/stop blocks.
`info-only` is a specific no-audio refusal. Core metadata tests and root
`mp3_gapless_player` acceptance never invoke the generator or FFmpeg.

The same saved MP3 video corpus is also consumed directly by standalone
`fvid-media/tests/matroska_mp3_export.rs`: 110 applicable cases qualify library
presentation decode/intervals, and `primed-free` qualifies public WAVE/DSP export,
resource limits and cancellation. `coded-marker` qualifies full library PCM.

`fvid-media/tests/raw_mp3_export.rs` reuses the saved MP3 video-associated raw
corpora for 115 gapless/control cases and 39 pure free-format cases. It qualifies
raw library PCM windows, public WAVE/DSP export, Info packet accounting, encoded
packet caps (1921-byte acceptance versus 1920 refusal on the 640k padded chain),
input/index allocation admission and cancellation. The parser is canonical for
both the player and media library; ordinary tests invoke no fixture generator.

### Owned MPEG Layer I and MPEG clock metadata

`mp1-native/` is authored offline by `scripts/generate_mp1_native_fixtures.py`.
The 258 primary cases span nine rates, all channel modes and joint boundaries,
CRC/indexed/free framing, every nonzero allocation width, all scalefactor indices
and all 32 bands, including dense frames. Independent signed-fraction/convolution
PCM and binary frame-size metadata accompany each short neutral VP9 video.
False ancillary headers are inserted only beyond the real coded payload.

Additional saved inputs qualify a single EOF-bounded free frame, a smaller valid
free chain and specific CRC/reserved-allocation/truncation/spacing refusals. The
bad-second-CRC derivative must not be hidden inside a larger candidate. Thirty-six
extra videos carry floor/ceil CodecDelay/tail metadata to qualify exact sample
PTS, durations, seek, rewind and audible PCM windows. Decoder references that
refuse free format are kept distinct from owned free-format acceptance.

Core `mp1_decoder`, root `mp1_player_owned` and standalone `mpeg12_export` tests
consume these saved artifacts. The latter also qualifies existing Layer II
raw/Matroska library exports and common MPEG probe/plan/WAVE DSP controls.
Ordinary tests require no generator, external media process or network access.

### MPEG Layer II/III exact navigation and quantized metadata

`mpeg-clock-metadata/` is authored by
`scripts/generate_mpeg_clock_metadata_fixtures.py` without external processes or
network access. Its 18 raw bases and 90 neutral VP9 video companions cover both
layers at nine rates, with floor/ceil CodecDelay and terminal padding variants.
Layer II low-frequency bodies use the same normative table-4 allocation at six
rates; the generator re-clocks unprotected authored headers only. PCM oracles
are referenced from existing authored corpora, never private media.

`mpeg23_sample_navigation` checks exact PCM/PTS/duration/seek/rewind and captured
workers. A cold second-packet Layer II decode proves the lost-history failure;
full seek replay is the acceptance path. `mpeg_clock_metadata` checks standalone
library decode and audible intervals. Tests consume saved files only. Complete
regeneration yields 109 identical files, including 108 hashed artifacts.

### Owned FLAC Matroska library regression

`flac-matroska/` is generated by `scripts/generate_flac_matroska_fixtures.py`
from saved authored FLAC frames and neutral VP9 pictures. Its 1460 short videos
cover all 1–8 channel counts and seven source depths. No private input or external
codec process is used. `flac_matroska` checks independent integer-oracle full and
interval PCM, public plans/WAVE/DSP, quad channel masks, cancellation and memory
admission. `flac_matroska_companions` really decodes each saved companion video.
The pre-fix test failed specifically at missing owned Matroska audio dispatch;
the same test is now enabled as acceptance. Ordinary tests read saved artifacts.

`raw_flac_export` reuses the same authored FLAC frame corpus and its saved
`flac-matroska` video companions. It checks every integer PCM oracle at full
length and sample intervals, metadata inspection, public WAVE/DSP/plans at all
channel counts, resource admission and cancellation. The before-fix public
plan failed specifically by dispatching FLAC into the WAVE handler. The
canonical bare FLAC index is now shared between the player and library, with
existing parser regressions retained in the player adapter tests.

### FLAC explicit speaker layouts

`flac-layout/` contains six authored FLAC inputs and six neutral VP9 companions,
produced by `scripts/generate_flac_layout_fixtures.py`. Only comment metadata is
added to existing authored FLAC frames; no private media or codec parameters are
used. The examples cover LFE, FC/BC, height, center-only, zero and partial masks.
`flac_layout` checks raw/Matroska public plans, WAVE masks and unchanged independent
PCM oracles; `flac_matroska_companions` decodes every video companion. The original
acceptance reproduced a default-mask substitution; a second run reached the old
partial-mask WAVE refusal. Both paths now pass. This is preservation acceptance,
not a claim of general height/partial spatial downmix or device playback.

The layout corpus is extended to 26 cases (53 files, 52 hashed artifacts),
including every named speaker and explicit mono/stereo DirectOut zero.
`pcm_speaker_matrix` checks independent impulse coefficients in float32/float64,
partial/unassigned positions, expansion and classic-WAVE versus explicit-zero
semantics. `flac_layout` additionally checks public mono/stereo interval exports
from raw and Matroska files against amplitude-scaled independent PCM oracles.
The original matrix admission refusals have enabled acceptance tests. Speaker-aware
audio-device negotiation is separate from these library export tests.

The corpus now includes rear-only stereo (27 cases, 55 files, 54 hashes).
`flac_device_layout` verifies raw/Matroska source masks reach the audio worker
and that routed playback/rewind preserves PCM and sample time. CPAL's
`explicit_flac_speaker_route_regressions` checks its real push/ring/callback
code with independent saved integer oracles, including flush/re-anchor. This
also reproduces the formerly overlong Matroska STREAMINFO setup. Every video
companion is decoded by `flac_matroska_companions`. A separate ignored physical
smoke test only opens/closes a paused device; ordinary tests remain hardware-free.

### AAC device positions and precise seek

`aac-device-layout/` is authored by `scripts/generate_aac_device_layout_fixtures.py`.
It wraps saved authored AAC PCE frames and independent cosine PCM in neutral VP9
Matroska and authored AVC MP4 video companions. Literal per-channel weights form
independent stereo oracles. The 18 layouts include bottom/height, repeated and
ten-channel front groups. Two positive-edit MP4 derivatives reproduce priming
plus sample seeks. Two floor/ceil CodecDelay videos additionally qualify exactly 17-sample trim.
The corpus has 77 files, 76 hashed artifacts, deterministic
complete regeneration and no private media/codec parameter sets. The source
height generator adds `normal-ten` without changing previous artifacts.

`aac_device_layout` first proves every raw/Matroska/MP4 source really decodes,
then checks routing, independent stereo PCM, workers, rewind and sample seeks.
The before-fix test reported missing routes in every original combination;
stronger seek acceptance reproduced packet landing and one extra sample lost
through ns rounding. Those reproductions are now enabled acceptance, including
strict 1 ns boundary controls and positive edits/EOF. CPAL callback tests use
independent PCM without a device. Physical paused-open smoke tests are ignored
by ordinary runs and explicitly invoked separately. PCE-aware library spatial
export and dynamic device-layout renegotiation are not qualified by this corpus.

### Quantized AAC sample clocks (2026-10-16)

`aac-clock-coarse.mkv` and `aac-clock-jumps.mkv` are authored ten-channel AAC
plus VP9 videos, six audio/video frames each. Explicit offline regeneration:
`python3 scripts/generate_aac_clock_fixture.py`. Hashes are recorded in
`aac-clock-generated.json`. No private media or FFmpeg is used.

`tests/aac_clock.rs` checks actual video decode, independent authored PCM,
millisecond packet quantization, forward/backward audio discontinuities,
rewind, and exact 50 ms / 50 ms + 1 ns seek trimming. The video timeline remains
monotonic so audio discontinuities cannot fail for an unrelated video error.
Ordinary tests consume saved fixtures only.

The same offline generator also saves `aac-clock-gap.mkv` (forward gap only)
and `aac-clock-long.mkv` (72 packets, 1.536 s, quantized to milliseconds).
The gap fixture qualifies native library silence insertion and intervals.
The long fixture qualifies paired decoder/sample-clock warm seek at 1.3 s
against uninterrupted PCM, and actual decode of all 72 VP9 frames. Endpoint
checks cover coarse rounding, forward/backward epochs and stability on rewind.
All four videos are hashed in `aac-clock-generated.json`.

The generator now saves eight hashed videos in total. `aac-clock-tail.mkv`
trims 13 samples from the final ten-channel AAC AU. `aac-clock-invalid-tail.mkv`
requests 1025 samples from a 1024-sample AU and must be explicitly refused.
`aac-clock-ps-{immediate,late}-tail.mkv` wrap original authored Main-PS ADTS
payloads with different 7/13-sample penultimate/final trims, so lookahead metadata
cannot accidentally pass by using equal trim values. Tests actually decode the
companion VP9 pictures, drain delayed audio at EOF, and check PCM and endpoints
on rewind. Public spatial export independently checks valid LC trimming and
atomic refusal. No private media or external encoder is involved.

### AAC signed windows, source origins and exact tick gaps

`generate_aac_clock_fixture.py` now saves 14 hashed videos. The five additional
`head`, `middle-head`, `head-gap`, `offset-head` and `full-head` cases preserve
explicit packet coordinates through signed trimming. `tick-gap` has PTS
0/21/42/65/86/107 ms: its exact 64 ms coded boundary must not absorb the genuine
1 ms jump to 65 ms. The library oracle includes 48 samples of silence there.

`crates/fvid-media/tests/aac_signed_padding.rs` checks independent native PCM,
public WAVE, requested intervals, delayed PS internal/final trimming and the
exact-tick gap. `tests/aac_signed_padding.rs` checks packet windows, source
positions, seek and duration. The CPAL signed-timeline unit checks the actual
stereo route/ring/callback against independent saved stereo PCM. All videos are
actually decoded by the player acceptance tests. The manifest records per-file
packet/channel geometry, including the 2048-sample stereo PS exceptions.

The three PS export generators and the ASC-clock derivative now place unchanged
scalar PCM according to declared -10/40/90 ms source timestamps. Negative
preroll is cropped, not shifted forward. Their 18 existing media artifacts were
verified unchanged while updating oracle metadata. This supersedes the former
first-packet rebasing expectations without changing codec payloads.

VP9 seek-cache checks also cover background preparation of unvisited GOPs,
restoring exact decoder states, native keyframe-seek semantics, and continued
raw/RGB playback through cached frames to EOF without skipped or repeated pictures.
All decoded picture/state caches are bounded and live only in memory for the open item.

### AAC scaled block-clock regressions

`aac-scaled-clock/` contains 12 short owned VP9/AAC videos and one owned ADTS
warm-seek control, with a hash manifest. Explicit offline regeneration:
`python3 scripts/generate_aac_scaled_clock_fixtures.py`. The generator uses exact
Python rational arithmetic for IEEE track scales and existing original AAC
payloads; it uses no private media, foreign encoder, FFmpeg or network.

Nine LC/PCE videos cover factors 2, 0.5, 1/3 and just above one, shifted cluster
origins, lacing, and two genuine gap cases. Two Main/PS videos combine fractional
scaling, shifted origins and lacing with immediate/late PS. The 72-AU warm case
is 1.536 s and includes a saved repeated ADTS control. The nine initially saved
videos remain byte-identical after adding PS/warm cases.

`tests/aac_scaled_clock.rs` actually decodes all videos and checks exact packet
PTS, independent LC PCM, full ADTS PS/long controls, strict sample seeks and the
paired warm checkpoint. `crates/fvid-media/tests/aac_scaled_clock.rs` checks
incremental grid retention/lace offsets, PCM intervals including gap starts,
public stereo export, accepted packet-prefix geometry and atomic budget refusal.
Tests consume saved fixtures only. Explicitly signalled PS prefixes use an
explicit PS codec control; truncated ADTS discovery would otherwise select
mono/core before late PS is encountered.

### PCM Matroska origin, trim and exact seek

`pcm-timeline/` contains 49 short authored VP9/PCM videos and a hash manifest.
Explicit offline generation: `python3 scripts/generate_pcm_timeline_fixtures.py`.
Forty-eight acceptance cases combine four timelines with the supported integer
and float widths/byte orders, compatibility defaults and stereo. Dyadic ramp
samples have independent exact integer/floating oracles. The scaled-gap cases
use legacy v3 TrackTimestampScale without v4 padding; signed-padding variants
use v4 and unit track scale. `bad-last-f32-1.mkv` is an explicit malformed-frame
refusal, with valid companion video, not intended playback acceptance.

`tests/pcm_matroska_timeline.rs` decodes every video and checks packet PTS/PCM,
requested-sample seek, duration, and an instrumented raw float64 seek with zero
preroll payload reads. Seeking beyond EOF must still report malformed stride.
`crates/fvid-media/tests/pcm_matroska_timeline.rs` verifies independent f32/f64
presentation intervals, planning/public WAVE payloads, and atomic refusal.
Ordinary tests read saved inputs only and require no FFmpeg or network.

`aac-scaled-clock/` additionally saves immediate/late PS negative-preroll cases,
bringing that corpus to 14 videos plus one authored ADTS control. The first source
block maps to -10 ms and exactly 480 source samples are cropped. Source-grid
coordinates, delayed output identity, strict seeks and public export are checked
without changing previous media artifacts or invoking generators in tests.

`mpeg-clock-metadata/mp2-48000-tick-gap.mkv` and its Layer III twin
add a genuine 1 ms gap after the first 24 ms audio packet. Their generator uses
only existing authored MPEG packets and neutral synthetic VP9 frames. The player
regression decodes the video and audio and requires the second packet at 25 ms;
previously its one-tick tolerance incorrectly moved it to 24 ms. This test covers
the unit-scale player clock; library export and scaled MPEG grids are covered by additional tests below. Ordinary tests consume saved files without generation.
The one-tick MPEG gap regression additionally covers library Layer II/III full
PCM, a 24–26 ms interval and public WAVE, preserving exactly 48 silence frames.

The MPEG clock corpus also includes four 44.1 kHz Layer II/III videos with
TrackTimestampScale 2 and the exact IEEE encoding of 1/3 (Matroska v3).
The fractional case reproduced a packet at 78,333,333 ns instead of its coded
sample position 78,367,346 ns. Player timing and duration now use retained source
block grids; tests decode every companion frame and check cold seeks at 30 ms
and 30 ms plus 1 ns. Library full/interval PCM uses independent saved oracles.
No external generator or decoder runs in ordinary tests.

Two additional MPEG Layer II/III 44.1 kHz videos combine factor 1/3 with
shifted cluster origins. The player seek test now also compares source PCM
against saved independent oracles; library full and interval exports cover the
same shifted cases. The source factor applies to block-relative coordinates.

FLAC companions now include `*-offset.mkv`, with authored audio starting at
50 ms and neutral VP9 at zero. The library used to rebase audio to zero;
full and 49–52 ms interval PCM regressions require the original leading silence
for all saved FLAC format/kernel cases. Player acceptance decodes the companion
video and compares independent integer PCM after seeks at zero, 51 ms and
51 ms plus 1 ns. Previously the 51 ms seek presented the complete 50 ms packet.
Inputs are generated separately using only existing authored FLAC bytes and the
owned neutral VP9 writer; ordinary tests do not require FFmpeg or network.

`flac-timeline/` contains two authored four-packet mono FLAC/VP9 videos at
44.1 kHz, with unit and fractional 1/3 track scales. Frames have canonical coded
numbers and independently specified constant levels 4096,-4096,8192,-8192.
Each frame contains 64 samples. Player exact timestamp/PCM and library full and
1.5–4 ms interval regressions cover continuity beneath quantized block clocks.
Generated separately by `scripts/generate_flac_timeline_fixtures.py`, using own
FLAC CRC/header/constant-subframe writers and own neutral VP9 construction.
`flac-timeline/` additionally has v4 `tail.mkv` and `discard-last.mkv`, which
trim 13 samples or all 64 samples of the last packet. Player duration after EOF
must match the audible endpoint and survive rewind/seek. The existing unit and
third-scale cases reproduce the old 6 ms versus exact 5,804,988 ns duration bug.
Library full/interval PCM includes both padding variants. These are acceptance
regressions; the original two saved videos remain unchanged.
The FLAC timeline duration assertions also run immediately after opening, before
any PCM decoding. The reader walks coded headers once and restores its packet
cursor; the same exact duration survives playback, seek and rewind. This is an
I/O pass over encoded packets, not a PCM preroll or an additional stored index.

`opus-clock/` has two authored three-frame VP9/Opus DTX videos: a genuine 1 ms
pause after the first 20 ms audio packet, and a positive 1 ms audio origin.
The player regression decodes every companion frame and each 960-sample Opus
packet, requiring source timestamps 0/21/41 ms and 1/21/41 ms respectively.
Previously the one-tick tolerance shifted 21 ms to 20 ms. The initial-origin
and continuity checks now use adjacent quantization bounds rather than a blanket
tick tolerance. Generation is explicit and offline via
`scripts/generate_opus_clock_fixtures.py`; ordinary tests require no FFmpeg.
Library Opus clock acceptance uses the same gap/origin movies. A direct owned
Opus packet decoder supplies PCM controls; the independently authored timeline
inserts exactly 48 silence samples. Full/19–22 ms interval PCM and public
float32 WAVE must match. This reproduced 2880 versus 2928 exported sample frames.

The existing synthetic `hevc-opus-quad-companion.mkv` and
`quad-rematrix-float32.wav` / `quad-rematrix-float64.wav` controls reproduce
inconsistent quad-to-mono arithmetic. Their unchanged acceptance tests require
mono to equal the average of the stereo speaker fold, in source precision.
The streaming f32/f64 gain adapters now preserve that evaluation order; both
previous exact-byte failures pass without weakening the oracle.

`opus-clock/short-third.mkv` is an authored four-frame VP9/Opus companion with
120-sample (2.5 ms) CELT DTX packets and TrackTimestampScale 1/3. The regression
first checks successful short-packet decoding, then requires timestamps at
0/2.5/5/7.5 ms and a 10 ms endpoint. Previously packet two appeared at
2,333,333 ns. Retained block grids now drive the player's exact coded sample
clock and indexed duration. Library full and 3–7 ms PCM interval acceptance
compares direct packet controls. Ordinary tests never run the generator.

`opus-clock/short-shift-third.mkv` and `short-lace-third.mkv` compose factor
1/3 with shifted cluster origins and, for the latter, two-packet Xiph lacing.
Every companion VP9 frame is decoded. Four independent short Opus packet
controls supply the source PCM; seeks at 3 ms, 3 ms plus 1 ns and 7.5 ms plus
1 ns must retain exactly the requested suffix with exact sample timestamps.
The 10 ms endpoint remains stable. Library full and 3–7 ms interval PCM cover
the same inputs. These extend acceptance coverage without codec substitutes,
private media or ordinary-test fixture generation.

`opus-clock/short-delay-tail.mkv` and `short-delay-head-tail.mkv` compose
17-sample Opus preskip/CodecDelay rounded to 354167 ns, 13-sample tail padding,
and an optional 9-sample interior negative DiscardPadding. Both are authored
v4 videos with unit track scale and four 120-sample CELT packets. The player
regression checks actual video, direct source PCM controls, exact seek at 1 ms
and 1 ms plus 1 ns, and the 9.375 ms audible endpoint. Before the fix the
endpoint was 9.374999 ms. The library checks full and strict 1 ms+1 ns–4 ms
interval PCM, inserting silence for the interior trimmed source window.
Ordinary tests read saved fixtures; generation is explicit and offline.

`alac-timeline/` contains two authored raw-ALAC/neutral-VP9 videos at 48 kHz,
with a 50 ms audio origin. `offset.mkv` has four 128-sample ramp packets and
reproduces lost export leading silence and quantized player PTS. `offset-gap.mkv`
has four 96-sample packets with a real 48-sample gap; its post-gap epoch lies on
an exactly representable container tick. This separates continuity/gap recovery
from ambiguous sub-sample discontinuity epochs. Every video is decoded; exact
source-ramp PCM and seeks at 51 ms/51 ms+1 ns/56 ms+1 ns are accepted. Library
full and 49–56 ms intervals preserve leading/interior silence. Own packet/cookie
and neutral VP9 writers only; ordinary tests do not run generation or FFmpeg.
Existing ALAC timeline oracles now keep an initial audible-offset sample and
interior head-trim silence, instead of concatenating later audio earlier.

ALAC duration acceptance now checks the exact absolute audible endpoint after
EOF and its stability across rewind and seek. The 50 ms origin case reproduced
10,667,000 ns instead of 60,666,666 ns. Two additional authored v4 companions,
`offset-delay-tail.mkv` and `offset-discard-last.mkv`, add 17-sample CodecDelay
plus 13-sample tail padding, or discard the entire last 128-sample packet.
Every companion video is decoded; source-ramp PCM, strict seek suffixes and
library full/interval windows remain independently specified. No private media,
external codec or fixture generation is used during ordinary test execution.
ALAC duration assertions also run immediately after open, before decoder
creation. The reader establishes coded geometry from shared element headers,
restores the playback cursor and retains the exact endpoint. This metadata pass
reads encoded packets but reconstructs no PCM. Header validation does not replace
full body/channel/stop-tag validation by the decoder. Existing mono/stereo
packet controls additionally compare header and reconstructed sample counts and
refuse truncated header prefixes.
ALAC seek acceptance counts actual decoder invocations as well as comparing
source PCM/timestamps. At 56 ms+1 ns only the selected suffix packets may decode;
beyond EOF no packet may decode. Previously all four packets were reconstructed.
The new path restores the sample clock from header counts and repositions before
the selected source packet. Encoded header I/O is distinct from PCM decoding.

`alac-multichannel/` is explicitly generated by
`scripts/generate_alac_multichannel_fixtures.py`. It saves 56 authored two-frame
VP9/ALAC Matroska videos and paired M4A controls for 2–8 channels, four bit depths
(16/20/24/32), and discrete/named cookies. Every source channel has its own signed
integer ramp. Apple `chan` tags supply independent expected output orders and
speaker masks; without `chan`, multichannel order stays discrete with mask zero.
The 6.1 back-centre mask is 0x13f; 7.1 B is front-wide mask 0xff.

Three invalid-layout movies reproduce count/tag/flags guards and are refusal
controls, not playback acceptance. Saved `stereo32.cookie`/`stereo32.frame`
reproduce the old raw CPE residual-width error (33 bits) without first hitting
the multichannel guard. Ordinary tests decode source PCM/video, strict Matroska
seek suffixes, full/interval library PCM and public MP4/Matroska WAVE masks.
A discrete-preview acceptance first reproduced complete silence; after the fix
it checks the explicit average-to-both-sinks policy, without assigning source
speaker roles. No private input, codec executable, network or test-time fixture
generation is used. Compressed multi-element qualification is expanded below; additional profiles remain separate work.

The ALAC multichannel corpus now has 336 valid variants (56 per coding mode):
`raw`, `rice`, `first-order`, `zero-run`, `adaptive-1`, `zero-run-resume`.
Compressed packets exercise ordinary Rice and escapes, nonzero CPE unmixing,
and explicit low bytes at 24/32 bits. Long modes use 128 samples per packet;
the others use 16. Predictor headers/raw-escape flags are asserted in tests.
Literal per-channel targets remain independent of residual coding. An adaptive
order-1/quantization-1 zero coefficient is forced through four updates by a
closed-form residual prefix. A common step at sample 80 makes zero-run coding
resume into nonzero residuals, including its modifier. Saved `rice` counters
prove the generator selected these syntax paths; acceptance additionally decodes
actual PCM against the independent targets in both containers and public WAVE.
All earlier authored media remains byte-identical. Higher-order adaptive and
other remaining ALAC tools are not implied by this acceptance corpus.
Manual independent validation used Apple's public ALAC decoder outside the
repository/test runtime: all 672 exact saved-packet bodies matched authored
integer PCM in source order. The reference harness/source stayed in `/tmp` and
is not invoked by tests or FVid. Packed 20-bit output was compared left-aligned
in its 24-bit storage, as specified by that reference. Container/layout checks
remain the separate native acceptance tests above.

MP4 intra-packet audio seek regression (`pcm-seek-inside.mp4`,
`alac-seek-inside.mp4`): authored 60 ms neutral VP9 video with 8 kHz mono
integer PCM or raw ALAC and a 1 kHz media clock, generated by
`scripts/generate_mp4_audio_clock_regressions.py`. Seeking to 1 ms previously
exposed packet PCM at 0 ms. The acceptance test in `mp4_audio_clocks.rs`
checks exact PCM, first-sample timestamps, repeated seeks and rewind across
five media timescales. Fixture generation is separate; tests need no FFmpeg.

ALAC ancillary acceptance extends `alac-multichannel` to 672 variants:
short and extended FIL (tag 6) and aligned/unaligned DSE (tag 4) appear before every
channel element, across 2–8 channels, four bit depths and named/discrete layouts.
The native header scan and reconstruction both skip bounded ancillary payloads.
Six `invalid-ancillary-*.mkv` videos truncate the first ancillary element;
tests require the specific packet-bounds error, not an unsupported-codec refusal.
An isolated manual Apple reference run matched all 1,344 authored packet integer
PCM outputs, including the 672 new ancillary packets. Apple code is not part of
production, fixture generation, or ordinary test execution. Higher-order
predictors and larger frame geometries still require broader qualification.

ALAC predictor qualification extends the corpus to 1,008 paired MKV/M4A
variants. Authored linear ramps use asymmetric chronological endpoint
coefficients `(order+2, 1)` at quantization 1 for orders 2, 4, 8, 16 and 30;
other coefficients are zero. The weighted ramp offsets give the next sample
exactly, so the residual oracle is a literal startup difference followed by
zeros, not a copy of the decoder's adaptive loop. Mode 31 has 16 samples and
literal first differences; it previously failed the generic filter-window
length guard even though it denotes first-order integration. These are
acceptance tests, including exact PCM, speaker layouts, interval export and
seek. Manual isolated Apple-reference verification matched all 2,016 packets.
This qualifies the tested fixed filters and short mode-31 packets, not arbitrary
multi-coefficient adaptation or all sample widths/frame lengths.

ALAC adaptive multi-coefficient qualification adds 280 variants (1,288 total).
Orders 2/4/8/16/30 start from zero coefficients, encode authored linear PCM with
a +23 integer step at sample 80, and exercise positive/negative residuals and
updates to at least two distinct coefficient positions. A separate encoder
inverts prediction using target PCM; it never derives expected PCM by decoding.
The tests check actual coded order, exact full and sought PCM, and manifest
branch counters. An isolated Apple decoder independently matched all 2,576
packet integer PCM outputs. Native playback and interval/WAVE exports run
without Apple code or FFmpeg. This qualifies these adaptation/step cases;
arbitrary coefficient extremes, quantization shifts and overflow boundaries
remain outside their scope.

`alac-predictor-overflow/rounding.mkv` is an authored two-frame neutral VP9
video with 24-bit stereo ALAC; `rounding.m4a` holds the same two audio packets.
The first predictor product is 2,147,483,646 (coefficient 32,767 times delta
65,538). With quantization 15, adding the rounding bias crosses signed32;
rounding must wrap before the arithmetic right shift. A widened accumulator
previously reconstructed the third sample as 196,612 instead of the authored
65,540 and corrupted subsequent predictions. The standalone generator uses
literal target PCM and an inverse encoder; an isolated Apple decoder confirmed
its exact integer PCM. `alac_predictor_overflow.rs` tests exact reconstruction,
actual companion-video decoding, seek at a sample boundary, MP4 playback and
public MKV/MP4-to-WAVE export. Ordinary tests are offline and use neither Apple
code nor FFmpeg.

ALAC zero-quantization acceptance adds 224 paired MKV/M4A variants (1,512 total):
coefficient-free modes 0 (copy) and 31 (first-order integration) use quantization
zero, across 2–8 channels, four bit depths, and named/discrete layouts, with or without preliminary integration. Neither
mode uses coefficient scaling, so the previous unconditional zero-quantization
refusal rejected these valid packets. Tests check the actual coded quantization
and predictor mode, exact full/sought PCM, and native interval/WAVE exports.
An isolated Apple reference matched all 3,024 packet integer PCM outputs.
Ordinary tests and fixture generation remain independent of FFmpeg/Apple code.
This specifically qualifies coefficient-free zero quantization; ordinary
coefficient filters are covered in the later q0 qualification.

ALAC clock consistency regression adds `invalid-rate-zero.mkv` and
`invalid-rate-mismatch.mkv` to `alac-multichannel`. Both retain the authored
neutral VP9 frames and valid three-channel ALAC packet bodies from the matching
48 kHz acceptance control; only the cookie sample-rate word changes (0 or
44,100). The player previously accepted these contradictory clocks while the
library Matroska wrapper refused them. Tests require the specific rate-disagreement
error after decoding the real companion video. Shared decoder construction,
packet sample-count scanning, layout lookup and memory admission now reject the
same mismatch. Matching 44.1/48/96 kHz decoder setups are acceptance controls.
This is malformed-metadata refusal coverage, not support for a new ALAC profile.

`alac-long-frames` adds 17 authored MKV/M4A pairs: raw and Rice ALAC at
4,097/8,192/16,384/65,536 samples per packet (explicit and implicit counts), plus a maximum-u32 cookie whose
packets explicitly contain only 16 samples. Neutral VP9 supplies real video
frames. The decoder previously refused all cookies above 4,096 and allocated
all scratch planes to the advertised maximum at construction. It now allocates
fallibly to actual packet sample counts, with no artificial 4,096 codec limit.
Tests compare literal periodic PCM, actual video decode, seek within and across
packets, and public MKV/MP4-to-WAVE exports. The truncated raw control declares
u32::MAX samples but has no sample body and must refuse before allocating.
Manual Apple reference validation matched 34 packet PCM outputs; the huge-cookie
case used a bounded frameLength=16 for reference validation only, with unchanged
packet bytes, so that check establishes PCM and not the reference's allocation
policy. FVid tests the original maximum-u32 cookie. Controlled export memory
admission remains conservative based on the advertised maximum; it is not a
measurement of allocation for a particular short packet. Generators and tests
use no FFmpeg, private media or network.

`alac-cookie-wrappers` contains 560 authored MKV/M4A pairs using legacy opaque
cookie wrappers from Apple's ALACMagicCookieDescription compatibility section:
optional `frma`, an `alac` configuration atom (fixed core size or full layout
size), and an optional 8-byte zero-ID terminator. Raw/Rice packets cover
2–8 channels, 16/20/24/32-bit depths and named/discrete layouts. These reuse
only the repository's authored synthetic sample patterns, never private codec
parameters. The old decoder read atom markers as cookie fields (for example,
reporting a 108-bit sample depth). Shared bounded normalization now extracts
the unchanged core fields/layout; container wrappers no longer apply raw-cookie
sample-rate offsets before decoder validation. Tests check actual video, exact
PCM, channel masks, seek, WAVE/native PCM export, and every prefix through decoder,
header-count, layout and admission entry points. The original nine malformed videos require
specific size/flags/terminator/rate/channel errors. An isolated manual Apple
reference decoded all 1,120 original wrapped-cookie packets to exact authored
integer PCM; it is not part of ordinary tests or fixture generation. No FFmpeg
or network is needed for generators or tests.

`alac-rice-boundaries` contains nine authored mono ALAC MKV/M4A pairs with
neutral VP9 companion frames and literal integer PCM targets. They cover:
zero initial history; zero effective history multiplier via the element's
pbFactor=0; small Rice limits 1/2/4; terminal/resumed runs of exactly 65,535;
multiple long runs reentering zero history with standard cookie parameters;
and coded magnitude 65,535 plus a post-run modifier before history clamping.
Seven of the first eight cases failed before the fixes, while the terminal-run
control passed. The clamp case still failed after the other three rule fixes,
confirming that it exercises a separate error. Native playback/seek and both
public WAVE/native PCM exports now match every literal sample. An isolated
manual Apple decoder matched all 18 original packet integer PCM outputs.
The generator is independent of decoding and uses no FFmpeg, foreign codec,
network or private media. Ordinary tests read checked-in fixtures only.

The corrected default multichannel fixture encoder uses the same history-zero,
65,535 modifier-reset and pre-modifier clamp semantics. Regeneration was checked
against all 3,037 existing primary corpus file hashes; none changed.

`alac-stereo-weights` contains 198 authored MKV/M4A pairs testing signed matrix
weights -128/-127/-64/-1/0/1/64/127 and shifts 0/1/7/31, plus zero-weight shift
255 which is ignored in the separated-channel path. Literal stereo values
alternate sign; 16/20/24/32-bit cases cover 8- and 16-bit low-bit paths where
supported by the 24/32-bit reference routines. The old decoder read -128 as
+128 and corrupted the first sample. It now sign-extends the coefficient while
retaining 32-bit wrapping multiply/arithmetic shift semantics. Tests inspect
actual coded fields and compare exact PCM, real companion video decoding,
seek, WAVE and native PCM exports. An isolated Apple decoder matched all 396
original packet integer PCM outputs. The oversized nonzero negative-weight
shift control must refuse the specific shift-range error; it is not an acceptance
claim for undefined shifts. No private parameters, FFmpeg, external codec or
network is used in generation or ordinary tests.

ALAC compatible-version validation adds four `invalid-version-*.mkv` videos to
`alac-cookie-wrappers`: versions 1 and 255, each in canonical and legacy-wrapped
cookies. They preserve valid authored raw ALAC packets and real neutral VP9
frames; only the compatible-version field changes. Apple's format requires 0.
These files previously opened successfully, and now refuse with the specific
`unsupported ALAC compatible version` error. Direct tests cover construction,
header count, channel layout and memory admission in both frontend and library.
Existing version-zero variants remain acceptance controls. These are refusal
regressions for unknown metadata versions, not new codec/profile acceptance.

`alac-header-controls` has four authored mono/stereo raw/Rice MKV/M4A controls
and twelve malformed MKV companions. Each mutation changes only the first or
last of the twelve reserved header bits, or the reserved shifted-byte code 3;
packet bodies, cookies and neutral VP9 frames remain the authored controls.
The old sample-count scan accepted these headers (returning 16 samples).
Shared header parsing now refuses the specific reserved-field error before
body reconstruction. Tests require the same refusal in frontend/library header
scan and decode, and player opening; unchanged controls retain exact PCM and
seek in both containers. An isolated manual Apple decoder accepted all eight
control packets with exact integer PCM and refused all twelve mutations with
nonzero decode status. These are malformed-field refusal tests, not acceptance
of an additional codec tool. Generation and ordinary tests require no FFmpeg,
foreign decoder, network or private source media.

Ordinary ALAC q0 acceptance adds 1,344 paired MKV/M4A variants (2,856 total):
fixed and adaptive orders 1/2/4/8/16/30, each with or without the preliminary
integration pass, across 2–8 channels, all four source bit depths and named or
discrete layouts. Unity coefficient scaling requires no rounding bias; the old
decoder unconditionally refused q0 for ordinary filters. Fixed filters use
closed-form integer ramp residuals, adaptive encoders invert authored ramp/step
PCM, and expected PCM remains literal input independent of decoding. Tests
inspect actual coded quantization/order/type and verify exact full/sought PCM,
interval decoding and public WAVE export. An isolated unmodified Apple reference
matched all 5,712 packet integer PCM outputs. These cases have residual widths
below 32 (32-bit source uses low-byte separation); full-width 32-bit q0 remains
outside independent qualification. The portable owned decoder avoids the
reference C expression's negative shift for q0 by explicitly selecting bias 0.
Seek checks additionally cover the exact packet boundary and one nanosecond
later, asserting the first audible sample's timestamp and exact PCM. Targeted
native file exports cover fixed/adaptive q0 in both containers, including a
1–2 ms window after warmup that crosses the authored adaptive step.
All 3,037 pre-existing primary corpus file hashes were preserved; ordinary tests
and generation use no FFmpeg, external decoder, network or private media.

`alac-integer-output/` is generated offline by
`scripts/generate_alac_integer_output.py`: authored mono raw and Rice/order-zero ALAC at 32 bits,
paired MP4/Matroska with neutral synthetic VP9 in Matroska. Literal signed PCM
includes adjacent high-magnitude values and both i32 extrema. The integer API
regression verifies low bits that f32 output cannot represent. No private media,
FFmpeg, network or external encoder is used. The predictor pair additionally covers order1 with positive q9 and coefficient1024 on positive/negative 32-bit ramps. Higher orders and adaptive full-width cases need separate qualification.

`alac-full-width-adaptive/` is generated offline by
`scripts/generate_alac_full_width_adaptive.py`. Twelve mono 32-bit cases cover
orders 1/2/4/8/16/30, positive and negative literal ramps with a jump at sample 80,
positive q1 and zero initial coefficients, without low-byte splitting. Each
contains two packets of 128 samples, paired MP4/Matroska, literal i32 PCM and
packet controls. Matroska also contains neutral authored VP9. The isolated
unmodified Apple reference matches all 24 packets exactly. Ordinary tests use
only committed authored data and own decoders, without that reference/FFmpeg.

`alac-stereo-integer/` is generated by
`scripts/generate_alac_stereo_integer.py`: 32 paired MP4/Matroska cases at 32 bits,
with independently varying low 8/16 bits, large alternating positive/negative high
planes, signed weights -128/-127/-64/-1/0/1/64/127 and shifts 7/31. Literal integer
PCM tests verify that low-byte reconstruction survives the weighted stereo
matrix. Neutral VP9 accompanies Matroska. An isolated unmodified Apple decoder
matches all 64 packets exactly. These cases use 25/17-bit residual planes, not an
unqualified 33-bit stereo residual path. Tests and generator need no external
codec, private media, network or FFmpeg.

`aac-indexed-pce.json` and `aac-indexed-pce-*-synthetic.mp4` are generated by
`scripts/generate_aac_indexed_pce.py` from the existing authored Main/LC/SSR
PCE packets, scalar PCM and synthetic AVC. Thirty-six mono/stereo acceptance videos
use matching indexed ASC. Main coupling at points 0/1/3 is included. Main/LC/SSR
variants use tags 3/15 and either an initial confirming PCE or first adoption
after earlier packets have built predictor/overlap history. Packet bytes are
kept in `aac-indexed-pce-packets.bin`; literal PCM is unchanged. They reproduce the former exact refusal `in-band PCE
needs an explicit configured program`, rather than a container/codec failure.
Twenty-four profile/rate mismatch videos retain the refusal requirement. Tests
compare independent authored PCM through MP4 and direct decoding, including a
truncated PCE before every packet to check rollback. No private media, external
encoder, FFmpeg or network is required by generation or ordinary tests.

`aac-ps-indexed-pce.json` and its paired packet blob/synthetic MP4s are generated
by `scripts/generate_aac_ps_indexed_pce.py`. Twelve acceptance videos cover
LC/SBR/PS signalling, 960/1024-sample cores, 24/48 kHz output, indexed mono ASC
and matching front SCE tag3 PCE. Authored stereo scalar references are reused
unchanged from `aac-ps-absence-pcm.bin`. The former exact failure is `in-band PS
PCE needs an explicit configured program`, in both PS probe and native decoder.
Twenty-four profile/rate mutation videos must refuse without publishing PCM or
committing state. Tests exercise direct decoding, checkpoints, truncated-PCE
rollback, container audio and synthetic video. Generation and ordinary tests
remain offline and require no external encoder, FFmpeg or private media.

`hevc-3d-pps-*-synthetic.mp4` is generated offline by
`scripts/generate_hevc_inactive_3d_pps.py`. Existing authored Main I/P/B pictures
and timestamps are unchanged; only PPS extension syntax in hvcC or an in-band
PPS update is replaced. Four acceptance videos set the PPS 3D flag with
`dlts_present_flag=0`, alone or before a future-extension tail. They reproduce the exact old PPS refusal,
verified directly before fixing playback. Pixel planes/timestamps and rewind
must match the 17-frame synthetic control. Active-DLT and missing-flag controls
are refusal tests, not acceptance of active 3D tools. Both hvcC and in-band
refusals preserve the precise PPS cause through container playback and rewind,
rather than masking it as a missing SPS. No private parameters,
media, external codec, FFmpeg or network are used by generator/tests.

`hevc-depth-tables/` is generated offline by
`scripts/generate_hevc_depth_tables.py`. Authored field payloads and literal
expected tables cover flags, delta coding, symmetric-difference prediction,
empty/singleton/duplicate lists and 8/10/12/16-bit values. Invalid payloads cover
missing prediction references, mismatched SPS/VPS context and difference/value
bounds. Component tests bound syntax before RBSP trailing bits, check every
short-field truncation, non-byte-aligned input, exact lookup and simultaneous
scratch/retained allocation admission. This is parser acceptance only. The
existing short `hevc-3d-pps-active-synthetic.mp4` remains a playback refusal
regression until 3D VPS/SPS/depth reconstruction is integrated. No complete
active-3D playback acceptance is claimed. No private media, FFmpeg, external
encoder or network is used during generation/tests.

`hevc-depth-pps-binding/` is generated offline by
`scripts/generate_hevc_depth_pps_binding.py`. Authored VPS depth scalability,
SPS identities and 14 DLT PPS variants exercise depth/texture role binding,
future tails, SPS-depth mismatch and VPS-layer-count mismatch. Metadata
acceptance is distinct from full 3D profile/picture qualification.
`texture-playback.mp4` contains unchanged 17-frame synthetic base texture
pictures and table metadata for another layer, with distinct SPS/PPS IDs.
OLS0 native playback matches control planes/POCs through reset. Depth-layer
reconstruction still has a specific refusal, including ordinary-picture API
misuse of a depth PPS. The generator/tests require no FFmpeg, private source,
external encoder or network. Reserved NAL layer ID63 is rejected in DLT context.

`hevc-depth-reconstruction/` is generated offline by
`scripts/generate_hevc_depth_reconstruction.py`. It reuses authored DLT payloads
and stores exhaustive scalar nearest-index and zero-padded lookup references,
DC predictions and nonuniform 4x4 block assignments at 8/10/12/16 bits. Nearest
search in the independent generator is linear; native code uses binary search.
Midpoint ties select the upper entry; sparse-table indices beyond NumValDlt
map to zero within the sample domain. A full 8-bit flag table checks the dense
boundary. Non-DLT prediction stays signed/unclipped; final DC block assignment
clips samples. Tests cover strided regions, untouched padding, extreme offsets,
empty tables and failure before mutation. These are reconstruction primitive
acceptance tests, not full 3D coded-picture acceptance. The short active-DLT
video remains a production refusal until entropy/depth-picture integration.
No private media, external encoder, FFmpeg or network is needed.

`hevc-3d-sps/` is generated offline by `scripts/generate_hevc_3d_sps.py`.
Eighteen complete SPS3D field payloads cover each of the14 feature flags,
sub-block bounds and cursor rollback. Fourteen short Main/Main10 synthetic
videos change only SPS syntax: eight acceptance videos preserve all17 I/P/B
frames/clocks through rewind, while active texture tools, truncated fields and
invalid sub-block sizes retain precise refusals. Depth-only flags are retained
but do not activate tools on a texture layer; a VPS depth-context regression
proves those same tools still refuse on the depth layer. A combined
range/multilayer/3D/SCC metadata case verifies extension order and palette values.
The older `hevc-multilayer-sps-*-three-d-synthetic.mp4` fixtures contain no 3D
body, so they now classify as truncated/invalid rather than valid unsupported
3D-tool examples. Active 3D coded-picture reconstruction is not qualified.
No private media/parameter sets, external encoder, FFmpeg or network is required.

`scripts/generate_hevc_3d_sps_updates.py` adds12 short Main/Main10 in-band SPS
update videos under `hevc-3d-sps/`. Unchanged17-picture control payloads receive
inactive, unused depth-only or alternating SPS3D fields before inter pictures.
Eight acceptance videos preserve picture planes/clocks and DPB/POC continuity
through rewind; explicit update-allowed SEI is included. Four refusal videos
exercise current-role active texture tools and no-update SEI with a known SPS
metadata change. The old specific failure was `HEVC SPS change requires a
random-access picture`. Metadata-only base reconstruction equivalence is
separate from parameter identity/guidance and full 3D reconstruction support.
Generation and tests remain offline with no FFmpeg or private media.

### Native HEVC DBBP

`hevc-wedge/dbbp-p-*.mp4` and `dbbp.json` are authored offline by
`scripts/generate_hevc_dbbp.py`: two explicit half partitions per 32x32 CU,
current-AU depth and distinct texture views, CU flag on/off, 32/128 pictures
and direct/worker ownership checks. `scripts/generate_hevc_dbbp_fractional.py`
authors sixteen `dbbp-fractional-*.mp4` movies and their `.json`/`.y16` scalar
convolution oracle. `tests/hevc_dbbp.rs` checks decoded luma/chroma, fractional
saved motion and rewind. These are coded playback acceptance tests, replacing
the previous DBBP SPS-tool refusal expectation. Ordinary tests read committed
fixtures and never run the generators or require FFmpeg/network access.

The same fractional generator also writes eight `dbbp-merge-*.mp4` movies,
`dbbp-merge.json` and `dbbp-merge.y16`. A fractional explicit first PU and a
zero-motion merged second PU exercise both own-past and current-source
references, both partition orientations and DBBP flag values. The motion
acceptance helper checks references, vectors, every output sample and rewind.

`scripts/generate_hevc_dbbp_iv.py` authors ten `dbbp-iv-*.mp4` movies and the
`dbbp-iv.json`/`.y16` oracle: source AMP vectors vary between vertical strips,
so ordinary IV subblocks differ from DBBP's full-CU default predictor. Two
additional shifted-lookup cases select IVShift. Pixel/reference/motion/rewind
acceptance is part of `tests/hevc_dbbp.rs`, with offline scalar convolution.

The fractional DBBP author also writes sixteen `dbbp-weighted-*.mp4` movies,
`dbbp-weighted.json` and `.y16`: explicit P weights for both references and
all three components, fractional motion, both partition orientations and CU
flag values. The scalar oracle applies weights before rounding/clipping the
predictor and before depth-contour selection; its manifest counts witnesses
against weighting already rounded/clipped pixels. Playback acceptance and
rewind are included in the ordinary offline DBBP tests.

`scripts/generate_hevc_dbbp_residual.py` authors thirty-two
`dbbp-residual-*.mp4` movies and their `.json`/`.y16` scalar oracle. Actual
signed transform DC coefficients exercise all-component and chroma-only
residuals, weighting, both partition orientations and CU flags. The oracle
adds the inverse-DCT residual after DBBP boundary filtering and records
witnesses against premature predictor clipping. The ordinary DBBP tests
validate every pixel and rewind without invoking generators or FFmpeg.

The DBBP residual author also writes `dbbp-ac-vertical-*` and
`dbbp-ac-horizontal-*` (32 movies per axis) and matching `.json`/`.y16`
oracles. These code a signed first-frequency AC coefficient, omit DC and use
an independent normative integer-basis inverse DCT for the expected spatial
residual. Weighted/unweighted, chroma-only/all-component, partition and CU
flag combinations are covered by the same offline playback/rewind checks.

`scripts/generate_hevc_dbbp_cu64.py` writes twelve `dbbp-cu64-*.mp4`
fixtures and `dbbp-cu64.json`: real 64x64 target CUs with PCM32 source
leaves in CTU64, both partition orientations/flags and direct/worker paths.
The DBBP scalar pixel oracle covers 64x64 depth contours; unit ownership
checks distinguish four CU commands from eight geometric PU commands in
worker-enabled 128x128 pictures.

The CU-size author additionally emits twelve `dbbp-cu16-*.mp4` movies and
`dbbp-cu16.json`. Correct CTU32 splitting yields four CU16 leaves, testing
the minimum DBBP size and 8x8 chroma composition. Shared pixel/motion/rewind
acceptance covers CU16, CU32 and CU64; unit command-count checks distinguish
64 complete CU16 predictions from 128 geometric PU predictions in worker-
enabled 128x128 pictures.

`scripts/generate_hevc_dbbp_b.py` authors 34 accepted `dbbp-b-*.mp4` movies
and two deliberately nonconforming explicit-BI/DBBP movies, plus
`dbbp-b.json`/`.y16`. Accepted cases check L1 removal for bi-merge at DBBP
flag one, preservation of both lists at flag zero, luma weights, and direct/
worker-eligible 128x128 pictures. Tests check pixels, saved lists/vectors and
rewind. Explicit-BI DBBP refusal has its own test and exact error expectation;
matching DBBP-off controls are playback acceptance cases. Refusal results
must not be reported as accepted playback of invalid streams.

`scripts/generate_hevc_dbbp_b_iv.py` writes 76 accepted
`dbbp-b-iv-*.mp4` movies and `dbbp-b-iv.json`/`.y16`: B AMP source motion,
IV/IVShift selection, signed fractional L0/L1 vectors, weights and CU32
DBBP default prediction versus normal IV subblocks. Sixteen multi-CU 128x128
cases exercise direct/worker reconstruction. The author cancels PU0 AMVP with
explicit MVD derived from its planned ready-neighbour map; tests assert that
motion before comparing independent full-precision pixel oracles and rewind.
Unit checks verify both source and target worker command counts. Ordinary
tests only read fixtures; generation remains offline and separate.

The B IV author also writes 76 `dbbp-b-iv-l1-*` accepted movies and
`dbbp-b-iv-l1.json`/`.y16`. Source PUs use only L1 and target L0 omits their
POC, exercising true L1-only IV/IVShift projection and retention under DBBP.
The scalar oracle applies L1 weights. Sixteen multi-CU 128x128 cases code both
predictors through L1 and check direct/worker ownership, exact pixels, saved
list/vector state and rewind. Existing normal B IV data is preserved.

`scripts/generate_hevc_dbbp_disparity.py` writes 160 accepted
`dbbp-disparity-*.mp4` movies and `.json`/`.y16`: depth-reference camera
refinement, signed offsets/precision, CU16/32/64, padded shifted depth contours
and direct/worker paths. `scripts/generate_hevc_dbbp_base_disparity.py` writes
72 accepted `dbbp-base-*.mp4` movies and `.json`/`.y16`: cross-view PU motion
seeds nonzero spatial DispVec in both axes, with fractional prediction and
CTU-row propagation. The MVD author respects ST/LT AMVP incompatibility.
Independent scalar oracles check every sample; saved explicit motion and
rewind plus native command-count checks guard the intended reconstruction
paths. Generation stays separate from ordinary offline tests.

The base-disparity author additionally writes 216 `dbbp-base-refined-*`
accepted movies and their `.json`/`.y16` oracle. Nonzero spatial base motion
shifts depth-corner sampling before signed camera refinement; DBBP then uses
the replacement horizontal vector rather than retaining base vertical motion.
Independent scalar alternatives count witnesses for both ordering errors.
Pixel/reference/motion/rewind acceptance and native direct/worker command
checks cover the joint P CU32 path. Earlier unrefined base fixtures are
byte-preserved; generators remain separate from offline test execution.

`avc-pcm-formats/` contains 18 synthetic two-frame progressive CAVLC I_PCM
streams authored by `scripts/generate_avc_pcm_formats.py`, plus independent
patterned sample expectations. Monochrome, 4:2:2 and 4:4:4, mixed component
precision and cropped borders reproduce the old 4:2:0-only reconstruction
refusal. The cropped monochrome cases additionally reproduce presentation's
incorrect chroma-alignment refusal. `tests/avc_pcm_formats.rs` accepts native
decode and MP4 playback, checking exact samples, memory boundaries and rewind.
This matrix does not qualify non-PCM, CABAC, inter or field coding. Tests do
not invoke the generator or require FFmpeg/network access.

`avc-monochrome-intra/` adds six owned 8/10/14-bit monochrome streams mixing
patterned PCM and compressed I16-horizontal / I4-DC macroblocks with zero
residual and disabled deblocking. The scalar oracle includes reconstructed
top/left edges for each I4 block. These reproduce the former non-PCM refusal;
`tests/avc_monochrome_intra.rs` accepts exact sample decode, MP4 packing and
reset/rewind. Generate separately with `scripts/generate_avc_monochrome_intra.py`.

`avc-monochrome-cabac/` has 15 two-slice 8/10/14-bit movies authored offline by
`scripts/generate_avc_monochrome_cabac.py`: monochrome CABAC I16 zero/signed DC,
I4 DC with CBP0, and PCM. Each macroblock is its own slice; signed DC is reversed
between the two blocks. Scalar DC arithmetic and patterned PCM provide exact
sample expectations. `tests/avc_monochrome_cabac.rs` checks native decoding,
component storage/precision, MP4 presentation, reset and rewind. This reproduced
the old CAVLC-only admission refusal; it does not qualify intra-slice mixed
contexts, deblocking or inter prediction. Ordinary tests do not generate media.

`avc-monochrome-deblock/` adds 18 owned two-slice I16 signed-level4 movies:
8/10/14-bit CAVLC/CABAC and filter modes0/1/2. Mode0 changes the vertical
macroblock boundary according to an independent scalar short bS4 oracle;
mode1 disables filtering and mode2 excludes this cross-slice boundary.
`scripts/generate_avc_monochrome_deblock.py` generates them separately from
`tests/avc_monochrome_deblock.rs`, which checks exact samples, MP4 output and
reset/rewind. Modes0/2 reproduced the former deblocking-disabled admission
refusal; mode1 was already admitted. No private media or external codec is used.

`avc-monochrome-inter/` contains twelve two-frame 8/10/14-bit movies with
patterned PCM references and CAVLC P-skip / signed integer motion, plus CABAC
P-skip. The offline author `scripts/generate_avc_monochrome_inter.py` computes
expected samples by independent source indexing with clamped border extension.
`tests/avc_monochrome_inter.rs` checks coded planes and MP4 packed output,
precision, reset and rewind. The CAVLC witnesses reproduced the original
inter-picture tools refusal after an accepted IDR. These do not establish
fractional motion, B prediction, weights, inter residual/filter or field support.

`avc-monochrome-fractional/` qualifies 384 owned CAVLC P16x16 movies: all
quarter-luma vectors -4..3 on each axis at 8/10/14 bits, with and without
explicit luma weights. `scripts/generate_avc_monochrome_fractional.py` uses an
independent scalar six-tap/quarter-average/weight oracle with border extension.
Its coverage records 91,611 fractional and 96,745 weighted sample witnesses.
`tests/avc_monochrome_fractional.rs` validates matrix uniqueness/completeness,
exact decode and MP4 samples, geometry/precision, reset and rewind. It covers
one-reference P16x16 without residual/deblocking; it does not qualify B/direct,
other partitions or CABAC coded motion. Generation is separate and offline.

`avc-422-intra/` adds 144 own profile244-signalled 4:2:2 CAVLC intra movies:
three patterned PCM neighbours and an I4/I16 macroblock with four chroma
prediction modes, signed/zero first-position DC, optional signed first-AC in
all eight blocks, and depth pairs 8/8, 10/12, 14/8. The offline author
`scripts/generate_avc_422_intra.py` computes a scalar 8x16 prediction and
rectangular-DC/first-AC oracle. The initial 72 movies reproduced the non-PCM
refusal. `tests/avc_422_intra.rs` checks exact coded and MP4 packed samples,
format/depth and reset/rewind. No CABAC/deblocking/inter/field or general
coefficient/profile coverage is implied. Tests do not run the generator.

`avc-422-cabac/` contains 150 owned unfiltered intra movies under profile244:
144 CABAC counterparts of the independently calculated `avc-422-intra/`
matrix, plus six mixed coded/zero AC-mask cases distinguishing rectangular
neighbour contexts in both halves of each chroma plane. One slice retains
contexts across three PCM restarts before its compressed I4/I16 macroblock.
`scripts/generate_avc_422_cabac.py` authors CABAC separately, reusing the owned
scalar sample oracle and adding explicit first-AC arithmetic for the masks.
`tests/avc_422_cabac.rs` checks exact coded, planar and MP4 data, precision and
reset/rewind. The original 144 reproduced the CAVLC-only admission refusal;
their bitstreams/oracles are byte-preserved when adding masked cases. No
external codec or private media is used; ordinary tests do not generate files.

`avc-422-deblock/` has 54 owned CAVLC/CABAC I16 movies at depth pairs8/8,
10/12,14/8 and modes0/1/2: 36 vertical/horizontal cross-slice steps plus18
internal y=4/y=12 steps authored with chroma DC scan position3. The independent
scalar QP26 oracle includes rectangular macroblock traversal and intersection
effects. Mode2 skips external slice edges but keeps internal filtering.
Generate offline with `scripts/generate_avc_422_deblock.py`; ordinary acceptance
is in `tests/avc_422_deblock.rs` (exact coded/planar/MP4 data, reset/rewind).
The initial36 reproduced the admission refusal; their bytes and oracles are
preserved when adding internal-edge cases. Other QPs/offsets, PCM, inter and
field filtering are outside this matrix. No private media or external codec.

`avc-422-inter/` includes 51 positive two-frame P movies at component depths
8/8,10/12,14/8. CAVLC skip and signed fractional/axial motion have variants with
distinct luma/chroma weights; CABAC skip supplies three more cases. The offline
author `scripts/generate_avc_422_inter.py` uses independent scalar six-tap luma,
4:2:2 bilinear chroma, edge extension and weight/clipping oracles. Exact coded,
planar and MP4 data plus reset/rewind are tested in `tests/avc_422_inter.rs`.
The original27 reproduced the inter tools refusal and remain byte-preserved.
`422-p-chroma-dc-pending` originally reproduced the eight-block coefficient
refusal; it is now accepted, its acceptance test enabled, and included as the
52nd positive movie. The old refusal expectation is removed and
`unsupported.json` is empty. This does not qualify B/direct, fields or inter filters.

`avc-422-inter-residual/` adds 27 CAVLC PCM/P16x16 residual movies at component
depth pairs 8/8,10/12,14/8, with signed/zero DC and none/full/masked first-AC in
all eight chroma blocks. A mixed PCM neighbour has changed samples and high
coefficient counts, separating reference prediction from intra data and
selecting high/low CAVLC tables. The independent scalar author is
`scripts/generate_avc_422_inter_residual.py`.
`avc-422-inter-residual-cabac/` has 27 CABAC counterparts with separately authored
entropy and the same scalar sample oracle, produced by
`scripts/generate_avc_422_inter_residual_cabac.py`. They reproduced the coded
CABAC inter refusal; both acceptance suites now check exact coded/planar/MP4
samples, precision and reset/rewind. Generation remains separate and offline.
This matrix uses zero motion, default scaling, QP26 and disabled filtering;
other coefficient positions, weight/residual combinations, transforms/bypass,
B/direct, fields and inter filtering remain outside its qualification.

`avc-422-b/` contains 108 own three-picture CAVLC/CABAC B prediction movies,
generated offline by `scripts/generate_avc_422_b.py`. It covers component depths
8/8, 10/12, 14/8; uni/bi, spatial/temporal direct and skip; default, explicit and
asymmetric implicit weights. `avc-422-b-motion/` adds 144 movies generated by
`scripts/generate_avc_422_b_motion.py`, with moving colocated P references and
independent nested scalar interpolation. The acceptance suites verify exact
coded, planar and native MP4 samples, depth, reset and presentation order on
rewind. Neither generator invokes an external codec or uses private media.

`avc-422-b-residual/` adds 405 own CAVLC B movies, authored offline by
`scripts/generate_avc_422_b_residual.py` after the basic B generator. Changed
PCM/inter macroblocks, five prediction modes, three weight modes, three depth
pairs and signed/zero DC with none/full/masked AC are checked by
`tests/avc_422_b_residual.rs`. Scalar residuals are added to independently
calculated weighted samples with clipping. The three B suites additionally
assert exact presentation timestamps, durations and decoded sample indices on
both playback passes. CABAC and moving B residuals are not covered here.

`avc-422-b-residual-cabac/` contains 405 CABAC counterparts generated by
`scripts/generate_avc_422_b_residual_cabac.py` after the basic B and CAVLC B
residual authors. Arithmetic entropy is authored separately (including embedded
B I_PCM and restart); expected samples come from the independent scalar oracle
used by CAVLC. `tests/avc_422_b_residual_cabac.rs` accepts every coded, planar
and packed MP4 sample, precision, reset/rewind and exact timing. This supersedes
the preceding CABAC-residual exclusion only for this zero-motion/QP26 matrix;
moving residuals and filtering remain outside its coverage.

`avc-422-b-motion-residual/` and `avc-422-b-motion-residual-cabac/` contain 486
movies each. Generate them offline with
`scripts/generate_avc_422_b_motion_residual.py` after the basic moving B author.
They combine moving P references, temporal/spatial/explicit-bi prediction,
default/explicit/implicit weights, three depth pairs, signed/zero DC and
none/full/masked signed AC. Expected samples use the separate scalar motion
and residual arithmetic, never decoder output. Both automated acceptance suites
check exact samples and timing, reset and rewind without running generators,
FFmpeg or network access. Filtering, fields and other partition tools remain
outside this fixture matrix.

`avc-422-inter-deblock/` contains 72 own two-frame 32x32 CAVLC/CABAC movies.
Generate offline with `scripts/generate_avc_422_inter_deblock.py`. PCM IDR
samples and native P16x16 motion create a shallow chroma step across vertical
or horizontal macroblock boundaries. Different motion produces bS=1; equal
motion produces bS=0 controls. Modes 0/1/2 and 8/8,10/12,14/8 component depths
cross both conditions. The scalar border oracle checks weak filtering and
unfiltered controls. Before admission was connected, the original 24 mode-0/2
movies reproduced the inter-tools refusal; mode-1 controls decoded. The enabled
`avc_422_inter_deblock` test checks every decoded/planar/MP4 sample, precision,
PTS/duration and reset/rewind without FFmpeg or generator execution. Other
strengths, offsets, residual internal edges, B boundaries and field filtering
need separate qualification.

`avc-422-inter-deblock-single/` contains 72 single-slice P counterparts of the
cross-slice border oracle. Generate using
`scripts/generate_avc_422_inter_deblock_single.py`. Mode 2 filters the internal
slice boundary exactly like mode 0, with equal-motion and mode-1 controls.
`avc-422-b-deblock/` and `avc-422-b-deblock-single/` each contain 144 three-frame
movies, generated by `scripts/generate_avc_422_b_deblock.py` and
`scripts/generate_avc_422_b_deblock_single.py`. They independently vary L0/L1
motion, both border directions, entropy modes, depths and filter modes. Equal
pixels in distinct reference pictures isolate boundary-strength identity rules
from prediction. Separate/common B slices distinguish mode-2 suppression from
mode-2 filtering. Acceptance tests compare every sample and exact timing,
including reset and presentation reordering on rewind. This is bS=0/1 coverage;
other strengths, swapped same-picture lists, partitions and fields remain.

`avc-422-p-intra-deblock/` has 54 P embedded-intra bS=3/4 movies, authored by
`scripts/generate_avc_422_p_intra_deblock.py` using the independent scalar
intra-border/internal-edge oracle. `avc-422-p-luma-deblock/` and
`avc-422-b-luma-deblock/` have 36 movies each, authored by
`scripts/generate_avc_422_p_luma_deblock.py` and
`scripts/generate_avc_422_b_luma_deblock.py`. All luma blocks contain signed DC
coefficients so bS=2 is required despite zero motion. At QP32 its scalar weak
filter oracle differs from bS=1; generation asserts that distinction as well
as the unfiltered control. Three component-depth pairs, CAVLC/CABAC, both border
directions and filter modes 0/1/2 are crossed. Acceptance tests check exact
coded/planar/MP4 samples and timing, reset and rewind. Generation stays separate
from ordinary tests, without external codecs or private media. Other QPs,
offsets, partition/residual bases and fields remain separate qualifications.

`avc-component-fields/` contains 36 own complementary CAVLC PCM field movies,
authored offline by `scripts/generate_avc_component_fields.py`. Formats 0/2/3,
three mixed component-depth pairs, both field orders and crop/no-crop cross the
matrix. Expected full-plane samples are a scalar spatial pattern; packets read
its even/odd rows separately. All 36 reproduced the exact I-field admission
refusal before component-aware PCM decoding/weaving. The enabled
`avc_component_fields` acceptance test checks exact coded planes, native cropped
bytes, normalized packed MP4 output (including monochrome neutral chroma),
pending-field state, reset/rewind and exact field-pair PTS/duration. No external
codec, private media or generator execution is required by ordinary tests.
Compressed/filtered non-420 fields, CABAC, FMO and MBAFF remain separate work.

`avc-component-fields-cabac/` contains 108 owned CABAC PCM field movies, authored
by `scripts/generate_avc_component_fields_cabac.py` after the CAVLC component
field author. Single/multi/ASO slice layouts cross formats 0/2/3, three depth
pairs, both field orders and crop/no-crop. Expected samples are the independent
spatial/parity oracle; arithmetic types, PCM flush/restart and slice termination
are separately authored. All 108 reproduced the exact non-420 field refusal
before the fix; the enabled acceptance checks every sample and exact pair
timing, precision, crop and reset/rewind. No private codec data, FFmpeg, network
or generation runs are required by tests. Compressed/filtered non-420 fields,
FMO and MBAFF remain separate work.

`avc-component-field-intra/` contains 600 own compressed complementary-field
movies generated offline by `scripts/generate_avc_component_field_intra.py`.
576 422 cases cross both entropy modes, field orders, I4/I16, four chroma modes,
three depth pairs, signed/zero DC and signed AC on/off; 24 monochrome cases
check luma-only syntax. Every compact field mixes three PCM neighbours with a
compressed macroblock. Independent scalar prediction/DC/field-AC oracles are
woven from distinct parity samples. Generation explicitly rejects a witness
that would match the incorrect frame AC scan. All 600 reproduced the old
non-420 field refusal; their enabled acceptance test checks every coded,
native planar and packed MP4 sample, precision, timing, field state and
reset/rewind. Ordinary tests invoke neither generators nor external codecs.
Other luma bases/QPs/scaling, filtered/P/B fields, FMO and MBAFF remain separate.

`avc-422-field-deblock/` contains 108 owned I-field movies generated by
`scripts/generate_avc_422_field_deblock.py`. It crosses three component-depth
pairs, CAVLC/CABAC, both parity orders, filter modes0/1/2 and external vertical,
external horizontal and internal chroma steps. Independent scalar filtering
uses bS=3 for horizontal field edges and bS=4 vertically; generation asserts a
pixel difference from incorrect frame strength, including eight-bit cases.
Mode1 and cross-slice mode2 controls remain unfiltered; internal horizontal
4x4 edges still filter under mode2. Original mode0/2 streams reproduced the
specific refusal before connection; enabled acceptance checks all coded,
planar/MP4 samples and exact timing, precision and reset/rewind. Generation is
separate, offline and free of external codecs/private media. Other luma bases,
QPs/offsets, mixed policies, inter fields, FMO and MBAFF remain separate work.

`avc-mono-field-deblock/` contains 72 own monochrome I-field movies generated
by `scripts/generate_avc_mono_field_deblock.py`. Signed I16 DC, 8/10/14 bits,
both entropy modes, both field orders, vertical/horizontal boundaries and
filter modes0/1/2 cross the matrix. The independent scalar full luma filter
walks macroblocks/edges in order, including p1/q1 and subsequent internal-edge
updates. Generation asserts that horizontal field filtering differs from the
wrong frame strength after clipping. Original mode0/2 cases reproduced the
monochrome-filter refusal; the enabled acceptance checks every sample, neutral
display chroma, timing, precision and field state through reset/rewind. Tests
need neither an external decoder nor generation, FFmpeg, network or private
media. Other coefficients/QPs/offsets, mixed policies, inter fields, FMO and
MBAFF remain separate qualifications.

`avc-mono-field-p/` contains 96 four-field monochrome P movies, authored by
`scripts/generate_avc_mono_field_p.py`. It uses the separately generated PCM
field fixtures, scalar quarter-pixel prediction/weights and explicit matching-
parity reference-list modification. Both entropy modes, both field orders,
three precision pairs, four motion vectors and weighted/unweighted prediction
cross the matrix. Initial I pairs decode before all 96 reproduce the old P
configuration refusal. Enabled acceptance checks every coded/native/packed Y
sample, absent coded chroma, neutral display chroma, exact pair timing, source
indices and reset/rewind. Ordinary tests do not invoke generation, FFmpeg or
network. Residual/opposite-parity/filter/other-partition and B/422/MBAFF field
combinations remain separate qualifications.

`avc-mono-field-p-residual/` contains 192 four-field movies authored by
`scripts/generate_avc_mono_field_p_residual.py` after the monochrome P author.
Signed DC or field-scanned first AC in every luma4 block crosses the existing
96 depth/entropy/parity/motion/weight cases. CAVLC/CABAC entropy is independently
authored; expected residuals use scalar inverse quantization/bases over the
separate scalar prediction oracle. Generation requires the AC result to differ
from incorrect frame scan after clipping. Enabled acceptance checks every
coded/native/packed sample, neutral chroma, exact pair timing/source indices,
field state and reset/rewind, with no generator, FFmpeg or network invocation.
This qualifies QP26/flat/4x4 residuals only; other bases/masks/scaling, transform8,
bypass, filtered/opposite-parity fields, B/422/MBAFF remain separate work.

`avc-mono-field-b/` contains 216 own six-field monochrome movies generated by
`scripts/generate_avc_mono_field_b.py`. L0/L1/bi, spatial/temporal direct/skip,
three weight modes, precision pairs, both entropy modes and parity orders are
crossed; distinct references and asymmetric implicit weights provide scalar
prediction witnesses. `avc-mono-field-b-motion/` adds 432 movies generated by
`scripts/generate_avc_mono_field_b_motion.py`, using moving future P fields,
scaled temporal vectors and independently nested scalar quarter-pixel
interpolation. Both acceptance suites compare all coded/native/packed samples,
neutral display chroma, exact PTS/duration/source indices, field state and
reset/rewind with real decode/presentation reordering. No generator, FFmpeg,
network or private media is used during tests. Residuals, filtering, opposite
parity, other partitions, 422 inter and MBAFF remain separate qualifications.

`avc-422-field-p/` contains 96 own four-field P movies generated by
`scripts/generate_avc_422_field_p.py`. Component depths, both entropy modes and
parity orders, signed/fractional motion and distinct Y/Cb/Cr weights cross the
matrix; scalar luma/chroma interpolation uses compact 422 geometry. Initial I
pairs decoded before all 96 reproduced the former P field refusal.
`avc-422-field-p-residual/` adds 864 counterparts authored by
`scripts/generate_avc_422_field_p_residual.py`, crossing signed/zero rectangular
DC and absent/full/masked AC. Separate scalar field-scan inverse bases and DC
arithmetic are applied after clipped weighted prediction; generation asserts
distinction from wrong frame AC scan. Enabled suites check all coded/native/
packed samples, precision, exact pair timing/source indices, field state and
reset/rewind without FFmpeg, private data, generation or network access during
tests. B/opposite-parity fields, luma bases/transform8, filtering combinations,
FMO and MBAFF remain separate qualifications.

`avc-422-field-b/` has 216 six-field movies authored by
`scripts/generate_avc_422_field_b.py`: distinct references, both entropy/parity
orders, three component-depth pairs, six prediction modes and three weight
modes. Independent component weights and asymmetric implicit weights witness
native 422 prediction. `avc-422-field-b-motion/` adds 432 movies authored by
`scripts/generate_avc_422_field_b_motion.py`, with moving future P fields and
nested scalar luma/chroma interpolation; temporal oracles must differ from
zero motion for each component. Enabled acceptance checks every coded/native/
packed sample, precision, exact reordered pair timing/source indices, field
state and reset/rewind. Ordinary tests invoke no generators, FFmpeg, network or
private media. Residual, opposite-parity/filter/other-partition, scaling/QP and
MBAFF combinations remain separate qualifications.

`avc-mono-field-p-opposite/` and `avc-422-field-p-opposite/` contain 96 movies
each, generated by `scripts/generate_avc_opposite_field_p.py`. Explicit list
modification selects previous opposite parity; scalar oracles must differ from
matching parity and, for 422 chroma, from the incorrect 420 parity phase.
The `-opposite-residual/` counterparts contain 192 monochrome and 864 422
movies, generated by `scripts/generate_avc_opposite_field_p_residual.py` with
independently calculated DC/field-AC residuals and masked rectangular contexts.
All four acceptance suites compare every sample, neutral monochrome display
chroma, exact pair timing/source indices, field state and reset/rewind without
an external decoder or generator/network invocation. B mixed-parity mapping,
other QPs/scaling/luma bases, filtering, partitions, FMO and MBAFF remain separate.

`avc-component-mbaff-pcm/` contains 192 own two-frame monochrome/422 MBAFF PCM
movies authored by `scripts/generate_avc_component_mbaff_pcm.py`. Three precision
pairs, both entropy modes, frame/field/mixed pair modes, ASO and crop cross the
matrix. Before the fix monochrome silently published 420 zero chroma, while
422 refused entropy admission. Independent physical-coordinate samples are
compared in enabled coded/native/packed playback acceptance, including crop,
precision, exact timing, state and reset/rewind. `unsupported.json` is now empty;
`accepted.json` lists all eight formerly refused monochrome/422 compressed
first-pair streams with exact sample acceptance.
Generation stays offline and separate from tests, without external codecs or
private media. Other filtering/FMO combinations, 444 and other MBAFF tools
remain separate qualifications.

`avc-mono-mbaff-intra/` contains 288 own two-frame movies authored by
`scripts/generate_avc_mono_mbaff_intra.py`: three luma depths, CAVLC/CABAC,
frame/field/mixed pairs, ASO, crop and I16 signed DC / I4 signed DC or AC.
Independent physical availability and DC prediction plus scalar inverse bases
supply expected samples; AC orientation must differ from the wrong scan.
All cases reproduced the old compressed monochrome MBAFF refusal before the
shared reconstruction fix. Enabled acceptance compares coded/native/cropped/
packed output, neutral display chroma, precision, timing, state and reset/rewind.
Four neutral monochrome compressed-pair cases in `avc-component-mbaff-pcm/`
move to `accepted.json` and exact sample acceptance; the four 422 counterparts
now also pass through the rectangular component reconstruction path.
No external codec, private media, network or generator execution is needed by
tests. Transform8/bypass, other modes/QPs/scaling, filtering/FMO, inter and
compressed 444 MBAFF remain separate qualifications.

`avc-422-mbaff-intra/` contains 1728 own two-frame movies authored by
`scripts/generate_avc_422_mbaff_intra.py`: three luma/chroma precision pairs,
CAVLC/CABAC, frame/field/mixed pair layouts, ASO, crop, I4/I16 luma, and signed
rectangular chroma DC with absent/full/masked AC. An independent physical-sample
availability model, 8x16 chroma prediction and scalar inverse bases define the
expected samples; AC cases require a witness distinguishing frame and field scans.
All cases reproduced the specific compressed 422 MBAFF syntax refusal before
its removal. Enabled acceptance compares every coded sample, native planes,
cropped/normalized packed playback, PTS/duration, frame state and reset/rewind.
The rectangular entropy contexts now use the actual MBAFF pair field flags.
Generation is offline, separate from tests, and uses no external codec or
private source data. This matrix uses flat scaling, QP26, neutral luma and
unfiltered single-pair slices; nonzero luma, other prediction modes/scaling/QPs,
FMO and inter 422 MBAFF need their own acceptance evidence. Rectangular chroma
loop filtering has a separate matrix below.

`avc-422-mbaff-deblock/` contains 576 own two-frame movies authored by
`scripts/generate_avc_422_mbaff_deblock.py`: three precision pairs, CAVLC/CABAC,
frame/field/mixed pairs, ASO, crop, all three filter-disable modes and full/masked
signed chroma AC with DC. All slices own one pair, so disable mode 2 tests both
internal filtering and cross-slice suppression. Before the fix the 384 filtered
cases reproduced the exact 422 MBAFF filtering refusal; 192 disabled-filter
controls remain part of the enabled acceptance matrix.
The independent oracle builds physical pixel ownership, filters vertical edges
before horizontal ones in pair address order, and distinguishes field bS3 from
frame bS4 plus both parities beneath a field pair. All enabled-filter cases must
change both chroma planes; mutation checks provide 1344 witnesses for horizontal
edge classes 4/12 and 576 for field-specific strengths. Acceptance checks every
coded sample, native/cropped/packed output, precision, timing and reset/rewind.
A module regression separately compares rectangular MBAFF chroma with filtering
the two extracted fields at 8/12/14 bits, with and without luma transform8, and
checks invalid subsampling leaves samples unchanged. Tests need no generator,
FFmpeg, network or private media. Other QPs/scaling/offsets, nonzero luma,
PCM/compressed filtering mixtures, FMO and inter MBAFF remain separate coverage.

`avc-422-mbaff-p/` contains 504 own three-frame movies authored by
`scripts/generate_avc_422_mbaff_p.py`: three precision pairs, CAVLC/CABAC,
frame/field/mixed pair layouts, ASO, crop, zero or signed integer motion and
same/opposite-parity references. The first frame is own PCM; the second uses
coded P16x16 prediction with zero CBP; the third uses zero-motion P prediction
from the second, so a transient correct picture cannot hide broken reference
storage. A physical-coordinate clamped sampler independently defines all
three full-resolution component planes. The matrix has 336 motion witnesses
and 216 opposite-parity witnesses, including distinction from the erroneous
420 half-sample chroma phase applied to 422 field references.
All cases reproduced the exact MBAFF reconstruction configuration refusal on
the second packet before enabling rectangular inter publication/reference views.
Enabled acceptance checks every coded sample, native/cropped/normalized packed
output, component precision, PTS/duration, frame state and reset/rewind. A module
regression separately checks rectangular CAVLC count neighbours across mixed
frame/field owners and foreign slices. Generators stay offline and outside
ordinary tests; no FFmpeg, network or private media is used. Fractional motion,
partitions, weighted prediction, skip and B pictures, inter filtering, FMO and
other tools need their own end-to-end qualification. Nonzero residuals have the
separate acceptance matrices below.

`avc-422-mbaff-p-residual/` contains 1728 own three-frame movies authored by
`scripts/generate_avc_422_mbaff_p_residual.py`: three precision pairs,
CAVLC/CABAC, four frame/field layouts, ASO, crop, signed or absent chroma DC,
full/masked/absent chroma AC and signed luma DC/AC. Selected signed integer
motions and same/opposite-parity references precede the residual; the final
zero-motion P picture must reuse the reconstructed result. The independent
scalar oracle adds/clips inverse bases to the physical prediction and requires
visible frame/field scan mutation witnesses for every AC case/component.
These cases pass the existing native rectangular inter reconstruction; this
matrix adds end-to-end residual qualification rather than changing a refusal
into an unverified success. Enabled acceptance compares every coded sample,
native/cropped/normalized packed output, precision, timing, reset and rewind.

`avc-422-mbaff-p-dense/` contains 96 own three-frame CAVLC movies authored by
`scripts/generate_avc_422_mbaff_p_dense.py`. A joined eight-macroblock P slice
crosses mixed frame/field pair boundaries with 0/1/4 AC coefficients per chroma
block and zero chroma DC; all three nC VLC classes below eight are required in
every case. An independent physical owner/availability map drives entropy
contexts, and an integer matrix-basis oracle supplies expected samples without
calling the decoder's inverse transform. Signed coefficients, three precision
pairs, crop, first/echo ASO and reference echo are covered. The same enabled
sample/playback/timing/reset/rewind contract applies. Both new generators are
offline and separate from tests, without FFmpeg, network or private source data.
These matrices use flat scaling, QP26, 4x4 transforms, disabled filtering and
full P16x16 partitions; other levels/scaling/QPs, 8x8/bypass, other partitions, inter filtering,
weighted prediction and FMO remain separate qualifications. Fractional motion
and baseline B pictures have separate matrices below.

`avc-422-mbaff-b/` contains 576 own four-frame movies authored by
`scripts/generate_avc_422_mbaff_b.py`: three precision pairs, CAVLC/CABAC,
frame/field/mixed pairs, ASO, crop, zero/moving explicit biprediction,
spatial/temporal direct and coded-direct-top/skipped-bottom pairs. Independently
sampled past/future arrays define the rounded mean; both lists and explicit
motion must visibly affect output. The future reference is PCM with a different
pair layout from the B picture, so frame/field reference-view selection is
also exercised. Decode order is I0/I4/B2/P6; display order is I0/B2/I4/P6.
The final zero-motion P picture must reproduce I4, proving non-reference B2
did not replace the reference store. Enabled acceptance checks exact coded,
native/cropped/normalized packed output, component precision, decode/sample
indices, reordered PTS/duration, reset and rewind. Direct-mode coverage here
uses intra co-located blocks with zero derived vectors; nonzero temporal/spatial
co-located motion and other B tools remain separate qualifications.

`avc-422-mbaff-p-fractional/` contains 1344 own three-frame movies authored by
`scripts/generate_avc_422_mbaff_p_fractional.py`: three precision pairs,
CAVLC/CABAC, four pair layouts and both reference parities (frame pairs have no
field parity syntax). Signed dx=-4..3 and dy=-2..1 cover all 16 quarter-luma
phases plus all eight horizontal/four vertical chroma interpolation phases of
422. Crop and ASO are selected across the grid. An independent scalar six-tap
luma and bilinear chroma sampler uses clamped physical reference views; it
provides 1302 witnesses against nearest/integer truncation and 576 against
same-parity selection and the erroneous 420 opposite-field phase. The last
zero-motion P picture must reproduce the fractional result from reference
storage. The complete coded/native/packed/crop/precision/timing/reset/rewind
acceptance contract applies. These are enabled acceptance tests for existing
native prediction, not refusal tests or fixture-generation checks. Generation
is offline, outside tests, and uses no FFmpeg, network or private media.
Other motion magnitudes, weighted/fractional residual combinations, partitions,
inter filtering, FMO and remaining B tools need separate coverage.

`avc-422-mbaff-b-colocated/` contains 960 own four-frame movies authored by
`scripts/generate_avc_422_mbaff_b_colocated.py`. The future POC4 reference is
predicted from I0 with signed even/odd nonzero quarter-pel vectors. B2 temporal
vectors use half-distance scaling after frame/field vertical-unit conversion,
including truncation toward zero before signed rounding. Same and converted
frame/field/mixed reference layouts, CAVLC/CABAC, three precision pairs,
ASO/crop, coded-direct and skipped-bottom pairs are covered. 192 explicit-bi
twins encode the independently calculated vectors; they and temporal/direct
variants must reconstruct the same scalar pixel oracle. Spatial variants here
are boundary controls with unavailable prediction neighbours. P6 must reproduce
the future POC4 reference, preserving non-reference B state. Enabled acceptance
checks complete coded/native/cropped/packed samples, precision, decode/display
order, timing and reset/rewind. Other POC ratios and motion/reference choices
remain separate qualifications.

`avc-422-mbaff-b-spatial/` contains 288 own four-frame movies authored by
`scripts/generate_avc_422_mbaff_b_spatial.py`. The B picture is one joined
MBAFF slice: its first pair explicitly seeds both motion lists, while later
pairs use spatial direct or coded-top/skipped-bottom syntax. An independent
physical owner/availability map, signed frame/field neighbour conversion and
median prediction define per-macroblock vectors; a scalar fractional sampler
supplies the resulting pixels. All four pair layouts, both entropy modes,
three precision pairs and positive/negative seeds are covered. Future P motion
0/0 must trigger colZero despite nonzero neighbours; raw 1/2 and 8/4 must retain
spatial vectors. Every case requires clipped-pixel witnesses against ignoring
colZero or forcing direct vectors to zero, and 72 cases additionally distinguish
raw from prematurely normalized colZero tests. CABAC skip contexts are authored
using the inferred pair mode before the field flag, then actual mode for type
and CBP contexts. Complete sample/playback/precision/order/timing/reset/rewind
acceptance applies, including P6 reference echo. Generation is offline, separate
from ordinary tests, and uses no FFmpeg, network or private media. Weighted
prediction, other references/partitions, inter filtering, residual combinations,
FMO and other unqualified tools remain separate work.

`avc-422-mbaff-p-weighted/` contains 504 own three-frame movies authored by
`scripts/generate_avc_422_mbaff_p_weighted.py`: three precision pairs,
CAVLC/CABAC, four pair layouts, signed fractional motion, both field-reference
parities and three explicit component weight profiles. Negative/zero/positive
weights, boundary offsets and denominators 0/1/2/7 are exercised; denominator-7
default flags supply identity weights 128 without coding an out-of-range
explicit weight. Half the cases add signed luma/chroma DC/AC residuals after
weighted/clipped prediction, then an identity-weighted P picture must reproduce
the final reference pixels. Independent component arithmetic counts 397348
clipped predictions. Enabled acceptance checks all coded/native/cropped/packed
samples, component precision, PTS/duration, reset and rewind.

`avc-422-mbaff-b-weighted/` contains 576 own four-frame movies authored by
`scripts/generate_avc_422_mbaff_b_weighted.py`. Three explicit profiles and
implicit weighting cross L0-only, L1-only, fractional biprediction, spatial,
temporal and skipped-bottom modes with mixed frame/field reference layouts.
The moving future P picture is at POC6 and B is at POC2, so implicit biprediction
requires 43/21, while implicit uniprediction must remain unweighted. Temporal
vectors independently use distance scale 85 after frame/field normalization.
Explicit negative/zero/positive/default weights, different luma/chroma
denominators, and offset averaging after component-depth scaling are checked
against scalar arithmetic; 500482 predictions clip. POC8 reference echo must
reproduce POC6 after the non-reference B picture. The complete coded/native/
packed/crop/precision/decode-order/presentation-timing/reset/rewind contract
applies. `scripts/avc_fixture_weights.py` authors only weight-table syntax and
independent scalar sample arithmetic; it never invokes the decoder.

`avc-422-mbaff-b-field-weights/` contains 288 own four-frame movies authored by
`scripts/generate_avc_422_mbaff_b_field_weights.py`. Explicit bottom-POC deltas
give the past/current/future frames field orders [0,2]/[4,8]/[6,10]. Field pairs
must use selected-field POCs rather than picture minima: same-parity weights
are 22/42 on top and 16/48 on bottom; opposite-parity biprediction requires
48/16 on top and -21/85 on bottom. Temporal vectors also use the selected-field
distance scale, and frame pairs retain frame-POC weighting. Scalar mutation
checks require different clipped pixels for frame-only POCs and wrong reference
parity; 1872 predictions clip. L0/L1 controls verify implicit weighting stays
disabled for uniprediction. CAVLC/CABAC, all four pair layouts, mixed reference
layouts, precision, crop, ASO, non-reference B and POC12 reference echo are
covered by enabled complete sample/timing/reset/rewind acceptance.
All three matrices pass existing native weighted prediction; they add explicit
end-to-end qualification, not a claimed decoder repair. Generation is offline
and separate from ordinary tests, with no FFmpeg, network or private media.
Other denominators/weights/reference indices, long-term or out-of-range implicit
fallbacks, weighted partition/FMO/inter-filter combinations and other tools
remain separate qualifications.

`avc-422-mbaff-p-partitions/` contains 612 own three-frame movies authored by
`scripts/generate_avc_422_mbaff_p_partitions.py`. P16x8/P8x16 and all P8x8
subdivision sizes 8x8/8x4/4x8/4x4, mixed subtypes and CAVLC-only P8x8ref0 are
covered with signed MVD sequences, CAVLC/CABAC, three precision pairs, four
frame/field layouts and same/opposite field references. An independent physical
motion grid applies preferred/unique/median predictors in syntax order; a scalar
fractional sampler reconstructs each luma 4x4 cell and its rectangular 422
chroma counterpart. Every case requires a clipped-pixel witness against using
one uniform macroblock vector. CABAC reference contexts publish complete groups
before MVD contexts; P8x8ref0 emits no reference syntax in CAVLC and is not
misrepresented as a CABAC macroblock type. Enabled acceptance compares every
coded/native/cropped/packed sample, precision, PTS/duration, state, reset and
rewind. The final zero-motion P picture must reproduce the assembled partition
result from reference storage. Generation is offline, separate from ordinary
tests, and uses no external codec, network or private media. B partitions,
other reference choices, weighted/residual partition combinations and other
tools require separate qualification; inter filtering is tested separately.

`avc-422-mbaff-inter-deblock/` contains 1512 own three-frame movies authored by
`scripts/generate_avc_422_mbaff_inter_deblock.py`. A smooth, wholly synthetic
PCM reference feeds moving P16x8, mixed-subtype and P4x4 pictures at canonical
QP50 (chroma QP39), with optional signed DC residual in every luma 4x4 block.
The residual-free cases exercise motion-derived bS0/1, while nonzero luma
coefficients require bS2 on both luma and chroma even without chroma residuals.
All three disable modes, independent pair slices, same/opposite field references,
four frame/field layouts, both entropy modes, three precision pairs, ASO and
crop are covered. A physical component-owner map independently traverses
vertical then horizontal edges, uses field vertical-vector threshold 2, filters
both parities of a frame boundary below a field pair, and retains all four
horizontal 422 chroma edge classes. Scalar weak-filter arithmetic supplies the
expected samples without calling the decoder.
924 cases visibly change; 504 disabled-filter controls and 84 enabled but
unchanged controls preserve expected pixels. Every enabled case with luma
residual must visibly filter all three components. Component-level mutation
comparisons supply 936 witnesses against frame-style field-vector thresholds,
1280 against omitting chroma horizontal edges 4/12, and 432 against omitting the
second mixed-boundary parity. Enabled acceptance checks all coded/native/
cropped/packed samples, precision, PTS/duration, state, reset and rewind. A final
unfiltered zero-motion P picture must reproduce the filtered reference, rather
than the pre-filter reconstruction. These cases pass existing native inter
filtering; this matrix adds qualification rather than claiming a decoder repair.
Generation is offline and separate from ordinary tests, with no FFmpeg, network
or private media. Other QPs/offsets/scaling, inter/intra mixtures, other references,
weighted/B/FMO filter combinations and remaining codec tools need separate
qualification.

`avc-422-mbaff-b-partitions/` contains 840 own four-frame movies authored by
`scripts/generate_avc_422_mbaff_b_partitions.py`: all 18 two-part B macroblock
types, all 13 B subtypes and four mixed subtype sets. Three precision pairs,
CAVLC/CABAC, four frame/field layouts, signed fractional MVDs, field references
0/1, ASO and crop are covered. An independent two-list physical motion grid
distinguishes unavailable neighbours from available cells not using a list;
reference-matching/preferred/median prediction applies before subpartition
publication. Spatial direct freezes external macroblock neighbours before any
explicit subpart is reconstructed. Reference syntax for both lists precedes
MVD syntax for both lists, and CABAC syntax contexts retain zero conditions
for direct/unused-list cells rather than their derived display motion.
A scalar sampler reconstructs every luma 4x4 cell and its 2x4 422 chroma region,
then selects uni- versus biprediction per cell. Enabled acceptance checks exact
coded/native/cropped/packed samples, precision, decode/display order, timing,
reset/rewind and a final P reference echo. Direct subtypes here use an intra
co-located reference; other co-located/subpartition combinations remain separate.

`avc-422-mbaff-b-partition-weights/` contains 1152 own four-frame movies authored
by `scripts/generate_avc_422_mbaff_b_partition_weights.py`. Sixteen representative
macro/subtype mixtures cross explicit component weights, implicit 43/21 weights,
and selected-field implicit weighting with field orders [0,2]/[4,8]/[6,10].
L0/L1-only cells use uni weighting (or identity for implicit weighting), while
bi cells combine raw list predictions before clipping and offset averaging.
Different references inside one macroblock may select different field POCs and
implicit weights. Independent scalar mutation comparisons provide 252 witnesses
against picture-only POCs and 252 against forced matching-parity references.
All four pair layouts, both entropy modes, three precision pairs, crop/ASO,
reordered timing and P reference echo are exercised by enabled complete sample/
playback/reset/rewind acceptance. The 5881 baseline B-partition files regenerate
identically after extending the authors for weights. Both matrices pass existing
native reconstruction; they qualify these combinations rather than claiming a
decoder repair. Generation stays offline and separate from tests, with no
FFmpeg, network or private media. Residual/filter/FMO combinations, other
references/profiles and remaining codec tools need separate qualification.

### AVC separate colour planes: progressive CAVLC PCM

`avc-separate-pcm` contains six independently authored two-frame High 4:4:4
Predictive streams: 8/10/14-bit samples, cropped output, and normal/reversed
NAL order. Each of the three colour planes has two separately coded PCM
macroblock slices. The original decoder rejected these streams with
`too many AVC slices for picture` before reconstructing any samples.
`tests/avc_separate_pcm.rs` checks all coded samples, cropped packed MP4
playback, presentation timestamps, rewind/reset, and rejection of missing or
duplicated colour-plane slices against owned integer-pattern oracles. Generate separately with
`python3 scripts/generate_avc_separate_pcm.py`; ordinary tests need neither
FFmpeg nor network access. This matrix covers separate-plane PCM only. The separate-plane compressed
intra matrix below extends acceptance to CAVLC/CABAC; the complementary-field
matrix further below covers field pictures. Inter prediction has its own
acceptance matrices further below.

### AVC independently coded colour-plane intra reconstruction

`avc-separate-intra` has 180 owned progressive High 4:4:4 Predictive streams:
8/10/14-bit output, CAVLC/CABAC, normal/reverse slice order, mixed PCM/I16/I4
planes, positive/negative/zero I16 DC, flat scaling, and distinct per-plane
SPS or PPS scaling weights. The SPS intentionally declares eight-bit chroma:
separate colour planes use the luma precision and luma QP derivation for all
three components. Scalar integer oracles select the weight belonging to the
actual `colour_plane_id`, so reusing the Y scaling list changes reconstructed
Cb/Cr samples. The pre-fix acceptance failed specifically with
`AVC separate-plane compressed macroblocks are not connected`.

Generate separately with `python3 scripts/generate_avc_separate_intra.py`.
`tests/avc_separate_intra.rs` checks decoded planes and reset, cropped packed
MP4 samples, presentation time/duration and rewind, retained-plane memory
accounting, and per-component 4x4/8x8 matrix selection and fallback. This does
not establish inter/field/FMO support, nor exercise every compressed intra
mode or nonzero 8x8 residual. Ordinary execution uses no FFmpeg or network.
Scaling selection follows H.264 (08/2024), clause 8.5:
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-202408-I%21%21PDF-E&lang=e&type=items

### Profile-invalid separate-plane FMO is refusal coverage

H.264 (08/2024) A.2.7 forbids FMO for High 4:4:4 Predictive (profile 244):
`num_slice_groups_minus1` must be zero. Consequently separate-plane FMO is
not a missing playback capability of this profile. `avc-separate-fmo` has ten
owned negative inputs covering map types 0..6 and both directions for changing
maps. `tests/avc_separate_fmo.rs` requires an explicit invalid-profile refusal;
it is not an acceptance test. Generate separately with
`python3 scripts/generate_avc_separate_fmo.py`.

The reverse slice-order cases in the preceding profile-244 matrices are
permissive decoder stress tests, not profile-conformance evidence: A.2.7 also
forbids arbitrary slice order. Their ordered cases remain playback acceptance.
Reference: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-202408-I%21%21PDF-E&lang=e&type=items

### Complementary separate-colour-plane intra fields

`avc-separate-fields` has 120 owned level-4.1 PAFF field pairs, with physical top/bottom
sample oracles independent of packet arrival order. The matrix covers CAVLC
and CABAC, 8/10/14-bit luma precision on all three independent components,
flat/per-component SPS matrices, mixed PCM/I16/I4 planes, signed/zero I16 DC,
and both top-first and bottom-first arrival. Each plane has two MB slices in
ascending address order; these are not the profile-invalid FMO/ASO combinations
above. The old decoder failed on the first field with
`AVC I field configuration is not connected`.

Generate separately with `python3 scripts/generate_avc_separate_fields.py`.
`tests/avc_separate_fields.rs` compares every woven coded sample, component
precision and crop, pending/completed-field state, reset, software MP4 packed
output, presentation/duration and rewind. This matrix does not qualify inter
fields, MBAFF, nonzero field AC/8x8 residual, or every intra/filter combination.
Generation and ordinary execution require no FFmpeg, external codec or network.

### Separate-plane MBAFF compressed intra

`avc-separate-mbaff-intra` contains 144 owned two-frame level-4.1 streams.
CAVLC/CABAC, 8/10/14-bit precision, I16 DC, I4 DC/AC and flat/distinct SPS
scaling weights are crossed with four pair layouts. The three colour planes
use different layouts in every case; field/frame mode and physical readiness
must be independent for each plane. Nonzero I4 AC has an independent scalar
orientation witness against applying the frame scan to field-coded blocks.
All slices remain in ascending pair-address order within each colour plane.
The pre-fix acceptance fails specifically with
`AVC separate-plane intra requires I frame or field slices without MBAFF`.

Generate separately with `python3 scripts/generate_avc_separate_mbaff_intra.py`.
`tests/avc_separate_mbaff_intra.rs` compares full coded planes, precision/crop,
reset, software MP4 packed samples, frame timestamps/duration and rewind.
The field-pair author above was corrected from level 5.1 to 4.1: H.264 A.3,
Table A-4 requires `frame_mbs_only_flag=1` at level 5.1. Both new MBAFF and
PAFF acceptance explicitly assert the corrected SPS/configuration level.
This does not qualify inter MBAFF, every intra/filter combination or nonzero
8x8 residual. Generation and tests remain independent and require no FFmpeg,
external decoder, network or private media.

### Independent colour-plane inter state

`avc-separate-inter` contains 72 owned level-4.1 progressive I/P/B/P streams:
8/10/14-bit precision, CAVLC/CABAC, temporal/spatial direct, B skip, explicit
bi partitions, and ordinary/explicit/implicit bipred weights. Y/Cb/Cr use
separate signed fractional vectors `(8,-4)`, `(-7,5)`, `(3,6)`. The co-located
P at POC6 predicts from I0; B2 derives its temporal vectors from that plane's
own stored motion (scale85). The final P8 references P6, proving the nonreference
B did not replace it. Own scalar six-tap interpolation and weighting oracles
cover every full-resolution component sample. There are 73656 sample witnesses
against reusing Y's motion for another plane. The pre-fix decoder accepted I0
and refused P6 with `unsupported inter-picture reconstruction tools`.

Generate separately with `python3 scripts/generate_avc_separate_inter.py`.
`tests/avc_separate_inter.rs` checks coded planes, crop/precision, reset, software
MP4 reordering to I/B/P/P, sample indices, timestamps/duration and rewind. It
also checks aggregate output storage and transitions to monochrome and back at
complete IDR boundaries, using existing owned PCM fixture parameter sets.
Each plane has its own native decoder/reference/motion history; the owner
reserves assembled output storage before dividing the reconstruction budget.
Raw slice headers are parsed before selecting a plane, preserving the original
`colour_plane_id` syntax and offsets. Resolved per-plane scaling lists are then
projected onto the native monochrome engine. No production bitstream rewrite,
external decoder or FFmpeg fallback is involved.

All 865 existing `avc-422-b-motion` files regenerate identically after extending
the authors with optional separate-plane syntax. The new 506 assets regenerate
identically too. This acceptance does not establish inter fields/MBAFF,
nonzero inter residual, all partitions/reference operations or every profile.
Generators remain separate from offline tests and contain no private media.

### Monochrome/separate-plane MBAFF P reconstruction

`avc-separate-mbaff-p` contains 144 owned level-4.1 I/P/P movies: CAVLC/CABAC,
8/10/14-bit precision, four layouts with different modes for Y/Cb/Cr, signed
fractional vectors, matching/opposite reference parity, and absent/DC/AC luma
residual. Each coded residual has one coefficient per 4x4 block at QP26. The
scalar oracle samples full-resolution frame or alternating field rows and
uses separate horizontal/vertical AC bases. P2 changes the pair layout and
must echo P1's complete post-residual/clipping reference samples exactly.
There are 881446 witnesses against zero motion and 220862 against selecting
the wrong reference field. Before the fix, I0 passed and P1 failed with
`invalid MBAFF reconstruction configuration`.

Generate separately with `python3 scripts/generate_avc_separate_mbaff_p.py`.
`tests/avc_separate_mbaff_p.rs` compares every coded component, crop/precision,
reset, software MP4 packed samples, timestamps/duration and rewind. Native
monochrome MBAFF now has checked Y-only references, luma residual contexts
and publication without decoded chroma storage; empty chroma planes are not
sent to chroma filtering. Other component layouts retain their existing
reference, publication and filtering paths.

The new 866 emitted files reproduce identically, as do 3025 existing MBAFF
422 P files and 995 component PCM files after extending the authors. This
matrix does not qualify B/direct, all partitions/reference operations, weights,
nonzero 8x8 residual or enabled monochrome deblocking. Generation stays
separate from offline tests; no FFmpeg, external codec or private media is used.

### Separate-plane MBAFF B/direct and vertical motion conversion

`avc-separate-mbaff-b` contains 240 owned level-4.1 I/P/B/P movies: CAVLC/CABAC,
8/10/14-bit precision, four base layouts with distinct pair topologies across
Y/Cb/Cr, matching and converted co-located P layouts, and `temporal`,
`temporal-skip`, `spatial`, `spatial-skip` and `bi-derived` modes. Each colour
plane uses its own co-located P motion vector `(8,4)`, `(-3,3)`, `(3,-3)` and
derives temporal vectors with per-pair frame/field vertical unit conversion
(`*2` for field-to-frame, `trunc(/2)` for frame-to-field) before scaling. The
final P frame predicts from P1, proving the nonreference B frame did not alter
the plane's reference store. There are 878868 sample witnesses against zero
motion and 310548 against omitting vertical frame/field unit conversion.

Generate separately with `python3 scripts/generate_avc_separate_mbaff_b.py`.
`tests/avc_separate_mbaff_b.rs` compares every coded component sample, crop and
precision, reset, software MP4 reordered presentation (`I0, B2, P1, P3`),
sample indices, timestamps/duration and rewind. The 1442 emitted files
regenerate identically without FFmpeg, external decoders, network or private
media.

### Non-separated 4:4:4 progressive CABAC and MBAFF CAVLC/CABAC PCM

`avc-444-pcm` contains 132 owned High 4:4:4 Predictive (`profile_idc = 244`,
`chroma_format_idc = 3`, `separate_colour_plane_flag = 0`) two-frame movies and
4 compressed-syntax refusal fixtures and one progressive compressed acceptance:

- 12 progressive CABAC (`frame_mbs_only_flag = 1`, `entropy_coding_mode_flag = 1`)
  movies across `(8,8)`, `(10,12)` and `(14,8)` luma/chroma depths, single-slice
  and ASO multi-slice ordering, and uncropped/cropped `[1,1,1,1]` geometry.
  Before the fix, these failed with `AVC non-4:2:0 PCM requires progressive single-group CAVLC I slices with deblocking disabled`.
- 96 multi-slice MBAFF (`frame_mbs_only_flag = 0`, `mb_adaptive_frame_field_flag = 1`)
  CAVLC/CABAC movies across `(8,8)`, `(10,12)` and `(14,8)` luma/chroma depths,
  `frame`, `field`, `columns` and `checker` pair topologies, ascending and ASO
  slice orders, and uncropped/cropped `[1,1,2,2]` geometry, plus 24 single-slice
  CABAC MBAFF movies exercising cross-pair `mb_field_decoding_flag` and
  `mb_type` neighbour context selection across mixed frame/field pairs. Before
  the fix, these fell through to compressed MBAFF reconstruction and failed with
  `AVC scaling reconstruction requires 4:2:0` or `MBAFF reconstruction exceeds memory budget`.
- 4 explicit refusal fixtures (`unsupported.json`) verifying that compressed
  non-separated 4:4:4 MBAFF CAVLC/CABAC macroblocks still return
  `Error::Unsupported`. `accepted.json` replaces the historical incomplete
  progressive CABAC refusal with a valid complete three-component example.

Generate separately with `python3 scripts/generate_avc_444_pcm.py`.
`tests/avc_444_pcm.rs` verifies native per-component Y/Cb/Cr sample planes,
`write_planar`, exact `required - 1` vs `required` memory budgets, decoder
reset, software MP4 `subsampling = Some([1, 1])` packed playback, timestamps,
duration, rewind and compressed-syntax refusals offline without FFmpeg,
network or private media.

### Separate-colour-plane PAFF P and B fields

`avc-separate-field-inter` contains 288 owned level-4.1 High 4:4:4 Predictive
(`profile_idc = 244`, `chroma_format_idc = 3`, `separate_colour_plane_flag = 1`,
`frame_mbs_only_flag = 0`, `mb_adaptive_frame_field_flag = 0`) PAFF inter-field
movies across 8, 10 and 14-bit precision, CAVLC and CABAC, and top-first and
bottom-first field arrival:

- 144 P-field movies (`separate-field-p-*`): same-parity and opposite-parity
  reference field selection (`o0`/`o1`), unweighted and explicit per-plane
  weighted prediction (`w0`/`w1`), and absent/DC/field-scanned-AC luma
  residuals (`none`/`dc`/`ac`) with distinct per-plane fractional motion
  vectors `(8, -4)`, `(-7, 5)` and `(3, 6)`, followed by a P-skip echo field
  pair (`I0, P2, P4`).
- 144 B-field movies (`separate-field-b-*`): ordinary, explicit and implicit
  bipred weights (`w0`/`w1`/`w2`) and `temporal`, `skip`, `spatial` and `bi`
  prediction modes across reordered `I0 / P6 / B2 / P8` PAFF field pairs with
  moving co-located P6 fields and independent per-plane temporal direct scaling.
- `coverage.json` records 294496 P-field per-plane motion witnesses, 220528
  opposite-parity selection witnesses, 107464 field-scan AC residual witnesses
  and 73424 B-field per-plane motion witnesses.

Before the fix, direct `avc_field_picture` entrypoints (`decode_skip_field`,
`decode_p_field`, `decode_inter_field_lists_with_order`,
`decode_inter_field_lists_with_motion`) rejected separate-colour-plane PAFF
fields with `invalid AVC P field slice count` or
`AVC P field reference configuration is not connected`.

Generate separately with `python3 scripts/generate_avc_separate_field_inter.py`.
`tests/avc_separate_field_inter.rs` verifies `AvcDecoder::decode_order`,
direct `avc_field_picture` entrypoints (including explicit refusal when direct B
fields omit reference motion context), and software MP4 playback (`I0, B2, P6, P8`
display reordering, timestamps, duration, `subsampling = Some([1, 1])` cropped
planes and rewind) offline without FFmpeg, network or private media.

### Separate-colour-plane 8x8 transforms, 12-list scaling, ASO and deblocking

`avc-separate-8x8-deblock` contains 216 owned level-4.1 High 4:4:4 Predictive
(`profile_idc = 244`, `chroma_format_idc = 3`, `separate_colour_plane_flag = 1`,
`transform_8x8_mode_flag = 1`) progressive and PAFF field movies across 8, 10
and 14-bit precision, CAVLC and CABAC, three 12-list 8x8 scaling modes
(`sps`, `pps` and `fallback` exercising H.264 7.4.2.1.1.1 / 7.4.2.2 fallback
rules A and B across `ScalingList8x8[0..5]`), `disable_deblocking_filter_idc`
values `0`, `1` and `2`, and ascending vs. arbitrary slice order (`a0` / `a1`):

- 108 progressive movies (`sep-8x8-*-prog-*`): two-frame `I_8x8` + `P_8x8`
  streams with two slices per colour plane (`first_mb = 0` and `1`), reference
  sample filtering across internal 8x8 DC edges (`8.3.2.2.2`), frame-scanned
  inter 8x8 AC residuals (`scan_idx = 1 -> raster_pos = 1`), and internal 8x8
  plus cross-slice (`disable_deblocking_filter_idc = 0` vs. `2`) loop filtering.
- 108 PAFF field movies (`sep-8x8-*-paff-*`): four field access units weaving
  into two full frames (`I_8x8` top/bottom field pair followed by `P_8x8`
  top/bottom field pair), exercising field-scanned inter 8x8 AC residuals
  (`scan_idx = 1 -> FIELD_SCAN[1] = raster_pos = 8`) and CABAC field 8x8
  significance/last contexts (`436..460`).

Before the fix, direct `decode_intra_slices`, `avc_separate_pcm_picture::decode`,
`avc_deblock::intra_plane_owned_subsampled` and `ScalingMatrices` rejected or
mis-scaled ASO separate-colour-plane compressed slices (`intra picture must begin at zero`,
`AVC separate-plane slice coverage gap or overlap`,
`invalid intra deblocking metadata`) or clobbered SPS 8x8 scaling lists with
`DEFAULT8` when a 6-list PPS was active.

Generate separately with `python3 scripts/generate_avc_separate_8x8_deblock.py`.
`tests/avc_separate_8x8_deblock.rs` verifies direct `decode_intra_slices` and
`avc_field_picture` entrypoints (including raw ASO slice order),
`AvcDecoder::decode_order`, 6-list PPS scaling preservation, and software MP4
playback offline without FFmpeg, network or private media.

### Separate-colour-plane MBAFF 8x8 transforms, 12-list scaling, ASO and deblocking

`avc-separate-mbaff-8x8-deblock` contains 432 owned level-4.1 High 4:4:4 Predictive
(`profile_idc = 244`, `chroma_format_idc = 3`, `separate_colour_plane_flag = 1`,
`frame_mbs_only_flag = 0`, `mb_adaptive_frame_field_flag = 1`,
`transform_8x8_mode_flag = 1`) 32×64 MBAFF movies (four macroblock pairs in a
2×2 pair grid) across 8, 10 and 14-bit precision, CAVLC and CABAC, three 12-list
8x8 scaling modes (`sps`, `pps` and `fallback` across `ScalingList8x8[0..5]`),
four pair-topology patterns (`t0` all-frame pairs `[0,0,0,0]`, `t1` all-field
pairs `[1,1,1,1]`, `t2` column-mixed pairs `[0,1,0,1]`, and `t3` checkerboard
pairs `[1,0,0,1]` with plane-shifted pair coding), `disable_deblocking_filter_idc`
values `0`, `1` and `2`, and ascending vs. arbitrary slice order (`a0` / `a1`):

- Each movie contains two MBAFF access units (`I_8x8` IDR followed by `P_8x8`
  reference picture) with four independent slices per colour plane (`first_mb`
  pair addresses `0..3` in `a0` or reversed `[3, 2, 1, 0]` in `a1`).
- Frame-coded pairs exercise intra 8x8 reference-sample filtering across the
  internal top/bottom pair macroblock edge (`parity = 0 -> parity = 1`) and
  frame-scanned 8x8 AC residuals (`scan_idx = 1 -> raster_pos = 1`); field-coded
  pairs exercise field-scanned 8x8 AC residuals (`scan_idx = 1 -> FIELD_SCAN[1] = 8`),
  field-expanded reference indices (`num_ref_idx_l0_active = 2`), and CABAC
  field 8x8 significance/last contexts (`436..460`).
- Deblocking covers internal 8x8 block edges (`x = 8` and `y = 8` only, never
  `4` or `12`) as well as cross-pair frame/frame, field/field, frame/field and
  field/frame vertical and horizontal boundaries under `disable_deblocking_filter_idc = 0`
  and `2`.

Before the fix, `avc_mbaff_picture::decode_intra_slices` rejected
`separate_colour_plane_flag = 1` with `AVC scaling reconstruction requires 4:2:0`
and rejected unsorted ASO slices with
`MBAFF intra picture must start at macroblock pair zero`, while
`avc_picture::reconstruct_macroblock` overwrote SI `ready_value` with `1` for
`IntraLuma::Blocks8`.

Generate separately with `python3 scripts/generate_avc_separate_mbaff_8x8_deblock.py`.
`tests/avc_separate_mbaff_8x8_deblock.rs` verifies direct
`avc_picture::decode_intra_slices` and `avc_mbaff_picture::decode_intra_slices`
(including raw ASO slice order), `AvcDecoder::decode_order`, and software MP4
playback (`subsampling = Some([1, 1])` cropped planes and rewind) offline
without FFmpeg, network or private media.


## Nonseparate H.264 4:4:4 compressed CAVLC intra

`avc-444-intra` contains 60 owned level 4.1 progressive examples with shared
macroblock modes and independent Y/Cb/Cr residual contexts. See its README for
exact acceptance coverage. Progressive CABAC is qualified below; compressed
inter/MBAFF coverage remains separate; intra fields are qualified below.
Generate separately with `scripts/generate_avc_444_intra.py`; ordinary offline
tests use `tests/avc_444_intra.rs` and need no FFmpeg or network.

## Nonseparate H.264 4:4:4 loop filtering

`avc-444-deblock` adds 324 compressed CAVLC intra filtering cases, and
`avc-444-pcm-deblock` adds 54 progressive CAVLC/CABAC PCM filtering cases.
`avc-444-mixed-deblock` adds 27 shared-slice PCM/I16 context and filtering cases.
All three use independent owned scalar oracles, full-plane and software MP4
acceptance checks, with fixture authoring separate from offline test execution.
See their READMEs for witnesses and the remaining compressed inter/MBAFF gaps; intra fields are qualified below.

SAMI attribute boundaries: `sami-attributes/` contains twelve authored pairs and a five-frame raw Y4M companion. `tests/sami_attribute_values.rs` checks whole attribute names, matching quotes, independent caption clocks and correct omission of untimed text; the native player regression also presents video and seeks. Generate separately with `scripts/generate_sami_attribute_regressions.py`.

## Nonseparate H.264 4:4:4 CABAC intra

`avc-444-cabac-intra` adds 486 owned compressed-intra cases with explicit
full-resolution residual contexts and 8x8 coded-block flags.
`avc-444-cabac-mixed` adds 27 shared-slice PCM/arithmetic-restart cases.
`avc-444-cabac-transform-switch` adds 162 mixed 4x4/8x8 neighbour-context cases.
The old progressive refusal in `avc-444-pcm` is now full-component acceptance;
the four historical MBAFF refusals now have complete shared-component acceptance
fixtures. See `avc-444-mbaff-intra` and the case READMEs for exact coverage,
table provenance and remaining inter gaps.

SAMI Sync clocks: `sami-sync/` pairs six authored paragraph sequences with independent SRT clocks and a five-frame Y4M companion. It covers replacement,clear events,quoted clocks,multiline captions,language gaps and an overflowing clock. Generate separately with `scripts/generate_sami_sync_regressions.py`; `tests/sami_sync.rs` checks the intended capability. Native Player validation uses the existing authored indexed VP9 clip for actual forward/backward seek.

SAMI break case: `sami-break-case/` isolates uppercase/mixed-case break tags with explicit supported paragraph clocks. `tests/sami_break_case.rs` checks successful multiline captions against independent SRT after the case fix. Generate with `scripts/generate_sami_break_regressions.py`.

SAMI sidecar extension: `sami-sidecar-extension/` isolates automatic discovery from explicit parsing. Lowercase,uppercase and language-suffixed `.sami` names have authored raw video companions; `.smi` is a healthy control and an unrelated filename must remain excluded. `tests/sami_sidecar_extension.rs` checks explicit loading and successful discovery after the extension fix. Generate with `scripts/generate_sami_sidecar_regressions.py`.

## Nonseparate H.264 4:4:4 intra fields and PAFF frames

`avc-444-fields` adds 540 paired-field scan/precision/filter/parity cases.
`avc-444-fields-mixed` adds 108 shared-slice PCM/I16 field pairs, and
`avc-444-paff-frames` adds 54 complete frames under field-capable SPS geometry.
All use owned samples, independent scalar oracles and native/software MP4
acceptance without external decoders or network. See their READMEs for exact
coverage. `avc-444-mbaff-intra` adds 360 compressed/PCM MBAFF intra videos
with I16/I4/I8 residuals and full-resolution component deblocking. Field inter has dedicated matrices below; nonseparate MBAFF inter qualification remains open.

SAMI body boundaries: `sami-body-boundaries/` contains four failing quoted/comment/raw-header cases and two healthy controls,with independent SRT and raw Y4M. `tests/sami_body_boundaries.rs` retains the reproduction and ignored intended acceptance pending the body-boundary fix. Generate separately with `scripts/generate_sami_body_regressions.py`.

## Nonseparate H.264 4:4:4 progressive CAVLC inter

`avc-444-inter` adds 90 P motion/weight/residual/mixed-intra videos.
`avc-444-b-inter` adds 72 B/direct and weighted biprediction/reordering videos.
`avc-444-inter-deblock` adds 54 full-resolution component-strength/QP videos.
All require exact native and software MP4 pixels, timestamps, reset and rewind,
without FFmpeg. CABAC has its own matrices below; field/PAFF inter has dedicated matrices below; MBAFF inter remains unqualified.
See each matrix README for the precise accepted combinations.

## Nonseparate H.264 4:4:4 progressive CABAC inter

`avc-444-cabac-inter` adds 324 P residual/motion/weight/skip/intra videos.
`avc-444-cabac-b-inter` adds 216 B/direct and weighted-reordering videos.
`avc-444-cabac-inter-deblock` adds 162 component CBF/QP/filter videos.
Every matrix covers cabac_init_idc 0/1/2, actual native/software MP4 pixels,
clocks, reset and rewind without FFmpeg. Explicit 8x8 CBFs see current-block
classification before residual decoding; skip clears full-resolution Cb/Cr
coefficient history. These are progressive matrices, not full H.264 completion.

## Nonseparate H.264 4:4:4 field/PAFF inter

`avc-444-field-inter` adds 768 P field parity/weight/AC-scan cases.
`avc-444-field-b-inter` adds 864 B field motion/direct/weight/reordering cases.
`avc-444-field-inter-deblock` adds 432 component CBF/QP/filter cases.
`avc-444-paff-inter` adds 192 actual frame pictures under field-capable SPS.
CAVLC and all CABAC init sets are included, with native/software MP4 pixels,
clocks, field state, reset and rewind. The prior progressive-only inter
restriction is replaced by actual frame/field syntax admission. Adaptive-frame
MBAFF inter and combinations not represented by these matrices remain open.

## Subtitle painting regression inputs

`subtitle-painting` contains authored two-line, wrapped and outlined SubRip cues
(1–9 seconds), plus a copy of the existing owned indexed VP9 color-bar video.
Generate explicitly with `scripts/generate_subtitle_paint_regressions.py`;
ordinary painter and native seek tests use only checked-in files. The painter
regressions inspect actual laid-out rows and tessellated glyph colors.
Acceptance also requires viewing the running native window.

## XM no-envelope key-off volume state

`xm-key-off-volume-state` pairs fourteen authored scores with independent C00 or
split-row volume-zero controls. Note97/K00/delayed Kxx must cut channel volume
without losing the active sample phase: explicit volume/default commands can
restore it,while note-only triggers keep the cut volume. The authored VP9/PCM
companion and all score bytes are checked in; ordinary tests need no generator,
encoder or network. Unsupported envelopes/fadeout are outside this profile.
The older `xm-volume-lifetime/released-control` reference now uses independent
C00 then volume64;the actual score and phase/worker checks are preserved.

## XM stopped-offset gain

`xm-offset-silence` contains four authored scores with out-of-range9FF followed
by a muted note-only retrigger. Two looping cases previously emitted a transient;
natural-end and initially-zero controls already stayed silent. Independent
controls omit only the muted note and require literal-zero PCM after360ms.
The checked-in owned VP9/PCM companion,16 PCM combinations and worker seek/EOF
targets run offline;generation is explicit and separate.

## XM valid empty instrument slots

`xm-empty-slot` compares absent valid slots2/9/128 with explicitly empty
instruments,including instrument-only selection and later recovery. Out-of-format
129/255 inputs at the default level,populated slots and initial empty selection
are controls. Ten scores,40 PCM combinations and actual-worker checks use only
checked-in authored media. The initial29-byte controls exposed a separate parser
refusal;verified selection controls use full243-byte headers.

## XM short empty instrument headers

`xm-empty-header` contains eight authored layouts with short or full empty
instruments,plus four malformed controls. First,middle,last and entirely empty
positions compare against full243-byte metadata and PCM.32 PCM combinations,
worker seek/EOF and parser/refusal/resource regressions run offline;the pure
parser checks also run without player/default features. Generation remains
explicit and uses only the existing owned waveform and VP9/PCM companion.

## XM invalid instrument defaults

`xm-invalid-defaults` contains fourteen authored scores. Invalid129/255 instrument
components must preserve channel defaults while valid notes/effects still run.
Independent normalized controls isolate mute,pan and K00;valid populated/empty128
components retain legitimate restores.56 PCM combinations and actual-worker
seek/EOF checks run offline on the owned waveform and VP9/PCM companion.

## XM whole tick spans

`xm-tick-span` contains ten authored equal-row tempo/speed scores. Controls at
BPM100/125 have exact tick periods;K03/EC3 at150BPM and K02/EC2 at100BPM
both target50ms. Native duration/PCM,core seek boundaries and actual-worker
checks use the full sources. An explicitly fixed100ms decoder window isolates
the subrow deadline. Tail length follows the final row. All generation is
separate and uses the existing owned waveform and VP9/PCM companion only.

## AVI index offset origins

`avi-index-origin` contains eight authored VP9/ADPCM AVI files: IMA/MS audio,
flat or nested `rec ` lists,and `idx1` offsets from `movi` or the file start.
Four unindexed controls share the same payloads;literal predictor PCM checks
native audio. Mixed origins,wrong offsets/lengths and oversized addresses must
refuse. The identical owned VP9 picture is decoded in a Matroska companion;
AVI picture playback is outside this regression. Generation is pure Python
and separate from ordinary offline Cargo tests.

## AVI main-header field boundaries

`avi-main-header-bounds` contains six malformed AVI headers whose missing
`dwStreams` can be borrowed from a following `JUNK` chunk. The intended behavior
is parser/source refusal. Four complete56/64-byte headers provide independent
PCM and worker navigation controls. The same authored VP9 frame has a Matroska
companion;no AVI picture playback claim is made. Generation is pure Python;
ordinary tests use checked-in bytes without FFmpeg or network access.

## FLAC coded clock continuity

`flac-number-timeline` contains eleven CRC-valid streams with contradictory
frame/sample numbers or changing blocking strategies. Intended whole-file
behavior is refusal;independent native decoding of each authored subframe still
matches literal PCM. Four legal controls retain worker play/rewind/seek/EOF,
variable block sizes,short final blocks and multi-byte coded numbers. An owned
VP9/PCM video companion is checked separately. Pure Python generation is explicit;
ordinary tests use checked-in bytes without FFmpeg or network access.

## Ogg page sequence integrity

`ogg-page-sequence` uses CRC-valid authored Opus pages to isolate sequence
loss,gaps,repeats and backward counters. One removed page hides20ms as a
positive granule origin. Controls keep normal/positive-origin programs,
chaining,per-serial interleaving and initial/wrapped counter compatibility.
Native PCM is compared with independent raw-packet decoding;DTX is finite
near-silence and is not assumed to be bitwise zero. Tests cover both the
canonical media reader and standalone streaming Opus reader,including rewind.

Counter discontinuities are an explicit refusal in the strict file profile.
Pasted-header live captures with gaps require coherent reframing/resequencing;
this regression does not claim PLC or reconstruction of missing audio.

## Ogg packet continuation boundaries

`ogg-continuation-boundary` contains authored missing-flag cases and valid joined
live prefixes. Packet-shaped bytes inside Opus padding must not become new audio.
Whole40ms packets are split at legal255-byte segment boundaries;joined captures
retain only a whole-page suffix and preserve granules with coherent counters.
Controls include nil termination,multiple pages,complete EOS-less prefixes and
empty-page compatibility. Nonzero padding is valid decoder-acceptance coverage
under RFC6716;it is not claimed as conforming encoder output. Native PCM uses
independent raw-packet controls and worker mid-packet/EOF seeks. Libopus packet
checks are explicit reference evidence and are not ordinary test dependencies.

## Ogg logical-stream identity

`ogg-stream-identity` isolates missing/repeated BOS and a foreign BOS relabeling
an existing streaming reader without replacing its OpusHead. All pages have
valid CRCs,consecutive counters and complete packets. The standalone single-stream
reader must refuse these boundaries before publishing audio or mutating identity;
this does not add streaming multiplexing. Zero and nonzero serial controls keep
native PCM and worker navigation. An owned VP9/PCM companion is decoded separately.
Generation is explicit and offline;ordinary tests consume the checked-in bytes.

The standalone reader requires a first BOS and rejects another BOS encountered
before that stream's EOS. Inputs previously admitted through missing/repeated BOS
now return an explicit framing error. Its public signatures stay compatible;
callers need correctly framed headers and a separate decoding context for another
logical stream. Existing stop-at-first-EOS behavior and rewind remain unchanged.

## AVI outer RIFF boundaries

`avi-riff-boundary` isolates a top-level AVI walk that ignored RIFF's size.
Eight short-root inputs must refuse mandatory lists outside their declared form.
Two complete first-RIFF inputs must ignore an idx1 outside that form instead of
validating it as their own index. Six controls retain complete and existing
clamped overdeclared/sentinel-root behavior. These are owned VP9/ADPCM AVI with
literal predictor PCM and a separately decoded Matroska companion. This does not
qualify full OpenDML or declare arbitrary trailing structures globally valid.
Generation is explicit,pure and offline;ordinary tests consume saved samples.

- `avc-data-partitions/`: owned Extended-profile A/B/C video acceptance with
  scalar PCM/intra/inter pixel oracles, combined-stream controls, ASO association,
  MP4 seek/rewind and separate malformed/missing partition regressions. Generate
  separately using `scripts/generate_avc_data_partitions.py`; no FFmpeg/network.

- `avc-data-partitions-b/`: owned two-reference Extended-profile partitioned B
  pictures with L0/L1/Bi, spatial/temporal direct, skip/coded and PCM/intra mixes,
  ASO, scalar pixels, combined controls, decode/presentation reordering and MP4
  PTS/seek/rewind checks. Generate separately with
  `scripts/generate_avc_partitioned_b.py`; ordinary tests need no FFmpeg/network.

- `avc-data-partitions-switching/`: owned partitioned primary/secondary SP and SI
  switching/ordinary controls at QSY0/26/51, signed luma/chroma residuals, PCM,
  ASO and independent matrix pixel/MP4 timing/seek/rewind acceptance. Generate
  separately with `scripts/generate_avc_partitioned_switching.py`; no FFmpeg or
  network is required for generation or ordinary tests.

- `avc-data-partitions-fmo/`: owned FMO maps0–6 with partition A/B/C, ASO and
  interleaved partitions, multiple slices per group, signed group-specific
  residuals, PCM/skip, scalar pixels and MP4 timing/seek/rewind acceptance.
  Generate separately with `scripts/generate_avc_partitioned_fmo.py`;
  generation and ordinary tests need no FFmpeg or network.

- `avc-data-partitions-interlaced/`: owned PAFF field and MBAFF A/B/C acceptance,
  layout/reference transitions, parity-distinct scalar pixels, I/PCM and P
  residuals, ASO/FMO, direct PCM API and MP4 clocks/seek/rewind. Separate missing/
  surplus B/C failures have short MP4 reproducers. Generate separately using
  `scripts/generate_avc_partitioned_interlaced.py`; no FFmpeg/network is needed.

- `avc-data-partitions-interlaced-switching/`: owned PAFF SP/SI and MBAFF SP
  partitioned switching, signed luma/chroma, skip, field order, ASO and matrix
  pixel/clock/seek/rewind acceptance. Generate separately with
  `scripts/generate_avc_partitioned_interlaced_switching.py`.
- `avc-data-partitions-interlaced-b/`: owned two-reference PAFF/MBAFF B A/B/C,
  direct/skip/PCM, decode/presentation reordering and successor-reference checks.
  Generate separately with `scripts/generate_avc_partitioned_interlaced_b.py`.
  Both generators and ordinary tests work without FFmpeg or network.

- `avc-data-partitions-mbaff-si/`: owned MBAFF SI/ordinary/PCM A/B with mixed
  layouts, constrained neighbours, signed luma/chroma, split/ASO, manual A traces,
  independent availability/matrix pixels and MP4 timing/seek/rewind acceptance.
  Generate separately with `scripts/generate_avc_partitioned_mbaff_si.py`;
  generation and ordinary tests work without FFmpeg or network.

- `avc-redundant-recovery/`: owned missing-primary recovery using the lowest
  complete redundant representation or disjoint slices across ranks; 84
  ordinary/A-B-C, split/ASO, PPS, startup
  and reference-lifetime cases. Scalar/control pixels, exact MP4 clocks,
  reset/rewind/seek and explicit partial-area refusal tests. Generate separately
  with `scripts/generate_avc_redundant_recovery.py`, without FFmpeg or network.

- `avc-primary-recovery/`: 88 owned missing/truncated primary-slice recovery
  videos with intact-region preservation, disjoint I/P replacements, ASO,
  ordinary/A-B-C, IDR startup and non-reference lifetime. Independent entropy
  failure checks, scalar/control pixels and MP4 clock/rewind/seek acceptance.
  Generate separately with `scripts/generate_avc_primary_recovery.py`;
  no FFmpeg or network is needed.

- `wavpack-false-stereo/`: 99 owned VP9/WavPack videos qualifying mono-coded
  stereo, 8/16/24/32-bit integer reconstruction, WVX CRC/padding, Matroska compact
  packet mapping, rewind, exact seek and audio worker PCM. Six short malformed
  videos preserve CRC/padding/length refusal checks. Generate separately with
  `scripts/generate_wavpack_false_stereo.py`, without external codecs or network.

- `wavpack-float/`: 180 owned IEEE-754 WavPack/VP9 videos plus four optimized
  INT32 witnesses. Bit-exact mantissa/exponent/sign restoration, signed zeros,
  subnormals, Inf/NaN payloads, old/new WVX, CRC, Matroska and audio-worker seek.
  Ten malformed short videos preserve precise refusal checks. Generate separately
  with `scripts/generate_wavpack_float.py`; no external codec or network required.

- `wavpack-multichannel/`: 115 owned 60 ms VP9/WavPack acceptance videos,
  3–4096 channels, complete multi-block assembly, legacy/new speaker metadata,
  integer/float PCM, 40-bit counters, compact Matroska, reset/seek, worker capture
  and stereo routing. Malformed components and mapping witnesses check exact
  refusal and atomic layout/PCM publication. Generate separately with
  `scripts/generate_wavpack_multichannel.py`, without external codecs or network.

- `wavpack-custom-rate/`: 30 owned mono/stereo/six-channel VP9/WavPack videos
  covering three-/four-byte custom rates and reserved-bit semantics, exact PCM,
  timebases, reset/rewind and reader presentation; five specific malformed-rate
  videos. Generate separately with `scripts/generate_wavpack_custom_rate.py`,
  without external codecs or network.

- `wavpack-hybrid/`: 105 owned 60 ms VP9/WavPack main-stream videos covering
  error-budget quantization, bitrate/deltas/balance, zero-error codes, integer
  fixup/clipping, zero runs, multichannel assembly, seek and worker capture.
  Five specific malformed/unsupported witnesses. Generate separately with
  `scripts/generate_wavpack_hybrid.py`; ordinary tests need no external codec.
  An optional independent reference harness lives under `benches/`.

## Grouped AVI ADPCM samples

`avi-adpcm-grouped` authors14 mono/stereo IMA/MS cases with several fixed-size
samples in a chunk,including nested records and idx1. Sample size and block
alignment are8 or16 bytes;dwLength counts three blocks. Four single-block
controls retain independent literal predictor PCM,including opposite-channel
stereo. Malformed partial blocks and wrong declared counts remain explicit
refusals. Logical packet expansion respects the reader's record budget.

Each output packet carries one independent block and its sample clock. Seek
keeps the existing block-rounded landing contract,including requests inside a
block;the tests separately cover rewind and EOF. Owned AVI video and a separate
Matroska companion contain the same authored VP9 control. Generation is explicit
and offline;ordinary tests use saved bytes without FFmpeg or network access.

- `wavpack-correction/`: 306 owned VP9/WavPack hybrid/correction pairs with
  lossy and lossless scalar oracles, all integer widths, shaping, predictor
  histories, CROSS_DECORR, multichannel, standard Matroska BlockAdditional,
  exact seek and worker capture. Includes malformed/atomicity controls and an
  explicit laced-correction refusal. Generate separately with
  `scripts/generate_wavpack_correction.py`; no external codec or network required.

- `wavpack-dsd/`: 256 owned VP9/WavPack videos covering raw/fast/high DSD,
  probability table forms, mono/stereo/false stereo/six channels, exact native
  bytes and scalar PCM, small-block FIR startup, replay-based exact seek and
  audio-worker windows. Fourteen malformed witnesses preserve specific errors;
  failed final components cannot advance filter/layout state. Generate separately
  with `scripts/generate_wavpack_dsd.py`; ordinary tests need no external codec.

- `wavpack-float-hybrid/`: 216 paired/main-only hybrid float videos covering
  WVX forms, exponent/shift/overshoot behavior, exact IEEE words, Matroska, seek
  and finite PCM worker windows. Includes WVX bound/CRC controls and legal
  Xiph/fixed/EBML main-stream lacing with a foreign additional ID. Generate
  separately with `scripts/generate_wavpack_float_hybrid.py`, without external codecs.

- `wavpack-files/`: 1,317 native `.wv` cases with optional `.wvc` companions,
  scalar PCM oracles and associated synthetic codec-control videos. Covers
  incremental opening/indexing, correction pairing, sample-clock seek/rewind,
  unknown totals, tags, malformed files and audio-worker playback. Generate
  separately with `scripts/generate_wavpack_files.py`; no external codec or
  network is used. See its README for the native/control-video distinction.

- `eac3-bsid/`: 36 owned 96 ms VP9/E-AC-3 videos and elementary controls
  covering backward-compatible BSID 11..16 across long/short/mixed transform
  windows and spectral extension. Exact version-16 PCM equivalence, rewind and
  reserved/future-ID rejection are tested offline in `tests/eac3_bsid.rs`.
  Generate separately with `scripts/generate_eac3_bsid.py`; no FFmpeg is used.

- `eac3-converted/`: 66 owned short VP9/E-AC-3 type-2 videos with matching
  independent-stream controls, covering 1/2/3/6 blocks, compatible IDs, transform
  windows, mixing metadata and first/continuation forms. `tests/eac3_converted.rs`
  checks exact PCM, duration, rewind and malformed original-frame-size rejection.
  Generate separately with `scripts/generate_eac3_converted.py`; no FFmpeg is used.

- `eac3-aht/`: eight owned scalar-AHT cases with independent controls and short
  VP9/Matroska videos (six active transforms, two all-plane declines). Exact PCM,
  plane flag parsing, duration and rewind are tested offline. Separate codec units
  cover normative VQ words and GAQ reconstruction; their full playback matrix is
  still open. Generate with `scripts/generate_eac3_aht.py`, without FFmpeg.

- `eac3-aht-ac/`: 36 owned nonzero AHT AC basis/mixture videos and conventional
  controls. Checks temporal reconstruction, raw/Matroska PCM, duration, rewind,
  and audio-worker seek. Exact representable spectra require byte equality;
  remaining controls use an analytic quantization-error bound. Generate separately
  with `scripts/generate_eac3_aht_ac.py`, without external codecs or network.

- `eac3-aht-quantizers/`: 400 owned VQ/GAQ AHT video/native cases and analytic
  conventional controls. Covers hebap 1..19, four GAQ modes and all 956 normative
  VQ indices; includes saturation and matched headroom inputs. Checks PCM,
  raw/Matroska duration, rewind and audio-worker partial seek. Generate separately
  with `scripts/generate_eac3_aht_quantizers.py`; no external codec/network.
  See its README for explicit reference-policy and VQ4 discrepancies.

- `eac3-aht-lfe/`: 48 owned stereo+LFE cases and conventional controls in
  raw, VP9/Matroska and VP9/MP4 forms. Checks nonzero LFE PCM, FL/FR/LFE
  assignment through the audio backend, stereo routing, rewind and partial seek.
  Generate separately with `scripts/generate_eac3_aht_lfe.py`, without FFmpeg
  or network. See its README for numeric bounds and qualification limits.

- `eac3-aht-coupling/`: 192 owned coupled stereo cases in raw and short
  VP9/Matroska form. Conventional controls resend equivalent coordinates while
  test streams reuse them. Checks ordinary/scalar-AHT coupling, phase retention,
  PCM, duration, rewind and partial seek. Generate separately with
  `scripts/generate_eac3_aht_coupling.py`; no external codec or network.

- `eac3-enhanced-coupling/`: 432 owned enhanced-coupling stereo cases and
  uncoupled scalar controls, elementary and short VP9/Matroska versions. Covers
  all amplitude codes, low six-bin bands, merged higher bands and scalar/AHT
  playback, rewind and partial seek. Generate separately with
  `scripts/generate_eac3_enhanced_coupling.py`. See its README for qualification
  limits and the explicitly non-green optional reference-control comparison.

- `eac3-coupling-updates/`: 448 owned cases for amplitude updates, participant
  changes, geometry changes and ordinary/enhanced tool transitions across
  layouts 2..7, native/converted streams and LFE. Uncoupled controls check actual
  PCM; raw/Matroska, duration, masks, rewind and partial seek are checked offline.
  Generate separately with `scripts/generate_eac3_coupling_updates.py`.

- `eac3-aht-updates/`: 356 owned AHT/coupling-update combinations across
  layouts 2..7 and LFE. Eligible planes use six-block AHT; uncoupled scalar
  controls use equivalent three-block frames. Checks PCM, plane flags,
  raw/Matroska clocks, masks, rewind and partial seek. Generate separately with
  `scripts/generate_eac3_aht_updates.py`; ordinary tests need no external codec.

- `eac3-spx-noise/`: 15 owned noisy/signal-only SPX cases with three attenuation
  variants. Reproduces the separate SPX random-generator reset defect and checks
  raw/Matroska rewind, normal frame continuity and audio-worker partial seek.
  Generate separately with `scripts/generate_eac3_spx_noise.py`, without FFmpeg
  or network.

- `eac3-zero-bit/`: 112 zero-bit dither cases and 16 coded-signal controls,
  conventional/AHT and native/converted. Checks per-block/channel silence with
  overlap, re-enabling, scaling, stereo independence, raw/Matroska rewind and
  worker seek. Generate separately with `scripts/generate_eac3_zero_bit.py`.
  See its README for the explicitly failing optional FFmpeg silence audit.

- `eac3-aht-mixed-zero/`: 272 owned delta-allocation cases with zero/VQ/GAQ
  mixtures, four gain modes and native/converted streams. Checks actual decoded
  allocation arrays, nonzero PCM against scalar controls, raw/Matroska clocks,
  rewind and partial seek. Generate separately with
  `scripts/generate_eac3_aht_mixed_zero.py`, without external codecs/network.

- `eac3-coupling-zero/`: 448 owned ordinary-coupling zero-bit cases,
  all AHT masks, native/converted, phase flags and per-channel/block dither.
  Checks coupled spectra, silence, independence, exponent scaling, rewind
  and worker partial seek; regenerate with `scripts/generate_eac3_coupling_zero.py`.

- `eac3-enhanced-noise/`: 224 owned enhanced-coupling dither cases with
  conventional/AHT, all 32 amplitude codes and unity-amplitude companions.
  Checks coupled spectra, silence, scaling, stereo independence, raw/Matroska
  rewind and worker seek; regenerate with `scripts/generate_eac3_enhanced_noise.py`.

- `eac3-lfe-zero/`: 96 owned conventional/AHT stereo+LFE cases with zero
  allocation and coded controls, raw/Matroska/MP4 readers, rewind and worker
  seek. Regenerate with `scripts/generate_eac3_lfe_zero.py`; see its README
  for the separate, explicitly failing FFmpeg LFE-silence comparison.

- `eac3-substreams/`: 88 owned multiplexed base-programme cases with
  extra independent/dependent frames, exact PCM/PTS, sample counts, rewind,
  worker seek and malformed length guards. Regenerate separately with
  `scripts/generate_eac3_substreams.py`. Extension-channel reconstruction remains open.

- `eac3-dependent/`: 496 owned low-level dependent-channel cases spanning all
  acmod/LFE layouts, coded PCM order, AHT masks and all channel-map locations,
  plus malformed-map raw/video guards. Generate with
  `scripts/generate_eac3_dependent.py`. Fixed-layout assembly is qualified separately by `eac3-presentation/`.

- `eac3-dynrng/`: all 256 normal DRC codes with signed-exponent and block-reuse
  PCM acceptance, raw and synthetic video; regenerate with
  `scripts/generate_eac3_dynrng.py` without external codecs.
- `eac3-presentation/`: 27 owned fixed-layout 7.1/10/16-channel assembly cases,
  replacement, common normal DRC, final clipping, separate-syncframe Matroska,
  rational clock and worker seeks. Generate with
  `scripts/generate_eac3_presentation.py`. Extended device mapping and full
  codec/profile coverage remain open.

- `eac3-transient-native/`: owned sounding stereo, dual-mono and mono+LFE
  transient streams, matched metadata-free controls, neutral VP9 Matroska videos
  and deliberately damaged-CRC streams. Generate separately with
  `scripts/generate_eac3_transient_native.py` (no external codecs/network).
  Decoder-unit tests check native reconstruction equivalence, active/inactive
  planes, per-block transient windows and gains, correction through the aligned
  adapter, reset and whole-frame CRC concealment. These are native reconstruction
  and adapter tests; delayed player packet/seek acceptance remains pending.
  The same folder now also includes unity-gain stereo player/control videos.
  `tests/eac3_transient_playback.rs` accepts default native transient playback,
  original packet clocks, EOF, rewind and worker partial-frame seek. Expanded
  presentation transient qualification is recorded separately in
  `eac3-transient-presentation/manifest.json`.


- `eac3-transient-presentation/`: 25 owned expanded transient cases for 7.1,
  replacement, 10-channel and 16-role PCM at 48/44.1 kHz, with unity/varying common
  DRC and real final clipping. Includes late windows correcting the prior
  programme and a temporarily absent replacement. Independent component controls
  retain headroom before the whole-buffer correction/role/DRC oracle. Generate
  separately with `scripts/generate_eac3_transient_presentation.py` without
  external codecs/network. Tests exercise direct/root dispatch, exact source
  packet grouping, EOF/reset, split Matroska and worker rewind/seek. Sixteen-role
  qualification is PCM-level; it does not certify an extended hardware layout.
  Fixed mapped configurations additionally have six-block MP4 samples and
  six-programme control oracles. Eighteen cases cover active transient PCM/PTS,
  EOF, rewind and worker seek in MP4; the changing-count and 16-role cases are
  explicitly outside this MP4 subset.

- `eac3-lfe-downmix`: 272 accepted owned E-AC-3 cases, all eight acmod
  layouts, all 32 LFE mixing levels, disabled mixing and per-frame changes.
  Explicit mono/stereo decoder output is compared with separately decoded
  native-channel controls; native Matroska PCM retains its separate LFE.
  Reset replays exactly. Includes the single-surround folding regression,
  which previously omitted S from right stereo and halved its mono weight.
  Generate separately with `scripts/generate_eac3_lfe_downmix.py` without
  external codecs/network. These fixtures qualify the explicit codec API,
  not automatic physical-device downmix negotiation in the player.

- `eac3-changing-roles`: accepted changing custom speaker maps in dependent
  substreams, with a fixed rendered 7.1 layout. Two stereo dependents exchange
  front/rear roles and return, preserving their coded transform histories.
  Eight cases cover 48/44.1 kHz, three/six blocks and optional backward/future
  transient correction. Raw and split Matroska acceptance includes exact PCM,
  reset, rewind, PTS and worker seek. Four six-block cases also exercise MP4
  with the same continuous-history oracle and a seek after the map change.
  Generate separately with `scripts/generate_eac3_changing_roles.py`; 45 owned
  artifacts, no external codecs/network. Six-block base 5.1 planes deliberately
  use zero allocation to fit the coded frame budget; active dependent planes
  carry distinct PCM. Changes to rendered layout or coded channel count are
  outside this acceptance set.

- `eac3-changing-widths`: 32 accepted expanded E-AC-3 cases with a dependent
  stream shrinking to one coded channel and returning to two/five full-band
  channels or mono+LFE, while rendered output remains 7.1. Fixed-width native
  controls encode zero coefficients during absent-plane intervals; the oracle
  excludes their absent PCM and applies scalar transient correction before
  programme-role placement. This distinguishes resetting continuing channels
  from replaying stale overlap when another channel returns. Includes 48/44.1
  kHz, three/six blocks, optional transient correction, raw/split Matroska,
  reset/rewind, exact PTS, mask and worker seek. Twelve six-block cases also
  cover MP4 and seek after the width transition. Six-block base 5.1 uses zero
  allocation; active dependents carry the PCM witnesses. Generate separately
  with `scripts/generate_eac3_changing_widths.py` (173 owned artifacts).
  The final rendered layout stays fixed; device layout renegotiation is not
  claimed. Ordinary tests require no generator, FFmpeg or network.

- `eac3-expanded-dual-drc`: 80 accepted unequal normal-DRC witnesses with a
  dual-mono final dependent. Includes 7/8-channel output, Left/Right versus
  Left/Centre replacement, dual-mono independent/non-final components, block
  updates/hold, active final clipping, transient correction and dual/stereo
  transitions. Quiet native-component controls preserve headroom until role
  assembly and the last dependent's DRC pair are applied; second-programme
  ownership follows coded Ch2, not an assumed right-speaker index. Raw/reset,
  split Matroska/rewind/PTS/worker seek and 32 six-block MP4 variants are checked.
  Generate separately with `scripts/generate_eac3_expanded_dual_drc.py`, without
  external codecs/network. Six-block base 5.1 uses zero allocation while active
  dependents provide PCM; other cases carry active base audio. These are finite
  normal-DRC/PCM qualifications, not RF-compression or physical-device proof.

- `eac3-rf-compression`: 632 native E-AC-3 compression witnesses. All 256
  `compr`/`compr2` words are exercised in dual-mono/stereo; representative words
  cover all eight acmod values with/without LFE. Three-frame sequences contain
  present/absent/different words, independent dual fallback, normal-DRC updates
  and hold, plus active transient/clipping variants. Exact raw/reset and explicit
  container RF API PCM/PTS are compared with quiet native controls; default Line
  PCM remains unchanged. Includes mono folding of unequal dual gains and explicit
  C+LFE -> stereo RF folding with LFE injection disabled. Generator
  `scripts/generate_eac3_rf_compression.py`: 1285 artifacts / 24,286,843 bytes.
- `eac3-rf-expanded`: 80 expanded RF cases / 32 MP4 variants. Final-dependent
  RF words change across frames and the absent second word falls back to normal
  DRC. Includes dual/stereo transitions, replacement ownership, transient
  correction, active final clipping and 7/8-channel layouts. Direct raw/reset,
  explicit RF Matroska/MP4 construction, PTS, rewind and direct stream seek are
  tested. Compatibility output instead uses independent RF words; explicit Line
  output ignores the RF words. These tests do not configure a physical device
  or add a player RF selector. Generator `scripts/generate_eac3_rf_expanded.py`:
  433 artifacts / 14,949,960 bytes. Both generators are owned/offline; tests use
  fixture bytes without external encoders, FFmpeg, network or generation.

- `ac3-rf-compression`: 659 accepted AC-3 RF compression witnesses. Owned D45
  dual-mono/mono/stereo spectra cover 32/44.1/48 kHz, LFE, all 256 RF words in
  dual-mono/stereo and representative words elsewhere. Existing repository-owned
  coupled stereo and 5.1 tones are repacked with BSI words/frame padding; their
  raw Line PCM must remain byte-identical after the same priming removal.
  Both AC-3 CRC regions are independently repaired and verified by the generator.
  Present/absent/different compression words, channel-independent fallback,
  normal block updates/hold, native output, reset and clipping-after-mono-fold
  are compared with quiet controls. 163 MP4 variants also exercise placeholder
  geometry resolution, exact PTS, rewind and direct RF stream seek in MP4 and
  Matroska. Explicit typed RF construction is tested; a player RF control is not
  implied. `scripts/generate_ac3_rf_compression.py`: 1507 artifacts / 24,779,358
  bytes, regenerated without encoder, process or network access.


`dolby-player-compression` qualifies session Line/RF propagation through the
real audio worker, reset/replacement, seek and rewind for owned AC-3/E-AC-3
MP4/Matroska samples. Neutral PCM verifies that the Dolby setting is ignored.
Regenerate offline with `python3 scripts/generate_dolby_player_compression.py`;
ordinary tests consume checked-in bytes and require neither FFmpeg nor network.


`ac3-dialogue-normalization` covers explicit AC-3 decoder dialogue-level
normalization: all 32 codes, independent dual programmes, mono/stereo/LFE,
three rates, frame metadata changes, reset/rewind, MP4/Matroska and Line/RF.
Loud RF cases require scaling before clipping/folding, using quiet controls
with identical mantissas. Regenerate with
`python3 scripts/generate_ac3_dialogue_normalization.py`; no external encoder,
FFmpeg, private source media or network is used. The acceptance test is
`tests/ac3_dialogue_normalization.rs`.


`eac3-dialogue-normalization` qualifies native BSI dialogue normalization
against quiet unclipped controls: all 32 values, independent dual programmes,
all ordinary channel layouts/LFE, frame updates, DRC/RF, transient correction,
clipping, same-decoder reset and exact seek. `eac3-dialogue-expanded` adds
7/8-channel replacement and role changes, common final-dependent DRC, direct
presentation API, independent compatibility output and Matroska/MP4 seek.
Regenerate with `scripts/generate_eac3_dialogue_normalization.py` and
`scripts/generate_eac3_dialogue_expanded.py`; authoring is offline, using only
owned synthetic repository media. Their respective integration test targets
are `eac3_dialogue_normalization` and `eac3_dialogue_expanded`.


`dolby-player-normalization` verifies session dialogue-normalization policy
with the real audio worker: AC-3/E-AC-3, dual-mono/LFE, loud/expanded samples,
Matroska/MP4, both Line/RF choices, startup/reset/replacement/rewind/seek and
neutral PCM. Exact references use typed decoder builders independently of
the worker's trait hooks. Regenerate offline using
`python3 scripts/generate_dolby_player_normalization.py`; run the
`dolby_player_normalization` integration target against checked-in bytes.


`eac3-programme-selection` qualifies explicit decoder selection of all eight
independent programme IDs, with distinct waveform controls, native layouts/LFE,
32/44.1/48 kHz, expanded dependent assembly, dual/stereo transitions, transient
history, Line/RF, exact PCM/PTS, reset/rewind and switching after EOF. It also
verifies that unselected-only input and damaged unselected payloads do not
affect selected history. This is decoder API acceptance, not simultaneous
programme mixing or player/elementary-reader selection integration.
Regenerate offline with `python3 scripts/generate_eac3_programme_selection.py`;
ordinary tests run `eac3_programme_selection` against checked-in bytes.


`eac3-programme-reader` qualifies elementary-reader/player-factory routing of
independent programmes. It contains eight short owned multiplexed movies,
standalone controls, heterogeneous geometry and authored negative inputs for
damaged unselected BSI, missing IDs and truncated tails. Tests compare selected
geometry, speaker masks, PCM/PTS, catalogue positions, packet ownership, seek,
rewind and worker output. Regenerate offline with
`python3 scripts/generate_eac3_programme_reader.py`; run integration target
`eac3_programme_reader` and the headless player factory regression.


`eac3-matroska-programmes` qualifies explicit Matroska reader selection of
all eight encoded independent programmes, including heterogeneous geometry,
expanded/transient assembly, individual syncframe blocks and placeholder
container rate. It compares exact PCM/PTS, masks, duration, seek, rewind and
worker capture to standalone controls, with separate missing/corrupt/wrong
selection tests. The API's physical-track index stays zero-based. Regenerate
with `python3 scripts/generate_eac3_matroska_programmes.py`; ordinary tests
run `eac3_matroska_programmes` against checked-in bytes offline.


`eac3-mp4-programmes` checks explicit independent-programme selection within
a physical MP4 audio track, with geometry, exact PCM/PTS, edit ranges, seek,
rewind and worker policy controls. Regenerate with
`python3 scripts/generate_eac3_mp4_programmes.py`; run `eac3_mp4_programmes`.
Its repaired multiple-descriptions movie uses three real chunks and descriptions
1 and 2 with equivalent setups. The previously ignored exact PCM acceptance is
now active. The earlier single-offset fixture was malformed beyond the old
one-entry refusal; it must not be used as a valid second-description witness.

`seek-ready-pending-spool.mkv` is an owned 320 ms, eight-frame VP9 video.
Regenerate with `python3 scripts/generate_seek_ready_pending_spool.py`. The
player unit test freezes reported future spool lead and injects a failing
audio worker, then verifies both requested seek pictures are presented while
read-ahead stays pending. The original code times out with the target frame
already queued. Ordinary tests use the checked-in movie offline.


`mp4-sample-descriptions` qualifies description indexing and equivalent-entry
VP9 playback in ordinary/fragmented MP4. It also contains a real 16→32→16
geometry change, independent one-entry decode controls, compact PCM chunks,
invalid selected indices and an external second reference. The changing video
is indexed through `open_indexed` and now plays through the native video
reader with seek/rewind acceptance enabled. Single-configuration export/remux consumers retain their explicit
reconfiguration refusal. Native audio now negotiates changing setups including
variable-rate edits; the finite PCM/E-AC-3 scheduler matrix is described below. Regenerate with
`python3 scripts/generate_mp4_sample_descriptions.py`; ordinary offline tests
run `mp4_sample_descriptions`. No external encoder or private media is used.


`mp4-video-reconfiguration` exercises actual reused-ID AVC setup changes,
AVC/VP9 coding switches, source precision, description 2 at startup, display
reordering and per-picture pasp/colour/HDR. Independent one-entry controls have
analytic constant-plane oracles. A constant-setup movie isolates the reordered
seek interval failure, and a shortened terminal configuration isolates stale
metadata immediately after rewind. Invalid late configuration tests error
latching and rewind recovery. Regenerate offline with
`python3 scripts/generate_mp4_video_reconfiguration.py`; run integration target
`mp4_video_reconfiguration`. The generator invokes no encoder/FFmpeg/network
and uses only owned bitstream writers and parameter sets.

The video-reconfiguration matrix also includes constant stereo PCM beside the
changing video. A real captured audio worker must preserve exact float PCM and
rewind, independently of the video description changes. An unchanged selected
audio track is accepted; changing selected PCM/E-AC-3 setups now have separate native reconfiguration
acceptance. Changing setups with edits retain an explicit refusal.


`mp4-audio-reconfiguration` contains own PCM and E-AC-3 coding/rate/channel
changes beside short owned VP9 video. PCM has analytic float32 oracles;
compressed spans have independent constant-setup controls. Ordinary and iso5
fragmented PCM, setup 3 at startup, exact worker PCM, delayed decoder tails,
RF/dialogue policy, seek/rewind, output-drain cancellation and error recovery
are tested. The leading-silence variable-rate edit acceptance is enabled. Regenerate with
`python3 scripts/generate_mp4_audio_reconfiguration.py`; run
`mp4_audio_reconfiguration` offline without FFmpeg/network.

`mp4-variable-rate-edits` adds owned PCM/E-AC-3 edit schedules beside synthetic
VP9: silence, repeated and disjoint source windows, ordinary/fragmented MP4,
reordered description tables, fractional movie clocks, negative source PTS and
repeated delayed E-AC-3 tails at source EOF. Explicit independent source-frame
cuts and rational timestamps check exact PCM and seek suffixes, including real
audio workers and rewind. A separate past-end witness must emit the available
PCM then fail for missing source extent. Regenerate separately with
`python3 scripts/generate_mp4_variable_rate_edits.py`; run
`mp4_variable_rate_edits` offline. The generator uses no FFmpeg or network.
This finite PCM/E-AC-3 matrix does not qualify every compressed codec/edit pair
or prove acoustic continuity during physical output-device reconfiguration.

The variable-rate edit matrix also includes ALAC raw and first-order Rice
configurations at 48 kHz stereo 16-bit, 32 kHz mono 16-bit, and 44.1 kHz stereo
24-bit. Eight ALAC movies preserve the analytic PCM oracle across repeated,
disjoint, fractional and negative-PTS schedules. Their MP4 offsets and cookies
are authored by `generate_mp4_alac_transitions.py`, invoked only by the fixture
generator. Every matrix movie additionally decodes its exact neutral VP9
companion. This qualifies these ALAC transitions, not every predictor/profile
or mixed-codec transition.

`mp4-mace-transitions` contains owned arithmetic MACE blocks and neutral VP9,
with QuickTime tables counting decoded samples rather than encoded packets.
MAC3/8 kHz/mono, MAC6/16 kHz/stereo and MAC3/8 kHz/stereo switch with different
block widths and timing, then return to the first setup. Constant-description
controls independently establish the predictor-reset baseline. Reordered setup
tables, negative PTS, repeated edits with silence, worker seek/rewind, exact index
geometry, video companions, index budget and corrupt chunk/composition tables
are covered. The pre-fix witness refused specifically for per-description MACE
block indexing. Regenerate separately with
`python3 scripts/generate_mp4_mace_transitions.py`; run `mp4_mace_transitions`
offline. Mixed MACE/PCM sample units and fragmented/hybrid changes are covered by the
extension below; this fixture matrix does not prove all MACE container variants.

The MACE matrix now also covers pure fragmented and hybrid initial-table plus
fragment input, reordered configurations, mixed MACE/PCM units, and repeated
edits on both packet-index forms. Fragment runs count encoded packets; initial
QuickTime MACE tables count decoded samples. Each selected description determines
its own table-to-packet unit. The independent controls check PCM/predictor
continuity; real workers check seek suffixes and rewind. A mixed-table target
checks 400 indexed packets from 1200 table units and exact per-span durations,
widths and budget enforcement. The previous-source replay fails on the specific
mixed-unit refusal before the acceptance fix. Broader AAC/ADPCM/other compressed
mixed-codec matrices remain unqualified.

`mp4-aac-transitions` authors AAC pulse spectra and independent scalar
IMDCT/overlap PCM: LC/1024 stereo 48 kHz, Main/960 mono 32 kHz, LC/1024 stereo
44.1 kHz, then the first configuration again. Ordinary/reordered/fragmented
MP4, identical duplicate setup entries, repeats with silence, explicit fractional
sample cuts and disjoint startup inside Main/960 check exact control PCM and
seek suffixes through actual audio workers. Separate analytic control checks
require nonzero output and numerical agreement. Every case retains exact owned
VP9 planes/clocks. The preroll regression first reproduced reading PTS 0 for a
seek into Main/960; acceptance requires the last actual setup boundary while
preserving history across equivalent entries. Generate separately with
`python3 scripts/generate_mp4_aac_transitions.py`; run `mp4_aac_transitions`
offline. These pulse streams do not exercise every Main predictor, HE/SBR/PS,
LTP, SSR, ER or multichannel transition combination.

`mp4-adpcm-transitions` authors analytic IMA4 ramps, IMA WAV opposite-channel
predictor plateaus and MS ADPCM predictor plateaus beside neutral VP9. Packets
contain one or multiple blocks; coding, channels and sample frequency change
and return. MP4 track clocks 2000/48000/96000 differ from native PCM clocks.
Low-clock input previously truncated a 64-frame packet to 16 frames; intentional
final-block trims also require conversion in high clocks. Constant controls,
regular/reordered/fragmented tracks, silence/repeated edits, seek/rewind and
actual audio workers compare exact independent PCM. Video planes and clocks
are checked for every case. Generate separately with
`python3 scripts/generate_mp4_adpcm_transitions.py`; run `mp4_adpcm_transitions`
offline. Decoder default duration units remain native PCM frames for callers
that do not set a source timescale; zero duration retains the available block.
This matrix does not qualify every ADPCM container, geometry or mixed codec.

ADPCM constant controls also test general owned-export admission, actual codec
names and byte-exact full/3--14 ms interval PCM export. Before the fix the root
export API refused ADPCM despite the existing owned lower-level decoders.
Changing-configuration export remains outside this finite acceptance matrix.

`mp4_mace_export` extends the owned MACE witnesses with MAC6 mono and constant
MAC3/MAC6 mono/stereo edit lists: 3 ms silence, source 4--12 ms, 2 ms silence,
then the same source window. General FVid export previously refused MACE; plain
constant-setup player seeks lost predictor history and constant edit lists were
ignored (30 ms source instead of 21 ms presentation). Acceptance checks full and
5--14 ms interval exports, codec admission/labels, direct seek suffixes, real
worker packet PCM/times, rewind, and exact owned VP9 companions. A fresh streaming
byte-PCM decoder provides presentation/state references; the existing static
MACE numeric/reference suite independently guards the unchanged predictor math.
Regenerate with `python3 scripts/generate_mp4_mace_transitions.py`; run
`mp4_mace_export` with player for all checks or without player for headless
export acceptance. Fixture generation stays separate and invokes no FFmpeg or
network. Changing-configuration and separate lower-level media-export acceptance
remain outside this finite matrix.

The same MACE target now verifies `fvid-media` directly: native decode, full and
interval PCM, plan codec names, WAVE publication and controlled-memory refusal
before output. The grouped MAC6 mono companion places 1+7+32 blocks in three
fragment packets, preserving the original 40 arithmetic-coded bytes and neutral
VP9. Its edited companion checks reset/preroll across repeated source ranges.
The float/byte public engines are compared across both codings, mono/stereo,
packet cuts and malformed-block transactions. Predictor tables/reconstruction
now share one Rust implementation between FVid and fvid-media. Existing static
codebook references also run inside fvid-media; no FFmpeg is invoked by tests.
Changing-description export remains unqualified.


The MP4 sample-description generator also writes equivalent-pcm.mp4 and
equivalent-aac.mp4 with equivalent audio entries selected 1/2/1, plus independent
f32le waveform oracles. The PCM witness splits three chunks using packet sizes.
`mp4_media_sample_descriptions` accepts their owned decode, checks indexed
packet routing across native codecs and rejects malformed description indices.
Its changing-configuration fixed-reader/remux test is a refusal safety test,
not acceptance of changing-configuration export. Generate fixtures separately
with scripts/generate_mp4_sample_descriptions.py; ordinary tests never invoke
the generator, FFmpeg or network.


`mp4-audio-export-transitions/` contains owned neutral VP9 videos with audio
configuration changes at a common output rate/layout. Generate separately with
`scripts/generate_mp4_audio_export_transitions.py`. AAC LC/Main switches between
960/1024 samples; delayed SSR crosses long-start/short/long-stop boundaries
before a codec-configuration switch and EOF. Scalar IMDCT/overlap and literal
SSR convolution produce the independent PCM oracles. PCM switches 16-bit,
float32 and 24-bit representations with arithmetic sample oracles. Regular,
fragmented and first-used-entry-3 variants, repeated edits, interval export,
float64 precision and WAVE publication run in `mp4_audio_export_transitions`.
The fixtures regenerate byte-exactly without external processes or network
(23 artifacts / 1605797 bytes). No private media or parameter sets are copied.
Rate/channel-change refusal is a safety assertion, not normalization acceptance.
