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
