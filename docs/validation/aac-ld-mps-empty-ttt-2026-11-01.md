# Energy quantization with zero-width TTT regions

The constructor used both signalled TTT modes to qualify tree525 energy
quantization, including a mode belonging to an empty region. The authored
`bands23-quant1-empty-high2` first reproduced the exact energy-quantization
refusal before the fix. Its quant0/predictive control decoded successfully;
all three spatial frames and the low-delay configuration parsed completely.
The initial upper-region reproducer was subsequently renamed
`bands23-quant1-empty-upper2` when lower-region coverage was added.

Qualification now considers the mode at each actual parameter band. Syntax
still validates both signalled modes. A dormant nonpredictive mode cannot
restrict the already qualified all-predictive path; making even one band of
that mode active still produces the existing refusal. DSP is unchanged.

The authored corpus covers all seven resolutions, quantization modes1/2,
zero-width upper and lower regions, and dormant modes2/3/4/5:112 programmes
plus14 quant0/predictive controls, three AUs each, MP4 and Matroska. Tests
compare exact PCM against the controls, native timestamps/durations in both
decorrelation modes, ordinary playback, nine seek points and rewind in each
container. Configuration and full spatial-frame parsing are checked separately.
This is equivalence to an existing qualified path, not a newly independent
whole-spatial oracle or general nonpredictive EBQ acceptance.

The explicit generator reproduced254 files byte-for-byte with subprocess and
socket creation denied. Ordinary tests require neither FFmpeg nor network.
No private or ISO source media or codec parameters are included.

The new four-test target and existing six-test physical-preparation target
completed:10 passed,0 failed,0 ignored. Exact command and log hash are in the
adjacent JSON. Full-current-tree qualification and full codec parity are not
claimed. Active nonpredictive EBQ, active unequal-grid HRTF and inherited
fragment prefix loss remain open.
