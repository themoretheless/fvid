# Existing prefix and loudnorm contract repairs

This follow-up adds no accepted ID. The registry remains at 418.

The seven gate failures reproduced at original commit d25e228c19 are repaired without changing any test. Existing authored synthetic fixtures cover prefix negotiation, interval trimming, remux selection, concatenation, and refusal/publication behavior. Baseline evidence is in ../edit-metadata-baseline-20261030.

Two further old failing tests were reproduced on the same separate original source/target: mix controlled-memory diagnostics and cumulative Matroska concatenation. The original concat test first fails at max_packets=1; the initial expanded repair run advanced it to max_packets=3, where header counting still applied a packet cap to an unreached packet. Both logs are retained.

Root causes and repairs:

- Metadata copying for bare ADTS needs no full-file duration probe. Validate only the selected encoded prefix before returning empty container tags.
- Stream selection and plan rendering must not perform a whole-file payload probe. Header-only selection retains the explicit-selection rule for extra streams; selected interval admission still checks actual encoded bytes and work.
- Concat geometry for an unreached segment must remain metadata-only. Encoded limits apply to the cumulative reached prefix; global parser and controlled-memory/index bounds remain active.
- Loudnorm defaults must be forwarded as an explicit first-audio selection when entering the owned multi-track route. Packet-budget refusals remain owned errors rather than unsupported fallthrough.
- ADTS normalization planning must not build a generic decode plan that decodes packet contents. Header-based decoder admission still charges combined decoder and output-tag memory, and loudness preflight retains packet-budget checks. Execution still refuses the authored truncated tail.
- A lowered MP4 metadata limit supplied through max_controlled_bytes must report the controlled-memory reason while retaining the specific metadata-budget reason.

The first expanded run exposed two regression mistakes in these repairs (missing Matroska index diagnostic mapping and combined ADTS/tag admission). Both were corrected; the failed run remains as evidence. Final qualification is recorded in status.json. No performance claim is made.

Saved logs normalize only trailing empty lines to one final newline. Diagnostic lines and results are retained.
