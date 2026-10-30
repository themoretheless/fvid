# Residual MDCT-to-QMF component and overlap

24 own programmes cover update sizes15/16/18/24/30/32, single and double residual
frames where the spatial frame fits32 slots, and dense/silent/edge-impulse input.
Twelve frames follow long/start/short/stop transitions. The seventh has zero new
spectra to distinguish overlap carry from a stateless transform. Update15 covers
the special last-short interleave;18/24/30 use additional short spectra.

The frozen C outputs execute the unchanged ISO conversion functions and numeric
windows extracted from the pinned official archive. The harness selects the
full-complex64-bin default phase (`twiddleExtern=NULL`) and shifts overlap buffers
as the original spatial processor does. Output is normalized by32768. The
reference uses f32 arithmetic/trigonometric arguments; owned Rust uses f64 math,
so tests compare with absolute tolerance5e-7. The observed maximum is3.081e-7
for raw input amplitudes up to1024; this is not an all-amplitude error bound.

The numeric frequency-window facts are transcribed from the reference's f32
realization by explicit generation. No external implementation is linked into
FVid or required by tests. Expected/reference equivalence here does not establish
native LD-QMF phase/bank selection, ICC correction application, PCM playback,
seek, or arbitrary residual-tool/profile support.

Explicit generation (never run by ordinary tests):

```sh
python3 scripts/generate_aac_mps_mdct_qmf_reference.py /path/to/SAOC_IS7_ReferenceSoftware__20160603.zip
```

This regenerates window facts, own stimuli and independent C outputs. Ordinary
tests need the saved files and owned Rust only, without FFmpeg/compiler/network.
No private source media, audio or codec parameters are included.
