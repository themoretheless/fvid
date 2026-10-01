# HEVC persistent Rice adaptation reproducer

Synthetic HM 18.0 64x64 three-picture streams cover 8/10-bit skip and bypass,
with persistent Rice enabled/disabled. Deterministic raw frames are generated
in Python and packaged by the owned fixture MP4 writer. HM encoder
reconstruction and HM decoding agree on all saved YUV references. No FFmpeg
is invoked. Generate separately
with `scripts/generate_hevc_context_samples.py --rice` and HM tool/config paths.
Normal tests do not invoke external tools.

Current status: unsupported. `persistent_rice_current_refusal_is_specific`
checks the enabled SPS fails with the remaining-range-tools error, while each
disabled counterpart decodes exactly through reset/rewind. This passing test
is refusal coverage, not playback acceptance. The separately ignored
`persistent_rice_acceptance_matches_oracle` must be enabled with the decoder
implementation and the old refusal expectation replaced.

Implementation must keep four StatCoeff counters: luma transformed, luma
skip/bypass, chroma transformed, chroma skip/bypass. Each coefficient group
starts its Rice parameter from StatCoeff/4, updates the statistic on its first
remainder, and uses uncapped persistent adaptation for subsequent levels.
CABAC snapshots must carry these counters for WPP/dependent-slice restore;
independent slice/tile starts reset them. Merely accepting the SPS flag is not
support for this tool.
