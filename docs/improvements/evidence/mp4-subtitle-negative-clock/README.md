# Signed MP4 subtitle spans before clipping and rounding

Six authored VP9/tx3g videos use50Hz or1001Hz subtitle clocks. Each has three real VP9 frames and three UTF-8 timed-text samples. Negative CTTS V1 puts one sample wholly before zero and another across zero; zero/positive unsigned CTTS controls retain ordinary timing. Independent integer-rational oracle endpoints are computed from PTS+duration before clamping to the visible time domain.

Original73ec5a006097d9f48a04474460ceeba51ceb178f passes source packet/video validation and fails cue-span and actual video-seek/active-cue validation. Fully negative text appears atzero, a crossing cue stays too long, and separate rounding loses a nanosecond on positive1001Hz spans. The corrected baseline confirms these precise errors in all relevant cases. An earlier baseline had a harness error:seek_raw already returns the sought frame. The corrected test checks that frame directly; the discarded run and hashes are retained. Subtitle track headers also reflect rounded presentation endpoints.

The fix composes explicit cue endpoints in signed wide arithmetic, then clips and converts to Duration. Wholly invisible/zero-representable spans are omitted. Unknown zero-duration cues continue through the established successor/fallback policy. Text/style parsing and cue ordering are preserved. No public signature changes.

Fixture generation is offline and separate from ordinary tests, with no FFmpeg, network or private media/codec parameters. Exact source PTS/durations, packet text and video planes are checked before cue acceptance. The integration also seeks real video frames and checks active subtitle content at exact start/end boundaries.

Qualification passes418 distinct checks:321 in64 integration suites and97 unique root MP4/subtitle units. Six overlapping unit checks are excluded from the count; two physical-hardware checks are explicitly ignored. Locked/offline core/player builds pass, including native subtitle paint/seek/end checks. FVID-0422 counts this one endpoint-composition problem. This is finite subsystem validation, not a whole-project or physical-device result.
