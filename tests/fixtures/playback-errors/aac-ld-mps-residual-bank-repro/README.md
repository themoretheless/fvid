# Forbidden LD/residual32/128 profile combinations

These44 residual movies are profile counterexamples,not missing LD-MPS modes.
See `docs/LD_MPS_PROFILE.md`. The44 ordinary controls and12 bare ELD scalar
controls remain positive.1872 residual blocks parse separately,but syntax and
numerical-bank limits do not establish an allowed codec profile.

The old ignored acceptance expectation has been replaced with a real profile
refusal test. Ordinary MPEG Surround/HQ residual integration remains a separate
unfinished capability;it is not implemented by this rejection.
Generate with `python3 scripts/generate_aac_ld_mps_residual_bank_repro_fixtures.py`.
All media is own synthetic data. Tests and generation need no FFmpeg,network
or external process;no private frames,audio or parameter sets.
