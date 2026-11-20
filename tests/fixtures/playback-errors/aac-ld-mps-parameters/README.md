# Stateful LD-MPS parameter component fixtures

552 own synthetic sequences / 4416 ecData blocks; no private media or codec
parameters. These are **component fixtures, not accepted spatial videos**.
Spatial rendering is still explicitly refused. Do not count these as movie,
playback, seek or rewind acceptance, or a closed codec gap.

Generate explicitly, offline:

```sh
python3 scripts/generate_aac_ld_mps_parameter_fixtures.py
```

Ordinary tests read the saved packet blob and manifest only:

```sh
cargo test --offline --no-default-features --features player --test aac_ld_mps_parameters
cargo test -p fvid-media --offline --no-default-features --features player --lib saved_sequences_preserve_entropy_history
```

The manifest includes expected fine output rows and quant-domain parse history.
Sequences cover every 1..23-band geometry, both quantizers, default/keep/
interpolate/coded modes, paired PCM, temporal Huffman, changing strides and
coarse/fine history transitions. Parse and output maps differ for some strides.
The separate reference validator requires an external frozen official ISO
archive and a C compiler; it is never invoked by ordinary tests.
