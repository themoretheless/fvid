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

This does not yet enable production Main10 export: the movie encoder still
needs P010 retained pools/copies, plus an owned HEVC Annex B/configuration writer.
The production CUDA features still include legacy libav.
