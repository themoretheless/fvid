# Stable immediate MP4 audio edit packet admission

Accepted FVID-0480. All eight final qualification gates completed successfully.

Published production 51c2ab2c11 admits every indexed packet when a finite edit
provides the natural endpoint and no explicit interval/work cap is given.
Valid PCM/ALAC sources therefore fail because a larger final packet, never read
by the selected edit, exceeds the encoded byte cap. Full controls prove that
this larger packet is valid and all three source windows match independent PCM.
Explicit max_packets=1 proves that the minimal edited source only needs one packet.

Final identical six-test published baseline: four controls pass,two acceptances
fail. The minimal audio-only and hybrid forms fail planning and decode in both
None/64 MiB modes. Sixty further PCM/ALAC hybrid cases cover movie scales
8000/1000/1001,elst versions0/1,early endpoint,preroll,repeated media,a leading
empty edit,and a wholly silent edit. Overall248 plan/decode variants falsely
refuse. Successful complete controls and genuinely reached oversized packet
refusals remain separate passing assertions.

The shared admission helper now receives the reader's movie clock. For stable
immediate decoder descriptions with an exact native media grid,it uses the same
AudioTimeline integer schedule as decoding and retains the largest required
source prefix,including preroll and repeated edits. Uniform indexes use their
closed timestamp formula rather than iterating virtual samples. Existing read
caps remain enforced. Invalid clocks fall back to conservative admission.
Delayed AAC,changing descriptions,declared gaps and non-native media grids keep
their previous conservative probing/admission; this does not claim to solve
unreached-packet admission for those different decoding paths. No public API or
codec support changes,and no performance claim.

A candidate-only regression was caught for pure silence:the new zero-read
prefix initially omitted admission of the decoder initialized before scheduling.
The final candidate charges the initial decoder even with no active packet.
The direct library16KiB refusal and pre-cancelled public plan are retained as
controls. Its failure log/source hashes are separate;the correction adds0IDs.

131 files regenerate exactly offline from authored neutral VP9,analytic PCM and
uncompressed ALAC,with no private media or codec parameters. Generation remains
separate from ordinary tests and needs no FFmpeg or network. Controls verify
exact PCM,actual decoded work,native software pictures,seek,rewind and EOF.
No physical audio/GPU or native-window duration qualification is claimed.

Initial two-test baselines and expanded candidate logs are history. Final
baseline restores all eleven published production files,then guardedly restores
the candidate. The candidate-source snapshots describe that restored stage;
final batch-source hashes additionally cover the compact-index arithmetic
refinement made before qualification. Final test/generator hashes remain identical for before and
qualification;existing Rust regressions are unchanged. Variants,clock plumbing,
resource correction and all controls count as one independent admission fix.
