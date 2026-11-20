# In-band RS correction strength

Previously, a fully authored EP2 ELD/LD-SBR video failed specifically with
`EP RS escaped correction rate is not yet implemented`. Its RS class set the
rate-escape flag and provided a three-bit strength code in each frame header.
The validator refused the descriptor before the existing header decoder could
resolve that code. The refusal is removed: rates are resolved before RS block
sizing, grouped-class validation and error correction. No FEC/CRC check is
bypassed. Concatenated source frames still forbid escaped parameters.

The external EP Tool `RetrieveHeader` uses the same eight-value mapping for
FEC types 0, 1 and 2. A component harness extracts this function verbatim from
the externally cached source and compares all 24 mappings. It supplies already
recovered header bits via an explicit identity FEC shim; it does not compare
foreign header protection, RS correction or complete audio. The original source
and extracted function hashes and compiler identity are pinned in the reference
receipt. Original source is not copied into the repository or linked at runtime.

The independent authored generator exercises all eight strengths (0, 3, 4, 6,
8, 12, 16, 24) across 12 successive access units. Its 32 accepted programmes
span 480/512-sample mono/stereo ELD cores, with and without double-rate LD-SBR,
until-end and declared lengths, per-frame CRC changes and corrected errors.
24 malformed programmes verify CRC failure, damage beyond capability and
truncation with precise error/retry expectations. MP4 video and Matroska contain
own synthetic nonzero sources; complete expected PCM comes from the existing
independent authored core/SBR oracle, never a production decoder snapshot.
The `k0` filename token is the generator's descriptor label, not a static
strength: frame metadata records every actual in-band value.

Additional two-member FEC=2/FEC=1 component vectors cover all strengths, exact
recovery and correction, plus rejection of unequal strengths within one group.
Continuous PCM, checkpoints, malformed-packet retry, both container paths,
interval exports and reader seek/rewind are automated. Ordinary tests require
no FFmpeg, network, C compiler or fixture generation. Explicit generation is
byte-identical with subprocess/socket creation forbidden.

The focused gate and rebuilt macOS device probe are recorded in adjacent JSON
receipts. This adds EP rate-escape playback; it is not complete codec parity or
qualification of the remaining LD-MPS EBQ/HRTF/topology tools.

Validation completed: 39 unique focused tests passed, none failed or ignored.
The final nine-test escaped-rate target supersedes its initial eight-test run;
duplicate runs are not added to the total. All 179 authored files reproduce
byte-for-byte. The rebuilt default-device probe completed 12 runs and 48 drains
(full playback, two seeks and rewind), spanning both frame sizes, mono/stereo,
core/SBR, clean/corrected and declared/dynamic-CRC cases. This establishes device
delivery, not subjective listening or physical speaker-wiring certification.
