# Ordinary128-bin LD-MPS with high-rate AAC-ELD

Generate separately with `python3 scripts/generate_aac_eld_mps128_fixtures.py`.
Every MP4/MKV contains own high-band AAC-ELD syntax, authored spatial parameters
and an own AVC picture.18 bare ELD controls have an independently generated
sparse-cosine/ELD-window PCM oracle. No private source media, encoder, process
or network is used. Ordinary tests read committed bytes only.

330 programmes cover64000/88200/96000Hz,480/512-sample cores,all8 trees,TTT
variants,plain/repeated headers,23→9→4 grids,delayed bursts and fragmented
transport. Each has MP4 and Matroska versions:660 acceptance movies.512-sample
cores use four128-bin slots;480-sample cores span four original AAC packets
per15-slot spatial frame.12 late corruptions in each container are precise
refusal/rollback cases,not acceptance. There are18 additional core-only MP4
controls. `exports` repeats the330 programmes as MKV for the worker harness.

The initial AAC-ELD controls passed before the runtime change,while
`n512-r64000-tree0-v1-plain.mp4` failed specifically with
`LD-MPS128-band decorrelation is not yet qualified`.
Acceptance now requires complete PCM,correct rate/layout/count,matching owned
and frontend exports,AudioStream playback,forward/backward/middle/EOF seeks,
rewind and repeated ranges. Worker metadata forces a published first spatial
frame plus further core history before each seek and probes both sides of
source boundaries4*n and8*n. Timestamps use exact MP4 sample clocks and less
than1ns Matroska rounding.

Late corruption must leave retained bytes and all native history unchanged:
retry is checked both before restoring a checkpoint and after restoring it,
including later source frames and EOF drain. Reader errors are repeated after
rewind/reset. Complete controlled prefixes drain; incomplete spatial windows
refuse without publishing PCM.

The PCM oracle composes the independent ELD core with the separately qualified
spatial session. Independent128-bin bank and QMF-chain component controls are
in `aac-ld-qmf128` and `aac-ld-mps-qmf128`; this is not an independent whole
codec decoder or formal conformance certificate.

Scope is ordinary,full-complex,EP0,no-SBR,downscale1 AAC-ELD spatial playback at
the listed high rates. Residual AAC at128 bins,STP/GES128,HRTF/arbitrary downmix,
nonzero time alignment,other protection/downscale combinations and physical
output/performance remain open. AAC-LD's standardized rate domain is unchanged;
this corpus must not be described as AAC-LD96kHz acceptance.

Profile boundary: ISO/IEC14496-3:2019 Table1.16 footnote a contemplates high
coded rates but requires Low Delay AAC v2 output adaptation to<=48kHz. These
tests verify native-rate output,not that profile-level adaptation or full
profile conformance. Source and scope are pinned in the source-audit report.
