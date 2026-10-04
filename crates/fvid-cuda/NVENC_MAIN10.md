# Direct NVENC Main10 status

The owned `NvencSession` exposes `initialize_p010[_with_colour]`,
`register_p010` and `submit_p010`. Registration extents/pitches are bytes; row
width is twice the pixel width. P010 pointers/pitches must be word-aligned.
The initialized format persists through resource registration, mapped-format
validation and picture submission, so a P010 session cannot accidentally use
the NV12 path. Wrong-format calls are refused before invoking the driver.

The pinned SDK 8.1 HEVC configuration selects Main10 profile GUID and
`pixelBitDepthMinus8=2`, preserving supplied colour VUI. This follows the
[NVIDIA NVENC programming guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/12.0/pdf/NVENC_VideoEncoder_API_ProgGuide.pdf)
and the pinned `nvEncodeAPI.h` layout documented in `nvenc_session.rs`.

Host tests verify profile/depth/colour, the SDK input-format value and bounded
P010 row/plane footprints. The ignored NVIDIA test submits four P010 frames and
drains timestamped encoded packets. It is a driver/resource smoke test; it has
not run here and does not verify encoded SPS or pixels.

The owned movie encoder now retains typed P010 pools until output is drained,
uses P010 device copies, and selects Main10 session registration/submission.
Its owned HEVC Annex B writer builds hvcC and Matroska packets. The production
MP4-to-Matroska CUDA route admits qualified Main10 and preserves ten-bit output.
An ignored end-to-end NVIDIA test covers blank/repeated movie occurrences,
encoded hvcC depth, timestamps and decoding by FVid. It has not run here.
CPU tests prove bit-exact Annex B conversion on synthetic Main/Main10 I/P/B
fixtures; they do not prove NVENC output or NVIDIA performance.
Production `media-cuda`/`cuda-hw` features now enable owned adapters without
libav. Remaining unsupported routes are reported explicitly; native functional
coverage and physical NVIDIA execution remain incomplete. Explicit
`legacy-ffmpeg` is reserved for reference benchmarks.
