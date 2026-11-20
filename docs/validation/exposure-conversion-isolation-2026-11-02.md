# Exposure kernel versus external RGB conversion

The previously ignored integer/float compatibility acceptance remains open. This diagnostic isolates its conversion component; it does not implement that converter, qualify full compatibility or close a codec gap.

The explicit benchmark uses FFmpeg 9.0.2 only from `crates/fvid-media/benches/diagnose_exposure_conversion.py`. It reads the existing authored three-frame 8x8 RGBA8/RGBA16 grids and never writes fixtures. Ordinary tests invoke neither this script nor FFmpeg.

For each precision, the raw no-filter control is byte-identical to input. The RGB result of `exposure=0` is byte-identical to both explicit packed RGB -> planar float RGB -> packed RGB controls, without the exposure kernel. At eight bits the identity preserves all RGB codes. At sixteen bits, 548 of 576 RGB components change, maximum absolute delta 65. Both identity outputs match the already committed external reference RGB files exactly.

A second own arithmetic three-frame control holds red fixed while varying green and blue. The eight-bit conversion preserves red127. The sixteen-bit float round-trip produces red32766..32777 from constant red32767. This is an observed dependency of that external conversion pipeline, not proof of a defect in FVid's exposure kernel or a claim about every converter/backend.

For half exposure, an independent exact half operation on the externally converted planar float RGB (alpha untouched), followed by the external output converter, reproduces FFmpeg's direct `exposure=-1` RGB output at both depths. This identifies a concrete qualification target: input/output conversion must be checked independently of the float exposure formula. The existing saved-float acceptance already checks eight exposure/black-point parameter sets.

The new native acceptance checks that identity exposure retains every authored packed RGB and alpha code in the existing three-frame grids at both depths. The old mismatch reproductions and ignored full-conversion acceptance are retained. No runtime kernel was changed. Exact foreign float-round-trip parity and native identity preservation are distinct contracts; this diagnostic does not silently replace one with the other.

For future codec comparisons, capture the negotiated conversion graph and qualify output pixel conversion before attributing RGB mismatches to a decoder. This is a correctness diagnostic, not a throughput or 60-fps benchmark.

Reproduce explicitly with an installed reference binary:

```sh
python3 crates/fvid-media/benches/diagnose_exposure_conversion.py --output /tmp/exposure-conversion.json
```

The adjacent JSON pins the external executable, version/build options, platform, own inputs, intermediate/output hashes, diagnostic source and normalized conversion graph logs. No external algorithm source or media was copied into production.

Focused native validation completed: eight passed, zero failed, one ignored (the existing complete conversion compatibility acceptance). The exact command, source and log hash are recorded in the JSON.
