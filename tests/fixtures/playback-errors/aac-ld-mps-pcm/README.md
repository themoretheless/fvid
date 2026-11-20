# Ordinary LD-MPS PCM-chain controls

224 controlled sequences/672 frames cover eight trees, seven resolutions,
32/64-bin analysis/synthesis, both decorrelator coefficient modes and all eight
clip, five surround and five LFE gain values. Saved interleaved f32 outputs
contain 1677312 samples. Inputs are own synthetic decoded PCM and residual QMF;
no private media, samples or codec parameter sets are copied.

The assembly corpus supplies spatial packets and qualified matrix endpoints.
Only fixed-gain configuration fields are changed; ordinary matrix construction
is independent of these gains. Endpoints retain the documented 212 residual
basis. Downmix sample `(11*f+7*i+3*c)%29-14` has denominator1024, with frame2
zero. Residual QMF uses the QMF-chain corpus's dyadic stimulus. Clip protection
scales downmix before analysis, never decoded residuals. Synthesis outputs are
reordered into LF/RF/C/LFE/LS/RS[/AL/AR], then fixed output gains are applied.
Additional channels remain AL/AR; no speaker mask is guessed.

Explicit research generation requires the frozen ISO archive and C compiler:

```sh
python3 scripts/generate_aac_ld_mps_pcm_reference.py /path/to/saoc-reference.zip
```

The original matrix-apply/decorrelate/merge routines operate on controlled
endpoints. Original analysis/synthesis run in two separate controls: archived
PI in `original.bin`, and a separate translation unit overriding only PI with
a double-precision constant in `expected.bin`. Archived sources are unchanged.
Their differences are retained, not hidden by altering the original baseline.
The unselected HRTF branch aborts. Padded W layouts qualify this interface,
not the original frontend's automatic channel-count selection.

Ordinary `aac_ld_mps_pcm` tests load saved data without generation, FFmpeg,
network, a compiler or reference software. Both controls are compared; complete
checkpoint/reset replay includes analysis, parser/matrix/decorrelators and
synthesis. An existing synthetic active-STP packet verifies a specific late
refusal with complete state/cursor rollback. That is not intended temporal-tool
acceptance. PCM signals are unclamped normalized floats with filter latency;
this does not qualify AAC/LD-SBR transport, timing/preroll, decoded residual
codecs, active temporal shaping, arbitrary-downmix/HRTF, 128-bin DSP or actual
movie playback/seek. Native spatial renderer refusal stays enabled.
