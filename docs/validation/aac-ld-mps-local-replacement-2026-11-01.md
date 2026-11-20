# LD-MPS local ancillary replacement

The native source owner previously rejected every increase in the assembler's
cumulative replacement counter. That also rejected an incomplete fragment
created and replaced inside the current AAC extension tail, before it acquired
any older PCM/source association. The assembler already supported that transport
operation; the blanket native guard introduced the playback refusal.

The extension reader now reports whether a replacement actually discarded a
fragment inherited at entry to the bounded tail. Completing an inherited block
clears its provenance. Later fragments in the same tail are local. A discard
of inherited data sets a sticky result, which a later local replacement cannot
clear. The native owner uses this result instead of comparing cumulative counts.
Raw public parsing still returns the same messages and preserves counter
semantics. Reader/assembler updates remain transactional. No persistent decoder
fields, new heap owners, source-window guessing or PCM arithmetic are added.

The scope is one logical AAC AU's bounded extension tail after the core and any
explicit LD-SBR prefix, not an arbitrary physical protected packet containing
several logical AUs. Discarded local bytes never enter the spatial session.
No earlier audio windows are skipped, concealed, relabelled or reset.

## Specific reproduction and acceptance

The before build restores the previous cumulative-counter guard while retaining
the new diagnostic parser. The first authored movie fails specifically on AU0:
`AAC LD-MPS replaced-fragment source association is not yet implemented`.
The normal complete-message control already plays. The same fixture accepts
with the provenance-based guard; this is not a new refusal-only test.

The corpus has40 authored in-band parent configurations and120 new programmes:
40 local nonempty starts,40 local empty starts, and40 programmes which first
complete an inherited fragment and then replace a new local fragment. Sources
include ordinary ELD PCM and single/double-rate LD-SBR shared-QMF interfaces,
480/512 core samples,24/48kHz output, and all eight spatial tree shapes.
The three-AU parent cores, parameters and pictures are authored in the repository;
no private or original ISO media/codec parameter sets are copied.

Each accepted programme has MP4 and Matroska video. Matroska channel metadata
uses the reconstructed spatial output layout. Tests compare complete assembled
messages and pending status with the parent control, then compare native PCM,
PTS/duration and checkpoint replay exactly. Truncating the final extension child
must fail and permit a correct retry. Public MP4 export, Matroska export, both
live readers' playback/rewind and sub-frame/frame/EOF/backward seeks preserve
all samples and channel order. No waveform fitting or new DSP oracle is used:
this changes message/source admission, while the completed codec inputs remain
byte-identical to the previously qualified authored controls.

Five additional MP4s replace a fragment inherited from an earlier AU. A later
local replacement is deliberately included to test the sticky rejection result.
They still reproduce the specific unimplemented source-association refusal.
Correctly retrying the original completing fragment preserves every native
history. These five are refusal/reproduction tests, not playback acceptance;
cross-AU replacement remains an open gap requiring its source-window semantics.

## Verification

The247-file corpus (240 accepted movies, five refusal movies, packets/manifest)
regenerates byte-identically with `subprocess.Popen`, `socket.socket` and
`socket.create_connection` replaced by failing sentinels. Explicit generation
is separate from ordinary tests. No FFmpeg, foreign decoder or network is used.

20 unique tests pass in `aac_ld_mps_local_replacement`, `aac_ld_mps_inband`,
`aac_ld_mps_session` and `aac_ld_mps_transport`, with zero failures/ignored.
The two new tests are also rerun after adding the final coverage and truncation
assertions; repeated execution is not counted as additional unique tests.

Four actual macOS `audio_probe --play-device` runs cover ordinary stereo,
ordinary six-channel inherited completion, double-rate480 LD-SBR and single-rate
512 LD-SBR. Full playback,25%/60% seek and rewind complete EOF drain for all16
phases. This is a short device/navigation smoke test, not subjective quality,
sustained throughput or a universal LD-MPS conformance claim.

The full codec-gap objective remains open, including inherited replacement,
unequal HRTF grids, unqualified EBQ configurations and other tool/profile axes.

```sh
python3 scripts/generate_aac_ld_mps_local_replacement_fixtures.py
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline \
  --no-default-features --features player --test aac_ld_mps_local_replacement \
  --test aac_ld_mps_inband --test aac_ld_mps_session --test aac_ld_mps_transport
```

Subsequent qualification: `aac-ld-mps-retransmission-2026-11-01.md` admits inherited whole-message retransmission preserving every accumulated prefix byte. The five negative movies now use a valid changed downmix gain in that prefix; the previous prefix-preserving pattern has new acceptance coverage. Arbitrary inherited prefix loss remains refused.
