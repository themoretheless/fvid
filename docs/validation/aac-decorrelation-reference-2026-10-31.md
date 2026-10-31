# Native LD-MPS renderer selection and reference checks

Build the optional probe without FFmpeg:

```sh
cargo build --offline --no-default-features --features player --example aac_spatial_reference
```

Run the built example explicitly:

```sh
target/debug/examples/aac_spatial_reference real INPUT.mp4 OUTPUT.f32le
python3 scripts/compare_aac_reference_pcm.py REFERENCE.wav OUTPUT.f32le
```

Use `complex` for the existing playback default. The probe requires one AAC
track with no edit list and a constant sample description. Its output is raw
decoder PCM; comparison does not silently apply priming,channel reordering or
a delay search. `--lag N` explicitly compares actual frame `i+N` to reference
frame `i`. This parameter is diagnostic unless independently justified by the
applicable normative procedure. JSON metrics do not assign conformance.

The [official ISO reference archive](https://standards.iso.org/iso-iec/23003/-2/ed-2/en/SAOC_IS7_ReferenceSoftware__20160603.zip)
has SHA-256
`b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`.
Its `mp4spatialdec/src/spatial_frontend.c` initializes decType to0 and describes
the0/1 implementation choices. Native production previously selected Complex
unconditionally,although the owned spatial modules implement both choices.
The new construction options expose this choice without changing defaults.

Reference MP4 and WAV members come from the
[official audio conformance inserts](https://standards.iso.org/iso-iec/14496/-26/ed-2/en/),
fetched by `scripts/fetch_iso_ld_mps_conformance.py OUTPUT_DIRECTORY` with the
pinned ISO inventory,member MD5 checks and ZIP CRCs. Media remains outside the
repository. Only hashes,run results and scalar measurements are recorded.

For LD tree5151,Real coefficients reduce normalized full-stream RMS difference
to9.233e-6,max5.795e-5. This explains the bulk of the earlier mismatch,not an
accuracy-category verdict. ELD tree5151 still differs after a diagnostic lag256;
single-rate LD-QMF32 remains unresolved too. Do not correct production timing
or phase from those fitted observations alone. The next step needs independent
core/filterbank timing evidence and an authored reproducer for each confirmed
decoder defect.

Regression tests use the existing `aac-eld-mps-core` and `aac-alignment` own
synthetic corpora. They cover both modes,all8 trees,clock negotiation,reset,
same-mode checkpoint replay and transactional rejection across modes. The root
native frontend and fvid-media decoder must produce identical samples.
