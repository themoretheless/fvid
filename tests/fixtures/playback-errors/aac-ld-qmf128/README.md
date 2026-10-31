#128-bin LD-QMF component controls

`generate_aac_ld_qmf128_fixtures.py` creates own absolute-time convolution and
complex-exponential analysis/synthesis traces using the already sourced640
LD-window facts. It needs no network or child processes.

`validate_aac_ld_qmf128_reference.py /path/to/official.zip` is a separate,
explicit research step requiring a C compiler. It reads the pinned official ISO
archive,compiles the original polyphase source unchanged,and freezes its output.
The precision control changes only the translation unit's PI macro. Ordinary
Rust tests never compile/download/run this reference.

The source's non64 branch averages the current and next coefficient at integer
floor(k*64/128); its1280-sized array has640 explicit LD values and implicit
zeros. The final neighbour is therefore C[640]=0. Full128-bin matrices use
analysis gain64/128 and synthesis gain1/64. No64-bin crop substitutes this bank.

Three own PCM stimuli and three complex-only synthesis inputs exercise impulse,
dense/tone spectra,low-real and bin127 imaginary activity,chunk/reset/checkpoint
behavior and late nonfinite rollback. Scalar f64 error is below1e-11. Original
C float PI causes a separately measured phase discrepancy (up to9.711e-5 in
analysis); the float-stage model matches that original output exactly,and the
precise-PI control isolates its float accumulation error. Tolerances for the
original/precise controls are distinct from the tight scalar check.

This is a full-complex default-phase bank component qualification,not complete
spatial bitstream or codec conformance. Video acceptance is in `aac-eld-mps128`.
