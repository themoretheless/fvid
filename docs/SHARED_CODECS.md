# Shared owned compressed-video kernels

`fvid-codecs` compiles the canonical AVC, HEVC, VP9 and AV1 source modules once. The root crate reexports these modules; `fvid-media::owned_codecs` exposes the same public types. Shared error and fallible allocation primitives live in `fvid-control`. Canonical source files remain in this monorepo; this change does not qualify standalone package publication. The player feature retains HEVC worker priority through `fvid-platform`.

Plain WebM VP9/AV1 requests now dispatch through these kernels in both native-only and legacy-enabled fvid-media builds. AV1 configuration OBUs initialize the decoder before packets, including files whose first packet has no sequence header. Malformed packets propagate errors; explicitly unsupported tools remain capability refusals. The native-only API refuses transformed compressed sources rather than silently dropping options.

This is a migration step, not complete FFmpeg independence. MP4 AVC/HEVC library dispatch, transformed compressed-video workflows, remaining exports and codec/profile gaps still require migration. Root media and CUDA features still enable legacy-ffmpeg. Presence in the capability inventory denotes linked packet-decoder components, not acceptance of every container workflow or profile.

Verification: shared-kernel unit tests, root and standalone media unit tests, native deband/perspective regressions, six shared-codec integration tests and the offline native dependency guard. Ordinary tests require neither FFmpeg nor network access.
