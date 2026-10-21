# E-AC-3 adaptive hybrid transform acceptance

Generate separately with `python3 scripts/generate_eac3_aht.py`. Owned bit writers
construct every stream without private input, generated source media, external
processes, FFmpeg or network access. Tests read checked fixtures only.

Eight cases cross native/converted stream types with neither, left, right, or
both planes selecting AHT. Six cases use active AHT; two check that `ahte=1`
may legitimately decline the transform on all planes. Each has three six-block,
48 kHz stereo syncframes and an owned VP9/Matroska video. Their high-efficiency
allocation saturates at scalar hebap 19. AHT spectra carry nonzero signed DC
coefficients and zero AC coefficients, which reconstruct exactly the same sound
as the corresponding conventional scalar control. This is active reconstruction,
not a passing refusal fixture.

`tests/eac3_aht.rs` checks conditional plane flags, geometry, exact scalar PCM,
raw and Matroska reader output, duration and rewind. With the prior blanket AHT
refusal restored, the final test fails at active `type0-mask1` header recognition.
Separate codec units cover every one of the 956 normative VQ codewords, inverse
DCT normalization, scalar/GAQ small/large escape paths, grouped gains and malformed
allocation/truncation. Those component tests do not establish end-to-end coverage
for VQ, nonzero AHT AC spectra, coupling, LFE or every GAQ configuration.

Normative source: ETSI TS 102 366 V1.3.1 Annex E, E.2.4 and Tables E.3.1..7:
https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.03.01_60/ts_102366v010301p.pdf
Optional independent PCM checks are confined to
`benches/check_eac3_aht_reference.py`. That tool accounts explicitly for
`audio_probe`'s existing 256-sample lead trimming. It does not generate fixtures
or run in ordinary tests. `benches/check_eac3_aht_tables.py` is a separate strict
reference-data audit; a mismatching reference file produces a nonzero result.
