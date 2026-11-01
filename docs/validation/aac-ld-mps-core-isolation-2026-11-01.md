# AAC-LD5151 core isolation, 2026-11-01

The original LD5151 core was isolated for all 2820 AUs without decoding or
re-encoding its core bits. The explicit research transform disables the ASC
spatial extension, retains the parsed core bit prefix of every AU, removes
spatial ancillary suffixes and adds zero byte alignment. Both original record
and transformed record SHA256 values are pinned by the script. Syntax input
must describe the expected mono 512-sample, non-PNS, non-LTP core.

These transformed records are research inputs, not unchanged ISO originals or
repository acceptance fixtures. No original media or codec parameters are
copied into fixtures. Ordinary tests do not invoke the transformation, foreign
decoders or network access.

The owned ordinary decoder exports 1443840 mono float samples. Recomposition
through the owned spatial decoder uses the original MP4 ancillary bits and
original per-sample timestamps, durations and timescale. All 8663040 output
scalars in each mode are byte-identical to the full owned decode:

| Mode | Output SHA256 |
| --- | --- |
| Real | c057cf1ab2a87e771a4cac34c48c0e03c21e94f377c2eca3e7580f6614591ca5 |
| Complex | 9f83aab620ee5fa2d5b3d8181dda2fb7ee13cc0061000dfbfe71a703647195aa |

This establishes stage-boundary integrity. It is not an independent spatial
waveform oracle and does not close the original Real-mode numerical residual.
No delay search, trimming or gain fitting is applied.

The unchanged external FDK probe also decodes the isolated core as mono 48kHz,
512 samples/AU, outputDelay=0. Against its PCM16 output the owned float core has
RMS 8.781423281721406e-6 and maximum absolute difference 2.8848648071289062e-5.
FDK fixed-point arithmetic and PCM16 quantization prevent using this comparison
alone to locate the much smaller whole-chain numerical residual. It is an
external control, not a normative acceptance threshold.

No new production defect is established here. Decoder arithmetic is unchanged.
Both diagnostic examples build offline with the player feature; complete
isolation/export/recomposition was rerun successfully after adding admission
guards. The earlier 693-test gate remains associated with production commit
3468160247, not this tooling addition.

## Explicit reproduction

Use the pinned originals fetched by `fetch_iso_ld_mps_conformance.py`. Keep all
research outputs outside the repository fixtures. With `BIN` pointing to the
built examples and `WORK` to an external research directory:

```sh
"$BIN/aac_spatial_reference" packets "$ORIGINAL" "$WORK/original.records"
"$BIN/aac_ld_core_inspect" "$WORK/original.records" > "$WORK/syntax.jsonl"
python3 scripts/isolate_aac_ld_mps_core.py "$WORK/original.records" \
  "$WORK/syntax.jsonl" "$WORK/core.records" --report "$WORK/isolation.json"
"$BIN/aac_ld_core_inspect" "$WORK/core.records" "$WORK/core.f32le"
"$BIN/aac_ld_recompose_reference" real "$ORIGINAL" \
  "$WORK/core.f32le" "$WORK/recomposed.f32le"
"$BIN/aac_spatial_reference" real "$ORIGINAL" "$WORK/full.f32le"
cmp "$WORK/recomposed.f32le" "$WORK/full.f32le"
```

Repeat the last three commands with `complex`. The JSON report records current
probe hashes and the separate FDK16 numerical control.
