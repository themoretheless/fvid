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

Remaining FFmpeg-dependent validation programs include `validate_media.py`,
`validate_hw_cuda.py` and `validate_klite_coverage.py`. Their codec/filter/reference
coverage has not been removed or relabeled as migrated. Production legacy media
operations and AAC aggregate allocation admission also remain separate work.
