# LD-MPS physical preparation and matrix-time component oracles

These are **component controls**, not accepted spatial movies or PCM/playback
results. All inputs are own synthetic metadata, decimal grids or rational
matrix coefficients. No private media, audio or codec parameter sets are used.

- 161 qualified base configurations / 483 frames and 56 active-arbitrary-tree
  quant-mode-zero overrides / 168 frames. Another 175 base configurations are
  explicit unqualified-constructor refusals, not positive physical acceptance.
- 1116 high-precision Decimal CLD energy splits, including -1000 dB parents.
- 28 four-frame exact-Fraction matrix sequences; all seven matrix planes occur.
- 1400 fixed-gain/sample-rate controls, including both QMF rate boundaries.
- Numeric grid, smoothing-time and native QMF map facts have frozen ISO provenance.

Generate the saved physical/matrix oracle explicitly, offline:

```sh
python3 scripts/generate_aac_ld_mps_prepare_fixtures.py
```

It reads saved frame/configuration fixtures and `facts.json`. Ordinary tests
only load saved data; they never regenerate it or require FFmpeg/network/C:

```sh
cargo test --offline --no-default-features --features player --test aac_ld_mps_prepare
```

Numeric extraction and optional reference validators require the external
frozen ISO insert. The independent decimal/fraction generator requires no
reference implementation. Coupled frame tests re-use the own encoded frame
corpus; the quant-zero variants override the typed configuration, and the
explicit C validator independently serializes that override into its SSC bits.

No actual matrix construction, decorrelation/upmix, residual AAC decoding,
STP/GES audio processing or spatial PCM routing is claimed. Checkpoint/reset
checks here are component replay evidence, not player seek/rewind acceptance.
