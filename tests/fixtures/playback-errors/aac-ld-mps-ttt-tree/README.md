# Full-output TTT-tree matrix controls

6912 synthetic parameter sets cover trees 2/3/4, all six TTT modes, both TTT
residual regions, both inversion settings, both LFE cutoff regions and every
non-LFE OTT residual mask. Four deterministic trials vary CLD/CPC/ICC values,
including negative coherence. No private media or parameter sets are copied.

`expected.bin` holds 1164288 scalar coefficients from unmodified original ISO
C routines: complex M1 (real/imag pairs), real M2-dry, then real M2-wet. M1
has three input columns: two downmix channels and decoded TTT residual basis.
Tree2 uses six W columns, including the dormant TTT column in nonzero modes;
trees3/4 use eight. Source/archive/driver hashes are in the reference report.

Explicit research regeneration needs the frozen external archive and compiler:

```sh
python3 scripts/generate_aac_ld_mps_ttt_tree_fixtures.py /path/to/saoc-reference.zip
```

Ordinary `aac_ld_mps_ttt_tree` tests read saved outputs without generation,
FFmpeg, network, reference software or a C compiler. The source graph composes
independently qualified OTT/TTT/inversion primitives rather than copying the
reference's expanded per-tree arithmetic. An analytic complex-QMF pipeline
checks the M1/M2 interface and native output channel order for all three trees.

Tree2 mode0 outside TTT residual bands retains the same TTT auxiliary value
in dry and wet coefficient planes. Its dry sample is zero in that region;
the dormant coefficient must not be confused with an active residual signal.

This is matrix component qualification, not spatial movie/PCM/playback/seek
acceptance. Frame-level low/high-mode assembly, W routing, residual decoding,
fixed output gains, temporal shaping, arbitrary downmix/HRTF/3D-stereo tools,
128-bin DSP and full LD-SBR spatial PCM integration remain open. The native
spatial renderer refusal remains enabled. Singular nonresidual zero-coherence
prediction and LFE residual routing are explicitly refused, not accepted media.
