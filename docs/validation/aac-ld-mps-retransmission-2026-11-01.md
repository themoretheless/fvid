# Prefix-preserving LD-MPS retransmission

Before this change, `plain-tree0-fragmented-gain-retransmit` failed at AU1
with the specific replaced-fragment source-association refusal. The old start
fragment and the new whole message contain exactly the same prefix bytes.
Appending only the remaining suffix with the already supported stop envelope
produces the identical spatial `Message`. Neither source ownership nor PCM
history needs to be guessed or discarded in this case.

The ancillary assembler now counts prefix loss separately from total
replacements. Both counters are checked before committing any state; reset
clears both. The bounded ELD-tail owner marks inherited source association
unresolved only if prefix bytes were lost. Completing or retransmitting the
inherited message clears that provenance; subsequent local replacements do not
inherit it. Arbitrary replacement that changes/truncates the accumulated prefix
remains refused. This is not concealment, resynchronization or inferred mapping
of lost spatial data to older audio.

The new92-file corpus has45 authored three-AU programmes: all40 fragmented-gain
controls across eight trees, ordinary ELD and shared single/double-rate LD-SBR,
plus five retransmission-then-local-replacement cases. Every programme has MP4
and Matroska video. Assembled payloads match ordinary stop continuation exactly;
native PCM and timestamps, checkpoint replay, malformed-tail retry, exported
PCM, live playback, rewind and forward/backward/EOF seeks match controls exactly.
The five previously saved inherited refusals actually preserved their prefix;
their old pattern now has acceptance coverage in the new corpus. The negative
fixtures now signal a valid changed downmix gain inside the inherited prefix,
so they still isolate real prefix loss and verify correct-packet retry.

The related21 tests pass with no failures/ignored tests. To keep the running
90-target baseline's existing executables intact, these five test files were
compiled as modules in one temporary test target with a distinct executable
name; that temporary source is removed after qualification. Ordinary saved
regressions remain standalone targets. The baseline started on26b1d066fd and
is not a full-current-tree result for this new implementation.

Four actual macOS `audio_probe --play-device` runs pass all16 full-playback,
quarter/60-percent seek and rewind drain phases. They cover plain trees2/7 and
480/512 single/double-rate LD-SBR, including retransmission then local replacement.
These short stereo-device checks are not subjective or physical multichannel
qualification. Raw device results are in the adjacent JSON.

All339 files across the92-file new corpus and247-file local-replacement corpus
regenerate byte-identically with process and socket creation blocked. Ordinary
tests only read saved authored files; no FFmpeg, network or generation required.
The former refusal pattern was verified before the production fix.

```sh
python3 scripts/generate_aac_ld_mps_retransmission_fixtures.py
python3 scripts/generate_aac_ld_mps_local_replacement_fixtures.py
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline \
  --no-default-features --features player --test aac_ld_mps_retransmission \
  --test aac_ld_mps_local_replacement --test aac_ld_mps_transport \
  --test aac_ld_mps_inband --test aac_ld_mps_session
```

The overall codec-gap goal remains open. Inherited replacement losing any
prefix, unequal HRTF-grid processing, unqualified EBQ configurations and other
codec/profile combinations are not completed by this equivalence fix.
