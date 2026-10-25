# Verified improvements

Accepted: **281 / 1000 independent, useful changes**. Only entries marked `accepted` in
[`registry.json`](registry.json) count. Fixture variants, test cases, documentation
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

Delivery verification uses a real 1 KiB BufReader and preserves the earlier raw telemetry in the evidence logs.
