# Fvid-media

FVid media operations. The default build uses owned implementations and needs no FFmpeg headers or libraries. The temporary `legacy-ffmpeg` feature retains operations that have not yet been migrated; it must not be treated as a completed dependency removal.

Owned WAVE operations include probing, remux/trim/concat, PCM export and DSP, loudness measurement, and linear `apply_loudnorm`. Linear normalization requires valid `measured_I`, `measured_TP`, `measured_LRA` and `measured_thresh`: the requested gain must fit both the true-peak ceiling and loudness-range target. It preserves rate, sample count, channel order and speaker mask, exporting float32 WAVE. Integer 8/16/24/32-bit and float32/64-bit sources are decoded without libav. Packet limits export a frame-aligned prefix; cancellation, progress, optional memory/RSS admission and atomic publication use the owned control path.

Dynamic `loudnorm`, measurement printing and other formats still need migration. The legacy entrypoint routes qualified linear WAVE requests to the owned implementation and keeps existing behavior for the remaining cases. Ordinary tests use analytic PCM and invoke no external reference process. Run the explicit comparison with `cargo bench -p fvid-media --bench ffmpeg_linear_loudnorm_reference` (optionally set `FVID_REFERENCE_FFMPEG`).

Build the legacy feature with FFmpeg development headers/libraries and libclang; set `FVID_FFMPEG_PREFIX` when they are outside `/opt/homebrew` on Apple Silicon, `/usr` on Linux, or `C:\ffmpeg-shared` on Windows. On Windows run `scripts/setup_ffmpeg_windows.ps1` and set `LIBCLANG_PATH` to LLVM `bin`. Qualified: macOS/FFmpeg 9.0.1 and Windows/FFmpeg 9.0.1 shared (BtbN). Unsafe is confined to generated FFI and native ownership/API boundaries in this crate; the root Fvid crate still forbids unsafe.

Full contracts, limitations and validation commands: [docs/MEDIA.md](../../docs/MEDIA.md). Available library codecs/filters are an inventory, not a promise that all are supported by the Fvid workflow API.

MIT applies to this adapter's authored code. The linked FFmpeg build and codec dependencies retain their own licenses.
