# Ordinary LD-MPS QMF-chain controls

224 controlled sequences (672 frames) cover eight trees, seven parameter
resolutions, 32/64 QMF bins and both real/complex decorrelator coefficient modes.
The source spatial packets and qualified matrix endpoints are from the saved
assembly corpus, variants 0/5. Inputs are own synthetic dyadic complex values;
the third frame is zero to expose persistent filter tails. No private media,
samples or codec parameter sets are copied.

Saved f32 dry/wet outputs contain 6709248 scalar values from unmodified original
ISO M1 application, decorrelation, W merge and M2 application functions. The
harness supplies the previously qualified matrix endpoints, including their
explicitly documented 212 residual-column basis. This is a controlled component
chain, not qualification of the original frontend's automatic channel counts,
bitstream parser, complete PCM decoder or spatial movie playback/seek.

The W geometry follows the qualified matrix interface, including tree2's dormant
sixth column. Seed schedules come from the full-complex frontend branch:
sequential indices except tree2 TTT seed3 and stereo seed2. Both coefficient
modes still use complex QMF data; they are implementation choices, not stream
flags or partially-complex alias-lock DSP.

Explicit research generation (frozen external ISO archive and C compiler):

```sh
python3 scripts/generate_aac_ld_mps_qmf_reference.py /path/to/saoc-reference.zip
```

Ordinary `aac_ld_mps_qmf` tests read saved data, with no FFmpeg, network,
regeneration, compiler or reference software. Samples use frame/t/channel/bin
indices f/t/c/q. Downmix real numerator is `(11*f+7*t+3*c+5*q)%29-14`, imaginary
is `(7*f+5*t+11*c+3*q)%23-11`, denominator256. Residuals use the same numerators
and denominator512. Frame2 is zero. Original C receives samples multiplied by
32768 and its output is divided back to preserve its ducking energy units.

An abort guard covers the unselected HRTF branch, which is unsupported here;
it does not implement HRTF. Original apply functions run on a 64MiB research
thread stack. No C code or such stack requirement enters production.

Tests compare all outputs, checkpoint/reset replay, nonzero silent-input tails,
missing TTT input, a missing OTT residual discovered only after filters run, and
nonfinite rejection. A failed frame retains every parser/matrix/filter history
and reader position. 128-bin DSP remains explicitly refused. Gains, temporal
shaping, decoded residual codecs, PCM synthesis and actual movie playback/seek
remain open; native spatial playback refusal stays enabled.
