# Silent-core LD-SBR harmonic acceptance

24 independently authored eight-AU programmes/48 MP4+Matroska movies cover
480/512 core frames, single/dual-rate output, CRC, fine envelope levels24/36/48
and noise7/30. The AAC core has max_sfb0; all nonzero PCM comes from LD-SBR.
The high-frequency grid/header comes from existing own generators, not ISO or
private codec parameter sets. No source media, frames or audio is copied.

Frozen gold uses constant zero-current-energy gain/noise/sinusoid formulas and
independent direct CLDFB column superposition. Ordinary tests qualify complete
PCM,exact8x amplitude steps,transport equivalence and worker playback/seek/rewind
with phase carried across15/16-slot frames. Gold is not native or FDK output.

The native decoder already accepted these tools. These fixtures isolate a
reference-configuration defect: the old external FDK default selected Low Power
for ordinary ELD while FVid uses complex QMF. Optional explicit complex/Low Power/
auto comparison qualifies that cause separately; this is not a newly implemented
native mode or full-codec conformance claim. Native acceptance tests pass before
and after the reference-probe correction. Passing CLI refusal is not PCM acceptance.

Regenerate separately:
`python3 scripts/generate_aac_ld_sbr_silent_harmonics_fixtures.py`.
Normal tests consume frozen assets and need no generator,network,FFmpeg or FDK.
