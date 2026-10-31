# Shared normalized128-band QMF admission regression

Explicit own generation:
`python3 scripts/generate_aac_ld_mps_shared128_fixtures.py`.
The66 short videos and analytical core PCM come only from the separately
authored `aac-eld-mps128` corpus. All8 trees,both TTT variants,64/88.2/96kHz
and480/512 cores are covered. No private media or codec parameter sets.

The pre-fix own movie decodes normally through native PCM. Its same analytical
core data,after the independently qualified128-band analysis bank,then refuses
at `invalid LD-MPS shared QMF frame`:the constructor selects128 while the
shared-input validator incorrectly expects64. Acceptance is enabled with the
validator fix; no passing refusal is counted as playback acceptance.

2,642,688 shared-input PCM values match PCM-session analysis exactly and the
same native movies within1e-9. The test exercises complete/source-split QMF
windows,timestamp/source-span conservation,cloned checkpoints,nonfinite last-bin
and late spatial corruption rollback,EOF drain,reset and replay.
These are component/session API checks,not new physical-device or GUI seek
qualification. The existing frozen C bank/chain controls run separately.

This fixes an already exposed normalized QMF API. It does not admit unqualified
coupled LD-SBR128 geometries,residual32/128 or mandatory profile-specific output
adaptation. Their existing refusals/qualification boundaries remain separate.
Own generation and ordinary tests require no FFmpeg,network or external process;
optional C reference generation remains outside ordinary tests.
