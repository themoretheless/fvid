# Zero-allocation LFE with conventional and AHT coding

96 owned cases: independent/converted streams, all eight full-band/LFE AHT
plane masks, explicit full-band dither on/off; 64 zero-allocation cases at
exponents 8/12 and 32 coded positive controls at exponent 10. Each has raw
three-frame audio, owned VP9/Matroska and VP9/MP4 forms. Regenerate separately:
`python3 scripts/generate_eac3_lfe_zero.py`. No FFmpeg/network is required.

Before the fix, `a0-e8-t0-m4-d0` decoded a LFE peak of 0.029320521 rather
than true zero. Conventional LFE and coded-signal controls passed. The three
silence/reader/worker tests reproduced this specific failure. During initial
test authoring, the generic positive threshold 0.001 incorrectly rejected the
seven-bin LFE signal (peak 0.00041176524); that authoring error was corrected
to a non-silence guard 1e-6 before recording the red baseline. Silence remains
an exact-zero requirement, not a tolerance.

The fix records AHT zero-allocation bins for LFE as well as full-band planes,
and suppresses only substituted LFE noise after inverse AHT. Coded LFE is
not muted. Four integrations check all three readers, full-band flags, coded
signal identity, sample counts, rewind and worker PCM/PTS with assignment
0xb and 20 ms partial-packet seek. A private decoder test verifies the actual
seven LFE allocation pointers (0 versus scalar 15/AHT 19) and coefficients.

The policy follows unchanged conditional-dither semantics: ETSI TS 102 366
6.3.4 makes substitution conditional on channel flags; E.1.2.4 audblk only
transmits/implicitly enables those flags for full-band channels. E.2.4 changes
allocation/quantizers/inverse DCT, not this flag syntax. Applying conventional
LFE true-zero substitution to AHT is an inference from those combined rules,
not an explicit normative sentence specifically naming AHT zero-bit LFE.
FFmpeg's parser independently initializes its LFE dither flag to zero, but
its AHT zero-pointer branch fills noise without checking that flag.

The optional strict LFE audit is deliberately separate:
`python3 benches/check_eac3_lfe_zero_reference.py --probe <audio_probe>`.
Against FFmpeg 9.0.2, FVid is silent 32/32 and the reference 16/32; all 16
reference AHT-LFE cases emit noise (maximum 0.0402078703045845). The audit
exits 1 and its result is not included in green ordinary validation.
Report: `/tmp/fvid-lfe-zero-reference.json`.

This covers wholly zero and wholly coded LFE spectra, not mixed allocation,
dependent presentations, all rates/profiles or complete E-AC-3 support.
