# MP4 edit clock integration (FVID-0403/FVID-0404)

A complete fragmented MP4 may state a single rate-one media edit with duration
zero: it covers subsequent indexed media. Readers retain the raw zero marker.
No existing field or function signature was removed. Existing finite edits keep
their movie clock and previous behavior. Code consuming newly accepted open
edits must use the resolved view instead of summing raw edit durations:

```rust
let clock = track.resolved_edit_clock(reader.movie_timescale())?;
let windows = fvid_media::owned_video_timeline::map_edits(
    clock.edits().iter().map(|edit| (edit.duration, edit.media_time)),
    track.timescale,
    clock.timescale,
)?;
```

For one open edit the view stores a resolved Edit on the stack and uses the
media clock. Its end is max(sample.pts + sample.duration), not decode duration
or a rounded movie duration. The original Track is unchanged. Resolve once at
construction and retain the resulting timeline, rather than rescanning an
expanded index for every decoded packet. A uniform index resolves in constant
space/time; no additional edit buffer is allocated.

Updated consumers: native video, owned video decode, probe, shader export window,
AAC packet remux, Matroska video remux, audio export schedules/normalization,
NVDEC CPU movie planning, and unknown-movie chapter bounds. Native audio open
edits use the existing rational variable-edit engine. The same engine now handles
finite PCM/ALAC edits even with a constant description, preserving sample phase
at fractional boundaries. Existing raw packet clocks remain media coordinates;
AudioStream emits presentation-relative PCM packets.

Audited unchanged consumers: native export and mix packet-count/eligibility
heuristics and multitrack eligibility inspect only emptiness/number/type of
edits, then invoke the updated timing owners. Container-only test helpers that
clear edits do not consume duration. Leading empty native video edits retain
first-picture normalization. Generic map_edits without a source endpoint still
rejects an unresolved duration zero; the new Track view supplies that context.

Validation: 46 no-player tests, 48 player tests, five CPU NVDEC-source/schedule
tests and four timeline unit tests pass, zero failed/ignored in those selections.
The player configuration builds. Tests include exact RGB/PCM, coarse and
fractional clocks, source-budget refusals, public plans and WAVE publication,
AAC packet remux, seek, rewind and repeated EOF. Full-project suite success and
NVIDIA hardware execution are not claimed. Evidence is in
`evidence/mp4-open-ended-edit` and `evidence/mp4-fixed-audio-edits`.

The before snapshot has independent build output. An initial reused-target
comparison exposed a stale library artifact and was discarded; changed
packages were cleaned and both states revalidated with separate target roots.
Neither that infrastructure correction nor fixture variants contribute IDs.
