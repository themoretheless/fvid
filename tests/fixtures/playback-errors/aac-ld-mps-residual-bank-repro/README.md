# Residual32/128 bank admission reproducers — playback is still unsupported

Explicit offline generation:
`python3 scripts/generate_aac_ld_mps_residual_bank_repro_fixtures.py`.
44 own AAC-ELD/MPS MP4 reproducers each have a paired ordinary control.
They cover24kHz/32-band and96kHz/128-band output,480/512-sample cores,all8
trees and both TTT variants. High-rate spatial windows cover four original
core packets; both domains use15/16-slot residual updates.12 own bare ELD
movies additionally compare their complete PCM against the scalar core oracle.

The passing test is a precise refusal/reproduction test,not residual playback
acceptance. All1872 compressed residual spectral blocks parse separately;
ordinary controls decode complete nonzero PCM at the intended format/count.
Residual movies refuse specifically with the native full-complex64-bin domain
guard and publish no PCM. This isolates bank admission from unrelated container,
core AAC or residual syntax errors. It does not prove whole-stream conformance.

The intended complete-PCM acceptance test is explicitly ignored until an
independently qualified bank/phase mapping is implemented. Running it explicitly
with `--ignored` still fails at the same bank refusal. Enable it with the real
fix and replace the refusal expectations; do not count this corpus as accepted
residual playback or closed codec coverage.

The pinned ISO residual MDCT->QMF source fixes its projection to64 bands.
The LD frontend source does not itself supply a complete residual32/128 oracle.
A guessed crop,zero-pad,phase rotation or time-domain substitute is not qualified
by these data. Normative mapping and independent PCM/QMF evidence remain open.

All media,spectral syntax and spatial parameters are authored locally. No private
videos,frames,audio or codec parameter sets. Own generation and ordinary tests
require no FFmpeg,network or external process. C research sources are not part
of the runtime or ordinary test execution.

Ordinary check:
`cargo test --offline --no-default-features --features player --test aac_ld_mps_residual_bank_repro`.
Explicit pending acceptance (currently expected exit101):
`cargo test --offline --no-default-features --features player --test aac_ld_mps_residual_bank_repro residual32_and128_movies_decode_complete_pcm -- --ignored`.
