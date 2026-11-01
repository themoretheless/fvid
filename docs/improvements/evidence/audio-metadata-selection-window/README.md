# Metadata discovery and active audio windows

Accepted FVID-0475 after the recorded frozen-source qualification. Ogg Opus/Vorbis metadata was probed using the selected
encoded-window byte/work caps. This rejected copying tags to MKA and inspecting
unreached concat stream headers despite successful source/codec admission.
Temporary call traces locate the consumers;all tracing is removed byte-for-byte.
The public full-file probe policy remains unchanged.

A private metadata-only probe first validates original cancellation,zero limits,
RSS and memory controls,then reads metadata with the normal container packet cap
and without the active media-packet work bound. Codec/interval inspectors retain
the original byte/work controls before output publication. The three consumers
are standalone PCM-to-Matroska metadata extraction,single-audio source discovery,
and the generic audio stream-index metadata fallback.

Final identical four-case baseline:two controls pass,two acceptances fail.
Controls decode/seek authored Opus and verify full source metadata;active oversized
packets,zero work/byte limits,low memory and cancellation refuse without output.
Acceptances cover automatic/explicit selection,Opus/Vorbis,WAVE/MKA publication,
and an exhausted concat prefix whose later stream is only inspected for headers.
The final candidate passes all four without weakening any existing Rust test.

Stand corrections are documented separately:Opus silence has a finite numerical
floor rather than bitwise-zero floats;all samples retain exact counts and a
1e-25 absolute bound. The front PCM oracle is independently normalized float32,
matching the specified float WAVE output. Initial failed stands are preserved.
These adjustments and extra variants add zero IDs.

The generator separately authors short silent Opus/Vorbis,arithmetic PCM,and
G.711 source controls. It uses no private media,FFmpeg,encoder or network.
Ordinary tests read committed bytes. No physical audio or performance claim.
