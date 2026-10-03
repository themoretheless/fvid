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
