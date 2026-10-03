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

## Owned route selection regressions

`wave-rematrix-implicit.wav` contains 4800 stereo PCM16 frames at 48 kHz,
with left 4096/right 12288 and a conventional WAVE format without an explicit
speaker mask. Its paired Y4M video has the same 0.1 second duration. The
previous generic layout check refused this input before WAVE's valid implicit
mono/stereo check could run. The acceptance test requires 4800 mono float32
samples equal to 0.25; unknown multichannel layouts remain rejected.

The INFO fixture also reproduces `media plan concat a.wav b.wav` being routed
to PCM Matroska decoding while the corresponding WAVE command copies its
samples and metadata. The acceptance test requires the default plan to equal
the WAVE copy plan; explicit `--output-format mkv/mka` must retain the owned
Matroska decode/write plan. Both regressions were run against the previous
implementation and failed for these specific reasons, then passed with the fix.

The same route selection issue affected homogeneous ADTS/AAC concatenation.
`aac-concat-route.aac` retains the first three complete packets from the
committed synthetic `aac-packet-prefix.aac`, dropping its incomplete tail;
`aac-concat-route.y4m` reuses the corresponding three-frame synthetic video.
Generation reads only those committed fixtures and never invokes a codec.
Separate ADTS and WAVE acceptance tests check packet/PCM copy plans in the CLI
and (when enabled) public media API; mixed-container PCM concat continues to
use its decoded route. Explicit Matroska plans remain available in both cases.

## Float WAVE speaker-mask preservation

`wave-float-wide-mask.wav` contains 16 synthetic float32 sample frames at 48 kHz
with eight channels and explicit speaker mask 0xff. Its paired one-frame 16x16
Y4M runs at 3000 fps. The same Python generator creates both. The previous
shared DSP output dropped this explicit mask for float32 inputs: the regression
failed specifically with mask 0 instead of 255 while retaining eight channels
and 16 frames. The acceptance test now requires all three values to survive.

## Read-only ancillary chunk inspection

`wave-probe-ancillary.wav` adds an odd-length synthetic opaque `xtra` chunk
and an empty `LIST/adtl` to the PCM control. Regenerate with
`generate_wave_probe_sample.py`; its matching short video is
`wave-probe-info.y4m`. Read-only probe must retain the original INFO metadata
and audio geometry while skipping these bounded ancillary chunks. Editing
still refuses unknown chunks whose metadata cannot yet be safely retimed.
`wave-probe-ancillary-overflow.wav` changes the LIST length to exceed RIFF:
this malformed-input refusal must remain `WAVE chunk exceeds RIFF extent`.
Tests use neither external codecs nor runtime fixture generation.
