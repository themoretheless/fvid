# Arbitrary-tree post-shaping component

Historical component milestone at8429dc0934. Native full-band playback is now
integrated; see `aac-ld-mps-arbitrary-playback-2026-11-01.md` for current evidence
and remaining boundaries. The earlier refusal fixture was promoted to acceptance.

Arbitrary-tree extension syntax and physical CLD preparation already exist,
but the matrix constructor refuses this tool. This work adds the owned
post-shaping gain-routing component required to implement it. It does not
remove that production refusal or claim arbitrary-tree playback acceptance.

The component compiles preorder binary trees into source-to-leaf paths and
routes their energy-normalized CLD gain products to dense output permutations.
Each leaf interpolates from its previous gain at parameter-slot endpoints.
Gain histories initialize/reset to unity, as the original decoder initializes
prevGainAT, rather than zero. Unsplit roots bypass splitting. Fixed base-channel
gains remain per source. Frame validation and local history updates keep errors
transactional. Partial-band splits and sparse speaker-position layouts remain
explicitly unqualified; no behavior is invented for them.

The reference generator compiles unmodified `sac_arbitraryTree.c` from the
pinned official ISO reference archive in the external cache. The authored
harness is reference-only. No ISO source or media is copied into the repository.
The adjacent reference JSON records archive, algorithm, initialization-source,
harness, generator, input and output SHA256 identities. Repeating reference
generation produced identical output bytes.

The saved component corpus has42 programmes,126 blocks, all seven parameter
grids and32/64/128-bin QMF banks, both dense output orders, all eight base-tree
fixed-gain schedules, simple/nested/balanced splits and multiple roots. Ordinary
tests read saved data, never compile C, fetch network data or invoke FFmpeg.
The source values and CLD parameters are synthetic and authored.

Four tests passed with no failures or ignored tests. The owned f64 calculation
matched478656 scalar C float outputs with maximum absolute error
4.4335261017991456e-8; the asserted tolerance is2e-7*(1+abs(reference)), allowing
original single-precision rounding. A separate analytic balanced-tree test
checks unity-initialized interpolation and steady-state energy conservation.
Clone/replay, reset, malformed inputs and error rollback are also exercised.

The authored three-AU `tree7-extra-ott` MP4 and Matroska reproduce the existing
specific production refusal after the SSC and all three full spatial frames
have parsed successfully. This is a passing refusal reproduction, not intended
playback acceptance. Promote it when the component is integrated. Its new OTT
parameters are authored, not taken from private or ISO streams. Re-running the
authored generator with subprocess/socket creation forbidden reproduced all8
files byte-for-byte, including the saved oracle files it leaves untouched.

## Required integration, still open

- Apply routing after temporal shaping and the dry/wet sum, before synthesis.
  Account for the reference fixed source gains once, avoiding the existing PCM
  stage applying them again to additional leaf channels.
- Carry output positions/count through PCM synthesis, native frame metadata,
  session queues and playback channel layout; do not guess sparse speaker maps.
- Retain/reset/reconfigure gain histories and synthesis channels consistently;
  account for them in decoder memory footprints and checkpoints.
- Replace the refusal with waveform acceptance for MP4/Matroska, both
  decorrelation modes, shared LD-SBR QMF, timeline spans, rewind and seek.
- Qualify partial-band/LFE trees and sparse output-position semantics before
  enabling those configurations. Nonzero energy quantization remains separate.

This is a necessary component milestone, not full LD-MPS or codec-gap completion.
