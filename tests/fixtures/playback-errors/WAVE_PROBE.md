# Shared owned WAVE probe regression

`wave-probe-info.wav` contains 0.1 seconds of 48 kHz mono PCM16 alternating
+/-128, with a RIFF INFO title after its audio payload. The paired
`wave-probe-info.y4m` contains three synthetic 16x16 frames at 30 fps.
Generate both with `python3 scripts/generate_wave_probe_sample.py`; generation
uses Python only and is separate from ordinary tests.

The former frontend WAVE description discarded INFO tags even though the
owned library parser retained them. The regression was run against that
implementation and failed with empty metadata rather than a parsing error.
The acceptance test checks the title, duration, full metadata equality between
frontend/library and CLI output. Frontend probing now delegates to the library
parser, including its container bit rate and audio frame-rate fields.

Library tests additionally cover PCM8/16/24/32, 24 valid bits in 32-bit storage,
float32, stereo speaker masks, and explicit WAVE format hints. No test invokes
FFmpeg or a fixture generator.
