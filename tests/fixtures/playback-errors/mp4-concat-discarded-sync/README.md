# Selected audio with unmarked discarded video sync

Owned PCM-coded AVC picture bytes have a conservative empty `stss` table, declaring no video sync samples. The file also contains two owned mono AAC-LC pulse packets at 48 kHz. Rewriting box sizes preserves original video mdat offsets and recomputes the added audio offset. There is no private media, external encoder or network requirement.

The original regression admits the container and reads its two tracks, but rejects concatenation selecting only AAC because the video sync guard inspects discarded tracks. Default/all-track and selected-video copy intentionally retain the refusal. These are refusal controls, not video playback acceptance.

Audio acceptance checks decoding the selected original track to 2048 samples, public plan/concat selecting only audio, byte-for-byte preservation of four AAC packets, and output decoding to exactly 4096 mono sample frames at 48 kHz. This extends FVID-0328 and does not receive another ID. Generation is separate from ordinary tests.
