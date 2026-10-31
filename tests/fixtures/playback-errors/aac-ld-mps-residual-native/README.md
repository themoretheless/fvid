# Historical LD/residual numerical compositions

These files are nonconforming LD profile combinations. See
`docs/LD_MPS_PROFILE.md`. Native AAC now rejects them before PCM,including
malformed variants whose deeper syntax remains tested separately.

Standalone session/matrix/residual math tests retain controlled numerical
comparisons,fill handling,ICC effects and transactional corruption checks.
Those are component tests,not codec acceptance. Ordinary MPEG Surround/HQ
residual integration remains separate and unfinished.
Generate explicitly with `python3 scripts/generate_aac_ld_mps_residual_native_fixtures.py`.
All media/parameters are locally authored;ordinary tests use frozen data only.
