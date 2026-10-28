# Layered HEVC container presentation order

Generated separately by `scripts/generate_mp4_layered_seek_coverage.py` without
FFmpeg or network. Four independently decodable access units repeat the owned
64×64 PCM witness from `hevc-layered-reconstruct-pcm-independent.mp4`. Each AU
contains the two independent layers. The configuration and pictures originate
from the project's synthetic generator, never private video.

Both files present four frames at 25 Hz (160 ms). `ordered.mp4` states PTS
0,1,2,3; `reordered.mp4` states 0,2,1,3 in decode order through signed `ctts`.
The common video reader must display intervals 0–1,1–2,2–3,3–4 in either case.
Sequential pixels must match the saved synthetic PCM oracle; seeks must agree
with sequential playback, including rewind and repeated EOF. This is valid-input
playback acceptance, not a malformed-input refusal test. No mutation audit.
