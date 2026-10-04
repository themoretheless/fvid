# Native HEVC NVDEC adapter status

`owned_nvdec_hevc::configuration` translates FVid-owned SPS/PPS data into
the pinned NVDEC HEVC configuration structure for 8-bit and 10-bit 4:2:0.
Host tests use the checked-in synthetic Main/Main10 fixtures and verify
parameter mapping, scaling-list conversion, tile bounds and reference sentinels.
They require neither FFmpeg nor an NVIDIA GPU.

This is configuration translation only. It is not an operational HEVC GPU
decoder: picture submission, short-term RPS bit accounting, POC/reference-slot
scheduling and production routing remain to be implemented. Main10 also needs
a compatible output/render/encode path. Driver acceptance and decoded image
correctness have not been verified on NVIDIA hardware.

The production CUDA features still include the legacy libav backend. This
adapter alone does not remove that dependency.
