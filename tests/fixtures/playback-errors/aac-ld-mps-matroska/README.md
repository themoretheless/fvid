# AAC-LD / LD-MPS Matroska source-window regressions

Generate separately with `python3 scripts/generate_aac_ld_mps_matroska_fixtures.py`.
The generator remuxes only the own AAC/MPS packets and own AVC picture in the
`aac-ld-mps-ld-core` corpus. Every MKV has an audio and a video track. No private
media, FFmpeg, encoder, child process or network is used. Ordinary tests only
read committed files.

314 acceptance movies cover480/512 core samples,24/48kHz,all8 trees,TTT variants,
repeat headers,burst/delayed and fragmented spatial messages,STP,GES,parameter
grid changes,escaped SSC lengths and absent SBR/PS flags. Two core-only controls
preserve the ASC flag-off behavior. Four late spatial corruptions and two bad
ASC lengths are specific-error rejection cases,not acceptance.

The same eight AAC packets and spatial parameters are in each paired MP4
control. PCM must match byte-for-byte through both Matroska export APIs and the
AudioStream reader after rewind,forward/backward/EOF seeks and repeated ranges.
The actual AudioWorker capture test also compares every PCM byte and checks
packet timestamps against the exact sample clock. Matroska publishes integer
nanoseconds,so its timestamp rounding must be downward by less than1ns; MP4's
existing rational timestamp comparison remains exact.

Before the fix,the480/48kHz plain movie failed at the second source packet with
`LD-MPS frame spans discontinuous timestamps`: export supplied PTS0 and a
positive duration for every packet. Correcting the untimed decoder association
then exposed `delayed Matroska source lookahead exceeded` on the delayed movie.
The acceptance tests require playback,not just refusal of those failures.

Controlled export must drain a complete four-packet prefix. A one-packet prefix
of a two-core spatial frame must refuse the unfinished association and emit no
PCM. All malformed files must reach their intended ASC/spatial error rather
than an unrelated container/geometry failure.

Scope is EP0 AAC-LD without SBR at the listed geometries. The paired MP4 chain
is a previously qualified compositional control,not an independent full decoder
oracle. CodecDelay/DiscardPadding combinations,arbitrary container clocks,
protected AAC+MPS combinations,full LD-MPS conformance and device output are not
newly qualified by this corpus.
