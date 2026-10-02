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
