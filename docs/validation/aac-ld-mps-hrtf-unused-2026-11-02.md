# Unused HRTF metadata in full-output OTT trees

The native AAC LD-MPS assembler now accepts valid HRTF metadata for full-output trees 0/1/5/6. These OTT-only multichannel paths do not use stereo HRTF projection. Metadata shape and values remain validated. Tree 7 and active unqualified unequal HRTF grids retain explicit refusals. This implements admission without changing PCM; it does not implement binaural rendering or qualify active grid conversion.

## Reproduction and acceptance

Before the fix, the existing player rejected `plain32-tree0-b23-hrtf4.mp4` specifically with the HRTF matrix-composition refusal, while its no-HRTF control decoded. The pinned executable and outputs are retained in the adjacent JSON.

The own synthetic corpus contains 1,512 programmes and 168 controls in MP4/video and Matroska: four trees, six PCM/shared-LD-SBR domains, all 49 spatial/HRTF grid pairs, metadata cycles and on/off/on. All 3,362 generated files were reproduced deterministically with subprocess and network creation disabled. Generation is explicit; ordinary tests use committed files without FFmpeg or network.

Acceptance compares complete PCM with no-HRTF controls, both containers, nine seek boundaries, rewind, packet clocks, checkpoint restore, truncated-packet retry and matrix interpolation. Invalid unused metadata still fails atomically. A separate component test checks joint ADG/HRTF history on 144 existing own streams; this is not an additional encoded combined-feature corpus.

## Independent source check

The unchanged original full-output 5151/5152/7571/7572 C routines produced 88,200 finite, byte-identical effective M1/M2 coefficients across 600 pairs with the 3D flag off/on. Original source is external; only the own harness and hash receipt are committed. This comparison covers unused metadata only, not the binaural or active TTT projection algorithms. See the reference JSON.

## Device output and limits

Twelve representative MP4/Matroska device runs completed all 48 playback/navigation drain phases. The device receipt pins the executable and captures every output. This verifies delivery to the default audio device, not subjective sound quality or speaker wiring.

Focused test results and log hashes are recorded in the adjacent JSON. This is a scoped LD-MPS milestone, not full codec/profile completion or a full-worktree qualification. Active unequal HRTF conversion and other documented codec gaps remain open.

The focused gate completed successfully: 27 tests (3D: 9, all-grids: 6, priority: 5, unused: 7), zero failures. The joint-history test is included in these 27, not counted twice.
