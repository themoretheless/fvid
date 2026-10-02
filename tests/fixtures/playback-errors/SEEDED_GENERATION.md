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

The existing library `transcode_lossless` API now dispatches qualified Y4M
geometry/filter exports through owned FFV1/Matroska code. Without the legacy
feature the same API is exported directly. Structural admission checks every
transform field, so unsupported options are never silently dropped; unsupported
legacy requests retain their previous backend during migration. At that stage intervals/seek,
metadata editing and allocation/RSS budgets were not qualified by this adapter.
The public API regression uses the committed synthetic aspect/range fixture,
checks complete output against owned writer bytes, and verifies that a one-byte
encoded-packet limit refuses publication and cleans the temporary output.

Owned public lossless export now enforces `max_packets` on source Y4M frames.
The same committed two-frame synthetic fixture checks limits zero, one, two
and three, progress counts, publication only when the full input fits, and
cleanup after rejection. A truncated second frame with a one-packet limit
must report the count limit rather than a truncation error, proving rejection
before reading/encoding that frame's payload. Ordinary decode without a limit
retains its existing bounded streaming behavior.

The public `crop_lossless` API is also exported without the legacy feature and
uses the same owned lossless adapter. Its committed synthetic 4:4:0 ten-bit
fixture is cropped at x=1: the test independently lists every selected sample
in Y, Cb and Cr, decodes the published FFV1 packets and checks exact precision,
chroma axes, aspect and FULL range. An unaligned vertical crop is refused
without publication. No external fixture generation is part of the test.

The subsequent owned public lossless interval path follows the existing API:
boundaries must be exact in the video's rational time base, PTS are rebased
by the interval start and input packet limits include pre-interval frames.
`y4m-interval-25-10.y4m` is a separately generated two-frame synthetic 25 fps
control. Selecting 40–80 ms verifies the second frame's exact ten-bit planes,
PTS zero, 40 ms duration and two consumed input frames. A one-packet budget,
non-exact/negative/empty interval and interval beyond EOF refuse publication
and clean temporary output. Writer-level streaming APIs retain their original
absolute source timeline; `seek=true` remains unqualified pending real seeking.

FileTags and Matroska chapter values now belong to the library and are
re-exported through the existing frontend paths. The file metadata serializer
is shared, preserving its validation and byte layout. The library FFV1 writer
accepts caller-supplied file tags/chapters before emitting any output. A short
synthetic frame compares complete container bytes with the frontend, parses
all thirteen tag fields and Unicode chapter titles/boundaries back, and checks
NUL/reversed-boundary refusals. This owns writer metadata; source-container
inspection and public metadata-edit policy migration remain separate work.

Public owned Y4M lossless export now applies known container tag edits:
deletions first, then ordered sets with last-value wins (including clearing by
an empty value). The committed synthetic fixture checks Unicode title, artist
clearing, track/album-artist aliases and reader roundtrip. Unknown keys, NUL and
more than 64 edits are explicit owned-path refusals before publication. Legacy
admission retains unsupported key/stream-tag operations on their existing path;
this is not an assertion that all metadata policies or source formats migrated.
The parser's first-value `FileTags::insert` semantics remain unchanged; explicit
editing uses the separate `set` method.

External Matroska rotation/Y4M, HDR/MP4 colour and file tags/chapters comparisons
now live in `benches/ffmpeg_matroska_metadata_reference.rs`, invoked explicitly
through `cargo bench` with both `FVID_REFERENCE_FFMPEG` and
`FVID_REFERENCE_FFPROBE`. All three original comparison bodies are retained.
Ordinary `native_matroska_metadata` tests use owned readers/writers and
synthetic controls only; no ignored external-tool comparisons remain there.

Explicit `transcode` with encoder `ffv1` and no encoder options now uses the
owned path for admitted Y4M transforms/policies and is available without the
legacy feature. Its public API regression compares the entire result with
owned `transcode_lossless`. Explicit unsupported encoder settings (such as
level 3) are refused by the owned adapter, while legacy admission retains those
requests on their previous backend; no encoder option is silently ignored.

The complete FFV1 v0/v1 decoder core now belongs to `fvid-media`, shared with
the frontend through its typed-error wrapper. `tests/ffv1_encoder.rs` consumes
the public library decoder directly for all depth/chroma roundtrips, malformed
packet recovery, storage admission and exported planar-frame checks. Existing
native playback tests retain frontend coverage. The explicit FFV1 benchmark
retains independent coded contexts/non-keyframes and encoder/decoder references.
No decoding algorithm changed in this ownership move; FFV1 versions/tools not
supported previously are not claimed supported by relocating the core.

### Library EBML framing ownership

The native Matroska reader and `fvid-media::owned_ebml` now include the same
bounded EBML framing and scalar implementation. This is an ownership change,
not a complete library Matroska demuxer: track parsing and indexing remain in
the native reader. In-memory synthetic controls cover offsets, element/value
limits, malformed and truncated headers, parent bounds and unknown sizes.
Existing native Matroska metadata/video tests verify reader compatibility;
ordinary checks use neither FFmpeg nor network access.

The library also owns the unchanged lenient file-tag and flat chapter-atom
reader, shared with the frontend. A synthetic in-memory Tags/Chapters control
checks the public API; full track/index parsing still remains in the frontend.

### Library WebM/Matroska reader

`fvid-media::owned_webm::WebmReader` now owns the complete bounded incremental
container reader, including tracks, packet indexing, clocks, colour/HDR and
file metadata. The frontend includes the same unchanged reader body with its
existing colour/error types. The library API has a synthetic in-memory VP9
packet-index control; this checks container framing, not VP9 decoding.
The frontend reader unit and native Matroska playback regressions remain active.
Unsupported lacing and other existing reader refusals are unchanged.

The owned probe dispatcher now accepts WebM/Matroska signatures and format hints.
A synthetic container written with the library muxer verifies tracks, packet
start timestamps, file title and chapter boundaries. Its opaque packet is not a
codec acceptance fixture. Codec profiles, pixel formats, stream durations and
frame rates are left unknown rather than inferred by this container-only probe.
An absent chapter end is represented as a zero-length chapter point because the
shared ChapterInfo API requires an end. Legacy probing remains until parity is
verified; no old metadata functionality is removed by this addition.

### Library codec configuration and HEVC framing

AVC/HEVC configuration records, ESDS extraction and length-prefixed NAL walking
now share unchanged parser bodies between the frontend and `fvid-media`.
AAC configuration continues to use the existing shared owned AAC parser.
HEVC header and bounded RBSP framing also share their unchanged implementation.
A synthetic library API control covers empty-record refusals, AAC LC parameters
and valid/truncated NAL lengths; existing frontend parser tests remain active.
This transfers ownership, not support for additional codec profiles or tools.

### Library multitrack Matroska muxing

The library now owns the existing validated track descriptions, entry preparation
and multitrack constructors for AVC/HEVC/AAC/FFV1/PCM/ASS/Opus. These bodies and
Opus transport framing are shared with the frontend. Synthetic FFV1/PCM opaque
packets verify byte-identical frontend/library output, parsed track metadata and
payload preservation; invalid AVC setup must refuse before writing output.
This container-only control is not a codec acceptance test. Existing native
metadata/video regressions remain active. File remux dispatch is not yet switched.

### Library Matroska identity copy

The validated container-preserving copy implementation is shared with the
library. It retains every source EBML byte, including unknown metadata and
attachments; it does not reconstruct or select tracks. Existing identity-copy
fixtures now also exercise the library's byte and packet-count results. The
synthetic multitrack control checks exact output, payload counts and rejection
of video input for audio-only output before writing. Caller-owned publication
and final completion notification remain separate from streaming copy.

### Library atomic Matroska remux

The public remux API selects owned byte-preserving Matroska identity copy for
.mkv/.mka output and unedited all-track requests in both feature modes. It
validates before creating private temporary output, flushes/syncs, publishes
without overwriting and emits completion only after publication. The synthetic
multitrack test covers exact bytes/counts, no overwrite, pre-cancel, cancellation
during copy, audio-only refusal and removal of temporary files. Selection, tag
editing, packet-count caps and memory/RSS policies remain unsupported by this
adapter; legacy mode retains its existing route for those requests.

Owned remux planning now dispatches Matroska identity requests in both feature
modes. It validates the index and encoded packet-size limit without creating
output or emitting execution progress, describes every track and retains all
container metadata/attachments. Synthetic tests check the packet/payload summary
and refusal of a too-small packet budget. Because the plan API has no destination,
.mka audio-only qualification and publication remain execution checks.

### Library ADTS/AAC Matroska mux and concatenation

Strict streaming ADTS muxing and compatible-segment concatenation now share the
frontend implementation with the library. Only import names and error conversion
change. A synthetic two-segment ADTS control checks unchanged raw packet payload
and nanosecond AAC clock; five existing channel/rate fixtures compare complete
library/frontend container bytes and native decoded PCM. This streaming API
leaves atomic file publication and final completion to the caller; it does not
provide a new codec profile or close aggregate AAC allocation budgeting.

### Library ADTS file remux

The public remux API now selects owned strict ADTS-to-Matroska export for .mka
or .mkv and unedited all-track requests in both feature modes. It shares atomic
publication with Matroska identity copy. A synthetic two-packet ADTS file checks
raw AAC payload, nanosecond timestamps, completion after publication, refusal
to overwrite, cancellation during copy and cleanup after a truncated packet.
No codec is invoked by this packet-copy test. Selection, metadata editing and
aggregate memory/count/RSS policies remain outside this adapter's admission.
ADTS planning is not yet routed to the owned file adapter.

ADTS remux planning now uses the same owned streaming packet reader and byte
limit as execution in both feature modes. It counts raw AAC packets/payload,
describes the AAC track and Matroska clock without output or execution progress.
The synthetic file test checks successful summary, matching too-small AAC payload
budget refusals in plan/execution and refusal of a truncated final packet.

### Explicit Matroska video reference benchmark

The two ignored external video/AAC and invisible-reference comparisons moved
unchanged to `cargo bench --bench ffmpeg_matroska_video_reference`. Only this
explicit benchmark requires FVID_REFERENCE_FFMPEG. The ordinary native Matroska
video suite retains its eight owned playback/seek/reference/poisoning checks and
no longer has external-tool tests or ignored reference comparisons.

The remaining ignored external Matroska AAC edit-boundary comparison moved
unchanged to `cargo bench --bench ffmpeg_matroska_aac_edit_reference`, explicitly
requiring FVID_REFERENCE_FFMPEG. Ordinary native mux tests retain all their
owned checks and no external-tool reference test. Two pre-existing native mux
failures (concat CLI container bytes and codec-delay/padding PCM) remain active
and unresolved; selected validation excludes them explicitly, not as proof of
the entire suite passing.

### ADTS concat dispatch regression

`python3 scripts/generate_adts_concat_fixture.py` writes two tiny synthetic
AAC-LC mono-silence ADTS segments using only bit fields and Python. Ordinary
tests consume the checked-in bytes. Before the dispatch fix, the generic
Matroska concat fallback intercepted ADTS and CLI output became PCM; the
synthetic acceptance test specifically observed A_PCM/FLOAT/IEEE instead of
A_AAC. All-ADTS input now reaches AAC packet-copy, preserving exact container
bytes against the specialized export API. Mixed input keeps its prior fallback.
The earlier five-fixture concat regression now passes; the separate
codec-delay/padding PCM mismatch remains unresolved.

### Leading negative AAC DiscardPadding regression

The checked-in synthetic mono-silence ADTS segment also builds a two-block
Matroska control with 128 leading and 256 trailing discarded samples. Before
fixing the export decoder's origin it produced 1792 instead of 1664 samples:
leading discarded data was reinserted as silence. The acceptance test now checks
1664 samples. Only the first block's head padding beyond codec-delay trimming
rebases the presentation origin; subsequent timestamp gaps remain intact.
All native mux tests are now active and pass, including the old padding PCM
oracle. Existing gap/interval/overlap tests verify retained timeline behavior.
The explicit `ffmpeg_matroska_padding_reference` benchmark compares sample counts
for codec delay and negative head padding against an external decoder; ordinary
tests do not require it. Specification: Matroska DiscardPadding element,
https://www.matroska.org/technical/elements.html#DiscardPadding.

### Library ADTS file concat

The public concat API now selects owned compatible ADTS-to-Matroska concatenation
in both feature modes, retaining WAVE dispatch for PCM files. It reuses strict
packet-size admission, streaming sequence framing and atomic no-overwrite
publication. The synthetic two-segment control compares entire output against
native AAC export, checks segment/packet counts and rejects a changed sample
rate without output or remaining temporary files. Track/tag edits and packet
count/memory/RSS policies still require further implementation. ADTS concat
planning is not yet connected to this library route.

ADTS concat planning now selects the owned library route in both feature modes.
It shares file/packet admission with execution and walks the complete compatible
sequence without output or execution progress. The synthetic concat control
checks all input paths, packet summary and identical configuration refusal in
plan/execution. Existing WAVE planning remains available through owned dispatch.

Explicit ADTS stream selection `[0]` now qualifies for owned remux, concat and
both plans, since ADTS exposes exactly one audio stream. Synthetic controls
compare complete selected/unselected output and assert owned backend dispatch.
Other indices and duplicate selection remain outside owned ADTS admission.
Matroska identity copy still requires all original tracks; its selection policy
is unchanged. Unknown metadata edits and memory/count/RSS policies still retain
legacy routing where enabled and require later implementation.

### Explicit subtitle reference benchmark

Four ignored external ASS/SRT/SubRip comparisons moved unchanged to
`cargo bench --bench ffmpeg_subtitle_reference`: ASS mux cue times/Unicode,
SRT text/style conversion, selection of externally muxed SubRip tracks and
font attributes. This explicit benchmark alone requires FVID_REFERENCE_FFMPEG.
The two ASS mux and four owned subtitle conversion tests remain ordinary tests
with no external invocation or ignored reference comparisons. The benchmark's
external SubRip mux is a reference workflow, not a fixture-generation dependency
for ordinary tests.

PCM Matroska AAC edits, ALAC and multichannel external sample comparisons run only in the explicit `ffmpeg_pcm_matroska_reference` benchmark. Ordinary PCM mux tests retain native sample, packet and timestamp checks without invoking FFmpeg.

The synthetic `adts-concat-a.aac` and `adts-concat-b.aac` fixtures also cover owned library packet limits: remux/concat plans and execution agree for limits 1, 2 and above EOF; concat uses a global limit; zero refuses without publication. An in-memory truncated ADTS tail verifies that a selected prefix does not consume later packets, while an unlimited plan rejects the same tail.

The mixed MP4/WAVE PCM concat external sample comparison runs in `ffmpeg_pcm_concat_reference`; ordinary concat tests retain native sample equality, CLI/API byte equality, plans and publication controls without invoking FFmpeg.

QuickTime PCM isolation/endian external comparisons and synthetic Matroska PCM external sample comparisons run only in `ffmpeg_pcm_formats_reference`. The ordinary synthetic Matroska test retains format, interval, mix, merge, plan and failure checks without FFmpeg. QuickTime external fixture isolation remains benchmark-only; this move does not add native MOV fixture generation.

The synthetic Matroska PCM signed-padding and gap external presentation-clock comparisons run in `ffmpeg_audio_timeline_reference`. Ordinary timeline tests retain exact padding/gap/interval sample checks and overlap publication refusal without FFmpeg.

External ALAC MP4 fixture and Matroska PCM comparisons run only in `ffmpeg_alac_reference`. Ordinary ALAC tests retain owned decoding, intervals, gain, mixing, damaged input and signed-padding checks without invoking FFmpeg.

Synthetic Y4M 420/422/444 external FFV1 pixel and filter-chain comparisons run only in `ffmpeg_y4m_lossless_reference`. Ordinary Y4M lossless tests retain native pixel, clock, aspect, CLI/API, filter and publication checks without FFmpeg.

External MP4 lossless-export video/copied-audio and spatial-transform comparisons run only in `ffmpeg_lossless_export_reference`. Ordinary lossless tests retain owned frame, timing, metadata, CLI/API, crop, plan and publication checks without invoking FFmpeg.

External MP4 concat AVC/HEVC video, AAC zero-delay/priming, mixed-track and independently generated no-PNS comparisons run only in `ffmpeg_mp4_concat_reference`. Ordinary concat tests retain native payload/frame/clock, PCM, CLI/API/plan and publication checks without FFmpeg.

External combined MP4-to-Matroska video/audio remux comparisons run only in `ffmpeg_mp4_remux_reference`. Ordinary remux tests retain owned frame/edit/metadata, dependencies/window, CLI/API/publication and rejection checks without FFmpeg.

External synthetic PCM amix normalization/weights and overlapping-layout amerge comparisons run only in `ffmpeg_audio_mix_reference`. Ordinary audio mix tests retain sample/channel order, AAC edits, CLI/API/plans and publication controls without FFmpeg.

The external AAC no-PNS encoding/reconstruction comparison runs only in `ffmpeg_aac_reconstruction_reference`. Its four rate/channel cases and numeric tolerances are preserved; ordinary AAC library/media tests continue using checked-in or synthetic inputs without external generation.

External WebM lossless VP9/AV1 pixels, rotation/filter and ten-bit crop/rotation comparisons run in `ffmpeg_webm_lossless_pixels_reference`. Their ordinary tests retain native frame, clock, geometry and metadata checks; audio-companion external comparisons remain separately pending migration.

External WebM AAC/Opus companion payload/timing/priming/PCM comparisons for VP9, AVC, HEVC and FFV1 video run only in `ffmpeg_webm_audio_companions_reference`. Their external synthetic audio generation is benchmark-only. The ordinary WebM lossless test file no longer calls FFmpeg or contains external ignored tests.
