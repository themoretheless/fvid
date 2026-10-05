# FFmpeg independence verification

## Status on 2026-10-06

The published verification baseline is commit
`b9c78463964ce7703d6832b7078523e171ada2b8`.

Production dependency graphs, ordinary tests, fixture generators and validators
were audited without FFmpeg adapters or external FFmpeg calls. Native macOS and
aarch64 Linux builds and tests completed without an FFmpeg SDK; runtime checks
also used a PATH with no external programs. FFmpeg remains a benchmark/reference
dependency, not a production backend.

On 2026-10-06 the user reported completing all requested Windows checks. Windows
verification is therefore recorded as user-confirmed, rather than outstanding.
No Windows command logs, tested commit identifier or individual hardware results
were supplied with that confirmation; this is not an independently observed CI
result or a separate assertion of NVIDIA performance or codec coverage.

GitHub-hosted Windows CI was separately blocked by an account billing issue.
That infrastructure failure does not invalidate the user's local verification.

FFmpeg independence does not establish universal codec-profile support, 60 fps
on every device, or macOS virtual-camera provisioning. Older implementation
entries in NATIVE_PLAYBACK.md and SHARED_CODECS.md describe intermediate states
and should not be read as the current production dependency status.
