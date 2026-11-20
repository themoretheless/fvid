# Exact LD-SBR Q31 interoperability

Generate explicitly with `python3 scripts/generate_aac_ld_sbr_q31_fixtures.py`.
Ordinary tests never run generators,foreign codecs,FFmpeg or network access.

18 authored eight-AU programmes in MP4 and Matroska cover480/512 core samples,
CRC,inverse-filter modes0/1/2/3,centered stereo coupling and asymmetric coupling.
Each combination has Q7/Q30/Q31. Q30/Q31 change the noise level in the fourth AU;the next AU
uses a temporal delta back toQ7 and another AU holds that temporal reference.
This distinguishes exact Q31 reconstruction/history from silently clamping31
or replacing output. Q7 control bits and direct-convolution PCM are asserted
identical to existing own inverse-filter/coupled fixtures. Q30/Q31 use the
same independent direct CLDFB/HF/noise oracle with explicit frame noise levels.
No private/ISO media,frames,audio or codec parameter sets are copied.

Six counterexamples reach Q31 then encode a temporal+1 in the fifth AU,producing Q32.
Default admission refuses exactly Q32 in both containers and retains Q31
history for a corrected retry.32 is not rewritten or silently concealed.

Default LD-SBR `NoisePolicy::AllowQ31` is a bounded interoperability extension,
not a new normative range: it admits exactly31 beyond the normative0..30 and
uses the unchanged2^(6-Q) formula. Existing coupled balance and integer/finite
bounds remain enforced. `AacDecodeOptions { ld_sbr_noise: NoisePolicy::Strict,
..Default::default() }` restores the normative rejection. Legacy SBR always
retains its existing strict contract. Policy survives reset/checkpoints and
incompatible-policy checkpoint restores fail before mutating state.

Before the fix,the new Q31 playback acceptance fails with the exact noise-range
error. After the fix,all eighteen programmes match independent PCM;both containers
match byte-for-byte. Audio-worker checks cover live-history forward/backwards,
repeated seeks,core boundaries,EOF,rewind and presentation timestamps. The older
`aac-eld-sbr-noise-range/n{480,512}-q31` movies also become default acceptance
without changing any movie/AU bytes;their explicit Strict-policy tests remain.

Complete decoding of all26 original ISO LD-SBR inputs is separate evidence,
not whole-profile waveform/phase conformance. Six of those failed onQ31 before
this change. External ISO media remain outside this authored fixture directory.
