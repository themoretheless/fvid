# Native library baseline after LD-MPS work

On runtime source commit d514924755, the offline no-default-features/player library gate completed successfully: fvid 1,090 passed / 26 ignored; fvid-media 596 passed / 3 ignored; zero failures. The long fvid run includes complete audio-worker replay and navigation over the own LD-SBR/LD-MPS corpora, including the signalled 3D TTT case. This validates the existing baseline; it does not implement the remaining unsupported combinations.

The adjacent JSON pins the exact command, executable hashes, complete output hash, Rust version and all 29 ignored tests with their reasons. The normal/build dependency tree and direct dynamic dependencies of both test executables contain no FFmpeg/libav entries. The source tree under src/crates and Cargo manifests/lockfile remained unchanged during the gate. Fixtures were not regenerated during test execution.

The skips must not be described as all environmental: `owned_exposure::integer_conversion_gap::complete_integer_float_rgb_pipeline_matches_reference` explicitly remains pending integer/float RGB compatibility acceptance. The other ignored reasons include physical-device/GPU and explicit loopback-network checks. No ignored test is counted as passed. No full-workspace/all-feature, hardware, 60-fps, benchmark or complete codec/profile conformance claim is made.

A separate admission-route audit passed 26 integration tests (AV1 camera tile-list, AVC primary/redundant recovery and HEVC depth chroma constraints). See `codec-admission-route-audit-2026-11-02.md`. Together the two commands have 1,712 passing tests, with the 29 skips belonging only to the library gate; that sum is a test-execution count, not a count of supported codecs or closed gaps.

Active unequal-grid HRTF mapping and the other current LD-MPS restrictions documented in the admission audit remain open. Source research continues using free sources only, as requested by the user. No standard purchase was authorized or performed.
