> Current dependency status: production `media`, `media-cuda`, `cuda-hw`, and
> the historical `legacy-ffmpeg` marker all use owned library APIs without libav.
> The default dependency guard covers all these graphs. Older dated audit notes
> below describe migration history. FFmpeg is optional for explicit reference
> benchmarks; complete native codec/tool coverage remains pending. Physical CUDA
> acceptance on Windows / RTX 5090 passed on 2026-10-05 (see below).

# Validation without FFmpeg

`validate_gpu.py` now compares CPU/GPU planar crop and reflection results with
an independent Python pixel oracle. It does not probe or execute FFmpeg unless
`--benchmark` is explicitly requested. FFmpeg command construction lives in
`benchmark_gpu_reference.py` and is also used by `benchmark_resident.py`.

`validate_resident.py` uses the same independent oracle, retaining CPU comparisons,
real GPU backend checks, ordered-chain validation, API counters and publication
failure checks. It requires a GPU-enabled binary; unavailable backends are not
counted as executed. Reference implementation hashes are retained in reports.

Run `python3 scripts/test_y4m_oracle.py` for known-pixel checks covering all three
8-bit chroma layouts, operation ordering, frame metadata and invalid/truncated
inputs. The oracle covers aligned planar 420/422/444 crop/hflip/vflip only; it is
not a reference implementation for codecs, resampling or unrelated filters.

Local verification on 2026-10-02 used a deliberately nonexistent `--ffmpeg` path
for GPU validation. CPU passed 24 transform and 10 large-frame cases. An actual
Metal build then exercised the same cases and eight rejection checks; resident
Metal exercised eight chains and its ignored hardware API acceptance test.
The first headless release binary could not exercise Metal and was rebuilt
with `--no-default-features --features gpu` before those hardware results.

`validate_hw_cuda.py` runs the seven required named physical CUDA/NVENC Cargo
tests without launching or importing an external FFmpeg comparison. Add
`--benchmark-reference` to preserve the full supplied-CLI comparisons, including
crop/reflections, frame hashes, cut timing, decode count and P010/Main10 shader
export. That unchanged comparison body lives in `benchmark_hw_cuda_reference.py`.
Reports distinguish Cargo qualification from completed CLI/reference checks;
the performance gate requires all named reference comparisons and matching
binary hash, accepting historical full-reference snapshots but refusing new
Cargo-only reports. Dispatch/report tests use simulated results and do not
constitute physical NVIDIA proof. On this Mac the actual ordinary validator
records failure because `nvidia-smi` is unavailable. The current `cuda-hw` Cargo
suite still links its legacy libav adapter: removing external commands here
does not prove a FFmpeg-free production CUDA dependency graph.

`validate_media.py` runs the native dependency/test/generator policy guard,
all owned media library unit tests, and the frontend headless library/integration
tests. It writes `native-media-validation.json`; no external reference module
is imported. Add `--benchmark-reference` for the complete existing CLI corpus
in `benchmark_media_reference.py` and its separate `media-validation.json`.
All reference helper/check computation is unchanged; only argument/report
entrypoints were adapted. The CPU performance gate keeps its 524-check threshold,
matching-binary requirement and full-reference scope; an ordinary test report
cannot qualify it. Policy tests simulate subprocess/benchmark dispatch and
check that failed or empty test execution replaces stale successful reports.
They are not evidence of codec or CLI conformance.

The dependency policy also audits five native validators and the reference-sample entrypoint for
known literal external launches/environment hooks, failing on missing or malformed
source. Explicit reference benchmark modules remain allowed. This complements
the dispatch tests; it does not resolve arbitrary computed executable paths.

Local ordinary media validation on 2026-10-02 executed 116 owned-library and
1,224 frontend Rust tests successfully, with one frontend test ignored. It
recorded `reference_completed=false` and `provided_cli_checks_completed=false`;
the full external corpus was not run in that invocation. The separate production
dependency audit still refuses `media`, `media-cuda` and `cuda-hw` because they
activate `legacy-ffmpeg`. Passing native tests does not close that boundary.

`validate_klite_coverage.py` now uses the same complete native Rust test/dependency
checks by default and writes `native-klite-validation.json`. It neither imports
the external oracle nor measures the supplied `--binary` in this mode; its report
explicitly says that it is not a K-Lite row census. `--help` is import-safe too.
`--benchmark-reference` runs the preserved full corpus in
`benchmark_klite_reference.py`, with every codec row, fixture/oracle comparison,
native/player/MCP surface, accounting and prose gate retained. Existing binary,
probe and Markdown options are forwarded in reference mode. That mode writes the
separate `klite-coverage.json`; failures replace stale success atomically, current
partial rows remain visible, and completion requires a successful reference exit
with a nonempty matching row report. Matrix/probe options are refused in native
mode instead of silently rewriting reference documentation.
The dispatch tests simulate those control paths; they do not constitute a new
K-Lite codec measurement. `fetch_klite_samples.py` is also import/help-safe and
requires `--benchmark-reference` before preparing any external samples. Its
unchanged download, integrity, codec-probe and prefix-generation computation
lives in `benchmark_fetch_klite_samples.py`; force/verbose/auto/add options and
the incomplete-pin failure exit code are preserved. Ordinary synthetic fixture
generation remains separate from this explicit benchmark preparation.
Production legacy media operations and AAC aggregate allocation admission remain
separate work. Native test success does not establish complete K-Lite codec parity.

Local ordinary K-Lite validation on 2026-10-03 executed 123 owned-library and
1,234 frontend tests successfully, with one frontend test ignored. A deliberately
nonexistent supplied CLI was not used; all reference/CLI-completion flags were
false. A fresh source/dependency audit passed for all six entrypoints and four
native build graphs. The production audit still failed all three legacy media/
CUDA graphs. The external K-Lite corpus was not executed in this verification;
its computational source was compared against the original, with only CLI
entrypoint, description and generated-command provenance adaptations allowed.

### Owned scheduled Y4M overlay

The owned streaming frame pipeline and lossless export now accept opaque overlay
from a second progressive Y4M source. The primary presentation timestamp chooses
the latest foreground frame at or before that time; foreground EOF repeats the
last complete frame. Interval selection retains the original clock. The owned
`overlay_video` and `plan_overlay` entrypoints are also selected by the production
legacy-enabled API before opening its libav input.

This route currently requires 8-bit JPEG-sited YUV420 on both inputs, matching
colour range and chroma-aligned placement. Alpha, other colour conversions,
compressed inputs and shuffleplanes combinations retain their previous route.
The general FFmpeg dependency removal is not complete.

`generate_y4m_overlay_fixtures.py` writes wholly synthetic primary/foreground
fixtures separately from test execution. The automated scheduling test checks
all planes, different frame rates, EOF repetition and interval timestamps; the
export test calls the public overlay and plan APIs. Restoring the old admission
condition reproduced the specific unsupported-transform refusal. An explicit
external reference comparison matched all six synthetic frames against FFmpeg;
ordinary tests use only owned code and committed fixture bytes.

### Test executable isolation

The Linux owned-library and CPU-only test steps set Cargo's target runner to
`/usr/bin/env PATH=/fvid/no-external-programs`. Compilation keeps its normal
PATH, while Cargo-launched Rust test executables cannot discover FFmpeg or
other external programs through PATH. Those two steps also set
`FVID_FFMPEG_PREFIX=/fvid/no-ffmpeg`, so enabling the legacy SDK accidentally
cannot use the system FFmpeg installation. Default-feature and player test
steps remain separate; this is not production legacy-backend removal.

Local macOS verification on 2026-10-03 ran all 130 owned-library tests with
an empty external-program PATH. Direct Mach-O dependency inspection listed
only libiconv and libSystem, with no libav libraries. The equivalent Cargo
runner plus a deliberately nonexistent FFmpeg SDK prefix also passed the three
scheduled-overlay tests. PATH isolation is a runtime discovery check, not a
sandbox against absolute executable paths; retain the source/dependency guard.

### Owned FFV1 monochrome

The shared FFV1 range-coded core now encodes monochrome keyframes through
`encode_gray` and decodes the previously refused monochrome flag. The public
planar Y/Cb/Cr output contract is retained: luma is unchanged, while missing
chroma is synthesized at its neutral value in full-resolution planes. Storage
admission includes those output planes. RGB, alpha, Golomb Rice and versions
above 1 remain separate missing tools; production libav removal is incomplete.

`generate_ffv1_gray_fixtures.py` compiles the owned encoder with rustc and writes
short synthetic 8/10/16-bit Matroska videos, elementary packets and analytical
luma samples. Ordinary tests read these committed artifacts without running the
generator. All 15 artifacts regenerated byte-identically. The baseline decoder
refused all six packets specifically as unsupported monochrome. A compatibility
check retained byte-identical encodings across 54 existing colour depth/layout
cases; explicit FFmpeg reference decoding matched all six grayscale frames.
The format basis is RFC 9043 sections 4.2 and 4.7:
https://datatracker.ietf.org/doc/html/rfc9043

Local verification passed all 132 owned-library tests and all seven native FFV1
playback tests, including luma/depth, timestamps, rewind and seek. These results
do not establish equivalence of the remaining production operations.

The complete local headless root isolation run also finished: 122 Cargo test
executables, 1234 passed tests, one ignored test and no failures. Each executable
was inspected with `otool -L` before running with an empty external-program
PATH; none linked libav. The artifacts were collected with
`cargo test --offline --no-default-features --lib --tests --no-run --message-format=json`
before the monochrome addition; the new seven-test FFV1 playback suite passed
separately afterwards, including the same empty-PATH Cargo runner. Feature-gated
legacy tests compile to empty suites in this mode, so this does not qualify the
remaining production `media`/CUDA legacy operations. Full local run evidence:
`/tmp/fvid-root-tests-isolated.json` and `/tmp/fvid-root-tests-isolated.log`.

### Owned pixel descriptions in production adapters

Pixel names and tightly packed frame storage for 42 existing representations
now come from the owned `PixelFormat` table. Production decode, filtering,
lossless export and transitions use this table for their 101 previous name
lookups. The legacy adapter retains its original name lookup for unmigrated
formats and NULL results; remaining libav operations are not removed by this
change. Memory estimates use the same owned format descriptions.

An explicit libavutil reference check verified all 42 enum mappings and names,
252 frame sizes (including odd chroma dimensions), and three unmigrated/null
routes. The legacy backend compiled successfully. Source comparison verified
that the four operation files changed only their name lookups and unnecessary
unsafe wrappers; their pixel-processing and scheduling bodies were retained.

## Owned default channel layouts

Default channel initialization and audio-plan default layout descriptions no
longer call libavutil. The owned table preserves thirteen named layouts,
including the unspecified-three-channel default 2.1; explicit PCM rematrixing
retains its separate 3.0 mask. Other channel counts preserve an unspecified
layout and an `N channels` description. This does not remove other legacy
channel-layout operations or the production backend dependency.

An explicit local libavutil reference comparison matched all initializer fields
and descriptions for 70 counts (1 through 64 plus zero, negatives and large
integer bounds). The owned library passed 135 tests, and the legacy/player
configuration compiled. These checks cover utility migration, not full codec
or backend equivalence.

Audio resampling identity checks and stream compatibility comparisons now use
owned speaker-mask/unspecified-layout equality. Equal counts with different
speaker assignments remain different; unspecified layouts do not implicitly
match a native mask. Custom and ambisonic layouts retain the legacy comparison.
An explicit libavutil reference checked 3,600 pairs spanning both migrated
orders, channel-count boundaries and distinct masks; all results matched.
The owned library passed 136 tests and legacy/player compilation passed.

## Complete owned channel-layout comparison

The comparison adapter now handles custom channel maps and ambisonic component
ordering as well as native masks and unspecified layouts, without any
`av_channel_layout_compare` fallback. Cross-representation equality compares
ordered channel identifiers; custom display names do not affect equality.
The implementation borrows existing maps and allocates no temporary map.
The adapter requires initialized valid layouts, including a readable custom map.

An explicit local libavutil reference checked all 484 ordered pairs of 22
layouts: masks, unspecified counts, named/reordered/duplicate/custom channels,
unknown and unused channels, and ambisonic components with and without stereo
extras. All comparisons matched. The owned library passed 137 tests; the
legacy/player configuration compiled. The comparison behavior was checked
against the upstream channel-layout API implementation:
https://github.com/FFmpeg/FFmpeg/blob/master/libavutil/channel_layout.c
Other layout copying, ownership and description operations remain separate
migration work; production libav dependencies are not yet eliminated.

## Owned inline-layout copying and reset

Ten audio/player copy and cleanup call sites now use FVid adapters. Native,
unspecified and ambisonic layouts require no allocated map: copying preserves
all fields, including the opaque user pointer, and reset clears the structure.
A destination custom map is released before replacement. Custom map cloning
and release retain backend allocation compatibility; this is not full channel
map ownership migration and does not remove the production backend dependency.

An explicit libavutil reference verified 484 source/destination combinations
across the 22 layouts used above, including custom-to-inline replacement,
map identifiers, names, opaque fields, and repeated cleanup. All results
matched. The legacy/player configuration compiled and diff checks passed.

## Owned Matroska declared frame-rate query

`owned_webm_probe::declared_video_frame_rate`, also exposed through the native
probe entry point, reads a selected video track's declared cadence without
libav or codec inference. It reduces 1,000,000,000 / DefaultDuration exactly;
non-video tracks, missing durations and unrepresentable signed rational rates
return no declared rate. Invalid stream indices are errors.

DefaultDuration is a nominal per-frame duration in nanoseconds, independent
of TimestampScale. It is not a measured average: existing average_frame_rate
metadata deliberately remains unchanged. See RFC 9559 section 5.1.4.1.13:
https://datatracker.ietf.org/doc/html/rfc9559#section-5.1.4.1.13

The 215-byte `matroska-default-duration-60fps.mkv` is two own-encoded FFV1
monochrome frames with 16,666,667 ns declared duration and millisecond-rounded
block timestamps. Its separate generator uses Rust/Python, with no FFmpeg or
network. All sixteen generator outputs regenerate byte-for-byte; the previous
fifteen artifacts remain unchanged. Acceptance checks cover exact declared
rate, unchanged average semantics, invalid index, audio exclusion, and decoding
both frames. This query does not switch the production legacy probe wholesale;
its remaining metadata parity and backend dependency still require migration.

Validation: 138 owned library tests and all ten native Matroska-probe/FFV1
integration tests passed with an empty test PATH. The library binary's direct
Mach-O dependencies contain no libav, swscale or swresample. Legacy/player
compilation passed. The source/fixture/validator policy and four supported
native dependency graphs passed; these graphs exclude production legacy media.

## Owned codec metadata catalog

Probe and operation-plan descriptions use an owned catalog for 44 codec names:
AVC, HEVC, AV1, VP8/VP9, FFV1, AAC/LATM, Opus, Vorbis, FLAC, ALAC, MPEG audio,
AC-3/E-AC-3, PCM representations, image and subtitle/data codecs. The catalog
contains 52 profile labels. AVC SPS-only profile descriptions share this
vocabulary while preserving their previous profile admission rules.
All production media-type string descriptions now use a static FVid mapping.
Unmigrated codec names and profiles retain the legacy metadata route.
Describing a codec/profile is not evidence that its decoder tools are supported.

An explicit local libavcodec/libavutil reference verified all 44 codec names,
180,400 profile queries (including missing profiles and integer bounds), three
unmigrated codec routes and ten media-type values. Every result matched.
A separate old/new owned comparison verified all 65,536 AVC profile/constraint
combinations, with no changes to naming or refusal. No compressed-media parsing,
packet decoding or profile admission was changed by this metadata migration.

Validation: 139 owned library tests and three public/library MP4-probe tests
passed with an empty test PATH. The owned library test executable had no direct
libav/swscale/swresample linkage. Legacy/player compilation, diff checks and
all four supported native dependency graphs passed. This completes only the
catalogued metadata routes; demuxing, filtering, encoding, muxing, unmigrated
metadata and production legacy feature coupling remain incomplete.

## AAC initialization transient allocation reduction

The owned KBD window builder now normalizes and mirrors within one preallocated
buffer. IMDCT setup no longer allocates a placeholder kernel before constructing
the real FFT kernel; its setup FFT only reads roots and reversal indices.
Retained windows, transforms and decoded PCM are unchanged.

`scripts/bench_aac_initialization_allocations.py` is an explicit offline own-code
benchmark against Git baseline 4ccf4852. It needs Git/rustc but no FFmpeg or
network and checks bitwise KBD/IMDCT output for 120, 128, 960 and 1024 samples.
On the current macOS arm64 runtime, KBD-1024 peak outstanding requested heap
payload decreased from 24,576 to 16,384 bytes (three allocation/reallocation
calls to one); IMDCT-1024 decreased from 114,688 to 82,040 bytes (six calls to
five). All four geometries improved and produced bitwise-identical output.
The meter excludes allocator overhead/RSS and cannot observe hidden temporary
allocations within system realloc; it is not a full decoder/export peak proof.

139 owned library tests passed, as did 32 AAC/ADTS integration tests with empty
test PATH (960 frames, PCE, coupling, decoder, streaming and file export).
Legacy/player compilation and supported native dependency/source policy passed.
ADTS aggregate allocation admission remains explicitly unsupported: these
initialization measurements do not justify enabling its complete export budget.

## Empty secondary overlay EOF acceptance

An initialized Y4M secondary source with no frames now leaves primary frames
unchanged instead of failing with `Y4M overlay source has no frames`. EOF after
at least one secondary frame still repeats that frame. Header/marker/payload
errors continue to propagate rather than being treated as an empty source.

The existing synthetic six-frame primary clip plus new 38-byte
`overlay-secondary-empty.y4m` reproduces the specific old execution error.
The acceptance test failed on that exact error before the fix. The independent
fixture generator writes the valid secondary header with no frames and uses no
FFmpeg/private media; the previous two overlay artifacts remain byte-identical.
Ordinary tests read committed fixtures without invoking the generator.

An explicit FFmpeg reference overlay produced all six original primary frames
byte-for-byte. Owned acceptance checks verify unchanged pixels for full and
interval execution, owned public plan/export selection, six exported FFV1
packets, their presentation timestamps and independently decoded samples.
Other unsupported overlay colour conversions remain outside the owned route;
this acceptance closes the empty-source compatibility gap, not every overlay
mode or the remaining production backend dependency.

Validation: 140 owned library tests and 13 native geometry tests passed with
empty test PATH. The owned library test binary had no direct libav/swscale/
swresample linkage. Legacy/player compilation, the supported native dependency
and source policies, and diff checks passed. The final regression also passed
after ensuring that the output reader closes before fixture cleanup.


Owned overlay comparison-clock regression
-----------------------------------------

Exact rational flooring selected secondary frames [0,0,1,2,3,4] for nearly
identical rates 2147483646/2147483645 and 2147483647/2147483646. The committed
synthetic acceptance test failed specifically at primary frame one before the
fix. The owned scheduler now compares rounded presentation times on a shared
rational clock (denominator below 500000), falling back to microseconds.
It consumes one equal-tick event per primary frame, including the synthetic
1 GHz / 2 GHz tie case. Full pixel acceptance expects [0,1,2,3,4,5] for both
pairs; explicit capped FFmpeg reference comparisons produced these sequences.
Primary presentation timestamps, intervals and termination remain unchanged.

Four new fixtures come from generate_y4m_overlay_fixtures.py without FFmpeg;
the three previous fixtures retain their original bytes. All 141 owned unit
tests passed, including execution with no external-program PATH. Native source
and dependency guards passed for the library, headless, player and camera
bridge. This closes a temporal compatibility gap in the owned overlay route;
production media/CUDA still require migration away from legacy-ffmpeg.


Owned scheduled overlay placement
---------------------------------

Odd integer overlay coordinates previously failed owned route admission with
"overlay placement must align with chroma samples" and required the legacy
backend. The dedicated synthetic unaligned primary/secondary Y4M fixtures
now exercise floor-to-even placement in the scheduled reader. For example
(1,1) places at (0,0); (-1,-1) places at (-2,-2) and is entirely clipped for a
2x2 foreground. The low-level compositor still requires aligned coordinates.
Its validation and mutation contract are unchanged.

The acceptance failed before the fix specifically at owned admission, then
passed for eight placements including both signed i32 extremes. It checks
all pixels and timestamps across six frames, repeat-last scheduling, owned
public planning and public export. Explicit FFmpeg reference comparisons
matched all 48 frames byte-for-byte against these acceptance expectations.
The two new fixtures are generated without FFmpeg or private media; all
previously committed overlay fixture bytes remained unchanged.

All 142 owned unit tests passed with an empty external-program PATH and no
libav linkage. Legacy/player compilation and native dependency/source guards
passed. Other overlay colour conversions and remaining production backend
operations still need migration; no feature was detached to hide them.


Owned backend error descriptions
--------------------------------

The shared legacy operation checker no longer calls av_strerror. It formats
ABI-tagged errors with an owned catalog and uses POSIX strerror_r for errno
messages on Unix. Unknown codes retain the numeric diagnostic, including
signed i32 extremes. Platforms without POSIX strerror_r use the portable
errno catalog rather than treating errno values as Win32 error numbers.
The surrounding operation label, code and success handling remain unchanged.

The explicit ffmpeg_error_reference benchmark is the only call site of
av_strerror in media sources. On macOS it matched 4130 descriptions covering
-2048..2048, tagged decoder/container/network codes, configuration changes,
unknown codes and signed extremes. The combined input/output-change bitmask
aliases input-change in this ABI; the benchmark verifies its actual reference
message rather than inventing a separate code. Other-platform runtime parity
has not been measured. ABI/reference source:
https://github.com/FFmpeg/FFmpeg/blob/master/libavutil/error.c

All 143 owned unit tests passed with empty external-program PATH; the test
binary links system libraries only, without libav. Legacy/player compilation,
native dependency/source guards and diff checks passed. Formatting diagnostics
is now independent; demux, codec and filter migrations are still incomplete.


Owned Y4M framestep decode and packet encoding
--------------------------------------------

The own streaming video reader now accepts decimal framestep steps 1..INT_MAX,
including default, positional and named options. It selects the first and every
Nth frame after interval admission, updates the declared output frame rate,
and retains each selected source presentation timestamp and duration. Skipped
payloads are still read and validated. Only a counter is added; no frame queue
or full-file PCM/video allocation is introduced. Overlay compares its original
source clock before output rate metadata changes.

framestep-six-frames.y4m is a six-frame indexed synthetic Y4M artifact produced
without FFmpeg by generate_y4m_overlay_fixtures.py. The acceptance initially
failed specifically because the requested transform was unsupported. It now
selects [0,2,4] for full input and [1,3,5] for the interval starting at 0.25 s.
Explicit framestep/showinfo reference checks matched all pixels, source PTS,
0.25 s frame durations and the 2/1 output rate. Option-order experiments also
confirmed that unnamed values after named options are invalid. Contract source:
https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/vf_framestep.c

Acceptance also exercises public transformed decode and owned FFV1 packet
encoding combined with overlay; independently decoded packets match every
expected sample and timestamp. All 146 own unit tests passed with empty test
PATH and no libav linkage, followed by three focused framestep tests after
preserving original header tokens for requests without this filter. Native
dependency/source guards and legacy/player compilation passed.

This enables own decode and direct packet encoding. The public lossless file
operation still requires separate consumed-frame accounting before accepting
framestep; its previous legacy route was preserved. General option expressions,
unrepresentable exact output rates and other codec/filter gaps remain open.

Final verification repeated all 146 own tests with an empty external PATH
after the header preservation adjustment. All 13 native geometry integration
tests also passed with the empty-PATH Cargo runner and invalid FFmpeg SDK prefix.


Owned framestep lossless file operation
-------------------------------------

The native lossless request now includes framestep and dispatches before
legacy input opening. Internal reader/encoder/mux accounting carries the
number of fully consumed source frames separately from selected output frames;
public existing result types remain unchanged. Full six-frame input exports
three FFV1 packets and reports six decoded frames. The exact 0.25..1 s interval
exports indices [1,3], rebases their PTS to [0,0.5] s and reports four consumed
frames, including origin/preroll. Durations remain the source 0.25 s values.

The file-operation acceptance failed before this fix at owned route admission
using the committed six-frame synthetic fixture. It now verifies public own
planning/export, counts, every pixel, packet timestamps/durations and independent
owned FFV1 decoding. The new framestep-discarded-truncated.y4m fixture truncates
the final discarded frame and is generated without external tools. Refusal
checks demonstrate that discarded payloads and source packet limits still
validate before publication; they are preservation checks, not successful
playback of a damaged input. Existing temporal artifacts remain byte-identical.

The explicit ffmpeg_framestep_reference benchmark independently decoded both
own files with FFmpeg and compared every pixel to the reference filter. Its
showinfo assertions also verified container PTS and source frame durations.
FFmpeg is called only by this benchmark, not fixture generation or tests.

All 148 own unit tests and 13 native geometry integrations passed with empty
external-program PATH. The own test binary links no libav; legacy/player
compilation, native source/dependency guards and diff checks passed. This
removes the Y4M framestep file-export fallback. General expressions, other
sources and the rest of production codec/filter migration remain incomplete.


Owned shuffleframes decode and lossless file operation
-----------------------------------------------------

The own temporal stage accepts decimal positional/named mapping lists with
spaces or pipes, duplicates and -1 dropped output positions. It retains only
one mapping-sized group and reuses its buffers; identity needs no copied frame.
Discard-only decoding does not retain pixel payloads. Output PTS comes from the
output position, while payload and duration come from the mapped source frame.
An incomplete group at EOF or interval end produces no output, matching the
reference filter. All source payloads, including discarded tails, still validate.

The seven-frame and truncated-tail Y4M fixtures come from the deterministic
Python generator without external codecs or private media. The initial native
acceptance failed specifically at unsupported transform admission. Passing
acceptance now covers reordered pixels, duplicated/dropped positions, interval
origin, framestep composition, consumed source counts, own public planning/file
export and independent own FFV1 packet decoding. Empty output is a passing
refusal/publication test, not an acceptance of an empty playable video.

The explicit ffmpeg_shuffleframes_reference benchmark matched four own exports
against independent FFV1 decoding, showinfo PTS/durations and reference filter
pixels. Its optional legacy feature forces the previous adapter with an
explicit level=1 encoder option, avoiding accidental use of the own route.
This discovered that the old standalone shuffle branch bypassed framestep:
it emitted six frames [2,1,0,5,4,3] instead of [4,2,0]. The branch now feeds
framestep before shuffle; the same synthetic regression and benchmark pass.
The optional benchmark also confirms legacy refusal when every group is empty.
Reference contract: https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/vf_shuffleframes.c

All 153 own unit tests and 13 native geometry integrations passed with empty
external-program PATH. The own test binary has no libav linkage. Both benchmark
modes, legacy/player compilation, native source/dependency guards and diff
checks passed. Previously committed temporal artifacts retain their bytes.
General mapping option expressions and other unmigrated codecs/filters remain
on the previous adapter; no production feature was detached to mask that work.


Owned Y4M reverse decode and lossless file operation
--------------------------------------------------

The own reverse stage stores fixed-size timing/payload records in a private
same-operation temporary file rather than retaining every decoded frame or a
per-frame metadata array in memory. Replay uses one payload buffer; upstream
frame/group buffers are released before replay. File handles close before
removal, including callback/read failures. Unix spool creation uses mode 0600.
Payloads replay backward; both PTS and duration replay in original forward
position order. Discard-only decoding without a frame consumer does not create a spool.

The synthetic six-frame and truncated-last-frame Y4M fixtures come from the
Python generator without external codecs or private data. Before the fix the
acceptance failed specifically because reverse was unsupported. Acceptance
now covers full and interval pixels/timing, owned public plan/export, exact
consumed counts, framestep/shuffle composition, independently decoded FFV1
packets, callback-error cleanup and damaged-source refusal without publication.
A scalar replay test checks deliberately unequal forward durations. The file
reader now checks cancellation on input reads/fill_buf, so reverse stops after
the first input frame in the controlled test, before replay or packet writing.

The explicit ffmpeg_reverse_reference benchmark independently decoded four
own outputs and verified filter pixels plus container PTS/durations. Its optional
legacy feature forces the original FFV1 adapter for the simple full-reverse
case with a nonempty level=1 encoder option; that case also matched. This does
not establish parity for other legacy temporal combinations. Contract source:
https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/f_reverse.c

All 159 own unit tests and 13 native geometry integrations passed with empty
external-program PATH. The own test binary has no libav linkage. Both benchmark
modes, legacy/player compilation, native dependency/source guards and diff
checks passed. Existing generated temporal artifacts remain byte-identical.
This migrates Y4M reverse; other source codecs and unimplemented production
filters still require their existing adapter and the overall migration is open.

## Core/player verification on macOS, 2026-10-05

`FVID_FFMPEG_PREFIX=/fvid/no-ffmpeg cargo test --locked --offline --features media,player --lib`
completed successfully: 893 passed, 0 failed, 23 ignored. The command was started
at `0cb90384`; the subsequent Opus capability inventory change has its own
passing targeted library test. This run includes native player, seek, codec and
GPU-render unit tests selected by these features. It does not execute the
ignored cases or establish NVIDIA performance or complete codec coverage.

`otool -L` on the resulting `fvid-c8ad722cfb4fc6a9` test executable listed macOS
system libraries/frameworks and no libav library. The offline flag prevents
Cargo network access; the invalid FFmpeg prefix checks SDK independence. This
local command retains the normal executable PATH, so runtime absence of external
tool calls is supported separately by source audits and CI's empty test PATH.

### Combined media/player CLI regression checks

Running the built core/player library tests directly with
`PATH=/fvid/no-external-programs` also passed: 893 passed, 23 ignored.
Combined `media,player` CLI compilation exposed a stale `play_paths` call to the
retired library player. The CLI now routes `media play` through the existing
native player parser, including invalid requests. With an empty executable PATH,
18 CLI unit tests and two native media-play CLI integration tests passed.

The AVC lossless acceptance suite explicitly selects `open_software`, so its
six passing sample/rewind/camera checks do not depend on automatic VideoToolbox
selection. Native export passed all 18 tests after updating the HueProgram API
and replacing an obsolete 10-bit EQ refusal with acceptance: 17 frames at 30 Hz,
10-bit luma saturated to 1023, original chroma preserved byte-for-byte, and only
the final Y4M published. Both use existing committed synthetic fixtures and run
without a reference process. Linux and Windows CI now cover combined media/player
CLI and export. The complete 163-target integration audit remains separate from
these passing targeted checks and must not be inferred from them.

### Explicit software selection for saved codec samples

Saved pixel/seek/rewind oracles now explicitly use `Mp4VideoReader::open_software`
or `NativeReader::software`. Automatic platform decoding is a separate contract:
these tests must exercise FVid's kernels even when VideoToolbox is compiled.
The selected combined `media,player` run, with an empty executable PATH and
invalid FFmpeg prefix, passed all 48 tests: AVC multislice 11, AVC scaling 3,
HEVC multislice 9, HEVC playback 9 and HEVC RExt smoothing 16. Existing synthetic
fixtures and independent stored samples remain unchanged; no oracle was weakened.

The dependency guard's eleven required graphs also passed offline for
`x86_64-unknown-linux-gnu` and `x86_64-pc-windows-gnu`, including root
`--all-features` (396 and 323 normal/build packages respectively). Missing Cargo
cache packages were fetched separately with the locked manifests before the
successful offline audits. These are dependency checks, not cross-platform
execution or hardware qualification.

### Portable filter math acceptance (2026-10-05)

Linux CI exposed deband sampling-coordinate differences from platform binary32
`sinf`; the saved macOS reference selected different radii after random-value
amplification. Owned deband now evaluates sine/cosine at binary64 precision,
then explicitly quantizes to binary32. The independent Python fixture generator
creates separate `deband-portable-*.expected.raw` acceptance pixels. Historical
reference bytes remain unchanged. A dedicated synthetic boundary case requires
luma 89 at (5, 1), and the full ramp acceptance covers four frames, rewind,
threshold/blur/coupling, 8/16-bit layouts, source clocks and owned CLI export.

Windows CI exposed two 16-bit colorchannelmixer power-preservation rounding
errors. The shared metric now evaluates its cube root at binary64 precision
before binary32 quantization. The two synthetic pixels and the complete saved
RGB/float reference suites retain their original independent expected bytes.
All 31 `owned_color` library tests passed with `PATH=/fvid/no-external-programs`
and `FVID_FFMPEG_PREFIX=/fvid/no-ffmpeg` on macOS. All 15 dependency policy tests
passed, and the locked offline production graph audit passed all 11 cases for
macOS, Linux and Windows, including all production features. These graph audits
are not execution proof for Linux/Windows or physical NVIDIA devices.

The changed root `native_deband` target passed all eight tests locally, then
passed all eight again by directly executing its compiled test binary with
`PATH=/fvid/no-external-programs` and `FVID_FFMPEG_PREFIX=/fvid/no-ffmpeg`.
The empty-PATH run includes real child CLI decode, Y4M export and FFV1 transcode,
followed by owned decode and exact independent pixel comparisons. Local results
do not yet prove the new Linux/Windows CI runs; those must execute the fix.

### Complete ordinary Python workflow boundary (2026-10-05)

The source guard now audits all 109 ordinary Python files under `scripts`,
including newly added workflows, nested helpers and Python tests. Only explicit
top-level `benchmark_*.py`, `benchmark.py` and `show_bench.py` entrypoints/modules
are exempt from the known FFmpeg launch/environment-hook check. The mandatory
validator registry is still checked separately so missing validators cannot be
silently omitted. All 18 policy tests passed, including new workflow rejection,
benchmark separation and malformed-source refusal; the offline graph guard
passed all 11 macOS cases. This remains a known-literal source audit, not arbitrary
computed-command analysis.

The explicit deband reference benchmark completed 1,064 four-frame comparisons
using `FVID_REFERENCE_FFMPEG` (benchmark mode only): 926 were pixel-identical and
138 exposed the intended portable random sampling-map difference. Its report
now preserves and names these differences instead of claiming universal exact
parity. Fixed-radius/fixed-direction cases still require exact reference pixels;
all comparisons require successful reference execution and equal output lengths.
Ordinary deband acceptance uses independent portable fixture pixels and launches
no FFmpeg. Historical reference fixtures were not regenerated by this benchmark.

On commit `f57334de`, both Linux and Windows CI completed the owned-library test
stage successfully. Their broader root test stages were still running at this
observation. The current local media/player CLI binary's Mach-O dependency list
contains system frameworks and no libav libraries; this is local linkage evidence,
not an executed Windows/Linux or NVIDIA hardware result.

### Full feature compilation and checkout regressions (2026-10-05)

`FVID_FFMPEG_PREFIX=/fvid/no-ffmpeg cargo check --locked --offline --all-features
--lib --bins --tests` completed successfully on macOS. This checks every root
production feature plus library, binary and integration-test targets; it is a
compilation check, not a linked all-feature executable or GPU execution result.

On `f57334de`, Linux's default test stage accepted owned `gblur` decoding but
failed an obsolete refusal assertion in `native_y4m_library_transform`. That
assertion now requires successful three-frame owned Y4M decode. Windows's default
stage exposed checkout-only CRLF conversion in `bitplanenoise.expected.txt`;
metadata values and pixel bytes already matched. `.gitattributes` now fixes LF
for expected text sidecars and binary handling for raw pixel/PCM fixtures.
With `core.autocrlf=true`, Git's checkout filters were checked against committed
blob bytes for the metadata sidecar, raw bitplane pixels and 16-bit RGBA fixture:
all three remain byte-identical. No reference bytes were edited or regenerated.

The focused `native_bitplanenoise`, `native_gblur_cli` and
`native_y4m_library_transform` integration targets then passed all 23 tests with
`CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER='/usr/bin/env PATH=/fvid/no-external-programs'`
and `FVID_FFMPEG_PREFIX=/fvid/no-ffmpeg`, using locked offline dependencies.
This includes exact bitplane pixels/metadata, Gaussian CLI decode/plan/export,
the formerly stale refusal, frame-clock selection and rejection of requested
malformed Y4M tails. Windows checkout was simulated above; new Windows/Linux CI
execution is still required to confirm the complete changed checkout.

### Quoted curves paths and public temporal plan contract (2026-10-05)

On `78315f45`, Windows default tests exposed a real curves option-parser bug:
colon splitting ignored quotes and truncated Windows drive paths. The parser
now splits only outside matching single/double quotes and preserves literal
backslashes. The portable regression reuses the synthetic four-frame grayscale
video and negative ACV/expected pixels; it uses a POSIX filename colon or Windows
drive colon, verifies the exact former quote error, requires exact exported
pixels and checks plot-file creation. Original reference bytes remain unchanged.

Linux exposed a separate test-contract mismatch: CLI temporal plans use the
public `fvid::media` facade and its own optimized framestep exporter, while the
test expected the internal generic exporter's different descriptor. The test
now compares the complete CLI JSON to the same public API, additionally requiring
no external graph, the no-external-backend note and an FFV1 encoding stage.
Execution routing was not changed. `native_curves`, `native_framestep` and
`owned_filter_inventory` passed all 19 tests with a deliberately empty test PATH,
invalid FFmpeg prefix and locked offline dependencies. Framestep checks include
actual packet timestamps, selected frames and retained AAC packets.

Before the quoted-path parser change, the production library also completed
`FVID_FFMPEG_PREFIX=/fvid/no-ffmpeg cargo build --locked --offline --all-features
--lib`. Its archive's undefined-symbol list had no known FFmpeg C API names
(`av_*`, `avcodec_*`, `avformat_*`, `avfilter_*`, `avio_*`, `sws_*`, `swr_*`). This
is library compilation/code-generation and symbol evidence, not a linked
all-feature CLI or physical NVIDIA result. The fresh all-feature library build
including the quoted-path change also completed successfully; its archive's
undefined-symbol list again contained none of those FFmpeg C API names.

The production CLI also completed `FVID_FFMPEG_PREFIX=/fvid/no-ffmpeg cargo
build --locked --offline --all-features --bin fvid`. `otool -L
target/debug/fvid` lists Apple frameworks and system libraries, with no libav,
libswscale or libswresample dependency. This proves the all-feature macOS CLI
links without a FFmpeg SDK; it does not establish physical NVIDIA execution.


## Windows CUDA acceptance — 2026-10-05

Windows / GeForce RTX 5090, driver 617.14, CUDA toolkit 13.4: all 28 required
physical tests across 11 suites passed via alidate_hw_cuda.py. The local
report is enchmarks/windows-cuda-validation.json; this is Cargo acceptance,
not a performance or external-reference qualification. Production builds use
owned codec APIs without libav.

NVENC bindings now target SDK 12.0 and select the extended P1 preset. Windows
codec buffers use conventional device allocations, and synchronous bitstream
locks wait for complete output. Hardware fixtures are saved synthetic AVC/HEVC
Main/Main10 videos large enough for device capability limits, generated only
by the explicit enchmark_cuda_fixture_reference.py --write-fixtures command.
Ordinary tests consume those files without FFmpeg or network access.

The debug CLI built with --no-default-features --features media-cuda exported
cuda-hevc-main10-edit-repeat.mp4 through NVDEC → GPU filter → NVENC: 14 frames,
256×192, zero host frame copies. The owned software decoder read all 14 output
frames as yuv420p10le with zero decode errors. Windows CLI stack reservation is
8 MiB to support owned codec parsing on the main thread.

Ordinary CUDA library tests: 40 passed (17 physical cases ignored here).
Ordinary owned media tests: 369 passed (18 physical cases ignored here).
A concurrent WAV temporary-file reuse race found by the Windows run was fixed
by retaining the originally reserved handle and using a monotonic identifier;
a synthetic concurrent PCM regression test passed. Native dependency audits
passed for all production feature graphs on x86_64-pc-windows-msvc.

The separate physical root CLI regression `native_cuda_cli` also passed. It
launches the Windows binary, exports the saved synthetic Main10 fixture, and
checks owned software decoding of every output frame with no host copies.
The six playback-control integration tests passed after conflict migration.

MP4 in-band PS reader dispatch (2026-10-09): the selected AAC track is probed
with the owned native candidate before decoder/backend creation. Actual accepted
PS payload selects the delayed stereo playback bridge without rewriting ASC or
consuming the logical packet cursor. `he_aac_ps_inband` compares original
LC/SBR 960/1024 synthetic video PCM to independent references, checks source
windows, EOF and rewind. Explicit disable flags retain ordinary dispatch.
This does not yet qualify output-clock changes, missing SBR fills in a selected
PS stream, PCE/coupling layouts, or Matroska implicit-PS discovery. The current
probe decodes eligible packets and may scan the complete track; startup cost
still needs a syntax-only probe and performance qualification.

Matroska in-band PS reader dispatch (2026-10-09): four additional original
AVC/AAC fixtures declare mono Audio metadata and unspecified PS in LC/SBR ASC,
then carry late PS payload with 960/1024 framing and negative source timestamps.
The reader's decoder factory negotiates stereo on actual accepted payload.
The integration regression first reproduces the former exact normal-AAC error
`SBR extended audio/PS synthesis is not yet implemented`, then accepts all
independently referenced stereo PCM through EOF and byte-identical rewind,
retaining original source timestamps and configuration. Generation is offline
and deterministic. Output-clock changes, missing SBR fill playback, complex
AAC layouts, implicit-PS export dispatch and probe startup performance remain
unqualified; this supersedes the Matroska discovery limitation above only.

Syntax-only in-band PS negotiation (2026-10-09): `InBandPsProbe` replaces full
native PCM decoding during MP4/Matroska reader preflight. It parses original
mono SCE, SBR and PS syntax transactionally, retaining syntax histories and
validating END/trailing bytes before publishing presence. It does not run IMDCT,
core synthesis, QMF, hybrid or stereo DSP. The existing synthetic missing-fill
video verifies that normal LC without SBR fill is accepted for discovery,
followed by late PS discovery, malformed-tail rollback, reset/replay and explicit
disable guards. This qualifies discovery only: native PS playback still refuses
missing SBR fill. Track-wide scanning and output clock negotiation remain gaps.

Implicit-PS Matroska PCM/WAV export (2026-10-09): the shared owned/root timeline
and WAV metadata probe negotiate actual PS payload before choosing stereo.
Four original mono-metadata LC/SBR 960/1024 videos reproduce the former exact
SBR/PS synthesis refusal. Acceptance compares independent PCM through full
export, gaps, ceil-rounded intervals and max-packet EOF drain; root and owned
PCM agree byte-for-byte and public WAV reports stereo geometry. Memory admission
reserves possible mono in-band PS before the probe; packet preflight checks
cancellation without consuming decoded-packet progress. Original ASC/timestamps
are retained. MP4 implicit-PS export, clock changes and missing-fill PS playback
remain separate gaps; this supersedes only the Matroska export limitation above.

Implicit-PS MP4 PCM/WAV export (2026-10-09): the shared root/owned timeline and
public WAV metadata selection probe original payload before choosing delayed
stereo PS. Four additional original LC/SBR 960/1024 MP4 fixtures declare mono
sample entries and omit PS signalling while retaining the authored packets and
repeated edit ranges. The regression reproduces the exact former ASC-only
SBR/PS synthesis refusal, accepts independent PCM for silence, repeated ranges,
intervals and max-packet EOF, compares root/owned bytes, inspects WAV geometry
and source speaker mask, and rejects low controlled budgets before PCM output.
No raw ASC or timestamps are rewritten at runtime. Output clock changes and
PS playback without SBR fills remain unqualified; the prior implicit-PS export
limitations are superseded only for the tested MP4 and Matroska geometries.

Implicit-PS MP4 presentation geometry (2026-10-09): real worker regression on
original mono-metadata LC/SBR 960/1024 video fixtures reproduced two distinct
failures: EOF attempted on a freshly reset in-band candidate after the edited
source range had already completed, and packet presentation reported 1024
frames where negotiated stereo PCM had 512. Completed ranges now suppress
codec EOF drain while retaining source validation; silence allocation and both edit and no-edit PCM
trim use negotiated channels. Four additional no-edit fixtures isolate the
ordinary presentation branch. Worker acceptance compares all rendered bytes to
independently qualified export, verifies contiguous stereo timestamps, source
EOF, seeks inside both ranges and silence, packet-boundary seek for no-edit
files, and rewind. Codec EOF/refusal checks
remain enabled for actual pending source output. Output-clock changes are not
qualified by this geometry fix.

Owned MP4 AAC decoded-geometry parity (2026-10-09): the owned parser now uses
ASC decoded rate/channels with the same resolution policy as the existing root
parser, retaining original media timescale, sample timestamps, durations and
edits. Four original explicit/in-band SBR/PS 960/1024 fixtures use mono 24 kHz
sample entries and 24 kHz source ticks with a 48 kHz ASC output clock. Reverting
only the owned parser change reproduces the exact former export refusal
`MP4 audio export requires valid clock and matching audio geometry` on the first
fixture. Acceptance compares independent PCM through repeated ranges, intervals
and packet-limit drain, root/owned bytes, WAV header and stereo speaker mask;
worker tests cover source-to-output clock conversion, seek and rewind. Five
assets regenerate deterministically without codec executables. This qualifies
MP4 explicit SBR output clocks; Matroska core-clock metadata, bare LC implicit
SBR clock discovery and missing-fill PS playback remain separate gaps.

Matroska OutputSamplingFrequency (2026-10-09): the shared root/owned reader
honors Audio/OutputSamplingFrequency (0x78B5), independently of element order,
with the same positive finite validation as SamplingFrequency. The default
remains SamplingFrequency when no output element is present. Source timestamps
remain nanoseconds. Four original LC/SBR in-band PS 960/1024 videos declare
SamplingFrequency=24000 and OutputSamplingFrequency=48000, with both orders.
Before the fix the exact reader regression reports 24000 instead of 48000.
Acceptance covers independent PCM/WAV, gaps, intervals, packet-limit EOF,
controlled memory admission, native AVC frame, negative preroll, seek and
rewind. Four malformed variants refuse zero/negative/NaN/infinity explicitly;
these are refusal tests, not playback acceptance. Nine assets regenerate offline
and deterministically. Element semantics follow
https://www.matroska.org/technical/elements.html#OutputSamplingFrequency .
ASC-only inference when OutputSamplingFrequency is omitted, implicit output
clock discovery and PS playback without SBR fills remain separate gaps.

Mono PCE PS qualification (2026-10-09): native PS decoding and the syntax probe
accept an explicitly configured sole normal front SCE with a nonzero tag (3).
Matching in-band PCEs are validated; wrong SCE tags, changed program layouts and
checkpoints from another PCE configuration are refused transactionally. Before
the fix, the original synthetic acceptance fixture reproduced the exact blanket
`native PS decoder requires one mono AAC-LC element` refusal. Six LC/SBR/PS
960/1024 cases cover both 24/48 kHz output modes against independent PCM,
checkpoint replay, reset and EOF. Twelve MP4/Matroska videos cover root/owned
PCM, negotiated reader factories and WAV export, including Matroska gaps.
Fifteen assets regenerate deterministically using only Python stdlib and owned
synthetic inputs; ordinary tests do not invoke generators or FFmpeg. This does
not qualify coupling, multiple elements, height layouts, general SBR/PCE or
missing-SBR-fill PS playback.

Matroska AAC ASC geometry (2026-10-09): when OutputSamplingFrequency is absent,
explicit or sync-extension SBR ASC supplies its declared decoded clock if Audio
states the core rate or no rate. Explicit PS ASC expands core channel metadata
to decoded stereo. Explicit container output clocks and conflicting core rates
are retained, and bare LC does not acquire a guessed doubled clock. Eight
original SBR/PS 960/1024 Matroska videos reproduce the old reader's 24/48 kHz
mismatch; real worker acceptance additionally exposed mono metadata retained
for explicit PS. Acceptance compares independent stereo PCM through root/owned
export, intervals, packet limits, WAV and memory admission; native AVC plus
worker preroll, EOF, seek and rewind are exercised on all eight videos. Three
control videos verify metadata precedence and no guessed LC clock. Twelve
assets regenerate offline and deterministically. AAC CodecPrivate carries ASC:
https://www.matroska.org/technical/codec_specs.html#a_aac . This closes ASC-declared
geometry, not implicit output-clock discovery from bare LC payloads or PS
playback without SBR fills.

AAC/PS missing SBR fill (2026-10-09): the native decoder now advances core
analysis/QMF and delayed stereo synthesis at the configured output clock when
raw_data_block has no SBR extension. It retains transmitted syntax histories,
queues direct dual mono for that frame, and does not invent PS presence. Eight
original 960/1024 video sequences cover startup, middle, trailing and all-missing
fills, with independent stereo PCM for both 24/48 kHz output clocks. Six additional
mono-metadata videos qualify actual in-band PS discovery. Before the fix the
acceptance fixture failed exactly with `PS AAC block requires SBR/PS fill`.
Four nonzero core PCM direct-convolution oracles verify that the bridge does
not replace core audio with zeros. Acceptance covers checkpoint/replay/reset,
EOF, root/owned MP4 PCM, WAV and playback decoder factories with original source
windows. A candidate that never receives PS still refuses EOF without committing
its pending frame; this is a refusal test, not PS playback acceptance. The former
missing-fill refusal entry is moved to accepted metadata; malformed trailing,
truncated, duplicate and late-element cases retain rollback checks. Seventeen
new assets regenerate offline deterministically, and ordinary tests require no
FFmpeg or generator. This supersedes the missing-fill limitation for the tested
sole-SCE layouts; it does not qualify general multi-element/coupled SBR/PS or
implicit output-clock discovery from bare LC metadata.

Sole-element PCE SBR (2026-10-09): native SBR admission now permits an explicit
normal front PCE containing one SCE or CPE without coupling. Existing PCE tag,
in-band layout and checkpoint identity validation remains active; QMF/DSP uses
the decoded one/two-channel geometry. Thirty original cases qualify 960/1024,
24/48 kHz, explicit/sync SBR and container-hinted implicit double-rate SBR, with
coupled and uncoupled CPE SBR. Before admission changed, the synthetic acceptance
fixture failed exactly with `SBR multielement/PCE synthesis is not yet implemented`.
All samples match the independent existing mono DSP oracle in every channel,
including reset and checkpoint replay. Eighteen original MP4 videos accept
root/owned PCM, intervals, WAV and playback decoder factories; implicit SBR is
also discovered from actual FIL payloads. Four tagged/layout-error videos refuse
with the intended PCE errors and preserve filter/noise state; these are refusal
tests. Twenty-four new assets regenerate offline deterministically; ordinary
tests never run generators or codec executables. Multi-element PCE, coupling
with SBR, height-layer PCE and implicit downsampled SBR remain unqualified and
are not claimed by this sole-element acceptance.

Multi-element AAC SBR (2026-10-09): SBR syntax and QMF/DSP state is now retained
per configured audio element offset, rather than once for the entire block.
FIL reads one SCE/CPE's width, independent of total output channels; synthesis
maps that element back to canonical PCE/standard PCM positions. Checkpoints,
reset, transactional decode and retained-allocation inspection include every
state. Core-only LC does not allocate these states. AAC coupling with SBR is
still refused; independent element processing removes the former sole-element
and height admission restrictions.

One hundred authored cases cover two SCEs, PCE and indexed 5.1, reordered PCE elements,
a missing FIL in one back pair, and mixed top-front/normal-back PCE positions and indexed height configuration 14,
with explicit/sync SBR, implicit double-rate, 960/1024 and 24/48 kHz. Every channel
matches independently generated mono/missing-fill DSP PCM, including LFE
upsampling and distinct histories. Reverting the implementation and allocation
visitor to the previous commit reproduces the exact former constructor refusal
`SBR requires one normal front SCE/CPE without coupling`. Sixty MP4 videos
accept root/owned PCM, intervals, WAV speaker masks, memory admission and
playback decoder factories with preserved source windows. A later CPE CRC-error
video reproduces the exact SBR CRC error and verifies rollback of earlier SBR,
core and filter state; it is refusal evidence, not acceptance. Sixty-three new
assets regenerate deterministically with no foreign codec or network. Ordinary
tests never execute the generator. This qualifies the listed layouts, not AAC
coupling, all possible profiles/height combinations or PS on multi-element cores.

### 2026-10-09 — dependent AAC CCE before target SBR

`generate_he_aac_dependent_coupling_fixtures.py` hand-authors 80 nonzero AAC
cases and 48 short MP4 videos. It covers 960/1024 core samples, SCE/CPE target,
coupling points before/after target TNS, separate stereo gains, 24/48 kHz
output, explicit/sync signalling and container-hinted implicit double-rate SBR,
and both present and absent target FIL. The spectral CCE has four alternating
nonzero coefficients; target TNS is active, so coupling before/after TNS is
observably different. The generator is offline and reads owned Huffman tables
and existing originally authored container/video seeds; no private media or
FFmpeg is involved. Ordinary tests consume checked-in fixtures only.

The PCM evidence has two parts: Python independently evaluates direct-cosine
IMDCT, sine windows, first-order TNS and chronological overlap for the core;
the Rust test passes that core into the separately qualified standalone SBR
DSP to verify decoder composition. This is not a newly independent complete
SBR implementation or a new external-codec accuracy comparison.

`he_aac_dependent_coupling` checks all samples, checkpoint replay, reset,
actual in-band SBR discovery, root/owned demux/export agreement, playback
sample windows, interval slices and WAV geometry. Two malformed MP4 videos
reproduce the exact missing-CCE-target error after a valid frame, and verify
that parsed SBR histories roll back. Independent PCE coupling remains a
refusal, explicitly tested across the signalling/rate/layout variants.

The original constructor refusal was reproduced before admission changed:
`SBR AAC coupling synthesis is not yet implemented`. Admission now allows
only dependent PCE coupling: existing spectral mixing happens before target
IMDCT and SBR. Independent CCE needs its own SBR state and mixing after SBR;
SBR FIL directly following CCE is still unsupported. The reference order was
inspected in [the primary decoder source](https://ffmpeg.org/doxygen/trunk/libavcodec_2aac_2aacdec_8c_source.html),
used for research only, without production linkage or code copying.

Local offline verification for this change: root/player library 909 passed,
23 ignored; owned media library 432 passed, 1 ignored; all 42 AAC integration
suites 172 passed. Total: 1513 passed, 24 ignored, no failures. Regeneration of
all 53 new assets was byte-identical. This does not establish acceptance of
independent CCE/SBR, CCE FIL, or untested profiles.

### 2026-10-09 — independent AAC CCE after per-element SBR

`generate_he_aac_independent_coupling_fixtures.py` authors 240 long-window
cases with 960/1024 core samples, mono/stereo targets, one/two independent CCEs,
tags 1/15, alternating CCE wire order, separate stereo gains, 24/48 kHz output,
explicit/sync ASC and container-hinted implicit double-rate output. Target FIL
can be absent; CCE FIL can be absent or missing for a middle packet. 144 short
MP4 acceptance videos include an original AVC track. Two malformed videos
exercise missing target and protected CRC failure in a later wire CCE.
Regeneration uses only the Python standard library and saved owned protocol
assets; ordinary tests neither generate fixtures nor invoke codec executables.

`he_aac_independent_coupling` uses independently computed direct-cosine CCE
core PCM followed by separately qualified standalone SBR DSP for each tag.
The target's silent core goes through a separate SBR state. Gains are applied
only after both target and CCE SBR, including the correctly silent cancellation
of two independently nonzero opposite mono CCEs without FIL. This qualifies
stage composition, not a new independently implemented full SBR oracle.

Decoder SBR slots reserve a separate four-bit CCE tag domain after canonical
audio-element slots. CCE FIL is mono and binds to the immediately preceding
independent CCE. Syntax/DSP states clone transactionally, participate in memory
reporting/checkpoints, and reset together with IMDCT/overlap synthesis history.
The ASC-aware budget includes 2 MiB for each configured independent CCE in
addition to output channels; MP4 additionally admits its three whole decoder
states. The prior independent-ASC constructor refusal test now accepts admission
and still rejects a dependent in-band PCE changing the configured program.

The primary reference's [ordering of IMDCT, SBR and independent coupling](https://ffmpeg.org/doxygen/trunk/libavcodec_2aac_2aacdec_8c_source.html)
was inspected for research; no implementation was copied or linked. Dependent
CCE-owned FIL, coupling with PS, other AAC profiles and unqualified window/tool
combinations remain open. Full native codec coverage is not established here.

Local verification: root/player library 909 passed, 23 ignored; owned media
library 432 passed, 1 ignored; 43 AAC integration suites 176 passed. After that
suite set was compiled, one additional focused acceptance test verified 12
bare-LC configurations without any FIL: candidate discovery retains 24 kHz and
matches ordinary LC PCM and the independent core oracle. Combined current-code
qualification: 1518 passed, 24 ignored, no remaining failures. All 149 new
fixture assets regenerate byte-identically. These checks qualify the stated
long-window CCE/SBR cases, not all AAC profiles or all codec tools.

### 2026-10-09 — dependent CCE-owned SBR FIL syntax

`generate_he_aac_dependent_fil_fixtures.py` adds valid mono FIL syntax after
an originally authored dependent CCE. 160 cases and 96 MP4 acceptance videos
cover coupling before/after TNS, mono/stereo target, 960/1024 frames, 24/48 kHz
output, explicit/sync/hinted implicit signalling, target FIL present/absent,
and CCE FIL present throughout or missing in a middle packet. The generator
extends the existing original dependent-Coupling packet author, whose 53
baseline assets remain byte-identical. New assets regenerate deterministically.

The decoder parses and retains the dependent CCE's own SBR header, CRC and
temporal coefficient history, indexed by four-bit CCE tag. Dependent CCE mixes
spectra before target IMDCT/SBR; it does not add a separate CCE SBR PCM signal.
The new test therefore requires byte-exact equality with the already qualified
no-CCE-FIL baseline, as well as comparison with independent direct-cosine core
PCM followed by separately qualified SBR DSP. Headerless later FIL, checkpoints,
reset and in-band clock discovery exercise retained syntax history. Memory
reporting includes this extra state, and conservative SBR admission now reserves
2 MiB per configured CCE (dependent or independent) plus output channels.

Four malformed/unsupported videos distinguish acceptance from refusals: two
corrupt CCE CRC, and two contain a PS extension in the dependent CCE FIL. They
verify the specific CRC or unsupported-extension error and rollback of all
preceding target/CCE histories. Passing the PS refusal is not PS acceptance;
CCE extension bytes are never silently ignored as supported audio processing.
Container tests compare root/owned PCM, interval slices, WAV geometry and actual
playback packet sample windows. Tests are offline and use checked-in assets;
generators remain separate from ordinary test execution.

The primary reference accepts CCE as [mono SBR data syntax](https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/aacsbr_template.c),
while its [sample conversion](https://ffmpeg.org/doxygen/trunk/libavcodec_2aac_2aacdec_8c_source.html)
performs CCE IMDCT/SBR only for independent coupling. These sources were used for
research only, without copying implementation or linking external codec code.
Coupling with PS, broader AAC profiles and unqualified tools remain open.

Local offline qualification: root/player library 909 passed, 23 ignored; owned
media library 432 passed, 1 ignored; all 44 AAC integration suites 180 passed.
Total: 1521 passed, 24 ignored, no failures. All 102 new assets regenerate
byte-identically, and the existing 53 dependent-SBR assets are unchanged.

### 2026-10-09 — AAC coupling with target PS

Owned PS decoding now admits dependent CCE before/after target TNS and
independent CCE with its own mono SBR, mixed into the target SCE left channel
after PS. Packet-indexed pending PCM follows PS lookahead and EOF draining;
checkpoint/reset and failed-packet rollback include CCE histories.
108 authored long-window cases and 54 synthetic acceptance MP4s cover
960/1024 samples, three coupling points, sparse tags, changing wire order,
missing FIL, output clocks and explicit/in-band signalling. Three malformed
or unsupported videos check missing targets, source CRC and source PS refusal.
Core PCM uses an independent cosine oracle; full output is stage composition
with the previously qualified owned SBR/PS DSP, not an independent full decoder.
Root/owned export, repeated ranges, WAV and playback seek/rewind are tested.
CCE-owned PS and multi-element target PS remain unqualified. Fixture generation
is separate and offline; ordinary tests need neither FFmpeg nor network.

### 2026-10-09 — PS discovery belongs to the target SCE

Twelve additional own synthetic videos cover 960/1024 and coupling points
0/1/3 with CCE-owned mono SBR FIL. Without target PS, the syntax probe stays
false, the PS candidate refuses EOF specifically for missing PS, and ordinary
owned AAC playback/export accepts mono. With target PS first appearing in
packet one, negotiation accepts stereo and preserves preceding core history
and delayed PCM. These are acceptance tests for already implemented behavior,
not a claim of CCE-owned PS support. Root/owned PCM agrees; the expected PCM
uses the independent core oracle and qualified own SBR/PS stage composition.
Full playback, rewind and seek agree with export. Fixture generation remains
offline and deterministic; tests do not generate fixtures or invoke FFmpeg.

### 2026-10-09 — sequential PS elements in one extension area

The former at-most-one-PS refusal is superseded for multiple PS elements
inside one mono target SBR extension area. Syntax and native parameter history
advance in wire order; the final parameters drive one QMF/PS synthesis step.
This matches the extension-area loop in the reference parser:
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/aacsbr_template.c
No reference implementation is linked or invoked by production or tests.

Twelve authored 960/1024, core/double-clock, explicit/implicit cases contain
a first coarse 10-band PS followed by a distinct fine 34-band PS. PCM must
equal a single-final-PS baseline using independent core PCM and the separately
qualified owned DSP. Six MP4 acceptance videos cover root/owned export, ranges,
WAV, playback, rewind and seek. A reserved IID mode in a later PS element
requires exact refusal and transactional rollback; it is not an acceptance.
Multi-element target audio programs and CCE-owned PS remain separate gaps.

### 2026-10-09 — sequential PS temporal history

Twelve further sequential-PS cases (960/1024, core/double clock, three ASC
signallings) transmit nonzero positive/negative temporal IID/ICC and modulo-8
IPD/OPD deltas. The second PS depends on the first in the same extension area;
following packets retain both updates. Assertions check every native band index
against scalar accumulated values before PCM comparison to separately encoded
absolute-parameter packets. Six own MP4 videos qualify export/ranges/WAV and
playback seek/rewind. This extends qualification of the sequential-PS fix; it
does not introduce a new production implementation or close other AAC profiles.
The generator's default behavior preserves the existing PS fixture bytes.

### 2026-10-09 — AAC gain-control syntax and empty adjustments

The channel reader no longer refuses every gain-control flag. A shared owned
parser reads max_band, adjustment counts, levels and locations for OnlyLong,
LongStart, EightShort and LongStop. Syntax storage is bounded by wire fields
(3 bands, 8 windows, 7 adjustments), and a truncated input commits no cursor.
Empty lists are a no-op and now proceed to spectral decoding. Active lists
retain a specific unsupported-synthesis error; SSR inverse PQF and actual gain
compensation remain gaps. This is not full SSR/gain-control acceptance.
The field widths are checked against the primary reference parser:
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/aac/aacdec.c

32 own long-window videos cover 960/1024, LC/PS, target/CCE and max_band 0..3.
PCM equals no-gain baseline exactly, with independent cosine core PCM for LC;
root/owned export, repeated ranges and playback rewind/seek agree. Two active
adjustment videos reproduce the precise refusal and preserve overlap/history.
Separate syntax tests cover all window modes, maximum lists and every byte
truncation. Fixture generation is deterministic/offline and outside tests.
Container clock overrides were added to the own MP4 generator; existing default
fixture bytes are unchanged.

### 2026-10-09 — AVC end-of-sequence/end-of-stream NAL admission

The owned AU preparer and stateful decoder now admit NAL types 10/11 as
non-VCL markers. They do not participate in slice coverage or fabricate
a decoded picture. Decoder output ordering remains caller-owned; playback
drains its existing B-frame queue at container EOF. Reference parsing also
classifies these as non-VCL:
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/h264dec.c

Six short synthetic MP4s append sequence, stream or both markers to the final
AU of the existing authored eight-frame two-slice I/P/B fixture, repacked at
2/4-byte NAL lengths. Before the fix both prepare and software playback failed
exactly with unsupported in-band AVC NAL. Acceptance checks compare each
decode-order picture to the unmodified baseline and full display-order YUV
to the saved oracle after rewind/seek. Marker-only packets return no picture,
and explicit decoder reset permits replay. Fixture generation is deterministic
and requires neither FFmpeg nor network. These cases do not qualify auxiliary
slices, scalable profiles or other still-unsupported AVC reconstruction tools.
