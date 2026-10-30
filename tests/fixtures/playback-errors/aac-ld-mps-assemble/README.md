# LD-MPS frame-to-matrix assembly controls

336 owned synthetic configurations and 1008 spatial frames cover all eight
trees, seven parameter resolutions, shared/per-box ICC, all six TTT modes,
low/high boundaries at zero/middle/end, inversion, LFE cutoff, residual bands,
32/64/128-bin matrix geometry, smoothing, keep history and a held endpoint.
No private media, samples or codec parameter sets are copied.

The saved oracle contains 3339000 f32 endpoint coefficients. Raw ordinary
matrices come from unmodified original ISO functions; temporal smoothing uses
80-digit Decimal arithmetic. Every interpolated slot is checked from saved
endpoints. The known original 212 residual-column discrepancy is explicitly
separated: its auxiliary dry fields use independently qualified signed-unit
residual injection (or zero outside the residual region). The raw original
output hash and the 6300 separated fields are recorded in the reference report.
The reference source is never patched or put into production.

Explicit generation, separate from ordinary tests:

```sh
python3 scripts/generate_aac_ld_mps_assemble_fixtures.py
python3 scripts/generate_aac_ld_mps_assemble_reference.py /path/to/saoc-reference.zip
```

The syntax generator is offline and needs no external software. The explicit
reference command needs the frozen ISO archive and a C compiler. Ordinary
`aac_ld_mps_assemble` tests load saved data without FFmpeg, network, regeneration,
a compiler or reference software.

Checkpoint/reset tests retain entropy, smoothing and all three matrix histories.
A separate synthetic packet parses successfully in physical preparation, then
fails specifically at singular predictive coherence; the matrix assembler must
roll back all histories and cursor. This is a passing refusal/transaction test,
not acceptance of the intended spatial audio. These are component packets,
not accepted spatial movies, decoded residual audio, PCM output or actual player
playback/seek evidence. Full rendering tools and native spatial playback remain
unqualified; the native renderer refusal stays enabled.
