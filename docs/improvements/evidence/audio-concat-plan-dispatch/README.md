# Concat stream reporting uses the configured decoder route

Accepted FVID-0476 after the recorded frozen-source qualification. After successful sample-domain concat admission,the plan
reported first-source streams by calling the legacy AAC/PCM planner directly.
G.711 RIFF was parsed as ADTS. Opus and Vorbis have the same reporting defect.
The public decoder planner already supports these sources and performs the
normal selection/budget validation;concat now uses that route.

The independent G.711 source control decodes to exact scalar-authored PCM.
With the metadata fix staged,the unchanged three-test baseline passes two
controls and fails all six plan variants as invalid ADTS:three formats under
default/controlled-memory configurations. A separate published-production-source
baseline confirms both G.711 cases fail as invalid ADTS;the unmodified published
Ogg route also exposes the preceding metadata-budget failure.

Final candidate passes all three tests. Successful plans name the actual codec
and decode_to_pcm disposition,and execution publishes both complete segments.
G.711 output matches every independent float32 sample twice;Opus/Vorbis have exact
frame counts and bounded finite silent numerical output. Invalid selection,
zero limits and tight memory still refuse before publication.

Temporary ADTS tracing identified the RIFF prefix and reporting call stack;
its source is restored byte-for-byte. Final source hashes match all qualification
gates after the published-baseline experiment. No new codec implementation,
public API signature,physical audio or performance claim is made.
