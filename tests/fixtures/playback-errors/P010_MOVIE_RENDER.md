# Main10 P010 movie rendering qualification

`hevc-main10-cuda-edit-repeat.mp4` uses only the checked-in synthetic
`tests/fixtures/hevc/main10-ipb.mp4` seed. The own generator
`scripts/generate_playback_error_samples.py` changes only edit/timeline metadata
and requires neither FFmpeg nor network access. It creates the same two blank
spans and repeated six-picture ranges as `hevc-cuda-edit-repeat.mp4`.
Normal tests never run the generator or use private videos/parameter sets.

Host acceptance decodes every packet with FVid and checks the exact 14-event,
0.6-second movie timeline. Owned P010 allocations use byte pitches and 16-bit
words whose significant bits are 15..6. Black fill initializes both complete
pitched planes, including padding, using limited luma 64 (or full-range 0) and
neutral chroma 512. CUDA's `cuMemsetD16Async` count is in 16-bit elements, so the
fill passes half the byte extent ([NVIDIA Driver API](https://docs.nvidia.com/cuda/cuda-driver-api/cuda_driver_api/group__CUDA__MEM.html)).

The ignored NVIDIA tests check actual black-fill words/padding and Main10 movie
rendering with reflections, blanks, repeats and an optional constant shader.
They have not run here. The render test checks events/formats/pass counts; it
does not compare decoded GPU pixels against the software decoder.

This is P010 buffer/render support, not complete Main10 production export.
NVENC Main10 and owned HEVC output/container integration remain pending.
