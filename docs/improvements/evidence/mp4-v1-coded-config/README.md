# Version1 coded-audio configuration offsets

Candidate FVID-0544 / batch555 is not accepted. Basea53758a846 reads ALAC and EAC3 configuration atoms at byte28 even when the recognized QuickTime sound-entry version is1 and requires an additional16-byte prefix. Fourteen short v1 companions preserve all version0 packets/configuration, three ALAC clocks and two EAC3 clocks, with normal/unspecified packet fields and optional dec3 packet-probe controls. Independent ffmpeg-full9.0.2 decodes14/14 v0/v1 pairs identically, with correct output frame counts.

Generator is pure and separate. Eight short-header companions distinguish specific version1 header truncation from a missing cookie or an unrelated atom-extent error. Actual intended successful playback and refusal tests remain separate.

Initial stand-authoring logs are not project-defect evidence: ALAC-specific owned_mp4_audio rejects EAC3 by contract, and the root generic audio factory requires player. The corrected stand uses the existing native EAC3 packet decoder under player (native-sample timing for elementary PCM; container packet clocks compared separately) and owned ALAC export. It must establish valid v0 controls before the candidate is changed. No old tests, codec algorithm or unsupported-version policy may be weakened.

Publication checkpoint: targeted version1 tests and related MP4 audio clock, reconfiguration and enda regressions pass. This is a preserved, tested parser fix, not an accepted improvement: native playback-worker qualification remains open and the registry stays at543. The raw-EAC3 priming authoring failure is also a test-authoring diagnostic, not a project defect.
