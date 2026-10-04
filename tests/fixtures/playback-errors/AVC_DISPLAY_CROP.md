# AVC display crop

`avc-display-crop.mp4` contains eight synthetic 62x46 I/P/B pictures. AVC codes
64x48 and uses SPS crop `[0, 2, 0, 2]` to remove padding on the right and bottom.
This exact nonzero crop triggered the old native production-route refusal.

Generate separately with `python3 scripts/generate_avc_crop_sample.py`: x264
encodes public arithmetic patterns and the repository's own fixture muxer writes
MP4. No FFmpeg, private input or private parameter sets are involved. Normal
regression tests only read the checked-in file, with no external programs/network.

Host acceptance parses the SPS, qualifies all packets, decodes every picture
with FVid and checks visible dimensions plus a nested crop. The ignored NVIDIA
acceptance exports visible size and a cropped/flipped variant through the actual
production entrypoint, then reads and decodes both outputs using FVid. Hardware
execution remains unverified; host acceptance alone does not prove GPU output.
