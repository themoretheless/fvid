# Offline playback fixture regeneration

Run `python3 scripts/generate_playback_error_samples.py` to regenerate the
28-file MP4/MOV/WebM/PCM regression corpus. Use `--output-dir PATH` for review
without changing checked-in fixtures. Generation uses only Python's standard
library and never invokes FFmpeg, libopus, another encoder or the network.

`synthetic-playback-seeds.zip` preserves the existing synthetic control clips
and independently produced Opus PCM references. These are the original public
96x64 test patterns and tones, not private recordings or parameter sets. This
is seed-based reproduction, not a new FVid encoding implementation: encoded
controls and reference PCM remain fixed; MP4 edit lists, sample entries and
composition timestamps are constructed by the generator itself. Keeping fixed
seeds preserves the original failures and packet/pixel/PCM expectations across
machines, including the explicitly forced Opus hybrid mode.

The seed JSON records SHA-256 for every archived file. All seeds are validated
before output creation. The generated JSON pins all 28 outputs including
metadata mutations. Run `python3 scripts/test_seeded_playback_generator.py`
explicitly to verify exact reproduction and corruption rejection. Ordinary
Cargo acceptance tests consume existing fixtures and never run the generator.

The prior FFmpeg/libopus-based generator was replaced; reference provenance is
retained here rather than presenting reference PCM as independently computed
by FVid. Other fixture generators/validation workflows remain separate work.

The Y4M header grammar, bounded line reader and container probe are now owned by
`fvid-media::owned_y4m` / `owned_y4m_probe`. Frontend compatibility uses the same
parser source body and delegates metadata inspection; it does not introduce a
second independent grammar. Public library probing selects the owned route for
progressive 8-bit and 9/10/12/14/16-bit 420/422/444. Legacy-enabled probing retains
its prior backend for unsupported chroma/interlace modes.

`tests/native_y4m_library_probe.rs` checks 13 chroma/depth forms, frontend/library
metadata equality, reduced fractional frame rates, tagged frame markers, bounded
header lines and truncated payloads. Synthetic frame payloads are constructed in
the test itself; no private media, external codec or fixture generator is used.
Core Y4M processing and root/domain API compatibility tests remain enabled.

`fvid_media::decode_video` now selects owned Y4M raw decode-and-discard for
this grammar, including when legacy support is enabled. It consumes every
sample byte through 8 KiB scratch storage, reports frame geometry/pixel format
and rejects incomplete payloads; metadata-only seeks are not counted as decode.
The frame-rate parser is shared with owned probing, including positive rational
validation, reduction and the existing omitted-rate default. Public integration
tests require the owned backend for all 13 chroma/depth variants and additionally
exercise three-byte reads and truncated-tail refusal on the synthetic WAVE-probe
Y4M control. Additional video transforms and other containers remain migration work.

The library's Y4M decode-and-discard now implements crop, horizontal/vertical
reflection and a half-open presentation interval. Each sample plane is processed
without RGB conversion; 10/16-bit horizontal reflection preserves sample byte
order. Input/output buffers are reused between transformed frames. Identity and
interval-only reads retain bounded 8 KiB scratch rather than full frame buffers.
Interval selection uses the original rational frame clock and stops before the
first unrequested header, so an unrequested damaged tail is not consumed.

`tests/native_y4m_library_transform.rs` compares 72 chroma/depth/crop/reflection
combinations with frontend CPU (8-bit) and sample-plane geometry (10/16-bit).
Known 8/16-bit pixel matrices independently check all three planes. The existing
three-frame synthetic WAVE-probe Y4M checks exact interval boundaries, cropped
output metadata, full-source refusal versus limited-range tail acceptance, and
explicit rejection of unsupported owned options. Legacy dispatch retains those
other options rather than silently ignoring them. Derived structural equality
on transformation requests makes future non-default options reject this route
until their implementation is added. No FFmpeg or network is invoked by tests.

Y4M decode-and-discard also owns nearest-sample scaling after crop/reflections.
It uses the existing media path's 16.16 point-sampling clock and preserves planar
sample depth and chroma layout. Crop and scale are fused into one output buffer;
no RGB or intermediate cropped frame is allocated. Output dimensions retain the
existing media API's even-size, 8192x4320 validation. The transform integration
suite compares 108 scaling/chroma/depth/reflection combinations against the
frontend media geometry, plus an independent known-pixel matrix and the public
library dispatcher. Invalid dimensions are explicit failures. These tests invoke
neither FFmpeg nor the network; other unsupported transforms remain migration
work rather than being silently dropped.

Y4M probe's independent ffprobe comparison is exclusively an explicit benchmark:
`FVID_REFERENCE_FFPROBE=/path/to/ffprobe cargo bench --manifest-path
crates/fvid-media/Cargo.toml --bench ffmpeg_y4m_probe_reference --offline`.
It creates its own tiny synthetic Y4M inputs at runtime and removes them on exit.
Ordinary `native_y4m_probe` tests validate the same 15 frequency/frame-count
combinations using exact integer timestamp expectations without external tools.

Owned Y4M decode now handles all four transpose modes after crop/reflections and
before point scaling. The single output buffer keeps 8/10/16-bit sample bytes
intact. Transposing 4:2:2 swaps chroma axes and reports 4:4:0 output rather than
silently relabeling the samples. The integration suite compares 288 compositions
against existing sample-plane geometry and independently checks four known
three-plane matrices, swapped dimensions, output chroma and public dispatch.
Existing synthetic Y4M controls are reused; no private samples or FFmpeg are used.

The owned Y4M geometry route supports black padding after transpose and before
point scaling. Padding reads the full/limited-range header tag, uses the media
adapter's depth-scaled black/neutral samples and retains swapped chroma axes.
The canvas must satisfy the existing public PadRect size/alignment/bounds rules.
Padding, crop, reflections, transpose and resize are fused into the reused output
buffer. Tests compare 144 compositions with media sample-plane geometry, check
known full/limited fill samples at 8/10/16 bits, invalid canvases and public decode
metadata. No external codec or fixture generator runs during these tests.

Negate, average blur and box blur external comparisons now live exclusively in
explicit root benchmarks: `ffmpeg_negate_reference`, `ffmpeg_avgblur_reference`
and `ffmpeg_boxblur_reference` (harness=false). Set FVID_REFERENCE_FFMPEG and run
cargo bench for the chosen target. They preserve the existing synthetic input,
layout/depth and option matrices. Ordinary native filter tests keep known-pixel,
parameter/storage, CLI/backend and interval checks without external executables.

Negate's sample-buffer implementation is owned by fvid-media and shared by the
frontend pixel pipeline and Y4M library decoder. It runs after geometry, preserves
8..16-bit sample maxima and validates high-depth storage before mutation. The
Y4M integration tests cover 54 layout/depth/option combinations, geometry order,
malformed-storage atomicity and public dispatch. Unsupported options still reject
owned admission. The previous generic unsupported-option test now uses unsharp,
as negate has an acceptance path rather than a refusal expectation.

Average blur and box blur now share library-owned implementations with the
frontend, using a common GeometryFrame storage type. Owned Y4M decode applies
these filters after geometry in the existing avgblur -> boxblur -> negate order.
The integration suite checks nine depth/filter compositions and public dispatch;
existing standalone filter tests retain independent known-pixel and direct-sum
expectations, invalid-storage atomicity, parameter validation and CLI intervals.
Explicit reference benchmarks preserve their complete external comparison matrix.

Pixelize and chromashift implementations are likewise library-owned and included
by the frontend, preserving its error API. The owned Y4M path applies negate,
pixelize, then chromashift after geometry, matching the existing pixel pipeline.
Twenty-seven depth/layout/filter compositions and public dispatch checks cover
integration. Existing standalone known-pixel, boundary and invalid-storage tests
continue to exercise the shared kernels; expressions are not silently admitted.

External pixelize and chromashift pixel comparisons now run only through the
explicit `ffmpeg_pixelize_reference` and `ffmpeg_chromashift_reference` benchmarks
with FVID_REFERENCE_FFMPEG. All 65 pixelize and 92 chromashift reference cases
retain their synthetic input, layout/depth and option matrices. Ordinary tests
retain known-pixel/edge expectations and decoder/CLI/invalid-storage checks
without an external executable, including when explicitly running ignored tests.

Owned Y4M decode now admits shuffleplanes and runs the existing library kernel
last in the pixel filter pipeline. Cross-size mappings point-expand to 4:4:4;
compatible chroma mappings retain original sampling (including transposed axes).
DecodeStats reports the resulting pixel format even for an empty interval. Tests
cover all 243 mapping/layout/depth combinations, frontend pipeline agreement,
independent known promotion pixels and public dispatch. Existing committed
shuffleplane synthetic controls continue to validate the underlying kernel.

The five owned gradient kernels (Sobel, Prewitt, Roberts, Kirsch, Scharr) now live
in fvid-media and are shared with the frontend. The Y4M decoder executes them
after negate and before pixelize/chroma/shuffle, in public option order. Tests
compare 54 depth/layout/compositions, including the five-filter chain, against
the frontend pipeline and exercise each public decoder option. Existing gradient
tests retain independent known-pixel, rounding and atomic-storage checks. Numeric
literal options are admitted; unsupported expressions retain explicit refusal.

Dilation and erosion now share library-owned kernels with the frontend. The
owned Y4M route applies them after pixelize and before chroma shift/shuffle, in
canonical dilation -> erosion order. Twenty-seven depth/layout/compositions and
both public decoder options exercise integration. Existing morphology tests
retain known threshold/edge pixels and invalid-storage atomicity checks; ordinary
execution requires neither FFmpeg nor regenerated fixtures.
