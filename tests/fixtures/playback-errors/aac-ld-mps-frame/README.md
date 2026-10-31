# LD-MPS spatial-frame syntax fixtures

336 own configurations / 1008 synthetic frames, covering every tree and LD
frequency resolution, implicit/explicit slots, OTT/TTT/shared ICC, smoothing,
STP/GES, downmix gains and arbitrary-tree extension regions. These are saved
**component payloads**, not accepted spatial movies or PCM rendering evidence.
The native spatial renderer refusal remains enabled. No private media, audio,
frames or codec parameter sets are used.

Generate explicitly, offline:

```sh
python3 scripts/generate_aac_ld_mps_frame_fixtures.py
```

Ordinary tests only read saved data:

```sh
cargo test --offline --no-default-features --features player --test aac_ld_mps_frame
```

`reshape-codewords.json` is a separate numeric input with frozen official ISO
provenance. Its explicit extraction script needs the frozen external ISO insert;
ordinary generation/tests require neither that archive, a compiler, FFmpeg nor
network access. The optional reference validator also needs that archive and a
C compiler. Its original mismatches and excluded undefined paths are retained;
a separate control qualifies three specific reference-source corrections.

`errors.bin` contains five saved failures with exact reasons in the manifest:
GES run overflow, interpolation with active/zero-band residual configuration,
an unused PCM codeword in grouped zero-width coding and empty unit-stride
Huffman coding. Grouped zero-width acceptance now has its own movie/seek corpus
in `aac-ld-mps-empty-groups`; the former grouped refusal was replaced by a
specific malformed codeword. Empty unit-stride PCM and zero-width default/keep
regions retain positive frame acceptance tests. Residual
AAC payloads remain opaque and are not audio acceptance evidence.
