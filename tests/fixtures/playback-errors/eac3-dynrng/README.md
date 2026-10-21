# All normal dynrng codes

256 owned conventional E-AC-3 stereo cases exercise every 8-bit dynrng code,
with the code sent in block 0 and reused in the other five blocks. Each has
three raw frames and a short synthetic VP9/Matroska video. The small control
signal needs no clipping for the entire gain range.

Regenerate separately: `python3 scripts/generate_eac3_dynrng.py`.
No external encoder, subprocess, network or private media is required.

ETSI TS 102 366 section 6.7.2.2/Table 6.29 treats the upper three bits as a
signed exponent. The independent gain oracle is the literal eight-element
power table [1,2,4,8,1/16,1/8,1/4,1/2] times (32+mantissa)/32. Before the fix,
code 0x01 produced sample -2.6481382e-7 instead of -6.779234e-5 because the
shared AC-3/E-AC-3 function tested bit 5 rather than bit 7 for the exponent sign.
`/tmp/fvid-presentation-dynrng-before.log` records the failing real PCM test.
The regression now checks exact sample gain and block reuse for all codes in
raw and Matroska playback. Silence/clipping tolerances were not loosened.

Optional FFmpeg 9.0.2 reference passes 256/256 at unchanged 1e-5, maximum
2.384185791015625e-7. `/tmp/fvid-presentation-dynrng-reference.json`.
Ordinary tests never run FFmpeg. This covers normal dynrng, not heavy compr/RF
mode or all other AC-3/E-AC-3 metadata combinations.
