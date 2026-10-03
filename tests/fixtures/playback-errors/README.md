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
