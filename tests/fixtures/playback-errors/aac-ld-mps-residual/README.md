# Residual extension syntax and spectral reconstruction

416 owned payloads cover all eight tree layouts, thirteen AAC rates, four window
sequences, both shapes, one-to-four residual frames, OTT ICC differences, TTT
streams and the additional short ICS for18/24/30-slot schedules. Factual AAC
Huffman/band geometry is shared with the owned fixture helpers. Expected
coefficients use the analytical inverse quantization of integers -1,0,1 at
global_gain140: -1024,0,1024. These are spectra, not PCM or a QMF oracle.

`payloads.bin` includes a following0xa5 sentinel after every valid payload.
`manifest.json` records exact bit lengths, sparse expected spectra and a specific
extra-short window mismatch. Truncation tests preserve the outer reader even
after earlier boxes/blocks were decoded. The capped-unary ICC unit test checks
all256 leading bytes at every bit offset and every truncated prefix.

`native-residual-refusal.mp4` is a twelve-frame own H.264/AAC-ELD movie with
nonzero residual AAC. Its ASC, all spatial frames and all residual payloads are
separately parsed successfully. Native playback must still refuse with the
specific missing residual-reconstruction message until MDCT-to-QMF conversion,
ICC correction application, source timing and playback/navigation are wired and
qualified. That is a passing refusal/reproduction test, not playback acceptance.
`native-packets.bin`, `native.asc` and `native.ssc` preserve the same own stream
for the later integration acceptance test. No private media or parameter sets
are included.

Explicit generation from the repository root:

```sh
python3 scripts/generate_aac_ld_mps_residual_fixtures.py
```

Generation is byte-identical with child-process and socket APIs denied. Ordinary
tests read frozen fixtures only and do not need FFmpeg, a compiler or network.
