# fvid-media

Optional Rust adapter to native FFmpeg libraries. Implements file probing, reference-counted packet remuxing/selection, strict-boundary trim/concat, and decoded-frame crop views with FFV1 lossless encoding. Does not spawn FFmpeg CLI processes.

Build with FFmpeg development headers/libraries and libclang; set `FVID_FFMPEG_PREFIX` when they are outside `/opt/homebrew` on Apple Silicon or `/usr` elsewhere. Only macOS/FFmpeg 9.0.1 is currently qualified for this adapter. Unsafe is confined to generated FFI and native ownership/API boundaries in this crate; the root Fvid crate still forbids unsafe.

Full contracts, limitations and validation commands: [docs/MEDIA.md](../../docs/MEDIA.md). Available library codecs/filters are an inventory, not a promise that all are supported by the Fvid workflow API.

MIT applies to this adapter's authored code. The linked FFmpeg build and codec dependencies retain their own licenses.
