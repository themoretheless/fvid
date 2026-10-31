# Signalled LD-MPS 3D-stereo inversion

540 authored twelve-packet programmes, each in MP4 and Matroska, plus five
independent saved stereo PCM controls. No private media or KEMAR data.
The SSC 3D-stereo flag selects an HRTF-weighted TTT inverse; output remains
six/eight channels. This is not a binaural-output renderer.

Coverage: HRTF set 0, trees 2/3/4, all six TTT modes, symmetric/asymmetric
five-direction responses, equal 23/9/4 parameter grids, phase cutoff at band 12,
standard-inverse priority, arbitrary-downmix before inversion, smoothing and
regularization. Owned ELD/LD-SBR inputs exercise 32/64/128-bin PCM paths.
The 267-byte SSC also exercises the escaped in-band configuration length.
The extra unequal-grid movie contains valid 23-band parameters and a 9-band
HRTF in both ASC and first in-band SSC. Its test is a specific refusal test,
not a playback acceptance test.

Generate explicitly with `python3 scripts/generate_aac_ld_mps_3d_fixtures.py`.
Reference generation is separate: `generate_aac_ld_mps_3d_reference.py` uses
the pinned original MPEG reference archive and an owned Decimal polar inverse.
Ordinary tests read frozen artifacts and need no FFmpeg, network or C compiler.

The reference retains original C raw TTT bases and M2, but replaces every
selected 3D inverse with an independently formulated 80-digit polar calculation.
The stock float C SVD loses orthogonality at near rank deficiency; its NaNs and
deviations are recorded, not treated as golden values. Rank-deficient inversion
uses an explicit determinant-phase orthogonal completion, also checked against
analytic rank-one and zero-matrix controls. Matrix endpoints use an absolute
3e-4 bound (observed maximum 0.000212201); the C float raw basis is amplified
by regularization. Native PCM composition is checked separately and exactly.

Tests cover decoding, timed source windows, seeks, rewind, checkpoints,
truncated-packet rollback and worker navigation. These are synthetic software
checks, not physical audio, real-time performance or full-profile conformance.
Unequal grids, HRTF on other trees, nominal-SSC downscale remapping and direct
downscaled shared-QMF remain outside this qualification.
