# MP4 unspecified subtitle endpoints on a signed source clock

A final decode sample may declare zero duration, while signed CTTS places it
before other presentation samples or before the video origin. Its display end
must be inferred from the next presentation timestamp, or from the established
two-second fallback at its original source timestamp. The old no-edit path first
clipped the start and successor to zero; the edited path also anchored the last
fallback at max(source start,0). Both could display invisible old text, obscure
a later caption, or extend a crossing caption beyond its intended end.

The original source is commit09e80937ca981546fbc6fecb84a45db2e61f5eb0.
`before-complete.log` passes the two packet/video/refusal controls and fails the
two exact cue/real-video-seek tests. At50Hz the unspecified sample at-3 ticks
with a successor at-2 incorrectly appears from0 to2seconds. A final sample at-1
incorrectly ends at2seconds instead of1.98seconds. A fallback wholly before zero
also incorrectly appears for2seconds. The failures are content/timing failures,
not parser rejection or a decoder error. Both50Hz and1001Hz are covered.

The initial authoring experiment had multiple zero-duration decode samples and
was correctly refused. `initial-fixture-refusal.log` and its source hashes retain
that distinct result; it is not the defect and is not counted. Final movies
contain one terminal zero-duration decode sample, two positive-duration controls,
valid visible tx3g/ftab entries and independently authored16x16 VP9 keyframes.

## Change and scope

Infer the successor or fallback endpoint in signed i128 source ticks once,
then pass both endpoints through the existing unedited or edit-program mapping.
Positive fallback behavior, explicit durations, finite edits, native rounding,
packet/cue limits and public signatures are preserved. This is specifically the
unspecified-duration inference case that FVID-0422 explicitly left unchanged;
that earlier entry covered stated-duration endpoints. All variants here represent
one issue, not a new count per clock, branch or fixture.

## Reproduction and checks

`python3 scripts/generate_mp4_subtitle_inferred_clock.py` generates the16 short
movies, three independent decoded-picture controls and manifest. Generation is
separate, standard Python only, with no FFmpeg, network or private source input.
The checked-in files are sufficient for ordinary tests:

```sh
cargo test --locked --offline --no-default-features --features player --test mp4_subtitle_inferred_clock
```

The new suite checks both parsers' exact signed timestamps, durations and packet
bytes, every decoded video picture and EOF/restart, exact nanosecond cue intervals,
repeated extraction, active cues after real native-video seeks, and independent
packet-budget/missing-track refusal controls. Positive cases and identity edits
provide controls. Related MP4 clock, seek, edit, text/Unicode and subtitle-unit
suites are recorded verbatim in `qualification-results.json` and logs. Ordinary
tests additionally execute with PATH excluding FFmpeg. Regeneration is compared
byte for byte. No changed test expectations, performance, physical-device or
visible-player rendering claim is made.

## Accepted qualification

Six gates pass83 Rust test executions,76 distinct checks after excluding seven
overlapping root checks,and zero ignores. Core/player builds pass. Three repeats
pass four tests each;four tests also pass with FFmpeg absent from PATH. All20
fixture files regenerate identically and2778 frozen hashes remain unchanged.
This accepts one ID,FVID-0519,batch530,total519/1000.
