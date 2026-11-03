# Standalone camera lock: zero-count CI repair

The existing Linux gate fails before any media tests: cargo check --locked --manifest-path crates/fvid-camera-ffi/Cargo.toml cannot use its stale lock. The previous published main92f56be10a already fails on this exact step; local locked/offline reproduction exits101. Add only the already-required miniz_oxide0.8.9 and adler2 packages plus the missing dependency edge; all existing package versions/checksums stay unchanged. Do not remove --locked from CI.

Locked/offline native checks and repeat pass; all4 existing ABI tests plus1 doctest pass. The Windows bridge cross-check also passes. No camera/media code, codec, interface, fixture, or existing test changes. This fixes baseline integration and therefore adds no accepted improvement ID. Graph delta and source/lock hashes are recorded; remote overall CI still needs its later independent failures handled.
