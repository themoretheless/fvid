# Recovering missing and truncated AVC primary slices

88 owned 32x16 Extended-profile synthetic videos. Generate separately with
`scripts/generate_avc_primary_recovery.py`; ordinary tests only read checked-in
artifacts. No FFmpeg, external commands, network or private media/parameters.

Coverage includes ordinary and A/B/C slices, either spatial half intact,
I/I, I/P, P/I and P/P slice mixtures, missing or truncated PCM payloads, ASO,
startup IDR and non-reference recovery. IDR fixtures correctly exclude data
partitioning. A good primary region contains base+2 PCM samples; the replacement
contains base+1 PCM or unchanged zero-CBP prediction from the previous reference.
The damaged primary area was independently authored as base+3 and truncated.

Every decoded sample is checked against a scalar oracle and independently
written rank-zero control through two successors and reset. Non-reference
pictures must leave the following reference pixels unchanged. Software MP4
checks exact 40 ms presentation/duration, frame count, EOF, rewind and sync seek.
Preparation checks original wire rank and entropy offset against selected
normalized headers. Truncated cases independently parse both primary headers,
finish the intact slice, and require a specific truncated-bit-field entropy
error on the damaged slice. Without redundancy the same primary packets must
still fail. The pre-fix acceptance test failed with `incomplete intra slice`.

954 generated artifacts (868,519 bytes) reproduce identically with external
commands/network disabled. This covers whole intact slices and disjoint
replacement slices with one PPS. Recovery inside a damaged slice, partial
spatial overlap, malformed headers/length framing and differing PPS remain open.
