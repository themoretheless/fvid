# Multi-core AAC-LD / LD-MPS acceptance

Generate separately:
`python3 scripts/generate_aac_ld_mps_multicore_fixtures.py`.
The generator uses only own AAC spectral syntax, authored spatial parameters,
a sparse direct-cosine AAC-LD oracle and an own AVC picture. Each MP4 and MKV
has a synthetic video track. No private media, network, child process or codec
executable is involved. Ordinary tests read committed bytes only.

This is qualification of existing runtime behavior, not a newly fixed refusal.
The first hypothesis that integer-nanosecond rounding breaks a spatial frame
was disproved: the existing Matroska player adapter already supplies AAC without
a container duration. No production code is changed by this milestone.

Six geometries cover all additional integral multi-core frames within32 spatial
slots for the qualified24/48kHz,480/512-sample cores:

| Core samples | Rate | Packets/spatial frame | Spatial slots |
|---|---|---|---|
|480|24000|2|30|
|480|48000|4|30|
|512|24000|2|32|
|512|48000|2|16|
|512|48000|3|24|
|512|48000|4|32|

462 valid MP4 controls cover all8 trees,TTT variants,repeated headers,burst
and fragmented transport,STP,GES and23→9→4 grids. Each programme has four
spatial frames and8/12/16 original AAC source packets.606 valid MKVs include
1ns,1us and1ms timestamp quantization and floor/ceil/absent DefaultDuration.
There are12 late spatial corruptions in each container; these are specific
refusal/rollback cases and are not counted as playback acceptance.

The scalar core oracle extends LTP/window history beyond the previous8 packets.
Its first8 packets and every corresponding PCM byte must reproduce the frozen
`aac-ld-layout` corpus. Native raw core PCM is checked independently before
spatial processing. Complete spatial PCM uses that scalar core plus the
separately qualified session; this is not an independent whole LD-MPS decoder.

Tests compare both Matroska exports to the paired MP4, initial/rewind playback,
forward/backward/EOF seeks and repeated ranges. Complete controlled source
prefixes drain; incomplete spatial windows refuse without publishing PCM.
Late failures must preserve native history, retained bytes, retry output,
checkpoint restore and subsequent source PCM. Reader errors are also repeated
after rewind/reset. The actual AudioWorker capture test checks live-history
navigation and published timestamps against the sample clock with<1ns rounding.

Scope is EP0,no SBR,24/48kHz and32/64-bin spatial processing. Other rates,
128-bin/residual domains,protected AAC+MPS,CodecDelay/DiscardPadding combinations,
independent full conformance,device output and real-time performance remain
outside this new qualification.
