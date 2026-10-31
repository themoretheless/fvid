# Canonical nominal LD-SBR CLDFB phase

## Defect and primary source

The nominal analysis matrix used `2n-96`. ISO/IEC14496-3:2009 section4.6.19.4.1
(printed272,PDF page748) specifies `2 exp(i*pi*(k+0.5)*(2n-95)/64)`.
The full PDF was rendered and visually checked, rather than relying on OCR.
The source SHA256 is
`913b72c340ba90bc9f4fc3bbc2edad8005369076c78a51f147bbbbbe6b5cf20d`.
[Primary publication copy](https://ossrs.io/lts/zh-cn/assets/files/ISO_IEC_14496-3-AAC-2009-90cda427e845cc176293bffee3f92f99.pdf).
The centered factor-one bank already uses95 with the same window. The fix
corrects nominal analysis, without altering synthesis, gain or core time origin.

## Specific independent acceptance

Eight locally authored eight-AU programmes/16 MP4+Matroska movies cover480/512,
mono/stereo,CRC and single/dual-rate LD-SBR. Independent core cosine PCM is
scaled exactly by16 via own global gains; direct canonical CLDFB/HF/noise
reconstruction supplies frozen PCM. No production decoder generates gold.
No private or ISO media/codec parameters enter the authored fixtures.

Before production correction both new acceptance tests fail: complex analysis
bins differ from centered factor-one, and n480-c1-x1 PCM differs by0.00013408763.
After correction both pass, with the unchanged1e-7 PCM threshold. Other dependent
PCM oracles are regenerated from the same independent mathematics. Ordinary
tests use frozen assets and require neither generators nor network/foreign codecs.
Two regeneration runs with sockets/subprocesses disabled reproduce all26 generated
files exactly; see `aac-cldfb-phase-generation-2026-10-31.json`.
All62878 previously tracked AAC/EP1 compressed inputs retain their pre-fix hashes.

An older downscaled test incorrectly expected nonzero terminal alignment bits to
fail. ELD byte_alignment accepts arbitrary values, already covered by the own
`aac-alignment` movies. The shared test now verifies identical PCM for that change,
including checkpoint replay, rather than restoring the obsolete refusal.
Twelve existing EP2 concatenated ELD padding movies (480/512,2..7 source AUs)
move from obsolete alignment-refusal metadata into ordinary acceptance. Their
new dedicated test verifies that EP recovery differs from the control in exactly
one terminal padding bit and yields identical PCM. The existing acceptance
loops now also verify their exports,range/seek/rewind and transport equivalence.
The movie/AU bytes remain unchanged. Independent RS and nested EP1 gold copies
are refreshed;no native decoder supplies reference PCM and no tolerance changes.

## Playback and external observation

The audio-worker regression checks the new corpus through playback/navigation.
A real macOS stereo48000Hz device drains n512-c2-x2 through full playback,
seek25%,seek60% and rewind. Device progress does not certify listening quality.

Optional `scripts/validate_aac_ld_sbr_fdk_pcm.py` performs an offline zero-lag
comparison on all26 checksum-pinned original LD-SBR inputs against the original
pinned FDK. Both before/after reports are complete and retain identical input,
ASC/AU and FDK PCM hashes. Only enclosing edit-box tags are hidden for raw
comparison; exported original/raw ASC/AUs are required byte-identical. All AUs
and scalar/rate/channel counts match. No fitted delay, phase correction, gain
normalization, skipped prefix or conformance threshold is used. A later2048-frame
window is separately labelled diagnostic.24/26 native PCM streams change;the
two Twi sine-only outputs are unchanged,as expected for zero core analysis.

These reports retain substantial raw waveform differences and do not prove
ISO/FDK waveform conformance. Decoder time-origin/reference-phase coupling remains
open. This is a specific canonical phase correction, not full codec completion.

See `aac-cldfb-phase-fdk-{before,after}-2026-10-31.json` for reproducible PCM
metrics and pinned source/probe/input hashes. Foreign codecs remain external
reference tools, outside production and ordinary tests.

## Completed scoped gates

1138 unique tests pass,0 fail,1 pre-existing ignored:
589 fvid-media unit tests,one actual worker corpus test and
548 tests across101 integration targets. The two new phase
acceptance tests are included,not counted twice. The broad discovery run found
eight targets with stale independent PCM copies or the obsolete alignment
refusal;all eight pass after correction. AAC EP1 itself is additionally checked.
Every final retry consumes frozen fixtures after generation has completed.
See `aac-cldfb-phase-tests-2026-10-31.json` and
`aac-cldfb-phase-integrations-2026-10-31.json` for counts and per-target evidence.
All scoped tests are offline and exclude default features. Integration/worker
gates enable player;media units do not. Native media uses opt-level2.
