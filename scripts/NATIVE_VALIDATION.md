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

The dependency policy also audits the four declared native validator sources for
known literal external launches/environment hooks, failing on missing or malformed
source. Explicit reference benchmark modules remain allowed. This complements
the dispatch tests; it does not resolve arbitrary computed executable paths.

Local ordinary media validation on 2026-10-02 executed 116 owned-library and
1,224 frontend Rust tests successfully, with one frontend test ignored. It
recorded `reference_completed=false` and `provided_cli_checks_completed=false`;
the full external corpus was not run in that invocation. The separate production
dependency audit still refuses `media`, `media-cuda` and `cuda-hw` because they
activate `legacy-ffmpeg`. Passing native tests does not close that boundary.

`validate_klite_coverage.py` still requires external FFmpeg tools. Its codec/filter/
reference coverage has not been removed or relabeled as migrated. Production legacy media
operations and AAC aggregate allocation admission also remain separate work.
