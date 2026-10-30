# LD-MPS OTT and complex-QMF decorrelation controls

Own synthetic component inputs only: 496 OTT covariance/residual matrices and
120 QMF sequences (32/64 bins, ten seeds, three configurations, real/complex
filter modes). The 96-slot sequences include a 32-slot burst and a silent tail.
They cover 552960 complex output samples. Inputs and expected outputs are saved
as little-endian f64 pairs in `signals.bin`, with offsets in `manifest.json`.

These are **component controls**, not accepted spatial movies, a full spatial
renderer, residual audio decoding, PCM output or playback/seek acceptance. No
private videos, frames, audio or codec parameter sets are copied.

Generate explicitly, offline:

```sh
python3 scripts/generate_aac_ld_mps_upmix_fixtures.py
```

Generation uses independent 80-digit Decimal covariance, convolution, analytic
phase-series and energy equations. Ordinary tests load saved data only:

```sh
cargo test --offline --no-default-features --features player --test aac_ld_mps_upmix
```

`decor-facts.json` contains frozen ISO numeric reflection/phase facts, including
C's six implicit zero rows for each used LD reverb band. Numeric extraction and
optional C comparison are separate explicit research commands needing the
external frozen ISO insert. No ordinary test requires a compiler, that archive,
FFmpeg, generation or network access.

QMF amplitudes are normalized (full scale one). The research C probe scales
inputs by 32768 and outputs back, preserving the original ducking threshold's
physical units. 128-bin decorrelation remains explicitly unqualified.
