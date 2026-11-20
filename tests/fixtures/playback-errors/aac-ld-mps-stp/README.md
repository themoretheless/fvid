# Owned LD-MPS STP playback regressions

400 twelve-frame synthetic H.264/AAC MP4s cover active/warm-disabled/alternating
STP and same-domain configuration transitions, plain ELD and coupled LD-SBR,
eight trees, 32/64 QMF bands and 23/9/4 parameter grids. The common ASC/first
header explicitly agrees on STP mode1. Configuration transitions are robustness
controls, not strict baseline MPEG-4 transport conformance claims. Two error
movies introduce late trailing bytes; no private source media or parameters are
included. The baseline native acceptance failed specifically on active STP PCM.

`manifest.json` and `packets.bin` describe the own streams. `component.json`,
`component-input.f32le` and `component-expected.f32le` qualify the DSP separately
against an adapted ISO reference. All 16 programmes have 104 slots and cross the
energy refresh boundaries at slots33,65,97. They exercise disabled flags, high-only
and low-only signal, silence, cancellation and ordinary broadband complex data.

Explicit generation from the repository root:

```sh
python3 scripts/generate_aac_ld_mps_stp_fixtures.py
python3 scripts/generate_aac_ld_mps_stp_reference.py /path/to/SAOC_IS7_ReferenceSoftware__20160603.zip
```

The first generator uses only owned Python fixture logic, with no subprocess or
network requirement. The second compiles the pinned external reference only at
generation time. Its three UB-removing adaptations are recorded in the source
and validation report: skip already-zero low wet bins in the final mix, initialize
absent tree7 channel indices, and process only populated dry groups for trees5/6.
The defined active-bin frequency/gain formula is unchanged. This is explicitly
an adapted component oracle, not an independent full decoder or normative
Annex B conformance claim. Ordinary tests use saved bytes only.
