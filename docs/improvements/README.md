# Verified improvements

Accepted: **178 / 1000 independent, useful changes**. Only entries marked `accepted` in
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

Delivery verification uses a real 1 KiB BufReader and preserves the earlier raw telemetry in the evidence logs.
