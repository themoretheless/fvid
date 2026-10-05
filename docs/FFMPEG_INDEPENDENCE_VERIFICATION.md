# FFmpeg independence verification

## Status on 2026-10-06

The published verification baseline is commit
`b9c78463964ce7703d6832b7078523e171ada2b8`.

Production dependency graphs, ordinary tests, fixture generators and validators
were audited without FFmpeg adapters or external FFmpeg calls. Native macOS and
aarch64 Linux builds and tests completed without an FFmpeg SDK; runtime checks
also used a PATH with no external programs. FFmpeg remains a benchmark/reference
dependency, not a production backend.

Windows and NVIDIA verification was supplied in commit `670dc44d` and confirmed
by the user on 2026-10-06. `benchmarks/windows-cuda-validation.json` records
28 passing physical CUDA tests on an RTX 5090 with driver 617.14, including
NVDEC/NVENC, NV12/P010, shader sampling and production decode/filter/export.
`benchmarks/windows-cuda-cli.json` records a passing Main10 CLI export: 14 frames,
zero decode errors and zero host frame copies. These are supplied hardware
results, not locally rerun macOS checks or universal GPU qualification.

GitHub-hosted Windows CI was separately blocked by an account billing issue.
That infrastructure failure does not invalidate the user's local verification.

FFmpeg independence does not establish universal codec-profile support, 60 fps
on every device, or macOS virtual-camera provisioning. Older implementation
entries in NATIVE_PLAYBACK.md and SHARED_CODECS.md describe intermediate states
and should not be read as the current production dependency status.
