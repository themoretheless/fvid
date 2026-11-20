# Codec admission-route audit

The objective remains absence of codec gaps, including native ownership, complete playback and navigation. Searching for `unsupported` alone is insufficient to identify missing functionality: several restrictions belong to lower-level APIs or profile-invalid parameter combinations. This audit checks selected existing routes on commit d514924755; it adds no decoder feature and does not close the overall objective.

| Suspected gap | Authoritative current route | Validation performed |
|---|---|---|
| AV1 tile-list rejected by ordinary packet API | `Decoder::decode_tile_list` accepts externally anchored camera context; ordinary packets cannot supply that external context | `av1_tile_list`: 9 tests, including accepted pixels, high depths/chroma, lossy transforms, CDF contexts and explicit missing-context refusal |
| AVC redundant-only packet requires primary picture | That restriction belongs to `prepare`/`prepare_with_parameters`; the decoder uses `prepare_for_decode` and redundancy recovery before DPB mutation | `avc_redundant_recovery`, `avc_primary_recovery`, `avc_redundant_primary`: 16 tests, including recovery, primary preservation, rewind and seek |
| HEVC 3D depth chroma reconstruction refused | The tested 3D Main depth profile requires monochrome; invalid chroma is rejected during contextual SPS parsing before reconstruction | `hevc_3d_chroma_constraints`: 1 validation test, with a valid monochrome control and invalid chroma witnesses; not acceptance of chroma depth reconstruction |

All 26 tests passed, zero failures or ignored tests. Fixtures already exist and ordinary test execution needs no FFmpeg or network. The adjacent JSON pins commands, sources, test sources and the complete log hash. These are selected integration tests, not a whole repository run or proof of full profile conformance.

VP9 `show_existing` was also checked in source: the top-level decoder selects its stored reference before entering the coded-picture reconstruction API. The latter's refusal of an uncoded picture is not evidence of absent top-level support. No additional VP9 acceptance result is claimed by this audit.

Actual open LD-MPS restrictions remain visible in production: active unequal HRTF grids, unqualified energy-quantization combinations, arbitrary-tree coverage/layout/history cases, and replaced inherited-fragment source association. They require implementation and independent acceptance evidence; existing refusal tests do not close them. The original-C HRTF tail-dependence diagnostic further limits use of that function as a mapping oracle. This is not an exhaustive list of all possible codec/profile gaps.

The separate `fvid`/`fvid-media` library gate also completed; see `codec-library-audit-2026-11-02.json`. Its counts are separate from these 26 integration tests. No runtime source was changed during these checks.
