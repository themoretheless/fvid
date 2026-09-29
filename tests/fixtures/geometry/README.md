# Quarter-turn reference fixtures

Reference-only generation: no FFmpeg is invoked by the Rust tests.

ffmpeg version 9.0.2 Copyright (c) 2000-2026 the FFmpeg developers

Input: 8x4 yuv420p10le, Y[i]=64+13*i (32 samples), Cb[i]=500+7*i and Cr[i]=600-9*i (8 each), little-endian u16.

Input SHA256: `61f43d78a6e2c2aba11218ec4ac010dee31890c7fa7f52dfa43003cb74017184`

Transpose modes follow https://www.ffmpeg.org/doxygen/trunk/vf__transpose_8c.html.

`/opt/homebrew/bin/ffmpeg -y -v error -f rawvideo -pixel_format yuv420p10le -video_size 8x4 -color_range tv -i /tmp/fvid-geometry-reference-input.yuv -vf transpose=clock -frames:v 1 -pix_fmt yuv420p10le -f rawvideo tests/fixtures/geometry/clock.yuv`

Output SHA256: `47d17d4ebf9f5346913707d28ce40c1bb1006e6ae63806e9834c7b105a77be25`

`/opt/homebrew/bin/ffmpeg -y -v error -f rawvideo -pixel_format yuv420p10le -video_size 8x4 -color_range tv -i /tmp/fvid-geometry-reference-input.yuv -vf transpose=cclock -frames:v 1 -pix_fmt yuv420p10le -f rawvideo tests/fixtures/geometry/cclock.yuv`

Output SHA256: `a20ad2ad827b03458ac9e05a984d0f0d400e7b8144bd60ac1ca110e9bdd8ae0a`

`/opt/homebrew/bin/ffmpeg -y -v error -f rawvideo -pixel_format yuv420p10le -video_size 8x4 -color_range tv -i /tmp/fvid-geometry-reference-input.yuv -vf transpose=clock_flip -frames:v 1 -pix_fmt yuv420p10le -f rawvideo tests/fixtures/geometry/clock_flip.yuv`

Output SHA256: `7785fd7ece9776ee5dfc50edf797969875ddfd6109438182adcc5538cb3efab4`

`/opt/homebrew/bin/ffmpeg -y -v error -f rawvideo -pixel_format yuv420p10le -video_size 8x4 -color_range tv -i /tmp/fvid-geometry-reference-input.yuv -vf transpose=cclock_flip -frames:v 1 -pix_fmt yuv420p10le -f rawvideo tests/fixtures/geometry/cclock_flip.yuv`

Output SHA256: `e2c2e16da3eecbb94bc50f395560d7b87492d57d10396a90f74177ca30b59625`

`422.y4m` is an authored single 4x2 frame with luma bytes 0..7,
Cb `[10,11,12,13]` and Cr `[20,21,22,23]`. It checks that decoded statistics
report 4:4:0 after a quarter-turn of 4:2:2 data.

The reference `.yuv` files contain only the turned 4x8 image. Padding is tested
against exact neutral sample values separately: the own 10-bit implementation
uses Y=64, Cb=Cr=512 for limited-range black. FFmpeg 9.0.2's `pad=...:black`
produced Cb=Cr=514 in a preliminary comparison, so no bit-equivalence claim is
made for that colour conversion.
