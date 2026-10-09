# Audio Implementation Status

AAC Main: owned frequency-domain prediction is integrated with channel/CCE
history, checkpoint/reset and stereo tool ordering. Two mono MP4 fixtures
qualify 1024/960 samples. Fourteen stereo fixtures additionally qualify explicit
and full MS, independent channel window sequences/shapes, intensity polarity,
PNS transitions and correlated noise with sine/KBD windows. Two mono fixtures
exercise a nonzero twentieth-order TNS effect in both spectral directions.
Independent PCM oracles, export, rollback, checkpoint and playback seeks pass.
Explicit PCE bootstrap preserves Main/LC/SSR profiles in mono/stereo ADTS.
Six Main coupling cases qualify tags 1/15 at points 0/1/3 with unity gains
and no target TNS, including checkpoint, rewind and seek. Single-raw-block
ADTS CRC checks header bits, protected SCE/CPE/CCE/LFE prefixes, the second
CPE ICS and complete PCE/DSE, with prescribed zero padding. ADTS transport
frames with one to four raw blocks expose separate 1024-sample packets;
multiplexed protection uses a header CRC and a CRC per raw block. Wider Main
coupling, multichannel PCE and SBR/PS interactions still need qualification;
these fixtures do not establish complete AAC profile conformance.

Ownership status: `codec::aac_decoder` now uses FVid NativeAacDecoder for AAC
playback; the Symphonia AAC dependency/feature is removed. Numeric protocol
tables retain their MPL-2.0 provenance. Current AAC qualification is recorded in
`CODEC_PROGRESS.md`; the incremental implementation notes below describe earlier
states. AAC SSR now has native AOT 3 packet synthesis and headless MP4 acceptance
for eight original 24 kHz mono/stereo fixtures with active gain and window
transitions. Fourteen further videos qualify dependent and independent SSR
coupling, including simultaneous CCE tags 1/15. Five more coupled videos and
one independently switched stereo CPE video qualify unequal SSR window extents,
bounded PCM alignment, original packet timing, EOF drain, checkpoint replay,
rewind, seek and interval export. SSR SBR/PS and removal of independent CCEs
with queued history remain unsupported. New independent CCEs can appear while alignment is active: six
further MP4/Matroska pairs qualify first occurrence, canonical tag reordering,
source timing and preservation of prior filter/PCM histories. Four fixed-clock
MP4 variants
and a shortened final-packet fixture qualify reassembly into 1024 samples per
AAC access unit and final duration trimming. Four Matroska videos and one
stereo ADTS stream qualify full/interval root and owned export against the same
MP4 PCM. These are authored 24 kHz cases, not general profile qualification.
Malformed AAC packets now return errors and require decoder reset; they are
not reported as successful empty output. Out-of-range unsigned timestamps
are rejected before touching codec state instead of wrapping to negative PTS.

AAC-LC/SSR `GASpecificConfig.extensionFlag=1` is accepted when the following
`extensionFlag3` is zero, including after an explicit PCE. Eight ASC variants
qualify indexed LC/SSR, PCE coupling, SBR absent/present sync signalling,
explicit HE-AAC and explicit/sync PS. Their full PCM and headless playback are
identical to baseline videos; nonzero or missing `extensionFlag3` refuses.

## What works

| Source | Codec | Path | Verified |
| --- | --- | --- | --- |
| MP4 | AAC-LC (`mp4a`) | `playback_mp4_audio` → `codec::aac_decoder` → cpal | yes, headless |
| MP4 | AAC SSR (`mp4a`, AOT 3) | owned four-band synthesis → `playback_mp4_audio` | yes, synthetic mono/stereo, gain, seek/rewind/ranges |
| WebM | AAC (`A_AAC`) | `playback_webm_audio` → `playback_aac::esds_for` → `codec::aac_decoder` | yes, headless |
| WebM | Vorbis (`A_VORBIS`) | `playback_webm_audio` → symphonia Vorbis → cpal | yes, headless |
| WebM | Opus (`A_OPUS`) | — | not supported |
| WebM | MP3, MP2 (`A_MPEG/L3`, `A_MPEG/L2`) | `playback_webm_audio` → symphonia MP2 → cpal | yes, headless |
| WebM | FLAC, ALAC, PCM (`A_FLAC`, `A_ALAC`, `A_PCM/*`) | `playback_webm_audio` → own or symphonia decoder | yes, headless |
| MP4, WebM | AC-3 (`ac-3`, `A_AC3`) | `playback_mp4_audio` / `playback_webm_audio` → `codec::ac3_decoder` → cpal | yes, headless |
| either | E-AC-3 | `codec::eac3_decoder` reads headers only (gap 6) | not supported |

MP4/AAC was proven end to end on a generated tone file with
`cargo run --features player --example audio_probe -- file.mp4`, which reports
the codec, packet and frame counts and the decoded duration without opening a
window. Add `--play` to run the same file through the real decode thread against
a silent, modelled device, which is how the queue pacing below is measured.
WebM/Vorbis uses the same probe, and `playback_webm_audio::tests` decodes a 2 s
tone fixture (`tests/fixtures/audio/vorbis-stereo.webm`) through the demuxer,
header handling and decoder together: 88 packets in, 89 088 stereo frames out,
peak amplitude matching ffmpeg's own reading of the file.
Matroska's MPEG audio tags share symphonia's one MP2/MP3 decoder and one dispatch
arm, so `A_MPEG/L2` only had to join the container's tag list; `mp2-stereo.mkv`
checks the whole route (1152 samples per Layer II block, the decoded peak matching
ffmpeg's reading of the same stream).
AC-3 in the two containers is checked by `tests/native_container_audio.rs`. That
decoder also drops the 256 samples of leading padding its encoder puts in front
of the sound: measured against ffmpeg's reading of the same stream at 48 000,
44 100 and 32 000 Hz, the two align at exactly that shift and at no other.

## Pipeline

```
container reader ── AudioStream ──┐
                                  ├─ audio_thread::AudioPlayback (own thread)
codec ─────────── AudioDecode ────┤
                                  └─ audio::AudioBackend → device clock
```

Each container implements `audio::AudioStream` (metadata plus a rewindable,
seekable packet cursor), each codec implements `audio::AudioDecode`, and each
device implements `audio::AudioBackend`. The decode thread holds all three as
trait objects, so nothing in it knows about MP4 or AAC. `PlatformBackend`
selects the device implementation; `NullBackend` stands in for builds and
tests without audio hardware.

### Timestamps

Packet timestamps stay in the source's own units: the MP4 **track** timescale
(the sample rate, not the movie timescale, which is usually 1000) and
nanoseconds for WebM. `AudioStream::timescale` reports the unit,
`AudioPlayback::seek` converts seconds into it, and
`AudioStream::time_of` converts back. Mixing the movie timescale in here
quietly moved seeks ~44x too early on 44.1 kHz files;
`audio_thread::tests::seek_counts_ticks_in_the_stream_timescale` locks it down.

### Configuration data

- MP4: the `esds` box is not a codec configuration. `codec::config::aac_specific_config`
  unwraps it to the bare AudioSpecificConfig that symphonia parses, and that
  unwrap happens inside `AacDecoder::new`.
- WebM AAC: `A_AAC` names the same coding but keeps only the bare
  AudioSpecificConfig in `CodecPrivate` (five bytes for the 48 kHz stereo take
  under `tests/fixtures/audio/aac-stereo.mka`). `playback_aac::esds_for` wraps
  those bytes in the descriptors an MP4 sample entry would have carried, the
  track is then handed to the dispatch under `mp4a`, and the rebuilt box is fed
  back through `aac_specific_config` before it is accepted, so a config this
  reader wrapped wrongly is refused at open instead of two frames in.
- WebM: `CodecPrivate` holds the three Vorbis header packets. The comment
  packet is dropped for the raw-concatenation layout, and Xiph-laced payloads
  pass through untouched. ffmpeg writes the laced form, so the pass-through
  branch is what the fixture exercises.

Two metadata bugs had to go before any Vorbis file could open. `SamplingFrequency`
(0xB5) is an IEEE-754 float of four or eight bytes and the demuxer read it as an
unsigned integer, so a 44.1 kHz track reported a rate of 0; and `CodecPrivate` was
matched under element id 0x5A6E2A, which is nothing in Matroska, so the header
packets never reached the decoder. Both are now covered: `float` reads the
4- and 8-byte forms, `Duration` reuses it, and
`container::webm::tests::audio_track_reads_its_float_rate_and_codec_private`
pins the rate and the private bytes. A rate that is not finite, or zero, is an
error at open rather than a track that dies later.

### Clock and sync

The backend clock counts frames the device callback actually read, so it reports
what has been heard rather than what has decoded. It stops dead while the ring is
starved instead of counting the silence the hardware plays out over it, which is
the right behaviour for sync: a gap the decoder caused must not pull the picture
forward. A seek or rewind flushes the device buffer and re-anchors the clock at
the timestamp actually landed on, which can precede the requested point.

The clock is commanded, not implied. `AudioPlayback::start` builds the backend
and the decoder and then waits; `resume` is both the first start and every
restart, so `CpalBackend::start` builds the stream without playing it. The
player issues `play()` at the moment it stops buffering and starts presenting,
which anchors both clocks to the same instant. Before that, the audio thread
clocked from the moment its own thread existed, and the preroll showed up as a
fixed offset lasting the whole file — a cost the sync gate cannot pay back,
because it can only ever hold video still, never hand it time.

The decode thread paces itself against that clock: it stops queueing packets
once the decoded frontier is `LEAD` (200 ms) ahead of the device, waiting on the
command channel so a seek still lands immediately. Without it the thread decodes
at CPU speed into the backend's finite ring, which drops what does not fit, so
only the first 500 ms of a track would ever be heard.

`audio_probe --play` measures the pacing headless, against a modelled device
that clocks at wall rate and refuses to queue more than a ring:

```
wall=2.828s fed=3.042s heard=2.823s dropped=0.000s
lead=0.219s queue never overflowed
```

The whole 3.042 s track reached the queue and nothing was dropped. The lead is
220 ms rather than 200 ms because the frontier is measured at the end of the
last packet queued, one 1024-sample AAC frame (23 ms) past the point the wait
was computed. The same file decodes in 30 ms of wall time (`audio_probe` without
`--play`, so with no device clock in the loop), about 100x faster than the
device consumes it: unpaced, the thread fills a 0.5 s ring in milliseconds and
the rest of the track is discarded.

### Skew, measured

`examples/av_sync.rs` runs the real video thread and the real audio thread over
the same file, presents frames on the cadence the player uses, and records
`audio clock - frame PTS` for each one — the exact quantity the sync gate
compares. Against a modelled device whose clock runs continuously between pushes,
the anchored run looks like this:

```
frames=147 seek=0s period=40ms slack=0.050s
skew mean=-0.006s min=-0.024s max=+0.007s
drift=-0.0029s gated=0 frozen=3 heard=5.817s fed=6.037s dropped=0.000s
verdict=every frame inside the slack
```

The whole 6 s run stays inside ±25 ms, half the 50 ms budget, and the first and
last thirds of it differ by 3 ms, so the two tracks agree about rate. From a seek
the same holds (mean −10 ms, min −26 ms, max +3 ms over the last 3 s of the
track). `gated=0` means no frame would have been held back by the player's gate.

The same file before the clock was anchored measured `mean=+0.233s max=+0.260s`
with three frames gated — the preroll, charged to every frame of the file.

`frozen=3` counts frames that met a motionless audio clock. The decode thread
stops sampling the device when the track runs out, so the tail of a run reports
where the last queued samples left off while video still has frames to show. The
player is indifferent to the same reading because `audio_ended` opens the gate;
the instrument drops those samples instead, so the end of a file is not scored as
an offset.

VP9 video with a Vorbis track measures the same way, which is the point: the
instrument does not care which container either of the two streams came from.

```
frames=147 seek=0s period=40ms slack=0.050s
skew mean=-0.009s min=-0.041s max=+0.007s
drift=-0.0005s gated=0 frozen=3 heard=5.799s fed=6.014s dropped=0.000s
verdict=every frame inside the slack
```

The reading moves between runs of the same file — a second run of it came out
`mean=-0.007s min=-0.023s` — because the instrument's own thread scheduling is
part of what it measures. What stays fixed is the order of magnitude: both
tracks land inside the slack, and neither shows a rate difference.

Video presentation waits until the audio clock reaches the frame PTS minus
`player::AUDIO_SYNC_SLACK` (50 ms). The measured run sits at half that, so the
budget stays where it is rather than being tightened to the sample rate's own
granularity: the audio position only advances in whole device buffers, and an AAC
frame is already 23 ms of it. When audio ends, fails to start, or its
track is unsupported, `audio_ended` makes the gate pass so the picture keeps
moving on the video clock instead of freezing at the last audio position.
Audio-thread events are drained every frame, which also keeps a failed track
from filling the bounded event channel and blocking the decode thread.

## Gaps

1. Nothing here was measured on real hardware. Every number above comes from the
   modelled device in `examples/harness`, which reproduces the two properties the
   player relies on — a clock that advances at the sample rate, and one that
   stands still while its queue is empty — but not a host that reconfigures mid
   stream, resamples, or runs slightly fast or slow against the CPU. Drift over a
   long file is unobserved for the same reason: 6 s of generated tone agreeing to
   3 ms says nothing about an hour.
2. The queue depths are not weighed against what they cost. The 500 ms ring and
   the 200 ms decode lead set how far ahead of the heard position the decoder
   runs; neither has been compared with the pause-to-sound delay a listener
   notices, and both are fixed constants rather than a function of the device's
   own buffer size.
3. Opus needs a codec outside symphonia (0.6.1 ships none), so it is a
   dependency decision rather than an implementation task. The demuxer also has
   one more piece to learn first: an Opus track's `SamplingFrequency` is 8000 by
   mandate, and the rate to decode at lives in `OutputSamplingFrequency` (0x78B5)
   plus a channel mapping family in `CodecPrivate`, neither of which is parsed.
4. symphonia's AAC decoder accepts only AAC-LC, at most two channels and 1024
   samples per frame, so HE-AAC reports a decoder-initialisation error and
   plays video-only.
5. No pause-aware clock: pausing stops the device, and the video deadline is
   reset on resume rather than re-derived from the audio clock.
6. E-AC-3 reads its frames but not its audio. `codec::eac3_decoder` walks a
   syncframe's `syncinfo`, `bsi` and `audfrm` to the bit its first block starts
   at, and refuses by name the three frame shapes this route cannot place: the
   adaptive hybrid transform, blocks switched to the short transform, and an
   attenuated high band. What is missing is a block decoder — Table E2.10's
   exponent strategies, the band structures they name, bit allocation, mantissas,
   the 512-point long transform and the downmix. That is a decoder of the AC-3
   core's size, not a wiring task.

## Owned AAC synthesis kernel in progress

`codec::aac_imdct` implements inverse MDCT independently of Symphonia for
120, 128, 960 and 1024 coefficients. Its explicit windowed normalization is
2/N; windowing and overlap-add belong to the caller. A precomputed cosine
recurrence avoids transcendental calls in the coefficient loop and reuses the
caller's output, but the kernel is still quadratic and is not yet wired into
packet decoding. Do not count this as a working owned AAC decoder.

Tests compare basis vectors against direct cosine evaluation for all four
sizes, reconstruct an independently analysed signal using overlapping sine
windows, and verify invalid input leaves output untouched. Spectrum entropy
parsing, inverse quantization, AAC window transitions and codec tools still
need implementation/integration before replacing Symphonia.

Transform reference: Shao and Johnson, “Type-IV DCT, DST, and MDCT algorithms
with reduced numbers of arithmetic operations”, section VII:
https://arxiv.org/abs/0708.4399 . The current kernel is a direct recurrence,
not an implementation or performance claim for that paper's fast algorithm.

`codec::aac_synthesis::LongSineSynthesis` now owns the persistent overlap for
long and eight-short sine/KBD-window spectra (960 or 1024 coefficients per frame),
including long-start and long-stop transitions. Short spectra must already be
deinterleaved into eight consecutive windows by the packet decoder. It reuses
scratch/output buffers, validates input before changing overlap, and resets
past-frame contributions explicitly. Tests independently calculate forward
spectra of a multitone signal and recover interior PCM blocks within 1e-7;
error and reset behavior are also verified. Transition reconstruction is checked
for both supported frame sizes, including shape changes. KBD windows are generated
locally (alpha 4 for long, 6 for short) and checked against independent Bessel
integral quadrature. The previous shape is retained for the first half-window.
Packet integration remains pending;
playback continues to use Symphonia until that work is complete.

Owned `codec::aac_ics` now reads AAC-LC window/shape/max_sfb syntax, all
128 short-window grouping masks, and section codebook runs including escape
lengths. The caller supplies the selected band-table limits. Reserved values,
LC prediction, truncated input, empty sections and band overruns are rejected
without advancing the caller's bit cursor. This parser is not yet wired into
the playback decoder; band tables and spectral/scalefactor entropy decoding
remain prerequisites for replacing Symphonia.

`IcsInfo::deinterleave` now restores per-window spectra from group/band/window
ordering, zero-filling bands above max_sfb. It accepts decoded placeholders for
zero, noise and intensity bands; their actual reconstruction remains separate.
All 128 grouping patterns are checked for both frame sizes using independently
sorted coefficient coordinates. Invalid layouts leave output untouched.

`codec::aac_quant::inverse_quantize` reconstructs ordinary spectral bands using
signed magnitude to the 4/3 power and accumulated scalefactor scaling. Tests
cover every escape magnitude, quarter-step scale factors, exact cube/fourth-power
anchors, and transactional rejection of invalid input. Output remains in AAC
spectral units: PCM normalization, noise/intensity reconstruction, entropy
parsing and packet integration are still required.

Owned `codec::aac_pulse` reads cumulative long-window pulse offsets and applies
up to four signed corrections before inverse quantization. It handles repeated
positions and zero coefficients, validates all positions before committing the
bit cursor, and rejects short-window use. The inverse quantizer now accepts the
post-pulse maximum magnitude 8251 (8191 plus four amplitude-15 corrections).
Tests exercise pulse parsing through inverse quantization, including both signs
at the maximum magnitude. Full packet decoding remains pending.

Owned `codec::aac_huffman` now decodes all eleven spectral codebooks and the
scalefactor codebook, including unsigned signs and book-11 escape magnitudes.
Reads are transactional. The initial implementation scans codewords and is not
a throughput claim. Exhaustive tests cover codewords and spectral tuples.
Numeric protocol tables in `aac_huffman_tables.rs` are extracted from Symphonia
0.6.1 and retain its MPL-2.0 notice (see LICENSE-MPL-2.0); the FVid decoding
implementation is separate. This does not yet replace the playback decoder.

`codec::aac_scalefactors` connects section codebooks to the owned Huffman reader.
It retains separate spectral scale, noise energy and intensity position values,
including the first noise band's nine-bit delta and accumulation across groups.
Invalid ranges/codebooks/truncation leave the input cursor unchanged. Noise and
intensity reconstruction and full packet integration remain pending.

`codec::aac_spectral` connects section codebooks and grouped band geometry to
owned Huffman tuple decoding. It emits group/band/window-ordered quantized data
for the existing deinterleaver, checks tuple alignment, and consumes no spectral
bits for zero/noise/intensity bands. Special-band zero placeholders still need
noise/intensity reconstruction. Tests cover mixed groups through deinterleaving,
zero-only spectra, truncation and malformed layouts. Playback integration is
still pending.

`codec::aac_bands::BandTables` selects 1024/128 AAC-LC band geometry directly
from AudioSpecificConfig sample rate, including explicit-rate intervals, and
supplies limits to the owned ICS parser. Numeric tables retain their Symphonia
0.6.1 MPL-2.0 notice in `aac_band_tables.rs`. Tests check indexed rates, interval
boundaries, complete/aligned band coverage and ASC-to-ICS validation. The
960/120 tables are now derived by retaining the lower boundaries and ending at
the shorter transform size; packet-level reference checks are described below.

`codec::aac_channel::ChannelData` now joins global gain, ICS, sections,
scalefactors, pulse syntax and spectral decoding into a transactional individual
channel reader. A constructed nonzero channel payload is tested through owned
inverse quantization, deinterleaving and synthesis. This is not yet real-file
playback proof: common-window stereo, TNS, gain control, special reconstruction,
raw-data-block dispatch and output normalization remain unfinished.

`codec::aac_pair` reads independent/common-window channel pairs and all valid
mid/side mask modes. Both channel reads are one transaction. Tests cover masks,
truncation, reserved mode rejection, and all 13 real packets from
`aac-stereo.aac` through their END elements. This establishes complete syntax
coverage for this fixture, not decoded PCM correctness or general file support;
stereo reconstruction and the remaining tools still need integration.

`ChannelData::ordinary_spectrum` joins deinterleaving, optional long-window
pulse correction and per-band inverse quantization, yielding window-ordered
spectral units. Tests verify different scales across short-window groups and
band boundaries. Noise/intensity bands and pulse corrections above max_sfb are
explicitly unsupported here; they must not silently become zeros. This stage
still needs stereo tools, PCM normalization and playback integration.

`ChannelPair::ordinary_spectra` applies mid/side sum/difference reconstruction
to masked ordinary bands after per-channel inverse quantization. Tests verify
opposite masks in two short-window groups and preserve unmasked coefficients.
Noise/intensity remain explicitly rejected by this path; full stereo PCM
correctness has not yet been established.

Channel-pair reconstruction now includes intensity stereo (books 14/15).
The parser retains explicit versus all-band mid/side mode so only explicit
mask bits invert intensity polarity. Tests cover both books, both mask modes,
masked/unmasked bands and all short windows. Noise substitution, PCM reference
comparison and playback integration remain pending.

Owned `codec::aac_noise::NoiseState` generates normalized spectral noise bands
for decoded energy scalefactors. State cloning reproduces correlated shapes at
different energies; reset is deterministic. Tests cover every valid energy,
multiple widths including one coefficient, correlation and error state retention.
The generator still needs integration into channel/pair reconstruction; its
random sequence is not a bit-identical claim against other decoders.

`ChannelData::spectrum_with_noise` now applies owned PNS to each window/band
and commits random state only after successful reconstruction of the channel.
Tests verify band energies across eight short windows and state rollback after
a later invalid band. Correlated stereo PNS and final PCM validation remain
pending; the existing ordinary-only entry point still rejects noise explicitly.

`ChannelPair::spectra_with_noise` integrates stereo PNS, sharing the left noise
shape at the right energy for correlated bands and bypassing ordinary mid/side
on noise bands. Random state commits after both channels and stereo validation.
All 13 stereo-fixture packets now reconstruct finite full-size spectra. Tests
cover correlation, independence, energy and rollback. This is not yet a PCM
reference comparison; TNS and final playback integration remain pending.

`synthesize_pcm` now applies AAC PCM normalization (1/65536 after the existing
2/N mathematical IMDCT). All 13 stereo-fixture packets pass owned parsing,
spectral reconstruction and synthesis against a saved FFmpeg 9.0.2 float PCM
oracle. RMS error is 0.000135962 and peak error 0.00264216, with different PNS
sequences; this is not bit-exact. Fixture provenance and thresholds are recorded
beside the reference. No runtime/test invocation of FFmpeg is introduced. TNS,
remaining profiles/tools, public packet decoding and playback remain pending.

`codec::aac_native::NativeAacDecoder` now exposes owned raw AAC-LC packet-to-PCM
operation for mono/SCE and stereo/CPE configurations, plus reset. It supports
fill metadata and data-stream skipping, rejects unimplemented elements/tools,
and commits synthesis/noise state only after a whole block succeeds. Public-API
tests match the complete stereo PCM oracle, verify reset, and reject malformed
packets without altering subsequent output. Synthesis state is currently cloned
per packet for transactional behavior; no performance claim is made. The player
still uses the existing wrapper; TNS, multichannel and 960-sample packet support
are not complete.

The public owned decoder now also passes the full `aac-mono-44k.aac` fixture
against a saved FFmpeg 9.0.2 mono PCM oracle: RMS error 0.0000315593 and peak
0.000223995. Separate regression gates are 0.00004 and 0.0003. This verifies
the SCE/44.1-kHz path in addition to CPE/48-kHz; it does not establish coverage
for the still-unimplemented TNS, multichannel or 960-sample packet cases.

Owned `codec::aac_tns` parses LC TNS filters, converts signed/compressed
reflection coefficients into predictors, and applies directional spectral
filtering within clipped band intervals. Tests verify coefficient resolution,
zero-order/short-window syntax, transactional parsing, band clipping and exact
first-order impulse responses in both directions. Sample-rate TNS limits and
channel integration remain pending, so packet decoding still rejects TNS.

TNS is now parsed by `ChannelData` and applied by `NativeAacDecoder` after
stereo reconstruction, before synthesis, using rate-specific long/short band
limits. The owned AAC suite passed (52 tests before the additional packet test).
An active-TNS synthetic mono packet produces exactly the independently filtered
and synthesized PCM; mono/stereo saved-oracle tests still pass. Real-file TNS
coverage, 960-sample bands, multichannel and player replacement remain pending.

Real encoder TNS coverage is now verified with `aac-tns.aac`: the test requires
two active filters and compares every decoded sample to the saved FFmpeg 9.0.2
oracle. RMS error 1.5812e-8, peak 1.7882e-7; PNS disabled. Fixture generation is
documented alongside it. Multichannel, 960-sample bands and player replacement
remain incomplete.

Native AAC now accepts standard ordered element layouts for 3–6 channels,
including LFE, and maps AAC center-first syntax into PCM channel order. A new
5.1 fixture has distinct tones in all channels: per-channel RMS <7.5e-9 and
peak <4.5e-8 against the saved oracle. Duplicate element tags and unexpected
layout/order are rejected. 7.1, arbitrary reordered elements/PCE, 960-sample
packet support and player replacement remain pending.

## Headless native AAC media API

`container::adts` now owns ADTS framing/configuration independently of the
`player` feature. The playback adapter reexports its existing parser API.
`native_media::decode_aac_pcm` accepts ADTS bytes and a caller-owned writer,
producing interleaved f32 little-endian PCM with sample/frame statistics.
It uses NativeAacDecoder without Symphonia or FFmpeg. ADTS encoder priming and
padding are retained; writer/decode errors propagate and may leave partial
output in the caller's writer. The plain CLI path `fvid media decode-audio INPUT.aac OUTPUT.f32le` now uses
this API without the `media` feature. It writes raw interleaved f32le samples
through a temporary file and publishes only on successful decode, never
replacing existing destinations. Other output containers and transformed audio
operations still use the legacy implementation.

Plain ADTS `media decode-audio INPUT.aac OUTPUT.wav` now also uses the owned
path. Its WAVEFORMATEXTENSIBLE IEEE-float header carries a standard channel
mask (mono through 5.1), valid sample bits, and a `fact` sample-frame count.
PCM is streamed to temporary output and the RIFF header is finalized before
publication; outputs exceeding RIFF's 32-bit size are rejected. No FFmpeg
runtime is used. Validation covered exact six-channel sample preservation,
header sizes/mask/count, and no-overwrite; independent ffprobe inspection
reported pcm_f32le, 48000 Hz, 5.1, 11264 frames for the active surround fixture.
The independent inspector is a validation tool, not part of export.

Owned ADTS PCM/WAV export accepts `--from SECONDS --to SECONDS` and `--quiet`.
Boundaries select sample starts in a half-open interval and round up using
integer arithmetic at the source sample rate. Reference/overlap pre-roll is
decoded before selection; the exported PCM matches the corresponding slice of
full decoding exactly. Empty or reversed intervals fail. WAV fact/data sizes
reflect only the selected samples; decoded-frame statistics include pre-roll.
Example: `fvid media decode-audio input.aac clip.wav --from 0.030001 --to 0.070001`.

Native PCM/WAV CLI export also accepts MP4/M4A with a single AAC audio track.
The owned MP4 reader supplies packets and the owned AAC decoder applies media
edits to remove priming and trailing encoder padding. Current support requires
sample-aligned source timestamps; discontinuous packet timing is rejected.
The M4A fixture presents exactly 5645 mono samples at 44100 Hz; six native audio
integration tests pass, and a CLI-created WAV was independently inspected with
that exact sample count. Other MP4 audio codecs retain their previous routing.

Native MP4/M4A AAC export now accepts `--from`/`--to` relative to the edited
presentation timeline. Boundaries round up to sample starts, decoding pre-roll
for AAC state and clipping the requested endpoint to the media edit. Tests
compare exact slices after priming removal and check empty/reversed ranges.
CLI inspection confirmed 1764 samples at 44100 Hz for 0.030001–0.070001 seconds.

MP4 AAC timing now converts track ticks to sample indices with checked integer
arithmetic instead of requiring timescale == sample_rate. A regression doubles
mdhd/stts/elst media ticks while preserving the timeline and checks byte-identical
full and interval PCM. Fractional-sample timestamps remain explicitly rejected.

MP4 AAC edit scheduling now handles multiple/repeated media ranges and empty
edits (silence). Each selected media range resets the decoder and replays its
pre-roll, avoiding a whole-track PCM cache at the cost of extra decoding for
repeated edits. Cumulative movie boundaries are rounded up to sample indices
without per-edit duration drift. User intervals address this composed timeline.
Nine integration tests pass, including exact range/silence/repeat PCM and a
cross-edit interval; source edits beyond available audio fail explicitly.

Matroska AAC export preparation: the owned demuxer now retains track CodecDelay
and signed per-BlockGroup DiscardPadding. Both retain nanosecond units regardless
of TimestampScale; positive padding trims the block end, negative its beginning.
Reference: https://www.matroska.org/technical/elements.html . A synthetic EBML
regression checks both child orders, compact signed values and i64 extremes with
a non-default segment clock. 23 container tests pass. These fields are metadata
only at this stage; native Matroska AAC PCM export and playback trimming are not
yet connected to them.

Native Matroska AAC PCM/WAV export is now connected to CodecDelay and signed
DiscardPadding. `media decode-audio input.mka output.wav` uses owned container
and codec code, including `--from`/`--to` on the trimmed sample sequence. Trim
metadata rounds to nearest samples to undo muxer nanosecond rounding; user
intervals still select sample starts with ceiling boundaries. A contiguous
packet clock may vary within the container timestamp precision; larger gaps
are rejected. Laced blocks remain unsupported by the container. Playback's
separate audio adapter has not yet been switched to this trim handling.
Eleven integration tests pass; an independent PCM oracle and CLI WAV inspection
both confirm exactly 48000 stereo samples for the MKA fixture. Synthetic files
verify CodecDelay and both DiscardPadding signs by exact PCM slicing.

Owned AAC PCM/WAV export supports `--volume GAIN` for ADTS, MP4 and Matroska,
including interval selection. Gain follows the existing CLI contract: finite
linear 0..=64, applied as f32 multiplication without integer clipping. Headers
bypass processing; invalid gain is rejected before output creation and any PCM
overflow aborts atomic publication. Twelve integration tests pass, including
all three containers with 0, 0.5 and 64 gain and exact sample-byte comparisons.

Owned AAC export supports `--channels 1` and `--channels 2`, or unchanged source
channel count. Mono duplicates to stereo; stereo averages to mono. For standard
3–6 channel layouts, the centre and back channels contribute at 1/sqrt(2), LFE
is omitted, and mono is the stereo average. This explicit FVid mixing policy
uses float arithmetic without clipping or normalization; it is not a claim of
bit-identical libswresample policy. Gain is applied after mixing. WAV channel
mask, byte rate and payload size describe the output layout, while sample-frame
count/timing stay unchanged. Other output layouts remain unsupported. Tests use
six distinct active source channels and an independent f64 mixing calculation.

Native AAC export supports `--rate HZ` (also `--sample-rate`) in 8000..=384000.
The owned streaming resampler uses a Blackman-windowed sinc, a ratio-dependent
low-pass cutoff/support, and integer rational output positions. It holds only
filter history/lookahead, extends edge samples at stream boundaries, and emits
ceil(input_frames * output_rate / input_rate) frames. Equal-rate conversion is
byte-preserving. Interval selection precedes mixing/gain/resampling; WAV sizes,
rate and frame statistics reflect the final output. This is a FVid algorithm,
not an assertion of libswresample-identical samples or broad quality benchmarking.
Two DSP tests verify exact counts, constant signals, channel isolation, chunk
independence, 1 kHz passband amplitude and 12 kHz rejection during 48-to-16 kHz
conversion. Fourteen integration tests pass, including CLI rate+stereo+gain.

## Owned AAC MP4 muxer

`fvid media remux INPUT.aac OUTPUT.m4a` (or `.mp4`) copies complete ADTS raw AAC
packets into a single-track indexed MP4, using owned sample tables, ES descriptors
and file writing. It does not decode or re-encode packets. No priming edit is
invented, because ADTS does not state one. Leading/trailing unrepresented data
and truncated frames are rejected. Rates through 65535 Hz use version-0 audio entries; higher rates use
QuickTime-compatible version-2 descriptions with floating-point sample rate. Output publication
is atomic and does not replace existing paths.
Two integration tests verify mono/stereo/5.1 packet equality, timing and exact
owned PCM before/after remux, plus CLI/no-overwrite/truncation behavior. Independent
ffprobe identified 13 AAC packets, 48000 Hz, 13312 samples; FFmpeg reference-only
decoding of the output matched the saved source PCM byte-for-byte. This adds an
owned AAC MP4 writer; it does not yet replace arbitrary multi-track remux.

Owned MP4 AAC parsing/writing now supports v2 sound descriptions. Real 88200
and 96000 Hz fixtures preserve packets and decoded PCM. Invalid floating-point
rates, fractional rates and invalid channel counts are rejected. High-rate output
uses the qt compatible brand rather than claiming version-0 ISO sample entries.

## AAC-LC 960-sample frames

NativeAacDecoder now accepts frameLengthFlag=1 (960 samples with 120-sample
short transforms). Band geometry covers all 13 indexed sampling rates and the
existing explicit-rate intervals. Long/short spectral parsing, sine/KBD window
transitions, overlap state and output length use the ASC-selected frame size.
No external decoder is used. ADTS does not signal this ASC flag; these fixtures
use Matroska CodecPrivate and MP4 esds.

Independent PCM checks cover synthetic mono 8/48/96 kHz streams with all bands
active, varying band gains/signs, grouped eight-short windows and long-start /
long-stop transitions. Peak error is below 1e-6 against saved FFmpeg references.
Tests also cover sample-exact interval extraction, MP4 vs Matroska PCM equality, CLI
WAV export and playback adapter reset. This is not a full AAC conformance claim;
7.1/PCE, HE-AAC/SBR, ER and unsupported coding tools remain separate work.
Reproduction details: `tests/fixtures/audio/aac-960.md`.

## Owned AAC fast IMDCT (2026-09-29)

AAC synthesis now uses an owned radix-2 FFT chirp convolution for DCT-IV, followed
by IMDCT symmetry expansion. All four sizes (120, 128, 960, 1024) run in
O(N log N). Immutable transform tables are shared across transactional decoder
clones; synthesis reuses complex scratch for both long and short windows without
per-block allocation. The public convenience `Imdct::inverse` allocates scratch;
`inverse_with_scratch` is the allocation-free entry point used in playback.

Direct cosine-basis, dense-spectrum, overlap/window-transition and saved PCM
reference tests passed: 56 AAC unit tests plus 22 native PCM export tests. A local
release microbenchmark, run after builds/tests completed, measured median times:

| Coefficients | FFT | Previous recurrence | Ratio |
|---|---:|---:|---:|
| 120 | 2.29 us | 26.45 us | 11.57x |
| 128 | 2.32 us | 31.59 us | 13.64x |
| 960 | 22.77 us | 2752.02 us | 120.86x |
| 1024 | 22.98 us | 3115.42 us | 135.58x |

Reproduce with `cargo run --release --locked --no-default-features --example
aac_imdct_bench`. This compares transform kernels only, not total player throughput
or virtual-camera delivery. Peak difference from the previous recurrence was
1.41e-13 for the benchmark spectrum; independent basis tests use a 1e-10 bound.
No external FFT/codec dependency or FFmpeg execution is used.

### Full owned AAC path measurement (2026-09-29)

`cargo run --release --locked --no-default-features --example aac_decode_bench`
now measures fresh container indexing, decoder construction, all packet decoding
and PCM delivery to a black-box sink. File loading, disk output, resampling and
speaker scheduling are outside the timed region. It checks stable output stats
and PCM byte counts on every iteration; five timed batches report the median.
Optional positional paths allow measuring other supported AAC files.

Local results after the fast IMDCT change:

| Fixture | PCM duration | Decode median | Audio duration / decode time |
|---|---:|---:|---:|
| mono ADTS, 44.1 kHz | 0.163 s | 0.641 ms | 253.7x |
| stereo ADTS, 48 kHz | 0.277 s | 1.847 ms | 150.1x |
| mono ADTS with TNS | 1.024 s | 4.668 ms | 219.3x |
| active 5.1 ADTS | 0.235 s | 4.527 ms | 51.8x |
| MP4 with edit list | 0.128 s | 0.626 ms | 204.6x |
| stereo Matroska | 1.000 s | 7.890 ms | 126.7x |
| 960-sample Matroska | 0.140 s | 1.273 ms | 110.0x |
| 960-sample MP4 | 0.140 s | 0.777 ms | 180.1x |

These short fixture measurements show AAC headroom on this machine; they are not
before/after end-to-end speedup numbers, video FPS, a virtual-camera delivery test,
or a guarantee for all input streams. Correctness remains covered separately by
the saved PCM and direct-transform reference tests.


### ADTS multichannel and implicit SBR clock qualification

The authored `adts-layout-sbr` matrix contains 28 companion MP4 videos and
56 protected/unprotected ADTS streams: indexed and PCE 5.1/7.1 with LFE,
implicit mono/stereo SBR, and SBR arriving after the first core packet.
Each stream multiplexes two, three, four, or varying raw blocks per transport
frame. Independent scalar PCM qualifies the four LC layouts; own MP4 PCM
qualifies transport equivalence, output length, intervals, rewind and seek.

A regression reproduced the player reporting 24 kHz for 48 kHz SBR output.
The reader now locates and validates an SBR candidate before publishing its
output rate, and rescales packet timestamps and seek/preroll boundaries into
that output clock. This is selected fixture qualification, not complete AAC
profile or SBR/PS conformance.

Generator: `scripts/generate_adts_layout_sbr_fixtures.py`; sources are authored
synthetic AAC and AVC/container seeds. No private media or parameters are
copied. CRC values use independent polynomial division and the own offline
region helper. Ordinary tests only read saved fixtures; generation and tests
use neither FFmpeg/libav nor network access.


The same authored matrix now covers indexed/sequential ADTS-to-MP4 and
ADTS-to-Matroska remux. A red acceptance reproduced SBR rejection after remux
at a wrongly declared core rate. Writers validate the first SBR candidate and
publish the negotiated output rate. MP4 sample durations scale with that rate;
Matroska preserves nanosecond timing and rewrites its fixed-size track header
before finalization. Packet payloads are not transcoded or buffered as a whole
stream. Acceptance verifies exact full PCM and container output clocks, including
delayed SBR, protected/unprotected multiplexing and multichannel LC.
