# Native residual AAC / LD-SBR playback

397 own twelve-frame H.264/AAC-ELD movies exercise the complete owned decoder:
all eight spatial trees, 480/512 core frames, 15/16 shared complex64-bin QMF
slots, 4/9/23 parameter bands, OTT and TTT residuals, partial/full cutoffs,
shared/per-box ICC, long/start/short/stop windows and both window shapes.
A dedicated movie accepts nonzero fill bits and extra fill bytes inside the
bounded residual extension, matching ISO SpatialDecParseExtensionFrame.
Plain/STP/GES, delayed/fragmented delivery, repeated headers, cutoff changes,
and disabling/re-enabling residual boxes are included. Source core/SBR prefixes
come only from the existing own synthetic corpus. No private data is used.

The full PCM comparison uses frozen independently qualified core/SBR QMF,
separately parsed residual spectra, per-box converter histories, independently
encoded already-corrected fine ICC controls, and explicit matrix/shaping/LD
synthesis composition. This qualifies native wiring and timing; it is not an
independent whole-decoder oracle: the cached ISO LD_MPS hybrid-analysis branch
omits residual preparation. Full LD residual conformance remains open.
The golden QMF may store only 32 active bands:
the upper half of its 64-bin bank is zero, never the next stored slot.

Playback tests compare live drain, reset/rewind, forward/backward/EOF seek,
repeated ranges and public/owned export with identical full PCM. Four corrupt
movies prove the specific late residual/ICC/spatial failure and preserve core,
SBR, residual overlap, source queues and pending-frame state on retry. Two
movies preserve explicit refusal for unqualified32-bin and update8 domains;
these refusal tests are not acceptance. Separate controls require both residual
signal and ICC correction to measurably alter PCM.

Generate explicitly from the repository root:

```sh
python3 scripts/generate_aac_ld_mps_residual_native_fixtures.py
```

Generation uses no encoder/compiler/network and is tested with socket and
process APIs denied. Ordinary tests read saved bytes only. Phase/source evidence
and gate counts live in `docs/validation/aac-ld-mps-residual-native-*-2026-10-31.json`.
