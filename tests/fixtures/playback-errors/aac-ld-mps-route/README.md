# LD-MPS W routing component controls

504 synthetic cases cover all eight trees, all seven parameter resolutions,
32/64/128 QMF bins and three residual-cutoff/mode patterns. Two-slot inputs
have deterministic dyadic complex values; the independent 80-digit Decimal
oracle saves 1881600 scalar dry/wet output values. No private media or codec
parameter sets are copied. These are component controls, not accepted movies,
residual audio decoding, PCM output or player playback/seek evidence.

Explicit offline generation:

```sh
python3 scripts/generate_aac_ld_mps_route_fixtures.py
```

Ordinary `aac_ld_mps_route` tests read saved data without generation, FFmpeg,
network, a C compiler or reference software. A separate optional C harness
compares original unmodified SpatialDecMergeResDecor and the original residual
index lookup with this oracle. Per-bin harness calls allow adversarial mode
patterns; this qualifies routing, not physical two-region stream configuration.

Direct dry channels copy M1 virtual signals and direct wet channels are zero.
OTT auxiliaries select decoded residual below their cutoff, otherwise wet
output from the decorrelator. The mode0 TTT W mixture combines original wet
outputs 3/4/5 before masking OTT residual channels. TTT residual itself enters
M1, not W column5; changing its cutoff/samples cannot replace/mute that W path.
The reference maps W column5's residual lookup to its unused LFE box, whose
active residual routing remains explicitly unqualified. Supplied TTT samples
are validated but not consumed by this component.

Owned/Decimal use exact 1/sqrt(2). Original C uses 0.707107f and float samples;
its small rounding difference is preserved in the reference report. Arithmetic
at 128 bins does not qualify the missing 128-bin QMF/decorrelation DSP. Full
frame assembly, decoded residual inputs, spatial PCM, temporal shaping and
actual synthetic movie playback/seek integration remain open.
