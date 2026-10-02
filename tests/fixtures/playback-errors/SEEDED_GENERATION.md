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

External edge-filter and morphology references are exclusively explicit root
benchmarks `ffmpeg_gradients_reference` and `ffmpeg_morphology_reference`, selected
with cargo bench and FVID_REFERENCE_FFMPEG. Their original synthetic matrices
retain all 90 gradient and 220 morphology comparisons (including RGB morphology).
Ordinary tests retain masks, thresholds, rounding, atomic validation, pipeline
order, API and CLI checks without starting FFmpeg, even under --ignored.

The completion gate `python3 scripts/check_native_dependencies.py --offline
--production` additionally audits root media, root media-cuda and library cuda-hw.
It checks every graph and reports all failures together. This gate currently
fails because all three activate legacy-ffmpeg; the default native graph check
alone is not evidence of complete production independence. The production gate
must pass after those implementations/features have been migrated.

The opaque overlay sample kernel is library-owned and shared with frontend
playback/filter code. Its library tests preserve mixed-depth range conversion,
RGB and chroma clipping, and invalid-source atomicity with known synthetic pixel
matrices. This exposes the compositor without libav; file-based overlay decode,
timing and frame scheduling still require a separate library migration and are
not claimed by the Y4M owned request admission.

Overlay export's conditional external decoder comparisons now run exclusively
in `ffmpeg_overlay_reference` (explicit cargo bench with FVID_REFERENCE_FFMPEG).
All eleven AVC/HEVC/VP9/AV1/Y4M controls and their output formats are preserved.
Ordinary overlay tests keep sample, stream/metadata, cancellation/publication,
geometry and CLI assertions and do not launch the reference executable even
when FVID_REFERENCE_FFMPEG is set to a nonexistent path.

Owned Y4M parsing/probing/decoding accepts vertical 4:4:0 chroma (including high
sample depth). The two-frame `y4m-vertical-chroma-10.y4m` is generated solely by
`scripts/generate_y4m440_fixture.py` from integer sample values, without external
codecs or private parameters. Acceptance checks exact transpose pixels and the
4:4:0 -> 4:2:2 metadata transition; library/frontend probe equivalence covers
additional 8/10/16-bit variants. Ordinary tests use the saved fixture directly.

Owned raw Y4M parsing/probing/decoding also accepts 8-bit 4:1:1 and 4:1:0.
Synthetic integer-plane tests check raw counts and known reflection pixels;
probe equivalence covers 4:1:1. Output geometry must align with chroma axes.
Quarter-turn 4:1:0 retains symmetric subsampling. Quarter-turn 4:1:1 requires a
separate pixel-format conversion and is explicitly refused by the owned route;
legacy admission retains that operation during migration rather than mislabeling
its output. Other supported transforms can use the native raw path.

FFV1 v1 all-intra range encoding is now library-owned, sharing its implementation
with frontend exports. Encoder integration tests call the public library API,
then mux/decode synthetic integer patterns to check losslessness across supported
sample depths and chroma layouts. Existing export tests retain frontend coverage.
This exposes the codec without libav; complete library file export/container
migration remains separate work and is not implied by the kernel move.

The FFV1 threaded-playback/seek regression accepts the retained packed planar
presentation path as well as RGB. For packed frames it independently transposes
each source plane, checks every 12-bit sample and 4:2:2 -> 4:4:0 axes, then verifies
CPU RGB against the original reader reference. Seek repeats those checks for the
selected timestamp/generation. The existing short five-frame synthetic clip is
encoded/muxed in memory by owned code; no external tools or private video are used.

The owned library's `encode_y4m` streaming API is covered by short synthetic
in-memory Y4M clips in `tests/ffv1_encoder.rs`. Three frames at 3:2 fps check
interval selection, absolute rational nanosecond timestamps and exact decoded
samples for supported 8/10/16-bit chroma layouts. A separate 4x2 ten-bit 4:2:2
clip checks independent clockwise sample indexing plus negate, 4:4:0 output,
sink-error propagation and a truncated-payload refusal before packet delivery.
Generation uses integer patterns, the owned encoder and decoder only. This API
emits packets to a caller-owned sink; it does not claim a completed library
Matroska/file-export migration or removal of the legacy backend.

The library's owned Matroska writer shares EBML serialization, packet validation,
error state and finalization with the frontend writer. The synthetic FFV1 tests
compare whole container bytes for one rationally timed packet, then export three
Y4M frames with interval selection and transpose, parse the result and verify
exact decoded planes and source timestamps. Empty/truncated input and sticky
packet failures have refusal tests. No private media or external generator is
used. This introduces writer-level FFV1 export; general codec admission, colour/
file metadata mapping and atomic file publication remain separate migrations.

Atomic library FFV1 file export uses the committed two-frame synthetic
`y4m-vertical-chroma-10.y4m`, generated separately by
`scripts/generate_y4m440_fixture.py`. Its acceptance test decodes all published
10-bit samples and checks completion only after the destination exists. It also
checks pre-cancel, cancellation after the first packet, an independently created
destination during progress, and truncation of the second frame: no overwrite,
no partial published file and no remaining temporary output. These checks never
invoke fixture generation or FFmpeg. The specialized video-only API does not
claim general legacy lossless API parity or general file metadata mapping.

The FFV1 external encoder/decoder comparisons now live exclusively in
`benches/ffmpeg_ffv1_reference.rs`, invoked explicitly through `cargo bench`
with `FVID_REFERENCE_FFMPEG`. This retains all depth/layout, independent
context/non-keyframe and AVC/HEVC-to-FFV1 reference cases. Ordinary FFV1 tests
use only owned code and committed synthetic fixtures; they contain no ignored
FFmpeg cases or external codec process invocation.

Owned Matroska video metadata serialization now validates and writes crop,
exact pixel aspect, colour codes/range, mastering display, content light and
rectangular rotation. The frontend delegates to that library serializer.
`library_ffv1_metadata_matches_frontend_bytes_and_refuses_invalid_tags` creates
one wholly synthetic FFV1 frame, compares complete container bytes for all four
rotations and parses colour/HDR/crop/aspect back. Invalid rotation, empty crop,
zero aspect, fractional content light and NaN chromaticity are refused before
any output bytes. Y4M tag mapping and general file metadata are still separate
work at that stage; aspect/range mapping is covered by the subsequent fixture below.

`y4m-aspect-full-10.y4m` is the same two-frame synthetic ten-bit control with
A16:15 and FULL range tags. Its generator remains separate from ordinary tests.
Owned FFV1 export now maps those declarations to Matroska, preserving exact
samples and rational pixel aspect through crop, transpose, pad and scale.
Seven geometry combinations check known fractions; invalid/duplicate aspect
and colour-range tags are refused before writing output. The native converter's
BT.601 matrix assumption is retained; primaries, transfer and HDR are not
inferred from Y4M. General file tags and unsupported extensions remain outside
this mapping.
