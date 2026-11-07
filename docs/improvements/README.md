# Verified improvements

Target: **1000 independent, useful changes**. The current accepted count is recorded in
[`registry.json`](registry.json); only entries marked `accepted` count. Fixture variants, test cases, documentation
and baseline integration repairs do not inflate the count.

## Batch 001 — correctness and interrupted I/O

| ID | Change | Regression |
|---|---|---|
| FVID-0001 | Horizontal row copy validates both slices and checked geometry before writing anything | `fvid-cpu/tests/hflip_contract.rs` |
| FVID-0002 | Shared I/O error exposes its typed source without changing display text | `fvid-control/tests/error_contract.rs` |
| FVID-0003 | EBML forward skips retry interrupted reads without losing their position | `tests/ebml_io_regressions.rs` |
| FVID-0004 | Subtitle-only transcoding selects a transcode execution plan | `tests/edit_contract.rs` |
| FVID-0005 | Crop constructor rejects overflowing right/bottom edges | `tests/edit_contract.rs` |
| FVID-0006 | Y4M line parsing retries interruption without clearing the partially read header/marker | `tests/y4m_io_regressions.rs` |
| FVID-0007 | Matroska identity copy retries interrupted final EOF verification | `tests/matroska_copy_io_regressions.rs` |

The three container regressions have separate short authored videos in
`tests/fixtures/playback-errors`. Controls prove valid video/container input;
fault readers inject one error at a known offset and separately check permanent
errors remain errors. Tests load saved bytes and need neither network nor FFmpeg.

Generate fixtures explicitly:

```sh
python3 scripts/generate_container_io_regressions.py
python3 -m unittest discover -s scripts -p test_container_io_regressions.py
```

Run focused checks:

```sh
cargo test --locked --offline -p fvid-control -p fvid-cpu
cargo test --locked --offline --no-default-features \
  --test ebml_io_regressions --test y4m_io_regressions \
  --test matroska_copy_io_regressions --test edit_contract
```

## Validation

[`batch-001-validation.json`](batch-001-validation.json) records 42 root tests,
23 control/CPU tests and 19 Python checks, all passing. The separate baseline
FFV1 qualification passed 16 tests and contributes no improvements to the count.
Every accepted bug had a failing pre-fix regression or explicit failing assertion
recorded in the registry.

## Baseline and limits

[`baseline-2026-10-14.json`](baseline-2026-10-14.json) records the initial HEAD,
dirty tracked paths and untracked count. The shared checkout was changing during
the audit; it is a timestamped observation, not an immutable source snapshot.

The pre-existing FFV1 Rice/v3/configuration work passed 16 focused tests and is
excluded from the new-change count. A concurrent `IntraPicture` chroma-depth
integration left four MP4 constructors uncompilable; completing those initializers
is also baseline repair, excluded from the count.

This batch makes no performance claim. Headless test playback does not establish
GUI, audio-device, hardware-decoder or platform-wide qualification. Unfinished
entries remain explicitly pending until their acceptance checks pass.

## Continuing

Use the next sequential ID for each newly confirmed independent problem. Record
the failing pre-fix observation, fix, regression and validation. Work in batches
of at most 20; complete the previous batch's acceptance checks before starting
the next. Preserve unrelated changes and qualify existing work separately.

After editing the ledger, run `python3 scripts/check_improvements_registry.py`.
This checks sequential IDs, the accepted count and evidence links; it never
accepts entries or counts accounting infrastructure as product improvements.

## Batch 002

- **FVID-0008** — Refuse duplicate HLS singleton declarations.
- **FVID-0009** — Refuse mixed HLS master/media tag classes.
- **FVID-0010** — Validate HLS numeric wire grammar.
- **FVID-0011** — Preserve cancellation during the final HLS output write.
- **FVID-0012** — Retry interrupted HTTP body reads.
- **FVID-0013** — Bound unknown-length HTTP body read-ahead at its byte budget.

Acceptance evidence: [`batch-002-validation.json`](batch-002-validation.json).

## Batch 003

- **FVID-0014** — Reject overflowing or malformed integer subtitle clocks.
- **FVID-0015** — Validate finite clip times and nonempty ranges.
- **FVID-0016** — Keep UTF-8 OSD prefixes on character boundaries.
- **FVID-0017** — Preserve quoted and at-sign clip paths.
- **FVID-0018** — Format minimum signed playback offsets safely.
- **FVID-0019** — Compute clock-sync midpoints without overflowing the difference.
- **FVID-0020** — Preserve proportional seek ratios before narrowing.
- **FVID-0021** — Apply bounded and wrapping control steps without overflow.
- **FVID-0022** — Bound negative audio-skew accounting in unsigned frame space.
- **FVID-0023** — Cycle tracks and devices without pre-modulo overflow.

Acceptance evidence: [`batch-003-validation.json`](batch-003-validation.json).

## Batch 004

- **FVID-0024** — Refuse a conflicting recurring segment clock without retiming the indexed prefix.
- **FVID-0025** — Honor Matroska track clocks with exact timestamp and duration conversion.

Acceptance evidence: [`batch-004-validation.json`](batch-004-validation.json).
52 distinct Rust regressions and 4 fixture-integrity checks passed. The exact-time
regression includes 1480 independent rational oracles. Synthetic VP9 playback
checks frames, seek, rewind and EOF; unchanged audio timelines and lacing also pass.
This qualifies software playback, with no GUI or hardware-decoder claim.

## Batch 005

- **FVID-0026** — Round declared Matroska durations exactly once and preserve representable boundaries.
- **FVID-0027** — Derive the final presentation interval independently of playback history.

Acceptance evidence: [`batch-005-validation.json`](batch-005-validation.json).
70 distinct Rust checks and 2 fixture-integrity checks passed; the player
configuration also type-checks. Native software playback covers seek, rewind,
EOF, explicit/default/unknown tails, nonzero origins and stable cache clocks.
Existing AV1 playback and HEVC reordered video/audio exports remain passing.

## Batch 006

- **FVID-0028** — Restore Matroska frame header stripping before decoding and lace timing.

Acceptance evidence: [`batch-006-validation.json`](batch-006-validation.json).
69 distinct Rust checks, fixture regeneration and player configuration checking
passed. Nine paired synthetic cases cover VP9 playback, whole/laced frames, late
tracks, exact Opus PCM, lossless FFV1 conversion and byte-identical copies.
At batch 006 acceptance, frame-only single-transform header stripping was supported; other compression,
encryption, codec-private transforms and transform chains remain unsupported.

## Batch 007 — empty projection defaults

FVID-0029 accepts legal zero-byte Matroska projection angles as their schema default zero. Four authored VP9 variants match the control through decoding, seek, rewind and EOF; malformed one-byte floats remain rejected. [Validation](batch-007-validation.json): 56 Rust tests and one Python integrity test passed; the player configuration checks successfully.

## Batch 008 — consistent recovery during FFV1 trimming

FVID-0030 preserves in-range frames when trimming irregular timestamp order that full export already accepts and sorts. The deliberately nonconforming input qualifies recovery behavior, not Matroska conformance. Exact interval boundaries, distinct decoded pixels, packet limits and failed-export publication checks pass. [Validation](batch-008-validation.json): 21 distinct Rust tests and one Python integrity test; player configuration check passed.

## Batch 009 — EBML semantic string termination

FVID-0031 stops strings at their first NUL before validating or interpreting them. DocType and CodecID padding no longer prevents playback; Unicode names, optional metadata and chapter titles discard obsolete suffix bytes. Strict prefix validation and stored-byte limits remain enforced. [Validation](batch-009-validation.json): 60 Rust tests and five Python fixture checks passed; player configuration check passed.

## Batch 010 — bounded Matroska zlib frames

FVID-0032 restores frame-only zlib compression before decoding. Eleven paired cases cover all three DEFLATE block types, defaults, lacing, late tracks, Opus and constant-resolution export. Native playback, exact PCM, seek/EOF, corruption refusal, byte limits, identity copy and atomic export checks pass. Controlled audio admission adds 10,596 estimated bytes for the synthetic compressed case, including 10,504 bytes of inflater state; this is not RSS measurement. [Validation](batch-010-validation.json): 80 distinct Rust tests and six Python checks passed; player configuration check passed.

## Batch 011 — MP4 recovery after a seek error

FVID-0033 prevents both MP4 readers from returning a different packet after a seek moves the cursor and then fails. Cached position becomes unknown before seeking; failed output is cleared and recovery resynchronizes correctly. The authored colored VP9 control passes sample, timestamp, seek, rewind and EOF checks. [Validation](batch-011-validation.json): 57 distinct Rust tests and one Python integrity test passed; player configuration check passed.

## Batch 012 — precise linker-setup inputs

FVID-0034 avoids rebuilding linker setup and both libraries after unrelated report/test-text changes. Two alternating warmed local checks measured 12.95/13.41 s before and 0.34/0.30 s after; actual script invocation counts fell from one to zero. This measures that incremental workflow, not application throughput or clean builds. [Measurements](build-watch-measurements.json), [replay harness](benchmark-build-watch.py), and [validation](batch-012-validation.json) include real Cargo dependency contracts, target-specific Windows output and both final configurations.

## Batch 013 — native RGB color pipeline

FVID-0035 connects five existing color kernels to native RGB export and permits color filtering of YUVA while keeping opacity untouched. Independent exposure pixels, identity precision, timeline selection, operator order, native playback, clocks, seek, rewind and EOF pass. [Validation](batch-013-validation.json): 517 Rust tests and one fixture integrity check passed; player configuration check passed. RGB colorhold at 10/12 bits remains explicitly unsupported.

## Batch 014 — contrast-adaptive RGB and opacity sharpening

FVID-0036 enables native CAS export for RGB and applies the separate opacity plane only when its mask selects it. Added smooth synthetic gradients prove actual RGB and opacity changes; zero masks preserve all samples. Channel masks, coded precision, native playback, clocks, seek, EOF and resource-limit refusals pass. [Validation](batch-014-validation.json): 32 Rust tests and one fixture integrity check passed; player configuration check passed.

## Batch 015 — Matroska audio defaults

FVID-0037 reads omitted and empty audio settings using the schema defaults, including inherited output frequency. Eight valid short video/PCM variants produce exact authored audio; explicit zeros remain invalid. Interval boundaries, decoder hard-stop prefixes, resource refusal, native video playback, clocks, seek, rewind and EOF pass. [Validation](batch-015-validation.json): 27 Rust tests and one fixture integrity check passed; player configuration check passed.

## Batch 016 — empty Matroska clock elements

FVID-0038 applies the segment/track clock schema defaults to empty elements. Four valid short clips preserve exact codec packets, timestamps, duration and native pixels; explicit-zero and malformed-float controls stay invalid. Seek, rewind, EOF and index resource limits pass. [Validation](batch-016-validation.json): 37 Rust tests and one fixture integrity check passed; player configuration check passed.

## Batch 017 — skip unselected sharpening planes

FVID-0039 avoids a full output allocation/copy when the requested CAS mask selects no available channel. Six alternating pairs measured the1080p8-bit mask-miss kernel at161.75µs before and7.65ns after; one6.22MB allocation disappears.4K and16-bit cases confirm the allocation saving;16-bit precision validation remains active. These are kernel measurements, not video throughput or RSS. [Measurements](cas-unselected-measurements.json), [replay](benchmark-cas-unselected.py), [validation](batch-017-validation.json):32 Rust tests and player build passed.

## Batch 018 — independent precision for MBAFF intra pictures

FVID-0040 enables different luma/chroma depths in native intra MBAFF decoding and uses each plane precision for loop filtering.128 authored40ms CAVLC/CABAC frame/field clips verify exact components, analytic RGB, clocks, seek, rewind, EOF and lossless FFV1 export. Mixed-depth inter MBAFF remains an explicit unsupported case. [Validation](batch-018-validation.json):59 Rust tests and one fixture integrity check passed; player configuration check passed. Temporary compiler-cache exhaustion was resolved without touching source work.

## Batch 019 — direct MBAFF PCM publication

FVID-0041 removes full-picture prediction views for independent PCM macroblocks. Six alternating pairs repeat exact allocator savings: a128-square frame drops from274 to18 allocations and3.32MB to0.11MB cumulative requested bytes; peak requested live increase falls from132KB to82KB. These are allocator metrics, not RSS or release throughput. Six size-series videos and32 neighbor variants preserve samples, clocks, seek, rewind, EOF and readiness atomicity. [Measurements](mbaff-pcm-memory-measurements.json), [probe](benchmark-mbaff-pcm-memory.rs), [replay](benchmark-mbaff-pcm-memory.py), [validation](batch-019-validation.json):41 distinct Rust tests, two Python checks and player build passed.

## Batch 020 — independent precision for MBAFF inter pictures

FVID-0042 connects mixed-depth P/B reconstruction, including unavailable references, residuals and component-specific deblocking QPs.164 MP4,66 Matroska and10 mixed-slice clips verify exact components, native playback, clocks, seek, rewind and EOF;44 edge-precision exports preserve lossless planes. This supersedes batch018’s inter limitation. [Validation](batch-020-validation.json):75 distinct Rust tests, two Python checks, player and minimal builds passed. Concurrent Opus integration repair is recorded separately and adds no ID.

## Batch 021 — owned AVC Matroska pipeline

FVID-0043 enables native AVC frame-picture decoding and lossless Matroska export using the existing codec.66 authored frame/MBAFF clips preserve exact promoted components and clocks; two missing-duration B-picture companions qualify duration inference. Trim/reverse, policy failure without output publication, and native playback/seek/EOF pass. Separate-field timing remains an explicit refusal. [Validation](batch-021-validation.json):25 Rust tests, five Python integrity checks and player build passed.

## Batch 022 — independent PAFF intra component precision

FVID-0044 reads each PCM plane at its declared precision and admits independent luma/chroma depths for intra fields.64 authored PCM/filtered-DC cases preserve woven components, analytic RGB, paired clocks, seek, rewind, EOF and exact lossless exports. Four truncated-chroma companions verify refusal before picture publication and reset recovery. [Validation](batch-022-validation.json):15 Rust tests, one Python integrity check and player build passed. Mixed-depth inter fields remain unsupported. Concurrent Opus documentation failures are recorded separately and add no count.

## Batch 023 — independent PAFF inter component precision

FVID-0045 propagates each plane precision through P/B references, interpolation, weights, residual reconstruction and filtering.160 authored CAVLC cases verify independent samples, RGB, paired clocks, seek, rewind, EOF and exact flipped lossless exports, including residual/reference echo and reordered B pictures. This supersedes batch022’s mixed-depth inter limitation for the qualified CAVLC cases. [Validation](batch-023-validation.json):20 Rust tests, two Python integrity checks and player build passed.

## Batch 024 — Matroska AVC paired-field clocks

FVID-0046 restores first-field timing and whole-picture duration in native/owned Matroska playback and export.36 authored companions cover unequal/default/missing field durations, hidden references and reordered B pictures with exact components, RGB, seek, rewind, EOF and lossless exports. Unpaired fields, visibility mismatch, duration overflow and packet limits refuse specifically without publication. This supersedes batch021’s PAFF refusal. [Validation](batch-024-validation.json):44 distinct Rust tests, one Python integrity check and player build passed. Earlier AVC policy regression was tightened without adding an ID.

## Batch 025 — usable Matroska track selection

FVID-0047 honors track usability during automatic video/audio/subtitle selection. Authored two-video and video/PCM/subtitle witnesses verify exact pixels, samples, cues, clocks, seek and EOF; disabled malformed packets are ignored for decode while still consuming stored-resource limits. Explicit audio inspection and byte-identical copies retain disabled tracks. Unsupported re-muxing refuses without publication. [Validation](batch-025-validation.json):94 distinct Rust tests, one Python integrity check and player test build passed. The additive reader accessor preserves existing Track structure and APIs.

## Batch 026 — preferred MP4 movie tracks

FVID-0048 uses stored track-header flags for default video/audio selection while preserving explicit indices, complete manual track lists and legacy fallback.10 authored video layouts and PCM/subtitle companions verify exact pixels/samples/cues, clocks, seek, EOF, transformed decode and WAVE intervals. [Validation](batch-026-validation.json):66 distinct Rust tests, one Python integrity check and player test build passed. Intermediate manual-selection regressions were fixed before acceptance; concurrent Ogg integration adds no count.

## Batch 027 — usable tracks survive MP4→Matroska

FVID-0049 preserves effective disabled-track state through copy and filtered FFV1/AAC export, including output reordering. Five authored clips verify exact compressed payloads, selected video pixels/precision, intervals, seek, EOF and numeric AAC silence. Atomic refusals and legacy no-enabled-group fallback remain intact. [Validation](batch-027-validation.json):23 Rust tests, one Python integrity check and player build passed. Movie/preview default preference remains a separate limitation.

## Batch 028

- **FVID-0050** — Honor Matroska default tracks through native/owned selection, movie-priority transport and actual player opening; preserve manual selection and fallback.
- **FVID-0051** — Present generic Matroska decoded audio with nanosecond source clocks, including negative-preroll clipping. A20ms PCM packet previously reached the backend at2500seconds.

73 Rust and12 Python tests plus a player binary build pass. Actual capture workers verify PCM identity, clocks, replay/seek/EOF; six compressed codec witnesses also verify worker replay. Concurrent specialized Vorbis work is preserved and excluded from credit. See [evidence](batch-028-validation.json). No hardware/device or performance claim.

## Batch 029

- **FVID-0052** — Preserve MP4 non-AAC track media clocks independently of decoded sample rate, including exact negative composition preroll clipping.

Twelve authored PCM/raw ALAC clips cover five clocks and negative composition starts. Actual worker verifies exact samples, replay, seek and EOF.40 Rust and1 Python test plus player binary build pass. AAC edit scheduling remains unchanged. See [evidence](batch-029-validation.json).

## Batch 030

- **FVID-0053** — Honor signed Matroska padding in generic decoded audio windows and declared endpoints, including whole discarded tails and negative-preroll overlap.

Seven valid authored videos pass exact samples, clocks, replay, seek and actual capture-worker playback. Oversized padding refuses before backend output.43 distinct Rust and13 Python tests plus player build pass. Specialized AAC padding remains unqualified; Opus/Vorbis behavior is preserved. See [evidence](batch-030-validation.json).

## Batch 031

- **FVID-0054** — Apply Matroska generic CodecDelay consistently to delivery, seek/index coordinates and declared endpoints.

Six authored PCM videos verify exact shifted windows, padding overlap, whole/all preroll and actual capture-worker replay/seek/EOF. An unrepresentable delayed timestamp refuses before backend output.50 Rust and14 Python tests plus player build pass. See [evidence](batch-031-validation.json).

## Batch 032

- **FVID-0055** — Conserve complete decoded access units through the CPAL output ring, with bounded fallible growth and explicit refusal instead of successful truncation.

The700ms authored video contains32768 PCM frames; the backend previously retained only24000. Actual production callback code and a virtual-device AudioPlayback worker now deliver every sample through replay/seek. Stereo wrap, rate change, flush and atomic admission pass:13 Rust and1 Python test plus player build. Ring storage/decoded packet limits are32MiB; this is not an overall RSS limit or a speed claim. Existing public signatures remain unchanged. See [evidence](batch-032-validation.json).

## Batch 033

- **FVID-0056** — Announce playback EOF after the output queue/DSP/device tail has drained; preserve Pause/Seek/Stop and surface device failure.
- **FVID-0057** — Finish finite time-stretched audio below processing lookahead and emit its exact remaining nominal duration.

Two authored60ms videos reproduce unread device PCM at premature EOF and zero output from a480-frame source at1.25rate. Production CPAL rendering and actual virtual-device workers verify completion, generation/error handling and finite output across rates.118 distinct Rust and3 Python tests pass, together with minimal-core checks and a player build. Existing backends remain source-compatible through default completion methods; asynchronous custom backends can implement the handshake. See [evidence](batch-033-validation.json).

## Batch 034 — rendered audio clock

FVID-0058 separates emitted source progress from WSOLA search retirement. Native media reproduces24ms instead of14ms after64 callback frames at1.75rate. Fractional phase, rate switching, suspension and navigation pass; see [validation](batch-034-validation.json). Virtual production callbacks qualify logical timing; physical DAC latency remains outside this proof.

## Batch 035 — public clock during packet pacing

FVID-0059 refreshes public playback position while waiting after large audio packets. Native video and real CPAL rendering reproduce a20ms device clock with a stale0ms frontend clock; the regression now passes alongside worker/navigation checks. [Validation](batch-035-validation.json).

## Batch 036 — refill follows device consumption

FVID-0060 recomputes queued source lead during pacing instead of ending it by wall time. The three-packet2× reproduction and worker/CPAL/minimal gates pass. [Validation](batch-036-validation.json).

## Batch 037 — decoded packet end

FVID-0061 computes source lead from presented PCM PTS and sample duration even when coded packet duration is absent. Native sample controls and Matroska/MP4 clock, padding and delay integration pass. [Validation](batch-037-validation.json).

## Batch 038 — packet clock range

FVID-0062 prevents large public PCM timestamps wrapping through a64-bit nanosecond intermediate. Valid native media precedes timestamp fault injection; this proves packet conversion rather than acceptance of huge container timelines. [Validation](batch-038-validation.json).

## Batch 039 — exact stream and seek coordinates

FVID-0063 preserves stored Matroska timestamps through stream-time conversion and the inverse seek command without floating-point rounding. Both directions count as one improvement. [Validation](batch-039-validation.json).

## Batch 040 — timestamped CPAL output

FVID-0064 preserves packet gaps as silence and avoids repeating overlap. Native waveform, seek, wrapped stereo and atomic resource-refusal tests pass. Producers using dummy repeated0 PTS must now send actual packet timestamps, as required by the scheduling contract. [Validation](batch-040-validation.json).

## Batch 041 — failed device pause

FVID-0065 reports a failed backend pause, attempts stop and terminates the worker. The native-media fault regression distinguishes successful pause from pause/stop failures. [Validation](batch-041-validation.json).

## Batch 042 — failed navigation flush

FVID-0066 prevents new decoding and clock publication after backend seek/rewind flush refusal. Native-media fault tests include successful navigation controls. Current CPAL flush is infallible; this verifies the custom backend error contract. [Validation](batch-042-validation.json).

## Batch 043 — PCM width narrowing

FVID-0067 refuses unsupported declared PCM widths before they can alias supported formats through a narrowing cast. Valid16/32bit and default-width controls retain exact decoding. [Validation](batch-043-validation.json).

## Batch 044 — strict playback PCM admission

FVID-0068 aligns playback with owned strict PCM validation. Complete finite controls play exactly; partial channel frames and NaN/Infinity refuse before backend output. Callers relying on silent tail dropping must repair or explicitly frame malformed input. [Validation](batch-044-validation.json).

## Batch 045 — admit audio setup before preference

FVID-0069 chooses a working alternative when preferred PCM geometry or initial Matroska Opus/AAC/Vorbis setup is unusable. Matroska defaults and MP4 movie preference apply after admission; manual order and specific refusal diagnostics remain. Decoder workers and real player factories pass. [Validation](batch-045-validation.json).

## Batch 046 — block padding spans laced frames

FVID-0070 distributes signed DiscardPadding across a laced block and preserves the owned export origin through fully discarded leading frames. Native samples, playback/seek/EOF, f32/f64 exports and precise refusals pass. [Validation](batch-046-validation.json).

## Batch 047 — restore quantized padding counts

FVID-0071 prevents nearest-nanosecond padding metadata from deleting an extra PCM/Opus sample and aligns the Vorbis endpoint with rendered frames. Bounds retain precise refusal. Earlier53/70 sub-sample oracles now use nearest-sample codec padding; seek boundaries still ceil. [Validation](batch-047-validation.json).

## Batch 048 — nanosecond preroll at zero

FVID-0072 preserves the audible neighbor when timestamp/CodecDelay quantization places its sample-grid boundary1ns below zero. Encoded clocks remain exact; off-grid and positive seek boundaries stay strict. Native workers and zero-output refusals pass. [Validation](batch-048-validation.json).

FVID-0073 reports a stable audible Opus endpoint, excluding fully discarded tail packets and decode-prefix length. Known indexed timing and EOF-derived fallback preserve PCM, preskip, rewind and seek. [Validation](batch-049-validation.json).

FVID-0074 publishes a successfully completed source duration to the live audio handle and sound-only player timeline. Unknown/invalid EOF preserves the prior value, while known zero is retained. [Validation](batch-050-validation.json).

FVID-0075 terminates failed decoder-reset navigation before device flush or publishing a seek clock. Successful native seek/rewind controls and terminal follow-up command checks pass. [Validation](batch-051-validation.json).

FVID-0076 makes terminal device-command errors stop output and worker consistently. Resume/gain/rate refusals cannot admit follow-up seek PCM; successful controls remain usable. [Validation](batch-052-validation.json).

FVID-0077 holds audio behind restarted/sought picture preroll, including Play during buffering. Rewind and paused seek preserve pause; actual Player controls verify exact PCM after readiness. [Validation](batch-053-validation.json).

FVID-0078 retains variable Opus lace packet clocks even when Track DefaultDuration is present. Exact native/owned PCM, workers/navigation and video-only unsupported-header fallback pass. [Validation](batch-054-validation.json).

FVID-0079 derives Vorbis lace clocks from adjacent block overlap, retaining priming and cross-group history.24 native/owned cases, actual worker navigation and precise invalid-mode refusal pass. The concurrent MP3 minimal-feature integration repair counts zero. [Validation](batch-055-validation.json).

FVID-0080 stops AVI block-audio seek at EOF instead of replaying the last ADPCM block. Native IMA/MS data, in-range controls and actual worker end clocks pass; AVI picture playback is not claimed. [Validation](batch-056-validation.json).

FVID-0081 honors known EOF in WAV/AIFF/AU packet readers instead of replaying the final window. Thirty failing targets across ten supported PCM variants now pass, with unchanged in-range controls. WAV8 was separately refused at this stage; FVID-0082 enables it. [Validation](batch-057-validation.json).

FVID-0082 admits valid unsigned8 WAV through the existing native unsigned decoder. Numeric boundaries, channels, extensible header, actual worker navigation/EOF and Player factory pass; float8 remains refused. [Validation](batch-058-validation.json).

FVID-0083 separates extensible WAV precision from stored sample width. Six canonical left-aligned layouts play exactly, while illegal storage/precision refuses specifically. Native workers, owned metadata and Player factory agree. [Validation](batch-059-validation.json).

FVID-0084 prevents maximum file-byte budgets overflowing the extra-byte probe in ten audio readers. Packet/clock identity, ordinary/exact/small limits, malformed refusals and native workers pass on the64bit test host. [Validation](batch-060-validation.json).

FVID-0085 refuses unsupported multipart/list WAV and repeated AIFF sound payloads instead of playing a prefix. Consistent metadata repetitions and normal ordering remain accepted. Multipart concatenation is not implemented; producers must provide one supported payload and consistent headers. [Validation](batch-061-validation.json).

FVID-0086 rejects orphan bytes in fixed-frame WAV/AU audio before packet rounding hides them.24 malformed runs now refuse;18 native controls preserve proper padding/annotation behavior and actual worker navigation. Concurrent MP3 export type repair counts zero. [Validation](batch-062-validation.json).

FVID-0087 rejects AU channel geometry outside the u16 audio interface instead of wrapping it under permissive limits. Native normal layouts and the scalar representable boundary pass; no wide physical-device support is claimed. [Validation](batch-063-validation.json).

FVID-0088 reports known truncated WAV/AU/AIFF payloads at EOF while preserving usable prefix decoding and legitimate shorter/unknown-size controls. Actual worker navigation and Player completion routing pass. [Validation](batch-064-validation.json).

FVID-0089 exhausts AC3/FLAC/MPEG Layer II at EOF seek, preserving in-range packet selection and native worker replay. [Validation](batch-065-validation.json).

FVID-0090 bounds MIDI/XM seeks after the complete release timeline and prevents extreme XM timestamp overflow. In-range and release windows remain available. [Validation](batch-066-validation.json).

FVID-0091 fixes native MIDI/XM PCM presentation clocks: millisecond source PTS now carry1000Hz timebase, preserving sample data and preventing overlapping scheduled windows. [Validation](batch-067-validation.json).

FVID-0092 includes MIDI/XM release windows in playback duration while preserving canonical score duration. Five timeline/tempo controls and actual workers pass. [Validation](batch-068-validation.json).

FVID-0093 restores MIDI/XM synthesis history for nonzero seek, matching continuous PCM without presenting preroll. Reconstruction remains linear in prior timeline. [Validation](batch-069-validation.json).

FVID-0094 widens MIDI clock products before division and refuses unrepresentable timelines. Long probes are parser-only; normal native playback controls pass. [Validation](batch-070-validation.json).

FVID-0095 retains fractional MIDI tempo-map phase, preventing shifted note-off boundaries and missing PCM windows. Wide clock checks remain intact. [Validation](batch-071-validation.json).

FVID-0096 rejects status-bit MIDI channel operands; FVID-0097 preserves following deltas after Tune Request/Continue/Stop. Malformed and valid-width controls are checked independently. [Channel validation](batch-072-validation.json), [width validation](batch-073-validation.json).

FVID-0098 refuses multi-track MIDI format2 until independent sequence selection exists, preserving single-track and simultaneous format1 controls. [Validation](batch-074-validation.json).

FVID-0099 validates native MIDI packet event order and time bounds before synthesis; malformed injected worker packets refuse without PCM or synth-state changes. [Validation](batch-075-validation.json).

FVID-0100 schedules MIDI events at their first eligible sample and preserves end-window state without early phase advance. Four onset/release boundary controls and actual workers pass. [Validation](batch-076-validation.json).

FVID-0088 follow-up: raw AC3 known-truncated final frames now fail EOF without losing usable prefixes or normal tail tolerance. This extends the same issue and adds zero IDs. [Validation](batch-077-validation.json).

FVID-0101 checks AC3 CRC before synthesis and applies existing silence concealment to damaged frames. Correct damaged sound is not claimed. [Validation](batch-078-validation.json).

FVID-0102 recovers exact sample ticks from their presented nanosecond Duration boundaries, preventing EOF seek from replaying audio. Non-boundary seek retains floor behavior. [Validation](batch-079-validation.json).

FVID-0103 accounts for native leading trim in raw AC3 packet presentation, duration and EOF while preserving coded frame indexes. Container mappings require separate qualification under the same issue. [Validation](batch-080-validation.json).

FVID-0103 follow-up: native AC3 Matroska/MP4 priming and padding now match120ms references; silence/repeat edit and seek controls pass. This extends the same issue with zero new IDs. [Validation](batch-081-validation.json).

FVID-0104 refuses clean AC3 frame rates that contradict the configured PCM clock, preserving channel folding and CRC concealment. [Validation](batch-082-validation.json).

FVID-0105 makes raw AC3 seek sample accurate with continuous native PCM. Manual AudioStream consumers must honor `present_decoded` for encoded preroll; direct coded access remains in `Ac3.frames`/`packet()`. Cold history replay is linear. [Validation](batch-083-validation.json).

FVID-0106 replaces bitwise AC3 CRC with a512byte table: two repeated native decode/reset series show about15% lower time with exact PCM equality. Measurement is optimized standalone production-body code, not whole-player speed. [Validation](batch-084-validation.json).

FVID-0105 follow-up: raw AAC LC/SBR/PS now seeks exact decoded output samples, including odd SBR/PS targets and warmed checkpoints. This extends the same issue with zero new IDs. [Validation](batch-085-validation.json).

FVID-0107 routes all stateless PCM widths through exact frame-byte seeks in WAV/AIFF/AU.21numeric source cases and117actual worker targets pass. [Validation](batch-086-validation.json).

FVID-0108 advances MACE/GSM/TrueSpeech packet clocks and coarse seek by complete packed frames, removing gaps from nominal2048frame windows. Six authored native worker controls pass. [Validation](batch-087-validation.json).

## Follow-up088 — predictive WAV/AIFF seek history (zero new IDs)

FVID-0105 now also covers12authored predictive WAV/AIFF sources.48pre-fix seek failures become60cold/60warmed exact native PCM suffix checks.33WAV/14AIFF units,7integration targets,5basic worker tests and player build pass. The broader worker suite records27passes/5concurrent AAC container seek-oracle failures separately. Cold history restoration remains linear. See[validation](batch-088-validation.json). Accepted count stays108.

## Batch089 — SMF running-status lifetime

FVID-0109 refuses missing channel status after meta/SysEx cancellation. Five pre-fix malformed acceptances and six valid native playback controls are covered;17parser/5reader tests,seven integration tests and player build pass. See[validation](batch-089-validation.json).

## Batch090 — SMF extension chunks

FVID-0110 skips bounded unknown chunks without consuming declared musical tracks. Four reproduced valid-input refusals become six native playback controls;17parser/5reader tests,seven integrations and player build pass. Truncated and missing-track refusals remain covered. See[validation](batch-090-validation.json).

## Batch091 — XM absolute PCM grid

FVID-0111 removes independent row rounding: three authored runs previously lost25/17/28samples. Four native held-note row sequences now match one-call PCM and sample clocks, with actual rewind/nonzero seek/EOF checks.11decoder/6reader tests and player build pass. Exact published-boundary selection is separately reproduced in batch092. See[validation](batch-091-validation.json).

## Batch092 — XM published seek boundaries

FVID-0112 makes row lookup use the same millisecond clock as packet publication.93wrong direct boundaries and15worker suffix failures are corrected;128direct boundaries/24worker suffixes,6reader units,related integrations and player build pass. See[validation](batch-092-validation.json).

## Batch093 — held-note EOF release

FVID-0113 adds playback-only note release at MIDI/XM score end. Held-note tails and release seeks now match independent explicit note-off controls.16MIDI/11XM decoder and5SMF/6XM reader tests,14integration tests and player build pass. Canonical events/rows and reserved duration remain unchanged. See[validation](batch-093-validation.json).

## Batch094 — interruptible native synth preroll

FVID-0114 restores MIDI/XM seek history through ordinary worker steps. Gated native decoding proves Pause and replacement seek are serviced between windows, and resumed PCM matches exact continuous suffixes.5SMF/6XM units,nine related integration tests and player build pass. Custom consumers must honor `present_decoded` after reset and discard preroll `None`; raw seek selection remains unchanged. Total cold seek work remains linear. See[validation](batch-094-validation.json).

## Batch095 — XM zero-based sample maps

FVID-0115 fixes silent/wrong waveform selection for standard map0/1. Three authored controls pass actual play/rewind/seek/EOF. Eleven old synthetic fixtures and10manifests were archived byte-exact,then normalized; native PCM hashes and every timestamp remain unchanged.18parser/11decoder/6reader tests,11integration tests and player build pass. Constructed legacy one-based `Instrument.keymap` values require subtracting1; pattern instrument numbers remain one-based. See[validation](batch-095-validation.json).

## Batch096 — XM signed8 PCM scale

FVID-0116 removes byte-replication bias from8bit XM samples. Four nonzero constants previously disagreed with equivalent16bit controls. All256signed values,five native8/16pairs and actual play/rewind/seek now pass;18parser/11decoder/6reader units,11integrations and player build pass. Corrected8bit amplitude is intentional;16bit data/timing/raw fixture bytes remain unchanged. See[validation](batch-096-validation.json).

## Batch097 — MIDI controller reset

FVID-0117 centers active notes and resets expression while preserving channel volume/program. Five authored CC121versus explicit-state controls pass native PCM/program and actual play/rewind/seek/EOF;16decoder/5reader tests,10integrations and player build pass. This qualifies supported controllers only. See[validation](batch-097-validation.json).

## Batch098 — MIDI attack lifetime

FVID-0118 keeps unrendered attacks alive when unrelated stop events prune completed voices. Five lost-note reproductions and three intended-silence controls now match independent survivor PCM in actual play/rewind/seek/EOF.16decoder/5reader units,nine integrations and player build pass. See[validation](batch-098-validation.json).

## Batch099 — XM effect opcode namespace

FVID-0119 corrects F/B/D bytes to15/11/13. Eight literal-effect modules now match expanded effect-free row/tempo controls in native PCM,actual playback andseek;18parser/11decoder/6reader tests,12integrations and player build pass. C40/G40guards qualify default-state no-ops only. The original early synthetic fixture is archived before tempo-opcode normalization,with unchanged PCM/clocks. See[validation](batch-099-validation.json).

## Batch100 — native MIDI sustain acceptance

FVID-0120 enables damper64with separate pedal/key state. The old ignored-control reproduction andignored acceptance are documented before correction; acceptance now runs.13authored delayed-note-off pairs and39actual worker targets pass,including fractional EOF,hard cut,reset andchannel isolation.17decoder/5reader units and player build pass;32voice budget remains bounded. This qualifies sustain only,not full GeneralMIDI/Sostenuto. See[validation](batch-100-validation.json).

## Batch101 — XM muted-note lifetime

FVID-0121 preserves held sample phase throughzero volume. Late/initial mute now restores the continuous waveform; explicit keyoff andunlooped-end controls stay silent. Native smoothing first64frames are outside the independent steady-phase oracle. Actual play/rewind/seek/EOF,11decoder/6reader tests,11integrations and player build pass. See[validation](batch-101-validation.json).

## Batch102 — native XM Cxxvolume acceptance

FVID-0122 enables Cxxafter separately documenting ignored behavior. Ten authored Cxxinputs now match every PCM byte of equivalent volume-column controls,including zero/recovery andprecedence.20actual worker targets,11decoder/6reader units,11integrations and player build pass. Existing native smoothing remains. See[validation](batch-102-validation.json).

## Batch103 — native XM Gxxglobal volume acceptance

FVID-0123 applies master volume toall channels before clipping,with sticky/reset state. Eleven mono/stereo cases and22actual worker targets pass,including32channel headroom.11decoder/6reader/32worker tests,18XM/17MIDIcore parser units withoutdefault features and player build pass. Earlier five concurrent AAC worker-oracle failures are now externally resolved andcount zero. See[validation](batch-103-validation.json).

## Batch104 — XM absolute tail endpoint

Zero-ID FVID-0092 followup fixes a1ms premature end at two of six tempos. Twelve direct and twelve actual worker boundary targets pass; current reader/build rechecks pass after a concurrent AAC build interruption. See[validation](batch-104-validation.json).

## Batch105 — native XM8xxpanning acceptance

FVID-0124 enables panning without resetting sample phase. Ten authored instrument-pan controls,mono/stereo PCM and20actual worker targets pass;11decoder/6reader units and player build pass. See[validation](batch-105-validation.json).

## Batch106 — XM volume-column panning acceptance

FVID-0125 enables C0..CF before standard8xx. Ten authored controls and20actual worker targets pass,with13integrations,11decoder units and player build. See[validation](batch-106-validation.json).

## Batch107 — XM pattern-break boundaries

FVID-0126 restores row0fallback for decoded Dxxvalues above63. Five authored path/duration/PCM controls and10actual worker targets pass,with18parser units and player build. See[validation](batch-107-validation.json).

## Batch108 — XM fine volume slides acceptance

FVID-0127 enables volume-column8x/9xonce perrow,withclamps andstandard-effect precedence. Eleven controls and22actual worker targets pass;15integrations,11decoder units andplayer build pass. See[validation](batch-108-validation.json).

## Batch109 — immediate XM key-off acceptance

FVID-0128 supports K00before same-row note triggering. Six independent note-off controls and12actual worker targets pass;13integrations,11decoder units andplayer build pass. Delayed Kxxremains unsupported. See[validation](batch-109-validation.json).

## Batch110 — XM ping-pong direction

FVID-0129 retains direction between sample steps. Five independent unfolded-loop controls pass with maximum PCM error7.450581e-9,and10actual worker targets preserve native bytes.13integrations,11decoder units andplayer build pass. Existing native endpoint convention remains qualified;no full FT2mixer-equivalence claim. See[validation](batch-110-validation.json).

## Batch111 — XM raw finetune scale

FVID-0130 converts raw finetune from1/128semitone withfullfractional precision.13independent linear waveform oracles reach maximum error1.8189894e-12;linear/Amiga semitone controls and26worker targets pass.12integrations,11decoder units andplayer build pass. A concurrent Matroska queue-type annotation counts zero. See[validation](batch-111-validation.json).

## Batch112 — XM transposed pitch range

FVID-0131 renders valid totalpitches97..119instead ofsilence. Six independent waveform/refusal controls and12actual worker targets pass;11integrations,11decoder units andplayer build pass after a concurrent Matroska build repair. See[validation](batch-112-validation.json).

## Batch113 — MIDI tempo meta extensions

FVID-0132 accepts bounded extension bytes after theknown tempo prefix.11valid playback/PCMcontrols andthree short-prefix refusal controls pass;17parser units andplayer build pass. The contradictory strict-EOTexperiment was reverted andcounts zero. See[validation](batch-113-validation.json).

## Batch114 — MIDI textual track names

FVID-0133 stops binary sequence IDs fromreplacing public tracklabels. Six independent label/PCMcontrols andactualplayback/rewind/seek pass;17parser units andplayer build pass. See[validation](batch-114-validation.json).

## Batch115 — native MIDI SMPTE clocks acceptance

FVID-0134 accepts24/25/29dropframe/30FPSdivisions withrational absolute clocks.12independent PPQevent/PCMcontrols,24worker targets andfiveheaderrefusals pass;17player/17coreparserunits andplayerbuild pass. Typedtime_division metadata andlegacyPPQsentinel are documented;FF54offset remains outside thisprofile. See[validation](batch-115-validation.json).

## Batch116 — SMF topology followup

Zero-ID FVID-0098followup refuses format0multi-track headers before mixing. Single0/simultaneous1actualplaybackcontrols,17parserunits andplayerbuild pass. See[validation](batch-116-validation.json).

## Batch117 — native MIDI stereo pan acceptance

FVID-0135 enablesCC10withcenter-preserving linearstereo balance.14isolated-voice PCMcontrols and28actualworkertargets pass;17decoder units,14integrations andplayerbuild pass. CC121preservespan;mono/wideroutputs anddefaultcenter remainunchanged. See[validation](batch-117-validation.json).

## Batch118 — native MIDI bend sensitivity acceptance

FVID-0136 enablesRPN0semitone/cents sensitivity foractive/future voices.25independent PCMcontrols,50workertargets andaseparatephase/resourceprobe pass;18decoder units,15integrations andplayerbuildpass. Null/NRPNselection andCC121preservation arequalified;otherRPN/increment modes remainoutsideprofile. See[validation](batch-118-validation.json).

## Batch119 — FLAC coded-number length

FVID-0137 fixesmulti-byteframe/sample-number indexing. Sevenauthoredstreams preserveallpackets/PCM;14actualworkertargets,22readerunits andplayerbuildpass. Wideheaderprobes arequalified separatelyfromfullaudio;unrelatedfloatingEOFrounding isisolated forafollowup. See[validation](batch-119-validation.json).

## Batch120 — sample-duration clock followup

Zero-IDFVID-0063followup alignsWAV/AIFF/AU/FLACpublicdurationsandEOFseekwithsamplegrid.24authoredcontrols/actualworkertargets,78readerunits andplayerbuildpass.Twofloat-basedlegacyoracles normalized;AAC/Ogg/AVIdurationauditremainsoutsidequalification. See[validation](batch-120-validation.json).

## Batch121 — legal FLAC block-size codes

FVID-0138 accepts8192/16384codesand8bitescape256. SixindependentPCM/headercontrols,12actualworkertargets,22readerunits andplayerbuildpass. ExplicitFFmpegreference matchesallsixstreams;normaltests/generation remainindependentofit. See[validation](batch-121-validation.json).

## Batch122 — FLAC explicit255kHzrate

FVID-0139 acceptslegal8bitkHzvalue255. Fourindependentrate-header/PCMcontrols,eightactualworkertargets,22readerunits andplayerbuildpass. ExplicitFFmpegreference matchesallrates;zero/mismatchingrateguard retained. See[validation](batch-122-validation.json).

## Batch123 — structural FLAC boundaries

FVID-0140 prevents sample bytes with matching CRCs from impersonating metadata or frames. Three literal-PCM/libFLAC controls, six actual worker targets,1460 prefix profiles,22 reader units and standalone export/resource checks pass. Two15-pair timing runs show about1percent typical index overhead; no speedup or tail-latency claim. The rejected slow implementation and all raw observations are retained. See[validation](batch-123-validation.json).

## Batch124 — bounded FLAC index growth

FVID-0141 reduces8192frame index reallocations from8191to11. Fifteen paired timing runs give median after/before0.761191; tight memory/packet limits and PCM/navigation regressions pass.129frame default capacity costs4064extra requested bytes; power-of-two cases retain the same peak. See[validation](batch-124-validation.json).

## Batch125 — AVI exact-duration followup

FVID-0063 now also covers AVI: three one-nanosecond public EOF clock errors reproduced among six independently decoded MS-ADPCM controls. Integer duration, actualplay/rewind/EOFseek,13reader units andplayerbuild pass. Zero newIDs. See[validation](batch-125-validation.json).

## Batch126 — XM sample-start offsets

FVID-0142 implements9xx withper-channel memory,256sample-frame units atboth widths andsample/loop end bounds.22independenttrimmed-instrument controls,44actualworker targets,18relatedintegrations and11decoderunits pass. No distinctpre-fixseekerror isclaimed. See[validation](batch-126-validation.json).

## Batch127 — MIDI coarse tuning

FVID-0143 implementsRPN2 foractive/future notes while preservingNoteOff identity.22independentnote/bendcontrols,44actualworkertargets,14relatedintegrations and19decoderunits pass. No separatepre-fixseekdefect orfullGMclaim. See[validation](batch-127-validation.json).

## Batch128 — ADTS exact-duration followup

FVID-0063 nowcoversAACduration: fourone-nanosecond publicEOFclock errors reproduced amongsix authoredsilence tracks,independentlydecodedbyFFmpeg.15relatedintegrations,11readerunits,exactPTSregressionandplayerbuildpass. Zero newIDs. See[validation](batch-128-validation.json).

## Batch129 — MIDI fractional channel tuning

FVID-0144 implementsall14bits ofRPN1 andcomposesfine/coarse/bend parameters.27independentcontrols,54actualworkertargets,14relatedintegrations and19decoderunits pass. Legacyunknown-RPNguardnowusesRPN3. See[validation](batch-129-validation.json).

## Batch130 — Ogg exact-duration followup

FVID-0063 nowcoversownedVorbis andlinkedOggcatalog durations. Eightauthoredprograms isolate threeone-nanosecond publicEOFclock errors.14root/26media integrations,ninereaderunitsandplayerbuildpass. Externalreferencequalifiesrawcodec PCM only;EOStrims andmixedchains arecheckednatively. Zero newIDs. See[validation](batch-130-validation.json).

## Batch131 — MIDI decoder buffer geometry

FVID-0145 replacesfivepublicdecoder overflowpanics withpreciseunaddressable-buffer errors,withoutnewduration/rate caps. Bothbuffersreservebeforestatechanges;normalrecovery,actualworkerrefusal,long/emptywindows,13distinctintegrations,19decoderunitsandplayerbuildpass. ScopeexcludesOSovercommit andunrunrelease/32bitconfigurations. See[validation](batch-131-validation.json).

## Batch132 — native MIDI operand-width followup

FVID-0096 nowalso coverspublicnative MIDIrecord operands.14malformedrecords preciselyrefuse;14boundary/unused-byte controls,recoveryandactualworker guards pass.12integrations,36playerunits,17minimalparserunitsandplayerbuildpass. Zero newIDs. See[validation](batch-132-validation.json).

## Batch133 — rejected channel-reference experiment

Reference-only MIDIstate access has noqualifiedreleasegain:15pairedruns medianratio1.009516,95%interval0.963550–1.024707. AllPCMbytesmatch;productionchange revertedexactly. Benchmark/fixturesremain forfurthermeasurements andcountzero. See[experiment](batch-133-experiment.json).

## Batch134 — measured channel-mix coefficient cache

FVID-0146 cachesMIDI gain/balance betweencontroller changes. ConfirmedaggregateCPUratio0.954948(95%interval0.949280–0.959212);all529200PCMbytesmatch.1/8voice profilesbenefit;32voice standalonegainisnotqualified. Inlinecachecost192bytes.18integrations,19unitsandplayerbuildpass. See[validation](batch-134-validation.json).

## Batch135 — measured MIDI render specialization

FVID-0147 selects gain-aware/plain loops once per window. Confirmation CPU ratios0.421926 for muted workloads and0.797809 for normal workloads;2293200 PCM bytes match exactly.19 integration tests,19 decoder units,fixture reproduction and player build pass. The boolean-only variant regressed normal rendering and was rejected. See[validation](batch-135-validation.json).

## Batch137 — native subtitle clock followup

FVID-0014 now also covers the separate native player parser. Four arithmetic panics reproduce independently; precise decimal truncation, healthy cues, two-frame video companion,19subtitle units and player build pass. Zero new IDs. See[validation](batch-137-validation.json).

## Batch136 — requested upper-panel seek status (review)

Moved preparation percentage from the bottom edge to the upper title area. Player builds and the synthetic VP9 control plays; active preparation-state visual capture remains incomplete. The initial attempt counted zero; active60percent state was later visually verified and accepted once asFVID-0152inbatch144. See[review](batch-136-review.json).

## Batch138 — bounded AVC mutation audit

Healthy16x16control decodes.6621 authored packet mutations produce3166refusals and no panics. Admitted changed sample data is not a defect. No production AVC change and zero IDs. See[audit](batch-138-audit.json).

## Batch139 — XM delayed key-off

FVID-0148 schedules Kxx on its nonzero row tick, preserving K00 precedence and FT2 parameter masking.44 exact mono/stereo PCM comparisons and22 actual worker targets pass, along with related integrations,11 decoder units and player build. Envelope/fadeout synthesis remains outside qualification. See[validation](batch-139-validation.json).

## Batch140 — XM tick volume cuts

FVID-0149 implements ECx while preserving held voice identity for later volume restoration.68 exact PCM comparisons and34 actual worker targets pass, including mixed ECx/Kxx deadlines. Related integrations,11 decoder units and player build pass. No performance gain is claimed. See[validation](batch-140-validation.json).

## Batch141 — compound XM navigation

FVID-0150 preserves an earlier Bxx destination when Dxx changes the break row. Six literal traces,24 independent PCM comparisons and12 actual worker targets pass;35 units,related integrations and player build pass. Reverse command order remains correct. See[validation](batch-141-validation.json).

## Batch142 — XM timeline row admission

FVID-0151 closes a final-jump bypass of Limits.rows. Three short budgets refuse; exact natural/jump/one-pass controls and six actual worker targets pass. Minimal build,35 units,related integrations and player build pass. Logical row bound is qualified separately from allocator capacity. See[validation](batch-142-validation.json).

## Batch143 — short XM destination followup

FVID-0126 now also resets a globally legal break that exceeds the actual destination pattern length. Twelve literal traces,48 PCM comparisons and24 actual worker targets pass;35 units,related integrations and player build pass. Zero new IDs. See[validation](batch-143-validation.json).

## Batch144 — requested upper-panel status verified

FVID-0152 places seek preparation below the upper file metadata. Actual native UI showed60percent preparation at the upper left with faded controls;player builds pass. Fullscreen and narrow-width visual coverage remain unqualified. See[validation](batch-144-validation.json).

## Batch145 — XM active sample ownership

FVID-0153 separates future instrument selection from the playing sample.36 exact PCM comparisons and18 actual worker targets pass;35 units,13 related integration targets and player build pass. Historical pan oracles now use static-pan note-on controls;all40 tested pan inputs remain unchanged. Unselected notes are silent and counted as missing samples. See[validation](batch-145-validation.json).

## Batch146 — finite XM pattern loops

FVID-0154 expands E6x with channel-local counters and bounded timeline admission.14 literal traces,56 exact PCM comparisons and28 actual worker targets pass;35 units,minimal configuration,related integrations and player build pass. Existing native global one-pass policy remains qualified separately from local finite repeats. See[validation](batch-146-validation.json).

Batch147 — FVID-0155: SAMI attribute tokenization. Eight failing variants and four healthy controls now agree with independent captions;33 related units,native video/seek and player build pass. Untimed captions are a correct-skip case. Unsupported Y4M seek in the initial test was corrected by using the existing authored indexed VP9 clip; this adds no ID. See [validation](batch-147-validation.json).

Batch148 — FVID-0156: canonical SAMI Sync clocks and selected-language clear events. Six independent SRT references,14 native Player seek targets,11 focused tests,33 related units,fixture reproduction and player build pass.7959 bounded malformed-text probes found no panics; they add no IDs and do not prove full HTML/SAMI conformance. See [validation](batch-148-validation.json).

Batch149 — FVID-0157/0158: case-insensitive SAMI BR tags and .sami sidecar discovery. Each defect failed its specific native Player assertion after actual video presentation before the fix.17 focused tests,33 related units,eight new native seek targets,fixture reproduction and player build pass. Discovery audit150 is incorporated here and adds no extra ID. See [validation](batch-149-validation.json).

Batch151 — FVID-0159: quoted/comment/raw-header text no longer terminates SAMI body discovery. Six independent controls,20 focused tests,33 related units,12822 bounded no-panic probes and player build pass. Physical video playback showed the caption and its later clear;the review exposed a separate renderer hard-break/placement defect for the next repair. See [validation](batch-151-validation.json).

## Batch 153 — subtitle painting

FVID-0160 preserves hard breaks,centers wrapped rows,renders a real black outline
and keeps captions above visible controls. Five output/native regressions,34 related
tests,offline fixture reproduction,player build and physical playback review pass.
See `batch-153-validation.json` for scope and the separate Documents-path wait.

## Batch 152 — no-envelope XM release state

FVID-0161 makes note97/K00/delayed Kxx cut channel volume while preserving
recoverable sample phase. Fourteen scores,56 exact PCM combinations,28 actual
worker seek/EOF targets,35 units,21 related tests and player build pass. The old
permanent-release lifetime oracle is migrated to independent C00/volume restore;
its historical files and failed log remain in evidence. See `batch-152-validation.json`.

## Batch 154 — stopped XM offset gain

FVID-0162 clears gain when9xx starts beyond the sample end,preventing a later
zero-volume note from emitting stale-gain audio. Four scores,16 exact PCM
combinations,8 worker targets,9 focused/related tests,35 units,fixture checks
and player build pass. See `batch-154-validation.json`.

## Batch 155 — valid empty XM slots

FVID-0163 selects valid unpopulated instrument slots instead of replaying the
previous instrument. Instrument-only rows preserve the active sample until a
note;valid instrument1 recovers. Ten scores,40 PCM combinations,20 worker
target variants,15 integrations,35 units,fixture checks and player build pass.
The old wrong-audio unit expectation and duplicate diagnostic are corrected.
Short empty headers remain separate;see `batch-155-validation.json`.

## Batch 156 — short empty XM instrument headers

FVID-0164 reads29-byte sample-free headers and skips declared padding without
reading the next instrument as metadata. Eight layouts,four malformed refusals,
32 PCM combinations,16 worker targets,13 integrations,35 player units,18 core
units,final player/core regressions and player build pass. Size0 quirks are not
qualified. See `batch-156-validation.json`.

## Batch 157 — invalid XM instrument defaults

FVID-0165 preserves mute,pan and K00 when an invalid instrument component is
ignored. Valid notes/effects and populated/valid-empty128 restores remain active.
Fourteen scores,56 PCM pairs,28 worker targets,20 integrations,35 units,unchanged
reference streams and player build pass. Historical proof prevents counting K00
variants as a separate fix. See `batch-157-validation.json`.

## Batch 158 — whole XM tick spans

FVID-0166 converts a whole tick interval before microsecond rounding. Equal
tempo/speed ratios retain row lengths,and Kxx/ECx no longer cut a sample early
in the authored boundary cases. Ten scores,40 PCM pairs,16 fixed-window pairs,
20 worker targets,20 integrations,35 player units,18 core units,2 core
regressions and player build pass. See `batch-158-validation.json`.

## Batch 159 — AVI index origins

FVID-0167 accepts the documented file-origin idx1 convention while requiring
a consistent origin across checked audio entries. Eight indexed cases,16 worker
target variants,7 integrations,34 related units,2 core regressions and player
build pass. Mixed origins and invalid addresses still refuse.
See `batch-159-validation.json`.

## Batch 160 — AVI main-header field boundaries

FVID-0168 refuses an avih stream count borrowed from the next chunk. Six
malformed inputs now refuse;four full/padded controls retain literal PCM and
worker navigation.11 integrations,34 related units,4 core regressions and
player build pass. See `batch-160-validation.json`.

## Batch 161 — FLAC coded-clock consistency

FVID-0169 refuses canonical but discontinuous frame/sample clocks and blocking
strategy changes before building the bare-FLAC index. Eleven malformed scores
refuse while four legal controls retain exact PCM and8 worker targets. Native,
shared-media,core,libFLAC MD5 and player build gates pass. Three old mask0
test oracles were aligned with unchanged committed routing behavior at zero
new IDs;original failures and sources remain archived. See `batch-161-validation.json`.

## Batch 162 — Ogg page sequence integrity

FVID-0170 detects page-counter discontinuities in the canonical and standalone
streaming readers,including correct counter restoration on rewind. Actual
missing audio no longer reaches silent successful EOF in the strict file profile.
32 integrations,19 root/7 media units,9 media integrations,2 core/2 streaming
regressions,65 Opus/Ogg units and player build pass. Initial/wrapped counters
and positive origins remain supported;discontinuous live captures need coherent
reframing rather than silent gap acceptance. See `batch-162-validation.json`.

## Batch 163 — Ogg packet continuation state

FVID-0171 prevents incomplete packet tails from becoming audio and admits valid
joined live prefixes by discarding only the unavailable fragment. Both readers
preserve whole packets and granules;rewind restores streaming fragment state.
38 integrations,19 root/7 media units,9 media integrations,5 core/5 streaming
regressions,65 Opus/Ogg units,22 new worker targets and player build pass.
See `batch-163-validation.json`.

## Batch 164 — streaming Ogg logical identity

FVID-0172 establishes serial ownership at the initial BOS and refuses another
BOS inside the active stream before its bytes can use the original OpusHead.
Both packet APIs,rewind,zero serials,continuation and counter profiles remain
qualified;stop-at-first-EOS behavior is preserved.40 integrations,19 root/7 media
units,9 media integrations,6 core/7 streaming regressions,65 Opus/Ogg units,4 new
worker targets and player build pass. See `batch-164-validation.json`.

## Batch 165 — AVI outer RIFF boundaries

FVID-0173 keeps AVI child walking inside the declared RIFF form. Mandatory
lists outside it refuse;an outside idx1 cannot change the completed first
RIFF's validation. Existing overdeclared/sentinel clamping and whole-input
budgets remain.17 integrations,34 related units,7 core checks,16 worker targets
and player build pass. Old zero-size unit fixtures were repaired at zero
additional IDs. See `batch-165-validation.json`.

## Batch 166 — grouped AVI ADPCM samples

FVID-0174 exposes grouped fixed-size ADPCM samples as independent block packets.
Raw chunk indices remain intact;logical packet expansion is bounded before
allocation.14 mono/stereo programs,4 plain controls,exact PCM and36 worker
targets pass.7 focused/18 related integrations,34 units,8 core checks and player
build pass. Existing block-rounded seek and G.711 behavior are preserved.
See `batch-166-validation.json`.

## Batch 167 — classic AVI linear PCM

FVID-0175 frames unsigned8,signed16/24/32 and IEEE32/64 PCM with full encoded
frame stride and bounded2048-frame packets.13 mono/stereo programs preserve
literal PCM,exact clocks,seek,rewind and26 worker targets.7 focused/25 related
integrations,34 units,9 core checks and player build pass. No performance or
AVI-video GUI qualification is claimed. See `batch-167-validation.json`.

## Batch 168 — AVI audio-record body extents

FVID-0176 refuses known audio chunks whose declared body exceeds its enclosing
run.16 formerly successful clipped PCM/ADPCM admissions now refuse precisely;
12 healthy direct/nested/sentinel controls retain exact PCM and24 worker targets.
4 focused/32 related integrations,34 units,11 core checks and player build pass.
See `batch-168-validation.json`.

## Batch 169 — mandatory AIFC codec identification

FVID-0177 refuses incomplete AIFC compression types instead of guessing NONE
and reinterpreting sowt bytes as big-endian PCM.16 wrong-output cases now refuse
precisely;16 healthy AIFF/AIFC controls retain exact PCM and32 worker targets.
5 focused/9 related integrations,14 AIFF units,1 core check and player build pass.
Complete22-byte codec types retain compatibility. See `batch-169-validation.json`.

## Batch 170 — AIFF fixed-frame alignment follow-up (0 IDs)

Existing FVID-0086 now also covers AIFF/AIFC:12 native false-success orphan
runs refuse before packet rounding.27 numeric controls,36 malformed cases,
14 related integrations,14 AIFF units,1 core check and player build pass.
All previous WAV/AU media hashes remain identical. Count stays177/1000.
See `batch-170-validation.json`.

## Batch 171 — effective fragmented MP4 sample descriptions

FVID-0178 resolves tfhd overrides over trex defaults before publishing samples.
Unsupported effective0 now refuses;overridden unused0 and empty0 runs stay valid.
Analytic VP9 planes/RGB,clocks and40 public native seek targets pass.4 focused/
54 related integrations,17 units,25 core checks and player build pass. The initial
seek test oracle was corrected to the documented final-frame clamp with exact
pixels/interval and repeatedEOF checks;it adds no ID. See `batch-171-validation.json`.

## Batch 172 — full signed 32-bit fragmented MP4 `trun.data_offset`

FVID-0179 resolves negative `trun.data_offset` magnitudes via `unsigned_abs()`
before `checked_sub`, eliminating the `i32::MIN` (`-2147483648`) negation
overflow panic. Corrupt short `min-underflow.mp4` now refuses cleanly as
`Error::Invalid` without unwinding, while a legitimate `-2 GiB` displacement
across a virtual `free` box (`valid-min`) indexes all 3 packets in 13,152 read
bytes and completes native VP9 playback, 8 clamped public seeks, rewind, and
repeated EOF in 75,630 read bytes. 4 focused/58 related integrations, 17 units,
27 core checks, fixture unittest and player build pass. See
`batch-172-validation.json`.

## Batch 173 — mutually exclusive fragmented MP4 `trun` sample flags

FVID-0180 rejects `trun` boxes that set both `first-sample-flags-present`
(`0x00_0004`) and `sample-flags-present` (`0x00_0400`) per ISO/IEC
14496-12:2015, 8.8.8, instead of consuming both fields and silently discarding
`first_sample_flags`. All 4 contradictory populated, late-fragment, and
empty-run cases now refuse as `Error::Invalid`, while 5 healthy controls
(regular MP4, default `tfhd`/`trex` flags, first-sample-only, per-sample-only,
and first-sample-lead-sync) keep exact packets, sync flags, 50 Hz clocks,
analytic VP9 planes/RGB, and 40 public clamped seek targets. 4 focused/62
related integrations, 17 units, 29 core checks, fixture unittest and player
build pass. See `batch-173-validation.json`.

## Batch 174 — exact fragmented MP4 `trex`, `tfhd`, and `tfdt` box extents

FVID-0181 enforces exact FullBox payload sizes for `trex` (24 bytes), `tfhd`
(exact match with flag-selected fields, including `at += 4` for `0x00_0020`),
and `tfdt` (8 bytes for version 0, 12 bytes for version 1) per ISO/IEC
14496-12:2015, 8.8.3, 8.8.7, and 8.8.12. All 5 overlong `trex`, `tfhd`, and
`tfdt` inputs now refuse as `Error::Invalid`, while 4 healthy controls (regular
MP4, `tfhd` sync/non-sync overrides over `trex`, and 64-bit `tfdt` v1) retain
exact packets, sync flags, 50 Hz clocks, analytic VP9 planes/RGB, and 32 public
clamped seek targets. 4 focused/66 related integrations, 17 units, 31 core
checks, fixture unittest and player build pass. See `batch-174-validation.json`.

## Batch 175 — per-`traf` fragmented MP4 `trun` data-offset continuity

FVID-0182 scopes implicit `trun` data-offset continuity (`flags & 0x00_0001 == 0`)
to a per-`traf` cursor initialized to `base` and updated after every `trun`
(including empty runs) per ISO/IEC 14496-12:2015, 8.8.8.2, instead of consulting
the global track `list.last()`. Valid multi-`trun` fragments (including an empty
leading `trun` that positions the cursor for a subsequent contiguous `trun`) now
index and play with exact analytic VP9 planes/RGB, 50 Hz clocks, and 32 public
clamped seek targets, while cross-`traf` and empty-reset runs whose `traf`
cursor lands outside `mdat` refuse as `Error::Invalid`. 4 focused/70 related
integrations, 17 units, 33 core checks, fixture unittest and player build pass.
See `batch-175-validation.json`.

## Batch 176 — mandatory fragmented MP4 `mfhd` and unique non-zero `trex` track IDs

FVID-0183 validates that every `moof` box carries a singleton version-0 8-byte
`mfhd` box with a non-zero `sequence_number` (ISO/IEC 14496-12:2015, 8.8.4 and
8.8.5) and that `mvex/trex` boxes declare unique non-zero `track_ID` values
(ISO/IEC 14496-12:2015, 8.8.3). All 6 malformed `mfhd` and `trex` inputs now
refuse as `Error::Invalid`, while 4 healthy controls (regular MP4, single
fragment, three sequential fragments, and distinct multi-track `trex` entries)
keep exact packets, sync flags, 50 Hz clocks, analytic VP9 planes/RGB, and 32
public clamped seek targets. 4 focused/74 related integrations, 17 units, 35
core checks, fixture unittest and player build pass. See
`batch-176-validation.json`.

## Batch 177 — hybrid fragmented MP4 `moov/stbl` + `moof/traf` indexing and validation

FVID-0184 indexes and validates the initial `moov/trak/mdia/minf/stbl` sample
table even when a file carries `mvex` fragments (ISO/IEC 14496-12:2015, 8.8.4
and 8.8.8), appending subsequent `moof/traf/trun` samples to those initial
samples and extending `track.duration` across the combined sample timeline.
Hybrid files with initial samples in `moov/stbl` followed by explicit or
implicit-`tfdt` fragments now preserve all samples, 50 Hz clocks, analytic VP9
planes/RGB, and 32 public clamped seek targets, while fragmented files with
corrupt initial `stbl` tables (`stco` outside `mdat` or invalid `stsz`) refuse
as `Error::Invalid` before decoder admission. 4 focused/78 related integrations,
17 units, 37 core checks, fixture unittest and player build pass. See
`batch-177-validation.json`.

## Batch 178 — `fvid-media` `owned_mp4` fragmented MP4 validation and hybrid indexing parity

FVID-0185 brings `fvid_media::owned_mp4::Mp4Reader`, `probe_mp4`, and
`decode_video` (`crates/fvid-media/src/owned_mp4_impl.rs`) into full fragmented
MP4 compliance with `src/container/mp4.rs`: resolving `trun.data_offset ==
i32::MIN` without overflow panics, enforcing effective sample description
selection, mutually exclusive `trun` flags (`0x00_0004` + `0x00_0400`), exact
`trex`/`tfhd`/`tfdt`/`mfhd` payload extents, unique non-zero `trex` track IDs,
per-`traf` `trun` data cursor continuity, and initial `moov/stbl` validation and
indexing in hybrid fragmented files. 4 focused/82 related integrations, 15
`owned_mp4` units, 41 core checks, fixture unittest and player build pass. See
`batch-178-validation.json`.

## Batch 179 — QuickTime version-1 and `wave/esds` `mp4a` audio sample descriptions in `fvid-media` `owned_mp4`

FVID-0186 updates `fvid_media::owned_mp4::Mp4Reader`
(`crates/fvid-media/src/owned_mp4_impl.rs`) to accept QuickTime version-1
`mp4a` `SoundDescription` headers (44 bytes) and resolve `esds` nested inside a
QuickTime `wave` extension atom, matching `src/container/mp4.rs`. All 6 healthy
ISO/QuickTime v0/v1/v2 direct and `wave/esds` controls decode 3,072 mono 48 kHz
float32 samples matching the independent analytical IMDCT + overlap-add
reference within `1e-6`, expose identical audio metadata in `probe_mp4`, remux
to Matroska, and pass player `AudioStream` seek/rewind/EOF, while 3 malformed
inputs (truncated v1 header, `wave` without `esds`, direct without `esds`)
refuse as `Error::Invalid`. 4 focused/99 related integrations, 15 `owned_mp4`
units, 44 core checks, fixture unittest and player build pass. See
`batch-179-validation.json`.

## Batch 180 — MP4/MOV empty, zero-duration, and blank-only edit-list (`edts/elst`) validation

FVID-0187 rejects empty `elst` boxes (`entry_count == 0`) and zero-duration v0/v1
edit entries (`segment_duration == 0`) in both `src/container/mp4.rs` and
`crates/fvid-media/src/owned_mp4_impl.rs`, rejects zero-duration audio edit
segments in `AudioTimeline::new`
(`crates/fvid-media/src/owned_mp4_audio_schedule_impl.rs`), and rejects
blank-only video edit lists (`media_time == -1` with no picture edit) in
`owned_video_timeline::map_edits` (`crates/fvid-media/src/owned_video_timeline.rs`)
instead of returning an empty edit vector that caused `NativeReader` and
`decode_video` to play unedited video frames. Two healthy v0/v1 edited controls
preserve exact video frame counts, `probe_mp4` durations, seek/rewind, and
analytical AAC-LC float32 PCM (`1e-7`), while four malformed inputs refuse as
`Error::Invalid`. 4 focused/51 related integrations, 6 timeline units, 23 core
checks, fixture unittest and player build pass. See `batch-180-validation.json`.

## Batch 181 — automatic enabled/default audio track selection and in-band PS export geometry in `native_media` and `native_export`

FVID-0188 updates `mp4_audio_index`, `mp4_aac_index`, `matroska_audio_index`, and
`matroska_aac_index` in `src/native_media.rs` to filter automatic audio candidates
through `Mp4Reader::automatic_track_indices(&[*b"soun"])` and
`WebmReader::preferred_track_numbers(...)` when `selected.is_none()`, and updates
`export_pcm_selected` in `src/native_export.rs` to negotiate in-band Parametric
Stereo (`negotiate_mp4_ps` / `negotiate_matroska_ps`) before configuring output
channels and WAV headers. Three synthetic MP4 controls, remuxed Matroska
dual-audio controls, existing `audiovisual.mp4` fixtures, and implicit-PS WAV
export match analytical float32 PCM (`1e-6`) and exact stereo WAV geometry
(48 kHz, 2 channels, 9,600 frames), while ambiguous dual-enabled tracks continue
to refuse automatic selection. 2 focused/25 related integrations, 14 MP4 units,
359 core checks, fixture unittest and player build pass. See
`batch-181-validation.json`.

## Batch 182 — MP4/MOV chunk-span ordering, positive sample sizes, and non-empty track sample tables

FVID-0189 enforces strictly increasing non-overlapping chunk spans (`offset >= prev_end`)
in `chunk_runs`, positive coded sample sizes (`size > 0`) in `expanded_index` (`stsz`)
and `index_fragments` (`trun`/`tfhd`), and non-empty sample tables (`count > 0` for
non-fragmented tracks in `sample_index` and `track.samples.len() > 0` after fragment
indexing in `index_fragments`) across both `src/container/mp4.rs` and
`crates/fvid-media/src/owned_mp4_impl.rs`. Two multi-chunk `stco` and `co64` controls
decode 3 exact 81-byte VP9 packets and 3 decoded 16x16 RGB frames across sequential
playback, EOF, rewind, and seek, while six malformed inputs (overlapping `stco`,
backward `co64`, zero-byte `stsz`, zero-byte `trun`, empty non-fragmented track, and
empty fragmented track) refuse as `Error::Invalid`. 2 focused/46 related integrations,
18 MP4 units, 359 core checks, fixture unittest and player build pass. See
`batch-182-validation.json`.

## Batch 183 — Matroska/WebM `BlockGroup` cardinality, reserved block flags, and deterministic equal-PTS/pre-origin seek

FVID-0190 enforces that every Matroska `BlockGroup` (`0xa0`) contains exactly one
`Block` (`0xa1`) child and rejects non-zero reserved flag bits in `SimpleBlock`
(`0xa3`, mask `0x70`) and `Block` (`0xa1`, mask `0x71`) in
`crates/fvid-media/src/owned_webm_reader_impl.rs`, while updating
`WebmVideoReader::seek_to_sync` (`src/playback/video/webm.rs`) and
`WebmAudioReader::seek` (`src/playback/audio/webm_audio.rs`) to select the first
keyframe or packet at a shared timestamp (`>` instead of `>=`) and to advance to
the first packet of `self.track_number` on pre-origin audio seeks. Three healthy
controls (equal-PTS VP9 invisible setup keyframe + show-existing frame seek,
valid `BlockGroup`, and multi-track delayed-origin + equal-PTS audio seek) preserve
exact 16x16 VP9 planes, `BlockGroup` duration, and 16-bit PCM packets across
sequential playback and seek, while four malformed inputs (`BlockGroup` missing
`Block`, duplicate `Block`, `SimpleBlock` reserved flags, and `Block` discardable
flag) refuse as `Error::Invalid`. 2 focused/7 related integrations, 10 `owned_webm`
units, 359 core checks, fixture unittest and binary build pass. See
`batch-183-validation.json`.

## Batch 184 — AVI duplicate header/format rejection, `idx1` partial-entry validation, and delayed-start PCM/G.711 seek landing

FVID-0191 rejects duplicate `avih` chunks in `hdrl`, duplicate `strh` and `strf`
chunks in `LIST strl`, and non-16-byte-multiple `idx1` chunks in
`src/container/avi.rs`, and clamps PCM/G.711 seek targets to `[start, end]` in
`AviAudioReader::seek_to` (`src/playback/audio/avi_audio.rs`) so pre-start seeks
on streams with `dwStart > 0` return the stream's first sample timestamp matching
`next_packet()`. A delayed-start PCM control (`dwStart = 100`) lands at sample
100 on pre-start seeks, slices exact 16-bit PCM frames on mid-record seeks
(sample 106), and reaches EOF at sample 108, while four malformed inputs
(duplicate `avih`, duplicate `strh`, duplicate `strf`, and unaligned `idx1`)
refuse as `Error::Invalid`. 2 focused/3 related integrations, 34 AVI units,
359 core checks, fixture unittest and binary build pass. See
`batch-184-validation.json`.

## Batch 185 — Ogg reserved page flags, spurious continuation rejection, Vorbis chain EOS enforcement, and multi-track Opus track-index reporting

FVID-0192 rejects non-zero reserved bits in Ogg `header_type_flag`
(`flags & !(CONTINUES | BEGINS | ENDS) != 0`), 0-segment pages with `CONTINUES`
when no packet is open, and mid-stream pages with `CONTINUES` when no packet is
open after completed audio packets in `crates/fvid-media/src/owned_ogg.rs`;
requires each preceding link in a chained Vorbis program to have `ended == true`
in `crates/fvid-media/src/owned_ogg_vorbis.rs`; and exposes `audio_track_index()`
on `fvid_media::owned_ogg_opus::Program` (`crates/fvid-media/src/owned_ogg_opus.rs`)
and `OggOpusAudioReader` (`src/playback/audio/ogg_opus.rs`). Three healthy
controls (two-track parallel Opus, 0-segment continuation bridge page, and
two-link chained Vorbis) preserve exact track indices (`0` and `1`), split-packet
reassembly, float32 PCM, and worker seek/EOF navigation, while five malformed
inputs (reserved BOS flag, reserved audio flag, spurious 0-segment continuation,
spurious mid-stream continuation, and unended first Vorbis link) refuse as
`Error::Invalid`. 2 focused/43 related integrations, 8 Ogg/Vorbis units,
359 core checks, fixture unittest and binary build pass. See
`batch-185-validation.json`.

## Batch 186 — WAV extensible `cbSize`/channel-mask bounds, AIFC G.711 sample-width enforcement, and `AuAudioReader::present_decoded` parity

FVID-0193 enforces `usize::from(cbSize) <= body.len() - 18` and
`channel_mask == 0 || channel_mask.count_ones() <= u32::from(channels)` in
`WAVE_FORMAT_EXTENSIBLE` (`src/playback/audio/wav.rs`), rejects non-8-bit
`sampleSize` values on AIFC `alaw`/`ALAW`/`ulaw`/`ULAW` tracks
(`src/playback/audio/aiff.rs`), and implements `present_decoded` and
`presentation_floor` tracking in `AuAudioReader` (`src/playback/audio/au.rs`)
matching `WavAudioReader` and `AiffAudioReader`. Three healthy controls
(extensible WAV, 8-bit AIFC A-law, and 16-bit AU PCM with seek/floor trimming)
preserve exact packets and decoded timestamps, while four malformed inputs
(speaker-mask popcount overflow, oversized `cbSize`, 16-bit AIFC `alaw`, and
24-bit AIFC `ulaw`) plus negative `source_pts` and unaligned decoded payloads
refuse as `Error::Invalid`. 3 focused integrations, 56 WAV/AIFF/AU units,
359 core checks, fixture unittest and binary build pass. See
`batch-186-validation.json`.

Correction in batch524:the AIFC8-only interpretation above is withdrawn. COMM sampleSize describes the original1..32-bit precision,not coded bytes. The16/24-bit controls are valid;WAV/AU safeguards remain unchanged. See `batch-524-validation.json`.

## Batch 187 — FLAC `STREAMINFO` ordering, exact length, block/frame bounds, reserved metadata rejection, and speaker-mask channel bounds

FVID-0194 enforces RFC 9639 §8.1/§8.2 metadata and geometry invariants across
bare FLAC (`crates/fvid-media/src/owned_flac.rs`) and Matroska FLAC
(`crates/fvid-media/src/owned_flac_decoder.rs`, `src/codec/flac_decoder.rs`,
and `src/playback/audio/webm_audio.rs`): requires the first metadata block at
byte 4 to be `STREAMINFO` (type `0`, exact length `34`), rejects duplicate
`STREAMINFO` blocks and reserved metadata block types (`7..=127`), validates
`min_block >= 16`, `max_block >= 16`, and `min_block <= max_block`, checks
non-zero `max_frame` against actual frame sizes even when `min_frame == 0`, and
rejects `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` values whose popcount exceeds the
stream channel count (`channel_mask.count_ones() <= u32::from(channels)`). Two
healthy bare/Matroska FLAC controls and ten malformed inputs pass; 2 focused/11
related integrations, 24 FLAC units, fixture unittest and binary build pass.
See `batch-187-validation.json`.

## Batch 188 — SMF End-of-Track enforcement, XM unpacked note clamping & 16-bit sample/pattern bounds, and SMF/XM `present_decoded` validation

FVID-0195 requires every SMF `MTrk` chunk to terminate with an End-of-Track
meta-event (`FF 2F`) in `src/container/smf.rs`, clamps out-of-range notes
(`> NOTE_KEY_OFF`) in unpacked XM pattern cells to `0` (`read_cell`), bounds
XM pattern cell decoding strictly to `&bytes[data..end]` (`parse_patterns`),
rejects odd `data_bytes` lengths on 16-bit XM samples (`parse_instruments` in
`src/container/xm.rs`), and validates `source_pts >= 0` and stereo float32 frame
alignment (`packet.data.len() % 8 == 0`) in `SmfAudioReader::present_decoded`
(`src/playback/audio/smf.rs`) and `XmAudioReader::present_decoded`
(`src/playback/audio/xm.rs`). Four healthy SMF/XM controls and three malformed
inputs pass; 4 focused/6 related integrations, 38 SMF/XM units, fixture
unittest and binary build pass. See `batch-188-validation.json`.

## Batch 189 — WavPack APEv2 flag & subblock header validation, MP3 `present_decoded` baseline checks, and AC-3 `open_mode` file budget

FVID-0196 rejects APEv2 footers with `flags & 0x60000000 != 0` (`is_header` or
`no_footer` set) and requires APEv2 header flags `u32le(&header, 20) == (flags | 0x20000000)`
in `Input::tags` (`src/playback/audio/wavpack.rs`), rejects 1..3-byte truncated
metadata subblock headers at the end of a WavPack block in `Block::parse`
(`src/codec/wavpack_decoder.rs`), validates `source_pts >= 0` and channel-frame
stride alignment and stamps `pts`/`timebase` in `Mp3AudioReader::present_decoded`
even when neither gapless metadata nor a seek floor is active
(`src/playback/audio/mp3.rs`), and enforces `bytes.len() > limits.file_bytes`
upfront in `Ac3AudioReader::open_mode` (`src/playback/audio/ac3.rs`). Healthy
WavPack, MP3, and AC-3 controls and four malformed WavPack inputs pass; 2
focused/3 related integrations, 19 unit tests, fixture unittest and binary
build pass. See `batch-189-validation.json`.

## Batch 190 — Audio decoder bounds and state resets (`FlacDecoder`, `mp2::Decoder`, `adpcm_decoder`, `XmDecoder`, `MidiDecoder`)

FVID-0197 validates `min_frame <= max_frame` (when `max_frame != 0`) in
`FlacDecoder::new` and enforces `frame.frames <= usize::from(self.max_block)` in
`FlacDecoder::decode_encoded` (`src/codec/flac_decoder.rs`), restricts
`fvid_codecs::mp2::Decoder::new` to the 9 standard MPEG Layer II sample rates
and checks `value.is_finite()` when narrowing `f64` synthesis output to `f32`
(`crates/fvid-codecs/src/mp2.rs`), requires 8-byte stereo payload alignment for
`CODEC_ID_ADPCM_IMA_WAV` in `frames_of` (`src/codec/adpcm_decoder.rs`), requires
`row.cells.len() == self.voices.len()` in `XmDecoder::decode_encoded` and resets
`self.ignored = self.initial_ignored` in `XmDecoder::reset`
(`src/codec/xm_decoder.rs`), and resets `self.rendered = 0` in
`MidiDecoder::reset` (`src/codec/midi_decoder.rs`). Three focused integration
tests, 3 MP2 integration tests, FLAC/ADPCM/XM/MIDI unit suites, fixture
unittest and binary build pass. See `batch-190-validation.json`.

## Batch 191 — FFV1 v3 non-keyframe range context initialization on quantizer-set growth and AV1 4:2:0 film-grain chroma point-count conformance

FVID-0198 initializes expanded range context banks (`grow[i] == true`) from
`self.sets[selected[i]].initial` instead of `[0; 32]` in
`Configuration::change_quantizers`
(`crates/fvid-media/src/owned_ffv1_v3_decoder_impl.rs`) so non-keyframe slices
switching to a larger quantizer table set do not get stuck at zero probability,
and enforces the symmetric AV1 6.8.20 4:2:0 chroma conformance check
`(g.point_counts[1] == 0) != (g.point_counts[2] == 0)` in `Grain::parse`
(`src/codec/av1_grain_params.rs`). Two focused integration tests, 17 FFV1/AV1
unit tests, fixture unittest and binary build pass. See
`batch-191-validation.json`.

## Batch 192 — Matroska `Picture::Packed` reorder-queue budget accounting, deferred in-band HEVC parameter sets, and MP4 `NativeReader::current_pts` pre-frame state

FVID-0199 accounts for `Picture::Packed` (`frame.data.len()` plus optional
`alpha` bytes), `Picture::Vp9`, and `Picture::Av1` plane storage in
`WebmVideoReader::next_decoded` (`src/playback/video/webm.rs`), opens Matroska
`V_MPEGH/ISO/HEVC` tracks via `HevcDecoder::from_inband_configuration` and uses
`d.try_parameters()` in `WebmVideoReader::bitstream_colour` and `advance`, and
returns `None` from `NativeReader::current_pts` (`src/playback/video/native.rs`)
on `NativeReader::Avc` when `*frames == 0` (before the first frame is read or
immediately after `rewind()`). Two focused integration tests, 13 related
Matroska/AVC/HEVC integration tests, 44 playback unit tests, fixture unittest
and binary build pass. See `batch-192-validation.json`.

## Batch 193 — WebVTT `NOTE`/`STYLE`/`REGION` block skipping, case-insensitive SAMI `&nbsp;` clearing, ASS `[Events]` scoping, and `Spool::seek` negative-offset rejection

FVID-0200 skips multi-line `NOTE`, `STYLE`, and `REGION` blocks until the next
blank line in `subtitles::parse_webvtt` (`src/subtitles.rs`) so commented-out
timings or CSS comments containing `-->` are not rendered as cues, folds
`&nbsp;` ASCII case-insensitively in `line_breaks` so uppercase `&NBSP;` and
mixed-case `&Nbsp;` paragraphs clear SAMI captions as intended, scopes
`subtitles::parse_ass` `Dialogue:` parsing to the `[Events]` section, and
rejects negative target offsets in `playback_spool::Spool::seek`
(`src/playback/runtime/spool.rs`) with `io::ErrorKind::InvalidInput` per the
`std::io::Seek` contract. Four focused integration tests, 33 subtitle unit
tests, 5 spool unit tests, and fixture unittest pass. See
`batch-193-validation.json`.

## Batch 194 — `.3dl` non-finite rejection, `.cube`/`.spi1d` domain validation, full-entry 1D `Lut::is_identity`, and non-degenerate `MasteringDisplay` validation

FVID-0201 rejects non-finite (`NaN`/`inf`) node and mesh values in
`Lut::from_3dl`, validates finite `min < max` input domains (`DOMAIN_MIN`,
`DOMAIN_MAX`, `LUT_1D_INPUT_RANGE`, `LUT_3D_INPUT_RANGE`, and `.spi1d` `From`)
in `Lut::from_cube` and `Lut::from_spi1d`, checks every entry across all three
channels against its domain ramp in `Lut::is_identity` for `Lut::One`
(`src/color/lut.rs`) so non-identity 1D LUTs with changes between the 17 coarse
grey probes are not skipped by `Grade::apply`, and rejects zero/collinear CIE
1931 primaries (`y == 0.0`, `x + y > 1.0`, or singular `rgb_to_xyz`/`xyz_to_rgb`
matrices such as all-zero placeholder MDCV payloads) and invalid luminance
bounds (`max_luminance <= 0.0` or `min_luminance > max_luminance`) in
`MasteringDisplay::from_corners` (`src/color/hdr.rs`). Four focused integration
tests, 69 root color unit tests, 68 `fvid-codecs` color unit tests, and 5
offline fixture unit tests pass. See `batch-194-validation.json`.

## Batch 195 — Playback worker seek/rewind terminal state & cache sync, ASS `-->` routing & `[Events]` scoping, and overflow-safe play controls

FVID-0202 marks `self.ended = true` and synchronizes `cache_packets` /
`indexed_count` when `reader.seek_raw` returns `Ok(None)` or `Err(error)` or
when `reader.rewind` returns `Err(error)` in `Worker::drain_commands`
(`src/playback/runtime/thread.rs`), preventing the playback worker from
immediately invoking `decode_next()` in the same generation after emitting a
terminal seek/rewind event; detects ASS headers (`[Script Info]`, `[Events]`,
`Dialogue:`) before checking `-->` in `parse_subtitle_text` and scopes
`parse_ass` `Dialogue:` lines to `[Events]` in
`crates/fvid-media/src/owned_play_controls_impl.rs`; widens coordinates to
`i64` with checked buffer bounds in `blit_bitmap_subtitle`; uses
`usize::checked_mul` in `detect_letterbox_bars`, `waveform_column_fills`,
`apply_box_denoise_pixel`, and `apply_unsharp_pixel`; and clamps
`fit_window_to_video` and `locked_window_size` to `max_w` / `max_h`. Four
focused integration tests, 5 `playback_thread` unit tests, and 3 offline fixture
unit tests pass. See `batch-195-validation.json`.

## Batch 196 — Native subtitle dot-decimal/coordinate/entity/ASS-key support, quote-aware `drawbox`/`drawgrid` with negative-thickness rejection, and fallible `f64` resampler history allocation

FVID-0203 accepts both `,` and `.` millisecond separators in `timestamp`,
strips optional trailing coordinate/style tokens (`X1:...`) after the end
timestamp in `try_convert_srt`, folds HTML character entities (`&AMP;`, `&LT;`,
`&GT;`, `&QUOT;`, `&APOS;`, `&NBSP;`) ASCII case-insensitively in `ass_text`,
and supports 1..=3 fractional digits as well as spaced/case-insensitive
`Format:` / `Dialogue:` keys and `[...]` section headers in `try_convert_ass`
(`crates/fvid-media/src/owned_subtitle_impl.rs`); adds quote-aware option
splitting and single/double-quote unquoting, rejects negative thickness `t < 0`
at parse and evaluation time, and uses overflow-safe coordinate arithmetic in
`Draw::selected` (`crates/fvid-media/src/owned_draw.rs`); and reserves history
queue capacity via `try_reserve(1)` before `push_back` in
`crates/fvid-media/src/owned_resample_f64.rs`. Four focused integration tests,
4 unit tests, and 3 offline fixture unit tests pass. See
`batch-196-validation.json`.

## Batch 197 — Player folder/playlist AVI & WavPack admission, bounded LUT/playlist/skin reads, and overflow/NaN guards

FVID-0204 adds `.avi`, `.wv`, and `.wvc` to `playable_path` and `pick_file`
(along with `.m4v` and `.sami` in `pick_file`), skips `[...]` INI section
headers and strips surrounding single/double quotes in `parse_playlist`, bounds
file reads in `read_lut` (16 MiB), `expand_inputs` (4 MiB), and
`player_skin::Skin::load` (64 KiB) via `File::open(...).take(...)`, uses
`at.checked_add(shift)?` when `subtitle_delay_ms < 0` in `Player::subtitle_line`,
and guards `video_rect` against non-positive frame or source dimensions in
`src/player.rs` and `src/player_skin.rs`. One focused integration test, 4
player/skin unit tests, and 3 offline fixture unit tests pass. See
`batch-197-validation.json`.

## Batch 198 — Lossless planner 48-filter & `framestep` step emission, `zscale` refusal, and post-rotation `transformed_aspect`

FVID-0205 emits `filter` steps for all 48 pixel filters executed by
`PixelFilters::apply_colour_clock` in canonical order as well as `framestep` in
`native_plan::transcode_lossless` (`src/native_plan.rs`), requires
`zscale: None` in `native_lossless::supports` (`src/native_lossless.rs`) so
unimplemented `zscale` requests are not silently ignored in `configuration()`,
updates `native_export::transformed_aspect` (`src/native_export.rs`) to apply
`geometry.rotate` canvas dimensions and 90°/270° aspect swaps before `pad` and
`scale`, and uses exact cardinal `sin`/`cos` at 0°/90°/180°/270° in
`RotateAngle::size` (`crates/fvid-media-info/src/video.rs`) matching
`owned_rotate::rotate_rgb_fill`. Three focused integration tests, 3
`fvid-media-info` unit tests, and 2 offline fixture unit tests pass. See
`batch-198-validation.json`.

## Batch 199 — Matroska probe codec/metadata/disposition parity, container `PixelCrop*` insets on identity decode and Y4M export, and canonical PCM codec names in `audio_source_info_selected`

FVID-0206 maps Matroska `A_PCM/INT/LIT`, `A_PCM/INT/BIG`, `A_PCM/FLOAT/IEEE`,
`A_ALAC`, and `A_MPEG/L1` to canonical codec names (`pcm_u8`,
`pcm_s{16,24,32}{le,be}`, `pcm_f{32,64}le`, `alac`, `mp1`) and preserves
file-level custom `SimpleTag`s (`reader.metadata`), `WritingApp` (`encoder`
fallback), track-scoped tags (`reader.track_metadata`), `LanguageIETF` / default
track language (`reader.track_languages`), and `reader.track_dispositions` in
`src/media_probe.rs`; applies container `PixelCrop*` insets (`reader.insets()`)
and post-inset visible dimensions in `decode_video_pipeline_overlay_step`
(`src/native_media.rs`), `export_y4m_sources` (`src/native_export.rs`), and
`PixelFilters::configure_vignette_source` (`src/native_pixels.rs`); and returns
canonical bit-depth-qualified PCM codec names (`pcm_s16le`..`pcm_f64le`) from
`audio_source_info_selected` (`src/native_media.rs`). Three focused integration
tests, 45 related integration tests, and 1 offline fixture unit test pass. See
`batch-199-validation.json`.

## Batch 200 — Matroska FLAC/MPEG container loudness dispatch, Opus/FLAC/ALAC/PCM speaker-mask rematrix admission, QuickTime 0-bit `sowt`/`twos` probe default, and `native_pcm::loudness` in-band PS negotiation

FVID-0207 dispatches `flac` to `decode_matroska_flac_pcm` and `mp1`/`mp2`/`mp3`
to `decode_matroska_mpeg_pcm` in `owned_container_loudness::decode_to_wave`
(`crates/fvid-media/src/owned_container_loudness.rs`); uses
`owned_matroska_opus_export::geometry_with_mask` (`OpusHead::wave_channel_mask()`)
for Matroska Opus and `0` for >2ch Matroska PCM in
`owned_audio_plan::matroska_descriptor` while requiring `mask != 0` on >2ch
rematrixing across `owned_audio_plan`, `owned_audio_export`, and Matroska
Opus/FLAC/ALAC/PCM exporters; defaults omitted QuickTime `sowt`/`twos` sample
size (`bit_depth == 0`) to 16-bit in `owned_mp4_probe_impl.rs`; and negotiates
implicit/in-band PS via `negotiate_mp4_ps`, `negotiate_matroska_ps`, and boxed
`negotiate_adts_aac_reader` in `src/native_pcm/loudness.rs`. Four focused
integration tests, 10 related integration tests, and 1 offline fixture unit test
pass. See `batch-200-validation.json`.

## Batch 201 — WAVE non-zero PCM padding bit rejection, G.711 WAVE and Ogg `plan_decode_audio`/`plan_mix_audio`/`plan_merge_audio` parity, and Ogg/FLAC/MP3 export `supports()` rematrix and budget admission

FVID-0208 rejects non-zero low `bits_per_sample - valid_bits` padding bits in
`read_wave_with_admission` (`crates/fvid-media/src/owned_audio_mix.rs`) and
`measure_for_normalization` (`crates/fvid-media/src/owned_wave_loudness.rs`) to
match `src/native_pcm.rs`; adds G.711 WAVE (`owned_g711_wave_export::descriptor`)
and Ogg Opus/Vorbis/Joint (`owned_ogg_audio_export::descriptor`) support to
`owned_audio_plan::source_descriptor_for_decode`, `plan_decode_audio`, and
`owned_audio_mix::audio_plan`; enforces speaker-mask validation (`mask != 0 &&
rematrix_layout_supported(...)`) on >2ch rematrixing and upfront dangling-symlink
checks (`destination.symlink_metadata().is_ok()`) in `owned_ogg_vorbis_export`,
`owned_ogg_joint_export`, and `owned_g711_wave_export`; and adds `supports()`
with `controlled memory budget exceeded:` preservation and rematrix layout
checks to `owned_raw_flac` and `owned_raw_mp3` (`owned_audio_export.rs`). Four
focused integration tests, 9 related integration/unit tests, and 2 offline
fixture unit tests pass. See `batch-201-validation.json`.

## Batch 202 — Owned and native probe parity for G.711 WAVE, Ogg, bare FLAC, and bare MP1/MP2/MP3, owned loudness/loudnorm pipeline expansion, and 0-sample trimmed MPEG planning rejection

FVID-0209 adds G.711 A-law (`pcm_alaw`) and mu-law (`pcm_mulaw`) inspection to
`owned_wave_inspect::inspect` while preserving `validate_pcm_only()` for linear
PCM WAVE callers; adds `owned_flac_probe`, `owned_mpeg_probe`, and Ogg
Opus/Vorbis/Joint probing across `crates/fvid-media/src/owned_probe.rs` and
`src/media_probe.rs` (preserving Vorbis comment tags and FLAC channel masks);
extends `owned_container_loudness`, `owned_container_loudnorm`, and
`src/native_loudnorm.rs` to plan, measure, and normalize G.711 WAVE, Ogg
Opus/Vorbis/Joint, bare FLAC, and bare MP1/MP2/MP3 via private float32 WAVE
spools; rejects 0-sample Lame-trimmed MPEG streams in `owned_raw_mp3::descriptor`;
and keeps `owned_adts_loudness::supports_plan` and `plan_loudness` strictly
configuration-only without scanning ADTS packets. Three focused integration
tests, 40 related integration tests, and 2 offline fixture unit tests pass. See
`batch-202-validation.json`.

## Batch 203 — G.711 WAVE `native_pcm::is_wave` disambiguation, multitrack Matroska/MP4 secondary audio codec routing in `media` and `native_loudnorm`, and `media_cli` owned audio dispatch without legacy features

FVID-0210 updates `native_pcm::is_wave` (`src/native_pcm.rs`) to return
`Ok(false)` for valid RIFF WAVE files whose `fmt ` chunk uses G.711 A-law
(`tag=6`) or mu-law (`tag=7`) companding so top-level `fvid::media`,
`fvid::native_audio_mix`, and `fvid::native_loudnorm` delegate G.711 WAVE to
`fvid_media` instead of failing in linear PCM inspection; adds
`uses_native_audio_route` in `src/media.rs` and per-stream `audio_source_info_selected`
checks in `src/native_loudnorm.rs` (`try_plan`, `try_apply`) so multitrack
Matroska (`A_FLAC`, `A_OPUS`, `A_MPEG/*`) and MP4/QuickTime (`ima4` and other
ADPCM) containers with `--streams` selecting a secondary track route to
`fvid_media` and `owned_container_loudnorm`; and updates `src/media_cli.rs` to
dispatch `plan decode-audio`, `.wav` `decode-audio`, `plan mix-audio`, `.wav`
`mix-audio`, `plan merge-audio`, and `.wav` `merge-audio` through `fvid::media`
when `native_export` / `native_audio_mix` do not handle the input format. Three
focused integration tests, 43 related integration tests, and 1 offline fixture
unit test pass. See `batch-203-validation.json`.

## Batch 204 — Audio trim, loudness, and constant-gain normalization routing across owned audio formats and multitrack containers

FVID-0211 delegates `owned_wave_remux::trim` and `owned_wave_plan::plan_trim` to
`owned_audio_export::decode_audio_transformed` and
`owned_audio_plan::plan_decode_audio` with `interval: Some((from, to))` whenever
linear PCM WAVE remux does not apply (`!supports`), enabling `.wav`/`.mka`/`.mkv`
trim and `plan_trim` for G.711 WAVE, Ogg Opus/Vorbis/Joint, bare FLAC/MP3, and
Matroska/MP4 audio tracks; updates `media::trim` and `media::plan_trim` to verify
`native_media::audio_source_info_selected` for single-stream `--streams`
selections so secondary Matroska (`A_FLAC`, `A_OPUS`, `A_MPEG/*`) and MP4
(`ima4`) tracks delegate to `fvid_media`; extends `native_pcm::loudness`,
`native_pcm::normalize`, and `native_plan::loudness` (`normalize_loudness`) to
handle G.711 WAVE, Ogg, bare FLAC/MP3, and secondary Matroska/MP4 tracks via
`owned_container_loudness::decode_to_wave`, `fvid_media::decode_audio_transformed`,
and `fvid_media::plan_decode_audio`; and routes `plan trim` and `trim` in
`src/media_cli.rs` through `fvid::media` without requiring `--features media`.
Three focused integration tests, 35 related integration tests, and 1 offline
fixture unit test pass. See `batch-204-validation.json`.

## Batch 205 — Controlled-memory loudnorm admission across ADTS/MP4/Matroska, Matroska audio mix/merge, and sample-domain audio concat across owned formats

FVID-0212 updates `native_loudnorm::try_plan` and `native_loudnorm::try_apply`
(`src/native_loudnorm.rs`) to delegate to `fvid_media::owned_adts_loudnorm` and
`fvid_media::owned_container_loudnorm` when `options.max_controlled_bytes.is_some()`,
enabling `fvid media plan loudnorm` and `fvid media loudnorm` with
`--max-memory-mib` across ADTS, MP4, and Matroska inputs; adds
`source_geometry_with_mask` (`crates/fvid-media/src/owned_audio_plan.rs`) and
extends `src/native_audio_mix.rs` (`eligible`, `plan`, `mix_audio`, `plan_merge`,
`merge_audio`) to validate and decode all single-stream `fvid_media` audio
formats (G.711 WAVE, Ogg Opus/Vorbis/Joint, bare FLAC/MP3, Matroska
FLAC/Opus/MPEG, MP4 ADPCM) for both `.wav` and `.mka`/`.mkv` outputs while
keeping multi-audio containers without explicit stream selection ineligible; and
extends `src/native_audio_mix/concat.rs`, `src/native_plan.rs`, and
`src/native_export.rs` (`crate::media::probe`) so sample-domain audio
concatenation to `.mka`/`.mkv` works across all single-stream owned audio
formats. Three focused integration tests, 34 related integration tests, and 1
offline fixture unit test pass. See `batch-205-validation.json`.

## Batch 206 — `.f32le`, `.mka`, and `.mkv` audio export, trim, and loudness normalization across all owned `fvid_media` audio formats

FVID-0213 adds `decode_fvid_media_audio_to_destination` (`src/media.rs`) so
`fvid::media::decode_audio`, `fvid::media::decode_audio_interval`, and
`fvid::media::decode_audio_transformed` decode `fvid_media` audio sources
(G.711 A-law/mu-law WAVE, Ogg Opus/Vorbis/Joint, bare FLAC/MP3, and secondary
Matroska FLAC/Opus/MPEG or MP4 ADPCM tracks) to a private temporary WAVE spool
when the destination is `.f32le` (or `.mka`/`.mkv` for standalone formats) and
publish the final `.f32le`, `.mka`, or `.mkv` output via
`native_export::export_audio_pcm_transformed_with_controls`; extends
`fvid::media::trim` (`src/media.rs`) to support `.f32le`, `.mka`, and `.mkv`
outputs across all `fvid_media` audio sources once `fvid_media::plan_decode_audio`
admits the interval; updates `fvid::native_pcm::normalize_loudness_file_controlled`
(`src/native_pcm/normalize.rs`) to use `decode_fvid_media_audio_to_destination`
so `.mka` and `.mkv` `normalize-loudness` outputs succeed on G.711 WAVE, Ogg,
bare FLAC/MP3, and secondary Matroska/MP4 tracks; and extends `decode-audio`
admission in `src/media_cli.rs` with
`fvid_media::owned_container_loudness::recognizes_standalone_audio` so
`fvid media decode-audio` supports `.f32le`, `.mka`, and `.mkv` outputs across
all standalone owned audio formats without `--features media`. Three focused
integration tests, 16 related integration tests, and 1 offline fixture unit test
pass. See `batch-206-validation.json`.

## Batch 207 — Packet, memory, and RSS budget enforcement across `decode_audio`, `trim`, `trim_pcm`, `loudness`, `normalize_loudness`, and `media_cli`

FVID-0214 routes `fvid::media::uses_native_audio_route` (`src/media.rs`) to
`fvid_media` when `options.max_controlled_bytes.is_some()`, enabling
controlled-memory-budget admission and execution for `decode_audio`,
`decode_audio_transformed`, and `plan_decode_audio` across PCM WAVE, ADTS, MP4,
Matroska, G.711 WAVE, Ogg, and bare FLAC/MP3; delegates `fvid::media::trim_pcm`
and `fvid::media::plan_trim_pcm` (`src/media.rs`) to `fvid_media::trim_pcm` and
`fvid_media::plan_trim_pcm` whenever custom budgets or WAVE metadata edits are
requested; adds `trim_audio_pcm_with_controls` (`src/native_export.rs`),
`measure_loudness_file_with_options` (`src/native_pcm/loudness.rs`),
`normalize_loudness_file_with_options` (`src/native_pcm/normalize.rs`),
`loudness_with_options`, and `normalize_loudness_with_options`
(`src/native_plan.rs`) so `fvid::media::trim`, `plan_trim`, `measure_loudness`,
`plan_loudness`, `normalize_loudness`, and `plan_normalize_loudness` enforce
`max_packets`, `max_packet_bytes`, `max_controlled_bytes`, and `max_rss_bytes`;
and extends `src/media_cli.rs` (`parse_native_audio_limit`, `decode-audio`,
`plan decode-audio`, `try_native_audio_trim`, `loudness`, `plan loudness`,
`normalize-loudness`, and `plan normalize-loudness`) to accept `--max-packets`,
`--max-packet-bytes`, `--max-memory-mib`, and `--max-rss-mib` across all owned
audio formats without `--features media`. Three focused integration tests, 15
related integration tests, and 1 offline fixture unit test pass. See
`batch-207-validation.json`.

## Batch 208 — Packet, memory, and RSS budget and metadata enforcement across `remux`, `plan_remux`, `concat`, `plan_concat`, and `media_cli`

FVID-0215 delegates `fvid::media::remux`, `plan_remux`, `concat`, and
`plan_concat` (`src/media.rs`) to `fvid_media` for linear PCM WAVE inputs
whenever custom packet/memory/RSS budgets or WAVE INFO metadata edits are
requested; adds `write_adts_aac_reader_limited` and
`concat_adts_readers_limited` (`src/container/mp4_write.rs`),
`open_with_packet_limit` and `write_limited` (`src/container/mp4_concat.rs`),
`remux_adts_aac_with_options`, `concat_adts_aac_with_options`, and
`try_concat_mp4_matroska_with_options` (`src/native_export.rs`),
`concat_adts_with_options`, `concat_matroska_to_with_options`, and
`concat_mp4_matroska_with_options` (`src/native_plan.rs`), and
`eligible_with_options`, `decode_source_to_f32le_with_options`, and
`concat_audio_with_options` (`src/native_audio_mix.rs`,
`src/native_audio_mix/concat.rs`) so `max_packets`, `max_packet_bytes`,
`max_controlled_bytes`, and `max_rss_bytes` are enforced across ADTS, MP4,
Matroska, and sample-domain audio concatenation; and wires `--max-packet-bytes`,
`--max-packets`, `--max-memory-mib`, `--max-rss-mib`, `--metadata`,
`--metadata-delete`, `--stream-metadata`, `--stream-metadata-delete`, and
`--quiet` into `remux`, `plan remux`, `concat`, and `plan concat` in
`src/media_cli.rs` without requiring `--features media`. Four focused
integration tests, 44 related integration tests, and 1 offline fixture unit test
pass. See `batch-208-validation.json`.

## Batch 209 — RSS and packet-byte budget and metadata enforcement across `transcode_lossless`, `crop-lossless`, `overlay`, `xfade`, `burn-subtitles`, and `media_cli`

FVID-0216 allows `max_rss_bytes` in `owned_lossless::policy`
(`crates/fvid-media/src/owned_lossless.rs`) and `owned_mp4_video_bridge::prepare`
(`crates/fvid-media/src/owned_mp4_video_bridge.rs`) and enforces
`owned_budget::check_rss_budget(options)` across `plan_transcode_lossless`,
`transcode_lossless`, `owned_ffv1_export`
(`crates/fvid-media/src/owned_ffv1_export.rs`),
`owned_matroska::export_y4m_ffv1_policy`
(`crates/fvid-media/src/owned_matroska.rs`), `owned_mp4_video_decode`
(`crates/fvid-media/src/owned_mp4_video_decode.rs`),
`owned_mp4_multitrack_export`
(`crates/fvid-media/src/owned_mp4_multitrack_export.rs`), and
`owned_webm_multitrack_export`
(`crates/fvid-media/src/owned_webm_multitrack_export.rs`); extends
`owned_lossless_command` (`src/media_cli.rs`) to handle `crop-lossless`
(preserving `--crop` requirement and non-crop transform refusals),
`--max-packet-bytes`, `--max-rss-mib`, and `--overlay`/`--overlay-x`/`--overlay-y`
without `--features media`; extends `try_owned_overlay` (`src/media_cli.rs`) to
accept `--max-packets`, `--max-packet-bytes`, `--max-rss-mib`, `--streams`, and
metadata flags and delegate controlled `overlay` and `plan overlay` requests to
`fvid::media::overlay_video` and `fvid::media::plan_overlay`; and wires
`--max-packet-bytes` and `--max-rss-mib` into `owned_xfade_command` and
`owned_subtitle_burn_command` (`src/media_cli.rs`). One focused integration test,
53 related unit and integration tests, and 1 offline fixture unit test pass. See
`batch-209-validation.json`.

## Batch 210 — Budgets and metadata controls across Y4M export/concat/trim, `burn-subtitles`, `xfade`, and `media_cli`

FVID-0217 adds `validate_y4m_options`, `export_y4m_pipeline_with_options`,
`concat_y4m_with_options`, `trim_y4m_with_options`, and
`export_y4m_sources_with_options` in `src/native_export.rs` alongside
`concat_y4m_with_options` and `trim_y4m_with_options` in `src/native_plan.rs`,
and wires `fvid::media::trim`, `plan_trim_y4m`, `concat`, and `plan_concat_y4m`
(`src/media.rs`) to enforce `max_packets`, `max_packet_bytes`, and
`max_rss_bytes` on Y4M exports, trims, and concatenations; extends
`owned_subtitle_burn` (`crates/fvid-media/src/owned_subtitle_burn.rs`) to allow
`metadata_set`, `metadata_delete`, `stream_metadata_set`, and
`stream_metadata_delete`, enforce `max_packet_bytes != 0`, and write Matroska
container and video-stream tags via
`PacketWriter::new_ffv1_with_track_description`; extends `owned_xfade`
(`crates/fvid-media/src/owned_xfade.rs`) to admit explicit `[0]` stream
selection, require `max_packet_bytes != 0`, and enforce
`owned_budget::check_rss_budget(options)` in `plan_xfade`; and extends
`src/media_cli.rs` so `export-y4m`, `try_video_concat`, and `try_video_trim`
accept `--max-packets`, `--max-packet-bytes`, `--max-rss-mib`, `--quiet`, and
`--progress`, `owned_xfade_command` accepts `--streams`, and
`owned_subtitle_burn_command` accepts `--metadata`, `--metadata-delete`,
`--stream-metadata`, and `--stream-metadata-delete`. One focused integration
test, 36 related unit and integration tests, and 1 offline fixture unit test
pass. See `batch-210-validation.json`.

## Batch 211 — Budgets, cancellation, and progress controls across `mix-audio`, `merge-audio`, `convert-subtitles`, and `media_cli`

FVID-0218 adds `eligible_with_options`, `plan_with_controls`,
`mix_audio_with_controls`, `plan_merge_with_controls`, and
`merge_audio_with_controls` in `src/native_audio_mix.rs` to enforce
`max_packets`, `max_packet_bytes`, `max_controlled_bytes`, `max_rss_bytes`,
`cancel`, and `progress` across `mix-audio`, `plan mix-audio`, `merge-audio`,
and `plan merge-audio`; adds `try_convert_controlled` in
`crates/fvid-media/src/owned_subtitle_impl.rs` (shared by
`crates/fvid-media/src/owned_subtitle.rs` and `src/native_subtitle.rs`) to
enforce `max_packet_bytes`, `max_packets`, `max_rss_bytes`, `cancel`, and
`progress` across SRT, ASS, and Matroska subtitle conversion; and wires
`--max-packets`, `--max-packet-bytes`, `--max-memory-mib`, `--max-rss-mib`,
`--progress`, and `-q`/`--quiet` into `native_mix`, `native_merge`, and
`convert-subtitles` in `src/media_cli.rs`. One focused integration test, 19
related integration tests, and 1 offline fixture unit test pass. See
`batch-211-validation.json`.

## Batch 212 — RSS budgets and metadata/stream controls across MP4, Matroska, and ADTS `remux`, `plan_remux`, `concat`, and `plan_concat`

FVID-0219 replaces `options.max_rss_bytes.is_none()` in
`owned_mp4_remux::policies` (`crates/fvid-media/src/owned_mp4_remux.rs`) and
`owned_matroska_remux::unedited_policies`
(`crates/fvid-media/src/owned_matroska_remux.rs`) with
`crate::owned_budget::check_rss_budget(options).is_ok()` and enforces
`check_rss_budget(options)` across `owned_mp4_remux::remux` and `plan_remux`,
`owned_matroska_remux::remux` and `publish`, `owned_adts_remux::remux` and
`concat`, `owned_concat::concat` and `plan_concat`, and
`owned_remux_plan::plan_remux`; updates
`fvid::media::validate_matroska_copy_options` (`src/media.rs`) to enforce
`fvid_media::owned_budget::check_rss_budget(options)` and admit `max_rss_bytes`;
and routes `fvid::media::plan_remux` to `fvid_media::plan_remux` for MP4 sources
when custom packet/RSS budgets or container/stream metadata edits are requested.
One focused integration test, 33 related integration tests, and 1 offline
fixture unit test pass. See `batch-212-validation.json`.

## Batch 213 — WAVE `INFO` metadata mutations across `decode-audio` and `trim` and controlled `mix-audio` / `merge-audio` in `fvid::media` and `media_cli`

FVID-0220 updates `fvid::media::uses_native_audio_route`, `trim`, and
`plan_trim` (`src/media.rs`) to delegate WAVE `INFO` container metadata
mutations (`metadata_set`, `metadata_delete`) to
`fvid_media::decode_audio_transformed`, `plan_decode_audio`, `trim`, and
`plan_trim` (while keeping headerless `.f32le` and Matroska PCM decode targets
rejected when metadata mutations are requested); adds public
`fvid::media::mix_audio_with_controls`, `plan_mix_audio_with_controls`,
`merge_audio_with_controls`, and `plan_merge_audio_with_controls`; and wires
`--metadata` and `--metadata-delete` into `decode-audio`, `plan decode-audio`,
and `try_native_audio_trim` (`trim`, `plan trim`, `trim-pcm`, `plan trim-pcm`)
plus controlled `mix-audio`/`merge-audio` calls in `src/media_cli.rs`. One
focused integration test, 44 related integration tests, and 1 offline fixture
unit test pass. See `batch-213-validation.json`.

## Batch 214 — WAVE `INFO` metadata preservation and mutations across `loudnorm` and `normalize-loudness` and `fvid::media::{plan_loudnorm, apply_loudnorm, apply_loudnorm_dual}` routing

FVID-0221 preserves source `INFO` tags in `decode_to_wave` across
`owned_ogg_opus_export.rs`, `owned_ogg_vorbis_export.rs`,
`owned_ogg_joint_export.rs`, and `owned_g711_wave_export.rs`; admits and
validates WAVE `INFO` metadata mutations (`metadata_set`, `metadata_delete`),
emits a `metadata` plan step, strips metadata mutations during decode/loudness
preflight, and writes edited WAVE `INFO` chunks via
`owned_wave_metadata::edit_info_chunks` across `owned_loudnorm.rs`,
`owned_loudnorm/report.rs`, `owned_adts_loudnorm.rs`,
`owned_container_loudnorm.rs`, and `src/native_loudnorm.rs`; adds public
`fvid::media::{plan_loudnorm, apply_loudnorm, apply_loudnorm_dual}` wrappers in
`src/media.rs` routing through `native_loudnorm::{try_plan, try_apply}` before
`fvid_media`; supports WAVE `INFO` metadata mutations in
`native_pcm::normalize_file_with_options` (`src/native_pcm/normalize.rs`) and
`native_plan::normalize_loudness_with_options` (`src/native_plan.rs`); and wires
repeatable `--metadata KEY=VALUE` and `--metadata-delete KEY` into `loudnorm`,
`plan loudnorm`, `normalize-loudness`, and `plan normalize-loudness` in
`src/owned_loudnorm_cli.rs` and `src/media_cli.rs`. One focused integration
test, 21 related integration tests, and 1 offline fixture unit test pass. See
`batch-214-validation.json`.

## Batch 215 — `CopyOptions` budgets, stream selection, and metadata mutations in `plan_transcode_lossless` and CLI routing for `plan crop-lossless` and filtered `overlay` with controls

FVID-0222 adds `plan_selection` to `owned_mp4_multitrack_export.rs` and
`owned_webm_multitrack_export.rs` (and returns an explicit error on
out-of-range stream metadata indices in MP4 multitrack qualification); passes
`options.max_packet_bytes` to `Mp4Reader::open` in
`owned_mp4_video_bridge::prepare_selected`; enforces multitrack and
single-stream MP4/WebM/Y4M `max_packets` and `max_packet_bytes` preflight,
selected stream order and `primary_video`/`copy` dispositions, `metadata` plan
steps, and budget notes in `owned_lossless::plan_transcode_lossless`; and routes
CLI `plan crop-lossless` and filtered `overlay` / `plan overlay` with custom
`CopyOptions` controls through `owned_lossless_command_with_name` in
`src/media_cli.rs`. One focused integration test, 39 related integration tests,
and 1 offline fixture unit test pass. See `batch-215-validation.json`.

## Batch 216 — WAVE `INFO` and Matroska container/stream metadata mutations and stream selection across `mix-audio`, `merge-audio`, and Matroska `concat`

FVID-0223 adds `publish_pcm_matroska_with_metadata` in
`src/native_export/pcm_matroska.rs` and `write_wave_f32_with_info_options` in
`src/native_audio_mix.rs`; admits and validates container/stream metadata
mutations (`metadata_set`, `metadata_delete`, `stream_metadata_set`,
`stream_metadata_delete`) and `--streams 0` across `mix-audio`, `merge-audio`,
and PCM Matroska `concat` in `src/native_audio_mix.rs` and
`src/native_audio_mix/concat.rs` (preserving and editing first-input WAVE `INFO`
chunks on `.wav` outputs while rejecting stream metadata on `.wav` outputs);
adds `container::mp4_concat::write_with_options` in
`src/container/mp4_concat.rs` supporting stream selection/reordering and
container/stream metadata mutations and updates
`native_export::try_concat_mp4_matroska_with_options`,
`native_plan::{concat_matroska_to_with_options, concat_mp4_matroska_with_options}`,
and `fvid::media::validate_concat_matroska_options` (including admitting `.mka`
output when `options.streams` selects only audio tracks from an A/V MP4); and
wires `--streams`, `--metadata`, `--metadata-delete`, `--stream-metadata`, and
`--stream-metadata-delete` into `native_mix`, `native_merge`,
`try_mp4_matroska_concat`, and `run_native` in `src/media_cli.rs`. One focused
integration test, 15 related integration tests, and 1 offline fixture unit test
pass. See `batch-216-validation.json`.

## Batch 217 — Matroska container and stream metadata mutations across ADTS `remux` and `concat` and across Matroska PCM audio `decode-audio` and `trim`

FVID-0224 adds `write_adts_with_options`, `concat_adts_with_options`, and
`extract_adts_matroska_metadata` in
`crates/fvid-media/src/owned_matroska_adts_impl.rs` and preserves track
`name`/`language` in `PacketWriter::rewrite_adts_rate`
(`crates/fvid-media/src/owned_matroska_packet_impl.rs`); admits and validates
Matroska container and stream metadata mutations (`metadata_set`,
`metadata_delete`, `stream_metadata_set`, `stream_metadata_delete`) and emits a
`metadata` plan step in `owned_adts_remux.rs`, `owned_concat.rs`,
`native_export::{remux_adts_aac_with_options, concat_adts_aac_with_options}`,
`native_plan::concat_adts_with_options`, and
`fvid::media::{validate_adts_copy_options_with_metadata, plan_concat}`; wires
`CopyOptions` into
`native_export::pcm_matroska::{Output::new_with_options, Sink::new, validate_matroska_pcm_metadata}`,
`native_export::PcmExportLimits`, and
`fvid::media::{decode_fvid_media_audio_to_destination, validate_owned_audio_options_for_destination, plan_decode_audio, trim, plan_trim}`
so `decode-audio` and `trim` to `.mkv`/`.mka` support container and stream
metadata mutations while WAVE `.wav` and headerless `.f32le` destinations
continue to reject unsupported metadata; and wires `--stream-metadata` and
`--stream-metadata-delete` into `decode-audio`, `plan decode-audio`,
`try_native_audio_trim`, and `try_native_audio_concat` in `src/media_cli.rs`.
One focused integration test, 6 related integration tests, and 1 offline fixture
unit test pass. See `batch-217-validation.json`.

## Batch 218 — `CopyOptions` packet/byte budgets, stream metadata deletions, and metadata/stream plan steps across `burn-subtitles` and `xfade` in API and CLI

FVID-0225 updates `owned_subtitle_burn::plan_burn_subtitles`
(`crates/fvid-media/src/owned_subtitle_burn.rs`) to enforce cancellation,
`max_packets`, and `max_packet_bytes` on both Y4M and FFV1 Matroska sources,
populate `streams` with probed video stream `0` (`disposition: "primary_video"`),
emit a `metadata` plan step when container or stream metadata mutations are
requested, and record budget notes; updates `owned_xfade::plan_xfade`
(`crates/fvid-media/src/owned_xfade.rs`) to enforce cancellation, `max_packets`,
and `max_packet_bytes` (including FFV1 encoded packet size preflight via
`visit_y4m_counted`), emit a `metadata` plan step, and record budget notes;
applies `options.stream_metadata_delete` before `options.stream_metadata_set` in
`owned_subtitle_burn::burn_subtitles` (Y4M path) and
`owned_xfade::write_y4m_ffv1_counted`; and updates `owned_xfade_command` and
`owned_subtitle_burn_command` in `src/media_cli.rs` to accept `-q`, reject
duplicate non-metadata flags, and reject `--progress` during `plan`. One focused
integration test, 2 related integration tests, and 1 offline fixture unit test
pass. See `batch-218-validation.json`.

## Batch 219 — Matroska container and stream metadata mutations and per-cue packet limits across `convert-subtitles` and `normalize-loudness` in API and CLI

FVID-0226 adds `try_convert_with_controls`, `apply_subtitle_matroska_metadata`,
and `publish_events_with_metadata` in
`crates/fvid-media/src/owned_subtitle_impl.rs` (plus public
`convert_subtitles_with_controls` in `crates/fvid-media/src/lib.rs` and
`src/media.rs`), enforcing per-cue `max_packet_bytes` and pre-publication
`max_packets`/cancellation/RSS checks across SRT, ASS, and Matroska subtitle
conversions while preserving and mutating Matroska container and stream `0`
metadata; updates `native_pcm::normalize_file_with_options`
(`src/native_pcm/normalize.rs`) and `native_plan::normalize_loudness_with_options`
(`src/native_plan.rs`) to validate and apply Matroska container and stream `0`
metadata mutations on `.mka`/`.mkv` outputs while keeping WAVE `INFO` validation
on `.wav` outputs; and wires `--metadata`, `--metadata-delete`,
`--stream-metadata`, `--stream-metadata-delete`, and `-q` into
`convert-subtitles`, `normalize-loudness`, and `plan normalize-loudness` in
`src/media_cli.rs`. One focused integration test, 8 related integration tests,
and 1 offline fixture unit test pass. See `batch-219-validation.json`.

## Batch 220 — Custom container and stream text metadata tags across MP4-to-Matroska `remux` and `plan_remux` and stream metadata deletions in Y4M and multitrack MP4 lossless exports

FVID-0227 updates `owned_mp4_remux::{policies, track_key, track_metadata, metadata, remux, plan_remux}`
(`crates/fvid-media/src/owned_mp4_remux.rs`) and adds
`write_selected_with_text_metadata_overrides` in
`crates/fvid-media/src/owned_mp4_matroska_impl.rs` so MP4-to-Matroska `remux`
and `plan_remux` admit and emit custom container `SimpleTag` entries (`<= 128`
key bytes, `<= 1024` value bytes, NUL-free) and stream-scoped `SimpleTag`
entries (excluding reserved `rotate`/`stereo_mode`/`alpha_mode`) mapped to the
selected output track order; and applies `metadata_delete` and
`stream_metadata_delete` in `owned_matroska::write_y4m_ffv1_policy`
(`crates/fvid-media/src/owned_matroska.rs`) and companion audio stream tag
deletions in `owned_mp4_multitrack_export::try_export`
(`crates/fvid-media/src/owned_mp4_multitrack_export.rs`). One focused
integration test, 4 related integration tests, and 1 offline fixture unit test
pass. See `batch-220-validation.json`.

## Batch 221 — Short `-q`/`--quiet` flag and `--progress`-during-`plan` parity across owned media CLI command and plan routes

FVID-0228 updates `owned_loudnorm_cli::{run, run_plan}`
(`src/owned_loudnorm_cli.rs`) and `src/media_cli.rs` (`plan loudness`,
`loudness`, `plan decode-audio`, `decode-audio`, `plain_decode`,
`try_native_audio_trim`, `try_owned_overlay`, `owned_lossless_command_with_name`,
and the `transcode`, `transcode-lossless`, `crop-lossless`, and `plan remux`
fast paths) to accept both `-q` and `--quiet` and normalize `-q` to `"--quiet"`
in duplicate-option tracking, and accepts `--progress` as a no-op during `plan`
in `try_owned_overlay`, `owned_lossless_command_with_name`,
`owned_xfade_command`, `owned_subtitle_burn_command`, `try_video_concat`, and
`try_video_trim`. One focused integration test, 3 related integration tests, and
1 offline fixture unit test pass. See `batch-221-validation.json`.

## Batch 222 — Vorbis comment metadata inspection and WAVE `INFO` preservation across bare FLAC (`probe`, `decode-audio`, `trim`, `loudnorm`) and Ogg (`probe`)

FVID-0229 adds `vorbis_comments`, `probe_metadata_from_entries`, and
`wave_info_from_extra` in `crates/fvid-media/src/owned_flac_layout.rs` and
`Flac::metadata_blocks` in `crates/fvid-media/src/owned_flac.rs`; updates
`owned_flac_probe::probe_flac` (`crates/fvid-media/src/owned_flac_probe.rs`)
and `owned_probe::probe_ogg` (`crates/fvid-media/src/owned_probe.rs`) to expose
canonical container and stream `0` Vorbis comment metadata (`title`, `artist`,
`album`, `date`, `comment`, `genre`, `copyright`, `encoder`, `track`,
`album_artist`, `disc`, `publisher`, `description`, `rating`, `language`, and
custom tags, while excluding `waveformatextensible_channel_mask`); and appends
source FLAC Vorbis comments as RIFF `INFO` chunks to the spooled WAVE file in
`owned_raw_flac::apply` (`crates/fvid-media/src/owned_raw_flac.rs`) and
`owned_container_loudness::decode_to_wave`
(`crates/fvid-media/src/owned_container_loudness.rs`) so `decode-audio`,
`trim`, and single/dual-pass `loudnorm` from `.flac` to `.wav` preserve and
selectively edit/delete source FLAC tags. Two focused integration tests, 9
related unit/integration tests, and 1 offline fixture unit test pass. See
`batch-222-validation.json`.

## Batch 223 — ID3v2/ID3v1 metadata inspection and WAVE `INFO` preservation across bare MPEG audio (`probe`, `decode-audio`, `trim`, `loudnorm`)

FVID-0230 adds `Mp3::probe_metadata` and `Mp3::wave_info` (supporting ID3v2.3 and
ID3v2.4 text, `COMM`, and `TXXX` frames across ISO-8859-1, UTF-16 with BOM,
UTF-16BE, and UTF-8 encodings plus ID3v1/v1.1 fallback) in
`crates/fvid-media/src/owned_mpeg_audio.rs`; updates
`owned_mpeg_probe::probe_mpeg` (`crates/fvid-media/src/owned_mpeg_probe.rs`) to
expose canonical container and stream `0` metadata (`title`, `artist`, `album`,
`date`, `comment`, `genre`, `copyright`, `encoder`, `track`, `album_artist`,
`disc`, `publisher`, `description`, `language`, and custom `TXXX` tags); and
appends source ID3 tags as RIFF `INFO` chunks to the spooled WAVE file in
`owned_raw_mp3::apply` (`crates/fvid-media/src/owned_raw_mp3.rs`) and
`owned_container_loudness::decode_to_wave`
(`crates/fvid-media/src/owned_container_loudness.rs`) so `decode-audio`, `trim`,
and single/dual-pass `loudnorm` from `.mp3` to `.wav` preserve and selectively
edit/delete source ID3 metadata. Two focused integration tests, 6 related
integration tests, and 2 offline fixture unit tests pass. See
`batch-223-validation.json`.

## Batch 224 — Matroska (`.mka`/`.mkv`) output and container/stream metadata mutations across `loudnorm` and `plan loudnorm` in API and CLI

FVID-0231 updates `native_loudnorm::try_plan` (`src/native_loudnorm.rs`) to
validate Matroska container and stream `0` metadata via
`validate_matroska_pcm_metadata` and insert a `metadata` plan step before
`write` when custom container or stream metadata mutations are requested;
updates `native_loudnorm::try_apply` (`src/native_loudnorm.rs`) to support
`.mka` and `.mkv` destinations across WAVE, ADTS AAC, and container/standalone
audio sources by normalizing to a private temporary WAVE spool and publishing
float32 Matroska PCM with container and stream `0` metadata via
`export_audio_pcm_transformed_with_controls` (while rejecting stream metadata
mutations on `.wav` outputs); and adds `--stream-metadata` and
`--stream-metadata-delete` to `owned_loudnorm_cli::{run, run_plan}` and
`--progress` no-op handling during `plan loudnorm` in
`src/owned_loudnorm_cli.rs`. One focused integration test, 7 related integration
tests, and 2 offline fixture unit tests pass. See `batch-224-validation.json`.

## Batch 225 — `.wav` output and WAVE `INFO` / Matroska metadata mutations across sample-domain audio `concat` and `plan concat` in API and CLI

FVID-0232 updates `native_audio_mix::concat::{eligible_with_options, concat_audio_with_options}`
(`src/native_audio_mix/concat.rs`) to accept `.wav`, `.mka`, and `.mkv`
destinations, preserve shared speaker masks across segments, and emit float32
WAVE headers with RIFF `INFO` metadata chunks while rejecting stream metadata
mutations on `.wav` outputs; updates
`native_export::try_concat_mp4_matroska_with_options` (`src/native_export.rs`)
to route `.wav` destinations to sample-domain PCM concat whenever lossless
byte-preserving PCM WAVE concat is not applicable (such as mixed compressed/PCM
inputs, G.711 WAVE, or heterogeneous PCM bit depths like 16-bit + 24-bit WAVE);
and updates `native_plan::concat_matroska_to_with_options`
(`src/native_plan.rs`), `media::plan_concat` (`src/media.rs`), and
`media_cli::try_mp4_matroska_concat` (`src/media_cli.rs`) to support `.wav`
concat outputs, `--output-format wav` in `plan concat`, and automatic
sample-domain fallback for heterogeneous-depth WAVE or Matroska-tagged WAVE
plans while keeping lossless byte-preserving WAVE concat for compatible PCM WAVE
inputs. One focused integration test, 10 related integration tests, and 1
offline fixture unit test pass. See `batch-225-validation.json`.

## Batch 226 — Preserve and selectively edit source RIFF `INFO`, MP4/MOV/M4A, and Matroska/WebM/MKA tags across WAVE audio exports, `trim`, `loudnorm`, and `normalize-loudness`

FVID-0233 adds `info_for_file_tags`, `mp4_source_info`, and
`matroska_source_info` in `crates/fvid-media/src/owned_wave_metadata.rs`;
appends source MP4 and Matroska RIFF `INFO` chunks (including container tags,
track fallback title, track language `ILNG`, segment `WritingApp` encoder, and
4-CC container/track tags) to the decoded WAVE spool in
`owned_mp4_audio_export::apply`,
`owned_matroska_{aac,opus,flac,alac,mp3,pcm}_export::apply`, and
`owned_container_loudness::decode_to_wave`, while preserving `info_chunks`
across spatial AAC fold-down in `owned_aac_export_routing::export`; and updates
`native_export::export_pcm_selected` (`src/native_export.rs`) to preserve and
selectively edit source WAVE `LIST`/`INFO` chunks (including opaque 4-CC tags)
and MP4/Matroska container/track tags on `.wav` outputs across `decode-audio`,
`trim`, `loudnorm`, and `normalize-loudness`. One focused integration test, 7
related integration tests, and 1 offline fixture unit test pass. See
`batch-226-validation.json`.

## Batch 227 — Preserve and selectively edit source WAVE, MP4/MOV/M4A, Matroska/WebM/MKA, and FLAC/Ogg/MP3 container and stream `0` metadata across Matroska PCM audio exports, `trim`, `loudnorm`, and `normalize-loudness`

FVID-0234 adds `MatroskaPcmMetadata`, `default_matroska_pcm_metadata`,
`extract_source_pcm_metadata`, `Output::new_with_source_metadata`, and
`Sink::new_with_source_metadata` in `src/native_export/pcm_matroska.rs`, and
updates `parse_matroska_pcm_metadata(base, options)` to apply container
(`metadata_delete`, `metadata_set`) and stream `0` (`stream_metadata_delete`,
`stream_metadata_set`) mutations (including case-insensitive custom tag
deletion) on top of source metadata; adds
`export_audio_pcm_transformed_with_source_metadata` and
`source_matroska_metadata` in `src/native_export.rs`; and updates
`media::decode_fvid_media_audio_to_destination` (`src/media.rs`) and
`native_loudnorm::try_apply` (`src/native_loudnorm.rs`) to extract
`source_matroska_metadata` from the original source before spooling to WAVE and
publishing `.mka`/`.mkv`. One focused integration test, 5 related integration
tests, and 1 offline fixture unit test pass. See `batch-227-validation.json`.

## Batch 228 — Preserve and selectively edit first-segment source WAVE, MP4/MOV/M4A, Matroska/WebM/MKA, and FLAC/Ogg/MP3 metadata and enforce WAVE controlled memory budgets across sample-domain audio `concat` and `plan concat`

FVID-0235 adds `extract_source_wave_info` in
`src/native_export/pcm_matroska.rs` and updates
`native_audio_mix::concat::concat_audio_with_options`
(`src/native_audio_mix/concat.rs`) to preserve and selectively edit
first-segment `source_info` chunks on `.wav` concat outputs and first-segment
`source_matroska_metadata` on `.mka`/`.mkv` concat outputs; enforces
`options.max_controlled_bytes` against `estimate_float_wave_export_bytes` for
PCM WAVE inputs in `owned_audio_plan::source_descriptor_for_decode`
(`crates/fvid-media/src/owned_audio_plan.rs`); and updates `media::plan_concat`
(`src/media.rs`) to return `fvid_media::plan_concat` directly for compatible PCM
WAVE inputs unless Matroska-only metadata mutations are requested. One focused
integration test, 11 related integration tests, and 1 offline fixture unit test
pass. See `batch-228-validation.json`.

## Batch 229 — Preserve and selectively edit first-input source WAVE, MP4/MOV/M4A, Matroska/WebM/MKA, and FLAC/Ogg/MP3 metadata across `mix-audio` and `merge-audio` to `.wav`, `.mka`, and `.mkv`

FVID-0236 updates `native_audio_mix::{mix_audio_with_controls, merge_audio_with_controls}`
(`src/native_audio_mix.rs`) to extract and selectively edit first-input
`source_info` chunks on `.wav` outputs via `extract_source_wave_info` and
first-input `source_matroska_metadata` on `.mka`/`.mkv` outputs via
`extract_source_pcm_metadata` and `Output::new_with_source_metadata`, and
updates `owned_wave_metadata::info_for_file_tags`
(`crates/fvid-media/src/owned_wave_metadata.rs`) to omit `ILNG` when
`track_language` is `"und"`. One focused integration test, 14 related
integration tests, and 1 offline fixture unit test pass. See
`batch-229-validation.json`.

## Batch 230 — Support `plan convert-subtitles` / `plan_convert_subtitles` across SRT, ASS, and Matroska subtitle sources and preserve source Matroska `WritingApp` (`encoder`) metadata during `convert-subtitles` in API and CLI

FVID-0237 adds `try_plan_with_controls` and dry-run cue/metadata validation in
`crates/fvid-media/src/owned_subtitle_impl.rs`; exposes
`plan_convert_subtitles` and `plan_convert_subtitles_with_controls` in
`crates/fvid-media/src/lib.rs` and `src/media.rs`; routes
`fvid media plan convert-subtitles INPUT [OUTPUT.mkv]` (with `--progress` no-op
and duplicate `--streams`/`--codec` checks) in `src/media_cli.rs`; and preserves
non-default source `reader.writing_app` (`ENCODER`) in
`try_convert_matroska_inner` so source `encoder` metadata is retained by default
and can be overridden or deleted via `--metadata` / `--metadata-delete`. One
focused integration test, 8 related integration tests, and 1 offline fixture
unit test pass. See `batch-230-validation.json`.

## Batch 231 — Support destination-optional `plan normalize-loudness` in CLI and ignore progress hooks in read-only `plan_loudness`, `plan_normalize_loudness`, and `plan decode-audio`

FVID-0238 allows `options.progress` as a read-only no-op in
`fvid::media::plan_loudness` and `fvid::media::plan_normalize_loudness`
(`src/media.rs`); accepts `--progress` as a no-op in `fvid media plan loudness`
and `fvid media plan decode-audio` (`src/media_cli.rs`); and updates
`fvid media plan normalize-loudness INPUT [OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv]`
in both `run` and `run_native` (`src/media_cli.rs`) so the destination path is
optional during planning while still enforcing destination-specific
`.wav`/`.mka`/`.mkv` extension and metadata constraints when an explicit output
path is supplied. One focused integration test, 10 related integration tests,
and 1 offline fixture unit test pass. See `batch-231-validation.json`.

## Batch 232 — Support compressed and rotated/cropped single-track Matroska video in owned subtitle burn, preserve source Matroska `WritingApp` (`encoder`) across FFV1 video exports, and enforce container metadata bounds in `owned_xfade` and `owned_lossless`

FVID-0239 updates `owned_subtitle_burn::prepare`
(`crates/fvid-media/src/owned_subtitle_burn.rs`) to admit all single-track
Matroska video sources supported by `owned_ffv1_export::supports` and compute
`width`/`height` from `track.visible()` with 90/270-degree rotation swap;
preserves non-default source Matroska `WritingApp` as `ENCODER` in
`owned_ffv1_export::export_processed`
(`crates/fvid-media/src/owned_ffv1_export.rs`) and preserves custom
container/track SimpleTags and `WritingApp` (`ENCODER`) in
`native_lossless_y4m::write_selected` (`src/native_lossless_y4m.rs`); and aligns
`owned_xfade::default_policy` (`crates/fvid-media/src/owned_xfade.rs`) and
`owned_lossless::metadata` (`crates/fvid-media/src/owned_lossless.rs`) to
enforce container metadata key/value bounds across planning and execution. One
focused integration test, 6 related integration tests, and 1 offline fixture
unit test pass. See `batch-232-validation.json`.

## Batch 233 — Support multitrack Matroska `V_MPEG4/ISO/AVC` and IETF track languages in owned lossless video export and enforce set-then-delete metadata precedence across `owned_webm_multitrack_export`, `owned_mp4_multitrack_export`, and `owned_ffv1_export`

FVID-0240 admits `"V_MPEG4/ISO/AVC"` and IETF track languages in
`owned_webm_multitrack_export::qualify`
(`crates/fvid-media/src/owned_webm_multitrack_export.rs`); populates
`vt.language` from `video.track_languages` / `video.track_legacy_languages` in
`owned_webm_multitrack_export::try_export` and
`owned_mp4_multitrack_export::try_export`
(`crates/fvid-media/src/owned_mp4_multitrack_export.rs`); preserves source
Matroska track languages (`track_languages` / `track_legacy_languages`,
defaulting to `"eng"`) in `native_lossless_y4m::write_selected`
(`src/native_lossless_y4m.rs`); and applies `metadata_set` /
`stream_metadata_set` before `metadata_delete` / `stream_metadata_delete` across
`owned_ffv1_export`, `owned_webm_multitrack_export`, and
`owned_mp4_multitrack_export`. One focused integration test, 9 related
integration tests, and 1 offline fixture unit test pass. See
`batch-233-validation.json`.

## Batch 234 — Support `9..=16`-bit, mixed-precision, and non-4:2:0 scheduled Y4M overlays, post-geometry overlay validation, Matroska `DefaultDuration` overlay clocks, modern CLI overlay filters, and track `FlagEnabled`/`FlagDefault` preservation across lossless video exports

FVID-0241 updates `owned_y4m_overlay::OverlayReader`
(`crates/fvid-media/src/owned_y4m_overlay.rs`) to support `8..=16`-bit,
mixed-precision (`owned_overlay::overlay_opaque_depth`), and non-`Yuv420`
(`Yuv422`, `Yuv440`, `Yuv444`, `Yuv411`, `Yuv410`) scheduled Y4M overlays with
subsampling-aware placement alignment; extracts `presented_header_after_geometry`
in `owned_y4m_decode.rs` (`crates/fvid-media/src/owned_y4m_decode.rs`) and uses
it in `overlay_supported`, `decode_reader_frames`, and
`owned_video_decode::process_frame_with_overlay`
(`crates/fvid-media/src/owned_video_decode.rs`) while stripping duplicate `F`
tokens before setting the nanosecond Matroska overlay clock; admits modern
geometry, pixel, and temporal filter flags and fixes `&args[command + 2]` in
`src/media_cli.rs`; and preserves source track `FlagEnabled` and `FlagDefault`
dispositions in `src/native_lossless.rs`, `src/native_lossless_y4m.rs`, and
`crates/fvid-media/src/owned_webm_multitrack_export.rs`. One focused integration
test, 9 related integration tests, and 1 offline fixture unit test pass. See
`batch-234-validation.json`.

## Batch 235 — Support rotated MP4 video tracks in owned video decode, bridge, and multitrack lossless export, account for trimmed AAC audio sample frames in MP4 multitrack interval export, and align `fvid::media::probe_as` format hints

FVID-0242 updates `owned_mp4_video_decode::decode_presented`
(`crates/fvid-media/src/owned_mp4_video_decode.rs`),
`owned_mp4_video_bridge::prepare_selected`
(`crates/fvid-media/src/owned_mp4_video_bridge.rs`), and
`owned_mp4_multitrack_export::qualify`
(`crates/fvid-media/src/owned_mp4_multitrack_export.rs`) to admit
`track.rotation` in `0 | 90 | 180 | 270`, materialize quarter turns on decoded
frames via `owned_display_geometry::apply` while preserving post-rotation
`track.pixel_aspect` and `effective_track_default`; tracks `trimmed_samples` in
`aac_packet_plan_window` (`crates/fvid-media/src/owned_mp4_aac_plan_impl.rs`)
and `plan_window` (`crates/fvid-media/src/owned_mp4_matroska_impl.rs`) and
reports `stats.trimmed_audio_sample_frames` in
`owned_mp4_multitrack_export::try_export`; and aligns
`src/media_probe.rs::try_probe_as` with `fvid_media::probe_as` format hints.
One focused integration test, 3 related integration tests, and 1 offline fixture
unit test pass. See `batch-235-validation.json`.

## Batch 236 — Support single-track MP4, monochrome FFV1, and RGB FFV1 sources in owned subtitle burn-in and enforce set-then-delete container and stream metadata precedence across subtitle burn, cross-fade, Y4M lossless export, and subtitle conversion

FVID-0243 updates `owned_subtitle_burn::{supports, prepare, plan_burn_subtitles, burn_subtitles}`
(`crates/fvid-media/src/owned_subtitle_burn.rs`) to bridge single-track MP4 video
via `owned_mp4_video_bridge::prepare`; supports `8..=16`-bit interleaved RGB
frames in `owned_text_raster::composite_white`
(`crates/fvid-media/src/owned_text_raster.rs`) and preserves true monochrome
(`encode_gray` / `encode_gray_with_alpha`) when `view.monochrome && process.is_some()`
in `owned_ffv1_export::export_processed`
(`crates/fvid-media/src/owned_ffv1_export.rs`); and enforces set-then-delete
container and stream metadata precedence (plus uppercase custom tag
normalization) across `owned_lossless::metadata`
(`crates/fvid-media/src/owned_lossless.rs`), `write_y4m_ffv1_policy`
(`crates/fvid-media/src/owned_matroska.rs`), `owned_subtitle_burn::burn_subtitles`,
`owned_xfade::write_y4m_ffv1_counted` (`crates/fvid-media/src/owned_xfade.rs`),
and `owned_subtitle_impl::apply_subtitle_matroska_metadata`
(`crates/fvid-media/src/owned_subtitle_impl.rs`). One focused integration test,
3 related integration tests, and 1 offline fixture unit test pass. See
`batch-236-validation.json`.

## Batch 237 — Enforce set-then-delete container and stream metadata precedence, case-insensitive tag removal, and uppercase Matroska custom tag normalization across MP4/ADTS/PCM Matroska remux, concat, and WAVE INFO editing

FVID-0244 updates `owned_mp4_remux::{track_metadata, metadata}`
(`crates/fvid-media/src/owned_mp4_remux.rs`), `mp4_concat::write_with_options`
(`src/container/mp4_concat.rs`), `extract_adts_matroska_metadata`
(`crates/fvid-media/src/owned_matroska_adts_impl.rs`),
`parse_matroska_pcm_metadata` (`src/native_export/pcm_matroska.rs`), and
`owned_wave_metadata::edit_info_chunks`
(`crates/fvid-media/src/owned_wave_metadata.rs`) to apply `metadata_set` and
`stream_metadata_set` before `metadata_delete` and `stream_metadata_delete`,
normalizes custom Matroska container and stream tags to uppercase with
case-insensitive removal, and enforces stream metadata key/value bounds and
forbidden key rejection (`rotate`, `stereo_mode`, `alpha_mode`) in
`src/container/mp4_concat.rs` and `src/native_plan.rs`. One focused integration
test, 6 related integration tests, and 1 offline fixture unit test pass. See
`batch-237-validation.json`.

## Batch 238 — Support 6.1/7.1 downmixing, `1|2 -> 3..=8` upmixing, and ALAC speaker masks across native audio export and planning, qualify Matroska Opus/MPEG channel layouts in `supports()`, and preserve bare MPEG ID3v2 language/custom tags in Matroska PCM export

FVID-0245 updates `WaveInfo::validate_rematrix` (`src/native_pcm.rs`),
`export_audio_pcm_transformed_with_source_metadata` (`src/native_export.rs`),
`native_plan::decode_audio_selected` (`src/native_plan.rs`), and
`PacketPcmDecoder::channel_mask` (`src/native_audio_decoder.rs`) to support
`1..=8 -> 1 | 2` downmixing (including 6.1 and 7.1), `1 | 2 -> 1..=8` upmixing,
explicit speaker masks via `PcmGain::with_mask` and `rematrix_layout_supported`,
standard 6.1/7.1 WAVE output masks, and ALAC speaker masks; qualifies target
channel conversion before decode and admits only `Ok(_)` or controlled-memory
budget errors in `owned_matroska_opus_export::supports`
(`crates/fvid-media/src/owned_matroska_opus_export.rs`) and
`owned_matroska_mp3_export::supports`
(`crates/fvid-media/src/owned_matroska_mp3_export.rs`); uses
`pcm_decode_options` in `owned_raw_mp3::source_info`
(`crates/fvid-media/src/owned_raw_mp3.rs`); and preserves container `language`
(such as ID3v2 `TLAN`) and uppercase custom tags in
`extract_source_pcm_metadata` (`src/native_export/pcm_matroska.rs`). Two focused
integration tests, 7 related integration tests, and 1 offline fixture unit test
pass. See `batch-238-validation.json`.

## Batch 239 — Align ADTS `remux`, `plan_remux`, and `concat` stream-zero selection, packet/RSS budgets, and Matroska metadata routing across native MP4 and Matroska paths

FVID-0246 adds `native_plan::remux_adts_with_options` (`src/native_plan.rs`) to
validate and plan ADTS remux with packet/RSS budgets, `streams: [0]`, and
Matroska metadata mutations; updates `fvid::media::remux`,
`fvid::media::plan_remux`, and `fvid::media::concat` (`src/media.rs`) to
validate ADTS options via `validate_adts_copy_options_with_metadata(options, true, matroska)`
and enforce `"ADTS has only stream 0"` before invoking
`remux_adts_aac_with_options`, `remux_adts_with_options`, and
`concat_adts_aac_with_options` on the native `"fvid"` backend; and aligns
`try_native_audio_concat` (`src/media_cli.rs`) stream validation with
`"ADTS has only stream 0"` and `"WAVE has only stream 0"`. Two focused
integration tests, 3 related integration tests, and 1 offline fixture unit test
pass. See `batch-239-validation.json`.

## Batch 240 — Align Matroska identity remux planning and MP4 remux/relocation packet/RSS budgets and backend attribution across top-level and owned paths

FVID-0247 adds `probe_webm_with_packet_limit`
(`crates/fvid-media/src/owned_webm_probe.rs`), `matroska_with_packet_limit`
(`src/media_probe.rs`), `native_plan::remux_with_options` (`src/native_plan.rs`),
and `validate_mp4_relocate_options` (`src/media.rs`); propagates
`options.max_packet_bytes`, `options.max_rss_bytes`, and `options.cancel`
through Matroska identity remux planning and MP4 fast-start relocation; and
normalizes `backend: "fvid"` and `fvid_payload_copies: 0` when
`fvid::media::remux` delegates MP4 -> Matroska remux with custom options to
`fvid_media::remux`. Two focused integration tests, 6 related integration tests,
and 1 offline fixture unit test pass. See `batch-240-validation.json`.

## Batch 241 — Enforce packet-byte, zero-packet, and RSS budgets across audio planning, loudness, loudnorm, mix/merge, and sample-domain concat paths and align CLI dispatch

FVID-0248 propagates `options.max_packet_bytes` and `scan_all()` through
`owned_matroska_{aac,alac,flac,mp3,opus,pcm}_export.rs` and
`owned_audio_plan::matroska_descriptor`; validates MP4 `SampleIndex` packet sizes
against `options.max_packet_bytes` in `owned_mp4_audio_export::descriptor`,
`native_loudnorm::try_plan`, `native_plan::loudness_with_options`,
`native_plan::concat_mp4_matroska_with_options`, and `native_audio_mix`; enforces
WAVE frame-size packet limits and `max_packets == Some(0)` across
`owned_audio_plan`, `owned_loudnorm`, `native_loudnorm`, `native_plan`, and
`native_audio_mix`; preserves budget refusal errors in `src/media.rs` mix/merge
wrappers instead of falling back to control-unaware planners; and routes
`plan overlay`, `overlay`, `plan loudnorm`, and `loudnorm` in `src/media_cli.rs`
through `fvid::media::*`. Three focused integration tests, 7 related integration
tests, and 1 offline fixture unit test pass. See `batch-241-validation.json`.

## Batch 242 — Enforce packet-byte and zero-packet budgets in `decode-audio` and `trim` planning/execution and align WAVE `trim`/`trim-pcm`/`concat` stream-0 errors, backend attribution, and budget notes

FVID-0249 adds `decode_audio_with_options`, `trim_audio_with_options`,
`trim_aac_with_options`, and `trim_adts_with_options` in `src/native_plan.rs` to
enforce `max_packet_bytes`, `max_packets == Some(0)`, and `max_rss_bytes` across
WAVE, ADTS, MP4, and Matroska inputs; rejects `max_packets == Some(0)` in
`validate_owned_audio_options_for_destination` (`src/media.rs`),
`validate_request` (`crates/fvid-media/src/owned_audio_export.rs`), and
zero-sample `decode_reader` (`src/native_pcm.rs`); passes `Limits.packet_bytes`
into `WebmReader::open` and propagates `scan_all()?` in `src/native_export.rs`
and `src/native_export/pcm_matroska.rs`; and aligns WAVE `trim`, `trim_pcm`, and
`concat` stream-0 errors (`"WAVE has only stream 0"`), `fvid::media::concat`
backend attribution (`"fvid"`), and budget notes in `src/media.rs`,
`src/media_cli.rs`, `crates/fvid-media/src/owned_wave_plan.rs`, and
`crates/fvid-media/src/owned_audio_plan.rs`. Two focused integration tests, 7
related integration tests, and 1 offline fixture unit test pass. See
`batch-242-validation.json`.

## Batch 243 — Enforce packet-byte, zero-packet, and RSS budgets in `loudness` and `normalize-loudness` channel-weight inspection and execution and align CLI metadata refusal and plan budget notes

FVID-0250 adds `default_weights_with_options`
(`loudness_channel_weights_with_options`) in `src/native_pcm/loudness.rs` and
`src/native_pcm.rs` and wires it through `src/media.rs`, `src/native_plan.rs`,
and `src/media_cli.rs`; rejects `options.max_packets == Some(0)` at preflight and
passes `limits.packet_bytes` to `WebmReader::open` in
`measure_file_with_options` (`src/native_pcm/loudness.rs`); deduplicates budget
notes in `native_plan::loudness_with_options` (`src/native_plan.rs`); and parses
metadata flags in CLI `loudness` and `plan loudness` (`src/media_cli.rs`) so
they report the owned loudness contract error before channel-weight inspection.
Three focused integration tests, 8 related integration tests, and 1 offline
fixture unit test pass. See `batch-243-validation.json`.

## Batch 244 — Enforce input packet-byte budgets in `convert-subtitles`, `burn-subtitles`, `xfade`, and FFV1 export and preserve CLI policy refusal messages across subtitle and cross-fade paths

FVID-0251 propagates `max_packet_bytes` into `WebmReader::open` and
`restrict_packet_bytes` in `crates/fvid-media/src/owned_subtitle_impl.rs`,
`crates/fvid-media/src/owned_subtitle_burn.rs`, and
`crates/fvid-media/src/owned_ffv1_export.rs`; enforces
`header.frame_len() > options.max_packet_bytes` in
`owned_subtitle_burn::burn_subtitles` and `owned_xfade::write_y4m_ffv1_counted`
to match `plan_burn_subtitles` and `plan_xfade`; parses `--max-memory-mib` in
CLI `convert-subtitles`, `burn-subtitles`, and `xfade` (`src/media_cli.rs`); and
retains owned `burn-subtitles` and `xfade` CLI dispatch when source inputs are
supported with default options so policy refusals (`--streams`,
`--max-memory-mib`, metadata limits) are reported directly in all build
configurations. Three focused integration tests, 3 related unit tests, and 1
offline fixture unit test pass. See `batch-244-validation.json`.

## Batch 245 — Enforce input and companion packet-byte budgets across Y4M, Matroska, and MP4 lossless export, `crop-lossless`, and `overlay` paths and preserve CLI policy refusal messages

FVID-0252 validates raw source and overlay foreground Y4M
`header.frame_len() > options.max_packet_bytes` in both `transcode_lossless` and
`plan_transcode_lossless` (`crates/fvid-media/src/owned_lossless.rs`); passes
`limits.packet_bytes = limits.packet_bytes.min(options.max_packet_bytes)` into
`WebmReader::open` in `crates/fvid-media/src/owned_webm_multitrack_export.rs` and
`crates/fvid-media/src/owned_mp4_multitrack_export.rs` and validates selected MP4
`SampleIndex` packet sizes against `options.max_packet_bytes` in
`owned_mp4_multitrack_export::qualify`; and parses `--max-memory-mib` in
`try_owned_overlay` and `owned_lossless_command_with_name` (`src/media_cli.rs`)
while retaining owned CLI dispatch when
`supports(source, &transform, &CopyOptions::default())` is true. Three focused
integration tests, 7 related unit tests, and 1 offline fixture unit test pass.
See `batch-245-validation.json`.

## Batch 246 — Enforce encoded packet-byte budgets and positive limit validation across Y4M export, concat, and trim planning and execution

FVID-0253 adds `validate_video_packet_budget` in `src/native_export.rs` to
inspect Y4M frame lengths, MP4 video `SampleIndex` sizes, and Matroska packet
sizes against `options.max_packet_bytes` before decoding or planning, and calls
`validate_video_selection` in `export_y4m_pipeline_with_options`; invokes
`validate_video_packet_budget` and rejects `options.max_packets == Some(0)` in
`concat_y4m_with_options` and `trim_y4m_with_options` (`src/native_plan.rs`);
and rejects `--max-packet-bytes 0` (`"packet byte limit must be positive"`) and
routes `--max-rss-mib` through `fvid_media::parse_max_rss_mib` in
`parse_native_audio_limit` and `run_native` (`src/media_cli.rs`). Three focused
integration tests, 4 related integration tests, and 1 offline fixture unit test
pass. See `batch-246-validation.json`.

## Batch 247 — Enforce packet-byte and zero-packet budgets and single-stream validation parity across MP4-to-Matroska remux and concat, native remux planning, and WAVE/ADTS operations

FVID-0254 enforces `(sample.size as usize) > options.max_packet_bytes` in
`owned_mp4_remux::plan_remux` (`crates/fvid-media/src/owned_mp4_remux.rs`) and
`mp4_concat::open_with_packet_limit` (`src/container/mp4_concat.rs`), and
rejects `options.max_packets == Some(0)` in `owned_mp4_remux::plan_remux`,
`owned_mp4_remux::remux`, and `mp4_concat::write_with_options`; validates
`options.max_packet_bytes > 0` and `check_rss_budget(options)` in
`native_plan::remux_with_options` (`src/native_plan.rs`); and checks
`options.streams.iter().any(|&s| s != 0)` before secondary policy validation in
WAVE and ADTS operations in `src/media.rs`. Three focused integration tests, 3
related integration tests, and 2 offline fixture unit tests pass. See
`batch-247-validation.json`.

## Batch 248 — Enforce packet-byte and controlled-memory budgets and stream-selection parity across native loudnorm, loudness, normalization, and audio mix/merge paths

FVID-0255 validates the first ADTS packet size against `options.max_packet_bytes`
and enforces WAVE stream selection and sample-frame `max_packet_bytes` in
`native_loudnorm::try_plan` and `try_apply`, and routes native container loudnorm
through `native_plan::decode_audio_with_options` (`src/native_loudnorm.rs`);
probes single-stream audio eligibility with default budget limits in
`native_audio_mix::validate_and_inspect_source_with_options`
(`src/native_audio_mix.rs`) so `plan_decode_audio` surfaces exact
`max_controlled_bytes` and `max_packet_bytes` errors; calls
`decode_audio_with_options` in `native_plan::loudness_with_options`
(`src/native_plan.rs`); and enforces single-stream and metadata guards in
`native_pcm::loudness::default_weights_with_options`
(`src/native_pcm/loudness.rs`). Three focused integration tests, 6 related
integration tests, and 2 offline fixture unit tests pass. See
`batch-248-validation.json`.

## Batch 249 — Enforce packet-byte budgets and stream-selection parity across Ogg Opus/Vorbis/joint, G.711 WAVE, and standalone container loudness and loudnorm paths

FVID-0256 adds `max_packet_size` on `owned_ogg_opus::Program`,
`owned_ogg_vorbis::Program`, and `owned_ogg_audio_program::Program` and rejects
oversized packets in `owned_ogg_opus_export::open`,
`owned_ogg_vorbis_export::open`, and `owned_ogg_joint_export::open`; enforces
`options.max_packet_bytes < usize::from(shape.channels)` in
`owned_g711_wave_export::open` (`crates/fvid-media/src/owned_g711_wave_export.rs`);
and validates stream selection in `native_loudnorm::try_plan` and `try_apply`
(`src/native_loudnorm.rs`) while routing recognized standalone audio directly to
`owned_container_loudnorm` so stream-selection and budget refusals propagate
consistently for both `.wav` and `.mka`/`.mkv` outputs. Three focused integration
tests, 6 related integration tests, and 2 offline fixture unit tests pass. See
`batch-249-validation.json`.

## Batch 250 — Enforce packet-byte budget and error parity across sample-domain audio concat and standalone audio trim planning and execution

FVID-0257 probes sample-domain concat eligibility with default budget/stream
controls in `try_concat_mp4_matroska_with_options` (`src/native_export.rs`) and
`concat_mp4_matroska_with_options` (`src/native_plan.rs`), then explicitly
validates output controls, WAVE sample-frame `max_packet_bytes`,
MP4/Matroska/ADTS packet budgets, and standalone audio `plan_decode_audio`
before execution or plan admission; and routes recognized standalone audio
directly to `decode_audio_transformed` / `plan_decode_audio` in
`owned_wave_remux::trim` (`crates/fvid-media/src/owned_wave_remux.rs`),
`owned_wave_plan::plan_trim` (`crates/fvid-media/src/owned_wave_plan.rs`), and
`media::trim` (`src/media.rs`) so packet-budget and stream-selection refusals
propagate directly without falling back to WAVE trim. Three focused integration
tests, 9 related integration tests, and 2 offline fixture unit tests pass. See
`batch-250-validation.json`.

## Batch 251 — Enforce packet-byte budget and container-specific policy diagnostics across Matroska/MP4 remux and non-WAVE concat planning and execution

FVID-0258 validates `options.max_packet_bytes > 0` up front and returns
container-specific refusals (`"request is not an owned Matroska identity remux"`,
`"request is not an owned MP4 Matroska remux"`,
`"request is not an owned ADTS Matroska remux"`) for non-WAVE inputs in
`owned_remux::remux` (`crates/fvid-media/src/owned_remux.rs`) and
`owned_remux_plan::plan_remux` (`crates/fvid-media/src/owned_remux_plan.rs`);
validates MP4 track sample sizes against `options.max_packet_bytes` in
`native_plan::remux_with_options` (`src/native_plan.rs`) and
`validate_mp4_packet_budget` (`src/media.rs`), allows positive custom
`max_packet_bytes` in `validate_mp4_relocate_options`, and aligns
`media::plan_remux` with `media::remux` on `is_native_mp4_matroska`; and refuses
non-WAVE inputs that fail ADTS `supports` with
`"request is not an owned ADTS Matroska concat"` in `owned_concat::concat` and
`owned_concat::plan_concat` (`crates/fvid-media/src/owned_concat.rs`). Three
focused integration tests, 3 related integration tests, and 2 offline fixture
unit tests pass. See `batch-251-validation.json`.

## Batch 252 — Enforce encoded FFV1 packet-byte budgets, positive limit checks, and policy refusal parity across lossless transcode planning and Y4M trim/concat

FVID-0259 validates `options.max_packet_bytes > 0` up front in
`transcode_lossless` and `plan_transcode_lossless`
(`crates/fvid-media/src/owned_lossless.rs`), admits sources in
`plan_transcode_lossless` when `supports(source, transform, &CopyOptions::default())`
holds so policy refusals match `transcode_lossless`, and verifies encoded FFV1
packet sizes against custom `options.max_packet_bytes` in
`plan_transcode_lossless` across Y4M and single-track Matroska/MP4 paths while
removing the bogus transformed-Y4M frame-byte check inside the Y4M visitor; and
validates `validate_video_packet_budget` before checking
`control.packet_limit_reached()` in `export_y4m_sources_with_options`
(`src/native_export.rs`) and after all `validate_video_selection` checks in
`concat_y4m_with_options` (`src/native_plan.rs`). Three focused integration
tests, 3 related integration tests, and 2 offline fixture unit tests pass. See
`batch-252-validation.json`.

## Batch 253 — Enforce positive packet-byte checks, source header error parity, encoded FFV1 packet-byte budgets, and per-cue formatted ASS event byte limits across xfade, subtitle burn-in, and subtitle conversion

FVID-0260 validates `options.max_packet_bytes > 0`, `check_rss_budget(options)`,
and `metadata(options)` before generic policy refusal in `plan_xfade` and
`xfade_video` (`crates/fvid-media/src/owned_xfade.rs`) and validates
primary/secondary Y4M headers and frame-rate parity in `xfade_video` so header
errors match `plan_xfade`; composites active subtitle cues and encodes FFV1
packets during `plan_burn_subtitles`
(`crates/fvid-media/src/owned_subtitle_burn.rs`) for both Y4M and Matroska/MP4
sources when `options.max_packet_bytes` is customized; and enforces formatted ASS
event payload byte limits per cue inside the cue loops of `try_convert_srt_inner`
and `try_convert_matroska_inner`
(`crates/fvid-media/src/owned_subtitle_impl.rs`). Three focused integration
tests, 5 related integration tests, and 1 offline fixture unit test pass. See
`batch-253-validation.json`.

## Batch 254 — Enforce encoded FFV1 packet-byte budgets, selected-track packet-byte admission, and metadata/policy refusal parity across multitrack Matroska and MP4 lossless planning and execution

FVID-0261 shares `check_encoded_ffv1` in `crates/fvid-media/src/owned_lossless.rs`
and invokes it during `owned_webm_multitrack_export::plan_selection` and
`owned_mp4_multitrack_export::plan_selection` when `options.max_packet_bytes` is
customized, while validating `metadata(options)` unconditionally in
`plan_transcode_lossless`, `owned_webm_multitrack_export::qualify`, and
`owned_mp4_multitrack_export::qualify`; scans Matroska containers with default
index limits before applying `restrict_packet_bytes(options.max_packet_bytes)`
and checking `packet_payload_size(index) > options.max_packet_bytes` on the
selected tracks in `owned_webm_multitrack_export::qualify`,
`owned_video_decode::decode_webm`, and `owned_ffv1_export::input_limited`; and
validates `check(options)`, track selection, and
`stream_metadata_set`/`stream_metadata_delete` track bounds before
`owned_lossless::policy(&normalized, true)` in both multitrack exporters. Three
focused integration tests, 6 related integration tests, and 1 offline fixture
unit test pass. See `batch-254-validation.json`.

## Batch 255 — Preserve upright display pixel aspect on rotated overlay inputs, validate post-geometry overlay compatibility in planning and execution, and honor pre-cancelled native copy options

FVID-0262 validates `transformed_aspect(reader.pixel_aspect(), visible_w, visible_h, geometry)`
directly against `self.source.reader.pixel_aspect()` without double-swapping
90/270-degree rotated tracks, and checks post-geometry main chroma subsampling
and chroma-aligned `(x, y)` placement in
`TimedOverlay::validate_main_with_geometry` (`src/native_export/overlay.rs`)
across `overlay_video_transformed`, `decode_video_pipeline_overlay_step`
(`src/native_media.rs`), `native_plan::overlay_with_geometry`,
`native_plan::decode_overlay_with_geometry`, and
`native_plan::transcode_lossless` (`src/native_plan.rs`); checks
`options.cancel` in `validate_native_copy_options` (`src/media.rs`); and routes
`!custom_policy && supports_overlay(&transform) && overlay_eligible(source)?`
directly in `owned_lossless_command_with_name` (`src/media_cli.rs`). Four
focused integration tests, 10 related integration tests, and 1 offline fixture
unit test pass. See `batch-255-validation.json`.

## Batch 256 — Scope Y4M export/concat/trim packet-byte budgets to selected video tracks and output Y4M frames, validate Y4M planner cancellation and segment compatibility, enforce xfade pixel aspect parity, and prioritize subtitle-burn cancellation and single-track admission

FVID-0263 scopes MP4 and Matroska packet-byte checks in
`validate_video_packet_budget_with_geometry` (`src/native_export.rs`) to the
selected video track and verifies decoded output Y4M frame byte size via
`inspect_first_y4m_frame`, checks `options.cancel` in `validate_y4m_options`,
and validates segment rate/layout compatibility in
`native_plan::concat_y4m_with_options` (`src/native_plan.rs`) while supporting
`--streams` and `--max-memory-mib` in `export-y4m` CLI (`src/media_cli.rs`);
enforces reduced pixel aspect ratio equality in `SecondaryReader::new`
(`crates/fvid-media/src/owned_xfade.rs`); and checks `options.cancel` in
`owned_subtitle_burn::policy` (`crates/fvid-media/src/owned_subtitle_burn.rs`)
while qualifying single-track Matroska video before applying
`options.max_packet_bytes`. Four focused integration tests, 7 related
integration tests, and 1 offline fixture unit test pass. See
`batch-256-validation.json`.

## Batch 257 — Scope Matroska and MP4 audio export, planning, loudness, loudnorm, metadata extraction, and mix/merge packet-byte budgets to the selected audio track

FVID-0264 opens Matroska containers with default index packet-byte limits and
applies `restrict_packet_bytes(options.max_packet_bytes)` plus selected-track
`packet_payload_size(packet_index) > options.max_packet_bytes` validation in
`validate_encoded_audio_packet_budget` (`src/native_plan.rs`),
`export_pcm_selected` (`src/native_export.rs`), `extract_source_pcm_metadata`
(`src/native_export/pcm_matroska.rs`), `native_loudnorm::try_plan`
(`src/native_loudnorm.rs`), `validate_and_inspect_source_with_options` and
`eligible_with_options` (`src/native_audio_mix.rs`), `matroska_descriptor`
(`crates/fvid-media/src/owned_audio_plan.rs`), and `open_audio_reader` /
`admit_audio_reader` (`crates/fvid-media/src/owned_matroska_audio.rs`), and
scopes MP4 packet-byte eligibility in `native_audio_mix::eligible_with_options`
to `reader.tracks().get(info.stream_index)`. Three focused integration tests,
13 related integration tests, and 1 offline fixture unit test pass. See
`batch-257-validation.json`.

## Batch 258 — Preserve selected source stream index across multi-stream Ogg audio planning and loudness, qualify discrete mono/stereo Ogg Opus loudness and channel rematrixing, and scope Ogg packet-byte budgets to the active decode prefix

FVID-0265 returns `(selected.source_index, Backend)` from `select_backend` and
threads `Some(stream_index)` plus `transform` through `descriptor_for_decode`,
`supports`, and `apply` in `crates/fvid-media/src/owned_ogg_audio_export.rs`
and `crates/fvid-media/src/owned_audio_plan.rs`; admits
`channels <= 2 && mask == 0` in `owned_ogg_opus_export::supports` and maps
`mask == 0 && channels <= 2` to `standard_mask(channels)` in
`owned_container_loudness::geometry`
(`crates/fvid-media/src/owned_container_loudness.rs`); and adds
`max_packet_size_in_prefix(until, max_packets)` to `OggOpus`, `OggVorbis`, and
`OggAudioProgram`
(`crates/fvid-media/src/{owned_ogg_opus.rs,owned_ogg_vorbis.rs,owned_ogg_audio_program.rs}`)
so `open_with_transform` validates `options.max_packet_bytes` only across
packets within the active `max_packets` and `interval` decode prefix. Three
focused integration tests, 7 related test suites, and 2 offline fixture unit
tests pass. See `batch-258-validation.json`.

## Batch 259 — Scope bare FLAC and bare MPEG Layer I/II/III packet-byte budgets to the active decode prefix across planning, trimming, and WAVE export

FVID-0266 adds `max_packet_size_in_prefix(until, max_packets)` to `Flac`
(`crates/fvid-media/src/owned_flac.rs`) and `Mp3`
(`crates/fvid-media/src/owned_mpeg_audio.rs`), adds `open_with_transform`,
`open_with_interval`, and `descriptor_with_transform` to
`crates/fvid-media/src/owned_raw_flac.rs` and
`crates/fvid-media/src/owned_raw_mp3.rs`, and threads `transform.copied()` from
`owned_audio_plan::source_descriptor_for_decode`
(`crates/fvid-media/src/owned_audio_plan.rs`), `supports`, `apply`, and
`decode_raw_checked` so `options.max_packet_bytes` is enforced over the active
decode prefix (`max_packets` and `interval`) while still rejecting oversized
packets in full-stream plans and decodes. Two focused integration tests, 7
related test suites, and 2 offline fixture unit tests pass. See
`batch-259-validation.json`.

## Batch 260 — Qualify discrete multichannel and non-standard sample-rate standalone/container audio in sample-peak PCM loudness and normalization, report encoded packet progress across spooled loudness decodes, and scope Matroska PCM loudness packet-byte budgets to the selected audio stream

FVID-0267 adds `geometry_unqualified`, `decode_to_wave_unqualified`, and
`decode_adts_to_wave` in `crates/fvid-media/src/owned_container_loudness.rs`
and uses them in `measure_file_with_options` (`src/native_pcm/loudness.rs`)
after validating `LoudnessMeter::new(rate, weights)` while forwarding encoded
packet progress during spooling and emitting
`ProgressEvent { done: true, ..spool.progress }` after measurement completes;
and scopes Matroska `max_packet_bytes` checks in `measure_file_with_options`,
`default_weights_with_options`, and `loudness_with_options`
(`src/native_plan.rs`) to the selected audio stream (`info.stream_index`) and
active `max_packets` prefix. Two focused integration tests, 6 related test
suites, and 2 offline fixture unit tests pass. See
`batch-260-validation.json`.

## Batch 261 — Scope mix-audio and merge-audio packet-byte admission to the active max_packets prefix and preserve 4-CC RIFF INFO tags when exporting or normalizing bare FLAC, Ogg Opus/Vorbis, and bare MP3 to WAVE

FVID-0268 preserves `options.max_packets` in
`is_single_stream_fvid_media_audio_with_options`,
`validate_and_inspect_source_with_options`, and `eligible_with_options`
(`src/native_audio_mix.rs`) while bounding MP4, Matroska, and ADTS packet-byte
inspection loops to `options.max_packets` (and keeping `concat` per-source
inspection unbounded on `max_packets` in `src/native_audio_mix/concat.rs`
because `concat` shares `max_packets` cumulatively across segments); and retains
printable 4-CC RIFF `INFO` keys (`key.len() == 4 && tag_for_key(key).is_ok()`)
alongside the 10 semantic keys in `owned_ogg_opus_export::info_for_tags`
(`crates/fvid-media/src/owned_ogg_opus_export.rs`),
`owned_flac_layout::wave_info_from_extra`
(`crates/fvid-media/src/owned_flac_layout.rs`), and `Mp3::wave_info`
(`crates/fvid-media/src/owned_mpeg_audio.rs`). Two focused integration tests, 6
related test suites, and 2 offline fixture unit tests pass. See
`batch-261-validation.json`.

## Batch 262 — Scope Matroska and MP4 audio packet-byte and controlled-memory admission to the active max_packets prefix and preserve QuickTime/iTunes ©too encoder tags across probe, WAVE INFO, and Matroska PCM exports

FVID-0269 bounds `validate_audio_packet_budget` and `admit_audio_reader`
(`crates/fvid-media/src/owned_matroska_audio.rs`),
`validate_encoded_audio_packet_budget` (`src/native_plan.rs`),
`native_loudnorm::try_plan` (`src/native_loudnorm.rs`), and
`extract_source_pcm_metadata_with_packet_limit`
(`src/native_export/pcm_matroska.rs`) by `options.max_packets`; parses
QuickTime/iTunes `©too` (`0xa9746f6f`) into `Mp4Reader::encoder()`
(`crates/fvid-media/src/owned_mp4_impl.rs`) and exposes it in `try_mp4`
(`crates/fvid-media/src/owned_mp4_probe_impl.rs`), `mp4_source_info`
(`crates/fvid-media/src/owned_wave_metadata.rs`), `export_pcm_selected`
(`src/native_export.rs`), and `extract_source_pcm_metadata_with_packet_limit`;
and clears output metadata edits before `loudness_channel_weights_with_options`
in `src/media.rs` and `src/media_cli.rs`. Two focused integration tests, 5
related test suites, and 2 offline fixture unit tests pass. See
`batch-262-validation.json`.

## Batch 263 — Scope unedited MP4 audio max_packet_bytes and max_controlled_bytes admission in fvid-media and loudness channel-weight inspection to the active max_packets prefix

FVID-0270 bounds `owned_mp4_audio_export::descriptor`
(`crates/fvid-media/src/owned_mp4_audio_export.rs`),
`owned_mp4_audio::admit_audio_reader`
(`crates/fvid-media/src/owned_mp4_audio.rs`), and
`default_weights_with_options` (`src/native_pcm/loudness.rs`) by
`options.max_packets` when `track.edits.is_empty()` and enforces
`largest_packet <= options.max_packet_bytes` inside `admit_audio_reader`. One
focused integration test, 2 related test suites, and 2 offline fixture unit
tests pass. See `batch-263-validation.json`.

## Batch 264 — Scope mono AAC in-band Parametric Stereo (PS) probing and MP4 sample-description inspection to the active max_packets prefix across MP4 and Matroska audio pipelines

FVID-0271 threads `max_packets` into `negotiate_mp4_ps`,
`negotiate_mp4_description_ps`, `mp4_normalized_geometry`,
`mp4_normalized_geometry_controlled`
(`crates/fvid-media/src/owned_mp4_audio_timeline_impl.rs`), and
`negotiate_matroska_ps`
(`crates/fvid-media/src/owned_matroska_audio_timeline_impl.rs`), exposes
`DecodeProgress::packet_limit` in
`crates/fvid-media/src/owned_matroska_audio.rs` and `src/native_media.rs`, and
bounds unedited MP4 sample-description loops in
`decode_mp4_audio_reader_targeted`, `decode_mp4_audio_normalized`,
`owned_mp4_audio_export::geometry`, and `owned_aac_export_routing::router` to
`active_samples`. Two focused integration tests, 3 related test cases, and 1
offline fixture unit test pass. See `batch-264-validation.json`.

## Batch 265 — Preserve QuickTime/iTunes ©too encoder metadata in MP4-to-Matroska remux and concat with set/delete override support and scope compressed MP4 concat packet-byte admission to selected streams and the active max_packets prefix

FVID-0272 seeds `text` with `("ENCODER", encoder.to_owned())` from
`Mp4Reader::encoder()` (`©too`) before applying `options.metadata_set` and
`options.metadata_delete` in `owned_mp4_remux::metadata`
(`crates/fvid-media/src/owned_mp4_remux.rs`) and `mp4_concat::write_with_options`
(`src/container/mp4_concat.rs`), as well as in
`owned_mp4_matroska_impl::write_selected`
(`crates/fvid-media/src/owned_mp4_matroska_impl.rs`) and
`matroska_write::write_mp4_aac_metadata` (`src/container/matroska_write.rs`),
and adds `mp4_concat::open_with_options` (`src/container/mp4_concat.rs`) wired
into `concat_mp4_matroska_with_options` (`src/native_plan.rs`) and
`try_concat_mp4_matroska_with_options` (`src/native_export.rs`) to validate
`max_packet_bytes` in DTS-interleaved order across `selected` tracks up to
`options.max_packets`. Two focused integration tests, 4 related test suites,
and 1 offline fixture unit test pass. See `batch-265-validation.json`.

## Batch 266 — Scope sample-domain audio concat packet-byte admission to the cumulative max_packets prefix across segments and defer MP4 reader packet-byte enforcement until after moov index open

FVID-0273 adds `Mp4Reader::restrict_packet_bytes`
(`crates/fvid-media/src/owned_mp4_impl.rs`) and opens `Mp4Reader` with
`Limits::default()` before restricting packet bytes or checking `0..max_samples`
across `owned_mp4_audio` (`crates/fvid-media/src/owned_mp4_audio.rs`),
`owned_mp4_audio_export` (`crates/fvid-media/src/owned_mp4_audio_export.rs`),
`owned_aac_export_routing` (`crates/fvid-media/src/owned_aac_export_routing.rs`),
`owned_wave_metadata` (`crates/fvid-media/src/owned_wave_metadata.rs`),
`src/media.rs`, `src/native_export.rs`, `src/native_export/pcm_matroska.rs`,
`src/native_loudnorm.rs`, `src/native_plan.rs`, and `src/native_audio_mix.rs`,
and adds `validate_concat_segments_with_options`
(`src/native_audio_mix/concat.rs`) wired into `concat_mp4_matroska_with_options`
(`src/native_plan.rs`), `try_concat_mp4_matroska_with_options`
(`src/native_export.rs`), `concat::eligible_with_options`, and
`concat::concat_audio_with_options` so sample-domain concat validates
`max_packet_bytes` and `max_controlled_bytes` across the cumulative
`options.max_packets` prefix and breaks cleanly when `remaining == 0`. Two
focused integration tests, 4 related test suites, and 1 offline fixture unit
test pass. See `batch-266-validation.json`.

## Batch 267 — Forward caller budgets through dynamic-description MP4 audio planning and scope MP4 remux, concat, loudness, and normalization admission to the active max_packets prefix and selected streams

FVID-0274 forwards a cloned `CopyOptions` (with `streams` normalized to
`selected` and container metadata stripped) from
`native_plan::decode_audio_with_options` (`src/native_plan.rs`) to
`fvid_media::plan_decode_audio`, requires `ensure_constant_description()` in
`native_media::audio_source_info_selected` (`src/native_media.rs`) so
dynamic-`stsd` MP4 audio routes through `fvid-media` normalization across
`decode_audio`, `loudness`, and `normalize_loudness`, bounds unedited
sample-description checks in `admit_audio_reader_targeted`
(`crates/fvid-media/src/owned_mp4_audio.rs`) and `decode_mp4_audio_normalized`
(`crates/fvid-media/src/owned_mp4_audio_normalized_impl.rs`) to
`0..active_samples`, and opens `Mp4Reader` with `Limits::default()` followed by
`restrict_packet_bytes(options.max_packet_bytes)` and
`validate_selected_prefix_packet_budget` in `owned_mp4_remux`
(`crates/fvid-media/src/owned_mp4_remux.rs`), `mp4_concat::open_with_options`
(`src/container/mp4_concat.rs`), and `native_pcm::loudness`
(`src/native_pcm/loudness.rs`). Three focused integration tests, 4 related test
suites, and 1 offline fixture unit test pass. See `batch-267-validation.json`.

## Batch 268 — Scope Y4M export, trim, and concat packet-byte admission and segment compatibility validation to the active max_packets prefix

FVID-0275 adds `validate_video_packet_budget_with_geometry_and_limit` and
`y4m_segment_frame_upper_bound` (`src/native_export.rs`), scopes
`export_y4m_sources_with_options` packet-budget validation to
`remaining_packets` when `interval.is_none_or(|(from, _)| from.is_zero())`,
checks `control.packet_limit_reached()` before `reader.read_frame_raw()?` so
trailing oversized packets are never read after reaching `max_packets`, and
bounds `concat_y4m_with_options` and `trim_y4m_with_options`
(`src/native_plan.rs`) packet-budget and segment-compatibility checks to the
active `max_packets` prefix. Two focused integration tests, 5 related test
suites, and 1 offline fixture unit test pass. See `batch-268-validation.json`.

## Batch 269 — Scope native AAC and audio source-info PS/SBR negotiation and packet-byte admission to the active max_packets prefix and interval across MP4, Matroska, and ADTS

FVID-0276 adds `aac_source_info_selected_with_options`,
`audio_source_info_selected_with_options`,
`audio_source_info_selected_with_interval_and_options`,
`is_owned_audio_source_with_options`, `is_owned_audio_trim_source_with_options`,
`is_aac_trim_source_with_options`, and `inspect_adts_with_options`
(`src/native_media.rs`), and threads caller `CopyOptions` (`max_packets`,
`max_packet_bytes`, `cancel`, `max_rss_bytes`) and decode intervals through
`src/native_plan.rs`, `src/native_pcm/loudness.rs`,
`src/native_pcm/normalize.rs`, `src/native_loudnorm.rs`,
`src/native_audio_mix.rs`, `src/native_audio_mix/concat.rs`,
`src/native_export.rs`, and `src/media.rs`. Two focused integration tests, 9
related tests across 5 suites, and 1 offline fixture unit test pass. See
`batch-269-validation.json`.

## Batch 270 — Negotiate implicit SBR output clock in owned ADTS loudness and loudnorm qualification, include budget notes in ADTS loudness and loudnorm plans, and scope ADTS loudnorm plan preflight to the active max_packets prefix

FVID-0277 adds `validate_header_configuration` and `adts_prefix_info` fallback
in `owned_adts_loudness` (`crates/fvid-media/src/owned_adts_loudness.rs`) and
verifies `qualified_rate(spool.stats.sample_rate)` after `decode_to_wave` in
`owned_adts_loudness::measure_loudness` and `owned_adts_loudnorm::apply`
(`crates/fvid-media/src/owned_adts_loudnorm.rs`) so implicit-SBR 11025->22050 Hz
ADTS inputs are admitted while metadata-only `plan_loudness` on standard-rate
headers preserves dry-run packet fault behavior, appends budget notes to
`owned_adts_loudness::plan_loudness` and `owned_adts_loudnorm::plan`, and bounds
the ADTS packet-byte loop in `native_loudnorm::try_plan`
(`src/native_loudnorm.rs`) by `options.max_packets`. One focused integration
test, 8 related tests across 4 suites, and 1 offline fixture unit test pass. See
`batch-270-validation.json`.

## Batch 271 — Scope ADTS concat and remux packet-byte admission to the active max_packets prefix while verifying segment compatibility across all inputs, and negotiate implicit SBR output rate with budget notes in owned ADTS remux and concat plans

FVID-0278 adds `StreamReader::restrict_packet_bytes`
(`crates/fvid-media/src/owned_aac/adts_impl.rs`) and opens all ADTS concat
segments with `StreamReader::open` followed by
`restrict_packet_bytes(options.max_packet_bytes)` and `SequenceReader::new` in
`owned_concat::open_readers` (`crates/fvid-media/src/owned_concat.rs`),
`native_plan::concat_adts_with_options` (`src/native_plan.rs`), and
`native_export::concat_adts_aac_with_options` (`src/native_export.rs`) so ADTS
header validity and identical AAC configurations are verified across all
segments while `options.max_packet_bytes` is enforced only on packets actually
read within the active `options.max_packets` prefix, and probes implicit SBR
output clock (`adts::probe_output_rate`) and appends budget notes in
`owned_adts_remux::plan_remux` (`crates/fvid-media/src/owned_adts_remux.rs`) and
`owned_concat::plan_concat` (`crates/fvid-media/src/owned_concat.rs`). One
focused integration test, 10 related tests across 5 suites, and 1 offline
fixture unit test pass. See `batch-271-validation.json`.

## Batch 272 — Scope MP4 and ADTS audio decode, trim, and plan packet-byte admission, controlled-memory admission, sample-description reconfiguration checks, and in-band PS negotiation to the active decode interval prefix

FVID-0279 adds `mp4_active_samples`, `negotiate_mp4_ps_with_interval`,
`negotiate_mp4_description_ps_with_interval`,
`mp4_normalized_geometry_with_interval`, and
`mp4_normalized_geometry_controlled_with_interval`
(`crates/fvid-media/src/owned_mp4_audio_timeline_impl.rs`),
`admit_audio_reader_with_interval` (`crates/fvid-media/src/owned_mp4_audio.rs`),
`descriptor_for_decode` and `geometry_for_decode`
(`crates/fvid-media/src/owned_mp4_audio_export.rs`), `router_for_decode`
(`crates/fvid-media/src/owned_aac_export_routing.rs`), and
`inspect_adts_with_interval_and_options`,
`is_owned_audio_source_with_interval_and_options`,
`is_owned_audio_trim_source_with_interval_and_options`, and
`is_aac_trim_source_with_interval_and_options` (`src/native_media.rs`), and
wires the decode `interval` through
`crates/fvid-media/src/owned_mp4_audio_normalized_impl.rs`,
`crates/fvid-media/src/owned_audio_plan.rs`, `src/native_plan.rs`,
`src/native_export.rs`, and `src/media.rs`. Two focused integration tests, 5
related tests across 3 suites, and 1 offline fixture unit test pass. See
`batch-272-validation.json`.

## Batch 273 — Scope Matroska audio decode, trim, and plan packet-byte admission, controlled-memory admission, and in-band PS negotiation to the active decode interval prefix

FVID-0280 adds `matroska_active_track_packets` and
`negotiate_matroska_ps_with_interval`
(`crates/fvid-media/src/owned_matroska_audio_timeline_impl.rs` and
`src/native_media.rs`), `validate_audio_packet_budget_with_interval` and
`admit_audio_reader_with_interval`
(`crates/fvid-media/src/owned_matroska_audio.rs`), `geometry_for_decode` /
`geometry_with_mask_for_decode`
(`crates/fvid-media/src/owned_matroska_{aac,flac,opus,alac,mp3,pcm}_export.rs`),
`matroska_descriptor_for_decode` (`crates/fvid-media/src/owned_audio_plan.rs`),
`matroska_source_info_with_interval`
(`crates/fvid-media/src/owned_wave_metadata.rs`), and
`extract_source_pcm_metadata_with_interval_and_packet_limit`
(`src/native_export/pcm_matroska.rs`), and wires the decode `interval` through
`crates/fvid-media/src/owned_matroska_pcm.rs`,
`crates/fvid-media/src/owned_aac_export_routing.rs`, `src/native_media.rs`,
`src/native_plan.rs`, and `src/native_export.rs`. Two focused integration tests,
9 related tests across 4 suites, and 1 offline fixture unit test pass. See
`batch-273-validation.json`.

## Batch 274 — Scope linked Ogg Vorbis/Opus program packet-byte admission to the active link's presentation interval prefix

FVID-0281 computes the link-relative presentation upper bound `local_until` in
`Program::max_packet_size_in_prefix`
(`crates/fvid-media/src/owned_ogg_audio_program.rs`) when the link shares the
program's integer sample clock (`link.source.rate() == self.rate && link.origin.phase.0 == 0`)
and forwards `local_until` to `Source::Vorbis` and `Source::Opus`
`max_packet_size_in_prefix(local_until, remaining)` so trailing packets inside
an active Ogg link that start at or after `until` are not checked against
`options.max_packet_bytes`. One focused integration test, 12 related tests
across 4 suites, and 2 offline fixture unit tests pass. See
`batch-274-validation.json`.

## Batch 275 — Scope single-edit MP4 audio packet-byte, controlled-memory, and in-band PS admission to the active max_packets and presentation interval prefix

FVID-0282 updates `mp4_active_samples`
(`crates/fvid-media/src/owned_mp4_audio_timeline_impl.rs`) to bound by
`max_packets` whenever `track.edits.iter().filter(|e| e.media_time >= 0).count() <= 1`
and bound by `interval` when `track.edits.is_empty() || (track.edits.len() == 1 && track.edits[0].media_time == 0)`,
and wires `mp4_active_samples` and single-media-edit packet counting through
`src/native_loudnorm.rs`, `src/native_pcm/loudness.rs`,
`src/native_audio_mix.rs`, and `src/native_audio_mix/concat.rs`. Two focused
integration tests, 7 related tests across 3 suites, and 2 offline fixture unit
tests pass. See `batch-275-validation.json`.

## Batch 276 — Enforce non-empty WAVE and G.711 WAVE sample and interval admission across owned and native decode, trim, loudness, loudnorm, and concat planning

FVID-0283 validates non-zero sample count (`size > 0` after `options.max_packets`
prefix bounding) and non-empty resampled output-clock `transform.interval`
windows in `owned_g711_wave_export::{descriptor_with_transform, supports}`
(`crates/fvid-media/src/owned_g711_wave_export.rs`),
`owned_audio_plan::source_descriptor_for_decode`
(`crates/fvid-media/src/owned_audio_plan.rs`), and
`owned_audio_export::supports` (`crates/fvid-media/src/owned_audio_export.rs`),
and rejects 0-sample WAVE and G.711 WAVE inputs in
`owned_container_loudness::geometry_unqualified`
(`crates/fvid-media/src/owned_container_loudness.rs`),
`owned_loudnorm::{supports, supports_dual, plan_loudnorm}`
(`crates/fvid-media/src/owned_loudnorm.rs`), and `native_loudnorm::try_plan`
(`src/native_loudnorm.rs`). Five focused integration tests, 6 related tests
across 2 suites, and 2 offline fixture unit tests pass. See
`batch-276-validation.json`.

## Batch 277 — Scope Y4M interval export, trim, and trim planning packet-byte budget to the active presentation interval and post-start max_packets prefix

FVID-0284 adds
`validate_video_packet_budget_with_geometry_interval_and_limit`
(`src/native_export.rs`) and wires it into `export_y4m_sources_with_options`
(`src/native_export.rs`) and `native_plan::trim_y4m_with_options`
(`src/native_plan.rs`) so non-reordered MP4 and Matroska (`V_VP9`, `V_AV1`,
`V_FFV1`) packet-byte checks stop at the interval end `to` and after
`max_packets` emitted frames within `[from, to)` (including pre-roll before
`from`). Three focused integration tests, 8 related tests across 3 suites, and
1 offline fixture unit test pass. See `batch-277-validation.json`.

## Batch 278 — Scope Matroska FFV1 lossless interval export, decode, and planning packet-byte budget to the active presentation prefix

FVID-0285 moves the per-packet `reader.packet_payload_size(index) > options.max_packet_bytes`
check into `Presentation::next`
(`crates/fvid-media/src/owned_webm_presentation.rs`) after `pre_read_stop` and
`max_packets` enforcement and removes the full-file pre-scan loops from
`decode_webm` (`crates/fvid-media/src/owned_video_decode.rs`) and
`owned_ffv1_export::input_limited`
(`crates/fvid-media/src/owned_ffv1_export.rs`). Four focused integration tests,
45 related `owned_ffv1_export` unit tests, and 1 offline fixture unit test
pass. See `batch-278-validation.json`.

## Batch 279 — Enforce foreground Y4M overlay packet-byte budget on Matroska and MP4 sources and non-empty selected frame count in lossless transcode planning

FVID-0286 validates `ov_header.frame_len()? <= options.max_packet_bytes` for
Y4M foreground overlays in `decode_webm`
(`crates/fvid-media/src/owned_video_decode.rs`) and enforces
`stats.video_frames > 0` across Y4M, single-track Matroska/MP4, and multitrack
Matroska/MP4 dry-runs in `plan_transcode_lossless`
(`crates/fvid-media/src/owned_lossless.rs`),
`owned_webm_multitrack_export::plan_selection`
(`crates/fvid-media/src/owned_webm_multitrack_export.rs`), and
`owned_mp4_multitrack_export::plan_selection`
(`crates/fvid-media/src/owned_mp4_multitrack_export.rs`). Four focused
integration tests, 12 related tests across 4 suites, and 2 offline fixture unit
tests pass. See `batch-279-validation.json`.

## Batch 280 — Enforce non-empty video frame presence and cross-fade timeline offset admission in plan_xfade and plan_burn_subtitles under default CopyOptions

FVID-0287 runs `visit_y4m_counted` unconditionally in `plan_xfade`
(`crates/fvid-media/src/owned_xfade.rs`), runs
`visit_reader_transformed_counted` and `decode_webm` unconditionally in
`plan_burn_subtitles` (`crates/fvid-media/src/owned_subtitle_burn.rs`) while
gating per-frame FFV1 encoding on custom `max_packet_bytes`, and rejects
0-frame Y4M and Matroska inputs in both `plan_burn_subtitles` and
`burn_subtitles`. Four focused integration tests, 10 related tests across 4
suites, and 1 offline fixture unit test pass. See `batch-280-validation.json`.

## Batch 281 — Enforce non-empty video frame presence and non-empty presentation interval admission in Y4M trim and concat planning under default CopyOptions

FVID-0288 always calls `inspect_first_y4m_frame` on Y4M, MP4, and Matroska
inputs in `validate_video_packet_budget_with_geometry_interval_and_limit`
(`src/native_export.rs`) regardless of `options.max_packet_bytes`, and rejects
empty `[from, to)` presentation intervals on Y4M, non-reordered MP4, and
non-reordered Matroska inputs with `"input has no decoded video frames"`. Five
focused integration tests, 12 related tests across 3 suites, and 1 offline
fixture unit test pass. See `batch-281-validation.json`.

## Batch 282 — Validate first-frame presence, spatial geometry, and pixel filter plane compatibility in native_plan::transcode_lossless across Y4M, Matroska, and MP4 inputs

FVID-0289 adds `native_lossless_y4m::validate_first_frame`
(`src/native_lossless_y4m.rs`) and `native_lossless::validate_first_mp4_frame`
(`src/native_lossless.rs`) and invokes them from
`native_plan::transcode_lossless` (`src/native_plan.rs`) so lossless transcode
planning under default `CopyOptions` validates first-frame presence, spatial
geometry (`crop`, `pad`, `scale`, `transpose`, `rotate`), and pixel filter
sample planes with the exact same diagnostics as execution. Four focused
integration tests, 6 related tests across 3 suites, and 2 offline fixture unit
tests pass. See `batch-282-validation.json`.

## Batch 283 — Route CLI media decode with shuffleframes, reverse, and input-format through owned decode and reject zero-frame decodes and lossless Y4M plans

FVID-0290 adds `try_owned_decode_command` (`src/media_cli.rs`) to route
`fvid media decode` with `--shuffleframes`, `--reverse`, `--input-format`, and
all owned `DecodeTransform` options through `fvid::media::decode_video_transformed`
without requiring `--features media`, rejects 0-frame inputs and 0-emitted-frame
temporal filter selections in `owned_y4m_decode::decode_reader_transformed`
(`crates/fvid-media/src/owned_y4m_decode.rs`) and `owned_video_decode::decode_webm`
(`crates/fvid-media/src/owned_video_decode.rs`) with `"input has no decoded video frames"`,
and validates first-frame presence on Y4M inputs under default `CopyOptions` in
`owned_lossless::plan_transcode_lossless` (`crates/fvid-media/src/owned_lossless.rs`).
Five focused integration tests, 22 related tests across 3 suites, and 2 offline
fixture unit tests pass. See `batch-283-validation.json`.

## Batch 284 — Validate post-compositing pixel filters against post-geometry frames in native overlay and decode-overlay planning and record rotate in CLI overlay geometry plan steps

FVID-0291 adds `TimedOverlay::validate_pair_with_geometry_and_filters`
(`src/native_export/overlay.rs`), `overlay_with_geometry_and_filters`, and
`decode_overlay_with_geometry_and_filters` (`src/native_plan.rs`), wiring them
into `native_plan::transcode_lossless` (`src/native_plan.rs`) and
`try_owned_overlay` (`src/media_cli.rs`) so overlay and decode-overlay planning
validate post-compositing `PixelFilters` against the composited post-geometry
first frame and include `rotate` in the `plan overlay` geometry step detail.
Six focused integration tests, 9 related tests across 3 suites, and 2 offline
fixture unit tests pass. See `batch-284-validation.json`.

## Batch 285 — Update native decode pixel_format and Y4M export preflight packet budget after cross-plane shuffleplanes subsampling promotion and reject --rotate in CLI crop-lossless fallback

FVID-0292 recomputes `stats.pixel_format` from `output.subsampling` after
`filters.apply_colour_clock(...)` in `decode_video_pipeline_overlay_step`
(`src/native_media.rs`), adds `inspect_first_y4m_frame_with_filters` and
`validate_video_packet_budget_with_geometry_filters_interval_and_limit`
(`src/native_export.rs`) so `export_y4m_sources_with_options` validates
post-filter frame byte size and filter compatibility during preflight, and adds
`--rotate` rejection and `filters.canonicalize_option_order()` to the CLI
`crop-lossless` fallback branch (`src/media_cli.rs`). Seven focused integration
tests, 9 related tests across 3 suites, and 2 offline fixture unit tests pass.
See `batch-285-validation.json`.

## Batch 286 — Validate first-frame geometry and pixel filters on Y4M inputs in owned_lossless::plan_transcode_lossless under default packet budgets

FVID-0293 updates the default-packet-budget Y4M planning branch of
`owned_lossless::plan_transcode_lossless` (`crates/fvid-media/src/owned_lossless.rs`)
to validate the first transformed Y4M frame via `visit_reader_transformed_counted`,
so planning on Y4M sources with metadata/stream options (`--metadata`, `--streams`)
or on default-FPS Y4M files without an explicit `F` token enforces spatial
geometry (`crop`, `pad`, `scale`) and pixel filter dimensions (`boxblur`) just
like execution, and removes a duplicate `transform.unsharp.is_some()` check in
`owned_y4m_decode::apply_pixel_filters_clock` (`crates/fvid-media/src/owned_y4m_decode.rs`).
Eight focused integration tests, 8 related tests across 3 suites, and 2 offline
fixture unit tests pass. See `batch-286-validation.json`.

## Batch 287 — Scan ADTS packets and include Matroska packet-budget step in native_plan::remux_with_options and route CLI plan transcode --encoder ffv1 and shorthand plan INPUT in default builds

FVID-0294 updates `native_plan::remux_with_options` (`src/native_plan.rs`) to
scan ADTS packets through `container::adts::StreamReader::open_with_packet_limit`
so truncated or corrupt packets after the first ADTS frame are rejected by
`fvid::native_plan::remux`, `fvid::media::plan_remux`, and `fvid media plan remux`,
and includes the Matroska `budget` step directly in `remux_with_options` so CLI,
native, and public API Matroska remux plans match identically. It also extends
`src/media_cli.rs` to route `fvid media plan transcode INPUT --encoder ffv1` and
shorthand `fvid media plan INPUT` in default builds without `--features media`.
Five focused integration tests, 11 related tests across 3 suites, and 2 offline
fixture unit tests pass. See `batch-287-validation.json`.

## Batch 288 — Reject encoder selection on CLI media plan and reject duplicate or unknown options on CLI overlay and plan overlay

FVID-0295 updates `src/media_cli.rs` so `fvid media plan` rejects `--encoder`
and `--encoder-option` (`"plan does not take metadata mutation or encoder selection"`)
and shorthand `fvid media plan INPUT` rejects metadata mutation flags in default
builds without `--features media`, matching `run_native`. It also hardens
`try_owned_overlay` (`src/media_cli.rs`) to reject duplicate `--overlay`,
`--overlay-x`, and `--overlay-y` flags (`"duplicate overlay option"`,
`"duplicate overlay-x option"`, `"duplicate overlay-y option"`) and unknown
`--...` options (`"unsupported overlay option: ..."`) instead of silently
overwriting earlier flags or falling through to the feature-gate error. Nine
focused integration tests, 20 related tests across 3 suites, and 2 offline
fixture unit tests pass. See `batch-288-validation.json`.

## Batch 289 — Validate positional path arity and flag-prefixed destinations in owned lossless, xfade, and burn-subtitles CLI commands and ignore --progress across owned plan subcommands

FVID-0296 updates `owned_lossless_command_with_name`, `owned_xfade_command`,
`owned_subtitle_burn_command`, and the native `transcode-lossless` /
`crop-lossless` fast path in `src/media_cli.rs` so missing destination paths
(where a `--...` flag occupies the destination slot) and extra positional path
arguments are rejected with explicit command-arity errors instead of writing a
file named `--quiet` or falling through to the feature-gate error. It also
accepts and ignores `--progress` during read-only planning across `plan remux`,
`plan normalize-loudness`, `plan trim`, `plan trim-pcm`, `plan mix-audio`,
`plan merge-audio`, and `plan concat`, matching `run_native`. Ten focused
integration tests, 20 related tests across 3 suites, and 2 offline fixture unit
tests pass. See `batch-289-validation.json`.

## Batch 290 — Support optional destination path and destination-specific metadata validation in CLI plan loudnorm and enforce positional path arity in owned_loudnorm_cli

FVID-0297 updates `owned_loudnorm_cli::run` and `owned_loudnorm_cli::run_plan`
(`src/owned_loudnorm_cli.rs`) and `run_native` (`src/media_cli.rs`) to parse
positional paths and flags in any order, enforce required positional path arity,
and accept an optional destination path `INPUT [OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv]`
in `fvid media plan loudnorm` (matching `plan normalize-loudness`). When a
destination path is provided to `loudnorm` or `plan loudnorm`, its extension
(`.wav`, `.mka`, `.mkv`) and `.wav`-specific RIFF INFO metadata constraints
(rejecting `--stream-metadata`, `--stream-metadata-delete`, and non-RIFF-INFO
container tag keys) are validated upfront. Two focused integration tests, 4
related tests across 3 suites, and 2 offline fixture unit tests pass. See
`batch-290-validation.json`.

## Batch 291 — Separate container route qualification from packet-budget enforcement in native_loudnorm::try_plan and try_apply

FVID-0298 separates container/codec route qualification in `native_loudnorm::try_plan`
and `native_loudnorm::try_apply` (`src/native_loudnorm.rs`) from packet-budget
checks (`max_packet_bytes`, `max_packets`). Previously, passing `options` into
`is_owned_audio_trim_source_with_options` and
`audio_source_info_selected_with_options(...).is_ok()` swallowed
`"MP4 packet exceeds budget"` errors into `false` during route selection, causing
multi-track MP4 sources without explicit `--streams` (`alac-two-tracks.m4a`) to
return `Ok(None)` instead of `Err("MP4 packet exceeds budget")`. Nine focused
integration tests in `tests/native_loudnorm_cli.rs`, 5 related tests across 4
suites, and 2 offline fixture unit tests pass. See `batch-291-validation.json`.

## Batch 292 — Allow controlled memory budgets alongside Matroska metadata mutations in decode_audio and trim and support destination-aware CLI planning for decode-audio and trim

FVID-0299 updates `decode_fvid_media_audio_to_destination` and `plan_decode_audio`
(`src/media.rs`) so combining `--max-memory-mib` (`max_controlled_bytes`) with
Matroska container or stream 0 metadata mutations (`--metadata`,
`--metadata-delete`, `--stream-metadata`, `--stream-metadata-delete`) on `.mka`
and `.mkv` audio decodes and trims is admitted and enforces the controlled
memory budget during `fvid_media::decode_audio_transformed` and
`fvid_media::plan_decode_audio`. It also updates `plan decode-audio`,
`decode-audio`, and `try_native_audio_trim` (`src/media_cli.rs`) to accept
positional paths and flags in any order, accept an optional destination path on
`plan decode-audio`, `plan trim`, and `plan trim-pcm` with destination-specific
extension and metadata validation, and report direct errors on invalid
`decode-audio` invocations in default builds. Two focused integration tests, 16
related tests across 3 suites, and 1 offline fixture unit test pass. See
`batch-292-validation.json`.

## Batch 293 — Support destination-aware CLI planning and flexible argument ordering in plan remux, remux, plan loudness, and loudness

FVID-0300 updates `plan remux`, `remux`, `plan loudness`, and `loudness` in
`media_cli::run` and `media_cli::run_native` (`src/media_cli.rs`) to parse
positional paths and flags in any order, accept an optional destination path
`INPUT [OUTPUT.wav|OUTPUT.mp4|OUTPUT.m4a|OUTPUT.mka|OUTPUT.mkv]` on `plan remux`
with destination-specific extension and metadata validation (`.wav` rejects
stream metadata and validates RIFF `INFO` keys; `.mp4`/`.m4a` rejects metadata
mutations), and report direct CLI errors on invalid `remux`, `plan remux`,
`loudness`, and `plan loudness` invocations in default builds. Five focused
integration tests, 13 related tests across 3 suites, and 2 offline fixture unit
tests pass. See `batch-293-validation.json`.

## Batch 294 — Support flexible argument ordering and destination-aware metadata validation in CLI concat and plan concat

FVID-0301 updates `try_native_audio_concat`, `try_video_concat`, and
`try_mp4_matroska_concat` (`src/media_cli.rs`) so CLI `concat` and `plan concat`
accept flags and positional paths in any order, validate destination extensions
and `.wav` / `.mp4` / `.m4a` metadata constraints before concatenating or
planning, and report direct CLI errors on missing inputs, unsupported output
extensions, or unknown concat flags in default builds. Five focused integration
tests, 13 related tests across 3 suites, and 2 offline fixture unit tests pass.
See `batch-294-validation.json`.

## Batch 295 — Preserve packet-budget, extension, and option validation diagnostics in CLI mix-audio, merge-audio, convert-subtitles, and plan convert-subtitles

FVID-0302 updates `native_mix`, `native_merge`, and `convert-subtitles` /
`plan convert-subtitles` in `src/media_cli.rs` and adds
`plan_convert_subtitles_for_destination` in `src/media.rs` so `.mka` / `.mkv`
outputs on `mix-audio` and `merge-audio` propagate specific packet-budget and
extension diagnostics (`"FLAC packet bytes exceed limit"`,
`"mix-audio requires .wav, .mka or .mkv output"`) instead of falling through to
the `media` feature-gate message, unknown flags and missing positional arguments
are rejected directly on `mix-audio`, `merge-audio`, `convert-subtitles`, and
`plan convert-subtitles`, and optional destination paths on
`plan convert-subtitles INPUT [OUTPUT.mkv]` are validated across both default
and `media`-enabled CLI paths. Three focused integration tests, 10 related tests
across 3 suites, and 2 offline fixture unit tests pass. See
`batch-295-validation.json`.

## Batch 296 — Enforce .y4m destination extension in export_y4m_pipeline_with_options and support flexible flag ordering and direct diagnostics in export-y4m, transcode-lossless, crop-lossless, xfade, and burn-subtitles CLI

FVID-0303 requires `.y4m` destination extensions in
`fvid::native_export::export_y4m_pipeline_with_options` (`src/native_export.rs`),
resolves the first non-flag input path in `pixel_decode_args`, and updates
`export-y4m`, `owned_lossless_command_with_name`, `owned_xfade_command`, and
`owned_subtitle_burn_command` (`src/media_cli.rs`) so `export-y4m`,
`transcode-lossless`, `crop-lossless`, `xfade`, `burn-subtitles`, and their
`plan` subcommands accept flags and positional paths in any order and report
direct CLI diagnostics on missing positional paths or unknown options in default
builds instead of falling through to the `media` feature-gate message. Eleven
focused integration tests, 9 related tests across 3 suites, and 2 offline
fixture unit tests pass. See `batch-296-validation.json`.

## Batch 297 — Support flexible flag ordering in video trim CLI and preserve WAVE decode routing and direct diagnostics in audio trim CLI

FVID-0304 updates `try_video_trim` and `try_native_audio_trim` in
`src/media_cli.rs` and `fvid::media::trim` in `src/media.rs` so Y4M `trim` and
`plan trim` accept flags and positional paths in any order, missing positional
paths and unsupported output extensions (`bad.mp3`) on `trim`, `trim-pcm`,
`plan trim`, and `plan trim-pcm` report direct CLI diagnostics instead of
falling through to the `media` feature-gate message in default builds, and
non-WAVE container audio trimmed to `.wav` routes through controlled audio
decoding in `fvid::media::trim`. Five focused integration tests, 15 related
tests across 3 suites, and 2 offline fixture unit tests pass. See
`batch-297-validation.json`.

## Batch 298 — Route WAVE-to-.f32le trim with custom budgets through controlled audio decoding and preflight destination metadata constraints in CLI decode-audio

FVID-0305 updates `fvid::media::trim` (`src/media.rs`), `try_native_audio_trim`
(`src/media_cli.rs`), and CLI `decode-audio` (`src/media_cli.rs`) so trimming a
WAVE input to `.f32le` with `max_controlled_bytes` (`--max-memory-mib`) or
custom packet budgets routes through `decode_fvid_media_audio_to_destination`
instead of failing in `trim_pcm` (`"owned WAVE remux requires .wav output"`),
and CLI `decode-audio` preflights `.f32le` and `.wav` metadata constraints to
match `plan decode-audio`. Two focused integration tests, 9 related tests across
3 suites, and 1 offline fixture unit test pass. See `batch-298-validation.json`.

## Batch 299 — Enforce .y4m destination extension and single-video-track selection in export_y4m, export_y4m_interval, export_y4m_transformed, and export_y4m_pipeline and support flexible flag ordering in CLI probe

FVID-0306 routes `export_y4m_transformed` and `export_y4m_pipeline` through
`export_y4m_pipeline_with_options` (`src/native_export.rs`) so all public
`export_y4m*` entrypoints enforce `.y4m` destination extensions and
single-video-track selection, and updates CLI `probe` (`src/media_cli.rs`) to
accept `--input-format` and `--quiet` / `-q` in any order relative to `INPUT`.
Eleven focused integration tests, 8 related tests across 3 suites, and 2 offline
fixture unit tests pass. See `batch-299-validation.json`.

## Batch 300 — Support flexible flag ordering and direct diagnostics in CLI transcode, transcode-lossless, and crop-lossless

FVID-0307 updates `transcode`, `transcode-lossless`, and `crop-lossless` in
`src/media_cli.rs` so `transcode` accepts flags (`--quiet`, `--encoder`,
`--encoder-option`, geometry and filter flags) and positional `INPUT` and
`OUTPUT` paths in any order, reports direct diagnostics when `INPUT`/`OUTPUT`
or `--encoder` are missing or when an encoder is unsupported in default builds,
and rejects unknown flags on `transcode-lossless` and `crop-lossless` with
direct CLI errors instead of falling through to the `media` feature-gate
message. Eleven focused integration tests, 8 related tests across 3 suites, and
2 offline fixture unit tests pass. See `batch-300-validation.json`.

## Batch 301 — Support flexible flag ordering and direct diagnostics in CLI shorthand plan and decode

FVID-0308 updates shorthand `plan` (`fvid media plan INPUT [flags]`) and
`try_owned_decode_command` (`fvid media decode`) in `src/media_cli.rs` so
shorthand `plan` accepts flags (`--quiet`, `--hflip`, filter/geometry and
budget flags) and `INPUT` in any order, reports direct diagnostics when `INPUT`
is missing or extra or when metadata flags are supplied, and `decode` reports
direct diagnostics on missing/extra `INPUT` (`"decode requires a single INPUT"`)
or unknown flags (`"unsupported decode option: <flag>"`) in default builds
instead of falling through to the `media` feature-gate message. Eleven focused
integration tests, 8 related tests across 3 suites, and 2 offline fixture unit
tests pass. See `batch-301-validation.json`.

## Batch 302 — Support destination-aware planning ([OUTPUT] and --output-format) in CLI plan mix-audio and plan merge-audio

FVID-0309 adds `resolve_plan_mix_or_merge_destination` and `--output-format`
support in `native_mix`, `native_merge`, and `run_native` (`src/media_cli.rs`)
so `fvid media plan mix-audio` and `fvid media plan merge-audio` accept an
optional leading `OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv` or
`--output-format wav|mka|mkv|matroska`, validate `.wav` vs Matroska metadata
constraints before planning, and reject unsupported destination formats or
extensions cleanly. One focused integration test suite, 18 related tests across
3 suites, and 1 offline fixture unit test pass. See `batch-302-validation.json`.

## Batch 303 — Support destination-aware planning ([OUTPUT] and --output-format) across CLI plan concat handlers

FVID-0310 adds `resolve_plan_concat_destination` and `--output-format` support
across `try_mp4_matroska_concat`, `try_video_concat`, `try_native_audio_concat`,
and `run_native` (`src/media_cli.rs`) so `fvid media plan concat` accepts an
optional leading destination path
`[OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv|OUTPUT.mp4|OUTPUT.m4a|OUTPUT.y4m]` or
`--output-format wav|mka|mkv|matroska|mp4|m4a|y4m`, validates `.wav` and
`.mp4`/`.m4a` metadata constraints before planning, and rejects unsupported
destination formats or extensions cleanly. Five focused integration tests, 14
related tests across 3 suites, and 2 offline fixture unit tests pass. See
`batch-303-validation.json`.

## Batch 304 — Enforce .mkv output extension across owned FFV1 lossless export, overlay, xfade, and subtitle burn-in and preserve crop-lossless plan diagnostics

FVID-0311 validates `.mkv` destination extensions up front across
`fvid_media::owned_lossless::transcode_lossless`,
`fvid_media::owned_xfade::xfade_video`,
`fvid_media::owned_subtitle_burn::burn_subtitles`, and `try_owned_overlay`
(`src/media_cli.rs`), routes `try_owned_overlay` through
`fvid_media::owned_lossless` when temporal flags (`--from`, `--to`,
`--framestep`, `--reverse`, `--shuffleframes`) or sources supported by
`owned_lossless` are used, and preserves the
`"plan crop-lossless requires a single INPUT"` diagnostic in
`owned_lossless_command_with_name`. Twelve focused integration tests pass. See
`batch-304-validation.json`.

## Batch 305 — Support destination-aware planning ([OUTPUT.mkv] and --output-format) across CLI plan transcode-lossless, crop-lossless, overlay, xfade, and burn-subtitles

FVID-0312 accepts an optional trailing `OUTPUT.mkv` destination path and
`--output-format mkv|matroska` during planning across
`owned_lossless_command_with_name`, `try_owned_overlay`, `owned_xfade_command`,
`owned_subtitle_burn_command`, and `run_native` (`src/media_cli.rs`), rejecting
non-`.mkv` destination extensions or output formats with
`"owned FFV1 export requires .mkv"` without creating files. Thirteen focused
integration tests pass. See `batch-305-validation.json`.

## Batch 306 — Support --output-format across CLI plan remux, trim, trim-pcm, decode-audio, normalize-loudness, loudnorm, and convert-subtitles and optional OUTPUT.y4m in Y4M plan trim

FVID-0313 supports `--output-format` during planning across `plan remux`,
`plan trim`, `plan trim-pcm`, `plan decode-audio`, `plan normalize-loudness`,
`plan loudnorm`, and `plan convert-subtitles` in `src/media_cli.rs` and
`src/owned_loudnorm_cli.rs`, validating destination-specific metadata
constraints (`.wav`, `.f32le`, `.mp4`/`.m4a`, `.mkv`) before planning, and
accepts an optional `OUTPUT.y4m` destination path in `try_video_trim` during
planning. Seven focused integration tests across two suites pass. See
`batch-306-validation.json`.

## Batch 307 — Support destination/metadata/output-format options in shorthand CLI plan and --max-memory-mib plus Y4M metadata diagnostics in Y4M video concat and trim

FVID-0314 updates shorthand `fvid media plan INPUT [OUTPUT]` in `run` and
`run_native` (`src/media_cli.rs`) to accept `1..=2` paths, metadata flags, and
`--output-format` without misclassifying `--output-format` as a video transform
flag, and admits `--max-memory-mib` plus explicit
`"Y4M output does not support metadata mutations"` checks across
`try_video_concat`, `try_video_trim`, `resolve_plan_concat_destination`,
`run_native`, `src/native_export.rs`, and `src/native_plan.rs`. Fourteen focused
integration tests pass. See `batch-307-validation.json`.

## Batch 308 — Add export-y4m read-only planning and enforce remux duplicate --streams and destination metadata validation

FVID-0315 adds `fvid::native_plan::export_y4m_with_options`
(`src/native_plan.rs`) and CLI `fvid media plan export-y4m`
(`src/media_cli.rs`), parses metadata flags in `export-y4m` so Y4M metadata
mutations fail with `"Y4M output does not support metadata mutations"`, rejects
duplicate `--streams` in `remux` and `plan remux` with
`"duplicate streams option"`, and validates `.wav` and `.mp4`/`.m4a` metadata
constraints in `remux` matching `plan remux`. Six focused integration tests
pass. See `batch-308-validation.json`.

## Batch 309 — Admit and enforce --max-memory-mib (max_controlled_bytes) across owned video exports and plans (transcode-lossless, crop-lossless, overlay, xfade, and burn-subtitles)

FVID-0316 admits `max_controlled_bytes` (`--max-memory-mib`) across
`owned_lossless`, `owned_matroska`, `owned_mp4_video_bridge`,
`owned_video_decode`, `owned_xfade`, and `owned_subtitle_burn`, enforces the
controlled memory budget against Y4M/Matroska/MP4 frame and packet working
buffers during both planning and execution before any output publication, and
records `controlled memory limit: {:?} bytes` in `plan_transcode_lossless`,
`plan_xfade`, and `plan_burn_subtitles` budget notes. Fifteen focused
integration tests pass. See `batch-309-validation.json`.

## Batch 310 — Admit and enforce --max-memory-mib (max_controlled_bytes) across Matroska identity remux, MP4 fast-start relocation, MP4-to-Matroska remux/concat, and ADTS remux/concat

FVID-0317 admits and enforces `max_controlled_bytes` (`--max-memory-mib`)
across Matroska identity remux, MP4 fast-start relocation, MP4-to-Matroska
remux and concat, and ADTS remux and concat during both planning and execution
before any output publication, and records `controlled memory limit` in all
remux and concat plan budget notes. Seven focused integration tests pass. See
`batch-310-validation.json`.

## Batch 311 — Admit and enforce --max-memory-mib (max_controlled_bytes) across convert-subtitles and plan convert-subtitles for SRT, ASS, and Matroska inputs

FVID-0318 admits and enforces `max_controlled_bytes` (`--max-memory-mib`)
across `try_plan_with_controls`, `try_convert_with_controls`,
`try_convert_matroska_inner`, `try_convert_ass_inner`, and
`try_convert_srt_inner` in `crates/fvid-media/src/owned_subtitle_impl.rs`,
validating script/packet working-set memory before any output publication and
recording `controlled memory limit: {:?}` in `plan convert-subtitles` budget
notes, and updates legacy refusal expectations in
`tests/subtitle_convert_burn_budget_parity_regressions.rs`. Four focused
integration tests across two suites pass. See `batch-311-validation.json`.

## Batch 312 — Enforce early controlled-memory budget admission and budget note reporting in WAVE/native plan_loudnorm and validate Matroska destination metadata in loudnorm CLI and plan loudnorm

FVID-0319 validates `options.max_controlled_bytes` during
`fvid_media::owned_loudnorm::plan_loudnorm` (`crates/fvid-media/src/owned_loudnorm.rs`)
using the same linear, dynamic, and report/dual-pass memory estimates as
execution, records `controlled memory limit` in both
`fvid_media::owned_loudnorm::plan_loudnorm` and `fvid::native_loudnorm::try_plan`
(`src/native_loudnorm.rs`) budget notes, and calls
`validate_matroska_destination_metadata` in `validate_loudnorm_destination`
(`src/owned_loudnorm_cli.rs`) when `.mka`/`.mkv` or `--output-format mka|mkv`
is supplied. Three focused integration tests pass. See
`batch-312-validation.json`.

## Batch 313 — Route CLI WAVE trim and plan trim to non-.wav destinations (.f32le, .mka, .mkv) through decode-audio and validate Matroska destination metadata in decode-audio and trim CLI/plan routes

FVID-0320 routes CLI `trim` from WAVE inputs to `.f32le`, `.mka`, or `.mkv`
through `fvid::media::trim` (`src/media_cli.rs`) regardless of `custom_limits`,
routes `plan trim` from WAVE inputs to `.f32le`, `.mka`, or `.mkv` (via
destination extension or `--output-format`) through
`fvid::media::plan_decode_audio`, and validates Matroska destination metadata in
`plan decode-audio`, `decode-audio`, `plan trim`, and `trim` when `.mka` or
`.mkv` is targeted. Three focused integration tests pass. See
`batch-313-validation.json`.

## Batch 314 — Enforce controlled-memory budget (max_controlled_bytes) against MP4/Matroska encoded packets and transformed Y4M output frames across export-y4m, trim_y4m, and concat_y4m plans and execution

FVID-0321 enforces `options.max_controlled_bytes` against MP4 and Matroska/WebM
encoded packet sizes and against the transformed Y4M output frame size across
Y4M, MP4, and Matroska/WebM inputs in
`validate_video_packet_budget_with_geometry_filters_interval_and_limit`
(`src/native_export.rs`), covering `export-y4m`, `plan export-y4m`, `trim` to
`.y4m`, `plan_trim_y4m`, `concat` to `.y4m`, and `plan_concat_y4m`. Eight
focused integration tests pass. See `batch-314-validation.json`.

## Batch 315 — Enforce controlled-memory budget (max_controlled_bytes) against encoded FFV1 packets, subtitle burn rasterization masks, and multitrack MP4/WebM companion packets across owned lossless, xfade, and subtitle burn plans and exports

FVID-0322 enforces `options.max_controlled_bytes` against encoded FFV1 packet
sizes across `owned_lossless`, `owned_ffv1_export`, `owned_mp4_video_bridge`,
`owned_xfade`, and `owned_subtitle_burn` in both planning and execution,
includes the `width * height` text rasterization mask in `owned_subtitle_burn`
controlled-memory checks, and bounds MP4/WebM multitrack companion packet sizes
and PCM staging chunks by `options.max_controlled_bytes` in
`owned_mp4_multitrack_export`, `owned_webm_multitrack_export`, and
`owned_webm_pcm_window`. Sixteen focused integration tests pass. See
`batch-315-validation.json`.

Delivery verification uses a real 1 KiB BufReader and preserves the earlier raw telemetry in the evidence logs.

## Batch 317 — Early MP4 relocation metadata admission

FVID-0323 checks the span-table controlled-memory estimate before scanning or growing it. The 64 KiB refusal reads zero box bytes instead of 80,032. Generous-budget relocation preserves every byte; native VP9 playback checks exact RGB, intervals, repeated EOF and rewind. Three player regressions and four related relocation tests passed. See `batch-317-validation.json`.

## Batch 318 — Retained MP4 concat plan memory

FVID-0324 counts the capacity of all retained timing tables, including unselected tracks. A synthetic 2/512-packet AAC fixture reproduces admission under a 40 KiB limit before the fix. Three focused tests and six related concat tests passed, including actual AAC decoding and AVC/HEVC frame parity. See `batch-318-validation.json`.

## Batch 316 — Enforce controlled-memory budget (max_controlled_bytes) against output WAVE INFO and Matroska metadata and PCM muxer working buffers across mix-audio, merge-audio, sample-domain concat, decode-audio, trim, normalize-loudness, and loudnorm plans and execution

FVID-0325 adds `pcm_matroska::check_output_memory_budget` (`src/native_export/pcm_matroska.rs`) and `Sink::new_with_source_metadata` working-set admission, and enforces `options.max_controlled_bytes` against output WAVE INFO and Matroska container/stream metadata plus mixer/merger/concat/publish working buffers across `mix-audio`, `merge-audio`, sample-domain `concat`, `decode-audio`, `trim`, `normalize-loudness`, and `loudnorm` in both planning and execution, while forwarding `max_rss_bytes` and `max_packet_bytes` during `decode_source_to_f32le_with_options` (`src/native_audio_mix.rs`). Eight integration tests across five suites pass. See `batch-316-validation.json`.

FVID-0324 follow-up: edited AAC remux counts retained timing capacity too. A 512-to-2 packet edit reproduces admission under 20 KiB. Three focused and five related tests pass after preserving parallel metadata accounting; successful output decodes exactly 2048 mono samples at 48 kHz. No additional ID is counted.

## Batch 319 — Enforce controlled-memory budget (max_controlled_bytes) against Matroska container/stream metadata working sets and MP4 sample-index (sizes/stsz/moov) working sets across ADTS and MP4-to-Matroska remux, plan_remux, concat, and plan_concat

FVID-0326 adds `estimate_adts_matroska_metadata_bytes` (`crates/fvid-media/src/owned_matroska_adts_impl.rs`), includes container and stream metadata and chapter working sets in MP4-to-Matroska `validate_selected_prefix_packet_budget` (`crates/fvid-media/src/owned_mp4_remux.rs`), and enforces `options.max_controlled_bytes` against both Matroska metadata working sets and MP4 `stsz`/`moov` sample-index working sets (`packets * 32 + 16384`) across ADTS and MP4-to-Matroska `plan_remux`, `remux`, `plan_concat`, and `concat`. Nine integration tests in `remux_concat_controls_regressions` pass. See `batch-319-validation.json`.

## Batch 320 — Selected MP4 concat presentation clock

FVID-0327 excludes unselected tracks from segment duration. An owned 40 ms AVC / 10.923 s AAC fixture previously produced a 10.9-second gap when only video was selected. Exact 0/40 ms output PTS, 80 ms duration, RGB parity, rewind, repeated EOF and six seeks passed. Two focused and nine related player tests, plus seven default tests, passed. See `batch-320-validation.json`.

## Batch 321 — Selected MP4 concat compatibility

FVID-0328 compares only selected track configurations across segments. Identical selected AVC video now concatenates when discarded AAC changes between 48/44.1 kHz or is absent. Selected incompatible audio still refuses. Six focused and eleven related player tests, and five focused plus four related default tests passed. Public API, exact RGB, duration, seek, EOF and both input orders are covered. See `batch-321-validation.json`.

## Batch 322 — Enforce controlled-memory budget (max_controlled_bytes) against output WAVE INFO metadata in fvid-media loudnorm and decode-audio plans and execution and against Matroska container/stream/chapter metadata in compressed MP4-to-Matroska concat

FVID-0329 includes source `LIST`/`INFO` and `metadata_set` working sets (`8 * (metadata_bytes + assignments + 12)`) in `owned_loudnorm` (`plan_loudnorm`, `apply_resolved`, `apply_dynamic`), adds `check_wave_metadata_budget` (`crates/fvid-media/src/owned_adts_export.rs`) and enforces it in `owned_audio_plan::plan_decode_audio`, `owned_container_loudnorm`, and `owned_adts_loudnorm`, and includes Matroska container/stream/chapter metadata working sets in `mp4_concat::open_with_options` (`src/container/mp4_concat.rs`). Eight integration tests across `audio_budget_controls_regressions` and `mp4_concat_retained_plans` pass. See `batch-322-validation.json`.


FVID-0328 follow-up: initial video sync admission now applies only to selected tracks. A conservative empty video stss no longer blocks selected AAC. Video/all-track refusal remains; four AAC packet bytes and 4096 decoded mono samples at 48 kHz are verified. Three focused plus twelve related player tests, and three focused plus five related default tests passed. No extra ID.

## Batch 323 — Enforce controlled-memory budget (max_controlled_bytes) against Matroska container/stream/chapter metadata working sets and multi-frame temporal and overlay working sets across transcode_lossless, xfade, and burn_subtitles

FVID-0330 adds `estimate_matroska_metadata_working_bytes`, `check_options_matroska_metadata_budget`, `check_source_matroska_metadata_budget`, and `estimate_transform_working_bytes` (`crates/fvid-media/src/owned_lossless.rs`), exposes `ShuffleFrames::group_len` (`crates/fvid-media/src/owned_shuffleframes.rs`) and `TemporalMix::frame_count` (`crates/fvid-media/src/owned_tmix.rs`), and enforces `options.max_controlled_bytes` against Matroska metadata working sets and multi-frame temporal/overlay working sets across `transcode_lossless`, `plan_transcode_lossless`, `xfade_video`, `plan_xfade`, `burn_subtitles`, and `plan_burn_subtitles`. Seventeen integration tests in `lossless_plan_regressions` pass. See `batch-323-validation.json`.

FVID-0323 capacity follow-up: span-table growth is admitted before reserve and final checks use actual capacity; moov-index discovery no longer allocates a separate vector, and controlled order storage has a known bound. A 580,000-byte budget previously admitted at least 589,824 bytes of span storage and copy buffer. The frozen original regression fails precisely on this refusal; all 16 default and 17 player regressions now pass. Synthetic playback, bytes, cancellation, progress and publication remain checked. No extra ID or RSS/performance claim.

## Batch 324 — Enforce controlled-memory budget (max_controlled_bytes) against multi-frame temporal pixel filter working sets (tmix, lagfun, hqdn3d) in native export-y4m and plan export-y4m across Y4M, MP4, and Matroska inputs

FVID-0331 exposes `TemporalMix::frame_count` (`crates/fvid-media/src/owned_tmix.rs`), adds `PixelFilters::working_bytes(output_frame_len, depth)` (`src/native_pixels.rs`) accounting for `tmix`, `lagfun`, and `hqdn3d` temporal working sets, and enforces `filters.working_bytes(bytes, depth) <= mem_limit` across Y4M, MP4, and Matroska inputs in `validate_video_packet_budget_with_geometry_filters_interval_and_limit` (`src/native_export.rs`). Eighteen integration tests in `lossless_plan_regressions` pass. See `batch-324-validation.json`.

## Batch 325 — MP4 parser metadata admission

FVID-0332 propagates controlled-memory ceilings into MP4 parser metadata limits for remux, concat and native route/planning probes. A legal 1MiB moov/free allocation was previously admitted by a 64KiB remux plan. Budget errors remain specific and unknown sample-entry refusal stays intact. Seven focused plus twelve related player tests, and six focused plus six related default tests passed, including actual RGB/seek/EOF acceptance with ample budget. See `batch-325-validation.json`. This is distinct from output tag/chapter estimation and is not a complete RSS bound.

## Batch 326 — Enforce controlled-memory budget (max_controlled_bytes) against output Matroska container and stream metadata working sets in convert-subtitles and plan convert-subtitles across SRT, ASS, and Matroska inputs

FVID-0333 adds `estimate_subtitle_matroska_metadata_working_bytes` (`crates/fvid-media/src/owned_subtitle_impl.rs`) and includes output Matroska container and stream 0 metadata working sets (`metadata_set`, `stream_metadata_set`, and preserved source Matroska tags) in `controls.max_controlled_bytes` admission across `try_convert_matroska_inner`, `try_convert_ass_inner`, and `try_convert_srt_inner`. Five integration tests across `subtitle_convert_burn_budget_parity_regressions` and `subtitle_normalize_metadata_regressions` pass. See `batch-326-validation.json`.

FVID-0332 audio follow-up: metadata ceilings now reach indexed MP4 audio readers, geometry/planning, f32/f64 decode, WAVE metadata extraction and option-aware native AAC probes. A 17ms PCM file with legal 2MiB moov padding previously bypassed a 1MiB ceiling. Six focused and eight related default tests, and six focused plus nine related player tests passed. Ample-budget decoding has exactly 136 mono samples at 8kHz and byte-identical WAVE format/PCM data. Refusal leaves no temporary output. No extra ID.

## Batch 327 — Propagate controlled-memory ceilings (max_controlled_bytes and NativeReader budget) to MP4 parser metadata_bytes admission across transcode_lossless, plan_transcode_lossless, burn_subtitles, plan_burn_subtitles, export-y4m, trim-y4m, concat-y4m, and NativeReader

FVID-0334 clamps `Limits::metadata_bytes` to `options.max_controlled_bytes` in `owned_mp4_video_bridge::prepare_selected`, `owned_mp4_video_decode::decode_presented`, `owned_mp4_multitrack_export::qualify`, and `validate_video_packet_budget_with_geometry_filters_interval_and_limit` (`src/native_export.rs`), and clamps `Limits::metadata_bytes` and `Limits::packet_bytes` to `budget` in `NativeReader::with_hardware` (`src/playback/video/native.rs`). Seven integration tests in `mp4_metadata_admission` pass. See `batch-327-validation.json`.

## Batch 328 — Propagate controlled-memory ceilings (max_controlled_bytes) and budget-error qualification across MP4 and Matroska audio loudness, loudnorm, mix, merge, sample-domain concat, spatial AAC routing, and Matroska PCM metadata extraction

FVID-0335 prefixes clamped MP4 metadata budget refusals with `"controlled memory budget exceeded: "` across `owned_mp4_audio_export::open`, `owned_mp4_audio`, `owned_wave_metadata::mp4_source_info`, and `owned_aac_export_routing::router_for_decode`, and propagates `options.max_controlled_bytes` across `owned_aac_export_routing::router_for_decode`, `owned_audio_plan::matroska_descriptor_for_decode`, `extract_source_pcm_metadata_with_interval_packet_and_memory_limit` (`src/native_export/pcm_matroska.rs`), `validate_and_inspect_source_with_options` (`src/native_audio_mix.rs`), `segment_packet_count` and `validate_concat_segments_with_options` (`src/native_audio_mix/concat.rs`), and `native_loudnorm::try_plan`/`try_apply` (`src/native_loudnorm.rs`). Seven integration tests in `mp4_audio_metadata_admission` and twenty-one unit tests in `fvid-media` `admission_tests` pass. See `batch-328-validation.json`.

## Batch 329 — Propagate controlled-memory packet index ceilings (max_controlled_bytes / size_of::<Packet>()) across Matroska video, subtitle burn-in, audio export recognition, Y4M export/trim/concat, and WebM playback entrypoints

FVID-0336 clamps Matroska `Limits::packets` to `max_controlled_bytes / size_of::<Packet>()` across `owned_video_decode::decode_webm`, `owned_ffv1_export::input_limited`, `owned_lossless::check_source_matroska_metadata_budget`, `owned_webm_multitrack_export::open`, `owned_subtitle_burn::prepare`, `owned_matroska_{aac,alac,flac,mp3,opus,pcm}_export::recognizes`, `validate_video_packet_budget_with_geometry_filters_interval_and_limit` (`src/native_export.rs`), `extract_source_pcm_metadata_with_interval_packet_and_memory_limit` (`src/native_export/pcm_matroska.rs`), `decode_fvid_media_audio_to_destination` (`src/media.rs`), and `WebmVideoReader::open_with_hardware` (`src/playback/video/webm.rs`). Eight integration tests in `mp4_metadata_admission` and sixteen related tests pass. See `batch-329-validation.json`.

## Batch 330 — Propagate controlled-memory Matroska packet-index and metadata ceilings (max_controlled_bytes / size_of::<Packet>()) across Matroska identity remux, Matroska subtitle conversion, sample-domain audio concat, and multitrack MP4 export

FVID-0337 clamps Matroska `Limits::packets` to `max_controlled_bytes / size_of::<Packet>()` and maps clamped packet-count refusals to `"controlled memory budget exceeded: Matroska packet index limit"` across `owned_matroska_copy_impl::inspect_with_packet_and_memory_limits`, `owned_subtitle_impl::try_plan_with_controls` and `try_convert_matroska_inner`, `segment_packet_count` (`src/native_audio_mix/concat.rs`), and `owned_mp4_multitrack_export::try_export`, and forwards `options.max_controlled_bytes` to `extract_source_pcm_metadata_with_interval_packet_and_memory_limit` in `concat_audio_with_options`. Nine integration tests in `mp4_metadata_admission` and ten related tests pass. See `batch-330-validation.json`.

## Batch 331 — Enforce controlled-memory budget (max_controlled_bytes) against selected companion audio track metadata working sets and multitrack packet-schedule / TrackPlan allocations in owned_webm_multitrack_export, owned_mp4_multitrack_export, and owned_lossless

FVID-0338 adds `check_multitrack_matroska_metadata_budget` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`) accounting for all selected tracks' source and mutated tags, filters `stream_metadata_set`/`stream_metadata_delete` to selected tracks in `owned_webm_multitrack_export::qualify`, `owned_mp4_multitrack_export::qualify`/`plan_selection`, and `owned_lossless::plan_transcode_lossless`, and enforces `max_controlled_bytes` on the multitrack packet-schedule and `TrackPlan` allocations before `Vec::with_capacity` in `owned_webm_multitrack_export` and `owned_mp4_multitrack_export`. Four integration tests in `multitrack_lossless_budget_parity_regressions` and five related tests pass. See `batch-331-validation.json`.

## Batch 332 — Enforce controlled-memory budget (max_controlled_bytes) against SRT file and parsed cue working sets in owned_subtitle_burn::prepare, against FrameMetadata and decode-loop frame buffers in owned_mp4_video_decode::decode_presented, and against presentation reorder queues and duration indices in owned_webm_presentation and owned_webm_codec

FVID-0339 enforces `options.max_controlled_bytes` on the SRT file size (`subs_bytes * 3`) before `read_to_string` and on the parsed `SubtitleCue` working set in `owned_subtitle_burn::prepare` (`crates/fvid-media/src/owned_subtitle_burn.rs`), bounds `frames: Vec<FrameMetadata>` and each decoded `pixels` buffer inside the AVC/AV1/VP9/HEVC decode loops of `owned_mp4_video_decode::decode_presented` (`crates/fvid-media/src/owned_mp4_video_decode.rs`), and bounds `future`, `durations`, `visible`, and cumulative B-frame reorder queue (`self.queue`) frame bytes in `Presentation` (`crates/fvid-media/src/owned_webm_presentation.rs`) and `presentation_durations` (`crates/fvid-media/src/owned_webm_codec.rs`). Five integration tests in `subtitle_convert_burn_budget_parity_regressions` and nine in `mp4_metadata_admission` pass. See `batch-332-validation.json`.

## Batch 333 — Propagate controlled-memory MP4 metadata and Matroska packet-index ceilings across native_probe::probe_with_options, decode_audio_with_options, audio_trim_selection_with_options, trim_aac_with_options, audio_source_info_selected_with_interval_and_options, validate_video_selection_with_options, and y4m_segment_frame_upper_bound_with_options

FVID-0340 adds `probe_with_options` / `try_probe_as_with_options` (`src/media_probe.rs` and `crates/fvid-media/src/owned_mp4_probe_impl.rs`) clamping MP4 `Limits::metadata_bytes` to `options.max_controlled_bytes` and Matroska `Limits::packets` to `options.max_controlled_bytes / size_of::<Packet>()`, and propagates `options` across `decode_audio_with_options`, `validate_encoded_audio_packet_budget`, `audio_trim_selection_with_interval_and_options`, `aac_trim_selection_with_options`, `is_owned_audio_trim_source_with_interval_and_options`, `audio_source_info_selected_with_interval_and_options`, `aac_source_info_selected_with_options`, `is_aac_source_with_metadata_limit`, `validate_video_selection_with_options`, and `y4m_segment_frame_upper_bound_with_options`. Eight integration tests in `mp4_audio_metadata_admission` and nine in `mp4_metadata_admission` pass. See `batch-333-validation.json`.

## Batch 334 — Propagate controlled-memory ceilings (max_controlled_bytes) across validate_concat_segments_with_options trailing-segment geometry probes, native_audio_mix::eligible_with_options, mix_audio_with_controls, merge_audio_with_controls, and native_loudnorm Matroska metadata extraction and stream selection

FVID-0341 copies `options.max_controlled_bytes` into `default_probe` and calls `crate::native_probe::probe_with_options(source, &geom_probe)` in `validate_concat_segments_with_options` (`src/native_audio_mix/concat.rs`), uses `try_probe_as_with_options` / `probe_with_options` in `is_single_stream_fvid_media_audio_with_options`, `eligible_with_options` (`src/native_audio_mix.rs`), and `selected_audio` (`src/native_loudnorm.rs`), and forwards `max_controlled_bytes` to `extract_source_pcm_metadata_with_interval_packet_and_memory_limit` in `mix_audio_with_controls`, `merge_audio_with_controls`, and `native_loudnorm::try_apply`. Nine integration tests in `mp4_audio_metadata_admission` and nine in `mp4_metadata_admission` pass. See `batch-334-validation.json`.

## Batch 335 — Propagate controlled-memory Matroska packet-index and MP4 metadata ceilings across owned_webm_probe::probe_webm_with_packet_and_memory_limit, owned_remux_plan::plan_remux, native_plan::remux_with_options, and native_plan::concat_mp4_matroska_with_options

FVID-0342 adds `probe_webm_with_packet_and_memory_limit` and `try_probe_webm_with_packet_and_memory_limit` (`crates/fvid-media/src/owned_webm_probe.rs`) clamping `Limits::packets` to `max_controlled_bytes / size_of::<Packet>()` and mapping clamped packet-count refusals to `"controlled memory budget exceeded: Matroska packet index limit"`, and propagates `options` across `owned_remux_plan::plan_remux` (`crates/fvid-media/src/owned_remux_plan.rs`), `native_plan::remux_with_options`, and `native_plan::concat_mp4_matroska_with_options` (`src/native_plan.rs`). Nine integration tests in `mp4_metadata_admission` and nine in `mp4_audio_metadata_admission` pass. See `batch-335-validation.json`.

## Batch 336 — Propagate controlled-memory ceilings (max_controlled_bytes) across NativeReader::software in Y4M first-frame inspection, concat compatibility, and export/trim/concat execution, and across export_pcm_selected MP4 metadata and Matroska packet-index limits

FVID-0343 adds `inspect_first_y4m_frame_with_filters_and_options` and `validate_y4m_concat_compatibility_with_options` (`src/native_export.rs`), passes `options.max_controlled_bytes.unwrap_or(usize::MAX)` to `NativeReader::software` in `inspect_first_y4m_frame_with_filters_and_options` and `export_y4m_sources_with_options`, forwards `options` to `y4m_segment_frame_upper_bound_with_options` and `validate_y4m_concat_compatibility_with_options` (`src/native_plan.rs`), and propagates `options` and `max_controlled_bytes` in `export_audio_pcm_selected_with_controls` and `export_pcm_selected`. Ten integration tests in `mp4_metadata_admission` and nine in `mp4_audio_metadata_admission` pass. See `batch-336-validation.json`.

## Batch 337 — Propagate controlled-memory ceilings (max_controlled_bytes) to FFV1, AVC, HEVC, VP9, and AV1 codec decoders in owned_webm_codec::Decoder, owned_video_decode::decode_webm, and owned_mp4_video_decode::decode_presented, and to owned_probe::probe_with_options in plan_transcode_lossless and plan_burn_subtitles

FVID-0344 adds `Decoder::new_with_budget` in `crates/fvid-media/src/owned_webm_codec.rs`, passes `options.and_then(|o| o.max_controlled_bytes).unwrap_or(usize::MAX)` in `owned_video_decode::decode_webm`, `owned_webm_codec::presentation_durations`, and all four codec branches (`AvcDecoder`, `Av1Decoder`, `vp9_decoder::Decoder`, `HevcDecoder`) of `owned_mp4_video_decode::decode_presented`, and adds `owned_probe::probe_with_options` (`crates/fvid-media/src/owned_probe.rs`) used by `plan_transcode_lossless` and `plan_burn_subtitles`. Eleven integration tests in `mp4_metadata_admission` and nine in `mp4_audio_metadata_admission` pass. See `batch-337-validation.json`.

## Batch 338 — Clamp ADTS StreamReader packet limits to max_controlled_bytes across ADTS decode, loudness, planning, and spatial routing, emit budget notes in owned WAVE plan_loudness, and forward CopyOptions to G.711 WAVE recognition in plan_trim and trim

FVID-0345 clamps `StreamReader::open_with_packet_limit` to `options.max_packet_bytes.min(options.max_controlled_bytes.unwrap_or(usize::MAX))` across `owned_adts_loudness`, `owned_adts_export`, `owned_audio_plan`, `owned_aac_export_routing`, and `owned_aac::stream`, appends the standard budget note in `owned_wave_loudness::plan_loudness`, and forwards `options` to `owned_g711_wave_export::recognizes` in `owned_wave_plan::plan_trim` and `owned_wave_remux::trim`. Ten integration tests in `mp4_audio_metadata_admission` and eleven in `mp4_metadata_admission` pass. See `batch-338-validation.json`.

## Batch 339 — Forward CopyOptions packet, count, and memory ceilings through owned_probe::probe_with_options across WAVE, Y4M, FLAC, MPEG, Ogg, and ADTS probes and preflight frame and mask budgets in owned_subtitle_burn::prepare and owned_video_decode::decode_webm

FVID-0346 adds `probe_wave_with_options`, `probe_ogg_with_options`, and `probe_adts_with_options` in `owned_probe.rs`, `probe_flac_with_options` in `owned_flac_probe.rs`, `probe_mpeg_with_options` in `owned_mpeg_probe.rs`, and `probe_y4m_with_options` in `owned_y4m_probe.rs` and wires them into `owned_probe::probe_with_options`, and enforces frame/mask packet and memory budgets up front in `owned_subtitle_burn::prepare` and before `process_frame_with_overlay` in `owned_video_decode::decode_webm`. Twelve integration tests in `mp4_metadata_admission` pass. See `batch-339-validation.json`.

## Batch 340 — Forward CopyOptions across native_probe::try_probe_as_with_options, extract_source_pcm_metadata_with_interval_packet_and_memory_limit, and decode_source_to_f32le_with_options for WAVE, Y4M, FLAC, MPEG, Ogg, and ADTS sources

FVID-0347 forwards `options` through `native_probe::try_probe_as_with_options` (`src/media_probe.rs`) into `probe_wave_with_options`, `try_y4m_with_options`, `probe_flac_with_options`, `probe_mpeg_with_options`, `probe_ogg_with_options`, and `probe_adts_with_options`, uses `native_probe::probe_with_options` / `try_probe_as_with_options` in `extract_source_pcm_metadata_with_interval_packet_and_memory_limit` (`src/native_export/pcm_matroska.rs`), and propagates `max_controlled_bytes` in `decode_source_to_f32le_with_options` (`src/native_audio_mix.rs`). Eleven integration tests in `mp4_audio_metadata_admission` pass. See `batch-340-validation.json`.

## Batch 341 — Enforce three-frame working-set and pre-export packet/memory budgets in owned_xfade and clamp ADTS PCE bootstrap packet limits to max_packet_bytes and max_controlled_bytes in owned_concat, native_audio_mix::concat, and native_loudnorm

FVID-0348 checks `frame_len.saturating_mul(3) > limit` in `plan_xfade`, `write_y4m_ffv1_counted`, and before `export_atomic` in `export_y4m_ffv1_counted` (`crates/fvid-media/src/owned_xfade.rs`), opens ADTS `StreamReader::open_with_packet_limit` with `options.max_packet_bytes.min(options.max_controlled_bytes.unwrap_or(usize::MAX))` in `owned_concat::open_readers` (`crates/fvid-media/src/owned_concat.rs`) and `segment_packet_count` (`src/native_audio_mix/concat.rs`), and clamps the ADTS first-frame preflight to `options.max_controlled_bytes` while forwarding `max_packet_bytes`, `max_packets`, and `cancel` in `native_loudnorm` (`src/native_loudnorm.rs`). Twelve integration tests in `mp4_audio_metadata_admission` pass. See `batch-341-validation.json`.

## Batch 342 — Forward CopyOptions cancellation and packet-byte ceilings across WAVE loudnorm, loudness, and remux supports/recognizes checks, owned_loudnorm::plan_loudnorm WAVE probing, and trailing-segment geometry validation in validate_concat_segments_with_options

FVID-0349 adds `recognizes_standalone_audio_with_options` and `recognizes_with_options` (`crates/fvid-media/src/owned_container_loudness.rs`) and wires them into `owned_wave_loudness`, `owned_loudnorm`, `native_loudnorm`, and `native_audio_mix`, passes `options.cancel.as_ref()` to `owned_wave_inspect::inspect` in `owned_wave_loudness::supports`, `owned_loudnorm::supports`, `owned_loudnorm::supports_dual`, and `owned_wave_remux::supports`, calls `probe_wave_with_options(source, options)` in `owned_loudnorm::plan_loudnorm`, and copies `options.max_packet_bytes` into `default_probe` in `validate_concat_segments_with_options` (`src/native_audio_mix/concat.rs`). Thirteen integration tests in `mp4_audio_metadata_admission` pass. See `batch-342-validation.json`.

## Batch 343 — Forward CopyOptions cancellation, packet-byte, packet-count, and controlled-memory ceilings through owned_ffv1_export::supports_with_options and owned_webm_multitrack_export qualification

FVID-0350 adds `owned_ffv1_export::supports_with_options(source, transform, options)` (`crates/fvid-media/src/owned_ffv1_export.rs`) forwarding `options` to `input_limited`, `presentation_durations`, and `decode::decode_webm`, wires it through `owned_lossless`, `owned_mp4_multitrack_export`, and `owned_subtitle_burn`, and replaces unbudgeted `try_webm` calls in `owned_webm_multitrack_export::plan_selection`, `supports`, and `try_export` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`) with `decode_webm(..., Some(o))`. Thirteen integration tests in `mp4_metadata_admission` pass. See `batch-343-validation.json`.

## Batch 344 — Pre-check standalone ASS and SRT script lengths against max_controlled_bytes before reading in owned_subtitle_impl, check Matroska metadata working-set budget before export_atomic in owned_subtitle_burn::burn_subtitles, and expose fvid::media::probe_with_options

FVID-0351 pre-checks `std::fs::metadata(source)?.len()` against 64 MiB and `max_controlled_bytes` before reading in `try_convert_ass_inner` and `try_convert_srt_inner` (`crates/fvid-media/src/owned_subtitle_impl.rs`), checks `estimate_matroska_metadata_working_bytes` before `mkv::export_atomic` in `owned_subtitle_burn::burn_subtitles` (`crates/fvid-media/src/owned_subtitle_burn.rs`), and adds `probe_with_options` and `probe_as_with_options` in `src/media.rs`. Six integration tests in `subtitle_convert_burn_budget_parity_regressions` pass. See `batch-344-validation.json`.

## Batch 345 — Clamp ADTS PCE bootstrap packet limits to max_packet_bytes and max_controlled_bytes in native_plan::concat_adts_with_options and forward CopyOptions cancellation and packet-byte limits across WAVE decode/trim_pcm/mix/merge planning and MP4 segment_packet_count

FVID-0352 opens ADTS segments with `StreamReader::open_with_packet_limit(..., options.max_packet_bytes.min(options.max_controlled_bytes.unwrap_or(usize::MAX)))` in `native_plan::concat_adts_with_options` (`src/native_plan.rs`), passes `options.cancel.as_ref()` to `native_pcm::inspect` in `decode_audio_with_options` and `trim_pcm_with_options`, calls `decode_audio_with_options` in `validate_and_inspect_source_with_options` (`src/native_audio_mix.rs`), and clamps `Limits::packet_bytes` in `segment_packet_count` (`src/native_audio_mix/concat.rs`). Fourteen integration tests in `mp4_audio_metadata_admission` pass. See `batch-345-validation.json`.

## Batch 346 — Clamp ADTS PCE bootstrap packet limits in native_export::concat_adts_aac_with_options and forward CopyOptions across trim_audio_pcm_with_controls, trim_y4m_with_options, and WAVE concat_info_with_cancel

FVID-0353 opens ADTS segments with `StreamReader::open_with_packet_limit(..., options.max_packet_bytes.min(options.max_controlled_bytes.unwrap_or(usize::MAX)))` in `native_export::concat_adts_aac_with_options` (`src/native_export.rs`), calls `aac_trim_selection_with_options` in `trim_audio_pcm_with_controls` and `validate_video_selection_with_options` in `trim_y4m_with_options`, and adds `native_pcm::concat_info_with_cancel` (`src/native_pcm.rs`) and `native_plan::concat_wave_with_options` (`src/native_plan.rs`) wired into `media::plan_concat` and `try_concat_mp4_matroska_with_options`. Fifteen integration tests in `mp4_audio_metadata_admission` pass. See `batch-346-validation.json`.

## Batch 347 — Enforce per-packet controlled memory budgets and pre-cancellation across ADTS and Matroska remux and concat planning and execution in fvid-media

FVID-0354 enforces `base_bytes + packets * 32 + meta_bytes > limit` inside the packet loops of `owned_adts_remux::plan_remux` (`crates/fvid-media/src/owned_adts_remux.rs`), `owned_concat::plan_concat` (`crates/fvid-media/src/owned_concat.rs`), and `write_aac_packets` (`crates/fvid-media/src/owned_matroska_adts_impl.rs`), and checks `options.cancel` in `owned_adts_remux::supports`, `owned_matroska_remux::supports`, `owned_remux::remux`, `owned_remux_plan::plan_remux`, `owned_adts_remux::{remux,plan_remux}`, `owned_matroska_remux::remux`, and `owned_concat::{concat,plan_concat}`. Fourteen integration tests in `mp4_metadata_admission` pass. See `batch-347-validation.json`.

## Batch 348 — Enforce per-packet byte limits in probe_ogg_with_options, max_packets in FLAC, MPEG, and Matroska probes, and CopyOptions across format-hinted probe_as_with_options and WAVE audio export admission

FVID-0355 checks individual Ogg packet lengths against `options.max_packet_bytes` in `probe_ogg_with_options` (`crates/fvid-media/src/owned_probe.rs`), enforces `options.max_packets` in `probe_flac_with_options`, `probe_mpeg_with_options`, `probe_webm_with_options`, and `native_probe::matroska_with_options`, enforces `max_packet_bytes`, `max_packets == Some(0)`, and `cancel` in `try_mp4_with_options`, adds `owned_probe::probe_as_with_options` wired into `fvid::media::probe_as_with_options`, and forwards `options.cancel` in `owned_audio_export::{supports,decode_audio_transformed}`. Fifteen integration tests in `mp4_metadata_admission` pass. See `batch-348-validation.json`.

## Batch 349 — Enforce CopyOptions budgets and cancellation across owned_audio_mix planning and execution, media mix/merge fallbacks, and native_loudnorm container plan fallback

FVID-0356 adds `mix_audio_with_controls`, `plan_mix_audio_with_controls`, `merge_audio_with_controls`, and `plan_merge_audio_with_controls` to `crates/fvid-media/src/owned_audio_mix.rs` (re-exported in `crates/fvid-media/src/lib.rs`), wires `src/media.rs` mix/merge fallbacks and `validate_owned_audio_options_for_destination` to enforce `controls`, and preserves `cancel`, `max_controlled_bytes`, `max_packet_bytes`, and `max_packets` in `native_loudnorm::try_plan` fallback `default_opts`. Sixteen integration tests in `mp4_audio_metadata_admission` pass. See `batch-349-validation.json`.

## Batch 350 — Clamp ADTS PCE bootstrap packet limits to max_controlled_bytes in native_media::aac_source_info_selected_with_options and inspect_adts_with_interval_and_options, and enforce CopyOptions budgets and cancellation across native_plan WAVE trim/concat and Matroska remux planning

FVID-0357 clamps ADTS `open_with_packet_limit` to `options.max_packet_bytes.min(options.max_controlled_bytes.unwrap_or(usize::MAX))` in `aac_source_info_selected_with_options` and `inspect_adts_with_interval_and_options` (`src/native_media.rs`), checks `cancel`, `max_packet_bytes == 0`, and `check_rss_budget` in `inspect_adts_with_interval_and_options`, enforces `CopyOptions` budgets and delegates custom budget/metadata plans in `native_plan::{trim_pcm_with_options,concat_wave_with_options}` (`src/native_plan.rs`), calls `matroska_with_options` in `native_plan::remux_with_options`, and checks `options.cancel` and `options.max_packet_bytes` in `uses_native_audio_route` (`src/media.rs`). Seventeen integration tests in `mp4_audio_metadata_admission` pass. See `batch-350-validation.json`.

## Batch 351 — Propagate Matroska scan_all errors and enforce WAVE metadata, RSS, cancellation, and decoded PCM budgets across owned_wave_metadata, owned_matroska_audio, owned_mp4_audio_export, and owned_raw_flac

FVID-0358 propagates `reader.scan_all()` errors in `matroska_source_info_with_interval` (`crates/fvid-media/src/owned_wave_metadata.rs`), checks `options.cancel`, `check_rss_budget(options)`, and `info.len() > max_controlled_bytes` in `owned_wave_metadata::{mp4_source_info,matroska_source_info_with_interval}`, checks `options.cancel` and `check_rss_budget(options)` in `owned_matroska_audio::open_audio_reader` (`crates/fvid-media/src/owned_matroska_audio.rs`) and `owned_mp4_audio_export::open` (`crates/fvid-media/src/owned_mp4_audio_export.rs`), and enforces `parse_packet_bytes` and per-frame decoded float PCM buffer admission against `options.max_controlled_bytes` in `owned_raw_flac` (`crates/fvid-media/src/owned_raw_flac.rs`). Eighteen integration tests in `mp4_audio_metadata_admission` pass. See `batch-351-validation.json`.

## Batch 352 — Enforce max_packets and CopyOptions budgets in Matroska remux planning/execution, native_plan ADTS remux packet scanning, and native_export WAVE LIST INFO metadata construction

FVID-0359 validates `options.max_packets == Some(0)` (`"packet work limit requires at least one packet"`) in `owned_remux_plan::plan_remux`, `owned_matroska_remux::remux`, `owned_remux::remux`, and `native_plan::remux_with_options`, calls `probe_webm_with_options(source, options)` in `owned_remux_plan::plan_remux`, bounds the ADTS packet scan loop in `native_plan::remux_with_options` (`src/native_plan.rs`) by `options.max_packets` and `check_rss_budget(options)`, and enforces `opts.max_controlled_bytes` before and after `edit_info_chunks` in `native_export::export_pcm_selected` (`src/native_export.rs`). Nineteen integration tests in `mp4_audio_metadata_admission` pass. See `batch-352-validation.json`.

## Batch 353 — Enforce decoded float PCM and WAVE metadata budgets in owned_raw_mp3, owned_raw_flac, and owned_ogg_opus_export, and validate zero-limit CopyOptions and CheckedRead in probe_mpeg_with_options and probe_ogg_with_options

FVID-0360 charges per-frame decoded float PCM buffer bytes against `options.max_controlled_bytes` in `owned_raw_mp3::decode_raw_checked` (`crates/fvid-media/src/owned_raw_mp3.rs`), enforces `info.len() <= max` in `source_info` across `owned_raw_mp3`, `owned_raw_flac`, and `owned_ogg_opus_export`, validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget(options)` in `probe_mpeg_with_options` (`crates/fvid-media/src/owned_mpeg_probe.rs`) and `probe_ogg_with_options` (`crates/fvid-media/src/owned_probe.rs`), and reads Ogg payload bytes via `CheckedRead` in `probe_ogg_with_options`. Twenty integration tests in `mp4_audio_metadata_admission` pass. See `batch-353-validation.json`.

## Batch 354 — Enforce zero-limit CopyOptions and RSS budgets across FLAC, ADTS, WAVE, WebM/Matroska, MP4, and Y4M probe entrypoints

FVID-0361 validates `options.cancel`, `options.max_packet_bytes == 0`, `options.max_packets == Some(0)`, `options.max_controlled_bytes == Some(0)`, and `check_rss_budget(options)` at admission across `probe_as_with_options`, `probe_adts_with_options`, and `probe_wave_with_options` (`crates/fvid-media/src/owned_probe.rs`), `probe_flac_with_options` (`crates/fvid-media/src/owned_flac_probe.rs`), `probe_webm_with_options` and `try_probe_webm_with_limits` (`crates/fvid-media/src/owned_webm_probe.rs`), `try_mp4_with_options` (`crates/fvid-media/src/owned_mp4_probe_impl.rs`), `probe_y4m_with_options` (`crates/fvid-media/src/owned_y4m_probe.rs`), and `matroska_with_options` / `try_probe_as_with_options` (`src/media_probe.rs`), and enforces `max_packet_bytes >= bytes_per_sample_frame` in `probe_wave_with_options`. Twenty-one integration tests in `mp4_audio_metadata_admission` pass. See `batch-354-validation.json`.

## Batch 355 — Enforce zero-limit CopyOptions in WAVE, ADTS, and MP4 remux/trim/concat and WAVE metadata budgets in Ogg Vorbis/joint and G.711 WAVE exports

FVID-0362 validates `options.max_packet_bytes == 0`, `options.max_packets == Some(0)`, and `options.max_controlled_bytes == Some(0)` at admission in `owned_wave_remux::copy_waves` (`crates/fvid-media/src/owned_wave_remux.rs`), `owned_adts_remux::{remux, plan_remux}` (`crates/fvid-media/src/owned_adts_remux.rs`), and `owned_mp4_remux::{remux, plan_remux}` (`crates/fvid-media/src/owned_mp4_remux.rs`), and enforces `info.len() <= max` in `owned_ogg_vorbis_export::open_with_transform` (`crates/fvid-media/src/owned_ogg_vorbis_export.rs`), `owned_ogg_joint_export::open_with_transform` (`crates/fvid-media/src/owned_ogg_joint_export.rs`), and `owned_g711_wave_export::open` (`crates/fvid-media/src/owned_g711_wave_export.rs`). Twenty-two integration tests in `mp4_audio_metadata_admission` pass. See `batch-355-validation.json`.

## Batch 356 — Enforce zero-limit CopyOptions in loudness, loudnorm, and audio mix/merge/concat entrypoints and WAVE metadata budgets in native_audio_mix

FVID-0363 validates `max_packet_bytes == 0`, `max_packets == Some(0)`, and `max_controlled_bytes == Some(0)` at admission across `owned_wave_loudness::preflight` (`crates/fvid-media/src/owned_wave_loudness.rs`), `owned_loudnorm::{plan_loudnorm, apply_resolved, apply_dynamic}` (`crates/fvid-media/src/owned_loudnorm.rs`), `owned_adts_loudness::validate_header_configuration` (`crates/fvid-media/src/owned_adts_loudness.rs`), `owned_adts_loudnorm::{apply, plan_loudnorm}` (`crates/fvid-media/src/owned_adts_loudnorm.rs`), `owned_container_loudness::{geometry_unqualified, decode_to_wave}` (`crates/fvid-media/src/owned_container_loudness.rs`), `owned_audio_mix::validate_mix_controls` (`crates/fvid-media/src/owned_audio_mix.rs`), `native_loudnorm::{try_plan, try_apply}` (`src/native_loudnorm.rs`), and `native_audio_mix` (`src/native_audio_mix.rs` and `src/native_audio_mix/concat.rs`), and enforces `edited.len() <= max` after `edit_info_chunks` in `native_audio_mix::{mix_audio_with_controls, merge_audio_with_controls, concat_audio_with_options}`. Twenty-three integration tests in `mp4_audio_metadata_admission` pass. See `batch-356-validation.json`.

## Batch 357 — Enforce zero-limit CopyOptions across lossless transcode/plan, xfade, subtitle burn/convert, Y4M export/concat/trim, and native_plan entrypoints

FVID-0364 validates `max_packet_bytes == 0`, `max_packets == Some(0)`, and `max_controlled_bytes == Some(0)` at admission across `owned_lossless::{policy, transcode_lossless, plan_transcode_lossless}` (`crates/fvid-media/src/owned_lossless.rs`), `owned_xfade::{plan_xfade, xfade_video, write_y4m_ffv1_counted, export_y4m_ffv1_counted}` (`crates/fvid-media/src/owned_xfade.rs`), `owned_subtitle_burn::{policy, supports}` (`crates/fvid-media/src/owned_subtitle_burn.rs`), `owned_subtitle_impl::{try_plan_with_controls, try_convert_with_controls}` (`crates/fvid-media/src/owned_subtitle_impl.rs`), `native_export::{validate_y4m_options, export_y4m_sources_with_options}` (`src/native_export.rs`), `native_plan::{decode_audio_with_options, trim_adts_with_options, loudness_with_options, concat_mp4_matroska_with_options}` (`src/native_plan.rs`), and `validate_concat_matroska_options` (`src/media.rs`). Twenty-four integration tests in `mp4_audio_metadata_admission` pass. See `batch-357-validation.json`.

## Batch 358 — Enforce zero-limit CopyOptions across audio export/plan, source_geometry, and ADTS/owned concat entrypoints

FVID-0365 validates `options.cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget(options)` at admission across `owned_audio_export::{validate_request, decode_audio_transformed}` (`crates/fvid-media/src/owned_audio_export.rs`), `owned_audio_plan::source_descriptor_for_decode` (`crates/fvid-media/src/owned_audio_plan.rs`), `owned_concat::{concat, plan_concat}` (`crates/fvid-media/src/owned_concat.rs`), `native_export::{remux_adts_aac_with_options, concat_adts_aac_with_options}` (`src/native_export.rs`), and `decode_fvid_media_audio_to_destination` (`src/media.rs`). Twenty-five integration tests in `mp4_audio_metadata_admission` pass. See `batch-358-validation.json`.

## Batch 359 — Enforce zero-limit CopyOptions across ADTS, MP4, Matroska, raw FLAC, and raw MPEG PCM stream decoders

FVID-0366 validates `max_packet_bytes == 0`, `max_packets == Some(0)`, and `max_controlled_bytes == Some(0)` at admission in `owned_matroska_audio::DecodeProgress::check` (`crates/fvid-media/src/owned_matroska_audio.rs`), `owned_aac::stream::DecodeProgress::check` (`crates/fvid-media/src/owned_aac/stream.rs`), `owned_raw_flac::check` (`crates/fvid-media/src/owned_raw_flac.rs`), and `owned_raw_mp3::check` (`crates/fvid-media/src/owned_raw_mp3.rs`). Twenty-six integration tests in `mp4_audio_metadata_admission` pass. See `batch-359-validation.json`.

## Batch 360 — Enforce zero-limit CopyOptions across native_pcm loudness/normalization and native_media audio inspection entrypoints

FVID-0367 validates `max_packet_bytes == 0`, `max_packets == Some(0)`, and `max_controlled_bytes == Some(0)` at admission across `native_pcm::loudness::{measure_file_with_options, default_weights_with_options}` (`src/native_pcm/loudness.rs`), `native_pcm::normalize::normalize_file_with_options` (`src/native_pcm/normalize.rs`), and `native_media::{aac_source_info_selected_with_options, inspect_adts_with_interval_and_options, audio_source_info_selected_with_interval_and_options}` (`src/native_media.rs`). Twenty-seven integration tests in `mp4_audio_metadata_admission` pass. See `batch-360-validation.json`.

## Batch 361 — Enforce zero-limit CopyOptions across native audio trim/plan/export, mix/merge eligibility, and source admission entrypoints

FVID-0368 validates `max_packet_bytes == 0`, `max_packets == Some(0)`, and `max_controlled_bytes == Some(0)` at admission across `native_media::{is_aac_source_with_metadata_limit, is_owned_audio_source_with_interval_and_options, is_aac_trim_source_with_interval_and_options}` (`src/native_media.rs`), `native_plan::{aac_trim_selection_with_options, audio_trim_selection_with_interval_and_options}` (`src/native_plan.rs`), `native_export::{export_pcm_selected, remux_mp4_stats_with_memory_limit, remux_matroska_with_packet_and_memory_limits}` (`src/native_export.rs`), `native_audio_mix::eligible_with_options` (`src/native_audio_mix.rs`), and `validate_owned_audio_options_for_destination` (`src/media.rs`). Twenty-eight integration tests in `mp4_audio_metadata_admission` pass. See `batch-361-validation.json`.

## Batch 362 — Validate CopyOptions before unbounded source I/O in public overlay, lossless transcode, audio route admission, and ADTS/WAVE concat planning

FVID-0369 validates `validate_native_copy_options(options, false)` before `overlay_eligible` and `supports_deband_source` in `plan_overlay`, `overlay_video`, `transcode_lossless`, `transcode`, and `plan_transcode_lossless`, and checks `cancel`, zero limits, and RSS budgets at the start of `uses_native_audio_route`, `validate_adts_copy_options_with_metadata`, `validate_concat_matroska_options`, and WAVE `plan_concat` in `src/media.rs`. Twenty-nine integration tests in `mp4_audio_metadata_admission` pass. See `batch-362-validation.json`.

## Batch 363 — Enforce zero-limit and RSS CopyOptions in owned lossless supports, FFV1 export supports_with_options, MP4 video bridge/decode, and plan_transcode_lossless fallback

FVID-0370 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget` at the start of `owned_lossless::supports` (`crates/fvid-media/src/owned_lossless.rs`), `owned_ffv1_export::supports_with_options` (`crates/fvid-media/src/owned_ffv1_export.rs`), `owned_mp4_video_bridge::prepare_selected` (`crates/fvid-media/src/owned_mp4_video_bridge.rs`), and `owned_mp4_video_decode::decode_presented` (`crates/fvid-media/src/owned_mp4_video_decode.rs`), and checks `check_rss_budget(options)` before `supports` while preserving budgets and cancellation in `owned_lossless::plan_transcode_lossless`. Thirty integration tests in `mp4_audio_metadata_admission` pass. See `batch-363-validation.json`.

## Batch 364 — Enforce zero-limit and RSS CopyOptions at admission in owned remux/plan dispatch and WAVE remux/trim/concat before policy or container probing

FVID-0371 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget(options)` at the start of `owned_remux::remux` (`crates/fvid-media/src/owned_remux.rs`), `owned_remux_plan::plan_remux` (`crates/fvid-media/src/owned_remux_plan.rs`), `owned_wave_remux::{policies, trim, concat, inspect_copy, copy_waves}` (`crates/fvid-media/src/owned_wave_remux.rs`), `owned_wave_plan::{plan_trim, plan_concat}` (`crates/fvid-media/src/owned_wave_plan.rs`), `owned_mp4_remux::policies` (`crates/fvid-media/src/owned_mp4_remux.rs`), and `owned_adts_remux::adts_matroska_policies` (`crates/fvid-media/src/owned_adts_remux.rs`). Thirty-one integration tests in `mp4_audio_metadata_admission` pass. See `batch-364-validation.json`.

## Batch 365 — Enforce CopyOptions cancellation, zero limits, and RSS budgets before policy/format/file checks across owned loudness, loudnorm, audio export supports, and Y4M/Ogg probing

FVID-0372 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget(options)` before policy, signature, or file checks across `owned_loudnorm` (`crates/fvid-media/src/owned_loudnorm.rs`), `owned_wave_loudness` (`crates/fvid-media/src/owned_wave_loudness.rs`), `owned_container_loudness` (`crates/fvid-media/src/owned_container_loudness.rs`), `owned_container_loudnorm` (`crates/fvid-media/src/owned_container_loudnorm.rs`), `owned_adts_loudness` (`crates/fvid-media/src/owned_adts_loudness.rs`), `owned_adts_loudnorm` (`crates/fvid-media/src/owned_adts_loudnorm.rs`), `owned_audio_export::supports` (`crates/fvid-media/src/owned_audio_export.rs`), `owned_probe::probe_ogg_with_options` (`crates/fvid-media/src/owned_probe.rs`), and `owned_y4m_probe::try_y4m_with_options` (`crates/fvid-media/src/owned_y4m_probe.rs`). Thirty-two integration tests in `mp4_audio_metadata_admission` pass. See `batch-365-validation.json`.

## Batch 366 — Enforce CopyOptions cancellation, zero limits, and RSS budgets before input-count, weight, extension, or argument checks in audio mix/merge/concat, native loudnorm, xfade, subtitle burn, and multitrack export supports

FVID-0373 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget` first across `owned_audio_mix` (`crates/fvid-media/src/owned_audio_mix.rs`), `native_audio_mix` (`src/native_audio_mix.rs`), `native_audio_mix::concat` (`src/native_audio_mix/concat.rs`), `native_loudnorm` (`src/native_loudnorm.rs`), `owned_xfade` (`crates/fvid-media/src/owned_xfade.rs`), `owned_subtitle_burn::supports` (`crates/fvid-media/src/owned_subtitle_burn.rs`), `owned_mp4_multitrack_export` (`crates/fvid-media/src/owned_mp4_multitrack_export.rs`), and `owned_webm_multitrack_export` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`). Thirty-three integration tests in `mp4_audio_metadata_admission` pass. See `batch-366-validation.json`.

## Batch 367 — Enforce CopyOptions cancellation, zero limits, and RSS budgets before stream/metadata/interval/destination checks across raw PCM stream decoders and container audio export backends

FVID-0374 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget` first across `owned_aac::stream::decode_adts_pcm` (`crates/fvid-media/src/owned_aac/stream.rs`), `owned_raw_flac` (`crates/fvid-media/src/owned_raw_flac.rs`), `owned_raw_mp3` (`crates/fvid-media/src/owned_raw_mp3.rs`), `owned_mp4_audio` (`crates/fvid-media/src/owned_mp4_audio.rs`), `owned_mp4_audio_export` (`crates/fvid-media/src/owned_mp4_audio_export.rs`), `owned_matroska_audio` (`crates/fvid-media/src/owned_matroska_audio.rs`), `owned_matroska_pcm` (`crates/fvid-media/src/owned_matroska_pcm.rs`), `owned_matroska_{pcm,aac,opus,flac,mp3,alac}_export`, `owned_ogg_{opus,vorbis,joint}_export`, `owned_g711_wave_export` (`crates/fvid-media/src/owned_g711_wave_export.rs`), and `owned_adts_export` (`crates/fvid-media/src/owned_adts_export.rs`). Thirty-four integration tests in `mp4_audio_metadata_admission` pass. See `batch-367-validation.json`.

## Batch 368 — Enforce CopyOptions cancellation, zero limits, and RSS budgets before extension, input-count, interval, or metadata/stream checks across native_export, native_plan, media, and video decode/remux

FVID-0375 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget` first across `native_export` (`src/native_export.rs`), `native_plan` (`src/native_plan.rs`), `container::mp4_concat::open_with_options` (`src/container/mp4_concat.rs`), `media` (`src/media.rs`), `owned_lossless::transcode_lossless` (`crates/fvid-media/src/owned_lossless.rs`), `owned_video_decode::decode_webm` (`crates/fvid-media/src/owned_video_decode.rs`), and `owned_matroska_remux::publish` (`crates/fvid-media/src/owned_matroska_remux.rs`). Thirty-five integration tests in `mp4_audio_metadata_admission` pass. See `batch-368-validation.json`.

## Batch 369 — Enforce CopyOptions cancellation, zero limits, and RSS budgets first across public media entrypoints, Matroska/WebM probes, owned lossless transcode, subtitle conversion, and owned concat

FVID-0376 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget` first across all public `media` entrypoints (`src/media.rs`), `media_probe::matroska_with_options` (`src/media_probe.rs`), `owned_webm_probe::probe_webm_with_options` (`crates/fvid-media/src/owned_webm_probe.rs`), `owned_lossless::transcode` (`crates/fvid-media/src/owned_lossless.rs`), `owned_subtitle_impl::{try_plan_with_controls, try_convert_with_controls}` (`crates/fvid-media/src/owned_subtitle_impl.rs`), and `owned_concat::{concat, plan_concat}` (`crates/fvid-media/src/owned_concat.rs`). Thirty-six integration tests in `mp4_audio_metadata_admission` pass. See `batch-369-validation.json`.

## Batch 370 — Enforce CopyOptions cancellation, zero limits, and RSS budgets first across native_pcm, native_media, owned ADTS/Matroska remux, MP4 video decode/bridge, FFV1 export, subtitle burn, and xfade

FVID-0377 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget` first across `native_pcm` (`src/native_pcm.rs`, `src/native_pcm/loudness.rs`, `src/native_pcm/normalize.rs`), `native_media` (`src/native_media.rs`), `owned_adts_remux::plan_remux` (`crates/fvid-media/src/owned_adts_remux.rs`), `owned_matroska_remux::remux` (`crates/fvid-media/src/owned_matroska_remux.rs`), `owned_matroska_copy_impl` (`crates/fvid-media/src/owned_matroska_copy_impl.rs`), `owned_mp4_video_decode` (`crates/fvid-media/src/owned_mp4_video_decode.rs`), `owned_mp4_video_bridge` (`crates/fvid-media/src/owned_mp4_video_bridge.rs`), `owned_ffv1_export` (`crates/fvid-media/src/owned_ffv1_export.rs`), `owned_subtitle_burn` (`crates/fvid-media/src/owned_subtitle_burn.rs`), and `owned_xfade` (`crates/fvid-media/src/owned_xfade.rs`). Thirty-seven integration tests in `mp4_audio_metadata_admission` pass. See `batch-370-validation.json`.

## Batch 371 — Enforce canonical CopyOptions cancellation, zero limits, and RSS budgets across overlay_video_transformed, export_pcm_selected, mp4_concat, owned_wave_loudness, WebM presentation/codec loops, and ADTS plans

FVID-0378 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget` in canonical order at entry and in packet loops across `overlay_video_transformed` (`src/native_export/overlay.rs`), `export_y4m_sources_with_options` and `export_pcm_selected` (`src/native_export.rs`), `mp4_concat::{open_with_options, write_with_options}` (`src/container/mp4_concat.rs`), `owned_wave_loudness::check_options` (`crates/fvid-media/src/owned_wave_loudness.rs`), `owned_webm_presentation::Presentation::next` (`crates/fvid-media/src/owned_webm_presentation.rs`), `owned_webm_codec::presentation_durations` (`crates/fvid-media/src/owned_webm_codec.rs`), and `native_plan::{remux_adts_with_options, concat_adts_with_options}` (`src/native_plan.rs`). Thirty-eight integration tests in `mp4_audio_metadata_admission` pass. See `batch-371-validation.json`.

## Batch 372 — Enforce canonical CopyOptions order and per-packet RSS/cancellation checks across ADTS remux/concat, MP4 relocate/Matroska copy options, WAVE metadata, AAC routing, Ogg Opus decode, PS negotiation, and probes

FVID-0379 validates `cancel`, `max_packet_bytes == 0`, `max_packets == Some(0)`, `max_controlled_bytes == Some(0)`, and `check_rss_budget` in canonical order and enforces per-iteration cancellation and RSS checks across `native_export::{remux_adts_aac_with_options, concat_adts_aac_with_options, concat_y4m_with_options}` (`src/native_export.rs`), `media::{validate_matroska_copy_options, validate_mp4_relocate_options, validate_mp4_packet_budget}` (`src/media.rs`), `owned_wave_metadata::mp4_source_info` (`crates/fvid-media/src/owned_wave_metadata.rs`), `owned_aac_export_routing::{router_for_decode, check, export}` (`crates/fvid-media/src/owned_aac_export_routing.rs`), `owned_ogg_opus_export::{preflight, decode}` (`crates/fvid-media/src/owned_ogg_opus_export.rs`), `owned_matroska_aac_export::geometry_for_decode` (`crates/fvid-media/src/owned_matroska_aac_export.rs`), `owned_mp4_audio_export::descriptor_for_decode` (`crates/fvid-media/src/owned_mp4_audio_export.rs`), `owned_probe::{probe_adts_with_options, probe_wave_with_options}` (`crates/fvid-media/src/owned_probe.rs`), `owned_y4m_probe::probe_y4m_with_options` (`crates/fvid-media/src/owned_y4m_probe.rs`), `owned_webm_probe::try_probe_webm_with_limits` (`crates/fvid-media/src/owned_webm_probe.rs`), and `media_probe::matroska_with_limits` (`src/media_probe.rs`). Thirty-nine integration tests in `mp4_audio_metadata_admission` pass. See `batch-372-validation.json`.

## Batch 373 — Decouple compressed/G.711 packet byte limits from float32 WAVE spool reads and enforce per-iteration RSS/cancellation across audio mix, merge, concat, and loudnorm routing

FVID-0380 keeps `spool_opts.max_packet_bytes` at its default when reading the decoded float32 WAVE spool in `native_audio_mix::decode_source_to_f32le_with_options` (`src/native_audio_mix.rs`), enforces canonical `CopyOptions` checks at entry of `decode_source_to_f32le_with_options`, `geometry_with_options`, and `segment_packet_count` (`src/native_audio_mix/concat.rs`), propagates `max_rss_bytes` across `native_loudnorm` (`src/native_loudnorm.rs`), `media` (`src/media.rs`), and `native_export` (`src/native_export.rs`), and enforces per-iteration cancellation/RSS checks and fallible output buffer reservations across `native_audio_mix`, `concat`, `native_loudnorm`, and `owned_audio_mix` (`crates/fvid-media/src/owned_audio_mix.rs`). Forty integration tests in `mp4_audio_metadata_admission` pass. See `batch-373-validation.json`.

## Batch 374 — Enforce target normalization memory and packet budgets in MP4/Matroska audio admission, supports, and Ogg Vorbis permutation

FVID-0381 validates canonical `CopyOptions` and active-sample packet budgets before `max_controlled_bytes` early return in `owned_mp4_audio::admit_audio_reader_with_interval` (`crates/fvid-media/src/owned_mp4_audio.rs`), enforces target normalization memory admission and controlled PS geometry probing in `owned_mp4_audio_export::normalization_target` (`crates/fvid-media/src/owned_mp4_audio_export.rs`), preserves `controlled memory budget exceeded:` in `owned_mp4_audio_export::supports` and `owned_matroska_{flac,alac,pcm,aac}_export::supports` (`crates/fvid-media/src/owned_matroska_{flac,alac,pcm,aac}_export.rs`), and enforces canonical entry and per-iteration cancellation/RSS checks in `owned_matroska_audio` (`crates/fvid-media/src/owned_matroska_audio.rs`) and `owned_ogg_vorbis_export::write_pcm` (`crates/fvid-media/src/owned_ogg_vorbis_export.rs`). Forty-one integration tests in `mp4_audio_metadata_admission` pass. See `batch-374-validation.json`.

## Batch 375 — Enforce active-frame decoded PCM memory budgets in raw FLAC and MP3 open/plan admission and CopyOptions controls across standalone loudness, WAVE Matroska metadata, and AAC routing

FVID-0382 charges active-prefix decoded float32 PCM frame bytes alongside `file_bytes + decode_admission_bytes + index_bytes` in `owned_raw_flac::open_with_interval` (`crates/fvid-media/src/owned_raw_flac.rs`) and `owned_raw_mp3::open_with_interval` (`crates/fvid-media/src/owned_raw_mp3.rs`), enforces canonical `CopyOptions` checks at entry of `owned_container_loudness::{recognizes_standalone_audio_with_options, recognizes_with_options}` (`crates/fvid-media/src/owned_container_loudness.rs`) and `owned_wave_metadata::matroska_source_info_with_interval` (`crates/fvid-media/src/owned_wave_metadata.rs`), passes cancellation/RSS checks in `owned_aac_export_routing::router_for_decode` (`crates/fvid-media/src/owned_aac_export_routing.rs`), and uses `router_for_decode(..., Some(transform), &probe)` in `owned_adts_export::supports` (`crates/fvid-media/src/owned_adts_export.rs`). Forty-two integration tests in `mp4_audio_metadata_admission` pass. See `batch-375-validation.json`.

## Batch 376 — Enforce Matroska PCM codec verification before budget admission in decode_matroska_pcm_f64 and canonical audio stream selection across Matroska export geometry and descriptors

FVID-0383 validates `A_PCM/INT/LIT`, `A_PCM/INT/BIG`, or `A_PCM/FLOAT/IEEE` before `admit_audio_reader_with_interval` in `owned_matroska_pcm::decode_matroska_pcm_f64` (`crates/fvid-media/src/owned_matroska_pcm.rs`), enforces `options.streams.len() <= 1` in `owned_audio_plan::matroska_descriptor_for_decode` (`crates/fvid-media/src/owned_audio_plan.rs`) and `owned_matroska_flac_export::channel_mask` (`crates/fvid-media/src/owned_matroska_flac_export.rs`), and routes all six `owned_matroska_{opus,mp3,flac,alac,pcm,aac}_export` geometry helpers through `matroska_audio_index` with explicit codec verification. Forty-three integration tests in `mp4_audio_metadata_admission` pass. See `batch-376-validation.json`.

## Batch 377 — Enforce combined Ogg WAVE metadata admission and non-empty interval prefix validation across Ogg Opus/Vorbis/Joint, raw FLAC, and raw MP3 open and plan routes

FVID-0384 validates `source_info` and charges `info.len()` in the combined `admit(...)` budget across `owned_ogg_{opus,vorbis,joint}_export::open_with_transform` (`crates/fvid-media/src/owned_ogg_opus_export.rs`, `crates/fvid-media/src/owned_ogg_vorbis_export.rs`, `crates/fvid-media/src/owned_ogg_joint_export.rs`), and enforces `check_nonempty_interval_prefix` across `owned_ogg_{opus,vorbis,joint}_export::open_with_transform`, `owned_raw_flac::open_with_interval` (`crates/fvid-media/src/owned_raw_flac.rs`), and `owned_raw_mp3::open_with_interval` (`crates/fvid-media/src/owned_raw_mp3.rs`) so out-of-range `transform.interval` windows are rejected with `no decoded audio samples` during planning and support checks. Forty-four integration tests in `mp4_audio_metadata_admission` pass. See `batch-377-validation.json`.

## Batch 378 — Enforce non-empty interval prefix validation and source WAVE metadata budget admission across MP4, Matroska, and ADTS audio decode planning and support routes

FVID-0385 validates source WAVE `LIST INFO` metadata admission and `check_nonempty_interval_prefix` across `owned_audio_plan::source_descriptor_for_decode` (`crates/fvid-media/src/owned_audio_plan.rs`), `owned_mp4_audio_export::{descriptor_for_decode, geometry_for_decode}` (`crates/fvid-media/src/owned_mp4_audio_export.rs`), all six `owned_matroska_{flac,opus,alac,mp3,pcm,aac}_export` geometry helpers, and `owned_aac::stream::adts_prefix_info_for_transform` (`crates/fvid-media/src/owned_aac/stream.rs`), and enforces `options.streams.len() <= 1` plus combined `index_bytes + info.len() <= max` in `owned_wave_metadata::{mp4_source_info, matroska_source_info_with_interval}` (`crates/fvid-media/src/owned_wave_metadata.rs`). Forty-five integration tests in `mp4_audio_metadata_admission` pass. See `batch-378-validation.json`.

## Batch 379 — Charge all thirteen canonical FileTags fields in Matroska metadata working-set admission across lossless export, MP4/ADTS remux, and subtitle conversion

FVID-0386 adds `FileTags::entries(&self)` (`crates/fvid-media/src/owned_file_tags.rs`) returning all thirteen canonical `(name, value)` pairs and iterates `tags.entries()` across `owned_lossless::estimate_matroska_metadata_working_bytes` (`crates/fvid-media/src/owned_lossless.rs`), `owned_mp4_remux::validate_selected_prefix_packet_budget` (`crates/fvid-media/src/owned_mp4_remux.rs`), `owned_matroska_adts_impl::estimate_adts_matroska_metadata_bytes` (`crates/fvid-media/src/owned_matroska_adts_impl.rs`), and `owned_subtitle_impl::estimate_subtitle_matroska_metadata_working_bytes` (`crates/fvid-media/src/owned_subtitle_impl.rs`). Forty-six integration tests in `mp4_audio_metadata_admission` pass. See `batch-379-validation.json`.

## Batch 380 — Enforce WebmReader::estimated_index_payload_bytes against max_controlled_bytes across Matroska probe, identity copy/remux, and subtitle conversion

FVID-0387 verifies `reader.estimated_index_payload_bytes() <= limit` after `scan_all()` in `owned_webm_probe::try_probe_webm_with_limits` (`crates/fvid-media/src/owned_webm_probe.rs`) and `media_probe::matroska_with_limits` (`src/media_probe.rs`), and includes `reader.estimated_index_payload_bytes()` in the index memory admission of `owned_matroska_copy_impl::inspect_with_packet_and_memory_limits` (`crates/fvid-media/src/owned_matroska_copy_impl.rs`) and `owned_subtitle_impl::{try_plan_with_controls, try_convert_matroska_inner}` (`crates/fvid-media/src/owned_subtitle_impl.rs`). Forty-seven integration tests in `mp4_audio_metadata_admission` pass. See `batch-380-validation.json`.

## Batch 381 — Enforce WebmReader::estimated_index_payload_bytes against max_controlled_bytes across Matroska lossless transcode, multitrack export, video decode, and subtitle burn-in

FVID-0388 verifies `reader.estimated_index_payload_bytes() <= limit` in `owned_lossless::check_source_matroska_metadata_budget` (`crates/fvid-media/src/owned_lossless.rs`), `owned_ffv1_export::input_limited` (`crates/fvid-media/src/owned_ffv1_export.rs`), `owned_webm_multitrack_export::qualify` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`), `owned_video_decode::decode_webm` (`crates/fvid-media/src/owned_video_decode.rs`), and `owned_subtitle_burn::prepare` (`crates/fvid-media/src/owned_subtitle_burn.rs`). Forty-eight integration tests in `mp4_audio_metadata_admission` pass. See `batch-381-validation.json`.

## Batch 382 — Enforce Mp4Reader::estimated_index_payload_bytes and WebmReader::estimated_index_payload_bytes against max_controlled_bytes across MP4 probe, remux, concat, video decode/bridge, multitrack lossless export, and native audio source inspection

FVID-0389 enforces `reader.estimated_index_payload_bytes() <= max_controlled_bytes` across `owned_mp4_probe_impl` (`crates/fvid-media/src/owned_mp4_probe_impl.rs`), `owned_mp4_video_bridge` (`crates/fvid-media/src/owned_mp4_video_bridge.rs`), `owned_mp4_multitrack_export` (`crates/fvid-media/src/owned_mp4_multitrack_export.rs`), and `native_media` (`src/native_media.rs`) for both `Mp4Reader` and `WebmReader`, includes `reader.estimated_index_payload_bytes()` in the working-set accounting of `owned_mp4_remux::validate_selected_prefix_packet_budget` (`crates/fvid-media/src/owned_mp4_remux.rs`), `owned_mp4_video_decode::decode_presented` (`crates/fvid-media/src/owned_mp4_video_decode.rs`), and `mp4_concat::open_with_options` (`src/container/mp4_concat.rs`), and iterates `first_tags.entries()` across all 13 canonical tags in `mp4_concat::open_with_options`. Forty-nine integration tests in `mp4_audio_metadata_admission` pass. See `batch-382-validation.json`.

## Batch 383 — Charge simultaneous input PCM buffers and merge channel-vector buffers in owned_audio_mix and enforce estimated_index_payload_bytes across native_plan, native_export, pcm_matroska, and native_audio_mix::concat

FVID-0390 charges simultaneous input PCM buffers plus output and intermediate vector buffers against `max_controlled_bytes` in `owned_audio_mix::{audio_plan_with_controls, mix_with_controls_and_decoder, merge_with_controls_and_decoder}` (`crates/fvid-media/src/owned_audio_mix.rs`), and enforces `reader.estimated_index_payload_bytes() <= mem_limit` for both `Mp4Reader` and `WebmReader` across `native_plan` (`src/native_plan.rs`), `native_export` (`src/native_export.rs`), `pcm_matroska` (`src/native_export/pcm_matroska.rs`), and `native_audio_mix::concat` (`src/native_audio_mix/concat.rs`). Fifty integration tests in `mp4_audio_metadata_admission` pass. See `batch-383-validation.json`.

## Batch 384 — Charge all thirteen canonical FileTags fields in pcm_matroska::{check_output_memory_budget, Sink::new_with_source_metadata} and enforce Mp4Reader::estimated_index_payload_bytes in media::validate_mp4_packet_budget, native_export::is_native_mp4_matroska_with_memory_limit, and owned_mp4_audio_export::open

FVID-0391 iterates `metadata.tags.entries()` across all 13 canonical `FileTags` fields in `pcm_matroska::check_output_memory_budget` and `pcm_matroska::Sink::new_with_source_metadata` (`src/native_export/pcm_matroska.rs`), and enforces `reader.estimated_index_payload_bytes() <= max_controlled_bytes` in `media::validate_mp4_packet_budget` (`src/media.rs`), `native_export::is_native_mp4_matroska_with_memory_limit` (`src/native_export.rs`), and `owned_mp4_audio_export::open` (`crates/fvid-media/src/owned_mp4_audio_export.rs`). Fifty-one integration tests in `mp4_audio_metadata_admission` pass. See `batch-384-validation.json`.

## Batch 385 — Include destination WAVE LIST/INFO working set in container audio export admission budgets and propagate Matroska source_geometry errors instead of silently degrading to stereo 48 kHz

FVID-0392 adds `owned_wave_metadata::wave_metadata_working_set_bytes` and `check_wave_metadata_budget_with_base(tags, options, base_bytes)` (`crates/fvid-media/src/owned_wave_metadata.rs`) so container and codec audio exports (`owned_mp4_audio`, `owned_matroska_audio`, `owned_raw_flac`, `owned_raw_mp3`, `owned_ogg_opus_export`, `owned_ogg_vorbis_export`, `owned_ogg_joint_export`, and `owned_g711_wave_export`) charge `base_bytes + wave_metadata_working_set_bytes(tags)` against `max_controlled_bytes`, and returns `Result<(u16, u32)>` from `source_geometry` in `src/media.rs`, `src/native_loudnorm.rs`, and `src/native_plan.rs` so Matroska budget/cancel/I/O errors propagate immediately. Fifty-two integration tests in `mp4_audio_metadata_admission` pass. See `batch-385-validation.json`.

## Batch 386 — Charge destination WAVE LIST/INFO working set in ADTS AAC decode admission and preserve metadata working-set admission in owned_adts_loudnorm and owned_container_loudnorm

FVID-0393 charges `decode_config_admission_bytes(asc, rate) + wave_metadata_working_bytes(0, options)` in `owned_aac::stream::check_adts_decode_admission` and `adts_prefix_info_for_transform` (`crates/fvid-media/src/owned_aac/stream.rs`), and enforces `owned_audio_plan::plan_decode_audio(source, &Default::default(), options)` in `owned_adts_loudnorm::{apply, plan_loudnorm}` (`crates/fvid-media/src/owned_adts_loudnorm.rs`) and `owned_container_loudnorm::{apply, plan_loudnorm}` (`crates/fvid-media/src/owned_container_loudnorm.rs`). Fifty-three integration tests in `mp4_audio_metadata_admission` pass. See `batch-386-validation.json`.

## Batch 387 — Charge simultaneous plan, cue, and packet payload working sets in owned_mp4_remux, owned_adts_remux, owned_concat, and owned_matroska_adts_impl

FVID-0394 charges `plan_bytes + packets * 32 + sample.size` against `max_controlled_bytes` in `owned_mp4_remux::validate_selected_prefix_packet_budget` (`crates/fvid-media/src/owned_mp4_remux.rs`), and charges `base_bytes + packets * 32 + meta_bytes + packet.len()` in `owned_adts_remux::plan_remux` (`crates/fvid-media/src/owned_adts_remux.rs`), `owned_concat::plan_concat` (`crates/fvid-media/src/owned_concat.rs`), and `owned_matroska_adts_impl::write_aac_packets` (`crates/fvid-media/src/owned_matroska_adts_impl.rs`). Fifty-four integration tests in `mp4_audio_metadata_admission` pass. See `batch-387-validation.json`.
























## Fragment default duration precedence

FVID-0395 preserves a present zero `tfhd.default_sample_duration` over a nonzero `trex` default. Both parser wrappers preserve DTS/PTS 0,0,1 and durations 0,1,1. Synthetic VP9 playback retains 40ms duration, exact RGB, seek, rewind and EOF. Seven no-default-feature and twelve player regressions pass. [Validation](batch-389-validation.json).

## Layered HEVC container presentation order

FVID-0396 selects the existing bounded common MP4 presentation queue for layered HEVC with decreasing container PTS. Owned reordered and ordered companions retain exact synthetic oracle pixels, 160ms duration, sequential intervals, seek, rewind and EOF. All 28 focused/related player checks pass. [Validation](batch-390-validation.json).

## Unknown MP4 media duration

FVID-0397 recognizes version-width all-ones `mdhd` durations and recovers actual track duration from validated samples, including fragment extension. Owned regular/fragmented v0/v1 witnesses retain 60ms duration, exact packets/RGB, probe, seek, rewind and EOF. Seven core and sixteen player checks pass. [Validation](batch-391-validation.json).

## MP4 time-header reserved flags

FVID-0398 rejects nonzero reserved `mvhd`/`mdhd` flags in both versions and wrappers. Eight short synthetic malformed companions require the precise refusal before decoding; separate controls retain exact packets/clocks and native RGB, seek, rewind and EOF. Seven core and sixteen player checks pass. [Validation](batch-392-validation.json).

## Batch 393 — FVID-0399: Validate complete MP4 movie/media time-header payload sizes

FVID-0399 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_time_header_sizes.rs`): Validate the complete version-specific fixed payload before reading timestamps: mvhd 100/112 bytes, mdhd 24/36 bytes. Core 9, player 20 and related integration 37 tests pass, no failures or ignored tests. Valid V0/V1 videos preserve exact RGB, 60ms duration, packet clocks, seek, rewind and repeated EOF. The old basic MP4 test constructor now authors a complete movie header; all original assertions remain. See `batch-393-validation.json`.

## Batch 394 — FVID-0400: Validate reserved MP4 fragment decode-time flags

FVID-0400 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_tfdt_flags.rs`): Reject nonzero reserved tfdt FullBox flags before consuming decode time. Eleven core and twenty player checks pass with no failures/ignored tests. Zero-flag V0/V1 fragments preserve three exact packet clocks and RGB frames at 50Hz/60ms, eight seeks per file, rewind, repeated EOF and budget rejection. Damaged companions fail specifically on their flags before decoding. See `batch-394-validation.json`.

## Batch 395 — FVID-0401: Preserve MP4 chapter clocks when the movie duration is unavailable

FVID-0401 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_unknown_movie_chapters.rs`): For unknown/all-ones or unstated zero movie duration, use the indexed supported-track presentation length only as the chapter clock-selection bound. Convert media ticks to movie ticks with wide arithmetic, and honor total movie edit durations including empty edits. Known movie duration and legacy movie-clock chapter behavior are preserved. Thirty core and 36 player tests pass, no failures/ignored tests. Twenty-four short regular/fragmented V0/V1 chapter witnesses preserve 0/20/40ms chapters, titles, 60ms exact RGB playback, seek, rewind, EOF, packet clocks and budgets; four edited companions preserve absolute chapter metadata. Native leading-edit playback policy is unchanged. An uncounted positive-CTS follow-up uses known tkhd presentation length or a nominal composition endpoint for chapter clock bounds when mvhd is unknown/zero. Forty-five selected checks and core build pass; no extra ID. See `batch-395-validation.json`.

## Batch 396 — FVID-0402: Support implicit MP4 fragment data-base addressing

FVID-0402 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_implicit_fragment_base.rs`): Resolve a first implicit traf base from its moof and later implicit bases from the preceding traf data end. Explicit absolute/moof bases override inheritance. For unrepresented tracks, resolve declared run byte extents without decoding, packet allocation or default-sized sample-count loops; unresolved foreign data cannot become an implicit base. Thirteen core, 21 player and 28 related integration tests pass, zero failed/ignored. Nine valid controls/witnesses preserve exact packet offsets/bytes/DTS/PTS, 60/120ms duration, neutral RGB, 72 seeks, rewind and repeated EOF. Three corrupt witnesses refuse specifically, including a huge foreign sample count; sample and packet budget checks remain enforced. See `batch-396-validation.json`.

## Batch 397 — FVID-0403..0404: Resolve open-ended fragmented MP4 media edits in exact media time

- **FVID-0403** — Resolve open-ended fragmented MP4 media edits in exact media time (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_open_ended_edit.rs`): Admit only the single fragmented open media edit, retaining raw duration=0. A stack-backed resolved edit view derives the composition endpoint from indexed samples and uses media timescale for the open range, avoiding movie-clock double rounding. Playback, owned decode, probe, shader windows, AAC remux and audio planning/export consume the view. Coarse 25Hz and fractional 1001Hz movie clocks preserve exact 20ms media frames; positive CTS and non-sample-aligned audio endpoints stay exact. Nonzero AAC/PCM/ALAC oracles, public WAVE publication, budgets, seek, rewind and EOF pass. Zero-count CI follow-up: retain rejection when an open edit has no media remaining and assert the exact positive open-tail window; all prior assertions remain. See docs/improvements/evidence/open-edit-unit-contract/validation.json.
- **FVID-0404** — Honor finite fixed PCM/ALAC playback edits (`src/playback/audio/mp4_audio.rs`, `tests/mp4_fixed_audio_edits.rs`): Route finite PCM/ALAC media edits through the existing rational variable-edit scheduler even for one constant sample description. Preserve original media packets and decoder configuration while clipping presentation to the requested range. Native duration, nonzero PCM, seek suffix, rewind, repeated EOF and fractional sample positions now match their owned oracles. AAC finite-path seek conventions and the established leading-empty-edit policy are preserved.

See `batch-397-validation.json`.

## Batch 398 — FVID-0405: Reuse validated single-block ADTS transport allocations without duplicating payload

FVID-0405 (`crates/fvid-media/src/owned_aac/adts_impl.rs`, `tests/adts_packet_working_set.rs`): After framing and complete CRC validation, retain the original transport allocation for one raw block; compact only the optional CRC prefix. PCE bootstrap reuses its body too; multi-block ordering/validation stays intact. Both parsers use 2 allocations and 1058 scoped peak bytes repeatedly. Offline automatic allocation regression and 27 focused PCM/CRC/PCE/multi-block/budget checks pass. Three pre-existing broader integration failures are separately reproduced and retained. No throughput or RSS claim. See `batch-398-validation.json`.

## Batch 399 — FVID-0406: Retry transient interrupted MP4 seeks while preserving cursor recovery

FVID-0406 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_interrupted_seek.rs`): Retry only Interrupted for idempotent absolute and End(0) seeks in shared MP4 opening, packet and bounded-range reads. Forty selected checks pass, including exact software-decoded colors, PTS/duration, random seek, rewind, repeated EOF, permanent-error recovery and related malformed/header/edit/fragmentation regressions. Finite before/after-movement faults and both parser wrappers are covered. See `batch-399-validation.json`.

## Batch 400 — FVID-0407: Report the complete unedited MP4 composition endpoint in native playback duration

FVID-0407 (`src/playback/video/mp4_dispatch.rs`, `tests/mp4_presentation_endpoint.rs`): Use a private lazy index-derived composition endpoint for NativeReader unedited duration, including the final equal-PTS group span. Known track/movie header bounds prevent hidden/non-output timestamp outliers from stretching the estimate, with sub-movie-tick quantization tolerance and zero/unknown markers preserved. Raw media clocks and edit windows remain intact. The follow-up passes 51 software playback/parser/seek/header/fragment checks and a core build; no extra ID is counted. Fully unstated duration remains a nominal index estimate. A zero-count signed-CTS follow-up distinguishes indexed decode coverage from genuine declared tails: negative preroll ends at20/40ms instead of a phantom60ms, including unknown headers. Nine authored cases preserve exact signed metadata/YUV, native playback/seek/rewind/EOF, declared tails and appended-fragment duration.65 integration checks,83 MP4 unit executions and both locked/offline builds pass;one old ignore remains. See `batch-400-validation.json`.

## Batch 402 — FVID-0408: Complete MP4 playback after an explicit zero-duration terminal presentation window

FVID-0408 (`src/playback/video/mp4.rs`, `tests/mp4_empty_tail.rs`): Decode and validate zero-duration terminal samples without publishing an empty window. Preserve explicit zero timing, skip only the terminal empty presentation and restart EOF seeks early enough to return the last visible frame. Streaming and owned decode agree on two exact pictures and clean EOF; corrupt terminal payloads still refuse. Thirty-two distinct checks and a core build pass, including timestamps, seek, rewind, EOF and private positive/invalid clock guards. See `batch-402-validation.json`.

## Batch 403 — FVID-0409: Recover covered and final MP4 pictures after decode-only sync landings

FVID-0409 (`src/playback/video/native.rs`, `tests/mp4_decode_only_seek.rs`): Replay once when a non-initial indexed landing produces no displayed frame or begins after the requested interval. Keep real leading gaps and codec errors unchanged, release unhelpful output before reset, and preserve subsequent playback. Exact software pixels, timestamps, EOF, trim/repeat edit mappings, error recovery and byte traversal guards pass: 46 distinct checks plus a core build. See `batch-403-validation.json`.

## Batch 404 — FVID-0410: Continue finite MP4 edit programs after source EOF without retiming the last picture

FVID-0410 (`src/playback/video/native.rs`, `tests/mp4_edit_window_eof.rs`): Advance finite edit windows on media EOF without modifying the last published clock. Recover a covering prefix once at internal sync restarts. A seek landing at source sample zero in a later edit is not the beginning of the presentation. Preserve decoder errors and short-circuit a complete source scan with no nonnegative display intervals. 34 distinct integration tests pass across 9 targets. Four new tests include exact authored YUV controls, six sequential/rewind edit programs, 84 seek positions plus continuation, all-hidden/negative-preroll byte bounds, and corrupt-codec refusal. No-default-features offline locked build check passes. See `batch-404-validation.json`.

## Batch 405 — FVID-0411: Refuse looping MP4 edits explicitly instead of silently treating them as finite

FVID-0411 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_edit_loop.rs`): Check elst FullBox flags before consuming its finite edit rows. Bit zero returns the explicit unsupported looping-edit capability error. Unknown bits return an invalid edit-list flag error, including unknown bits combined with the loop bit. Both wrappers share the check. 49 distinct checks pass in 11 integration targets, including 3 new tests. Zero-flag V0/V1 trim controls retain exact analytic YUV, software RGB, PTS/duration, random seeks, rewind and EOF. Related finite/open-ended video/audio edits, variable rates, decode-only coverage, header sizes and seek I/O remain green. Locked offline no-default-features core build check passes. See `batch-405-validation.json`.

## Batch 406 — FVID-0412: Reject malformed fixed-size MP4 pixel-aspect boxes instead of discarding field-read errors

FVID-0412 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_pasp_sizes.rs`): Require exactly the two u32 spacing fields (8 payload bytes) for a present pasp. Propagate field read errors rather than converting them into absence. Existing zero-spacing fallback and absent-box behavior stay intact. 51 distinct checks pass in 8 integration targets. Dedicated positive controls cover exact normalized 2:1 aspect, square absence and existing zero fallback, random packet reads, analytic YUV16, native software RGB, exact clocks, 24 seeks, rewind and EOF. Related MP4 parsing, rotation/aspect, sample-description changes, video reconfiguration, edit/refusal and timing regressions pass. Locked offline no-default-features core build check passes. See `batch-406-validation.json`.

## Batch 407 — FVID-0413: Support empty-duration MP4 fragments with correct implicit decode-time continuation

FVID-0413 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_empty_fragment.rs`): Support tfhd.duration-is-empty without manufacturing sample entries. Maintain a per-track decode cursor independent of sample-list tails, apply the explicitly present or inherited default duration, preserve tfdt overrides, and include a trailing empty interval in track duration. Refuse every trun in an empty-duration traf, even a zero-row run, and check its end timestamp. 42 distinct integration checks pass across 10 targets, including expanded 4-test regression. Ten valid 3-picture controls include explicit V0/V1 and omitted tfdt, leading/middle/trailing gaps, inherited/zero duration and consecutive gaps. Both parser wrappers agree on packet bytes, DTS/PTS, nominal duration and track endpoint; software playback retains analytic YUV, RGB, exact intervals, 120 seek positions with continuation, rewind and repeated EOF. Exact three-sample budget passes and two-sample budget refuses, so gaps do not consume sample entries. Locked offline no-default-features core build check passes. See `batch-407-validation.json`.

## Batch 409 — FVID-0414: Preserve playable MP4 audio beside unreferenced unknown sample descriptions

FVID-0414 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_unused_audio_descriptions.rs`): Retain opaque unknown audio SampleEntry metadata as already done for video, validating its common data reference. Select supported codec state only from actual sample or compact-chunk references before clock construction; unknown unselected entries cannot hide supported audio or reach a decoder. Fully unsupported tracks and actually selected unknown entries retain explicit capability refusal. 50 distinct integration checks pass across 9 targets, including all 6 expanded new tests. Twenty authored regular, fragmented and compact-index cases retain both tracks, exact selected descriptions, encoded packet bytes, DTS/PTS, independent nonzero PCM across coding/rate/channel/precision transitions, seek/rewind, six-picture video companions, owned normalized waveform equality and actual audio-worker replay. Core locked/offline no-default-features check passes. Four unrelated owned audio failures are reproduced before and after the fix. An uncounted follow-up qualifies automatic PCM eligibility using the first referenced setup, preventing an unused unsupported first entry from selecting a quiet alternate. Fifty-two checks and a core build pass. Prior owned audio test-oracle failures were requalified separately in batch-410. See `batch-419-prior-requalification-validation.json`.

## Batch 411 — FVID-0415: Preserve shifted PCM tails when reconstructing quantized MP4 audio clocks

FVID-0415 (`src/playback/audio/mp4_clock_recovery.rs`, `tests/mp4_audio_presentation_end.rs`): Use the recovered run presentation endpoint, including its composition origin, as the clipping/seek extent. Preserve explicitly stated time beyond indexed decode coverage (including empty tails). For the non-recovered path, cache a nominal indexed composition endpoint lazily, preserve stated empty tails, and retain explicit edit/recovered-clock duration precedence. Uniform PCM uses its compact aggregate endpoint. 62 distinct integration checks pass: 61 in ten related targets plus one added worker regression (two focused checks overlap and are not counted twice). All 38 own PCM/ALAC+VP9 movies retain independent PCM byte counts/values, exact rational timestamps, seek suffixes, rewind, EOF, and all three video pictures. The worker plays and rewinds both shifted 1001Hz PCM/ALAC cases with the full 1920-byte waveform. Locked/offline no-default-features core build passes. See `batch-411-validation.json`.

## Batch 416 — FVID-0416: Use composition endpoints for native MP4 audio seek bounds and unedited AAC playback

FVID-0416 (`src/playback/audio/mp4_audio.rs`, `tests/mp4_reconfigured_seek_endpoint.rs`): Share the cached exact composition endpoint between duration, changing-setup/preroll seek bounds and unedited AAC crop limits.83 distinct checks pass (69 integrations across 13 suites, 14 audio-reader units), plus locked/offline core and player builds.21 short authored video witnesses cover signed CTTS/TRUN, regular/fragmented PCM, equivalent/changing setups, AAC pulse reference/history, exact suffix PCM/PTS, rewind, EOF and actual playback workers. See `batch-416-validation.json`.

## Batch 417 — FVID-0417: Preserve fractional AAC composition origins and exact native seek boundaries

FVID-0417 (`src/playback/audio/mp4_audio.rs`, `tests/mp4_aac_fractional_origin.rs`): Use exact rational presentation origins and crop relative to those origins before rounding the retained sample window.86 distinct checks pass (72 integrations across14 suites plus14 root audio-reader units), with locked/offline core and player builds.16 authored AAC/VP9 videos with independent analytical pulse reference cover three media rates, signed offsets, exact PCM/PTS, both seek APIs, rewind, EOF and actual playback workers. See `batch-417-validation.json`.

## Batch 418 — FVID-0418: Inspect changing MP4 audio metadata while preserving explicit constant-format DSP admission

FVID-0418 (`src/native_media.rs`, `tests/native_mp4_initial_metadata.rs`): Separate metadata inspection of the first actual decoder setup from explicit constant_audio_source_info_selected admission variants. Migrate repository DSP/CLI route callers with identical arguments/control flow and document external admission migration.Initial qualification had173 passing checks and seven separately proved original failures on d25e228c19. The uncounted prefix follow-up subsequently repaired those failures, passed210 checks/core/player builds and published the change. Current acceptance is backed by batch-419 prior-ID requalification; no old ID is recounted. See `batch-419-prior-requalification-validation.json`.

## Batch 419 — FVID-0419: Preserve declared packet-free MP4 audio tails through open and finite edits

FVID-0419 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_open_edit_tail.rs`): Distinguish indexed decode coverage from composition endpoints, retain genuine declared tails without undoing signed CTTS shifts, and split native edit schedules into exact source/silence ranges. Append bounded zero chunks in owned f32/f64 integer and rational normalized paths only after source packets drain and only within declared time. Twelve open/finite AAC/PCM/ALAC video pairs preserve nonzero PCM then exact silence, both edit/tfdt versions, trim origins, timestamps, seek/rewind/EOF, probes, resampling/rematrixing and actual workers. Invalid codec packets and edits beyond the declared tail retain explicit refusals and no WAVE publication.281 distinct checks pass with locked/offline core/player builds. An original SSR interval-routing failure and registry metadata repairs are uncounted. See `batch-419-validation.json`.

## Batch 420 — FVID-0420: Preserve declared packet-free middle MP4 audio intervals and exact seek boundaries

FVID-0420 (`crates/fvid-media/src/owned_mp4_audio_normalized_impl.rs`, `tests/mp4_audio_middle_gap.rs`): Split source clock/resampling runs at declared DTS/PTS gaps while retaining decoder history. Emit bounded silence in owned f32/f64 and native schedules, drain delayed PS before the gap without reading oversized future payloads, preserve repeated edits and exact coarse-clock boundaries/seek. Fifty valid synthetic VP9/audio witnesses cover AAC/PS/PCM/ALAC, native PCM/PTS, seek/rewind/EOF, real playback workers and normalization; malformed codecs retain refusal/no publication. A short two-day-gap file proves bounded seek/output.816 distinct tests pass with locked/offline core/player builds; two hardware tests explicitly ignored. See `batch-420-validation.json`.

## Batch 421 — FVID-0421: Accept representable MP4 composition times from unsigned wide decode timestamps

FVID-0421 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_wide_decode_clock.rs`): Compose DTS and offset in i128, then validate the resulting signed PTS; retain sample-span and decode-end overflow checks. Three valid authored colored VP9 videos preserve exact index/packet/picture data, seek and EOF; a separate real-overflow video remains refused.908 distinct tests pass across58 integration suites,56 root MP4 units and547 media-library units, with locked/offline core/player builds and two explicit hardware ignores. Four original unit-test failures are separately reproduced and corrected with exact rational timestamp assertions and full implicit-base index/payload parity; no extra IDs counted. See `batch-421-validation.json`.

## Batch 422 — FVID-0422: Preserve signed MP4 subtitle spans before clipping and rounding their endpoints

FVID-0422 (`src/playback/subtitles/mp4_subtitles.rs`, `tests/mp4_subtitle_negative_clock.rs`): Compose explicit cue endpoints in signed wide arithmetic before clamping/rescaling, omit invisible spans, and retain unspecified-duration successor/fallback behavior. Six short authored VP9/UTF-8 tx3g videos verify cue contents and exact nanosecond bounds, hidden negative text, ordinary positive controls, true video seek and active-cue boundaries.418 distinct checks pass:321 in64 integration suites and97 unique root units, excluding six overlapping checks. Native subtitle paint/seek/end checks and locked/offline core/player builds pass; two hardware checks explicitly ignored. See `batch-422-validation.json`.

## Batch 423 — FVID-0423: Decode BOM-tagged UTF16 MP4 timed text and reject malformed cue strings

FVID-0423 (`src/playback/subtitles/mp4_subtitles.rs`, `tests/mp4_subtitle_unicode.rs`): Decode UTF16BE/optional LE before markup processing, preserve unmarked UTF8 behavior, use code-unit-aware boxed NUL termination and complete group-layout recognition. Thirteen acceptance videos and fifteen separate malformed-cue recovery witnesses verify exact Unicode, surrogate pairs, newlines, empty text, source packets/pictures, real seek/clock bounds and healthy following text.423 distinct checks pass:325 in65 integration suites and98 unique root units, excluding seven overlaps; locked/offline core/player builds and native paint/seek/end checks pass. Two hardware tests explicitly ignored. An introduced prefix-selection regression and older font-descriptor fixture qualification are corrected without additional IDs. See `batch-423-validation.json`.

## Batch 424 — FVID-0424: Respect per-block Matroska subtitle durations instead of extending cues through gaps

FVID-0424 (`src/playback/subtitles/webm_subtitles.rs`, `tests/matroska_subtitle_spans.rs`): Use explicit BlockDuration before track defaults/next-cue fallback; omit zero-length intervals and preserve missing-duration compatibility.24 authored VP9/UTF8/ASS videos verify exact packets/pictures, bounds, EOF and real seeks; a separate duration overflow remains refused. 377 distinct checks pass across66 integration suites and48 root subtitle units; core/player locked offline builds pass, one physical-GPU check ignored. Two old literal expectations corrected from independently verified packet spans, no extra IDs. See `batch-424-validation.json`.

## Batch 425 — FVID-0425: Decode canonical Matroska SSA events with an empty Layer field

FVID-0425 (`src/subtitles.rs`, `tests/matroska_ssa_text.rs`): Recognize numeric ReadOrder with empty SSA or numeric ASS Layer; preserve complete dialogue clocks and fallback/refusal behavior.Three authored VP9/SSA/ASS videos verify exact source packets/pictures, text/newlines/Unicode, real seek gaps/endpoints and EOF. 117 distinct checks pass across20 integration suites and48 root subtitle units; locked/offline core/player builds pass, one physical-GPU test ignored. Two original budget expectation failures are reproduced separately and repaired with exact zero-packet policy and explicit32KiB refusal/64KiB index-aware admission; no extra IDs counted. See `batch-425-validation.json`.

## Batch 426 — FVID-0426: Suppress negative-PTS Matroska subtitle frames after decoding

FVID-0426 (`src/playback/subtitles/webm_subtitles.rs`, `tests/matroska_negative_subtitles.rs`): Read and decode each caption, then omit negative-PTS frames instead of clamping them tozero. Six authored VP9/UTF8/SSA/ASS videos retain exact packets, decoded pictures/intervals and EOF; whole-negative/cross-zero captions stay hidden while positive controls and healthy following text remain visible. Actual seeks select the exact expected picture and cue at exclusive endpoints/gaps.120 distinct checks pass across21 integration suites and48 root subtitle units; locked/offline core/player builds pass, one physical-GPU check ignored. All codec/sign variants count as one improvement. See `batch-426-validation.json`.

## Batch 427 — FVID-0427: Apply CodecDelay to native Matroska subtitle presentation clocks

FVID-0427 (`src/playback/subtitles/webm_subtitles.rs`, `tests/matroska_subtitle_delay.rs`): Subtract nanosecond CodecDelay from current and successor PTS in i128 before admitting nonnegative presentation times. Explicit BlockDuration, track defaults and next-cue fallback keep their duration policy at the correct origin; negative-after-delay captions are decoded without display.54 authored VP9/UTF8/SSA/ASS videos verify exact raw packets, parsed delay, actual pictures/EOF, text/bounds and real seeks with exact selected frame intervals.123 distinct checks pass across22 integration suites and48 root subtitle units; locked/offline core/player builds pass, one physical-GPU check ignored. All variants and retained initial witnesses count as one improvement. See `batch-427-validation.json`.

## Batch 428 — FVID-0428: Respect Matroska subtitle Invisible flags and preserve visible fallback intervals

FVID-0428 (`src/playback/subtitles/webm_subtitles.rs`, `tests/matroska_invisible_subtitles.rs`): Read/decode captions before suppressing Invisible output; derive fallback endpoints from the next visible packet.24 authored VP9/UTF8/SSA/ASS Block/SimpleBlock videos verify exact packets/flags/pictures, visible controls, hidden middle/tail/all text, EOF and actual seeks with exact selected frame intervals and40/60/80ms boundaries.126 distinct checks pass across23 integration suites and48 root subtitle units; locked/offline core/player builds pass, one physical-GPU test ignored. All mask/codec/block variants count as one visibility improvement. See `batch-428-validation.json`.

## Batch 429 — FVID-0429: Apply MP4 subtitle edit programs and align them with the automatic video clock

FVID-0429 (`src/playback/subtitles/mp4_subtitles.rs`, `tests/mp4_subtitle_edits.rs`): Map clipped and repeated cue appearances with exact rational clocks, preserve empty edits/open media and source precedence for coincident PTS, and normalize the leading origin of the same automatically selected video track. Preserve no-edit zero-origin fallback; bound each returned cue list by Limits.samples.30 authored VP9/tx3g videos plus a malformed-edit refusal verify exact metadata/packets/pictures, actual seeks, EOF, fractional endpoints and expansion limits.495 distinct checks pass across84 integration suites and98 unique root units; locked/offline core/player builds pass, two hardware checks ignored. Fixture repairs and an introduced tie-order regression add no IDs; necessary cue-clock migration is documented. See `batch-429-validation.json`.

## Batch 430 — FVID-0430: Preserve valid MP4 text tracks with dormant unsupported sample descriptions

FVID-0430 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_text_descriptions.rs`): Retain common-header-validated opaque text catalog entries, validate actual references after complete fragment indexing, preserve parallel track header metadata, and compare legacy text samples to their recognized primary setup.43 authored VP9/text videos cover3 handlers, classic/fragmented reference selection and dormant/active/mixed unknown entries; malformed unused data references still refuse exactly. Exact packets/pictures/intervals, actual native seek/captions and EOF pass.1043 distinct checks pass across85 integration suites and deduplicated root/media units; locked/offline core/player builds pass. Two existing hardware checks and one pending RGB compatibility check remain ignored. All variants and internal guard/alignment repairs count once. See `batch-430-validation.json`.

## Batch 431 — FVID-0431: Read version-zero MP4 chapter-list layout and restore chapter navigation

FVID-0431 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_chapter_versions.rs`): Select chapter count/entry start from the stated version instead of assuming four reserved bytes.20 authored VP9 movies cover version0/1, 10MHz/legacy movie clocks, known/unknown/zero duration and optional empty/truncated metadata. Both parser paths return exact UTF8 titles/times; real native seeks select exact20ms intervals; every movie decodes three exact pictures and EOF.1012 distinct checks pass across86 integration suites and deduplicated root/media units, plus locked/offline core/player builds. One physical-GPU test, two explicit HTTP integration checks and one existing RGB compatibility acceptance remain ignored. All variants count once. See `batch-431-validation.json`.

## Batch 432 — FVID-0432: Read standard compact MP4 sample-size tables with bounded indices and strict table validation

FVID-0432 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_compact_sizes.rs`): Borrow and decode4/8/16-bit stz2 entries after checked exact body lengths; preserve normal stsz and constant-size PCM/MACE chunk indexing, and reject ambiguous tables/reserved fields/invalid widths/counts/zero sizes.20 authored video/text/PCM/MACE movies verify exact packets/pictures, native caption and audio media seeks, PCM suffixes, real fragmented init/edit clocks and EOF;13 damaged companions require exact refusal.58 files regenerate identically.1061 distinct checks pass across95 integration suites and deduplicated root/media units; locked/offline core/player builds pass. One physical-GPU test and one existing RGB compatibility acceptance remain ignored. All variants and introduced prototype/test repairs count once; no performance claim. See `batch-432-validation.json`.

## Batch 433 — FVID-0433: Validate terminal-zero STTS runs in indexed packet units

FVID-0433 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_terminal_zero_runs.rs`): After classic index construction, reject more than one real zero-duration tail packet using expanded/uniform index units. Preserve a single zero tail, including one six-sample MACE block; the existing STTS rule makes zero packets a suffix.12 authored healthy movies preserve exact video packets/pictures, native seek/EOF, raw PCM/MACE packet clocks and decoded blocks;16 malformed timing companions refuse consistently,8 newly rejected.37 files regenerate identically; logical RLE equivalence and unchanged media hashes prove the intended failure.1067 distinct checks pass across97 integration suites and deduplicated root/media units, plus locked/offline core/player builds. One physical-GPU test and one existing RGB compatibility acceptance remain ignored. All table/codec variants count once; no audible zero-window audio, TRUN-policy or performance claim. See `batch-433-validation.json`.

## Batch 434 — FVID-0434: Resolve bounded sample-entry indices into multiple self-contained MP4 data references

FVID-0434 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_local_data_references.rs`): Validate the complete nonempty local dref catalog against its declared count and exact existing self-contained URL/alias form; validate each common SampleEntry reference as a nonzero in-range index, including unused opaque descriptions.19 authored movies cover URL/alias orders, third/largest65535 indices, alternating descriptions, PCM/MACE, captions and real text fragments. Exact video/text packets, pictures, native seeks/EOF, independent PCM and baseline MACE PCM pass;12 malformed/unsupported companions retain specific refusals.47 files regenerate identically.1087 distinct checks pass across101 integration suites and deduplicated root/media units; locked/offline core/player builds pass. One physical-GPU test and one existing RGB compatibility acceptance remain ignored. All variants count once; no external-resolution or performance claim. See `batch-434-validation.json`.

## Batch 435 — FVID-0435: Recognize QuickTime preview atoms before movie data

FVID-0435 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_preview_prefix.rs`): Add only pnot and PICT to the common opening-atom predicate; all box/track validation stays in the parser. Five valid MOV orders retain exact packets/pictures, native playback, rewind, EOF and seven seeks each. Four malformed/empty companions retain parser-specific refusals; JPEG/PNG/GIF-specific refusals stay intact.16 offline authored files regenerate identically; independent ffprobe validates15 packets/clocks.1092 distinct checks pass across102 integration suites and deduplicated root/media units, plus locked/offline core/player builds. No standalone PICT, preview-rendering, performance or physical-device claim. See `batch-435-validation.json`.

## Batch 436 — FVID-0436: Preserve equivalent equal-offset MACE composition runs across codec block boundaries

FVID-0436 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_mace_composition_runs.rs`): Join only consecutive equal-offset CTTS runs before the existing whole-block check, validating every positive run against remaining raw samples without an expanded temporary table.17 equivalent pairs cover MAC3/MAC6 mono/stereo, mixed PCM/MACE, unsigned zero/positive and signed negative offsets, and real block-boundary changes. Both reader APIs preserve packet bytes/index clocks; native audio retains exact PCM/times across playback, rewind and seven seeks; VP9 pictures/seek/EOF pass.20 damaged or unrepresentable companions retain exact refusals.55 offline authored files regenerate identically.1095 distinct checks pass across103 integration suites and deduplicated root/media units; locked/offline core/player builds pass. Existing ignores unchanged. No performance or physical-device claim. See `batch-436-validation.json`.

## Batch 437 — FVID-0437: Preserve MP4 extended language tags through bounded Matroska transport

FVID-0437 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_extended_language.rs`): Read valid/well-formed BCP47 CString prefixes with preferred-language precedence and explicit und override; preserve case and ignore malformed optional metadata. Write extended tags in LanguageBCP47 with legacy und fallback, preserving existing3-letter output; enforce an explicit16KiB writer/reader bound with exact-bound acceptance and bound+1 refusal before output.150 authored offline files cover VP9/AVC, PCM/subtitles, both mdhd versions, invalid tags, padding and independent Matroska. Actual video/PCM/cues, exact packet bytes/times, seek/rewind/EOF and metadata transport pass.140 movies independently retain packet hashes/PTS.1204 distinct checks pass across113 integration suites and deduplicated root/media units; locked/offline core/player builds pass. Existing ignores unchanged. FVID-0337 and zero-cap test-contract repairs add no IDs; a failed phase-budget variant is reverted. No performance/RSS/hardware claim. See `batch-437-validation.json`.

## Batch 438 — FVID-0438: Reject nonletter and reserved-bit packed mdhd language metadata

FVID-0438 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_packed_language.rs`): Optional language metadata falls back to unknown for reserved-bit or nonletter codes, preserving valid three-letter codes and existing und behavior.48 authored mdhd v0/v1 movies pass root/owned readers and probe, exact packets/PTS, actual decoded pictures, seven seeks, rewind and EOF.1206 distinct checks and locked/offline core/player builds pass. Extended-language precedence regressions unchanged. See `batch-438-validation.json`.

## Batch 439 — FVID-0439: Preserve every presentation frame after single-layer MP4 sync fallback

FVID-0439 (`src/playback/video/mp4.rs`, `tests/mp4_seek_reordered_coverage.rs`): Choose only sync candidates whose presentation time exceeds all earlier decode-order PTS, retaining full sequential suffix and equal-PTS group coverage. Existing rewind/reference/parameter preroll remains. Exact decoded color planes, sample indexes, presentation intervals and EOF pass for ordered and reordered movies; independent packet hashes and PTS match.116 integration suites549 pass, root MP4/WebM and media units pass,1216 distinct checks after4 duplicate events, and locked/offline core/player builds pass. No throughput claim. See `batch-439-validation.json`.

## Batch 440 — FVID-0440: Preserve PCM pauses represented solely by MP4 audio composition offsets

FVID-0440 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_audio_composition_gap.rs`): Recognize positive audio presentation gaps independently of DTS and use the presentation endpoint for the normalized default schedule, preserving the stricter non-audio gap rule and exact recovered-clock boundary handling. Two authored short videos preserve identical PCM packet bytes/PTS and video packets. Native/owned exact640-frame PCM including middle silence, nine seeks, trim intervals, cancellation/resource refusals and actual AudioPlayback worker pass.117 integration suites553 pass; root/media units and locked/offline core/player builds pass,1220 distinct checks after4 duplicate events. No extra count for the partial repair or test variants. See `batch-440-validation.json`.

## Batch 441 — FVID-0441: Preserve Macintosh numeric languages in explicitly branded QuickTime files

FVID-0441 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/quicktime_mac_language.rs`): Read a fixed-size major brand without scanning/allocating compatible brands; decode all105 numeric values in the current Apple table only for a complete unique qt brand. Preserve explicit script distinctions, ISO behavior, und and elng precedence, and unknown values.459 authored short movies cover both mdhd versions,105 languages, unknown values, extended-size and invalid optional branding, and elng fallback. Both readers/probe, exact packet/decoded planes/RGB, seven seeks, rewind and EOF pass; all459 independently retain video packet hashes/PTS.118 integration suites555 pass, root/media units and locked/offline core/player builds pass,1222 distinct checks after4 duplicate events. Mapping variants add no IDs. See `batch-441-validation.json`.

## Batch 442 — FVID-0442: Reduce CPU seek overhead for presentation-ordered MP4 video

FVID-0442 (`src/playback/video/mp4.rs`, `tests/mp4_seek_navigation.rs`): Derive a conservative strict-order flag in the existing suffix-index pass; partition the target and search backward within its sync interval. Reordered/equal-PTS inputs retain full prefix coverage, with no added heap index. Seven interleaved baseline/after pairs meet predeclared criteria: navigation35.94→0.434ms (82.77x),actual seek+RGB54.55→18.49ms (2.95x),all7 pairs win,no opening regression. Scope is this memory-resident16x16 all-keyframe workload,not whole-player throughput. Three new black-box tests and existing fixtures preserve sparse sync bounds,reordered/equal PTS,exact pictures/intervals,rewind/EOF and budget refusal.119 integration suites558 pass;1225 distinct checks plus both builds pass. See `batch-442-validation.json`.

## Batch 443 — FVID-0443: Avoid redundant reorder-index storage for ordered MP4 video

FVID-0443 (`src/playback/video/mp4.rs`, `tests/mp4_ordered_index_budget.rs`): Use immutable next-sample PTS for strict order without retaining a suffix array; keep the charged array for reordered/equal PTS and refuse zero reorder working budget. Safe private payload/ledger measurements show ordered1200000→0 bytes,general case1200000 unchanged. Actual150000 pictures and seven opens/seeks pass at2MiB; intrinsic VP9 and general reorder refusals remain. Seven default-release CPU pairs meet predeclared<=10% median regression guard: navigation+4.3%,actual seek+RGB effectively unchanged,opening faster.120 integration suites563 pass;1231 distinct checks and both builds pass. Scope is retained index/decoder budget,not total RAM or player throughput. See `batch-443-validation.json`.

## Batch 444 — FVID-0444: Avoid discarded packet-index searches during mandatory MP4 audio preroll

FVID-0444 (`src/playback/audio/mp4_audio.rs`, `tests/mp4_audio_preroll_navigation.rs`): Perform the packet lookup only when no mandatory preroll is needed; retain requested-floor clamping and complete history replay. Seven paired baseline/after runs meet predeclared criteria:1000 warm seek setups137.64ms to0.004833ms,all7 pairs win. Real seek-to-first-PCM26.54ms versus26.17ms is practically unchanged,not a whole-player speedup claim. Eight short authored controls preserve exact independently decoded PCM,fractional/backward/EOF seeks,direct packet landing and actual AudioPlayback worker output.121 integration suites566 pass;1234 distinct checks and core/player builds pass. See `batch-444-validation.json`.

## Batch 445 — FVID-0445: Accelerate repeated packet seek for verified ordered MP4 audio

FVID-0445 (`src/playback/audio/mp4_audio.rs`, `tests/mp4_audio_packet_seek.rs`): Lazily certify nondecreasing immutable PTS and partition inclusively,retaining the last equal-PTS sample. Reordered inputs retain original max-PTS search and compact inputs retain existing lookup;no per-sample heap index. Seven paired runs:1000lookups137.178→0.180ms(760.9x),actual software seek-to-PCM137.682→0.908ms(151.6x),all7pairs win. Cold direct/AudioStream seek improve;opening+3.92%passes predeclared10%guard. Five authored120ms clips preserve raw reordered/equal/negative packet landings,ordered analytic PCM,video seek/EOF and actual audio worker behavior.122integration suites570pass;1238distinct checks and both builds pass. Scope is this memory-resident IMA4 workload,not whole-player throughput. See `batch-445-validation.json`.

## Batch 446 — FVID-0446: Refuse unsupported native MP4 audio composition before silent PCM corruption

FVID-0446 (`src/playback/audio/mp4_audio.rs`, `tests/mp4_audio_composition_order.rs`): Cache a selected-track playback-order check before emitting any step,silence or decoded PCM. Exact recovered clock windows avoid container-rounding false refusals. Return typed Unsupported for overlapping/reordered presentation;raw extraction and standalone decoding remain available. Seven new60ms authored video witnesses distinguish ordered/negative/gap playback acceptance from four unsupported-order refusal cases;actual workers emit Error without PCM. Existing timing,quantized clocks,raw packet navigation,seek,EOF and budgets remain green.123integration suites575pass;1243distinct checks and both builds pass. This does not add full reordered-audio playback support;full selected-track validation is documented. See `batch-446-validation.json`.

## Batch 447 — FVID-0447: Honor empty terminal MP4 audio windows without muting rounded native audio

FVID-0447 (`src/playback/audio/mp4_audio.rs`, `tests/mp4_audio_zero_window.rs`): After recovered-clock handling,return Some(0) for an explicitly empty decoded source window. Decode/validate coded bodies and enforce packet limits before PCM clipping. Preserve the positive native window of rounded25Hz AAC durations1,1,0. Eighteen authored short videos cover five codec families,signed origins,known/unknown headers,STTS layouts and grouped blocks;exact/scalar PCM,raw clocks,video,rewind,EOF seek and actual workers pass. Corrupt terminal AAC,oversized empty packet and malformed timing still refuse.124integration suites580pass;1248distinct checks and both builds pass. All variants count once;no performance claim. See `batch-447-validation.json`.

## Batch 448 — FVID-0448: Bound decoded MP4 audio by its declared source windows

FVID-0448 (`src/playback/audio/mp4_audio.rs`, `tests/mp4_audio_partial_window.rs`): Bind decoded non-AAC/changing PCM at the matching source sample endpoint before seek-floor trimming,including terminal and interior positive partial ALAC windows. Reuse the cached ordered packet lookup;preserve exact signed native-frame ceiling,decoder validation and the recovered-clock path. Six short synthetic video controls/witnesses preserve raw timing and coded audio bytes,scalar PCM,rewind,sample seek,EOF and actual worker PCM/PTS. Original8media/oracle files unchanged.125integration suites584pass;1252distinct checks and locked/offline core/player builds pass. Interior baseline2pass2fail becomes green. This extends the same cause and adds no ID;IMA4 already correct. No performance claim. See `batch-448-validation.json`.

## Batch 449 — FVID-0449: Preserve representable wide MP4 audio times and seek stamps

FVID-0449 (`src/playback/audio/mp4_audio.rs`, `tests/mp4_audio_wide_time.rs`): Convert exact time into Duration seconds/remainder without narrowing total nanoseconds. Retain normal seek stamp spelling;use exact reduced source fraction when wide native ticks fit but nanoseconds do not. Five authored tiny VP9/IMA4 movies cover ordinary,boundary,near-signed-limit and actual-overflow cases without huge allocation. Scalar PCM,raw clocks,video,rewind,sample seek,EOF and actual worker PCM/PTS pass;real signed PTS overflow still refuses.126integration suites588pass;1256distinct checks and locked/offline core/player builds pass. All variants and conversion paths count once;no performance claim. See `batch-449-validation.json`.

## Batch 450 — FVID-0450: Compact exact MP4 reorder bounds and accelerate coverage-safe seek

FVID-0450 (`src/playback/video/mp4.rs`, `tests/mp4_reordered_seek.rs`): Store exact32sample prefix/suffix blocks within the same charged budget;bound safe sync searches and recover exact local bounds without an additional cache. Final7paired rounds meet predeclared criteria:43.12ms to0.417ms navigation and70.40ms to26.02ms actual seek+RGB,all7wins;open median improves. Index payload150k packets1,200,000to75,008bytes,ordered0unchanged;all150k exact pictures decode at2MiB. Nine short authored videos verify all landings/suffixes,YUV,native RGB/intervals,signed/equal/reversed/boundary cases,rewind and EOF.127integration suites591pass;1259distinct checks and core/player builds pass. Pre-existing MACE observer-order race repaired with unchanged assertions and counts0. Prototype/variants add no IDs;no whole-player/RSS/GPU claim. See `batch-450-validation.json`.

## Batch 451 — FVID-0451: Make paused MP4 one-frame keyboard navigation exact and terminating

FVID-0451 (`src/player.rs`, `src/player_mp4_frame_step_tests.rs`): Honor keyboard direction,seek immediately before the displayed frame START for backward CFR/VFR/fractional timing,and park after one picture. Handle first-picture and EOF boundaries without playlist advance;prepare paused sound at the shown start for exact explicit resume. Manual steps present ready pictures without waiting for future spool lead or a paused audio clock. Three authored tiny VP9 movies and scalar YUV/PCM oracles verify actual native keyboard/presenter/audio-worker behavior.11cases pass40repetitions;127integration suites591pass,1475distinct checks and both locked/offline builds pass. Five existing ignores unchanged. All variants and candidate amendments count once;no performance claim. See `batch-451-validation.json`.

## Batch 452 — FVID-0452: Install selected audio clock before resuming a queued navigation burst

FVID-0452 (`src/audio_thread.rs`, `src/audio_thread_seek_resume_tests.rs`): Coalesce requested device start until navigation and final controls are installed;a laterPause cancels the pending start while pausing an already running device remains immediate. Two short authored VP9/IMA4/ALAC movies preserve exact continuous/seek-suffix PCM and valid3picture video controls. Seven order/real-thread tests pass20repetitions;all39audio-thread tests,127integration suites591pass,1511distinct checks and both locked/offline builds pass. Seven pre-existing observer failures reproduced on published source and repaired with exact rational clocks/public error observation count0. Five old ignores unchanged;variants count once,no performance claim. See `batch-452-validation.json`.

## Batch 453 — FVID-0453: Hold terminal stop-bound output and park active playback producers

FVID-0453 (`src/player.rs`, `src/player_stop_bound_tests.rs`): Park active sources when the bound is spent and retain the final picture on subsequent repaint. Seeking out of terminal state restores the chosen unpaused transport,with video audio still held for preroll;paused navigation and queue continuation retain their behavior. Three authored CFR/VFR/fractional VP9 clips and scalar WAV/YUV controls verify exact source data,held RGB/intervals,actual worker pause/stable clock and seek-resumed playback/PCM. Six cases pass20repetitions;127integration suites591pass,1514physical-source-distinct checks and core/player builds pass. Introduced seek reentry stalls reproduced4pass2fail and amended without an extra ID. Five old ignores unchanged. Existing bound threshold selection retained;no performance claim. See `batch-453-validation.json`.

## Batch 454 — FVID-0454..0455: Drive stop bounds from the active presentation clock

- **FVID-0454** — Drive stop bounds from the active presentation clock (`src/player.rs`, `src/player_stop_clock_tests.rs`): Use active audio or rate-scaled interval/deadline clock;check before presenting and fence at/after-bound pictures even when A/V slack would make them ready. Preserve explicit seek/manual-step behavior and cleanup terminal loading state. Three authored CFR/VFR/fractional VP9 companions plus scalar WAV/YUV validate exact source data,before/at-cutoff clock decisions and actual remaining-period hold. Bound-only stage passes all3bound tests while independent EOF still fails. Final7cases pass20repetitions;127integration suites591pass,1521physical-source-distinct checks and both builds pass. Variants/control/probes add0;no performance claim.
- **FVID-0455** — Keep native EOF pending until the final picture interval completes (`src/player.rs`, `src/player_stop_clock_tests.rs`): Retain the actual worker EOF and generation until the active presentation clock completes the last picture. Manual stepping still terminates against paused clocks;natural audio tail remains unchanged. Actual native EOF/controlled audio and silent timed VFR witnesses pass. Candidate completed-audio handoff regression reproduced6pass1fail and amended by keeping a completed audio clock authoritative for its audio-paced picture;adds0IDs. Final7cases pass20repetitions;127integration suites591pass,1521physical-source-distinct checks and core/player builds pass. Five old ignores unchanged;no performance claim.

See `batch-454-validation.json`.

## Batch 456 — FVID-0456..0457: Finish failed video navigation and park audio on current decoder refusal

- **FVID-0456** — Finish failed video navigation and park audio on current decoder refusal (`src/player.rs`, `src/player_seek_failure_tests.rs`): Clear pending seek, preview, loading and step state on a current decoder/converter refusal; park video and audio while retaining the specific error and seek-recoverable handles. Teardown-only stage passes four of five tests; stale generation error remains independent.
- **FVID-0457** — Keep queued video refusals attached to their navigation generation (`src/playback/runtime/thread.rs`, `src/player_seek_failure_tests.rs`): Carry generation privately through decoder and converter event queues and discard only errors older than the current request, updating queue accounting for each consumed event. Public Event and Playback interfaces remain compatible. The replacement seek returns exact pixels and a subsequent current-generation bad seek still reports its specific refusal. Five tests pass twenty repetitions;all nine qualification gates pass,1531 physical-source-distinct checks and both builds. Controls and variants add no IDs;no performance claim.

See `batch-456-validation.json`.

## Batch 458 — FVID-0458: Hand completed short-audio clock to continuing video without startup delay

FVID-0458 (`src/player.rs`, `src/player_audio_handoff_tests.rs`): On current healthy audio completion reanchor the silent video deadline from the final delayed device clock and current picture end,including overdue intervals and playback rate. Avoid pending seek/paused clocks. Identical test advances to at least40ms promptly,then completes exact80..100ms final picture/native RGB/EOF without loading. Two tests pass20repetitions;all9gates pass1533physical-source-distinct checks,both builds and unchanged5old ignores. No performance or physical audio/window claim. See `batch-458-validation.json`.

## Batch 459 — FVID-0459..0460: Preserve silent picture phase across live playback-rate changes

- **FVID-0459** — Preserve silent picture phase across live playback-rate changes (`src/player.rs`, `src/player_rate_clock_tests.rs`): Rescale future or overdue silent presentation deadline around the transition instant before installing the new rate. Active audio retains its device-owned clock;preroll and paused controls remain separate. Rate-only repair passes both rate acceptances and source control while independent pause remains failing. Both variants hold remaining picture duration,seek to the actual final frame and reach natural EOF. Final4tests pass20repetitions and all9gates pass1537physical-source-distinct checks plus core/player builds. Variants and repeats add0IDs;no performance claim.
- **FVID-0460** — Retain paused silent picture phase and resume its remaining interval (`src/player.rs`, `src/player_rate_clock_tests.rs`): Retain the silent media position on pause and reconstruct the remaining picture deadline on resume at the chosen rate. A paused rate change leaves the held position unchanged;new media,seek,restart and shown paused/stepped pictures reset retained state appropriately. Identical acceptance checks paused phase stability,remaining-period hold after a paused0.5x change,exact next interval,real seek and natural final EOF. Final4tests pass20repetitions;9gates pass1537physical-source-distinct checks and both builds. Five old ignores unchanged;no physical audio/window or performance claim.

See `batch-459-validation.json`.

## Batch 461 — FVID-0461: Use the active media clock as the origin of relative jump requests

FVID-0461 (`src/player.rs`, `src/player_jump_clock_tests.rs`): Derive relative jump target from presentation_clock,which includes current pending seek and retained paused position. Three forward/back/paused transport variants verify requested timestamp,correct resulting native interval and exact RGB;EOF/zero clamping preserves paused state. Five identical tests pass20repetitions;all9gates pass1542physical-source-distinct checks and both builds. Variants/control/repeats add0IDs. No performance or physical audio/GPU/window claim. See `batch-461-validation.json`.

## Batch 462 — FVID-0462: Preserve requested sub-frame playback phase after seek preroll

FVID-0462 (`src/player.rs`, `src/player_seek_phase_tests.rs`): Retain the requested phase within the returned covering interval when seek preroll finishes. Anchor only the remaining silent picture duration at selected rate and preserve paused target phase for resume. Manual frame steps retain picture-start semantics;active audio retains its device clock. Identical tests verify post-preroll3300ms phase,exact native RGB,remaining700ms scaled hold,next interval,natural EOF,and timely stop3500ms. Five tests pass20repetitions;all9gates pass1547physical-source-distinct checks and both builds. Four phase variants/control/repeats add0IDs;no performance or physical audio/GPU/window claim. See `batch-462-validation.json`.

## Batch 463 — FVID-0463: Advance the silent media clock through missed refreshes without stalling stop bounds

FVID-0463 (`src/player.rs`, `src/player_late_clock_tests.rs`): Extrapolate both future and overdue deadlines in rate-scaled integer media time. Preserve signed overdue phase across paused rate change/resume. Park explicit terminal clock and clamp natural silent EOF;release parked state on source/navigation changes,including manual pictures. Exact native RGB catch-up,stop-before-future-picture,stable terminal/EOF clock,seek recovery,and zero-stop manual navigation pass. Candidate terminal/resume/manual-clock leaks reproduced and amended add0IDs. Five identical tests pass20repetitions;all9gates pass1552physical-source-distinct checks and both builds. Five old ignores unchanged. Additional native-window10s autoplay and three inspected screenshots add0IDs;no performance,physical audio or hardware-codec claim. See `batch-463-validation.json`.

## Batch 464 — FVID-0464: Time A-B marks and repeat boundaries from the active media clock

FVID-0464 (`src/player.rs`, `src/player_ab_clock_tests.rs`): Use active clock for marks and rewind;spend B before awaiting a new frame or closing native EOF,and fence queued pictures starting at/after B. Explicit pause,seek preview,manual steps and earlier hard stop retain priority. Same six cases verify exact native RGB,remaining hold,no out-of-range picture,pause/resume,EOF loop and450ms hard stop. All20repetitions(120case executions) and9gates pass1558physical-source-distinct checks plus both builds;five old ignores unchanged. Variants/repeats add0IDs. Basic native rendering observed;uninterrupted window duration remains an explicitly separate pre-existing close-request limitation reproduced on baseline and candidate,including paused0ms. No performance,physical audio or hardware-codec claim. See `batch-464-validation.json`.

## Batch 465 — FVID-0465: Navigate chapters from the active media phase instead of the future picture end

FVID-0465 (`src/player.rs`, `src/player_chapter_clock_tests.rs`): Use presentation_clock as chapter request origin,retaining pending-seek and paused positions. Seven unchanged tests verify precise target,post-preroll phase,selected native interval and exact RGB,paused state,and actual final-chapter playback to natural EOF. Twenty repetitions pass140case executions. Shared9gates pass1569physical-source-distinct checks and both builds;five old ignores unchanged. Complete existing FVID-0458 output-error clock boundary in the same qualified batch without another ID. Controls,variants and repeats add0IDs;no performance,physical audio/GPU or new full native-window duration claim. See `batch-465-validation.json`.

## Batch 466 — FVID-0466: Keep the media phase monotonic across real input-buffer recovery

FVID-0466 (`src/player.rs`, `src/player_io_clock_tests.rs`): Hold the genuine buffering phase and anchor signed recovery deadlines from it. Preserve paused rate changes,remaining picture hold,seek,stop and natural EOF while retaining brief-delay and prefetched missed-callback catchup. Seven unchanged tests pass20repetitions,140cases. Nine gates pass1576physical-source-distinct checks;five old ignores unchanged. Genuine-starvation freeze correction amends0463 without another ID. No performance or new full native-window duration claim. See `batch-466-validation.json`.

## Batch 467 — FVID-0467: Spend an explicit sound-only stop before issuing an A-B rewind

FVID-0467 (`src/player.rs`, `src/player_sound_stop_loop_tests.rs`): Check explicit stop before the sound-only loop and return once consumed. Actual producer pauses without a seek,terminal phase parks at the requested bound,and subsequent explicit navigation resumes to natural EOF. Five unchanged regressions pass20repetitions,100case executions. All9gates pass1581physical-source-distinct checks and both builds;five old ignores unchanged. No performance,physical audio or new native-window duration claim. See `batch-467-validation.json`.

## Batch 468 — FVID-0468: Resolve pending sound-only transport bounds before closing healthy native EOF

FVID-0468 (`src/player.rs`, `src/player_sound_eof_loop_tests.rs`): Retain healthy EOF ownership when a reached stop or valid A-B rewind is pending. Existing tick_sound resolves stop before loop and defers while paused. Failed devices and unreachable B still complete normally. Eight unchanged regressions verify exact PCM suffix after native A seek and natural subsequent EOF;20repeats pass160case executions. All9gates pass1589physical-source-distinct checks and both builds;five old ignores unchanged. No performance,physical audio or new native-window duration claim. See `batch-468-validation.json`.

## Batch 469 — FVID-0469: Preserve pending native audio seek intent across successive transport commands

FVID-0469 (`src/player.rs`, `src/player_audio_seek_intent_tests.rs`): Retain accepted navigation intent separately from actual device position until the worker commits its latest seek/rewind after successful flush and sample anchoring. Use it only for sound-only navigation/timeline;public position and A/V sync keep actual clocks. Seven unchanged regressions verify requested versus actual phase,exact native PCM suffixes,paused resume,quantized75ms+1ns release and natural EOF. Twenty repeats pass140cases;all9gates pass1596physical-source-distinct checks and both builds with five old ignores unchanged. Existing Worker/mock fixtures receive required private fields only;assertions/bodies are independently verified unchanged. No performance,physical audio or new native-window duration claim. See `batch-469-validation.json`.

## Batch 470 — FVID-0470: Keep the active A-V phase when replacing a native audio track

FVID-0470 (`src/player.rs`, `src/player_track_clock_tests.rs`): Prefer pending seek and logical terminal intent,retain an active unshifted source-audio clock,and convert picture-owned phase back to source time. Sound-only transport keeps its raw clock,including healthy EOF. Twelve unchanged regressions verify native selected PCM prefixes,pause/resume,positive/negative delay,missing sound,terminal bounds in both modes,pending seek,exact video RGB/generation and final-picture seek/EOF. Twenty repeats pass240cases;all9gates pass1608physical-source-distinct checks and both builds with five old ignores unchanged. Private factory delivery is verified behavior-preserving and adds0IDs. Stand repairs,additional cases and unaccepted candidate corrections add0IDs. No performance,physical audio or new native-window duration claim. See `batch-470-validation.json`.

## Batch 471 — FVID-0471: Hold replacement audio until native video buffering and seek preroll finish

FVID-0471 (`src/player.rs`, `src/player_track_preroll_tests.rs`): Include buffering and seek-preview permission when deciding whether to play the new audio handle. Existing presentation readiness releases it once the current-generation picture is ready. Seven unchanged regressions verify acknowledged silence before input release,one resume afterward,native selected PCM prefixes,actual displayed RGB/current generation,final-picture seek and EOF. Twenty repeats pass140cases;all9gates pass1615physical-source-distinct checks and both builds with five old ignores unchanged. Distinct double-marker acknowledgments prevent deferred-Play observation races. Stand guard/pixel/resume observation repairs add0IDs and relax no existing assertions. No performance,physical audio or new native-window duration claim. See `batch-471-validation.json`.

## Batch 472 — FVID-0472: Admit selected audio source before releasing the current live descriptor

FVID-0472 (`src/player.rs`, `src/player_track_source_lifetime_tests.rs`): Open the selected native reader while current source ownership is alive. Admission failure leaves the working handle intact. After successful admission,capture current transport phase,stop/join old output,and install the pinned reader. Metadata/worker installer body is byte-identical after private extraction. Eight unchanged tests verify moved-source forward/backward/paused playback,original-inode exact PCM despite namespace replacement,old-stream continuation on refusal,exclusive device order and native EOF. Twenty repeats pass160cases;all9gates pass1623physical-source-distinct checks and both builds with five old ignores unchanged. Source transaction does not claim rollback after later asynchronous decoder/device initialization failure. No performance,physical audio or new native-window duration claim. See `batch-472-validation.json`.

## Batch 473 — FVID-0473: Keep video time monotonic when a replacement audio track has already ended

FVID-0473 (`src/player.rs`, `src/player_short_track_clock_tests.rs`): Retain the published picture phase and raw source anchor during audio-clock replacement. A shorter endpoint cannot take time backward;real EOF/error hands time to the retained phase. Real catch-up,silent-picture ownership and explicit navigation release the guard. Subsequent track selection preserves the raw intent. Nine unchanged tests verify no fabricated past-end PCM,exact native recovery PCM,paused and silent handoff,775ms stop,backward450ms recovery and natural1500ms video EOF. Twenty repeats pass180cases;all9gates pass1632physical-source-distinct checks and both builds with five old ignores unchanged. No metadata-only EOF shortcut,performance,physical audio or new native-window duration claim. See `batch-473-validation.json`.

## Batch 474 — FVID-0474..0476: Do not let excluded MP4 transport clocks reject selected video concat

- **FVID-0474** — Do not let excluded MP4 transport clocks reject selected video concat (`src/container/mp4_concat.rs`, `tests/mp4_concat_unselected_plan.rs`): Preserve successful transport plans and retained-plan memory accounting,ignore unsupported plans only for excluded tracks,and always propagate cancellation. A private mask rejects selecting an unavailable plan before output. Five unchanged tests verify exact source YUV/AAC PCM,selected-AAC refusal,native80ms video playback and exact RGB/seek/rewind/EOF,public publication and one-packet prefix. Existing successful APIs and parser validation remain. No performance claim.
- **FVID-0475** — Separate audio metadata discovery from active encoded-window quotas (`src/media_probe.rs`, `tests/audio_metadata_selection_window.rs`): Validate original cancellation,zero,RSS and memory controls before a private metadata-only probe. Preserve original encoded byte/work caps for codec/interval admission. Use the helper for standalone PCM metadata,audio-source discovery and generic stream selection. Four unchanged tests verify automatic/explicit trim,WAVE/MKA output,exact frames and bounded finite silence,Opus seek,an exact nonzero PCM concat prefix,and active oversized/zero/resource/cancellation refusals without output. Full-source public probe policy remains unchanged.
- **FVID-0476** — Use configured decoder dispatch when reporting admitted audio concat streams (`src/native_plan.rs`, `tests/audio_concat_plan_dispatch.rs`): Report concat streams through the existing public decoder planner,retaining its codec routing and controls. Three unchanged tests verify G.711 source PCM against an independent scalar oracle,correct codec/disposition,publication of both complete segments across three formats and two memory modes,and invalid-selection/zero/resource refusals. G.711 concatenation retains every exact float sample;Opus/Vorbis retain exact frame counts and bounded finite numerical silence. No new codec or public API signature.

See `batch-474-validation.json`.

## Batch 475 — FVID-0477..0478: Count standalone audio packet work before admitting later concat segments

- **FVID-0477** — Count standalone audio packet work before admitting later concat segments (`src/native_audio_mix/concat.rs`, `tests/audio_concat_packet_work.rs`): Use admitted G711/Ogg/FLAC/MPEG packet indexes to predict bounded work; later segments receive the same byte admission as execution. All five tests pass.
- **FVID-0478** — Count admitted Matroska audio without the legacy export codec gate (`src/native_media.rs`, `tests/audio_concat_container_index.rs`): Share preferred/enabled header track selection for counting while retaining the legacy exporter codec gate. All three tests pass, including complete PCM publication and VP9 seek/EOF controls.

See `batch-475-validation.json`.

## Batch 476 — FVID-0479: Count MP4 edited decode work before admitting later concat segments

FVID-0479 (`src/native_audio_mix/concat.rs`, `tests/mp4_concat_edit_work.rs`): Edited sources count actual work through the admitted bounded MP4 decoder into io::sink, with no progress or spool; unedited sources retain index counting. All six regressions pass, including preroll, checkpoints/repeats, exact exhaustion, complete PCM concat and native video seek/EOF. See `batch-476-validation.json`.

## Batch 477 — FVID-0480: Scope stable immediate MP4 audio packet admission to the edited source prefix

FVID-0480 (`crates/fvid-media/src/owned_mp4_audio_timeline_impl.rs`, `tests/mp4_edit_packet_admission.rs`): Pass the movie clock into shared admission and derive the required source prefix from the same exact integer edit schedule, retaining preroll and repeats. Initial decoder memory remains charged for silence-only edits; delayed/reconfigured/non-native-grid paths retain conservative admission. Six regressions and all eight qualification gates pass. See `batch-477-validation.json`.

## Batch 478 — FVID-0481: Clamp wide indexed MP4 seek requests before signed tick conversion

FVID-0481 (`src/playback/video/native.rs`, `tests/mp4_wide_seek_target.rs`): Bound MP4 Duration requests by the indexed presentation extent before narrowing to signed track ticks. Both APIs select the same final picture/interval as ordinary end seeks and reach EOF. Both regressions and all seven qualification gates pass; WebM delegation and existing decoder/I/O checks remain unchanged. See `batch-478-validation.json`.

## Batch 479 — FVID-0482: Avoid per-sample description scans in controlled compact MP4 planning

FVID-0482 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_uniform_plan_scan.rs`): Compare complete headers and addressable endpoints over sparse runs; retain the reference walk for dense/expanded indexes. Final release median 0.094917ms,94.86percent reduction,7/7 wins;all21 paired plans identical. Full/prefix analytic PCM,duplicate/unused/changed headers,byte/memory refusals,coordinate guards and native video seek/EOF pass. All seven qualification gates pass;no decoder/player speedup claim. See `batch-479-validation.json`.

## Batch 480 — FVID-0483..0484: Anchor unedited MP4 wall-time EOF seeks at the presentation end

- **FVID-0483** — Anchor unedited MP4 wall-time EOF seeks at the presentation end (`src/playback/audio/mp4_audio.rs`, `tests/mp4_audio_eof_seek_clock.rs`): Return the known end marker after wall-time seeking has already clipped the entire unedited presentation, preserving raw packet navigation and decoder validation.
- **FVID-0484** — Preserve exact MP4 audio wall-time seek floors and native anchors (`src/playback/audio/mp4_audio.rs`, `tests/mp4_aac_subtick_seek.rs`): Keep an exact rational PCM clipping floor and native packet-phase clock anchor while preserving existing preroll, raw tick navigation and validation of empty terminal windows.

See `batch-480-validation.json`.

## Batch 483 — FVID-0485: Avoid per-virtual-sample unused-description scans in compact MP4 playback clock setup

FVID-0485 (`src/playback/audio/mp4_clock_recovery.rs`, `tests/mp4_clock_description_scan.rs`): Query readable run starts for sparse UniformIndex,retaining the reference walk for dense/expanded indexes. Paired medians8.23875ms and2.76375ms:83.94/84.79percent reduction,7/7wins in both sets,all fingerprints equal. Full analytic PCM,actual workers,video,seek/EOF,private coordinate semantics,eight gates and three repeated fifteen-test runs pass. No whole-player or decoder speedup claim. See `batch-483-validation.json`.

## Batch 487 — Charge simultaneous index, cue, and frame working sets in owned_subtitle_impl and owned_mp4_video_bridge

FVID-0486 retains `reader_index_bytes` across `owned_subtitle_impl::try_convert_matroska_inner` (`crates/fvid-media/src/owned_subtitle_impl.rs`) and charges `reader_index_bytes + bytes + event_bytes + (cues.len() + 1) * 80 + configuration/HEADER + meta_working_bytes + 16384`, charges per-cue `bytes.len() + normalized/text.len() + configuration/HEADER.len() + event_bytes + (events.len() + 1) * 80 + meta_working_bytes + 16384` in `try_convert_ass_inner` and `try_convert_srt_inner`, and charges `meta_working_bytes` plus simultaneous frame buffers (`pixels.len() * 2` for YUV/RGB or `pixels.len()` for monochrome) and `packet.len()` in `owned_mp4_video_bridge::prepare_selected` (`crates/fvid-media/src/owned_mp4_video_bridge.rs`). Fifty-six integration tests in `mp4_audio_metadata_admission` pass. See `batch-487-validation.json`.

## Batch 488 — Charge simultaneous Matroska metadata, index, frame, and FFV1 packet working sets in owned_xfade, owned_subtitle_burn, owned_lossless, owned_matroska, and owned_ffv1_export

FVID-0487 adds `owned_lossless::options_matroska_metadata_working_bytes` (`crates/fvid-media/src/owned_lossless.rs`), charges `index_bytes + meta_working_bytes` in `check_source_matroska_metadata_budget` and `owned_ffv1_export::export_processed` (`crates/fvid-media/src/owned_ffv1_export.rs`), and charges `meta_working_bytes` together with simultaneous frame buffers, SRT cues/masks, and encoded FFV1 packets across `owned_xfade` (`crates/fvid-media/src/owned_xfade.rs`), `owned_subtitle_burn` (`crates/fvid-media/src/owned_subtitle_burn.rs`), `owned_lossless`, `owned_matroska` (`crates/fvid-media/src/owned_matroska.rs`), and `owned_ffv1_export`. Fifty-seven integration tests in `mp4_audio_metadata_admission` pass. See `batch-488-validation.json`.

## Batch 489 — Charge simultaneous index, metadata, packet schedule, cue, and frame working sets in owned_webm_multitrack_export, owned_mp4_multitrack_export, and owned_subtitle_burn visitors

FVID-0488 charges `index_bytes + meta_working_bytes` together with packet sizes and multitrack packet schedules (including `video.estimated_index_payload_bytes()`) in `owned_webm_multitrack_export` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`) and `owned_mp4_multitrack_export` (`crates/fvid-media/src/owned_mp4_multitrack_export.rs`), and charges `cues_and_meta_bytes` plus simultaneous frame buffers (`view.pixels.len() * 2 + mask_bytes` in container `inspect`/`process` and `frame_len + mask_bytes` in Y4M visitors) and `packet.len()` in `owned_subtitle_burn` (`crates/fvid-media/src/owned_subtitle_burn.rs`). Fifty-eight integration tests in `mp4_audio_metadata_admission` pass. See `batch-489-validation.json`.

## Batch 490 — Charge simultaneous index, presentation table, packet, and frame working sets in owned_video_decode, owned_webm_codec, owned_webm_presentation, and owned_mp4_video_decode

FVID-0489 charges `index_bytes + future_bytes (+ inferred_bytes)` together with packet payload sizes and decoded / transformed frame buffers across `owned_webm_presentation` (`crates/fvid-media/src/owned_webm_presentation.rs`), `owned_webm_codec::presentation_durations` (`crates/fvid-media/src/owned_webm_codec.rs`), and `owned_video_decode::decode_webm` (`crates/fvid-media/src/owned_video_decode.rs`), and charges `table_bytes + packet.len() (+ pixels.len())` in `owned_mp4_video_decode::decode_presented` (`crates/fvid-media/src/owned_mp4_video_decode.rs`). Fifty-nine integration tests in `mp4_audio_metadata_admission` pass. See `batch-490-validation.json`.

## Batch 491 — Charge simultaneous input/output frame buffers and container index working sets in Y4M and Matroska FFV1 lossless export

FVID-0490 exposes `owned_y4m_decode::has_geometry_or_pixel_transform` (`crates/fvid-media/src/owned_y4m_decode.rs`), charges simultaneous `input` + `output` + `encode` frame buffers (`frame_len * 3` when geometry/pixel transforms are active, `frame_len * 2` otherwise) plus `packet.len()` and `meta_working_bytes` in `owned_lossless::{plan_transcode_lossless, transcode_lossless}` (`crates/fvid-media/src/owned_lossless.rs`) and `owned_matroska::write_y4m_ffv1_policy` (`crates/fvid-media/src/owned_matroska.rs`), and includes `index_bytes + meta_working_bytes` in `owned_ffv1_export::export_processed` (`crates/fvid-media/src/owned_ffv1_export.rs`), `owned_lossless::check_encoded_ffv1_with_controlled`, and `owned_subtitle_burn` (`crates/fvid-media/src/owned_subtitle_burn.rs`) container visitors. Sixty integration tests in `mp4_audio_metadata_admission` pass. See `batch-491-validation.json`.

## Batch 492 — Charge simultaneous index, packet, and codec setup working sets in owned_mp4_probe_impl, owned_webm_probe, media_probe, and owned_probe::probe_ogg_with_options

FVID-0491 charges `index_bytes + sample.size` in `owned_mp4_probe_impl::try_mp4_with_options` (`crates/fvid-media/src/owned_mp4_probe_impl.rs`), charges `index_bytes + packet_payload_size + packet_read_overhead` for zlib-compressed Matroska tracks in `owned_webm_probe::try_probe_webm_with_limits` (`crates/fvid-media/src/owned_webm_probe.rs`) and `media_probe::matroska_with_limits` (`src/media_probe.rs`), and charges `index_estimate` plus codec setup and packet timeline working sets (`vorbis_admission::inspect`, `OpusDecoder::decode_admission_bytes`, or `owned_ogg_audio_program::decoder_reserve`) in `owned_probe::probe_ogg_with_options` (`crates/fvid-media/src/owned_probe.rs`). Sixty-one integration tests in `mp4_audio_metadata_admission` and fifteen in `mp4_metadata_admission` pass. See `batch-492-validation.json`.

## Batch 493 — Charge simultaneous display-geometry, alpha-channel transform, and MP4 presentation bridge working sets in owned_video_decode, owned_mp4_video_decode, and owned_mp4_video_bridge

FVID-0492 charges simultaneous source and rotated/cropped buffers after `owned_display_geometry::apply`, charges simultaneous RGB opacity wrapper and output buffers in `transform_alpha` and `owned_video_decode::decode_webm` (`crates/fvid-media/src/owned_video_decode.rs`), charges `table_bytes + packet.len() + frame.data.len() + rotated.data.len()` in `owned_mp4_video_decode::rotate_packed` (`crates/fvid-media/src/owned_mp4_video_decode.rs`), and includes `table_bytes` in `owned_mp4_video_bridge::prepare_selected`'s `base_working_bytes` (`crates/fvid-media/src/owned_mp4_video_bridge.rs`). Sixty-two integration tests in `mp4_audio_metadata_admission` and fifteen in `mp4_metadata_admission` pass. See `batch-493-validation.json`.

## Batch 494 — Charge simultaneous Y4M double-buffered frames and container SRT cues plus source index and metadata in owned_subtitle_burn and owned_ffv1_export

FVID-0493 charges `frame_len * 2 + mask_bytes` alongside `meta_working_bytes` / `cues_and_meta_bytes` across Y4M `prepare`, `plan_burn_subtitles`, and `burn_subtitles` (`crates/fvid-media/src/owned_subtitle_burn.rs`), includes `index_bytes` in container `prepare`'s `base_working`, sums `cues_only_bytes + source_index_and_meta_bytes` for `container_base_bytes`, and passes `cues_only_bytes` into `owned_ffv1_export::export_processed` (`crates/fvid-media/src/owned_ffv1_export.rs`) as `extra_base_bytes`. Sixty-three integration tests in `mp4_audio_metadata_admission` and fifteen in `mp4_metadata_admission` pass. See `batch-494-validation.json`.

## Batch 495 — Charge simultaneous source/stage index, metadata, packet schedule, audio window, and packet payload working sets in owned_webm_multitrack_export and owned_mp4_multitrack_export

FVID-0494 charges `max_packet_bytes` / `bytes.len()` and `pcm_index_bytes` alongside `index_and_meta_bytes + video_index_bytes + schedule_bytes` in `owned_webm_multitrack_export` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`), charges `index_and_meta_bytes + positions_bytes (+ packet.len())` in `owned_webm_audio_window::apply` (`crates/fvid-media/src/owned_webm_audio_window.rs`), deducts `base_working_bytes` and charges `base_working_bytes + data_bytes` in `owned_webm_pcm_window::prepare` (`crates/fvid-media/src/owned_webm_pcm_window.rs`), and charges `mp4_index_bytes` during video bridge planning/export plus `payload.len()` during muxing in `owned_mp4_multitrack_export` (`crates/fvid-media/src/owned_mp4_multitrack_export.rs`). Sixty-four integration tests in `mp4_audio_metadata_admission` and fifteen in `mp4_metadata_admission` pass. See `batch-495-validation.json`.

## Batch 496 — Charge simultaneous Matroska copy index plus packet sizes and MP4 Matroska remux video presentation/duration scratch plus queue working sets in owned_matroska_copy and owned_mp4_remux

FVID-0495 charges `index_bytes + packet.size` against `max_controlled_bytes` in `owned_matroska_copy::inspect_with_packet_and_memory_limits` (`crates/fvid-media/src/owned_matroska_copy_impl.rs`) and includes video `presentation` / `durations` scratch storage plus `BinaryHeap` queue entries in `owned_mp4_remux::validate_selected_prefix_packet_budget` (`crates/fvid-media/src/owned_mp4_remux.rs`). Sixty-five integration tests in `mp4_audio_metadata_admission` and fifteen in `mp4_metadata_admission` pass. See `batch-496-validation.json`.

## Batch 497 — Charge simultaneous container index plus metadata working sets and pre-check WAVE LIST INFO metadata budgets in pcm_matroska and native_export

FVID-0496 charges `reader.estimated_index_payload_bytes()? + 8 * (tag_bytes + 12) + 16384` against `max_controlled_bytes` for MP4 and Matroska sources in `pcm_matroska::extract_source_pcm_metadata_with_interval_packet_and_memory_limit` (`src/native_export/pcm_matroska.rs`), propagates budget and cancellation errors in `pcm_matroska::check_output_memory_budget`, and pre-checks `info_chunks_bytes` via `wave_metadata_working_bytes` before allocating `info_chunks` in `pcm_matroska::extract_source_wave_info` and `native_export::export_pcm_selected` (`src/native_export.rs`). Sixty-six integration tests in `mp4_audio_metadata_admission` pass. See `batch-497-validation.json`.

## Batch 498 — Enforce float32 WAVE qualification, direct WAVE memory admission, and the 64-channel merge ceiling in owned_audio_mix::audio_plan_with_controls

FVID-0497 inspects non-G.711 WAVE inputs directly in `owned_audio_mix::audio_plan_with_controls` (`crates/fvid-media/src/owned_audio_mix.rs`), requires `wave_info.float && wave_info.bits_per_sample == 32`, enforces `size <= max_controlled_bytes` without charging the unused export spool overhead, and rejects `merge && total_out_channels > 64` with `merge-audio output exceeds 64 channels`. Sixty-seven integration tests in `mp4_audio_metadata_admission` pass. See `batch-498-validation.json`.

## Batch 499 — Require explicit speaker mask bit counts to match channel counts in owned_wave_loudness::weights and owned_container_loudness::geometry

FVID-0498 rejects `mask.count_ones() != u32::from(channels)` with `owned loudness speaker positions are not yet qualified` in `owned_wave_loudness::weights` (`crates/fvid-media/src/owned_wave_loudness.rs`) and `owned_container_loudness::geometry` (`crates/fvid-media/src/owned_container_loudness.rs`), preventing extensible WAVE and G.711 WAVE inputs with partial speaker masks (`0 < mask.count_ones() < channels`) from passing `supports` / `plan_loudness` / `plan_loudnorm` and misframing interleaved PCM channels during measurement or dynamic loudness normalization. Sixty-eight integration tests in `mp4_audio_metadata_admission` pass. See `batch-499-validation.json`.

## Batch 500 — Charge the 192 kHz dynamic normalization working set in dual-pass plan_loudnorm and the 192 kHz output report meter in dynamic loudnorm memory admission

FVID-0499 requires `!dual_pass && params.gain.is_some()` before using the single-pass linear memory estimate in `owned_loudnorm::plan_loudnorm` (`crates/fvid-media/src/owned_loudnorm.rs`) and charges `report_memory(192000, resampled_frames, channels)` for `params.print != Print::None` in dynamic and dual-pass `plan_loudnorm` as well as `apply_dynamic`, matching the 192 kHz `output_report` meter allocation. Sixty-nine integration tests in `mp4_audio_metadata_admission` and thirteen `loudnorm` unit tests pass. See `batch-500-validation.json`.

## Batch 501 — Validate container and stream metadata mutations and charge video presentation/duration scratch plus BinaryHeap queue working sets in mp4_concat::open_with_options

FVID-0500 validates container and stream metadata mutation counts, key/value bounds, NUL bytes, selected stream indices, and forbidden display-transform keys in `mp4_concat::open_with_options` (`src/container/mp4_concat.rs`), and charges `track.samples.len() * (size_of::<(i64, usize)>() + size_of::<i128>())` for video tracks plus `selected.len() * size_of::<Reverse<(i128, usize, usize, usize)>>()` in `plan_bytes`. Seventy integration tests in `mp4_audio_metadata_admission` and three `container::mp4_concat` unit tests pass. See `batch-501-validation.json`.

## Batch 502 — Charge the 3.4-second loudness meter working set in container and ADTS plan_loudness and the 192 kHz dynamic normalization working set in container and ADTS plan_loudnorm

FVID-0501 charges the 3.4-second `LoudnessMeter` working set in `owned_container_loudness::plan_loudness` (`crates/fvid-media/src/owned_container_loudness.rs`) and `owned_adts_loudness::plan_loudness` (`crates/fvid-media/src/owned_adts_loudness.rs`), enforces `owned_loudnorm::check_spool_loudnorm_budget` in `owned_container_loudnorm::plan_loudnorm` (`crates/fvid-media/src/owned_container_loudnorm.rs`) and `owned_adts_loudnorm::plan_loudnorm` (`crates/fvid-media/src/owned_adts_loudnorm.rs`), and prefixes `ADTS packet exceeds budget` with `controlled memory budget exceeded:` when `max_controlled_bytes < max_packet_bytes`. Seventy-one integration tests in `mp4_audio_metadata_admission` and fourteen `loudness` unit tests pass. See `batch-502-validation.json`.

## Batch 503 — Charge the 3.4-second LoudnessMeter working set in native_plan::loudness_with_options and propagate max_controlled_bytes to spooled WAVE measurement in native_pcm::measure_file_with_options

FVID-0502 charges the 3.4-second `LoudnessMeter` working set against `options.max_controlled_bytes` in `native_plan::loudness_with_options` (`src/native_plan.rs`) for both WAVE and spooled container/ADTS inputs, and propagates `pcm_options.max_controlled_bytes = options.max_controlled_bytes` when measuring the decoded WAVE spool in `native_pcm::measure_file_with_options` (`src/native_pcm/loudness.rs`). Seventy-two integration tests in `mp4_audio_metadata_admission` pass. See `batch-503-validation.json`.

## Batch 504 — Charge the staged video.mkv Matroska index and encoded FFV1 packet working sets in multitrack lossless plan_selection and try_export

FVID-0503 charges `video_index_bytes` and `max_packet_bytes` (including encoded FFV1 packet lengths via `owned_lossless::encode_ffv1_len_with_controlled`) in `owned_webm_multitrack_export::plan_selection` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`) and `owned_mp4_multitrack_export::plan_selection` (`crates/fvid-media/src/owned_mp4_multitrack_export.rs`), and enforces `schedule_bytes + data.len() <= limit` when reading staged video packets in `owned_mp4_multitrack_export::try_export`. Seventy-three integration tests in `mp4_audio_metadata_admission` pass. See `batch-504-validation.json`.

## Batch 505 — Charge simultaneous Y4mReader YUV/RGB/RawFrame working sets and container index plus packet/filter working sets in native_export Y4M admission

FVID-0504 charges `frame_len + rgb_len + frame_len` (plus `filters.working_bytes(bytes, depth)` when geometry or pixel filters are active) for Y4M inputs in `inspect_first_y4m_frame_with_filters_and_options` (`src/native_export.rs`) and charges `index_bytes + packet_size` and `index_bytes + filters.working_bytes(bytes, depth)` for MP4 and WebM/Matroska inputs in `validate_video_packet_budget_with_geometry_filters_interval_and_limit`. Seventy-four integration tests in `mp4_audio_metadata_admission` pass. See `batch-505-validation.json`.

## Batch 506 — Charge simultaneous filter working set, live source frame, encoder frame copy, and retained non-video stream metadata during lossless FFV1 export and planning

FVID-0505 exposes `extra_working_bytes` on `owned_video_decode::FrameView` (`crates/fvid-media/src/owned_video_decode.rs`), includes `view.extra_working_bytes` in `encode_ffv1_len_with_controlled` (`crates/fvid-media/src/owned_lossless.rs`) and `owned_ffv1_export::export_processed` (`crates/fvid-media/src/owned_ffv1_export.rs`), charges `estimate_transform_working_bytes` alongside the encoder frame copy and encoded FFV1 packet in `owned_lossless::plan_transcode_lossless` and `owned_matroska::write_y4m_ffv1_policy` (`crates/fvid-media/src/owned_matroska.rs`), and passes `extra_base_bytes` from `input.index_and_meta_bytes` into `owned_ffv1_export::export_processed` during Stage 1 of `owned_webm_multitrack_export::try_export` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`). Seventy-five integration tests in `mp4_audio_metadata_admission` pass. See `batch-506-validation.json`.

## Batch 507 — Charge non-WAVE decoded PCM working sets in audio mix/merge planning and simultaneous i32/f32 frame buffers in raw FLAC admission

FVID-0506 updates `total_input_pcm_bytes` and `min_frames` for Ogg, raw FLAC, and raw MP3 inputs in `owned_audio_mix::audio_plan_with_controls` (`crates/fvid-media/src/owned_audio_mix.rs`) so `plan_mix_audio_with_controls` and `plan_merge_audio_with_controls` enforce the combined decoded PCM + mix/merge output buffer budget against `max_controlled_bytes`, and charges `size_of::<i32>() + size_of::<f32>()` per sample in `owned_raw_flac::open_with_interval` and `owned_raw_flac::decode_raw_checked` (`crates/fvid-media/src/owned_raw_flac.rs`). Seventy-six integration tests in `mp4_audio_metadata_admission` pass. See `batch-507-validation.json`.

## Batch 508 — Charge WAVE decode_reader and BufReader/BufWriter I/O working sets in native_export, media::decode_audio, native_loudnorm, and native_audio_mix

FVID-0507 enforces `capacity.max(frame_bytes) + 16384 + sink_bytes <= limit` for WAVE inputs in `native_export::export_pcm_selected` (`src/native_export.rs`), propagates `max_controlled_bytes` on `publish_options` (and preflights `.f32le` spool export before decoding) in `media::decode_fvid_media_audio_to_destination` (`src/media.rs`) and `native_loudnorm::try_apply` (`src/native_loudnorm.rs`), and includes `sources.len() * 8192` in `native_audio_mix` mix/merge `stage_working_bytes` (`src/native_audio_mix.rs`) and `81920` in `validate_concat_segments_with_options` (`src/native_audio_mix/concat.rs`). Seventy-seven integration tests in `mp4_audio_metadata_admission` pass. See `batch-508-validation.json`.

## Batch 509 — Preflight LoudnessMeter, spool loudnorm, and output metadata memory budgets before decoding temporary WAVE spools or running measurement passes

FVID-0508 preflights the 3.4-second `LoudnessMeter` working set (and `weights.len() == channels` for ADTS) before decoding temporary WAVE spools in `owned_container_loudness::measure_loudness` (`crates/fvid-media/src/owned_container_loudness.rs`), `owned_adts_loudness::measure_loudness` (`crates/fvid-media/src/owned_adts_loudness.rs`), and `native_pcm::measure_file_with_options` (`src/native_pcm/loudness.rs`), preflights `check_spool_loudnorm_budget` before decoding in `owned_container_loudnorm::apply` (`crates/fvid-media/src/owned_container_loudnorm.rs`) and `owned_adts_loudnorm::apply` (`crates/fvid-media/src/owned_adts_loudnorm.rs`), and enforces `check_output_memory_budget` before the measurement pass in `native_pcm::normalize_file_with_options` (`src/native_pcm/normalize.rs`) and unconditionally under `max_controlled_bytes` in `native_plan::normalize_loudness_with_options` (`src/native_plan.rs`). Seventy-eight integration tests in `mp4_audio_metadata_admission` pass. See `batch-509-validation.json`.

## Batch 510 — Preflight Stage 2 WAVE loudnorm and compressed audio export DSP/resampler/metadata memory budgets before Pass 1 measurement or Stage 1 spool decoding

FVID-0509 preflights `destination.symlink_metadata().is_ok()` and `plan_loudnorm` before `measure_phase` in `owned_loudnorm::apply_loudnorm` (when `params.print != Print::None`) and `owned_loudnorm::apply_loudnorm_dual` (`crates/fvid-media/src/owned_loudnorm.rs`), adds `check_spool_wave_export_budget` in `owned_adts_export` (`crates/fvid-media/src/owned_adts_export.rs`), and calls it in `owned_audio_plan::plan_decode_audio` (`crates/fvid-media/src/owned_audio_plan.rs`) and before Stage 1 spool decoding across `owned_g711_wave_export`, `owned_raw_flac`, `owned_raw_mp3`, `owned_ogg_opus_export`, `owned_ogg_vorbis_export`, `owned_ogg_joint_export`, `owned_mp4_audio_export`, and `owned_matroska_{pcm,aac,opus,flac,mp3,alac}_export`. Seventy-nine integration tests in `mp4_audio_metadata_admission` pass. See `batch-510-validation.json`.

## Batch 511 — Preserve native seek anchors across occupied edit boundaries

FVID-0510 reapplies each selected edit segment minimum occupied-sample start after native-sample rounding crosses a segment boundary. Three authored AAC+VP9 movies reproduce correct PCM with an early clock anchor before the fix;direct navigation and actual AudioWorker flush anchors now match the first available native sample. Seven merged-state gates pass975 test executions,with two prior ignored tests,and three repeated33-test runs pass. The ADTS initial transport-admission integration repair restores two unchanged unit tests and adds zero IDs. Parallel entries through FVID-0509 are preserved. See `batch-511-validation.json` and `evidence/mp4-edit-frontier-seek`.

## Batch 512 — Bound occupied final edit seek frontiers at declared EOF

FVID-0511 prevents exact and raw MP4 seek clocks from escaping the declared presentation endpoint when a preceding native sample occupies every remaining edit. Six authored AAC/PCM+VP9 movies retain full independent PCM/video while direct and actual-worker EOF anchors now stay bounded. Raw navigation keeps its media-tick contract. Seven gates pass980 test executions,two prior ignored tests,and three repeated38-test runs pass. See `batch-512-validation.json` and `evidence/mp4-edit-frontier-eof-seek`.

## Batch 513 — Unedited native PCM seek precision,zero new IDs

FVID-0484 now preserves exact Duration floors for unedited native PCM,including compact indexes and changes in rate,channels and storage format. Eleven authored movies retain independent full PCM/video,precise suffixes,raw navigation and actual-worker anchors. The fractional Player test checks native sample601 instead of the previously repeated sample600;the original source fails that stricter expectation,and all other player assertions remain. Seven gates pass985 executions,two prior ignores,and three repeated43-test runs pass. Counter511,next FVID-0512. See `batch-513-validation.json`.

## Batch 514 — Unedited ALAC seek precision,zero new IDs

FVID-0484 now retains exact Duration floors for unedited ALAC and computes its native anchor from the actual coded sample-count header. Nine valid authored raw/Rice/media-clock/reordered/fill movies preserve full PCM/video and raw navigation;the separate truncated-body movie still refuses EOF seeks after its valid header. Seven gates pass990 executions,two prior ignores,and three repeated48-test runs pass. No performance claim. Counter511,next FVID-0512. See `batch-514-validation.json`.

## Batch 515 — Edited ALAC native seek precision,zero new IDs

FVID-0484 now retains exact Duration floors in pure ALAC edit timelines. Fifteen authored raw/Rice/reordered/fill/changing-rate movies verify bit-exact PCM/video,leading silence,repeated ranges and actual-worker anchors. A malformed active packet still refuses playback after seek. Seven gates pass995 executions,two prior ignores,and three repeated53-test runs pass. Counter511,next FVID-0512. See `batch-515-validation.json`.

## Batch 516 — ADPCM native seek precision,zero new IDs

FVID-0484 now retains exact Duration floors in pure ADPCM timelines. Sixteen authored movies verify bit-exact PCM/video,grouped blocks,rate changes,silence,repeated edits and actual-worker anchors. Invalid active MS predictors still refuse. Seven gates pass1000 test executions,two old ignores,and three repeated58-test runs pass. Counter511,next FVID-0512. See `batch-516-validation.json`.

## Batch 517 — MACE native seek precision,zero new IDs

FVID-0484 now retains precise Duration floors in unedited pure MACE3/MACE6 tracks without losing decoder history. Ten authored mono/stereo,grouped,changing-description and fragmented/reordered/hybrid movies compare exact PCM suffixes and native timestamps with full natural playback;actual-worker anchors and raw reseeks pass. Seven gates pass1004 test executions,two old ignores;16 additional MACE subsystem tests pass,and three repeated78-test runs pass. Counter511,next FVID-0512. See `batch-517-validation.json`.

## Batch 518 — Edited MACE native seek precision,zero new IDs

FVID-0484 now preserves exact Duration floors in pure MACE edit timelines,including initial/intermediate silence and repeated source ranges. Eight authored movies compare every native timestamp and PCM byte with independently sliced full natural playback;raw reseek and actual-worker anchors pass. Seven gates include all old MACE suites and pass1024 test executions,two prior ignores;three repeated82-test runs pass. Counter511,next FVID-0512. See `batch-518-validation.json`.

## Batch 519 — Constant ADPCM/AC3 edit seek precision,zero new IDs

FVID-0484 now preserves exact Duration floors for constant AudioTimeline cursors. Six authored ADPCM/AC3 cases reproduce the failure;two EAC3 rates are passing controls. All eight verify natural PCM/video,silence,repeats,exact native suffixes,raw clock restoration and actual-worker anchors. Seven gates include19 older ADPCM/Dolby tests and pass1047 test executions,two prior ignores;three repeated105-test runs pass. One independent strict packet-stamp test fails before,passes after and repeats three times,including48k native cursors. Counter511,next FVID-0512. See `batch-519-validation.json`.

## Batch 520 — Mixed PCM/MACE/ADPCM native seek precision,zero new IDs

FVID-0484 now preserves exact Duration floors across supported PCM/MACE/ADPCM description mixtures. Sixteen authored classic/reordered/fragmented/hybrid files cover normal playback,silence/repeated edits,native PCM suffixes/actual packet timestamps,raw reseek and actual-worker anchors. Seven gates pass1061 test executions,two prior ignores;three repeated119-test runs pass. Counter511,next FVID-0512. See `batch-520-validation.json`.

## Batch 521 — Exact native packet stamps and mixed coded seek

FVID-0512 preserves representable rational native PCM timestamps after raw media-tick clipping. A pure44.1k fixture proves the right waveform had the wrong native sample label,without any Duration request;the isolated stamp correction passes its regression while FVID-0484 mixed routing still fails. A separate zero-count extension then corrects AAC/ALAC/PCM routing and packet-specific ALAC counts. Sixteen mixed movies preserve rational timestamps,PCM,history,silence/repeated edits and actual-worker anchors. Seven gates pass1068 test executions,two prior ignores;three repeated126-test runs pass. Counter512,next FVID-0513. See `batch-521-validation.json`.

## Batch 522 — Aligned-clock mixed Dolby native seek,zero new IDs

Zero-count FVID-0484 extension for aligned-clock mixed PCM/AC3/EAC3. Per-packet native counts include selected EAC3 programmes and grouped AC3;fresh seeks derive actual Dolby rates before decoder negotiation rather than trusting placeholder metadata. Nineteen authored classic/reordered/fragmented/hybrid movies,with/without silence and repeated edits,verify complete PCM/video,independent reset-control PCM/native clocks,exact rational suffixes,raw reseek,fresh reader/worker seeks and actual device anchors. Baseline full controls pass;precise/fresh/worker seeks fail before and pass unchanged after. Missing programme and full-packet budget refusal are distinct from successful CRC concealment. Seven gates pass1078 executions,two prior ignores;the final10-test stand passes focused,in integration,and in a final repeat. No new ID,performance,physical audio device or visual-player claim. Counter512,next FVID-0513. See `batch-522-validation.json`.

## Batch 523 — First seek into concealed Dolby packets,zero new IDs

Zero-count correction of batch522/FVID-0484. A first seek into a CRC-bad middle EAC3 packet previously rejected a stream whose complete playback and warm seek correctly concealed that packet. Native-rate selection now shares checked description setup/cache with ordinary configuration without activating the future decoder. Six authored classic/fragmented/hybrid movies at48k programme0 and44.1k programme1 with48k metadata hint compare independent controls,zero-concealed middle samples,nonzero neighbors,exact rational timestamps/PCM,warm/raw/fresh seeks and actual-worker anchors. Baseline two controls pass,two fresh seek tests fail specifically on native-rate probing;all four unchanged tests and ten older mixed Dolby tests pass after. Seven gates pass1082 test executions,two old ignores;three four-test repeats pass with stable source. No new ID,performance,physical-device or visual-player claim. Counter512,next FVID-0513. See `batch-523-validation.json`.

## Batch 524 — QuickTime G.711 native playback and export

One compatibility capability:existing Rust G711 companders now decode QuickTime alaw/ulaw through parsed sample tables,player/native descriptors,precise native-clock seek,owned float32/float64 intervals and public float64 WAVE export. Forty-eight authored v1/compressionID-2 movies cover8k mono,48/44.1k stereo,dense/uniform/fragmented/hybrid tables,silence/repeated edits,all256 codes and neutral VP9. Native counts use one coded byte per channel rather than uncompressed sampleSize16. Independent PCM/WAVE controls,exact rational timestamps/PCM,EOF/MAX/raw reset,actual workers,compact record budgets,orphan-channel refusal and full packet admission pass. Primary QuickTime references and initial before failures are preserved. A separate zero-count FVID-0193 correction restores standards-valid AIFC original-width metadata;the unchanged old G711 test failed identically on original source before that correction. Nine gates pass1114 Rust executions,four Python units,two old ignores;three15-test repeats pass with2399 stable source/fixture hashes. No speedup,physical-device or visual-player claim. Counter513,next FVID-0514. See `batch-524-validation.json`.

## Batch 525 — FVID-0514: AIFF storage padding

Explicitly block-aligned PCM/G711 AIFF/AIFC now excludes SSND tail padding from sound and seek duration. Original worker reproduces extra audio;ten authored variants,strict refusal controls,81 test executions,two build checks and three9-test repeats pass. Full stored bytes remain charged. Evidence:`evidence/aiff-block-padding/README.md`. Accepted total:514/1000.

## Batch 526 — FVID-0515: AIFF integer precision

AIFF/AIFC signed PCM now supports every valid1..32-bit logical precision with whole-byte per-channel storage and strict zero-padding validation.192 authored variants and168 actual-worker captures pass;seven gates cover146 test executions,two build checks and three14-test repeats. A pre-existing CLI plan-progress test is corrected with stronger acceptance/refusal checks and zero extra IDs. Evidence:`evidence/aiff-integer-precision/README.md`. Accepted total:515/1000.

## Batch 527 — FVID-0516: indexed audio EOF

Eight public payload adapters now return empty beyond their declared packet count instead of repeating the final packet/XM tail. All valid bytes and actual-worker playback are preserved. PCM and coded/tracker originals reproduce the exact EOF failures;14 gates pass826 executions,one prior ignore,two build checks and three24-test repeats. Both new suites pass without FFmpeg. A pre-existing WAVE metadata size expectation is corrected with stronger checks and zero extra IDs. Migration:`evidence/audio-index-eof/API-MIGRATION.md`;evidence:`evidence/audio-index-eof/README.md`. Accepted total516/1000.

## Batch 528 — FVID-0517: bare FLAC precise time seek

Duration navigation now presents the first native FLAC sample at/after the request while preserving raw packet seek behavior. Six independent rate/channel files verify PCM,clocks,fractional/frame boundaries,EOF/MAX,mode changes and actual workers. Five gates pass94 executions,two build checks and three13-test repeats;the new suite passes without FFmpeg. Evidence:`evidence/flac-sample-seek/README.md`. Accepted total517/1000.

## Batch 529 — FVID-0518: ordered raw Dolby window lookup

Linear per-packet frame-index scanning becomes logarithmic without an extra heap index. Two five-pair benchmark runs preserve result checksums and improve65536-record lookup at least267x;this is a helper measurement,not player throughput. Tiny indexes have subnanosecond median overhead.87 test executions,zero ignores,two build checks and three10-test repeats pass with2752 frozen hashes. Existing authored Dolby fixtures and actual-worker controls remain intact. Evidence:`evidence/ac3-window-lookup/README.md`. Accepted total518/1000.

## Batch 530 — FVID-0519: signed unspecified MP4 subtitle endpoints

Infer successor/fallback endpoints before clipping signed media time,preventing phantom pre-origin subtitles and overlong negative final lines.16 authored short videos pass exact cues,packets,pictures and real-video-seek lookup. Six gates pass83 executions(76 distinct checks),zero ignores,both builds;three4-test repeats and4 no-FFmpeg executions pass.20 fixture files regenerate identically with2778 frozen hashes. Initial invalid authoring refusal is retained separately and not counted. Evidence:`evidence/mp4-subtitle-inferred-clock/README.md`. Accepted total519/1000.

## Batch 531 — FVID-0520: bounded AVI G.711 packet buffers

G.711 uses the existing2048-frame PCM window mechanism. Three baseline/four final candidate observations reduce simultaneous encoded+decoded packet Vec capacities from40965/81930 to10240/20480 bytes for8193-frame mono/stereo chunks;small controls remain unchanged. This is about75% lower packet capacity,not whole-reader memory or throughput,and entails more packet calls.20 authored AVI/independent PCM controls preserve waveform,clocks,seek and actual workers. Five final gates pass106 executions,zero ignores,both builds,three4-test repeats and4 no-FFmpeg executions.42 fixture files regenerate identically;2825 hashes stay stable. Incidental formatter-only helper changes were restored and all gates rerun. Evidence/packet-boundary note:`evidence/avi-g711-windows/README.md`. Accepted total520/1000.

## Batch 532 — FVID-0521: ordered AVI native seek selection

Binary search over existing record ends replaces linear scanning for PCM/G.711. Two independently repeated comparisons of preserved executables measure the actual public method in the default dev build:minimum243x at65536 records,94x at16384 irregular records,and8.31x at1024. Small16-record controls add7..14ns median cost;parsing,I/O,decoding,release and whole-player timing are outside the claim.15 short authored AVI/float controls preserve exact PCM,native clocks,seek/EOF and actual workers. Five gates pass109 executions,zero ignores,both builds,three3-test repeats and3 no-FFmpeg executions.32 files regenerate identically;2860 source/fixture hashes stay stable. Evidence:`evidence/avi-seek-index/README.md`. Accepted total521/1000.

## Batch 533 — FVID-0522: native Ogg stereo-device negotiation

Supply the existing speaker-aware stereo plan, permuting Vorbis coefficients into emitted native order and forwarding catalogue routes. Preserve native PCM geometry, clocks and decoder behavior. Twenty-three authored reader cases cover mono through eight speaker channels, discrete nine-channel Opus, multiplex selection and same/mixed-codec six-to-seven-channel chains. Forty-four direct/catalogue routes and176 actual-worker sessions verify exact stereo PCM, native timestamps,seek0/30/150ms and EOF/MAX; actual CPAL ring/callback output matches an independent speaker oracle. Seven frozen-source gates pass168 Rust executions with two pre-existing physical-device ignores,core/player checks,three3-test repeats and four tests without FFmpeg. All27 fixture files regenerate identically and2890 source/fixture hashes remain stable. One device-negotiation defect across adapters; no physical-device,performance or new public-signature claim. Accepted total522/1000. See `batch-533-validation.json` and `evidence/ogg-player-stereo/README.md`.

## Batch 534 — FVID-0523/0524: WAVE speakers and bare native-time navigation

Validated WAVE_EXTENSIBLE masks now reach device routing; classic headers retain unspecified roles. Separately,WAVE/AIFF/AU Duration seeks select the first native sample at/after time while preserving raw coordinates and predictive history. Identical original/mask-only/precise-only/full variants pass1/2/4/6 tests,proving independent results. Sixteen WAVE,three AIFF/AU and12 prior authored predictive sources cover exact PCM/clocks,seek/EOF,refusal and143 actual-worker sessions. Nine final gates pass178 executions,two prior hardware ignores,both builds,three6-test repeats and6 no-FFmpeg executions.42 files regenerate identically;2948 hashes stay stable. Duration migration and physical-device limits are documented in `evidence/wave-speaker-playback/README.md`. Accepted total524/1000. See `batch-534-validation.json`.

## Batch 535 — FVID-0525: extensible PCM audio in AVI

Existing WAVE GUID/precision/speaker validation now admits PCM and float AVI strf extensions.22 authored AVI/WAVE pairs prove exact native PCM/clocks,masks,bounded packet windows,seek/EOF/MAX,rewind,budgets and88 actual-worker stereo sessions. Unsupported GUIDs remain skippable,including before valid audio; malformed extensions specifically refuse. Nine final gates pass204 executions,two prior hardware ignores,both builds,three4-test repeats and4 no-FFmpeg executions.75 files regenerate identically;3025 hashes stay stable. No external signature,decoder,performance,physical-device or AVI video-playback claim. Accepted total525/1000. See `batch-535-validation.json` and `evidence/avi-extensible-pcm/README.md`.

## Batch 536 — FVID-0208 native padding refusal extension,zero new IDs

Zero-count batch536 extends the already counted FVID-0208 nonzero-padding refusal to native WAVE/AVI playback. Original f0cfb6c714 accepts and fully decodes36 authored unused-bit violations,including first,window-boundary and final samples;valid PCM/video and72 actual-worker seek/EOF controls already pass. Retain the admitted padding count internally and validate reduced-precision little-endian integer PCM before any packet can be emitted,using the same helper for RIFF sample runs and AVI physical records. Full-width integer and float paths retain their existing behavior;full-width controls with identical nonzero low bits remain accepted. Six8/16/24/32-bit precision profiles include1-bit precision and unsigned8;12 good declarations each have exact independent float PCM controls and both containers. Nine frozen gates pass212 executions,two old physical-device ignores,core/player builds,three3-test repeats and3 no-FFmpeg executions. All77 files regenerate identically and3104 hashes remain stable. Reduced-precision input now incurs a full admission scan;no performance or physical-device claim. New regression tests/pcm_padding_playback.rs and batch-536-validation.json preserve the evidence;no new ID.

## Batch 537 — FVID-0526: native classic RIFX audio

Read RIFX chunk lengths,fact counts and classic format scalars in big endian while retaining raw packet payload bytes. Reuse validated classic WAVE geometry,mark integer PCM big endian,and supply existing fl32/fl64 plus endian metadata for IEEE floats. Add the backward-compatible Wav::is_big_endian getter for raw-parser consumers. Sixteen mono/stereo8/44.1/48kHz cases cover unsigned8,signed16/24/32,float32/64,A-law/mu-law,integer extrema,signed float zero,odd JUNK padding and fact metadata. Exact independent PCM/raw bytes/native clocks,cold precise seek/rewind/EOF/MAX,64 actual-worker sessions and exact truncated-prefix EOF failure pass. Unknown codings and RIFX extensible headers remain explicit Unsupported refusals;wrong alignment/byte rate are Invalid. Original RIFF PCM fact-hint compatibility is preserved and explicitly checked on original source;an incorrect initial new-test refusal expectation is archived and not counted as a product defect. Nine frozen gates pass195 Rust executions,two old physical-device ignores,core/player checks,three5-test repeats and5 no-FFmpeg executions. All77 files regenerate identically and3185 hashes stay stable. Independent libsndfile matches all16 files;FFmpeg9.0.2 diagnostic mismatches/unsupported cases are retained and not used to weaken the oracle. One native-player format capability;owned export/loudness RIFX support,physical devices and performance remain unqualified. Accepted total526/1000. See `batch-537-validation.json` and `evidence/rifx-playback/README.md`.

## Batch 538 — FVID-0527: bounded RF64/ds64 playback

Admit RF64 with mandatory first ds64,checked64-bit RIFF/data extents,and ordered per-ID consumption of optional size-table entries. Finite32-bit fields override unused64-bit values;fact markers and absent fact resolve ds64 sample metadata. Validate truncated ds64/table,duplicate descriptors,missing overrides,overflow and caller limits before payload reservation. Parse only the declared RF64 envelope;EOF checks retain missing-tail errors even when PCM is complete. Resolved data size4294967295 remains a real RF64 size rather than the classic unknown-length sentinel. Seventy authored cases cover scalar/extensible PCM,IEEE float,G711 and eight prior invented predictive codecs,base/table/finite/no-fact forms,repeated IDs and trailing metadata. Exact native PCM/packet bytes,clocks/masks,seek/rewind/EOF/MAX and280 actual-worker sessions pass. Two tiny truncated fixtures declare4294967295/4294967296 data bytes:default limits reject them,explicit large limits expose the exact4097-frame prefix but require EOF errors. A64-bit private unit proves retained sample capacity<=8192 and encoded packet capacity<=4096 bytes;not a peak-RSS or full4GiB playback qualification. Nine frozen gates pass201 Rust executions,two old physical-device ignores,core/player checks,three5-test repeats and5 no-FFmpeg executions. All136 files regenerate identically and3325 hashes stay stable. Independent FFmpeg matches20/20 base signals;libsndfile diagnostics are retained separately. One bounded native format capability,not an ID per guard/variant. Public signatures and in-memory architecture remain unchanged;owned export/loudness RF64,physical devices,streaming and performance are unqualified. Accepted total527/1000. See `batch-538-validation.json` and `evidence/rf64-playback/README.md`.

## Batch 539 — FVID-0528: Play bounded Sony Wave64 streams natively with complete GUID and extent checks

FVID-0528 (`src/playback/audio/wav_wave64.rs`, `tests/wave64_playback.rs`): Full GUIDs, header-inclusive 64-bit sizes and eight-byte alignment feed shared WAVE admission without payload rewriting. Exact PCM/packets/native seek and EOF pass for 32 cases, 128 worker sessions; invalid extents refuse specifically, large declared payloads retain only available samples and fail EOF. Player factory accepts w64 and case-insensitive paths. See `batch-539-validation.json`.

## Batch 540 — FVID-0529: Play flat PCM BW64 while ignoring reserved ds64 dummy words

FVID-0529 (`src/playback/audio/wav_rf64.rs`, `tests/bw64_playback.rs`): BW64 shares checked size resolution and existing WAVE sample admission, with optional RF64-only sample counts. Reserved dummy words and RF64-only fact replacement do not influence BW64 timing. All 64 PCM cases preserve exact samples, packets, seek and EOF; 256 playback-worker sessions and native factory pass. Specific malformed metadata refusals and bounded large-declaration prefix/EOF checks pass. See `batch-540-validation.json`.

## Batch 541 — FVID-0530: Play IMA ADPCM WAVE with whole-block scheduling and exact fact-tail limits

FVID-0530 (`src/playback/audio/wav.rs`, `tests/wave_ima_playback.rs`): Validate mono/stereo four-bit WAVEFORMATEX block geometry and samples-per-block, route complete blocks to the existing owned decoder, bound positive fact tails and retain exact native clocks/seek. Thirty-six RIFF/RF64/Wave64 cases include blocks exceeding 2048 decoded frames, trimmed final blocks and absent fact; 144 playback-worker sessions and native player factory pass. Specific metadata and corrupt-step refusals, exact truncated prefixes and unknown-data/fact EOF checks pass. Final frozen gates pass243 Rust executions plus18 no-external-tool repeats, two pre-existing physical-device ignores, and core/player checks. All89 files regenerate identically and3636 source/fixture hashes remain stable. FFmpeg matches24/24 full/absent-fact cases and libsndfile8/8 RIFF controls; padded-tail disagreements are retained explicitly and never weaken analytic fact expectations. Public Coding enum migration is documented. See `batch-541-validation.json`.

## Batch 542 — FVID-0531: Play Microsoft ADPCM WAVE with retained coefficient setup and exact block clocks

FVID-0531 (`src/playback/audio/wav_ms_adpcm.rs`, `tests/wave_ms_adpcm_playback.rs`): Validate four-bit mono/stereo ADPCMWAVEFORMAT, block/sample geometry, standard coefficient prefix and bounded7..256-entry table; retain the exact interpreted setup for the existing decoder. Seventy-three RIFF/RF64/Wave64 cases preserve analytic PCM, encoded bytes, setup, clocks, native seek/rewind/EOF and positive fact tails, including long blocks and predictor255. Metadata refusals, decoder-specific bad predictor/delta errors, exact truncated prefixes, unknown-data/fact EOF and292 real-worker sessions pass. Preserve Unsupported EXTENSIBLE MS routing so later AVI PCM remains playable. Seventeen frozen gates pass254 Rust executions, two old device ignores, core/player checks and22 no-external-tool executions. All199 files regenerate identically;3840 source/fixture hashes remain stable. FFmpeg matches24 standard complete/absent-fact cases; fact-tail/extra-predictor and libsndfile limits remain explicit diagnostics rather than weakened oracles. The public Coding::MsAdpcm addition has documented downstream match migration. No performance or physical-device claim. See `batch-542-validation.json`.

## Batch 543 — FVID-0532: Seek independent WAVE ADPCM blocks without decoding unrelated history

FVID-0532 (`src/playback/audio/wav.rs`, `tests/wave_block_seek.rs`): Select the packet window and complete IMA/MS coded block containing the native target; scale skipped blocks into native packet PTS and preserve intra-block presentation floors. Seventy-three RIFF/RF64/Wave64 cases verify exact native suffixes, raw/warm seeks, EOF/MAX, rewind and146 actual-worker sessions. Selected corrupt headers still refuse; unrelated skipped payload does not block valid random access. Other stateful codings retain full history. Eighteen frozen gates pass260 Rust executions plus18 no-external-tool repeats, two old physical-device ignores and core/player/example builds. All370 short fixture files regenerate identically and4215 source/fixture hashes remain stable. Same-source benchmark binaries and hashed temporary120-second inputs produce288 PCM/clock-verified ABBA measurements: late seek decodes28–36 instead of960000–960003 frames, with pooled dev-build medians96.840–252.785ms before and4.230–6.792us after. Start-of-file controls stay within the recorded1.25 median-ratio gate (observed0.974–1.027). These are already-open RAM input/CPU-path measurements, not release, cold-I/O or physical-player latency claims. One navigation improvement, no extra ID for its measured performance benefit. See `batch-543-validation.json`.

## Batch 544 — FVID-0533: Play delayed AVI ADPCM streams on their absolute native sample timeline

FVID-0533 (`src/playback/audio/avi_audio.rs`, `tests/avi_adpcm_start.rs`): Convert AVI stream start units through the already-validated block clock into a checked native sample origin. Offset packets, seek landing and absolute endpoint duration/EOF; subtract the origin for block selection and clamp earlier targets to the first block. Preserve PCM/G711 record spans, coded bytes, index checks and limits. The 144 authored mono/stereo, original/equivalent-clock, single/grouped/nested/indexed cases cover starts0/1/73/u32MAX, literal PCM, exact native clocks, raw seek/rewind/EOF/MAX and432 actual software-worker sessions per regression run. Typed specific malformed refusals remain strict. Private storage assertions retain only three logical blocks and a small coded file for the maximum start. The native player factory admits18 delayed equivalent-clock controls with exact PCM. Final qualification passes157 Rust executions, two existing physical-device ignores and core/player checks; three4-test repeats run without external tools. All175 files regenerate identically and4392 source/fixture hashes remain stable. One format/timeline capability; no extra IDs for variants, tests or resource assertions. No physical-output, AVI-video, performance or RSS claim. See `batch-544-validation.json`.

## Batch 545 — FVID-0534: Play packed QuickTime version-2 LPCM with native high sample rates

FVID-0534 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_lpcm_v2.rs`): Validate the version2 structure extent, marker, rate, channels, flags and constant packet geometry, then normalize supported packed/interleaved signed8/16/24/32, unsigned8 and float32/64 descriptions to existing PCM dispatch without rewriting payloads. Explicit integral rates avoid the legacy16.16 restriction. The112 authored48/96kHz mono/stereo cases cover both byte orders and one/three-frame packets, exact PCM/native clocks, precise seek/rewind/EOF/MAX and336 actual software-worker sessions per stand. Owned full/interval f32/f64 exports preserve low float64 bits;28 legacy controls remain valid. Six malformed headers refuse with typed specific Invalid errors; planar/unpacked/unknown layouts remain skippable. Qualification passes156 Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats. All262 fixture files regenerate identically and4656 source hashes remain stable; independent FFmpeg matches224/224 scalar outputs. Newly admitted Track/SampleDescription codecs normalize to existing PCM identifiers, documented in evidence; no change to existing interfaces or formats. One format capability, no separate IDs for its variants, tests or clock width. No physical-device, broad LPCM-layout, performance or RSS claim. See `batch-545-validation.json`.

## Batch 546 — FVID-0535: Validate constant MP4 streams against their selected decoder header instead of catalog entry one

FVID-0535 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_constant_selected_description.rs`): Retain temporary per-description parser recognition through fragment indexing, publish the initial selected recognized header in Track metadata, and compare used descriptions to that representative header in the constant guard. Preserve the complete catalog, packet bytes and indexes; never promote opaque versions by FourCC or placeholder geometry. Initialize the now-admitted constant audio fast path from the first sample description rather than slot0. Fourteen positive variants and five unsupported-selection controls cover compact64-frame PCM, six-fragment120ms PCM and three distinct VP9 frames, unused supported/opaque/known-FourCC-unknown-version descriptions, root and owned APIs, exact geometry/configuration/bytes, PCM duration, rewind/seek/EOF,30 actual software-worker sessions and exact software-video seek suffixes. Genuine three-setup PCM remains refused by both constant APIs. Qualification passes170 Rust executions, two old device ignores, core/player checks and three external-tool-free5-test repeats. All28 generated files are byte-identical and4687 source/input hashes remain stable. An incomplete candidate fast-path failure and an authoring error are retained separately and not counted. One selected-header invariant with both rejection and admission consequences; no separate IDs for variants or compatibility fixes. Public signatures/struct fields remain unchanged; indexed consumers still select per packet. No performance, RSS, physical-device or hardware-video claim. See `batch-546-validation.json`.

## Batch 547 — FVID-0536: Preserve compact PCM and MACE index admission behind dormant compressed descriptions

FVID-0536 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_unused_index_geometry.rs`): Resolve the primary codec/channels and MACE-unit changes from STSC-selected descriptions before choosing the index representation. Preserve ordinary chunk-order/coverage/divisibility/extent/overlap checks and compact-size validation; refuse zero-byte units before publishing a uniform index. Forty authored variants cover mono PCM64/2048 frames, MACE3/6 with72 decoded samples and dormant ALAC/IMA4/MACE/float entries in either order. Root/owned APIs retain one audio chunk under the original limit4, exact logical units, every coded packet and raw clock; native PCM/MACE, owned PCM export, precise seek/rewind/EOF/MAX and120 actual software-worker sessions per stand pass. Four malformed controls keep specific typed refusals, exhausted budgets do not admit audio, and two positive-CTTS companions remain expanded and cannot bypass limits. Qualification passes178 Rust executions, two old device ignores, core/player checks and three external-tool-free repeats. All53 generated files are byte-identical and4742 source/input hashes remain stable. Three same-core-profile before/after metric repeats preserve packet checks:32 retained-index costs fall12/64/2048 to1 and eight remain unchanged; largest reported whole-reader index payload estimate84536 to2648 bytes. This supports a functional budget-admission fix, not a separate optimization ID or an RSS/speed claim. Distinct from FVID-0535 representative-header selection: this failure occurs while constructing the index. No public signature/struct change, physical-device or hardware-video claim. See `batch-547-validation.json`.

## Batch 548 — FVID-0537: Use physical QuickTime float width for native frame clocks and seek while retaining declared metadata

FVID-0537 (`crates/fvid-media/src/owned_mp4_audio_packet_frames.rs`, `tests/mp4_float_legacy_width.rs`): Add a shared physical PCM channel-frame byte helper: fl32/fl64 use32/64-bit storage and other PCM retains its resolved width. Use it in coded-frame counting and both live PCM seek/frontier calculations. Preserve raw Track/SampleDescription/AudioStream bits and the unchanged each_pcm_tag stated16 assertion; no decoder algorithm, payload or existing signature changes. Ninety-six qualified legacy/native-width pairs cover mono/stereo, both byte orders, direct/nested enda, five qualified clocks and16 independently sliced delay/trim/silence/repeat edits. Exact native PCM/time/duration, precise seek/rewind/EOF/MAX,288 actual software-worker sessions per stand and full/interval owned f32/f64 exports with low float64 bits pass. Half-physical-frame inputs keep specific incomplete-frame refusals;16 past-source pairs remain explicit existing refusals, not successful playback. Qualification passes167 Rust executions, two old device ignores, core/player checks and three external-tool-free repeats. All280 generated files are identical on regeneration and5024 source/input hashes stable. Qualified FFmpeg reference matches160/160 scalar outputs;16 clock47 duration1 pairs remain labeled unqualified with retained divergence diagnostics, outside accepted playback evidence. Early aligned, packet-header and unsupported-tail authoring stages are retained and not counted. One physical-width invariant, no extra IDs for timestamps, export, variants or refusals; no performance, physical-device or hardware-video claim. See `batch-548-validation.json`.

## Batch 549 — FVID-0538: Admit classic QuickTime v0 NONE and zero PCM aliases with strict geometry

FVID-0538 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_v0_pcm_aliases.rs`): Normalize only v0 compressionID0 packetSize0 mono/stereo8/16-bit NONE/zero to raw/twos; preserve payload/index/clocks and common reference validation. Zero channels/rate retain specific Invalid refusals. Other versions/compression/width/layout remain unsupported and preserve later valid tracks. Twenty-four qualified cases cover three clocks, exact PCM/video, duration, seek/rewind/EOF/MAX,72 actual software-worker sessions per stand and full/interval owned f32/f64 export. Qualification passes171 Rust executions, two existing physical-device ignores, core/player checks and three external-tool-free repeats. All102 authored files regenerate identically and5128 source/input hashes remain stable. Independent FFmpeg matches24/24 qualified cases;24 wider diagnostics remain explicitly unsupported and are not counted. One compatibility capability; no performance or hardware claim. See `batch-549-validation.json`.

## Batch 550 — FVID-0539: Scope finite MP4 audio-edit packet admission to referenced descriptions

FVID-0539 (`crates/fvid-media/src/owned_mp4_audio_timeline_impl.rs`, `tests/mp4_edit_selected_admission.rs`): Resolve the first referenced header and retain the cheap equal-catalog condition, otherwise establish constant selected geometry with the existing compact-index-aware traversal. Eighty PCM/ALAC variants cover dormant supported/opaque/duplicate descriptions,8000/1001 movie clocks, v0/v1 early/preroll/repeat/gap/silence edits. The audible prefix now admits400 bytes while valid larger packets still refuse when reached; exact f32/PCM-only f64 output, public plan/raw export, memory/cancellation controls, native duration/PCM/seek/rewind/EOF,240 actual software-worker sessions per stand and exact software video pass. Native occupied-source-sample frontiers and owned cumulative movie-grid policies remain distinct and unchanged, with separate analytic rational oracles; ALAC remains refused by the explicitly PCM-only precise API. Qualification passes197 Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats. All145 authored files regenerate identically and5275 portable source/input hashes remain stable. Incorrect initial tfhd rewriting/output API/oracle assumptions are archived as authoring diagnostics, not counted bugs. One edit admission invariant distinct from selected Track headers and compact-index geometry; no performance, hardware or public-interface claim. See `batch-550-validation.json`.

## Batch 551 — FVID-0540: Charge MP4 decoded-packet memory only to admitted decoder descriptions

FVID-0540 (`crates/fvid-media/src/owned_mp4_audio.rs`, `tests/mp4_unused_expansion_budget.rs`): Accumulate the unchanged expansion factors in the existing admitted-decoder loop, preserving all retained catalog/index costs, initial decoder admission and64x MACE growth when MACE is selected. No added scan, allocation or public-interface change. Eight dormant-MACE variants now admit their selected PCM under the original caps with exact480-frame f32/precise f64 output, public planning, controlled-memory/cancellation refusal,60ms native duration, seek/rewind/EOF,24 actual software-worker sessions/anchors per stand and exact software VP9 pictures. Four actually selected fragmented MACE controls still refuse predeclared tight caps before caller output and admit9216 frames at256KiB, matching direct elementary packet/history decoding; the unchanged static authored MACE codebook reference test also passes. Qualification passes199 Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats. All26 synthetic files regenerate identically and5303 portable source/input hashes remain stable. One retained-buffer admission invariant, separate from compact-index and finite-edit prefix defects. Estimates exclude parser temporaries/caller I/O under the existing contract; no RSS, actual-allocation reduction, performance or hardware claim. See `batch-551-validation.json`.

## Batch 552 — FVID-0541: Use compact MP4 chunks when checking selected audio description IDs

FVID-0541 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_admission_catalog.rs`): Add one private compact-index-aware membership query: readable positive hits stop immediately; dense/expanded indexes retain per-sample traversal and malformed/unreadable negative paths fall back. Wire six membership checks without changing public signatures, index representation, catalog contents, packet bytes, clocks, budgets or decoder algorithms. The original scalar predicate agrees at1008 prefix/ID/shape combinations per compiled equivalence test. Seven balanced release process pairs, two warmups and nine16-plan rounds each preserve every captured plan fingerprint:32-description first/last targets fall675.570→101.479us and679.234→100.042us (6.657x/6.790x), each winning7/7 pairs; one-description control93.867→93.461us remains within the predeclared10% bound. Final191 passing Rust executions, two old device ignores, core/player checks and three external-tool-free repeats preserve exact full/prefix f32/f64 PCM, compact cost, packet/memory/cancellation refusals, native seek/rewind/EOF,three actual target-worker sessions/anchors and software VP9 pictures, plus related dynamic/opaque/edit/MACE regressions. All five synthetic files regenerate identically and5312 final source/input hashes stay stable. Initial partial wiring and first qualified runtime are archived; review added early positive exit and requalified the changed final runtime, not a second ID. One planning optimization on this warm-cache local one-chunk workload; no general playback, codec-throughput, allocation, RSS or hardware claim. See `batch-552-validation.json`.

## Batch 553 — FVID-0542: Use compact header comparisons when admitting constant MP4 readers

FVID-0542 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_constant_open_catalog.rs`): Preserve the first selected header versus representative Track comparison, then reuse the existing compact-aware header-change predicate instead of another full virtual-sample scan. Preserve empty-track and0/1-catalog behavior, equivalent aliases, opaque/changing-header refusal, all public signatures, decoder state, bytes, clocks, index/catalog contents and budgets. The exact original scalar guard agrees at168 structural/primary/catalog combinations per compiled property. Seven balanced release process pairs, two warmups and nine1024-open rounds each retain reader/index fingerprints:32-description first/last opens fall87.290→3.452us and87.191→3.439us (25.284x/25.354x), winning7/7 pairs each. One-description control1.517→1.536us is1.25% slower, within the predeclared10% bound, and is not claimed as a speedup. The pilot calibrated1024 iterations before candidate changes or paired measurements; acceptance thresholds were unchanged. Final190 Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats preserve every packet/clocks/description ID through native and owned constant APIs, exact PCM words, compact budget4, typed reconfiguration refusal, native PCM/seek/rewind/EOF/worker anchors/software pictures and related dormant/opaque/MACE/dynamic regressions. Five existing341ms authored fixtures regenerate identically;5314 source/input hashes stay stable. One constant-header admission optimization, distinct from ID-membership planning; no new format/failure, allocation, acquisition, planning, decoder or playback-throughput/hardware claim. See `batch-553-validation.json`.

## Batch 554 — FVID-0543: Preserve direct QuickTime PCM byte order beside unrelated wave metadata

FVID-0543 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_enda_wave_fallback.rs`): Read nested enda first, preserving its existing precedence and wave validation, then consult the direct field only if the nested field is absent. Keep the existing0/1 flag validation and duplicate/truncation refusals. Sixty-four valid variants cover four existing PCM tags, mono/stereo, both byte orders,1000/48000 clocks and two child orders; exact literal f32/f64 output retains low int32/float64 bits, parsed configuration,60ms duration, precise seek/rewind/EOF,192 actual software-worker sessions/anchors per stand, public raw export, sample intervals and exact software VP9 pictures. The12 malformed files now produce the same typed Invalid/non-unsupported errors as direct-only controls; resource/cancellation checks remain strict before output. Qualification passes195 Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats. All140 short authored files regenerate identically and5456 source/input hashes stay stable; all64 valid reference input hashes still match. One missing-fallback invariant with successful decode and validation-bypass consequences, not per-tag/variant/refusal IDs. No old tests changed; no new format, public interface, decoder algorithm, conflict policy, performance or hardware claim. See `batch-554-validation.json`.

## Batch 555 — FVID-0544: Read ALAC and EAC3 configuration after the QuickTime version-one audio header

FVID-0544 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_v1_coded_config.rs`): A shared checked version0/1 child offset now skips the16-byte v1 extension and gives typed Invalid refusal for incomplete admitted v1 headers. Fourteen valid companions cover ALAC8000/1000/48000 clocks and EAC3 stereo48000/1000 clocks, specified/unspecified packet fields, and optional dec3 packet-probe controls. Configuration, all coded packets and rational container timestamps match v0 controls; native PCM matches every sample, ALAC matches its literal oracle, and all short synthetic videos decode without error. Exact native seek from0/1ns/20ms/end/MAX, rewind, EOF and42 software audio-worker sessions/anchors pass per stand. Independent ffmpeg-full9.0.2 confirms14/14 full-length v0/v1 pairs;31 fixtures regenerate identically and5489 source/input hashes remain stable. Qualification passes191 Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats. One shared header-offset invariant, not per-codec/fixture IDs. No old tests weakened; no decoder algorithm, public interface, new format, performance or physical-device claim. See `batch-555-validation.json`.

## Batch 556 — FVID-0545: Reject reserved flags before interpreting MP4 sample tables

FVID-0545 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_sample_table_flags.rs`): Validate the low24 reserved flag bits only for eight interpreted sample-table kinds before index construction. Existing stz2 flags/reserved-field checks and diagnostic stay unchanged; CTTS version1 and defined flags in unrelated headers remain admitted. All81 malformed companions now give the expected typed Invalid/non-unsupported refusal; only72 are newly denied. Twenty-seven zero controls preserve literal full-length PCM and parsed sample timestamps, three decoded VP9 pictures,60ms duration, exact seek from0/1ns/20ms/end/MAX, rewind, EOF and81 software-worker sessions/anchors per stand. Independent ffmpeg-full confirms27/27 full-length literal audio controls. All110 authored files regenerate identically; every malformed companion differs only in1..3 flag bytes, and5601 source/input hashes remain stable. Qualification passes187 Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats, including unchanged compact-size/padding/enablement tests. One missing sample-table header invariant across eight kinds, not per-table/codec/bit IDs. No old tests weakened; no codec, public API, unknown-version, performance or physical-device claim. See `batch-556-validation.json`.

## Batch 557 — FVID-0546: Keep full-width media ticks until after uniform MP4 seek division

FVID-0546 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_uniform_seek_width.rs`): Divide clamped nonnegative ticks by duration in u64, then checked-convert the quotient to usize and clamp to the last sample. Empty indices, zero durations, negatives and expanded behavior remain unchanged. Production Rust wasm32 now passes220/220 checks:44 real parsed-media requests and176 public-index boundaries, including2^32/2^40/i64MAX, large deltas and quotient overflow. Three additional runs with unavailable PATH/media tools pass220 each using the explicit Node runtime. Ordinary native tests require no Node, FFmpeg, fixture generation or network. Four short synthetic videos retain literal full PCM, parsed index shape, decoded VP9, precise Duration/raw seeks, rewind/EOF and12 actual software-worker sessions/anchors per stand; all4 match independent FFmpeg literal PCM. Six authored files regenerate identically and5614 source/input hashes stay stable. Qualification passes177 Rust test executions, two existing device ignores, core/player checks, a Windows media cross-check and unchanged backend diagnostics. A separate locked wasm probe is wired into Linux CI; local and dedicated remote Linux wasm32 execution pass; whole CI remains separately limited by pre-existing integration failures. The platform prerequisite restricts the existing errno catalog to Windows and keeps the numeric fallback on targets with no OS errno catalog; Unix/Windows branches retain their behavior. One32-bit media-index narrowing defect, not per-case/fixture/CI/build-prerequisite IDs. No public API, codec algorithm, performance, general browser/player or physical32-bit device claim. See `batch-557-validation.json`.

## Batch 558 — FVID-0547: Count every declared MP4 track against the track limit

FVID-0547 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_track_budget_order.rs`): Charge the ordinal of every declared trak before parsing it, independent of support/skip status and order. Opaque unsupported tracks remain skippable within the total budget. Ten authored60ms movies cover video-only, recognized video+PCM and all permutations of video with one/two unsupported sound tracks. All25 below-total budget cases now give the existing typed Invalid/non-unsupported refusal through both readers; exact/ample budgets retain track/refusal counts, packet clocks and full exact software VP9 video. Twenty actual software-video reader sessions per stand preserve seek/rewind/EOF. Independent FFmpeg confirms all10 complete videos; no unsupported-audio playback is claimed. Twelve files regenerate identically and5629 source/input hashes are frozen. Qualification passes215 Rust test executions, two existing device ignores, core/player builds, six external-tool-free native repeats and two220-case actual wasm32 compatibility runs. One total-track resource invariant, not per-reader/order/codec/fixture IDs. No codec algorithm, public signature, packet/sample budget, speedup or hardware/UI claim. A separately verified zero-count spatial-budget test-stage correction retains the original early packet contract and all assertions. See `batch-558-validation.json`.

## Batch 559 — FVID-0548: Validate MP4 track ID uniqueness before unsupported-codec skips

FVID-0548 (`crates/fvid-media/src/owned_mp4_impl.rs`, `tests/mp4_opaque_track_ids.rs`): Record every nonzero tkhd ID before codec-dependent early returns; preserve existing Invalid duplicate diagnostic and track budget. Eighteen malformed movies now refuse through both readers;20 valid unique-ID controls retain exact packets, clocks, software VP9 video, EOF, raw seeks and rewind. Forty actual software video sessions per stand pass. All20 complete valid videos match independent FFmpeg and40 fixture files regenerate exactly. Qualification passes207 Rust executions with two existing device ignores, core/player checks, six external-tool-free stand repeats and220 actual wasm32 compatibility cases. After a separate zero-count ALAC CI contract repair, current-source core/player MP4 stands pass12 more tests and the complete fvid-media suite passes. All5671 source/input paths are frozen with an explicit five-file ALAC/PCM delta; MP4 candidate and fixtures unchanged. One movie namespace invariant, not per-reader/version/order IDs; unsupported sound is not claimed decoded. No public API, performance, GPU or hardware claim. See `batch-559-validation.json`.

## Batch 560 — FVID-0549: Probe compact MP4 metadata without enumerating virtual audio samples

FVID-0549 (`crates/fvid-media/src/owned_mp4_probe_impl.rs`, `crates/fvid-media/tests/mp4_uniform_probe.rs`): Use one representative constant-size sample for prefix quota checks and earliest PTS; calculate constant byte/tick ratio from the last readable uniform sample, preserving unavailable terminal timestamps, zero durations and empty-index behavior. Expanded paths, edits, API and all limits remain. Candidate medians29.625us/30.000us reduce probe latency96.12/97.15percent with7/7 pair wins in both large cases; complete metadata matches in every pair. Small64-frame controls remain within predeclared5us noise allowance (+0.166us unbounded,-0.707us bounded). Eight supported authored controls compare start and bitrate to full-sample oracles and preserve prefix/resource gates; the changing-header refusal remains. All12 existing fixture files regenerate exactly and5673 source/input paths are frozen. Complete media suite728 passes(one existing ignore),core424 andplayer1256 pass(26 existing ignores),default library379 passes,core/player checks and2 external-tool-free probe tests pass. Related integrations retain exact PCM, actual video seek/rewind/EOF and full-width time navigation. Two separately evidenced zero-count repairs correct old open-edit and last-sample-versus-exclusive-end test expectations while retaining all assertions and adding endpoint acceptance. One probe pipeline optimization across wrappers; no decode,hardware,UI or broad workload speedup claim. See `batch-560-validation.json`.

## Batch 561 — FVID-0550: Preserve exact fractional MP4 video edit boundaries

FVID-0550 (`src/playback/video/mp4_exact_edits.rs`, `tests/mp4_subtick_video_edits.rs`): Private rational edit program preserves movie duration, clipped frame periods, PTS, seek, EOF, rewind and cache time ranges without changing public constructors. Fifteen supported edited programs include prime movie clocks, negative/reordered CTTS and wide-origin fMP4; separate subnanosecond Unsupported refusal and indexed late-seek IO gate remain strict. Five regression tests and actual Player playback for all15 programs pass;22 files regenerate identically. Qualification core400/player1270(26 existing ignores)/default381, core/player/wasm library checks and5699 stable hashes. Related finite-edit oracle now uses exact movie time with every pixel/seek/EOF assertion retained. ADTS diagnostic and Windows CI resource settings are separate zero-count repairs; remote MSVC verification pending. One timing defect, not per-case IDs; no GPU/hardware/browser or general speedup claim. See `batch-561-validation.json`.

## Batch 562 — FVID-0551: Infer fragmented MP4 subtitle endpoints at the next distinct PTS group

FVID-0551 (`src/playback/subtitles/mp4_subtitles.rs`, `tests/mp4_fragment_subtitle_equal_pts.rs`): Find the next strictly greater PTS group logarithmically in the existing sorted index before signed clipping/edit mapping. Preserve explicit durations, terminal fallback, admission, quotas and public API. Candidate3pass; exact source YUV, RGB/intervals after real native seeks, repeated cue extraction and boundary lookups pass. Related integrations21pass; full player library1207pass with26 existing ignores. Fourteen files regenerate exactly without FFmpeg/network; relevant source hashes and stand extension recorded. One successor-selection defect, distinct from FVID-0519 signed endpoint clipping; no per-case IDs, performance/export/hardware/GUI styling claim. Broader CPU and remote platform checks remain separately in progress. See `batch-562-validation.json`.

## Batch 563 — FVID-0552: HEVC base-layer dispatch and start-bound seek

Probe Matroska HEVC layered capability from NAL headers and parameter sets without decoding base-layer slice entropy during open, and defer `Player` initial frame presentation until after the configured start-bound seek. Sequential playback still refuses damaged earlier pictures with the exact NAL/RBSP/PPS error, while exact seeks and start-bound playback into a later valid CVS succeed across all 6 `hevc_base_dispatch_seek` tests, `matroska_parameter_seek` and `player::hevc_start_seek_tests`. Accepted total 552/1000. See `batch-563-validation.json` and `evidence/hevc-base-dispatch-seek/`.

## Batch 564 — FVID-0553..0560: Full CPU audit regressions and budget/timeline parity

Resolve all 34 failing integration test targets (47 tests) from the full CPU audit (`evidence/full-cpu-audit-20261103/failures.json`):
- **FVID-0553** — Truncate finite sub-pixel drawing expressions within `i64` bounds before integer canvas clipping (`owned_draw.rs`, `tests/native_drawing.rs`).
- **FVID-0554** — Charge one decoded float32 PCM frame working set alongside container metadata in ADTS and Matroska audio memory budgets (`owned_adts_export.rs`, `tests/audio_budget_controls_regressions.rs`).
- **FVID-0555** — Scope Matroska and MP4 packet byte budgets to selected streams, active intervals and `max_packets` prefixes across Y4M, lossless FFV1, subtitle burn and audio decode/plan routes (`src/native_export.rs`, `tests/y4m_prefix_budget_regression.rs`).
- **FVID-0556** — Enforce explicit speaker masks for multichannel downmix admission in audio decode plans (`owned_audio_plan.rs`, `tests/container_loudness_plan_regressions.rs`).
- **FVID-0557** — Honor edit-list timescale scaling in MP4-to-Matroska remux and preserve LIST/INFO metadata on Matroska PCM exports (`owned_matroska_pcm_export.rs`, `tests/native_matroska_pcm.rs`).
- **FVID-0558** — Preserve full Opus packet duration when `TrackEntry` `SeekPreRoll` is zero without `DiscardPadding` (`owned_matroska_audio_timeline_impl.rs`, `tests/native_opus_audio_export.rs`).
- **FVID-0559** — Enforce selected companion track metadata and packet schedule budgets consistently in multitrack Matroska and MP4 lossless planning and export (`owned_mp4_multitrack_export.rs`, `tests/multitrack_lossless_budget_parity_regressions.rs`).
- **FVID-0560** — Charge simultaneous `Y4mReader` and `RawFrame` working sets in native Y4M export and planning (`src/native_export.rs`, `tests/lossless_plan_regressions.rs`).

Accepted total 560/1000. See `batch-564-validation.json`.

## Batch 565 — FVID-0561..0563: Import tri-planar 4:2:0 VideoToolbox surfaces zero-copy into Metal textures and player display

- **FVID-0561** — Import tri-planar 4:2:0 VideoToolbox surfaces zero-copy into Metal textures and player display (`crates/fvid-vt/src/metal.rs`, `crates/fvid-vt/src/lib.rs`): Request Metal compatibility on PLANAR_420/PLANAR_420_FULL decoder pools, import plane 2 as an R8Unorm Cr texture in FrameSurface::import_metal, and bind 3-plane native surfaces directly in VideoGpu::import_surface_rotated across all rotations.
- **FVID-0562** — Zero-copy VideoToolbox surface import and export in resident Metal GPU pipelines (`src/gpu/mod.rs`, `tests/resident.rs`): Add GpuPipeline::upload_surface and ResidentFrame::export_surface on macOS Metal with zero-copy CVMetalTexture import and compute-shader plane packing/unpacking for both NV12 and tri-planar 4:2:0 surfaces, verified bit-exact against CPU reference.
- **FVID-0563** — Staged storage-buffer compute execution and packed RGBA texture display in VideoGpu (`src/player_gpu.rs`, `tests/gpu_shader.rs`): Add ExecutionMode::StagedCompute for discrete Vulkan/Metal/D3D12 adapters with storage buffers, and pack RGB24 into R32Uint textures for GPU-side unpacking in mode 4.0 while aligning RGBA uploads to 64-pixel rows.

See `batch-565-validation.json`.

## Batch 566 — FVID-0564: Infer Matroska subtitle endpoints at the next strictly later visible PTS group

FVID-0564 (`src/playback/subtitles/webm_subtitles.rs`, `tests/matroska_subtitle_equal_pts.rs`): Advance inferred Matroska subtitle endpoints to the first visible packet with a strictly later presentation timestamp (`*next_pts > *pts`), so simultaneous `SimpleBlock` subtitle cues without `BlockDuration` or `DefaultDuration` end at the next distinct subtitle group instead of falling back to `start + LAST_LINE` (`2s`) and lingering across video seeks. Verified across 12 synthetic Matroska witnesses (`UTF8`, `SSA`, `ASS` across `equal`, `terminal`, `explicit`, and `distinct` modes) and 26 related subtitle regression tests. See `batch-566-validation.json`.

## Batch 567 — FVID-0565..0569: Matroska tail/cluster timing and subtitle text/endpoint fidelity

- **FVID-0565** — Unfabricated Matroska AVC/HEVC/PAFF tail and field-pair duration inference (`src/playback/video/webm.rs`, `tests/matroska_reordered_tail.rs`).
- **FVID-0566** — Walk lazy Matroska clusters to the first visible video packet on open (`src/playback/video/webm.rs`, `tests/matroska_lazy_cluster_origin.rs`).
- **FVID-0567** — Skip empty and markup-only Matroska subtitle blocks when inferring untimed cue endpoints (`src/playback/subtitles/webm_subtitles.rs`, `tests/matroska_empty_subtitle_successor.rs`).
- **FVID-0568** — Isolate HTML angle-bracket and ASS brace-override state machines in subtitle plain-text stripping (`src/subtitles.rs`, `tests/subtitle_nested_delimiters.rs`).
- **FVID-0569** — Accept canonical SSA `Marked=N` layer fields in Matroska `S_TEXT/SSA` blocks (`src/subtitles.rs`, `tests/subtitle_nested_delimiters.rs`).

See `batch-567-validation.json`.

## Batch 568 — FVID-0570..0574: Subtitle BOM handling, in-band AVC/HEVC opening colour/HDR, Matroska AVC probe profile/level, and Dolby indexed duration fallback

- **FVID-0570** — Strip UTF-8 BOM (`\u{FEFF}`) across sidecar, Matroska, and MP4 `tx3g` subtitle parsers (`src/subtitles.rs`, `src/playback/subtitles/mp4_subtitles.rs`, `tests/subtitle_bom_handling.rs`).
- **FVID-0571** — Preserve opening and post-`rewind()`/`seek_to_sync(0)` `bitstream_colour()` and `bitstream_hdr()` for Matroska single-layer and multilayer HEVC and AVC (`src/playback/video/webm.rs`, `src/playback/video/webm_layered.rs`, `tests/inband_open_signal_and_ac3_duration.rs`).
- **FVID-0572** — Preserve opening and post-`rewind()` `bitstream_colour()`, `active_colour()`, and `bitstream_hdr()` for MP4 in-band `avc3` and single-layer/multilayer/mixed `hev1` (`src/playback/video/mp4.rs`, `src/playback/video/mp4_dispatch.rs`, `src/playback/video/mp4_mixed_hevc.rs`, `tests/inband_open_signal_and_ac3_duration.rs`).
- **FVID-0573** — Populate `StreamInfo.profile` and `StreamInfo.level` for Matroska `V_MPEG4/ISO/AVC` tracks in media probe (`crates/fvid-media/src/owned_webm_probe.rs`, `src/media_probe.rs`, `tests/inband_open_signal_and_ac3_duration.rs`).
- **FVID-0574** — Fallback to indexed block and packet-span duration for Matroska `A_AC3` and `A_EAC3` when `Segment/Info/Duration` is omitted (`src/playback/audio/webm_audio.rs`, `tests/inband_open_signal_and_ac3_duration.rs`).

See `batch-568-validation.json`.

## Batch 569 — FVID-0575..0579: Matroska probe/subtitle codec parity, `DefaultDuration` and `S_TEXT/SSA` conversion, ASS `Format:`/`\h`, and MP4 VUI `nclx` fallback

- **FVID-0575** — Normalize Matroska Dolby and text subtitle codec names across `owned_webm_probe`, `media_probe`, and `owned_codec_metadata` (`crates/fvid-media/src/owned_webm_probe.rs`, `src/media_probe.rs`, `crates/fvid-media/src/owned_codec_metadata.rs`, `tests/matroska_subtitles_and_mp4_vui_fallback.rs`).
- **FVID-0576** — Support `DefaultDuration` fallback and `S_TEXT/SSA` / legacy codec IDs in Matroska subtitle conversion and planning (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/matroska_subtitles_and_mp4_vui_fallback.rs`).
- **FVID-0577** — Support `S_TEXT/WEBVTT`, `S_TEXT/ASCII`, and legacy `S_UTF8`/`S_ASS`/`S_SSA` tracks in `WebmSubtitleReader` (`src/playback/subtitles/webm_subtitles.rs`, `tests/matroska_subtitles_and_mp4_vui_fallback.rs`).
- **FVID-0578** — Support custom `[Events]` `Format:` column order, case-insensitive `Dialogue:`, and `\h` hard space in ASS/SSA subtitle parsing (`src/subtitles.rs`, `tests/matroska_subtitles_and_mp4_vui_fallback.rs`).
- **FVID-0579** — Preserve container `colr` (`nclx`) `primaries`, `transfer`, and `matrix` when MP4 AVC or HEVC VUI reports unspecified code `2` (`crates/fvid-media/src/owned_mp4_video_decode.rs`, `tests/matroska_subtitles_and_mp4_vui_fallback.rs`).

See `batch-569-validation.json`.

## Batch 570 — FVID-0580..0584: Matroska VUI colour fallback, `ColourDescription::filled_with` code `2`, MP4 lossless VUI/HDR preservation, `fvid-media` subtitle parsing parity, and `.vtt`/`.ass`/`.ssa` subtitle burn-in

- **FVID-0580** — Preserve Matroska container `Colour` fallback when bitstream VUI/sequence header reports unspecified code `2` or omits colour description (`crates/fvid-media/src/owned_video_decode.rs`, `crates/fvid-media/src/owned_ffv1_export.rs`, `tests/mkv_vui_fallback_and_subtitle_burn.rs`).
- **FVID-0581** — Treat H.273 unspecified code `2` as replaceable in `ColourDescription::filled_with` (`src/color/hdr.rs`, `tests/mkv_vui_fallback_and_subtitle_burn.rs`).
- **FVID-0582** — Preserve bitstream VUI `colour` and HDR10 SEI/OBU `hdr` metadata in `native_lossless::write_mp4_selected` (`src/native_lossless.rs`, `tests/mkv_vui_fallback_and_subtitle_burn.rs`).
- **FVID-0583** — Support `\h` hard space, HTML tags, general `{...}` override blocks, SSA `Marked=N`, custom `[Events]` `Format:`, and inline WebVTT cue IDs in `fvid_media::plain_subtitle` and `parse_subtitle_text` (`crates/fvid-media/src/owned_play_controls_impl.rs`, `tests/mkv_vui_fallback_and_subtitle_burn.rs`).
- **FVID-0584** — Support external `.vtt` WebVTT and `.ass`/`.ssa` ASS/SSA subtitle files in `owned_subtitle_burn::supports`, `plan_burn_subtitles`, and `burn_subtitles` (`crates/fvid-media/src/owned_subtitle_burn.rs`, `tests/mkv_vui_fallback_and_subtitle_burn.rs`).

See `batch-570-validation.json`.

## Batch 571 — FVID-0585..0589: `WebmVideoReader`/`Mp4VideoReader` container `Colour` fallback, VP9 `CS_UNKNOWN` fallback, `.ssa`/`.vtt`/`S_TEXT/WEBVTT` subtitle conversion, and case-insensitive `ass_block`

- **FVID-0585** — Preserve container `Colour` (`colr`) `matrix` and `full_range` fallback and per-frame `presented_signal` in `WebmVideoReader` and `Mp4VideoReader` when AVC or HEVC VUI omits colour description or reports code `2` (`src/playback/video/webm.rs`, `src/playback/video/mp4.rs`, `src/playback/video/native.rs`, `src/playback/video/packed.rs`, `tests/video_colour_and_subtitle_convert.rs`).
- **FVID-0586** — Fall back to container `Colour` (`colr`) `matrix` and `full_range` for Matroska and MP4 VP9 `CS_UNKNOWN` (`color_space == 0`) and AV1 unspecified matrix (`crates/fvid-media/src/owned_webm_codec.rs`, `crates/fvid-media/src/owned_video_decode.rs`, `src/playback/video/webm.rs`, `src/playback/video/mp4.rs`, `tests/video_colour_and_subtitle_convert.rs`).
- **FVID-0587** — Support standalone `.ssa` (`SubStation Alpha v4.00`) script conversion and planning to Matroska `S_TEXT/ASS` (`crates/fvid-media/src/owned_subtitle_impl.rs`, `crates/fvid-media/src/owned_matroska_tracks_impl.rs`, `tests/video_colour_and_subtitle_convert.rs`).
- **FVID-0588** — Support standalone `.vtt` WebVTT and Matroska `S_TEXT/WEBVTT` subtitle conversion and planning to Matroska `S_TEXT/ASS` (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/video_colour_and_subtitle_convert.rs`).
- **FVID-0589** — Match `Dialogue:`, `Comment:`, `Format:`, and `Style:` prefixes case-insensitively in `subtitles::ass_block` (`src/subtitles.rs`, `tests/video_colour_and_subtitle_convert.rs`).

See `batch-571-validation.json`.

## Batch 572 — FVID-0590..0594: ADTS Main/SSR/LTP & PCE probe, Y4M default `25:1` rate in `Y4mReader`, `440`/`411`/`410` native Y4M export, `C411p9..16`/`C410p9..16` Y4M headers, and open-ended Matroska chapter end inference

- **FVID-0590** — Support AAC `Main`/`LC`/`SSR`/`LTP` profiles, actual `frame_samples`, and in-band PCE `extradata_bytes` in ADTS probe (`crates/fvid-media/src/owned_probe.rs`, `tests/y4m_adts_and_mkv_probe.rs`).
- **FVID-0591** — Default omitted Y4M `F` frame-rate token to `25:1` (`25 fps`) in `Y4mReader::new` and `NativeReader` (`src/playback.rs`, `tests/y4m_adts_and_mkv_probe.rs`).
- **FVID-0592** — Support `440` (`[1, 2]`), `411` (`[4, 1]`), and `410` (`[4, 4]`) planar chroma subsampling in native Y4M export, inspection, and `VideoGeometry` (`src/native_export.rs`, `src/native_geometry.rs`, `tests/y4m_adts_and_mkv_probe.rs`).
- **FVID-0593** — Support high-bit-depth `C411p9..16` and `C410p9..16` Y4M headers in `Header::parse` and `try_y4m_with_options` (`crates/fvid-media/src/owned_y4m_impl.rs`, `crates/fvid-media/src/owned_y4m_probe.rs`, `tests/y4m_adts_and_mkv_probe.rs`).
- **FVID-0594** — Infer open-ended Matroska `ChapterAtom` end timestamps from next chapter start or container duration in `owned_webm_probe` (`crates/fvid-media/src/owned_webm_probe.rs`, `tests/y4m_adts_and_mkv_probe.rs`).

See `batch-572-validation.json`.

## Batch 573 — FVID-0595..0599: AIFF zero-frame block-aligned SSND rejection, `AviAudioReader` `seek_time` and `present_decoded`, Y4M/MP4 FFV1 `411`/`410` & default-rate eligibility, and Matroska bitmap subtitle / `data` track probing

- **FVID-0595** — Reject AIFF block-aligned `SSND` chunks whose `declared_frames == 0` trims the sample run to empty (`src/playback/audio/aiff.rs`, `tests/aiff_avi_ffv1_and_mkv_probe.rs`).
- **FVID-0596** — Support sub-sample `seek_time`, `present_decoded` `presentation_floor` trimming, and G.711 clock range check in `AviAudioReader` (`src/playback/audio/avi_audio.rs`, `tests/aiff_avi_ffv1_and_mkv_probe.rs`).
- **FVID-0597** — Support `411`/`410` subsampling and omitted Y4M `F` frame-rate token in `native_lossless` and `native_lossless_y4m` (`src/native_lossless.rs`, `src/native_lossless_y4m.rs`, `tests/aiff_avi_ffv1_and_mkv_probe.rs`).
- **FVID-0598** — Map Matroska `S_VOBSUB` (`dvd_subtitle`) and `S_HDMV/PGS` (`hdmv_pgs_subtitle`) bitmap subtitle codec IDs in `media_probe` and `owned_webm_probe` (`src/media_probe.rs`, `crates/fvid-media/src/owned_webm_probe.rs`, `tests/aiff_avi_ffv1_and_mkv_probe.rs`).
- **FVID-0599** — Report `"data"` `media_type` for non-AVS Matroska tracks in `owned_webm_probe` (`crates/fvid-media/src/owned_webm_probe.rs`, `tests/aiff_avi_ffv1_and_mkv_probe.rs`).

See `batch-573-validation.json`.

## Batch 574 — FVID-0600..0604: Matroska `A_WAVPACK4` probe normalization, `owned_codec_metadata` audio/subtitle descriptors, WAVE `ILNG` <-> `language` mapping, and `AacAudioReader` / `Mp3AudioReader` sub-sample `seek_time`

- **FVID-0600** — Normalize Matroska `A_WAVPACK4` codec ID to `"wavpack"` in `media_probe` and `owned_webm_probe` (`src/media_probe.rs`, `crates/fvid-media/src/owned_webm_probe.rs`, `tests/mkv_wavpack_wave_ilng_and_aac_mp3_seek.rs`).
- **FVID-0601** — Register canonical audio (`wavpack`, `adpcm_ima_qt`, `adpcm_ima_wav`, `adpcm_ms`, `mace3`, `mace6`) and bitmap subtitle (`dvd_subtitle`, `hdmv_pgs_subtitle`) codecs in `owned_codec_metadata::descriptor` (`crates/fvid-media/src/owned_codec_metadata.rs`, `tests/mkv_wavpack_wave_ilng_and_aac_mp3_seek.rs`).
- **FVID-0602** — Map RIFF INFO `ILNG` tag to and from semantic `"language"` metadata key in WAVE probe and metadata editor (`crates/fvid-media/src/owned_wave_metadata.rs`, `crates/fvid-media/src/owned_probe.rs`, `tests/mkv_wavpack_wave_ilng_and_aac_mp3_seek.rs`).
- **FVID-0603** — Implement sub-sample `div_ceil` `seek_time(Duration)` in `AacAudioReader` (`src/playback/audio/aac.rs`, `tests/mkv_wavpack_wave_ilng_and_aac_mp3_seek.rs`).
- **FVID-0604** — Implement sub-sample `div_ceil` `seek_time(Duration)` and `sample_duration` in `Mp3AudioReader` (`src/playback/audio/mp3.rs`, `tests/mkv_wavpack_wave_ilng_and_aac_mp3_seek.rs`).

See `batch-574-validation.json`.

## Batch 575 — FVID-0605..0609: `Ac3AudioReader`, `WavpackAudioReader`, and Ogg Vorbis/Opus/Catalog sub-sample `seek_time`, ID3v2.4 `3DI` footer parsing, and numeric `TCON` genre normalization

- **FVID-0605** — Implement sub-sample `div_ceil` `seek_time(Duration)` and `sample_duration` in `Ac3AudioReader` (`src/playback/audio/ac3.rs`, `tests/ac3_wavpack_ogg_seek_and_id3_tags.rs`).
- **FVID-0606** — Implement sub-sample `div_ceil` `seek_time(Duration)` in `WavpackAudioReader` (`src/playback/audio/wavpack.rs`, `tests/ac3_wavpack_ogg_seek_and_id3_tags.rs`).
- **FVID-0607** — Implement sub-sample `div_ceil` `seek_time(Duration)` in `OggAudioReader`, `OggOpusAudioReader`, and `playback_ogg_catalog` (`src/playback/audio/ogg_audio.rs`, `src/playback/audio/ogg_opus.rs`, `src/playback/audio/ogg_catalog.rs`, `tests/ac3_wavpack_ogg_seek_and_id3_tags.rs`).
- **FVID-0608** — Support ID3v2.4 10-byte `3DI` footer in `id3v2_bytes` and `parse_id3v2_into` for bare MPEG audio (`crates/fvid-media/src/owned_mpeg_audio.rs`, `tests/ac3_wavpack_ogg_seek_and_id3_tags.rs`).
- **FVID-0609** — Normalize numeric ID3v2 `TCON` genre codes (e.g. `"(17)"` or `"17"`) via `id3v1_genre_name` (`crates/fvid-media/src/owned_mpeg_audio.rs`, `tests/ac3_wavpack_ogg_seek_and_id3_tags.rs`).

See `batch-575-validation.json`.

## Batch 576 — FVID-0610..0614: MP4 `alaw`/`ulaw` G.711 probe normalization, full 80-entry ID3v1/ID3v2 genre table, Vorbis/ID3 `YEAR`/`ORGANIZATION`/`ENCODED-BY` metadata aliases, and `SmfAudioReader` / `XmAudioReader` sub-millisecond `seek_time`

- **FVID-0610** — Normalize MP4 `alaw` and `ulaw` audio sample entry FourCCs to `"pcm_alaw"` and `"pcm_mulaw"` in `owned_mp4_probe_impl` (`crates/fvid-media/src/owned_mp4_probe_impl.rs`, `tests/mp4_g711_probe_id3_vorbis_and_smf_xm_seek.rs`).
- **FVID-0611** — Expand `id3v1_genre_name` to the full standard 80-entry ID3v1 genre table (`0..=79`) (`crates/fvid-media/src/owned_mpeg_audio.rs`, `tests/mp4_g711_probe_id3_vorbis_and_smf_xm_seek.rs`).
- **FVID-0612** — Normalize `YEAR` -> `"date"`, `ORGANIZATION` -> `"publisher"`, and `ENCODED-BY` / `ENCODED_BY` -> `"encoder"` across Vorbis comments, ID3v2 `TXXX`, and `FileTags::field` (`crates/fvid-media/src/owned_flac_layout.rs`, `crates/fvid-media/src/owned_ogg_opus_export.rs`, `crates/fvid-media/src/owned_mpeg_audio.rs`, `crates/fvid-media/src/owned_file_tags.rs`, `tests/mp4_g711_probe_id3_vorbis_and_smf_xm_seek.rs`).
- **FVID-0613** — Implement sub-millisecond `div_ceil` `seek_time(Duration)` in `SmfAudioReader` (`src/playback/audio/smf.rs`, `tests/mp4_g711_probe_id3_vorbis_and_smf_xm_seek.rs`).
- **FVID-0614** — Implement sub-millisecond `div_ceil` `seek_time(Duration)` in `XmAudioReader` (`src/playback/audio/xm.rs`, `tests/mp4_g711_probe_id3_vorbis_and_smf_xm_seek.rs`).

See `batch-576-validation.json`.

## Batch 577 — FVID-0615..0619: Case-insensitive `[Events]` and `WEBVTT` subtitle sniffing, HTML entity decoding in `subtitles::plain`, case-insensitive WebVTT `NOTE`/`STYLE`/`REGION` conversion, Matroska `S_DVBSUB` -> `dvb_subtitle` probe normalization, and Y4M `pixel_aspect`/`full_range` probe validation

- **FVID-0615** — Case-insensitive `[Events]` and `WEBVTT` header sniffing in `subtitles::parse` (`src/subtitles.rs`, `tests/subtitles_case_entities_dvbsub_and_y4m_probe.rs`).
- **FVID-0616** — Decode named and numeric HTML character entities in `subtitles::plain` after stripping tags (`src/subtitles.rs`, `tests/subtitles_case_entities_dvbsub_and_y4m_probe.rs`).
- **FVID-0617** — Case-insensitive `WEBVTT` header and `NOTE`/`STYLE`/`REGION` block skipping in `parse_webvtt` and `owned_subtitle::try_convert_srt_inner` (`src/subtitles.rs`, `crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/subtitles_case_entities_dvbsub_and_y4m_probe.rs`).
- **FVID-0618** — Normalize Matroska `S_DVBSUB` codec ID to `"dvb_subtitle"` and register in `owned_codec_metadata` (`src/media_probe.rs`, `crates/fvid-media/src/owned_webm_probe.rs`, `crates/fvid-media/src/owned_codec_metadata.rs`, `tests/subtitles_case_entities_dvbsub_and_y4m_probe.rs`).
- **FVID-0619** — Validate Y4M `pixel_aspect()` and `full_range()` tokens during `owned_y4m_probe` inspection (`crates/fvid-media/src/owned_y4m_probe.rs`, `tests/subtitles_case_entities_dvbsub_and_y4m_probe.rs`).

See `batch-577-validation.json`.

## Batch 578 — FVID-0620..0624: WAVE `concat` `effective_channel_mask()` validation, `WAVEFORMATEXTENSIBLE` DirectOut and `wValidBitsPerSample` G.711 WAVE handling, `WAVEFORMATEXTENSIBLE` G.711 in `AviAudioReader`, and non-empty FLAC Vorbis comment selection for RIFF `INFO`

- **FVID-0620** — Compare `effective_channel_mask()` when validating WAVE `concat` input compatibility (`crates/fvid-media/src/owned_wave_remux.rs`, `tests/wave_remux_g711_avi_and_flac_info.rs`).
- **FVID-0621** — Preserve explicit DirectOut zero speaker mask in `owned_g711_wave_export` via `effective_channel_mask()` (`crates/fvid-media/src/owned_g711_wave_export.rs`, `tests/wave_remux_g711_avi_and_flac_info.rs`).
- **FVID-0622** — Require `wValidBitsPerSample == 8` for `WAVEFORMATEXTENSIBLE` G.711 in `owned_wave_inspect` (`crates/fvid-media/src/owned_wave_inspect.rs`, `tests/wave_remux_g711_avi_and_flac_info.rs`).
- **FVID-0623** — Admit `WAVEFORMATEXTENSIBLE` G.711 (`Coding::ALaw` and `Coding::MuLaw`) in `AviAudioReader::open_at` (`src/playback/audio/avi_audio.rs`, `tests/wave_remux_g711_avi_and_flac_info.rs`).
- **FVID-0624** — Skip empty Vorbis comment values when mapping standard FLAC tags to RIFF `INFO` in `wave_info_from_extra` (`crates/fvid-media/src/owned_flac_layout.rs`, `tests/wave_remux_g711_avi_and_flac_info.rs`).

See `batch-578-validation.json`.

## Batch 579 — FVID-0625..0629: Non-empty Ogg comment selection, Vorbis `AudioTrack` title/language population, 64-entry RIFF `INFO` cap in MP3 and `FileTags` WAVE export, empty RIFF `INFO` filtering in `probe_wave`, and empty ID3v2 frame filtering in `owned_mpeg_audio`

- **FVID-0625** — Skip empty Ogg comment values in `info_for_tags`, `owned_ogg_opus::Program`, and `owned_ogg_audio_catalog` (`crates/fvid-media/src/owned_ogg_opus_export.rs`, `crates/fvid-media/src/owned_ogg_opus.rs`, `crates/fvid-media/src/owned_ogg_audio_catalog.rs`, `tests/ogg_mp3_wave_metadata.rs`).
- **FVID-0626** — Populate `AudioTrack` `name` and `language` from the Vorbis comment header in `owned_ogg_vorbis::Program` (`crates/fvid-media/src/owned_ogg_vorbis.rs`, `tests/ogg_mp3_wave_metadata.rs`).
- **FVID-0627** — Cap RIFF `INFO` entries at 64 while preserving canonical tags in `Mp3::wave_info` and `info_for_file_tags` (`crates/fvid-media/src/owned_mpeg_audio.rs`, `crates/fvid-media/src/owned_wave_metadata.rs`, `tests/ogg_mp3_wave_metadata.rs`).
- **FVID-0628** — Skip empty RIFF `INFO` entries in `probe_wave_with_options` (`crates/fvid-media/src/owned_probe.rs`, `tests/ogg_mp3_wave_metadata.rs`).
- **FVID-0629** — Skip empty ID3v2 text, `TXXX`, and `COMM` frames in `owned_mpeg_audio::parse_id3v2_into` (`crates/fvid-media/src/owned_mpeg_audio.rs`, `tests/ogg_mp3_wave_metadata.rs`).

See `batch-579-validation.json`.

## Batch 580 — FVID-0630..0634: `WAVEFORMATEXTENSIBLE` G.711 storage width validation in `Wav::parse`, FLAC and MP3 `AudioTrack` title/language population, SMF `FF 03` trailing NUL trimming, and XM tail packet tempo header consistency

- **FVID-0630** — Require 8-bit storage and precision for `WAVEFORMATEXTENSIBLE` G.711 in `Wav::parse` (`src/playback/audio/wav.rs`, `tests/wav_flac_mp3_smf_xm_playback.rs`).
- **FVID-0631** — Populate `AudioTrack` `name` and `language` from Vorbis comments in `FlacAudioReader::audio_tracks` (`src/playback/audio/flac.rs`, `tests/wav_flac_mp3_smf_xm_playback.rs`).
- **FVID-0632** — Populate `AudioTrack` `name` and `language` from ID3 metadata in `Mp3AudioReader::audio_tracks` (`src/playback/audio/mp3.rs`, `tests/wav_flac_mp3_smf_xm_playback.rs`).
- **FVID-0633** — Trim trailing NUL bytes from SMF `FF 03` track/sequence names before emptiness check (`src/container/smf.rs`, `tests/wav_flac_mp3_smf_xm_playback.rs`).
- **FVID-0634** — Write final timeline row BPM and speed into XM release tail packet header (`src/container/xm.rs`, `tests/wav_flac_mp3_smf_xm_playback.rs`).

See `batch-580-validation.json`.

## Batch 581 — FVID-0635..0639: `WAVEFORMATEXTENSIBLE` 4-bit ADPCM storage width validation, `max_packets`-bounded bare MP3 mix/merge planning, Matroska `S_TEXT/ASS` `ReadOrder`/`Layer` validation, inline `WEBVTT` header cue resource limits, and strict `subtitles::clock` range checks

- **FVID-0635** — Require 4-bit storage and precision for `WAVEFORMATEXTENSIBLE` G.722 and Creative ADPCM in `Wav::parse` (`src/playback/audio/wav.rs`, `tests/wav_adpcm_mp3_plan_and_subtitle_validation.rs`).
- **FVID-0636** — Bound bare MP3 frame estimation by `controls.max_packets` in `audio_plan_with_controls` (`crates/fvid-media/src/owned_audio_mix.rs`, `tests/wav_adpcm_mp3_plan_and_subtitle_validation.rs`).
- **FVID-0637** — Validate `ReadOrder` and `Layer` integer fields in Matroska `S_TEXT/ASS` and `S_TEXT/SSA` conversion (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/wav_adpcm_mp3_plan_and_subtitle_validation.rs`).
- **FVID-0638** — Enforce `max_packets` and `max_controlled_bytes` on WebVTT cues sharing the `WEBVTT` header block (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/wav_adpcm_mp3_plan_and_subtitle_validation.rs`).
- **FVID-0639** — Require colon-separated `MM:SS` or `HH:MM:SS` with `minutes < 60` and `seconds < 60` in `subtitles::clock` (`src/subtitles.rs`, `tests/wav_adpcm_mp3_plan_and_subtitle_validation.rs`).

See `batch-581-validation.json`.

## Batch 582 — FVID-0640..0644: Bare MP3 `standalone_packet_work` Info/window accounting, `max_packets`-bounded interval checks for bare FLAC and bare MP3, non-empty `transform.interval` validation in Matroska PCM `geometry_for_decode`, and Matroska `LanguageIETF` preservation in WAVE `ILNG` export

- **FVID-0640** — Count leading Info packet and active presentation window frames in `standalone_packet_work` for bare MP3 (`crates/fvid-media/src/owned_audio_plan.rs`, `tests/audio_plan_mp3_flac_and_matroska_pcm.rs`).
- **FVID-0641** — Bound bare FLAC interval prefix check by `options.max_packets` in `open_with_interval` (`crates/fvid-media/src/owned_raw_flac.rs`, `tests/audio_plan_mp3_flac_and_matroska_pcm.rs`).
- **FVID-0642** — Bound bare MP3 interval prefix and spool budget by `options.max_packets` in `open_with_interval` (`crates/fvid-media/src/owned_raw_mp3.rs`, `tests/audio_plan_mp3_flac_and_matroska_pcm.rs`).
- **FVID-0643** — Validate non-empty `transform.interval` during Matroska PCM `geometry_for_decode` (`crates/fvid-media/src/owned_matroska_pcm_export.rs`, `tests/audio_plan_mp3_flac_and_matroska_pcm.rs`).
- **FVID-0644** — Use `track_languages` (`LanguageIETF`) in `matroska_source_info_with_interval` for WAVE export (`crates/fvid-media/src/owned_wave_metadata.rs`, `tests/audio_plan_mp3_flac_and_matroska_pcm.rs`).

See `batch-582-validation.json`.

## Batch 583 — FVID-0645..0649: `max_packets`-bounded interval prefix checks for Ogg Opus, Ogg Vorbis, and linked Ogg Joint programs, `max_packets`-bounded G.711 WAVE spool budget in `apply`, and actual G.711 frame count in `plan_decode_audio` Stage-2 budget check

- **FVID-0645** — Bound Ogg Opus interval prefix check by `options.max_packets` in `open_with_transform` (`crates/fvid-media/src/owned_ogg_opus.rs`, `crates/fvid-media/src/owned_ogg_opus_export.rs`, `tests/ogg_packet_limit_interval_and_g711_budget.rs`).
- **FVID-0646** — Bound Ogg Vorbis interval prefix check by `options.max_packets` in `open_with_transform` (`crates/fvid-media/src/owned_ogg_vorbis.rs`, `crates/fvid-media/src/owned_ogg_vorbis_export.rs`, `tests/ogg_packet_limit_interval_and_g711_budget.rs`).
- **FVID-0647** — Bound linked Ogg Joint interval prefix check by `options.max_packets` in `open_with_transform` (`crates/fvid-media/src/owned_ogg_audio_program.rs`, `crates/fvid-media/src/owned_ogg_joint_export.rs`, `tests/ogg_packet_limit_interval_and_g711_budget.rs`).
- **FVID-0648** — Bound G.711 WAVE spool export budget by `options.max_packets` in `apply` (`crates/fvid-media/src/owned_g711_wave_export.rs`, `tests/ogg_packet_limit_interval_and_g711_budget.rs`).
- **FVID-0649** — Use actual G.711 frame count in `plan_decode_audio` Stage-2 spool budget check (`crates/fvid-media/src/owned_audio_plan.rs`, `tests/ogg_packet_limit_interval_and_g711_budget.rs`).

See `batch-583-validation.json`.

## Batch 584 — FVID-0650..0654: Canonical RIFF `INFO` preservation past 64 custom Vorbis comments in FLAC and Ogg WAVE exports, and actual prefix sample-frame Stage-2 WAVE spool budget admission for Ogg, bare FLAC/MPEG, and ADTS AAC

- **FVID-0650** — Preserve canonical RIFF `INFO` tags past 64 custom Vorbis comments in bare FLAC WAVE export (`crates/fvid-media/src/owned_flac_layout.rs`, `tests/flac_ogg_info_cap_and_spool_budget.rs`).
- **FVID-0651** — Preserve canonical RIFF `INFO` tags past 64 custom Vorbis comments in Ogg WAVE export (`crates/fvid-media/src/owned_ogg_opus_export.rs`, `tests/flac_ogg_info_cap_and_spool_budget.rs`).
- **FVID-0652** — Enforce actual prefix sample frames and metadata size in Ogg Stage-2 WAVE spool budget (`crates/fvid-media/src/owned_ogg_audio_export.rs`, `crates/fvid-media/src/owned_ogg_opus_export.rs`, `crates/fvid-media/src/owned_ogg_vorbis_export.rs`, `crates/fvid-media/src/owned_ogg_joint_export.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/flac_ogg_info_cap_and_spool_budget.rs`).
- **FVID-0653** — Enforce actual prefix sample frames in bare FLAC and bare MPEG Stage-2 WAVE spool budget in `apply` (`crates/fvid-media/src/owned_raw_flac.rs`, `crates/fvid-media/src/owned_raw_mp3.rs`, `tests/flac_ogg_info_cap_and_spool_budget.rs`).
- **FVID-0654** — Enforce actual decoded prefix sample frames in ADTS AAC Stage-2 WAVE spool budget (`crates/fvid-media/src/owned_aac/stream.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/flac_ogg_info_cap_and_spool_budget.rs`).

See `batch-584-validation.json`.

## Batch 585 — FVID-0655..0659: `max_packets`-bounded Ogg frame estimation in `audio_plan_with_controls`, Ogg and bare MPEG `probe_with_options` audio packet work alignment, untitled Matroska chapter metadata parity in `src/media_probe.rs`, and actual prefix frame accounting in `check_spool_loudnorm_budget`

- **FVID-0655** — Bound Ogg Opus, Vorbis, and Joint frame estimates by `controls.max_packets` in `audio_plan_with_controls` (`crates/fvid-media/src/owned_audio_mix.rs`, `tests/mix_probe_chapter_and_loudnorm_budget.rs`).
- **FVID-0656** — Check Ogg audio packet work instead of framing header packet count in `probe_ogg_with_options` (`crates/fvid-media/src/owned_probe.rs`, `tests/mix_probe_chapter_and_loudnorm_budget.rs`).
- **FVID-0657** — Check active presentation frames plus leading `Info` packet against `max_packets` in `probe_mpeg_with_options` (`crates/fvid-media/src/owned_mpeg_probe.rs`, `tests/mix_probe_chapter_and_loudnorm_budget.rs`).
- **FVID-0658** — Omit empty `title` metadata for untitled Matroska chapters in frontend `media_probe` (`src/media_probe.rs`, `tests/mix_probe_chapter_and_loudnorm_budget.rs`).
- **FVID-0659** — Enforce actual prefix sample frames and metadata size in `check_spool_loudnorm_budget` (`crates/fvid-media/src/owned_loudnorm.rs`, `crates/fvid-media/src/owned_container_loudnorm.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/mix_probe_chapter_and_loudnorm_budget.rs`).

See `batch-585-validation.json`.

## Batch 586 — FVID-0660..0664: Loudness histogram bin memory and negotiated ADTS geometry in `check_meter_budget`, negotiated ADTS geometry and prefix sample frames in `owned_adts_loudnorm`, G.711 / MP4 / Matroska Stage-2 metadata working set in `source_descriptor_for_decode`, and ADTS mix/merge output buffer budget in `audio_plan_with_controls`

- **FVID-0660** — Include loudness histogram bin memory and negotiated ADTS geometry in `check_meter_budget` (`crates/fvid-media/src/owned_container_loudness.rs`, `crates/fvid-media/src/owned_adts_loudness.rs`, `tests/loudness_histogram_adts_and_spool_meta.rs`).
- **FVID-0661** — Enforce negotiated ADTS geometry and actual prefix sample frames in `owned_adts_loudnorm` spool budget (`crates/fvid-media/src/owned_aac/stream.rs`, `crates/fvid-media/src/owned_adts_loudnorm.rs`, `tests/loudness_histogram_adts_and_spool_meta.rs`).
- **FVID-0662** — Propagate G.711 WAVE RIFF `INFO` metadata size into `source_descriptor_for_decode` Stage-2 budget (`crates/fvid-media/src/owned_g711_wave_export.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/loudness_histogram_adts_and_spool_meta.rs`).
- **FVID-0663** — Populate Stage-2 WAVE metadata bytes for MP4 and Matroska audio inputs in `source_descriptor_for_decode` (`crates/fvid-media/src/owned_audio_plan.rs`, `tests/loudness_histogram_adts_and_spool_meta.rs`).
- **FVID-0664** — Include ADTS decoded prefix frame estimates in `audio_plan_with_controls` mix and merge budget (`crates/fvid-media/src/owned_audio_mix.rs`, `tests/loudness_histogram_adts_and_spool_meta.rs`).

See `batch-586-validation.json`.

## Batch 587 — FVID-0665..0669: MP4 and Matroska actual prefix sample frames in Stage-2 WAVE spool / loudness / loudnorm / mix / merge budgets, identity ADTS Stage-2 WAVE spool budget enforcement, and G.711 WAVE Stage-2 decode spool enforcement in `audio_plan_with_controls`

- **FVID-0665** — Enforce actual prefix sample frames in MP4 audio Stage-2 WAVE spool, loudness, and loudnorm budgets (`crates/fvid-media/src/owned_mp4_audio_export.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `crates/fvid-media/src/owned_container_loudnorm.rs`, `tests/container_stage2_frames_and_spool_budget.rs`).
- **FVID-0666** — Enforce actual prefix sample frames in Matroska audio Stage-2 WAVE spool, loudness, and loudnorm budgets (`crates/fvid-media/src/owned_matroska_audio_timeline_impl.rs`, `crates/fvid-media/src/owned_matroska_pcm_export.rs`, `crates/fvid-media/src/owned_matroska_aac_export.rs`, `crates/fvid-media/src/owned_matroska_flac_export.rs`, `crates/fvid-media/src/owned_matroska_opus_export.rs`, `crates/fvid-media/src/owned_matroska_alac_export.rs`, `crates/fvid-media/src/owned_matroska_mp3_export.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `crates/fvid-media/src/owned_container_loudnorm.rs`, `tests/container_stage2_frames_and_spool_budget.rs`).
- **FVID-0667** — Enforce MP4 and Matroska Stage-2 frame estimates and float32 precision in `audio_plan_with_controls` (`crates/fvid-media/src/owned_audio_mix.rs`, `tests/container_stage2_frames_and_spool_budget.rs`).
- **FVID-0668** — Enforce Stage-2 WAVE spool budget on identity-transform metadata-free ADTS AAC inputs (`crates/fvid-media/src/owned_adts_export.rs`, `tests/container_stage2_frames_and_spool_budget.rs`).
- **FVID-0669** — Enforce Stage-2 WAVE decode spool budget for G.711 WAVE inputs in `audio_plan_with_controls` (`crates/fvid-media/src/owned_audio_mix.rs`, `tests/container_stage2_frames_and_spool_budget.rs`).

See `batch-587-validation.json`.

## Batch 588 — FVID-0670..0674: Standalone audio container execution in `owned_container_loudnorm::apply`, monochrome `shuffleplanes` refusal in `process_frame_with_overlay`, alpha/crop/mask working memory and upfront destination check in `owned_subtitle_burn`, source Matroska tag sanitization and dry-run metadata validation in `owned_subtitle`, and ADTS stream tag cap and deferred 0-packet progress in `owned_matroska_adts_impl`

- **FVID-0670** — Support standalone audio containers (`G.711 WAVE`, `Ogg`, `bare FLAC`, `bare MPEG`) in `owned_container_loudnorm::apply` (`crates/fvid-media/src/owned_container_loudnorm.rs`, `tests/standalone_loudnorm_gray_shuffleplanes_and_subtitles.rs`).
- **FVID-0671** — Refuse `shuffleplanes` on monochrome video frames consistently in `process_frame_with_overlay` (`crates/fvid-media/src/owned_video_decode.rs`, `tests/standalone_loudnorm_gray_shuffleplanes_and_subtitles.rs`).
- **FVID-0672** — Charge alpha, display-geometry, and mask working memory in subtitle burn planning and execution and reject existing outputs upfront (`crates/fvid-media/src/owned_subtitle_burn.rs`, `tests/standalone_loudnorm_gray_shuffleplanes_and_subtitles.rs`).
- **FVID-0673** — Filter overlong or NUL-containing source Matroska tags in subtitle conversion and validate `PacketWriter` metadata during `dry_run` (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/standalone_loudnorm_gray_shuffleplanes_and_subtitles.rs`).
- **FVID-0674** — Enforce 256-tag Matroska stream metadata cap in `extract_adts_matroska_metadata` and defer 0-packet ADTS progress until packet admission (`crates/fvid-media/src/owned_matroska_adts_impl.rs`, `tests/standalone_loudnorm_gray_shuffleplanes_and_subtitles.rs`).

See `batch-588-validation.json`.

## Batch 589 — FVID-0675..0679: Decompressed zlib payload budget and deferred 0-packet progress in `owned_matroska_copy`, `plan_xfade` colour-range/metadata dry-run validation and upfront existing-output check in `owned_xfade`, deferred 0-packet progress in `owned_wave_remux`, source MP4 and per-track scoped metadata working set in `owned_mp4_multitrack_export::qualify`, and upfront existing-output and preflight validation in `native_export` Y4M export/concat

- **FVID-0675** — Charge decompressed zlib packet payload and read overhead in Matroska identity copy and defer 0-packet progress until preflight succeeds (`crates/fvid-media/src/owned_matroska_copy_impl.rs`, `tests/matroska_zlib_copy_xfade_wave_and_y4m_preflight.rs`).
- **FVID-0676** — Validate Y4M colour range, pixel aspect, and Matroska `PacketWriter` metadata in `plan_xfade` and check existing outputs upfront in `xfade_video` (`crates/fvid-media/src/owned_xfade.rs`, `tests/matroska_zlib_copy_xfade_wave_and_y4m_preflight.rs`).
- **FVID-0677** — Defer initial 0-packet WAVE remux and concat progress until preflight and temporary reservation succeed (`crates/fvid-media/src/owned_wave_remux.rs`, `tests/matroska_zlib_copy_xfade_wave_and_y4m_preflight.rs`).
- **FVID-0678** — Include source MP4 metadata and per-track scoped stream tags in multitrack MP4 lossless qualification budget (`crates/fvid-media/src/owned_mp4_multitrack_export.rs`, `tests/matroska_zlib_copy_xfade_wave_and_y4m_preflight.rs`).
- **FVID-0679** — Reject existing outputs upfront and defer 0-packet progress until preflight in native Y4M export and concat (`src/native_export.rs`, `tests/matroska_zlib_copy_xfade_wave_and_y4m_preflight.rs`).

See `batch-589-validation.json`.

## Batch 590 — FVID-0680..0684: 256-tag Matroska cap and audio title/language budget in `owned_webm_multitrack_export::qualify`, upfront destination check in `overlay_video_transformed` and MP4 `©too` encoder preservation in `native_lossless::write_mp4_selected`, O(1) uniform sample table and `index_bytes + sample_size` budget in MP4 fast-start relocation, aligned Y4M overlay/FFV1 packet budget diagnostics in `owned_lossless`, and upfront destination check plus deferred 0-packet progress in WAVE PCM audio export

- **FVID-0680** — Enforce 256-tag Matroska container and stream metadata caps and audio track title/language budgets in `owned_webm_multitrack_export::qualify` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`, `tests/webm_multitrack_tags_overlay_mp4_relocate_and_pcm_preflight.rs`).
- **FVID-0681** — Check existing overlay destination upfront before scanning main input and preserve MP4 `©too` encoder tag in `native_lossless::write_mp4_selected` (`src/native_export/overlay.rs`, `src/native_lossless.rs`, `tests/webm_multitrack_tags_overlay_mp4_relocate_and_pcm_preflight.rs`).
- **FVID-0682** — Charge simultaneous MP4 index and sample working set in O(1) for uniform sample tables during MP4 fast-start relocation planning and remux (`src/media.rs`, `src/native_plan.rs`, `tests/webm_multitrack_tags_overlay_mp4_relocate_and_pcm_preflight.rs`).
- **FVID-0683** — Align Y4M lossless overlay frame working-set check, guarded frame cloning, and FFV1 packet budget diagnostics between `plan_transcode_lossless` and `transcode_lossless` (`crates/fvid-media/src/owned_lossless.rs`, `crates/fvid-media/src/owned_matroska.rs`, `tests/webm_multitrack_tags_overlay_mp4_relocate_and_pcm_preflight.rs`).
- **FVID-0684** — Reject existing outputs upfront and defer 0-packet progress until preflight succeeds in native and owned WAVE PCM audio exports (`src/native_export.rs`, `src/native_media.rs`, `crates/fvid-media/src/owned_audio_export.rs`, `tests/webm_multitrack_tags_overlay_mp4_relocate_and_pcm_preflight.rs`).

See `batch-590-validation.json`.

## Batch 591 — FVID-0685..0689: Concatenated chapter cap, track preferences, and upfront destination check in MP4-to-Matroska concat, upfront destination checks across owned/native Matroska remux and FFV1 export, upfront destination/symlink checks and fallible buffers in mix/merge/concat audio, 256-tag Matroska cap in `pcm_matroska`, and simultaneous index + packet working set in `validate_encoded_audio_packet_budget`

- **FVID-0685** — Enforce concatenated chapter limit, preserve MP4 track preferences, and check destination upfront in MP4-to-Matroska concat (`src/container/mp4_concat.rs`, `src/native_export.rs`, `tests/mp4_concat_chapters_remux_preflight_and_pcm_tags.rs`).
- **FVID-0686** — Reject existing destinations upfront before opening or scanning inputs in owned and native Matroska remux and FFV1 export (`crates/fvid-media/src/owned_mp4_remux.rs`, `src/native_export.rs`, `tests/mp4_concat_chapters_remux_preflight_and_pcm_tags.rs`).
- **FVID-0687** — Check destination and broken symlinks upfront before scanning inputs and use fallible buffers in mix, merge, and concat audio (`src/native_audio_mix.rs`, `src/native_audio_mix/concat.rs`, `tests/mp4_concat_chapters_remux_preflight_and_pcm_tags.rs`).
- **FVID-0688** — Enforce 256-tag Matroska container and stream metadata caps and fallback probe metadata memory budget in `pcm_matroska` (`src/native_export/pcm_matroska.rs`, `tests/mp4_concat_chapters_remux_preflight_and_pcm_tags.rs`).
- **FVID-0689** — Charge simultaneous index and packet working set in `validate_encoded_audio_packet_budget` for MP4 and Matroska audio planning (`src/native_plan.rs`, `tests/mp4_concat_chapters_remux_preflight_and_pcm_tags.rs`).

See `batch-591-validation.json`.

## Batch 592 — FVID-0690..0694: Upfront destination checks and direct output filling in owned audio mix/merge, upfront destination checks and fallible block buffers in loudnorm and normalize-loudness, deferred progress and fallible rings in WAVE loudness and `LoudnessMeter`, upfront destination checks and fallible buffers in subtitle conversion, and upfront destination checks in WAVE trim/concat and ADTS remux/concat

- **FVID-0690** — Reject existing destinations upfront and eliminate redundant intermediate `Vec<f32>` allocations in owned audio mix and merge (`crates/fvid-media/src/owned_audio_mix.rs`, `src/media.rs`, `tests/audio_mix_loudnorm_subtitle_and_wave_preflight.rs`).
- **FVID-0691** — Reject existing destinations upfront before pass-1 measurement or WAVE processing and use fallibly reserved block buffers in loudnorm and normalize-loudness (`crates/fvid-media/src/owned_loudnorm.rs`, `src/native_loudnorm.rs`, `src/native_pcm/normalize.rs`, `tests/audio_mix_loudnorm_subtitle_and_wave_preflight.rs`).
- **FVID-0692** — Defer 0-packet WAVE loudness progress until preflight and use fallible allocations in `measure_for_normalization` and `LoudnessMeter::new` (`crates/fvid-media/src/owned_wave_loudness.rs`, `crates/fvid-media/src/owned_loudness.rs`, `tests/audio_mix_loudnorm_subtitle_and_wave_preflight.rs`).
- **FVID-0693** — Reject existing subtitle destinations upfront before reading inputs and use fallible source buffers in ASS and SRT conversion (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/audio_mix_loudnorm_subtitle_and_wave_preflight.rs`).
- **FVID-0694** — Reject existing destinations upfront in native WAVE trim/concat, native and owned ADTS remux/concat, and owned WAVE audio export (`src/native_pcm.rs`, `src/native_export.rs`, `crates/fvid-media/src/owned_adts_remux.rs`, `crates/fvid-media/src/owned_concat.rs`, `crates/fvid-media/src/owned_audio_export.rs`, `tests/audio_mix_loudnorm_subtitle_and_wave_preflight.rs`).

See `batch-592-validation.json`.

## Batch 593 — FVID-0695..0699: Upfront destination check and fallible buffers in `decode_audio_transformed`, fallible buffers in G.711 WAVE and Ogg Vorbis/joint audio exports, single-pass metadata extraction in raw FLAC/MP3 export and planning, upfront destination check in container/ADTS loudnorm, and fallible `Vec<u8>` output reservation in `Resampler`

- **FVID-0695** — Reject existing destinations before container recognition and use fallible allocations in `decode_audio_transformed` (`crates/fvid-media/src/owned_audio_export.rs`, `tests/audio_export_preflight_and_resampler_fallible.rs`).
- **FVID-0696** — Use fallible buffers and reusable packet scratch in G.711 WAVE and Ogg Vorbis/joint audio exports (`crates/fvid-media/src/owned_g711_wave_export.rs`, `crates/fvid-media/src/owned_ogg_audio_export.rs`, `crates/fvid-media/src/owned_ogg_vorbis_export.rs`, `crates/fvid-media/src/owned_ogg_joint_export.rs`, `tests/audio_export_preflight_and_resampler_fallible.rs`).
- **FVID-0697** — Eliminate redundant full-file re-reads for WAVE metadata in raw FLAC and MP3 export and audio planning (`crates/fvid-media/src/owned_raw_flac.rs`, `crates/fvid-media/src/owned_raw_mp3.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/audio_export_preflight_and_resampler_fallible.rs`).
- **FVID-0698** — Reject invalid args, non-`.wav` extensions, and existing destinations before policy checks in container and ADTS loudnorm (`crates/fvid-media/src/owned_container_loudnorm.rs`, `crates/fvid-media/src/owned_adts_loudnorm.rs`, `tests/audio_export_preflight_and_resampler_fallible.rs`).
- **FVID-0699** — Use fallible `Vec<u8>` output reservation in float32 and float64 streaming resamplers (`crates/fvid-media/src/owned_resample.rs`, `crates/fvid-media/src/owned_resample_f64.rs`, `tests/audio_export_preflight_and_resampler_fallible.rs`).

See `batch-593-validation.json`.

## Batch 594 — FVID-0700..0704: Fallible single-pass PCM f32/f64 decode, fallible MACE planar and output reservation, fallible WebM audio window position reservation, upfront destination check in WebM multitrack export, and O(1) per-packet schedule budget check and memory-budget error attribution in MP4 multitrack export

- **FVID-0700** — Use fallible `try_reserve_exact` and single-pass inline finiteness validation in `PcmDecoder::decode_pcm` and `PcmDecoder::decode_pcm_f64` (`crates/fvid-media/src/owned_pcm_decoder_impl.rs`, `tests/pcm_mace_and_multitrack_export_preflight.rs`).
- **FVID-0701** — Use checked arithmetic and fallible `try_reserve_exact` for planar and interleaved output buffers in owned and playback MACE decoders (`crates/fvid-media/src/owned_mace_impl.rs`, `src/codec/mace_decoder.rs`, `tests/pcm_mace_and_multitrack_export_preflight.rs`).
- **FVID-0702** — Pre-reserve packet position table with `try_reserve_exact` in `owned_webm_audio_window::apply` (`crates/fvid-media/src/owned_webm_audio_window.rs`, `tests/pcm_mace_and_multitrack_export_preflight.rs`).
- **FVID-0703** — Check `.mkv` extension and existing destination upfront before opening or decoding input video in `owned_webm_multitrack_export::try_export` (`crates/fvid-media/src/owned_webm_multitrack_export.rs`, `tests/pcm_mace_and_multitrack_export_preflight.rs`).
- **FVID-0704** — Precompute `schedule_base_bytes` once outside the packet loop and attribute memory-budget-clamped packet rejections in `owned_mp4_multitrack_export` (`crates/fvid-media/src/owned_mp4_multitrack_export.rs`, `tests/multitrack_lossless_budget_parity_regressions.rs`).

See `batch-594-validation.json`.

## Batch 595 — FVID-0705..0709: Fallible stream and chapter reservation in WebM probe, single-open header qualification in Y4M `supports_transformed`, single-open header reuse in `try_y4m_with_options`, fallible frame buffer in `encode_y4m_counted`, and upfront destination check in `transcode_lossless`

- **FVID-0705** — Pre-reserve stream and chapter lists with `try_reserve_exact` in `owned_webm_probe::try_probe_webm_with_limits` (`crates/fvid-media/src/owned_webm_probe.rs`, `tests/y4m_probe_and_lossless_preflight.rs`).
- **FVID-0706** — Read and parse the primary Y4M header once per call in `owned_y4m_decode::supports_transformed` (`crates/fvid-media/src/owned_y4m_decode.rs`, `tests/y4m_probe_and_lossless_preflight.rs`).
- **FVID-0707** — Reuse the already-opened reader and header buffer in `owned_y4m_probe::try_y4m_with_options` (`crates/fvid-media/src/owned_y4m_probe.rs`, `tests/y4m_probe_and_lossless_preflight.rs`).
- **FVID-0708** — Use fallible frame buffer allocation in `owned_ffv1_encoder::encode_y4m_counted` (`crates/fvid-media/src/owned_ffv1_encoder.rs`, `tests/y4m_probe_and_lossless_preflight.rs`).
- **FVID-0709** — Reject existing destination upfront in `owned_lossless::transcode_lossless` before transform and container validation (`crates/fvid-media/src/owned_lossless.rs`, `tests/y4m_probe_and_lossless_preflight.rs`).

See `batch-595-validation.json`.

## Batch 596 — FVID-0710..0714: Fallible buffers in WAVE, Ogg, and MP4 probes, upfront destination checks in Matroska remux, ADTS concat, and WAVE remux/trim/concat, and single-pass parsed reuse in bare FLAC and MP3 WAVE exports

- **FVID-0710** — Use fallible buffer and `try_reserve_exact` allocations in WAVE and Ogg probe paths (`crates/fvid-media/src/owned_probe.rs`, `tests/probe_remux_and_raw_audio_single_pass.rs`).
- **FVID-0711** — Pre-reserve stream and chapter lists with `try_reserve_exact` in `owned_mp4_probe::try_mp4_with_options` (`crates/fvid-media/src/owned_mp4_probe_impl.rs`, `tests/probe_remux_and_raw_audio_single_pass.rs`).
- **FVID-0712** — Reject existing destinations upfront before `supports()` in `owned_matroska_remux::remux` and `owned_concat::concat` and use fallible reader reservation (`crates/fvid-media/src/owned_matroska_remux.rs`, `crates/fvid-media/src/owned_concat.rs`, `tests/probe_remux_and_raw_audio_single_pass.rs`).
- **FVID-0713** — Reject existing destinations and non-`.wav` extensions upfront in `owned_wave_remux` and pre-reserve segment descriptors with `try_reserve_exact` (`crates/fvid-media/src/owned_wave_remux.rs`, `tests/probe_remux_and_raw_audio_single_pass.rs`).
- **FVID-0714** — Reuse already-opened and parsed `Flac` and `Mp3` structures during spooled WAVE exports in `owned_raw_flac::apply` and `owned_raw_mp3::apply` (`crates/fvid-media/src/owned_raw_flac.rs`, `crates/fvid-media/src/owned_raw_mp3.rs`, `crates/fvid-media/src/owned_flac.rs`, `crates/fvid-media/src/owned_mpeg_audio.rs`, `tests/probe_remux_and_raw_audio_single_pass.rs`).

See `batch-596-validation.json`.

## Batch 597 — FVID-0715..0719: Single-pass Ogg Vorbis `Program::from_ogg_source_track`, fallible Ogg Opus/Vorbis input reservation, single-open G.711 WAVE codec descriptor, fallible Y4M frame buffers in subtitle burn-in, and unified Matroska FLAC geometry and channel mask extraction

- **FVID-0715** — Reuse the already-parsed `Ogg` container in `owned_ogg_vorbis_export::open_with_transform` without re-opening or re-reading `source` (`crates/fvid-media/src/owned_ogg_vorbis_export.rs`, `crates/fvid-media/src/owned_ogg_vorbis.rs`, `tests/ogg_g711_subtitle_burn_and_flac_mask.rs`).
- **FVID-0716** — Pre-reserve Ogg file byte buffers with fallible `try_reserve_exact` in `owned_ogg_opus::Program::open_selected` and `owned_ogg_vorbis::Program::read` (`crates/fvid-media/src/owned_ogg_opus.rs`, `crates/fvid-media/src/owned_ogg_vorbis.rs`, `tests/ogg_g711_subtitle_burn_and_flac_mask.rs`).
- **FVID-0717** — Record the G.711 codec name during `owned_g711_wave_export::open` instead of re-opening `source` and re-inspecting `g711_law` in `descriptor_for_decode` (`crates/fvid-media/src/owned_g711_wave_export.rs`, `tests/ogg_g711_subtitle_burn_and_flac_mask.rs`).
- **FVID-0718** — Use fallible frame buffer allocation for Y4M frames in `owned_subtitle_burn::plan_burn_subtitles` and `burn_subtitles` (`crates/fvid-media/src/owned_subtitle_burn.rs`, `tests/ogg_g711_subtitle_burn_and_flac_mask.rs`).
- **FVID-0719** — Unify geometry, `channel_mask`, and frame estimation in `owned_matroska_flac_export::geometry_with_mask_and_frames_for_decode` (`crates/fvid-media/src/owned_matroska_flac_export.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/ogg_g711_subtitle_burn_and_flac_mask.rs`).

See `batch-597-validation.json`.

## Batch 598 — FVID-0720..0724: Fallible Ogg audio catalog and packet assembly reservation, chunked Matroska/MP4 timeline PCM writes, and single-pass MP4 audio descriptor + frame estimation

- **FVID-0720** — Pre-reserve Ogg audio catalog byte buffer with fallible `try_reserve_exact` before `read_to_end` (`crates/fvid-media/src/owned_ogg_audio_catalog.rs`, `tests/ogg_catalog_and_timeline_pcm_chunking.rs`).
- **FVID-0721** — Use fallible `try_reserve_exact` for Ogg continued and uncontinued packet assembly in `Ogg::parse` (`crates/fvid-media/src/owned_ogg.rs`, `tests/ogg_catalog_and_timeline_pcm_chunking.rs`).
- **FVID-0722** — Batch-pack Matroska timeline PCM output into a reusable fallible scratch buffer and skip cloning non-selected track packets (`crates/fvid-media/src/owned_matroska_audio_timeline_impl.rs`, `tests/ogg_catalog_and_timeline_pcm_chunking.rs`).
- **FVID-0723** — Batch-pack MP4 timeline PCM output into a reusable fallible scratch buffer instead of writing one sample at a time (`crates/fvid-media/src/owned_mp4_audio_timeline_impl.rs`, `tests/ogg_catalog_and_timeline_pcm_chunking.rs`).
- **FVID-0724** — Reuse the opened MP4 reader across `descriptor_with_frames_for_decode` and `owned_audio_plan::source_descriptor_for_decode` (`crates/fvid-media/src/owned_mp4_audio_export.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/ogg_catalog_and_timeline_pcm_chunking.rs`).

See `batch-598-validation.json`.

## Batch 599 — FVID-0725..0729: Single-pass Ogg audio descriptor, reuse opened MP4/Matroska readers for RIFF INFO metadata, and single-open ADTS reader in `owned_adts_export`

- **FVID-0725** — Reuse the opened and parsed `Ogg` container in `owned_ogg_audio_export::descriptor_for_decode` (`crates/fvid-media/src/owned_ogg_audio_export.rs`, `crates/fvid-media/src/owned_ogg_opus.rs`, `crates/fvid-media/src/owned_ogg_opus_export.rs`, `crates/fvid-media/src/owned_ogg_vorbis_export.rs`, `crates/fvid-media/src/owned_ogg_joint_export.rs`, `tests/ogg_mp4_matroska_adts_single_pass_info.rs`).
- **FVID-0726** — Extract `mp4_source_info_from_reader` and reuse it across `owned_mp4_audio_export` and `owned_audio_plan` without re-opening and re-indexing the MP4 file (`crates/fvid-media/src/owned_wave_metadata.rs`, `crates/fvid-media/src/owned_mp4_audio_export.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/ogg_mp4_matroska_adts_single_pass_info.rs`).
- **FVID-0727** — Extract `matroska_source_info_from_reader` so Matroska audio geometry/frame preflight reuses the already-opened `WebmReader` (`crates/fvid-media/src/owned_wave_metadata.rs`, `crates/fvid-media/src/owned_matroska_aac_export.rs`, `crates/fvid-media/src/owned_matroska_opus_export.rs`, `crates/fvid-media/src/owned_matroska_flac_export.rs`, `crates/fvid-media/src/owned_matroska_alac_export.rs`, `crates/fvid-media/src/owned_matroska_mp3_export.rs`, `crates/fvid-media/src/owned_matroska_pcm_export.rs`, `tests/ogg_mp4_matroska_adts_single_pass_info.rs`).
- **FVID-0728** — Return RIFF `INFO` metadata bytes alongside Matroska geometry and frame estimates to eliminate redundant Matroska opens in `apply` and `owned_audio_plan` (`crates/fvid-media/src/owned_audio_plan.rs`, `crates/fvid-media/src/owned_matroska_aac_export.rs`, `crates/fvid-media/src/owned_matroska_opus_export.rs`, `crates/fvid-media/src/owned_matroska_flac_export.rs`, `crates/fvid-media/src/owned_matroska_alac_export.rs`, `crates/fvid-media/src/owned_matroska_mp3_export.rs`, `crates/fvid-media/src/owned_matroska_pcm_export.rs`, `tests/ogg_mp4_matroska_adts_single_pass_info.rs`).
- **FVID-0729** — Reuse the opened `AdtsStreamReader` in `owned_adts_export::supports` and `decode_to_wave_with_rate` via `decode_adts_reader_pcm` (`crates/fvid-media/src/owned_adts_export.rs`, `crates/fvid-media/src/owned_aac/stream.rs`, `tests/ogg_mp4_matroska_adts_single_pass_info.rs`).

See `batch-599-validation.json`.

## Batch 600 — FVID-0730..0734: Single-pass Ogg Opus RIFF INFO reuse, single-pass ADTS loudness/loudnorm header validation, container loudness/loudnorm geometry reuse, single-pass WAVE loudnorm header inspection, and single-open AAC export routing

- **FVID-0730** — Return RIFF `INFO` bytes from `open_with_transform_and_info` and `open_with_info` in `owned_ogg_opus_export` so `decode_to_wave` and `apply` do not re-parse `OpusTags` (`crates/fvid-media/src/owned_ogg_opus_export.rs`, `tests/loudnorm_and_aac_routing_single_open.rs`).
- **FVID-0731** — Return validated channels alongside sample rate in ADTS loudness preflight and eliminate redundant `StreamReader` opens in ADTS loudnorm (`crates/fvid-media/src/owned_adts_loudness.rs`, `crates/fvid-media/src/owned_adts_loudnorm.rs`, `tests/loudnorm_and_aac_routing_single_open.rs`).
- **FVID-0732** — Reuse validated container geometry in `measure_loudness` and container loudnorm via `decode_to_wave_from_geometry` and `check_meter_budget` (`crates/fvid-media/src/owned_container_loudness.rs`, `crates/fvid-media/src/owned_container_loudnorm.rs`, `tests/loudnorm_and_aac_routing_single_open.rs`).
- **FVID-0733** — Inspect WAVE headers once in `owned_loudnorm::supports` and `supports_dual` instead of opening and inspecting the source file twice (`crates/fvid-media/src/owned_loudnorm.rs`, `tests/loudnorm_and_aac_routing_single_open.rs`).
- **FVID-0734** — Seek and reuse the already-opened `File` handle after prefix inspection in `owned_aac_export_routing::router_for_decode` (`crates/fvid-media/src/owned_aac_export_routing.rs`, `tests/loudnorm_and_aac_routing_single_open.rs`).

See `batch-600-validation.json`.

## Batch 601 — FVID-0735..0739: Reuse `stage2_frames` in `audio_plan_with_controls`, single-open ADTS and Matroska in `owned_audio_plan`, single-pass WAVE header/INFO inspection in `decode_audio_transformed`, and single-pass Ogg `open_selected_ogg` across export and loudness

- **FVID-0735** — Reuse `info.stage2_frames` from `source_descriptor` in `owned_audio_mix::audio_plan_with_controls` instead of re-opening and re-parsing G.711 WAVE, Ogg, FLAC, MP3, and Matroska inputs (`crates/fvid-media/src/owned_audio_mix.rs`, `tests/mix_plan_and_wave_ogg_single_pass.rs`).
- **FVID-0736** — Reuse the opened ADTS `StreamReader` via `adts_prefix_info_from_reader_for_transform` in `owned_audio_plan::source_descriptor_for_decode` (`crates/fvid-media/src/owned_audio_plan.rs`, `crates/fvid-media/src/owned_aac/stream.rs`, `tests/mix_plan_and_wave_ogg_single_pass.rs`).
- **FVID-0737** — Seek and reuse the already-opened `File` handle after EBML signature check in `owned_audio_plan::matroska_descriptor_for_decode` (`crates/fvid-media/src/owned_audio_plan.rs`, `tests/mix_plan_and_wave_ogg_single_pass.rs`).
- **FVID-0738** — Retain `WaveInfo` and RIFF `INFO` chunks during `read_wave_with_admission` in `owned_audio_export::decode_audio_transformed` instead of opening and inspecting the WAVE file up to three times (`crates/fvid-media/src/owned_audio_export.rs`, `tests/mix_plan_and_wave_ogg_single_pass.rs`).
- **FVID-0739** — Reuse the parsed `Ogg` container from `open_selected_ogg` across `owned_audio_export::supports`, `decode_audio_transformed`, and `owned_container_loudness::decode_to_wave_from_geometry` (`crates/fvid-media/src/owned_ogg_audio_export.rs`, `crates/fvid-media/src/owned_audio_export.rs`, `crates/fvid-media/src/owned_container_loudness.rs`, `crates/fvid-media/src/owned_ogg_opus_export.rs`, `crates/fvid-media/src/owned_ogg_vorbis_export.rs`, `crates/fvid-media/src/owned_ogg_joint_export.rs`, `tests/mix_plan_and_wave_ogg_single_pass.rs`).

See `batch-601-validation.json`.

## Batch 602 — FVID-0740..0744: Single-pass Ogg probe container reuse, single-pass WAVE probe G.711 law and cancellation propagation, reuse `source.file_bytes()` in FLAC/MPEG probes, eliminate redundant WAVE remux `supports` check in `trim`, and single-open unhinted signature probing

- **FVID-0740** — Reuse the already-parsed `Ogg` container via `Program::from_ogg_source_track` and drop raw file `bytes` early in `owned_probe::probe_ogg_with_options` (`crates/fvid-media/src/owned_probe.rs`, `tests/probe_and_wave_remux_single_pass.rs`).
- **FVID-0741** — Pass the already-evaluated G.711 law and `options.cancel` into `inspect_probe_with_law` in `owned_probe::probe_wave_with_options` instead of scanning the WAVE header a second time (`crates/fvid-media/src/owned_wave_inspect.rs`, `crates/fvid-media/src/owned_probe.rs`, `tests/probe_and_wave_remux_single_pass.rs`).
- **FVID-0742** — Reuse `source.file_bytes()` in `owned_flac_probe::probe_flac_with_options` and `owned_mpeg_probe::probe_mpeg_with_options` instead of calling `std::fs::metadata(path)` a second time (`crates/fvid-media/src/owned_flac_probe.rs`, `crates/fvid-media/src/owned_mpeg_probe.rs`, `tests/probe_and_wave_remux_single_pass.rs`).
- **FVID-0743** — Remove the redundant `supports(source, destination, options)` header inspection before `copy_wave` in `owned_wave_remux::trim` (`crates/fvid-media/src/owned_wave_remux.rs`, `tests/probe_and_wave_remux_single_pass.rs`).
- **FVID-0744** — Unify RIFF/RIFX/RF64 WAVE and other container prefix checks into a single 12-byte read in `owned_probe::probe_as_with_options` (`crates/fvid-media/src/owned_probe.rs`, `tests/probe_and_wave_remux_single_pass.rs`).

See `batch-602-validation.json`.

## Batch 603 — FVID-0745..0749: Single-pass WAVE `plan_trim`, single-open G.711 `recognizes` and empty INFO skip, single-open bare FLAC/MP3 `decode_to_wave`, and single-descriptor container loudnorm preflight

- **FVID-0745** — Remove the redundant `supports(source, options)` WAVE header inspection before `plan` in `owned_wave_plan::plan_trim` (`crates/fvid-media/src/owned_wave_plan.rs`, `tests/wave_plan_g711_flac_mp3_loudnorm_single_pass.rs`).
- **FVID-0746** — Open `path` once in `owned_g711_wave_export::recognizes` and skip a redundant `info_chunks` RIFF scan in `open` when `metadata == 0` (`crates/fvid-media/src/owned_g711_wave_export.rs`, `tests/wave_plan_g711_flac_mp3_loudnorm_single_pass.rs`).
- **FVID-0747** — Reuse `parsed.file_bytes()` in `owned_raw_flac::decode_raw_checked` and expose `owned_raw_flac::decode_to_wave` so `owned_container_loudness` opens and parses bare FLAC once instead of twice (`crates/fvid-media/src/owned_raw_flac.rs`, `crates/fvid-media/src/owned_container_loudness.rs`, `tests/wave_plan_g711_flac_mp3_loudnorm_single_pass.rs`).
- **FVID-0748** — Reuse `parsed.file_bytes()` in `owned_raw_mp3::decode_raw_checked` and expose `owned_raw_mp3::decode_to_wave` so `owned_container_loudness` opens and parses bare MPEG audio once instead of twice (`crates/fvid-media/src/owned_raw_mp3.rs`, `crates/fvid-media/src/owned_container_loudness.rs`, `tests/wave_plan_g711_flac_mp3_loudnorm_single_pass.rs`).
- **FVID-0749** — Derive loudness rate, channel count, speaker mask, and codec directly from `source_descriptor` in `owned_container_loudnorm::apply` and `plan_loudnorm` instead of calling both `owned_container_loudness::geometry` and `source_descriptor` (`crates/fvid-media/src/owned_container_loudnorm.rs`, `crates/fvid-media/src/owned_audio_plan.rs`, `tests/wave_plan_g711_flac_mp3_loudnorm_single_pass.rs`).

See `batch-603-validation.json`.

## Batch 604 — FVID-0750..0754: Single-pass Matroska audio output memory budget checks in `media` and `native_loudnorm`, single-pass source geometry and mask reuse in `native_audio_mix` and `concat`, and single-open `is_wave_reader` + frame-aligned `Sink::write` in `native_pcm::loudness`

- **FVID-0750** — Skip redundant post-decode `check_output_memory_budget` in `decode_fvid_media_audio_to_destination` when `prechecked_channels` matches `decoded.channels` (`src/media.rs`, `tests/audio_mix_concat_loudness_single_pass.rs`).
- **FVID-0751** — Skip redundant post-normalization `check_output_memory_budget` in `native_loudnorm::try_apply` when `prechecked_channels` matches `stats.channels` (`src/native_loudnorm.rs`, `tests/audio_mix_concat_loudness_single_pass.rs`).
- **FVID-0752** — Return inspected source `(rate, channels, mask)` tuples from `validate_mix_sources_with_controls` in `native_audio_mix` so `mix_audio_with_controls` does not re-inspect all inputs for speaker masks (`src/native_audio_mix.rs`, `tests/audio_mix_concat_loudness_single_pass.rs`).
- **FVID-0753** — Return `(rate, channels, mask)` from `validate_concat_segments_internal` in `native_audio_mix::concat` so `concat_audio_with_options` does not re-inspect all segments (`src/native_audio_mix/concat.rs`, `tests/audio_mix_concat_loudness_single_pass.rs`).
- **FVID-0754** — Reuse the already-opened `BufReader<File>` via `is_wave_reader` and add a frame-aligned `chunks_exact` fast path to `Sink::write` in `native_pcm::loudness` (`src/native_pcm.rs`, `src/native_pcm/loudness.rs`, `tests/audio_mix_concat_loudness_single_pass.rs`).

See `batch-604-validation.json`.

## Batch 605 — FVID-0755..0759: Single-pass Matroska budget check across `normalize_file_with_options` and `media`, single-open WAVE INFO inspection in `extract_source_wave_info`, and single-open WAVE/MP4 reader reuse in `native_plan` and `default_weights_with_options`

- **FVID-0755** — Pass `prechecked_channels` from `normalize_file_with_options` into `decode_fvid_media_audio_to_destination_with_prechecked_channels` so Matroska output memory budget is not checked twice (`src/native_pcm/normalize.rs`, `src/media.rs`, `tests/normalize_wave_info_and_plan_single_open.rs`).
- **FVID-0756** — Reuse the already-opened `BufReader<File>` and skip a redundant `info_chunks` RIFF scan when `info_bytes == 0` in `extract_source_wave_info` (`src/native_export/pcm_matroska.rs`, `tests/normalize_wave_info_and_plan_single_open.rs`).
- **FVID-0757** — Reuse a single `File` handle across MP4 prefix inspection, `is_wave_reader`, and `native_pcm::inspect` in `native_plan::decode_audio_with_options` (`src/native_plan.rs`, `tests/normalize_wave_info_and_plan_single_open.rs`).
- **FVID-0758** — Reuse the already-opened `BufReader<File>` in `native_plan::remux_with_options` and evaluate `is_wave(source)` once in `native_plan::loudness_with_options` (`src/native_plan.rs`, `tests/normalize_wave_info_and_plan_single_open.rs`).
- **FVID-0759** — Open `source` once via `is_wave_reader` in `native_pcm::loudness::default_weights_with_options` (`src/native_pcm/loudness.rs`, `tests/normalize_wave_info_and_plan_single_open.rs`).

See `batch-605-validation.json`.

## Batch 606 — FVID-0760..0764: Single-open WAVE/Y4M/MP4/Matroska reader reuse and single-pass packet budget validation in `native_export`, and blockwise `float_buf` / zero-conversion f32 fast path in `native_pcm::decode_reader`

- **FVID-0760** — Reuse the already-opened WAVE reader and `WaveInfo` and skip a redundant `info_chunks` scan when `info_bytes == 0` in `native_export::export_pcm_selected` (`src/native_export.rs`, `src/native_pcm.rs`, `tests/export_pcm_and_y4m_single_open.rs`).
- **FVID-0761** — Open `source` once in `native_export::validate_video_packet_budget_with_geometry_filters_interval_and_limit` (`src/native_export.rs`, `tests/export_pcm_and_y4m_single_open.rs`).
- **FVID-0762** — Open `source` once in `native_export::y4m_segment_frame_upper_bound_with_options` (`src/native_export.rs`, `tests/export_pcm_and_y4m_single_open.rs`).
- **FVID-0763** — Skip the redundant second `validate_video_packet_budget_with_geometry_filters_interval_and_limit` call for the first source in `export_y4m_sources_with_options` (`src/native_export.rs`, `tests/export_pcm_and_y4m_single_open.rs`).
- **FVID-0764** — Batch-write converted float32 blocks and add a zero-conversion 32-bit float fast path in `native_pcm::decode_reader` (`src/native_pcm.rs`, `tests/export_pcm_and_y4m_single_open.rs`).

See `batch-606-validation.json`.

## Batch 607 — FVID-0765..0769: Single-open and single-scan Y4M/Matroska/MP4 lossless eligibility and overlay admission in `native_lossless_y4m`, `native_lossless`, and `native_export::overlay`

- **FVID-0765** — Open `source` once in `native_lossless_y4m::eligible` instead of up to three times (`src/native_lossless_y4m.rs`, `tests/lossless_y4m_and_overlay_single_open.rs`).
- **FVID-0766** — Validate and open Y4M/Matroska sources in a single pass via `open_lossless_source` in `native_lossless_y4m::validate_first_frame` and `write_selected` (`src/native_lossless_y4m.rs`, `tests/lossless_y4m_and_overlay_single_open.rs`).
- **FVID-0767** — Reuse a single `BufReader<File>` for the MP4 prefix check and `Mp4Reader::open` in `native_lossless::eligible` (`src/native_lossless.rs`, `tests/lossless_y4m_and_overlay_single_open.rs`).
- **FVID-0768** — Delegate `native_export::overlay::overlay_eligible` directly to `native_lossless::eligible` (`src/native_export/overlay.rs`, `tests/lossless_y4m_and_overlay_single_open.rs`).
- **FVID-0769** — Cache `planar_eligible` in `native_export::overlay::overlay_video_transformed` (`src/native_export/overlay.rs`, `tests/lossless_y4m_and_overlay_single_open.rs`).

See `batch-607-validation.json`.

## Batch 608 — FVID-0770..0774: Single-open MP4/Matroska audio source inspection in `native_media` and single-open WAVE inspection + cached `AudioSourceInfo` reuse in `native_loudnorm`

- **FVID-0770** — Open `source` once in `native_media::inspect_audio_source_selected` instead of up to four times (`src/native_media.rs`, `tests/native_media_and_loudnorm_single_open.rs`).
- **FVID-0771** — Remove redundant `is_mp4_audio_container` and `is_matroska_source` pre-opens in `native_media::is_owned_audio_source_with_interval_and_options` (`src/native_media.rs`, `tests/native_media_and_loudnorm_single_open.rs`).
- **FVID-0772** — Combine MP4 and Matroska container prefix checks into a single `File::open` in `native_media::is_owned_audio_trim_source_with_interval_and_options` (`src/native_media.rs`, `tests/native_media_and_loudnorm_single_open.rs`).
- **FVID-0773** — Reuse a single `File` handle across `is_wave_reader` and `native_pcm::inspect` in `native_loudnorm::try_plan` (`src/native_loudnorm.rs`, `tests/native_media_and_loudnorm_single_open.rs`).
- **FVID-0774** — Cache and reuse `(selected, AudioSourceInfo)` across admission and execution in `native_loudnorm::try_apply` (`src/native_loudnorm.rs`, `tests/native_media_and_loudnorm_single_open.rs`).

See `batch-608-validation.json`.

## Batch 609 — FVID-0775..0779: Single-pass lossless/overlay planning, cached loudness source info, and single-pass audio trim selection in `native_plan` and `native_export`

- **FVID-0775** — Cache `planar_eligible` in `native_plan::transcode_lossless` instead of calling `native_lossless_y4m::eligible` twice (`src/native_plan.rs`, `tests/native_plan_lossless_overlay_and_trim_single_pass.rs`).
- **FVID-0776** — Cache `constant_audio_source_info_selected_with_options` in `native_plan::loudness_with_options` (`src/native_plan.rs`, `tests/native_plan_lossless_overlay_and_trim_single_pass.rs`).
- **FVID-0777** — Remove redundant `probe(foreground)` calls in `native_plan::overlay_with_geometry_and_filters` and `decode_overlay_with_geometry_and_filters` (`src/native_plan.rs`, `tests/native_plan_lossless_overlay_and_trim_single_pass.rs`).
- **FVID-0778** — Eliminate duplicate `audio_stream_index_from_metadata` and `constant_audio_source_info_selected_with_interval_and_options` calls in `native_export::trim_audio_pcm_with_controls` (`src/native_export.rs`, `tests/native_plan_lossless_overlay_and_trim_single_pass.rs`).
- **FVID-0779** — Eliminate duplicate container inspection for non-AAC sources in `native_plan::trim_audio_with_options` (`src/native_plan.rs`, `tests/native_plan_lossless_overlay_and_trim_single_pass.rs`).

See `batch-609-validation.json`.

## Batch 610 — FVID-0780..0784: Single-open audio route & remux prefix classification, cached WAVE concat scan, and direct FFV1 transcode dispatch in `media`

- **FVID-0780** — Reuse a single `File::open` across `native_pcm::is_wave_reader` and the MP4 prefix/constant-description check in `media::uses_native_audio_route` (`src/media.rs`, `tests/media_dispatch_single_open.rs`).
- **FVID-0781** — Read the 8-byte source prefix once in `media::remux` across `max_packets == Some(0)` validation and container routing (`src/media.rs`, `tests/media_dispatch_single_open.rs`).
- **FVID-0782** — Classify Matroska, ADTS, and MP4 prefixes in a single `File::open` pass in `media::plan_remux` (`src/media.rs`, `tests/media_dispatch_single_open.rs`).
- **FVID-0783** — Cache `first_wave` during the `all_wave` scan in `media::plan_concat` instead of reopening `sources.first()` on fallback (`src/media.rs`, `tests/media_dispatch_single_open.rs`).
- **FVID-0784** — Dispatch directly to owned FFV1 exporters in `media::transcode` instead of repeating option, deband, and eligibility checks via `transcode_lossless` (`src/media.rs`, `tests/media_dispatch_single_open.rs`).

See `batch-610-validation.json`.

## Batch 611 — FVID-0785..0789: Single-open WAVE/owned-audio source inspection and blockwise channel interleaving in `native_audio_mix`

- **FVID-0785** — Reuse a single `File` handle across `is_wave_reader` and `native_pcm::inspect` in `native_audio_mix::validate_and_inspect_source_with_options` (`src/native_audio_mix.rs`, `tests/native_audio_mix_single_open.rs`).
- **FVID-0786** — Reuse a single `File` handle across `is_wave_reader` and `native_pcm::inspect` in `native_audio_mix::eligible_with_options` (`src/native_audio_mix.rs`, `tests/native_audio_mix_single_open.rs`).
- **FVID-0787** — Inspect WAVE and owned-audio sources in a single pass in `native_audio_mix::concat::geometry_with_options` (`src/native_audio_mix/concat.rs`, `tests/native_audio_mix_single_open.rs`).
- **FVID-0788** — Reuse a single `File` handle across `is_wave_reader`, `native_pcm::inspect`, and MP4/Matroska/ADTS readers in `native_audio_mix::concat::segment_packet_count` (`src/native_audio_mix/concat.rs`, `tests/native_audio_mix_single_open.rs`).
- **FVID-0789** — Interleave channels into a contiguous block buffer and issue a single `sink.write_all` per 4096-frame block in `native_audio_mix::merge_audio_with_controls` (`src/native_audio_mix.rs`, `tests/native_audio_mix_single_open.rs`).

See `batch-611-validation.json`.

## Batch 612 — FVID-0790..0794: Single-pass Matroska subtitle planning, single-open ASS/SRT fstat, zero-allocation LF normalization, and early descriptor release in `pcm_matroska`

- **FVID-0790** — Return `(stream_index, codec_name, stats)` from `try_convert_matroska_inner` so `try_plan_with_controls` scans Matroska subtitle sources only once (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/subtitle_plan_and_convert_single_open.rs`).
- **FVID-0791** — Open `source` once and use `source_file.metadata()` in `try_convert_ass_inner` (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/subtitle_plan_and_convert_single_open.rs`).
- **FVID-0792** — Open `source` once and use `source_file.metadata()` in `try_convert_srt_inner` (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/subtitle_plan_and_convert_single_open.rs`).
- **FVID-0793** — Add `normalize_line_endings` with a zero-allocation `Cow::Borrowed` fast path for LF-only text and single-pass CRLF/CR normalization in `owned_subtitle_impl.rs` (`crates/fvid-media/src/owned_subtitle_impl.rs`, `tests/subtitle_plan_and_convert_single_open.rs`).
- **FVID-0794** — Drop the prefix `BufReader<File>` before delegating to `native_probe` in `pcm_matroska::extract_source_pcm_metadata_with_interval_packet_and_memory_limit` (`src/native_export/pcm_matroska.rs`, `tests/native_audio_mix_single_open.rs`).

See `batch-612-validation.json`.

## Batch 613 — FVID-0795..0799: Single-open FLAC/MPEG/Y4M probing and early signature descriptor release in `media_probe` and `owned_probe`

- **FVID-0795** — Drop the 12-byte signature `BufReader<File>` before delegating to container probes in `media_probe::try_probe_as_with_options` (`src/media_probe.rs`, `tests/probe_single_open.rs`).
- **FVID-0796** — Drop `signature_file` before delegating to container probes in `owned_probe::probe_as_with_options` (`crates/fvid-media/src/owned_probe.rs`, `tests/probe_single_open.rs`).
- **FVID-0797** — Validate the `fLaC` signature inside `owned_raw_flac::open_with_interval` and remove the redundant `recognizes(path)` open in `owned_flac_probe::probe_flac_with_options` (`crates/fvid-media/src/owned_flac_probe.rs`, `crates/fvid-media/src/owned_raw_flac.rs`, `tests/probe_single_open.rs`).
- **FVID-0798** — Validate the MPEG audio signature inside `owned_raw_mp3::open_with_interval` and remove the redundant `recognizes(path)` open in `owned_mpeg_probe::probe_mpeg_with_options` (`crates/fvid-media/src/owned_mpeg_probe.rs`, `crates/fvid-media/src/owned_raw_mp3.rs`, `tests/probe_single_open.rs`).
- **FVID-0799** — Read the Y4M header line once without seeking back to offset 0 in `owned_y4m_probe::try_y4m_with_options` (`crates/fvid-media/src/owned_y4m_probe.rs`, `tests/probe_single_open.rs`).

See `batch-613-validation.json`.

