# Audio Implementation Status

Ordinary AAC LTP (AOT4) is now admitted by the native decoder and MP4 path.
Mono/stereo, dependent/independent CCE, and 960/1024 long/short transitions
are qualified on authored PCM fixtures. Mono LTP/SBR explicit/sync/implicit
24/48 kHz signalling, late SBR and protected/multiplexed ADTS are qualified
on authored fixtures. Owned LTP/PS now covers 1024-frame window transitions,
CCE points 0/1/3, explicit/sync/implicit 24/48 kHz and stereo ADTS discovery.
Wider PS geometry/layouts, additional CCE/absence/source-extension combinations
and ER/LD/ELD/USAC remain incomplete. The incremental sections below retain
historical states.

ER AAC-LC (AOT17) has owned baseline, section-resilience, RVLC and HCR/epConfig0 paths with indexed
channel layouts and 960/1024 core frames. Authored mono/stereo/3.0/5.1 PCM
qualification is below; protected epConfig, PCE, SBR/PS and other ER profiles
remain incomplete.

ER AAC-LTP (AOT19)/epConfig0 has owned mono/stereo 960/1024 acceptance,
including all seven section/RVLC/HCR combinations, deferred common-window
prediction data and independent-window CPE HCR lengths. Protected epConfig,
PCE and SBR/PS remain incomplete.

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
rewind, seek and interval export. Explicit independent CCE absence/return now
preserves queued source history. Mono SSR/PS is qualified across three silent
core window schedules, explicit/sync signalling, both output clocks, MP4,
Matroska, ADTS and two-frame EOF drain. SSR/PS points 0/1/3 coupling and nonzero
SSR spectrum/gain composition now have stable configured program acceptance.
Live source changes, target TNS composition, PCE replacement and wider
profiles/layouts still require implementation or additional acceptance evidence. New independent CCEs can appear while alignment is active: six
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


### Implicit PS in mono-core ADTS

`adts_implicit_ps` reproduces the old ordinary AAC decoder refusal on a late
in-band PS element. Six authored single/multiplexed ADTS streams, protected
and unprotected, share a short synthetic AVC+AAC companion video and existing
independent scalar stereo PCM. The core configuration stays mono AAC-LC at
24 kHz; selected PS playback/export yields stereo at 48 kHz.

Mono ADTS discovery now scans the selected encoded prefix with the own
transactional syntax probe before publishing output geometry. Encoded records
are spooled to a private temporary file; retained RAM does not grow with prefix
length. Replay selects the own PS decoder and drains its delayed frame at EOF.
The native player uses the same verified PS choice and rescales source timing.
Acceptance verifies companion packet identity, independent full stereo PCM,
exact intervals, packet-limit geometry, rewind, seek and repeated EOF drain.
A one-packet prefix stays mono when its future PS packet was not selected.

Generator: `scripts/generate_adts_ps_fixtures.py`. Only existing authored
packets, scalar PCM and AVC/container seeds are used; no private media or
parameters are copied. CRC values use the own offline region helper and
independent polynomial division. Tests read saved fixtures without generation,
FFmpeg/libav or network access. This qualifies the tested 1024-sample late PS
route; first-packet PS remux, broader PCE/coupling combinations and complete
HE-AAC v2 conformance still require separate qualification.


### First-packet PS ADTS remux and source geometry

`adts_ps_remux` reproduces two failures: the mux probe passed an initial PS
payload to the ordinary AAC synthesis decoder, and `aac_source_info` reported
mono/core speaker geometry for verified in-band PS. Six new authored ADTS
streams (single/multiplexed, CRC/plain) and a short AVC+AAC companion cover
first-packet PS, missing-middle PS and restart, alongside the prior late-PS
fixtures. Independent scalar stereo PCM qualifies the new video and ADTS.

Mono implicit SBR/PS mux candidates are now validated by the own transactional
syntax probe. Indexed/sequential MP4 and streaming Matroska preserve original
raw packets, negotiated rate, sample clock/duration and complete stereo PCM.
ASC/core channel declarations remain mono; verified in-band PS selects stereo
output. Source inspection now probes MP4/Matroska payloads before returning
decoded rate, channel count and speaker mask. Tests cover companion source
inspection and both remux outputs, including late PS.

Generator: `scripts/generate_adts_ps_first_fixtures.py`, sharing the existing
own ADTS PS generator. Only authored packets, independent scalar PCM and
AVC/container seeds are used. No private media or parameter sets are copied.
Ordinary tests read saved fixtures and use neither FFmpeg/libav nor network
access. This supersedes the first-packet PS remux gap; broader mono PCE/CCE
combinations and complete HE-AAC v2 conformance still need qualification.


### Mono PCE / PS / CCE ADTS transport qualification

The new `adts-pce-ps` companion video and six protected/plain ADTS variants
reproduce the prior SBR signalling refusal for an explicitly configured normal
front mono PCE. The own PS syntax parser already supported this program and SCE
tag; discovery incorrectly excluded every PCE. Selection now uses the parser's
validated mono-program shape in export, native playback and mux rate probing.
Other AAC layouts remain on the ordinary decoder path.

Independent scalar stereo PCM qualifies the mono PCE video and ADTS. The
`adts-pce-ps-cce` matrix adds nine previously qualified authored PS/CCE programs:
coupling points 0/1/3, tags 1/15/both, reordered sources, source SBR and missing
target FIL. Fifty-four ADTS variants cover single, three-block and varying
transport grouping, with CRC/plain protection. Core direct-cosine and separately
qualified PS/SBR composition remain covered by `he_aac_ps_coupling`; the new
matrix qualifies transport and playback rather than claiming a new independent
whole-decoder oracle for CCE.

Acceptance checks complete PCM, exact raw packet preservation in MP4/Matroska,
output clocks/durations, source geometry, EOF drain, rewind and seek. Generators:
`scripts/generate_adts_pce_ps_fixtures.py` and
`scripts/generate_adts_pce_ps_cce_fixtures.py`, sharing the own offline ADTS PS
generator and region helper. No private media or parameter sets are copied.
Tests use saved files without generators, FFmpeg/libav or network access.
These selected mono PCE/CCE combinations do not establish complete AAC profile,
gain/tool or HE-AAC v2 conformance.


### Stereo PCE SBR in ADTS

`adts_pce_sbr` reproduces the extension-signalling refusal caused by excluding
every explicit PCE from ordinary ADTS SBR discovery. The underlying own
AAC decoder already supported a tagged sole normal-front CPE program. The
discovery gate now uses the resolved mono/stereo channel count and leaves
supported SBR program-shape validation to the native decoder.

Two short authored companion videos and twelve ADTS variants qualify coupled
and uncoupled SBR with CPE tag 3, single/three/mixed raw-block grouping and
CRC/plain protection. Both channels are compared directly with the existing
independent scalar SBR PCM. Acceptance verifies complete PCM, exact interval
selection, output rate/count/duration, raw companion packet identity, indexed
and streaming MP4 and Matroska remux, output-clock timestamps, rewind and seek.

Generator: `scripts/generate_adts_pce_sbr_fixtures.py`, sharing the own offline
ADTS fixture writer and CRC region helper. Only authored PCE/SBR payloads,
independent scalar PCM and AVC/container seeds are used. No private media or
parameters are copied. Ordinary tests read saved fixtures and invoke no
generator, FFmpeg/libav or network access. This qualifies the sole normal-front
stereo CPE at the implicit double-rate clock; arbitrary multichannel SBR
programs and other AAC tools/profiles still require separate implementation
and qualification.


### Multi-element ADTS SBR discovery and bounded PCM spool

`adts_multi_sbr` separately reproduces the former multichannel extension
signalling refusal and, after enabling discovery, the fixed mono/stereo spool
record rejection. LC ADTS discovery now admits the native decoder's supported
resolved layout instead of limiting its channel count to two. Private PCM
records are bounded by 2048 frames times the admitted channels (at least stereo
for PS); push/read enforce that extent. Encoded records remain at most 8191
bytes and retained RAM does not grow with prefix length. Optional controlled
memory admission now reserves possible SBR state for all discovered channels.

Nine authored short companion videos and 54 CRC/plain ADTS variants qualify
two-SCE, height two-SCE, 5.1 PCE/indexed, element reordering, silent LFE and
missing-element SBR. Every output lane is compared with independent scalar
SBR or missing-SBR PCM using the canonical channel mapping. Acceptance checks
full PCM, exact intervals, packet identity, output geometry/clocks, indexed
and sequential MP4 and Matroska remux, timestamps, rewind and seek. A controlled
budget rejection is tested before publishing PCM.

Generator: `scripts/generate_adts_multi_sbr_fixtures.py`, sharing the own ADTS
writer and offline CRC region helper. Only authored payloads, independent PCM
and AVC/container seeds are used; no private media or parameters are copied.
Ordinary tests read saved files without generation, FFmpeg/libav or network
access. This qualifies the tested implicit double-rate programs, not arbitrary
multichannel SBR, nonzero LFE extension processing or complete AAC conformance.
The 14-valued indexed height layout cannot be directly represented by the
three-bit ADTS channel configuration and is not included in this matrix.

### AAC Main implicit SBR — 2026-10-09

Main (AOT1), as well as LC, now negotiates implicit double-rate SBR from a valid
FIL in ADTS export, native player and owned remux. Main predictor state remains
in the core decoder; PS probing stays restricted to LC. The six-frame authored
active-prediction fixture verifies explicit/sync/implicit PCM equivalence,
protected/multiplexed ADTS, MP4/Matroska remux and rewind/seek. This is transport
and signalling qualification plus independent scalar PCM for the fixed authored
composition (Main prediction through SBR, 1e-9 tolerance), not complete AAC
Main+SBR conformance. Wider SSR+SBR tools/layouts,
LTP/ER/ELD/USAC and broader profile tools still require implementation/qualification.


### AAC SSR + SBR container synthesis — 2026-10-09

Explicit/sync/container-clock SBR now runs after SSR PCM alignment. A two-slot
packet queue preserves parsed SBR frames, layout and original duration until
corresponding PCM is ready; EOF, checkpoint/reset and rollback preserve all
histories. Delayed metadata allocations are counted in retained payload;
optional controlled-memory admission reserves their bounded transactional copies.
No new global/default memory limit is introduced.

Mono 24→48 kHz synthetic transitions with silent/nonzero core and active gain
control match independent scalar PCM at 1e-9 tolerance. Native export and player
rewind/seek/intervals pass. ADTS discovery and its player EOF/seek path are
qualified separately. Independent CCE SBR now runs through per-source aligned
PCM and independent SBR DSP before final gain/mixing; the qualified authored
programs cover mono/stereo targets and one/two simultaneous unit-gain CCE tags
1/15. Two additional stereo programs qualify separate left/right gain lists,
a constant right gain of 0.5 and positive time-varying gain 1/0.5/2 across
SSR source-chunk boundaries. Eight more programs qualify all four common
gain scales and both sign-flag values, which preserve positive common gain
for independent CCE; signed differential gain belongs to dependent coupling. Wider tools/layouts, broader CCE gain configurations, transitions between
mixed and source-aligned modes, source retirement and broader downsampled SSR/SBR tools
remain separate work, not claimed as complete.


### AAC SSR/SBR downsampled container output — 2026-10-09

Four explicit/sync target programs and fourteen explicit independent CCE
programs qualify 24 kHz output from the 24 kHz SSR core with SBR present.
The oracle independently evaluates 32-band QMF synthesis; expected PCM is
not a decimated production output or a production decoder result. Silent
and nonzero target programs, mono/stereo CCE targets, one/two sources and
common-gain scales/sign flags are exercised. Native checkpoint/error/reset/EOF
and player rewind/seek pass for both 24 and 48 kHz output clocks.
Implicit FIL present from the first packet is now also qualified at an unchanged
24 kHz container clock for direct target and CCE programs. Late FIL discovery,
SSR PS, arbitrary source switching/retirement and wider tools remain separate.

A late-FIL CCE video with a zero core prefix now qualifies mixed-to-source
queue upgrade without dropping pending PCM, including checkpoint/reset/EOF
and player seek across the boundary. The additional nonzero SCE/CCE programs
now qualify pre-FIL QMF history at a fixed 24 kHz clock: each aligned source
is analyzed and synthesized before FIL, while the published prefix remains
bit-exact core PCM. This does not establish arbitrary late rate changes,
untimed variable-length SSR discovery, PS or source retirement.


### Main/LC late SBR with fixed core output — 2026-10-09

Main/LC discovery warm-up now honors a fixed core output hint before FIL,
for direct target and independent CCE DSP alike. The old change from
Double warm-up to Core synthesis at first FIL no longer discards/refuses
history. Authored 24 kHz SCE/CCE programs retain bit-exact core prefixes
and compare the complete waveform against independent scalar QMF output.
Checkpoint/error/reset/EOF and player rewind/seek cover the transition.
Broader signalling/rate switches, layouts and coupling tools remain open.


### SSR/SBR independent window schedules — 2026-10-09

Five independent target/CCE long/start/short/stop schedules now have full PCM
acceptance with explicit, sync-extension and implicit SBR at 24/48 kHz.
The 35 authored programs include five bit-exact nonzero core controls;
complete SBR output agrees with independent scalar IPQF→QMF references.
Checkpoint/error/reset/EOF and player rewind/seek cover all programs.
The existing queue/DSP handles these schedules without decoder changes.
This covers a stable mono target / unit independent CCE roster; dynamic
retirement, SSR PS and broader tools/layouts remain separate work.


### SSR independent CCE absence/return — 2026-10-09

An absent CCE now keeps its canonical lane and previously queued PCM/gains.
Only the uncovered explicitly absent timeline receives gainless zeros; coded
SSR history is retained for return, and SBR uses its existing no-FIL
pure-upsampling path. The old roster-change refusal is replaced with PCM
acceptance. Ahead/behind/exact and return schedules have independent core
and 24/48 kHz SBR waveform references, checkpoint/reset/EOF and player seek
acceptance. Ordinary incomplete coded streams still refuse at EOF. This
does not cover PCE replacement, SSR PS or all wider coupling/layout tools.


### SSR/PS syntax preparation prerequisite (2026-10-09)

The owned mono SBR/PS stage now separates validated packet syntax from DSP:
`prepare` / `prepare_upsampling` retain at most two unprocessed frames;
`process_prepared` consumes them in wire order when aligned core PCM arrives.
The existing immediate `read` and absent-FIL API remain transactional wrappers.
EOF refuses to discard unprocessed syntax. The retained queue compares the full
opaque prepared frame, so a foreign same-index payload cannot substitute for
accepted syntax. Failed CRC, truncated or nonfinite PCM, duplicate/out-of-order
frames and excess lookahead leave the relevant reader/history/DSP unchanged.
Checkpoint replay and reset preserve that contract. Independent authored scalar
stereo references qualify delayed submission at core and doubled output clocks.

`scripts/generate_aac_ssr_ps_fixtures.py` authors three silent SSR sine/KBD window
schedules with existing original SBR/PS payloads, explicit/sync signaling and
both clocks: three core controls and twelve combined videos. Stereo gold comes
from the independently authored PS/QMF scalar references, not this decoder.
Ordinary tests read committed artifacts; no FFmpeg, foreign decoder, network or
private media is used. Enabled tests prove valid SSR core and PS stage output
and reproduce the exact native SSR/PS profile gate. Combined native waveform
acceptance is explicitly ignored until SSR alignment, PS lookahead and the two
EOF frames are integrated. This change does not claim native SSR/PS playback
acceptance or complete codec conformance.

Validation for the preparation prerequisite: locked/offline root unit tests
(909 passes), owned-media unit tests (461 passes), and the SSR/PS, native PS,
SBR/PS, PS absence and PS coupling integration suites. The ignored combined
SSR/PS acceptance was explicitly run separately and fails at the exact native
profile gate; this is a red acceptance check, not a passing playback test.
The 19 new artifacts regenerate identically. Changed Rust formatting and
`git diff --check` pass.


### Native mono SSR/PS alignment and transport acceptance (2026-10-09)

Mono AAC-SSR now uses its own four-band gain/window/IPQF synthesis before a
fixed 1024-core-sample alignment queue. SBR/PS syntax validates in the original
packet; retained prepared frames are consumed with matching aligned core PCM.
Alignment and PS hybrid lookahead retain two original packet identities.
EOF feeds the final aligned PCM and returns both remaining PS frames on
successive calls (one-input streams still produce their single complete frame).
Native packet transactions, checkpoint/replay and reset retain syntax, gain,
overlap, aligned PCM and PS state together. SSR does not allocate the unused
LC filterbank. Configured SSR PS coupling remains an explicit unsupported tool.

The playback bridge now retains up to two signed source timestamps/durations.
MP4 and Matroska export queues retain source windows/packets and drain every
EOF frame. ADTS owned export and playback discover mono SSR PS in-band,
provide negotiated stereo at twice the core clock and drain both frames.

The old native SSR/PS profile refusal test has been removed and its waveform
acceptance enabled. The 12 mono MP4 cases cover explicit/sync signalling,
24/48 kHz output, sine/KBD and long/start-stop/start-short-stop schedules.
Their nonzero stereo PCM matches the independent authored scalar gold.
Twelve Matroska and three unprotected ADTS streams qualify the same PCM;
twelve additional MP4s add silence and repeated source ranges. Enabled tests
cover full/range export, both EOF identities, source timing including negative
PTS and short durations, malformed packet rollback, checkpoint replay, reset,
rewind and player seek. Repeated-range playback uses the actual AudioStep
reset/silence scheduling contract. The intermediate missing EOF/window bug
reproduced `audio edit extends outside available samples` before the queue fix.

All artifacts come from `scripts/generate_aac_ssr_ps_fixtures.py`, original
silent SSR core syntax and existing original independent SBR/PS scalar gold.
No private source, foreign decoder, FFmpeg or network is used. Ordinary tests
read committed artifacts and do not run generators. This qualifies these mono
programs; nonzero SSR spectral/gain tools composed with PS, SSR PS CCE, broader
layouts/profiles and general codec conformance remain separate requirements.
This section supersedes the earlier preparation-only native refusal status.

Validation: 909 root and 461 owned-media unit tests plus 36 selected integration
tests passed (1406 distinct passes, 24 existing ignored unit tests). All five
SSR/PS tests, including the formerly ignored native waveform acceptance, are
enabled. Checks were locked/offline without FFmpeg. All 46 artifacts regenerate
identically, and the 18 original binary fixtures/references remain unchanged.
Changed Rust module/test formatting and `git diff --check` pass.


### Native SSR/PS dependent and independent CCE composition (2026-10-09)

The configured SSR PS coupling gate is removed. Points 0/1 use spectral mixing
and the target SSR gain/window/IPQF pipeline, with the same mandatory dependent
window-shape check as the ordinary SSR decoder. Point 3 synthesizes each CCE
with its own tag-keyed SSR filterbank/gain history. A source-preserving alignment
queue retains raw PCM and original output gain boundaries, then applies that
source's original SBR FIL (or pure upsampling) at the fixed PS core clock.
Only channel 0 selected by the mono SCE target receives independent coupling
after PS, consistently with the existing owned LC/PS dispatch contract.
Canonical source lanes preserve prior chunks when tags are added; explicitly
absent lanes have no output gains. Native packet transactions, checkpoint,
reset and both EOF frames retain the source syntax, PCM and coupling identity.
SSR CCEs do not allocate an unused LC filterbank.

`scripts/generate_aac_ssr_ps_coupling_fixtures.py` authors 14 nonzero SSR core
controls, 36 combined PS programs and three malformed videos. The matrix
covers points 0/1/3, tag 1 alone and tags 1/15 together, active/inactive SSR gain,
sine/KBD and long/start/short/stop transitions, both 24/48 kHz PS clocks, eight
independent-source SBR programs and four standalone nonzero mono SSR/PS programs.
Controls use the original independently evaluated scalar SSR/IPQF PCM. Combined
acceptance uses that scalar core plus separately qualified owned SBR/PS DSP
composition; it is not a new independent end-to-end PS numerical oracle.
Malformed shape, absent target and source CRC reproduce their specific errors
and preserve queued audio through a complete valid continuation and EOF replay.
All native waveform and export/range/rewind/seek tests are enabled. The old
profile-gate reproduction passed before the implementation and was removed
when the intended acceptance was enabled.

No private media, foreign codec, FFmpeg or network is used. Ordinary tests
read committed artifacts and do not run the generator. This qualifies the
listed stable configured programs, not all coupling or codec conformance.
Distinct source spectra, independently switched source windows, live source
appearance/absence with PS, target TNS composition, PCE replacement and broader
profiles/layouts remain further qualification or implementation requirements.
This section supersedes earlier SSR/PS coupling and nonzero-core refusal notes.

Validation: 909 root and 461 owned-media unit tests plus 36 selected integration
tests passed (1406 distinct passes, 24 existing ignored unit tests). All five
new SSR/PS coupling tests are enabled. The 56 new artifacts regenerate
identically. Checks were locked/offline without FFmpeg; changed native/test
formatting and `git diff --check` pass.


### Distinct SSR/PS coupling source histories (2026-10-09)

The coupling fixture matrix now contains 20 core controls and 48 PS programs.
Six new core controls and twelve new PS videos use independent scalar SSR/IPQF
PCM for tags 1 and 15, different sine/KBD histories and a 2:1 spectral amplitude
ratio. Source wire order reverses on alternating packets. The PS cases cover
inactive/active gain, 24/48 kHz output and absent, shared or asymmetric source
SBR payloads. Native waveform, rollback, rewind, seek, ranges and delayed EOF
acceptance use each source's own reference PCM and extension history.

These are authored synthetic fixtures; ordinary tests need neither FFmpeg nor
network access. The combined PS reference composes independently evaluated SSR
PCM with the qualified owned SBR/PS stage, rather than providing a new fully
independent PS oracle. This qualification does not cover independently switched
source window sequences, source absence/return under PS, or PCE replacement.
No production decoder change was required for this extension.


### SSR/PS independent CCE absence and return (2026-10-09)

Sixteen further authored videos qualify final disappearance and coded-history
pause/return for CCE1 under PS. Source-ahead, source-behind, exactly aligned
absence and return schedules run at 24/48 kHz with and without source SBR.
Target packets remain long-window silent SSR with the original PS payloads.
Source coded ordinals pause during absence; the original scalar SSR alignment
PCM is preserved, including queued samples beyond the last present packet.
Per-chunk gain intervals distinguish those samples from zero absent lanes.

The reference warms source QMF with aligned PCM even when output gain is absent,
then applies the original gain intervals after extension DSP. It composes the
independent scalar SSR source oracle with the already qualified owned SBR/PS
stage; it is not a new independent full PS numerical oracle. Native acceptance
checks checkpoint/retry, reset, packet indices, both delayed EOF frames and
MP4 export. The existing playback test also covers ranges, rewind and seek.
The matrix now has 20 core controls and 64 PS programs. No production decoder
change was needed. PCE replacement, multiple disappearing sources and wider
profiles/tools remain separate work. No private media, FFmpeg or network is
used by the generator or ordinary tests.


### SSR/PS in-band PCE coupling roster changes (2026-10-09)

Twelve synthetic programs reproduce the former exact refusal
`PS AAC in-band PCE changed the configured layout` when only the CCE roster
changes. The native PS decoder and syntax-only PS probe now accept that roster
change transactionally while keeping profile, clock, target SCE tag and layout
fixed. Existing source synthesis/QMF history remains keyed by tag; queued gain
intervals and PCM survive removal and a returning tag resumes its history.

The initial ASC program is retained separately from the current in-band PCE.
Checkpoint restore compares the initial configuration and restores current PCE
state; reset returns to ASC. A checkpoint from another initial roster remains
incompatible. The videos cover aligned/ahead sources, final removal and return,
with/without source SBR at 24/48 kHz. Their waveform reference equals the original
absence/return programs, using independent scalar SSR core and qualified owned
SBR/PS composition. An incompatible stereo PCE companion retains the exact
layout refusal and verifies queued-frame rollback.

This supersedes the fixed-CCE-roster restriction for native PS. General PCE
layout/target-tag replacement, ordinary non-PS AAC roster changes and wider
profiles remain separate work. The matrix has 20 core controls, 76 PS programs
and four malformed/unsupported companions. Generation and ordinary tests use
no private media, foreign decoder, FFmpeg or network.


### Ordinary AAC in-band PCE coupling roster changes (2026-10-09)

NativeAacDecoder now treats the in-band CCE roster as transactional packet
state, retaining its initial ASC program separately. The fixed profile, sample
clock, tagged output elements, height and PCM layout remain checked. Checkpoints
restore current PCE together with synthesis and queued PCM; reset restores ASC.
SBR reserves the four-bit CCE slot domain for explicit PCE programs even when
the current roster is empty, preserving histories through removal/return.

Twelve additional SSR core/SBR videos reproduce the former exact layout refusal
for a valid roster-only change. Their accepted PCM equals the original scalar
SSR/QMF absence/return oracle. They cover source-ahead/behind/exact/return with
core-only and 24/48 kHz SBR output. Tests cover timing, checkpoint replay, reset,
rewind, seek and EOF. Two malformed companions retain exact layout and absent
CCE errors and prove configuration/PCM rollback before continuing the baseline.

Fixtures are authored and generated offline without FFmpeg, foreign decoders
or private media. Dynamic Main/LC roster acceptance, initially empty roster
arrival, output element/tag/layout changes and broader profiles/tools remain
separate qualifications; this does not claim those cases are complete.


### Main/LC dynamic CCE PCE roster qualification (2026-10-09)

Twelve authored mono Main/LC videos qualify the ordinary decoder's roster
changes: static/dynamic PCE controls for mixed-window absence/return, long-only
source absence/return, and two independently arriving sources after an empty
initial roster. CCE1/15 have distinct residual spectra and window histories,
wire order alternates, and each source's coded ordinal pauses during absence.
Target PCM is also nonzero; Main prediction warms, activates and resets on the
authored schedule. Each program has twelve packets (512 ms at 24 kHz).

`scripts/generate_aac_pce_roster_fixtures.py` evaluates independent scalar Main
prediction and direct IMDCT/window overlap. Static and dynamic variants must
produce identical PCM and agree with that oracle. Incorrect scalar controls
that discard source histories on absence differ measurably; the long-window
Main program additionally detects discarding predictor alone while preserving
overlap. No production decoder is used by generation.

Enabled tests cover full native export, exact dynamic/static PCM identity,
checkpoint replay around every PCE, malformed trailing syntax rollback, reset
to empty ASC roster, rejection of a foreign initial roster checkpoint, playback
ranges, rewind and seek. All three tests passed on all twelve videos; fixture
generation and ordinary tests require no private media, FFmpeg or network.
No production change was needed beyond the preceding roster-state fix.

This qualifies Main/LC core independent coupling at point 3. Dynamic dependent
coupling, source SBR across initially empty rosters, multi-channel roster changes
and actual output layout/profile transitions remain separate work.


### Main/LC dependent CCE roster qualification (2026-10-09)

Thirty-two further videos qualify dependent coupling points 0/1 with changing
CCE1/15 rosters in Main and LC. Each has nonzero target and source spectra,
static/dynamic PCE controls, source removal/return or initially empty roster
arrival. Window-transition programs use matching target/source ICS geometry.
Long-window companions carry first-order target TNS: point 0 mixes sources
before TNS, while point 1 mixes after TNS. The scalar predictor, spectral sum,
TNS recurrence and direct IMDCT/overlap oracle computes those stages explicitly.

An oracle-sensitivity test checks eight paired TNS programs and proves the two
points produce different PCM. Native export for both agrees with the oracle,
and static/dynamic variants remain exactly equal. All four roster tests passed
on the resulting 44-video matrix, including trailing-syntax rollback around
PCE changes, checkpoint/reset, empty-initial-roster rejection, playback ranges,
rewind and seek. No production decoder change was required.

Existing twelve videos and scalar/packet prefixes remain unchanged. The
generator and ordinary tests use no private media, FFmpeg or network. These
programs qualify core mono dependent coupling and first-order target TNS;
source SBR across dynamic rosters, stereo/multichannel changes, changing actual
output layouts/profiles and wider codec tools remain separate work.


### SSR source SBR after initially empty PCE roster (2026-10-09)

Six authored videos qualify CCE1 arrival after two complete target packets.
Static-roster controls and initially empty dynamic PCE variants cover core-only
SSR and source SBR at 24/48 kHz. On packet two the dynamic program announces
CCE1 before its coded source/FIL. Independent scalar SSR alignment provides
2048 silent samples followed by four source-window chunks totaling 4096 samples;
the direct QMF/SBR oracle evaluates extension output at each negotiated clock.

Native PCM must agree with that oracle and static/dynamic exports must match
exactly. Tests additionally check the initial silence, checkpoint/replay, reset,
rewind, seek and EOF. Reset restores the empty initial roster and rejects a CCE
without its new PCE; foreign initial-roster checkpoints remain incompatible.
This qualifies the fixed four-bit SBR slot domain added by the earlier roster
fix: an empty current roster does not remove storage for a later CCE tag.
No additional production decoder change was needed.

The absence/roster matrix now has 30 positive videos and two negative companions.
Generation and ordinary tests use no private media, FFmpeg or network. Dynamic
Main/LC source SBR, source PS, multiple late SSR sources, actual target layout
and profile changes and wider codec tools remain separate work.


### Main/LC source SBR under dynamic CCE PCE rosters (2026-10-09)

Thirty-six authored videos qualify independent CCE1 with Main/LC core, source
SBR at 24/48 kHz, and static/dynamic PCE rosters. Six-packet programs cover
source absence/return, initially empty roster arrival and final retirement.
Twelve core controls retain nonzero spectra, alternating window shapes and
Main prediction activation/reset. Target is silent mono SCE0.

The generator evaluates scalar prediction and direct IMDCT/overlap per coded
source ordinal, then direct QMF/SBR convolution on that source clock. When a
complete CCE is omitted, its coded synthesis/extension history pauses while
target presentation continues with zero coupling. This ordinary Main/LC contract
is distinct from SSR's variable-length chunk alignment and absent-lane padding.
Static and dynamic PCM must be identical and both agree with the scalar oracle.

Four enabled tests passed: complete PCM and silence intervals, trailing-syntax
rollback/checkpoint/reset/EOF, playback ranges/rewind/seek, and reference
sensitivity to discarding source extension DSP at return. Incorrect numerical
controls retain core predictor/overlap and reset only extension DSP; all eight
paired SBR programs differ measurably. No production decoder change was needed.
Generation and ordinary tests use no private media, FFmpeg or network.

Dependent coupling with dynamic target/source SBR, source PS, simultaneous
multiple late SBR sources, actual output-layout/profile changes and wider codec
tools remain separate work. This qualification does not claim full codec parity.


### Native nonzero AAC Main + PS acceptance (2026-10-09)

The native PS core-profile gate now admits Main (AOT 1). Target spectrum passes
through the existing owned Main predictor before TNS, coupling, synthesis and
SBR/PS. One predictor bank belongs to the target; configured CCE banks are keyed
by tag and updated before source TNS. Packet clones and opaque checkpoints
retain those banks; reset clears target and source histories. Syntax-only PS
probe admits the same Main mono configuration. Existing LC/SSR behavior remains
covered by adjacent regression suites.

Three authored videos reproduce the former exact profile refusal: a nonzero
Main core control and explicit PS at 24/48 kHz. They exercise sine/KBD, long,
start, short and stop windows, prediction warm-up/activation and group reset.
The core control agrees with independent scalar prediction/IMDCT. Combined
acceptance composes that core PCM with the already qualified owned PS stage;
it is not a new fully independent PS numerical oracle. The refusal expectation
is replaced by enabled waveform acceptance with original indices 0–11 and EOF.

Tests verify malformed trailing syntax rollback, checkpoint replay, reset,
MP4 export, PS probe, playback ranges, rewind and seek. Four integration suites
passed (14 tests), together with all 461 owned-media unit tests (one existing
ignored). Fixture generation and ordinary tests require no private media,
FFmpeg or network. Main PS configured CCE/source SBR, PNS/TNS compositions,
in-band discovery/transports and wider layouts/profiles need separate acceptance
matrices; this mono qualification does not claim those tools are complete.

### AAC Main PS dependent CCE prediction qualification (2026-10-09)

Four authored MP4 videos cover two distinct Main CCE sources (tags 1 and 15),
coupling points 0/1, 24/48 kHz PS output, long/start/short/stop sequences,
sine/KBD windows, distinct predictor reset groups and alternating wire order.
The scalar oracle maintains separate source predictors and combines spectra
before direct IMDCT; the already qualified PS stage supplies the extension
composition reference. No private media, FFmpeg, network or foreign decoder is
used. Generator: `scripts/generate_aac_main_ps_cce_fixtures.py`; acceptance:
`tests/aac_main_ps_cce.rs`. Generation is separate from ordinary tests.

Acceptance checks waveform, per-packet rollback/checkpoint replay, delayed EOF
indices, reset, MP4 export, probe replay, intervals, rewind and seek. A numerical
control discarding predictor history differs from the oracle. Temporarily
omitting the production CCE Main prediction hook makes waveform acceptance fail
at sample 5317 for point 0 / 24 kHz; the production hook is restored byte-for-byte.
All ten generated artifacts reproduce deterministically.

This matrix has no target TNS, source SBR, CCE absence/roster transitions or
independent point 3 coupling. Points 0/1 therefore share PCM in this matrix;
TNS-sensitive ordering and those other combinations still need separate Main PS
acceptance. This is bounded evidence, not full AAC or codec parity.

Validation: 12 integration tests passed across `aac_main_ps_cce`,
`aac_main_ps` and `he_aac_ps_coupling`, locked/offline with production
`media,player` features and no FFmpeg. Production code is unchanged from
`ec338f53a`; the temporary mutation was fully restored.

### AAC Main PS target TNS/coupling ordering (2026-10-09)

The Main PS CCE matrix now includes four additional long-window MP4 videos
(points 0/1, output 24/48 kHz) with first-order target TNS and the same distinct
source predictors/reset groups. An independent scalar all-pole recurrence is
applied after spectral coupling for point 0; point 1 adds the source after the
silent target's TNS. The full PS PCM reference composes the independent core
with the previously qualified PS stage. Acceptance requires observable point
0/1 PCM differences at both clocks and continues waveform, transactional
checkpoint, reset, delayed EOF, probe, interval, rewind and seek checks.

A temporary mutation reversing production point selection passed the original
non-TNS cases but failed `0-24000-tns` at PCM sample 887, proving sensitivity to
this specific stage-order error. Production source was restored byte-for-byte.
The generator deterministically reproduces 18 artifacts; all eight previously
committed videos/core/control files remain byte-identical.

This supersedes the previous target-TNS qualification gap for this first-order,
long-window mono Main PS matrix only. Source TNS, higher orders, independent
point 3, source SBR and CCE absence/roster transitions still need qualification;
full AAC/all-codec parity remains incomplete.

Validation: 11 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_pce_roster`, locked/offline with `media,player`.
Production code remains unchanged; fixture generation stays outside tests.

### AAC Main PS forward/reverse source TNS acceptance (2026-10-09)

Eight additional authored MP4 videos extend the two-source Main PS matrix to
16 videos. CCE tag 1 has forward first-order TNS and tag 15 has reverse TNS;
they retain distinct predictor histories and reset groups. Cases combine
source TNS with and without target TNS at coupling points 0/1 and output
24/48 kHz. The independent scalar core oracle applies Main prediction before
source TNS, then spectral coupling and target TNS at the appropriate point,
followed by direct IMDCT. The previously qualified PS stage remains the
extension composition reference rather than a new independent PS oracle.

Enabled tests verify waveform, checkpoint/rollback, reset, delayed EOF,
MP4 export, probe, intervals, rewind and seek across all 16 cases. An explicit
numerical control omitting source TNS changes the final PS PCM in all eight
new cases. A temporary production mutation omitting source TNS passes the
prior eight cases, then fails `0-24000-source-tns` at sample 851. Production
source has been restored byte-for-byte. All 38 generated artifacts are
deterministic, and the 16 prior videos/core/control files remain unchanged.
Generation is separate from ordinary tests and uses no private media,
FFmpeg, foreign decoder or network.

This covers first-order source TNS with long sine/KBD windows in this mono
Main PS dependent CCE matrix. Higher orders, short-window source TNS,
point 3/source SBR, PNS and CCE absence/roster transitions remain separate
qualification gaps; no complete AAC or all-codec parity claim is made.

Validation: 12 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_pce_roster`, locked/offline with `media,player`
and no FFmpeg. Production code is unchanged.

### AAC Main PS independent point-3 sources (2026-10-09)

Four authored MP4 videos cover independent CCE tags 1/15 at PS output
24/48 kHz. Each source has distinct residuals, Main prediction history,
window-transition schedule and sine/KBD shapes. Two cases carry source SBR
on tag 1 while tag 15 uses upsampling; two use upsampling for both. Wire
order alternates, including CCE placement before and after the target SCE.
The scalar oracle independently computes each source's Main prediction and
IMDCT/overlap. Qualified SBR/PS stages then render source/core composition,
adding independent coupling only to the left SCE output after PS, aligned
with the delayed frame index. This is composition evidence, not a new
independent numerical SBR/PS implementation.

Acceptance checks waveform, source-history sensitivity, packet rollback and
checkpoint replay, reset, all 12 source frame indices including delayed EOF,
MP4 PCM export, syntax probe replay, intervals, rewind and seek. A temporary
production mutation omitting CCE Main prediction fails waveform acceptance
at sample 7196 for 24 kHz upsampling. The original source was restored
byte-for-byte. Generator `scripts/generate_aac_main_ps_independent_fixtures.py`
reproduces ten artifacts deterministically and stays separate from ordinary
tests. No private media, FFmpeg, foreign decoder or network is used.

This qualifies continuously coded Main PS point-3 sources and one authored
source-SBR geometry. Source absence/arrival, PCE roster transitions, multiple
source-SBR geometries, source PS, PNS and wider layouts/profiles remain open;
full AAC/all-codec parity has not been demonstrated.

Validation: 12 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_main_ps_independent`, locked/offline with
`media,player` and no FFmpeg. Reassigning source SBR FIL to the other
source changes the composition reference at both clocks, guarding binding
sensitivity. Production code is unchanged.

### AAC Main PS point-3 source absence and PCE roster transitions (2026-10-09)

Sixteen additional authored MP4 videos extend point-3 qualification to 20
videos: tag 1 disappears for three target frames and returns, or first appears
after three frames, while tag 15 remains coded. Each schedule has static and
dynamic PCE variants, source-SBR on/off and 24/48 kHz output. Dynamic PCE
removes/re-adds the coupling tag while preserving the mono target layout.
Source packet windows, prediction and SBR payloads follow a compact coded
ordinal clock that pauses on absence; the target PS presentation clock
continues. The reference selects independent scalar Main/IMDCT core PCM by
that coded ordinal and retains tag-keyed qualified SBR DSP/history state.

Waveform, packet rollback/checkpoints, reset, delayed EOF, export, probe,
intervals, rewind and seek acceptance cover all 20 videos. Static and dynamic
roster variants must produce byte-identical PCM. A numerical control resets
only the absent source's DSP and observably changes return PCM at both rates,
with source-SBR enabled and disabled. A temporary production mutation clearing
CCE states on every PCE update passes static cases but fails waveform at
sample 8192 in `0-24000-return-dynamic`. Production source is fully restored.
All 26 artifacts reproduce deterministically; the eight prior video/core/
control files remain byte-identical. Generation remains separate from tests,
with no private media, FFmpeg, foreign decoder or network.

This supersedes the tag-1 absence/arrival and compatible PCE roster gap for
this mono Main PS point-3 matrix. Initial empty rosters, simultaneous absence
of both sources, source TNS/PNS combinations, multiple source-SBR geometries,
source PS and wider layouts/profiles remain unqualified; all-codec completion
is not established.

Validation: 13 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_main_ps_independent`, locked/offline with
`media,player` and no FFmpeg. Production code is unchanged.

### AAC Main PS empty rosters and simultaneous source absence (2026-10-09)

Sixteen authored MP4 videos extend the independent point-3 matrix to 36
videos. Both CCE sources disappear together for three target frames and
return, or first appear after three target-only frames. Static and dynamic
PCE variants cover source-SBR on/off and 24/48 kHz output; the dynamic late
arrival ASC begins with an empty coupling roster. Source clocks remain
compact coded ordinals, while target PS advances throughout absence.

The existing scalar Main/core plus qualified SBR/PS composition checks now
cover every case, including waveform, packet rollback/checkpoints, reset,
all target frame indices and delayed EOF, export, probe, intervals, rewind
and seek. Static/dynamic roster PCM must remain identical. The discarded-DSP
numerical control also covers simultaneous source absence. A dedicated test
compares frames with no CCE against a target-only PS reference, and requires
the right channel to remain target-only for every frame even when sources
return. This guards leaked stale coupling PCM and target-clock disruption.

All 42 generated artifacts are deterministic; the 24 prior videos/core/control
files remain byte-identical. Fixture generation is separate from ordinary
tests and uses no private media, FFmpeg, foreign decoder or network.
This covers empty initial rosters and simultaneous absence for this mono
Main PS point-3 matrix. Source TNS/PNS combinations, multiple source-SBR
geometries, source PS, broader profiles/layouts and all-codec parity remain
unqualified. The SBR/PS reference uses qualified owned stages rather than a
new independent numerical SBR/PS oracle.

Validation: 14 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_main_ps_independent`, locked/offline with
`media,player` and no FFmpeg. Production code is unchanged.

### AAC Main PS point-3 source TNS/SBR matrix (2026-10-09)

A separate 36-video authored matrix qualifies forward first-order source TNS
on tag 1 and reverse TNS on tag 15 for independent Main PS coupling point 3.
Long sine/KBD source windows, distinct predictor histories/reset groups,
source-SBR on/off and 24/48 kHz output combine with continuous sources,
single-source absence/arrival, simultaneous absence/arrival, initially empty
rosters and static/dynamic PCE variants. The scalar oracle predicts each
source before directional TNS and direct IMDCT/overlap; qualified owned
SBR/PS stages supply the extension composition reference. A separate source
core control omits TNS, preserving prediction and overlap.

Enabled acceptance checks waveform, packet rollback/checkpoints, reset,
frame indices/delayed EOF, export, syntax probe, intervals, rewind and seek.
Static/dynamic roster PCM stays identical; absent-source DSP discard,
predictor-history discard, source FIL reassociation and source-TNS omission
controls all change final PCM. Empty frames equal target-only PS PCM and
independent sources do not change the right target channel. A temporary
production mutation omitting source TNS fails waveform at sample 968 for
24 kHz upsampling; original production source is restored byte-for-byte.

Generator `scripts/generate_aac_main_ps_independent_tns_fixtures.py` reproduces
44 artifacts deterministically. It stays separate from test execution and
uses no private media, FFmpeg, foreign decoder or network. Prior fixture sets
are unchanged. This qualifies first-order long-window source TNS combinations
in this mono Main PS point-3 matrix only. Short-window/higher-order source TNS,
PNS, additional SBR geometries, source PS and broader profiles/layouts remain
open; neither a new independent numerical PS oracle nor all-codec parity is
claimed.

Validation: 15 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_main_ps_independent_tns`, locked/offline with
`media,player` and no FFmpeg. Production code is unchanged.

### AAC Main PS independent source PNS/TNS/SBR acceptance (2026-10-09)

Eight authored MP4 videos qualify PNS-band switching in two independent Main
PS CCE sources (tags 1/15), with source TNS on/off, source-SBR on/off and
24/48 kHz output. Both sources switch a second band from ordinary residuals
to PNS and back, with distinct noise energies and alternating wire order.
A single scalar noise generator follows actual CCE parse order; per-tag
scalar Main predictors reset the noise-band lines before subsequent ordinary
prediction. Optional first-order TNS runs forward on tag 1 and reverse on
tag 15, followed by direct IMDCT/overlap. Qualified owned SBR/PS stages
provide the extension composition reference rather than a new numerical PS
oracle.

The explicit wrong core control advances predictor state on zeroed PNS input
but omits per-line reset, matching the targeted production mutation. Its
final PS PCM observably differs in every case. Temporarily removing
`bank.reset_lines(range)` from production Main/PNS reconstruction causes
waveform acceptance to fail at sample 21544 after ordinary coefficients
return. Production source is restored byte-for-byte. Other acceptance checks
cover packet rollback/checkpoints (including noise RNG), reset, delayed EOF
indices, MP4 export, syntax probe, intervals, rewind and seek; source FIL
reassociation remains numerically observable.

Generator `scripts/generate_aac_main_ps_pns_fixtures.py` reproduces 18 artifacts
deterministically, separately from tests, with no private media, FFmpeg,
foreign decoder or network. Prior fixture sets remain unchanged. This matrix
uses continuously coded long-window mono Main PS point-3 sources. PNS with
short windows, source absence/PCE changes, broader band geometry, additional
SBR profiles, source PS and wider codecs remain separate gaps; full AAC or
all-codec completion is not claimed.

Validation: 10 integration tests passed across `aac_main_ps_pns`,
`aac_main_tools` and `aac_main_ps`, locked/offline with `media,player`
and no FFmpeg. The existing Main TNS order-21 check remains a refusal
regression, not order-21 acceptance. Production code is unchanged.

### AAC Main PS PNS source absence and PCE changes (2026-10-09)

48 additional authored MP4 videos extend the Main PS PNS matrix to 56 cases.
Single-source return, simultaneous-source return and initially empty-roster
arrival combine with static/dynamic PCE, source TNS on/off, source-SBR on/off
and 24/48 kHz output. New programs have 16 target frames, allowing ordinary
coefficients to return after the compact source-clock PNS interval despite
three-frame source absence. The scalar noise RNG consumes only coded PNS
bands in actual wire order; per-tag prediction and IMDCT states pause while
that source is absent. Dynamic PCE preserves target layout and histories.

Six enabled integration tests pass for every case: independent core plus
qualified SBR/PS waveform composition, omitted-PNS-reset control, source FIL
binding sensitivity, rollback/checkpoints, reset, all target frame indices
including EOF, export, syntax probe, interval, rewind and seek. Static/dynamic
roster PCM is identical. Discarding absent source DSP is numerically observable,
and frames with no sources equal target-only PS PCM; the right target channel
remains unaffected by point-3 coupling. The dynamic/static comparison also
matches source-TNS mode, so different tool programs cannot be confused.

All 90 artifacts reproduce deterministically and the 16 prior video/core/
control files remain byte-identical. Generator execution remains separate
from tests, with no private media, FFmpeg, foreign decoder or network.
Command: `cargo test --locked --offline --no-default-features --features
media,player --test aac_main_ps_pns` (6 passed, no failures). Production code
is unchanged. This qualifies long-window PNS across the selected compatible
roster schedules only. Short-window PNS, dependent-coupling PNS, wider band
geometry/profiles, additional SBR tools and all-codec parity remain open;
SBR/PS uses previously qualified owned stages, not a new independent oracle.

### AAC Main PS short-window PNS point-3 acceptance (2026-10-09)

A separate 56-video matrix qualifies PNS in grouped eight-short windows and
long/start/short/stop source transitions, with distinct schedules for CCE
tags 1/15. Source clocks pause on single/simultaneous absence and late
arrival; initially empty, static and dynamic PCE variants combine with
source-SBR on/off and 24/48 kHz output. Optional directional first-order TNS
runs on non-short windows only; this does not qualify short-window TNS.
A scalar global noise generator follows actual source wire order and generates
one independently normalized noise band per short window. Scalar per-tag
Main prediction resets all history on short windows before subsequent long
prediction; direct IMDCT/overlap supplies the independent core reference.
Qualified owned SBR/PS stages remain the extension composition reference.

The wrong core control retains old long-window predictor state throughout
short windows, without applying prediction to short PCM. It changes final
PCM in every case. A temporary production mutation omitting `bank.short_window()`
fails waveform at sample 12122 for 24 kHz upsampling. Production source is
restored byte-for-byte. Acceptance also covers checkpoint/rollback including
noise RNG, reset, all target frame indices/delayed EOF, export, syntax probe,
intervals, rewind and seek; static/dynamic PCM equality, absent-DSP discard,
source FIL binding and target-only empty frames remain checked.

Generator `scripts/generate_aac_main_ps_pns_short_fixtures.py` reproduces 90
artifacts deterministically, separately from ordinary tests, without private
media, FFmpeg, foreign decoders or network. Prior fixture sets are unchanged.
This covers one grouped short-window geometry for mono Main PS independent
coupling point 3. Other grouping patterns, short-window TNS, dependent-coupling
PNS, broader band/profile/layout geometry and all-codec parity remain open.

Validation: 12 integration tests passed across `aac_main_ps_pns_short`,
`aac_main_tools` and `aac_main_ps`, locked/offline with `media,player`
and no FFmpeg. Existing TNS order-21 coverage remains a refusal test,
not order-21 acceptance. Production code is unchanged.

### AAC Main PS dependent-coupling PNS/TNS acceptance (2026-10-09)

16 authored MP4 videos qualify two PNS-switching Main CCE sources at dependent
coupling points 0/1, source TNS on/off, target TNS on/off and 24/48 kHz PS
output. Sources use distinct residuals and noise energies, alternating wire
order and long sine/KBD windows. Independent scalar reconstruction applies
wire-order PNS and per-tag Main prediction/reset, directional source TNS,
f32 spectral addition, target TNS at the selected point and direct IMDCT.
The qualified owned PS stage supplies extension composition reference.

A first draft used exactly opposite residuals, causing omitted-reset errors
to cancel in the mixed core. The distinct-source fixture replaces that weak
control: skipped PNS line reset now changes every core oracle. Acceptance
verifies all 16 waveforms, checkpoint/rollback/reset, delayed EOF indices,
export, probe, interval, rewind and seek. Target TNS distinguishes points 0/1
at both rates with source TNS enabled and disabled. A temporary production
mutation reversing point selection fails waveform at sample 751 in
`0-1-0-24000`; production source has been restored byte-for-byte.

Generator `scripts/generate_aac_main_ps_dependent_pns_fixtures.py` reproduces
34 artifacts deterministically, separately from ordinary tests, without
private media, FFmpeg, foreign decoders or network. Prior fixture sets are
unchanged. This matrix covers continuously coded, long-window mono Main PS
dependent sources with first-order TNS and one two-band PNS geometry.
Dependent short-window/grouping PNS, source absence/PCE changes, wider
profiles/layouts and all-codec parity remain open. It does not establish a
new independent numerical SBR/PS oracle.

Validation: the offline media/player integration run passed 10 tests across `aac_main_ps_dependent_pns`, `aac_main_ps`, and `aac_main_tools`; no failures. Production source was restored byte-for-byte after the mutation check.

### AAC Main PS dependent PNS source absence/arrival (2026-10-09)

The owned generator `generate_aac_main_ps_dependent_pns_absence_fixtures.py`
authors 16 short synthetic MP4 videos: dependent coupling points 0/1,
24/48 kHz output, optional directional source TNS and target TNS.
Tag 15 first appears on packet 2; tag 1 is absent on packets 6–8 and returns
on packet 9 after its earlier PNS bands. Target and source wire order alternate.
The scalar oracle advances global noise only for actually coded sources and
preserves independent Main prediction histories across absent packets. It
computes source/target TNS, f32 spectral sums and direct IMDCT overlap itself.
The target PS composition uses the separately qualified owned PS stage; this
is not a new independent numerical oracle for PS. Generation is separate from
ordinary offline tests and uses no FFmpeg, network or private media.

The acceptance covers waveform, packet checkpoint/rollback, reset/EOF,
MP4 export, probe replay, interval crop, rewind and seek. Short-window dependent
PNS, grouped bands, target layout transitions and wider codec profiles remain
separate qualification work; this matrix does not prove full AAC parity.

Sensitivity control: temporarily clearing absent CCE states in the production
PS decoder failed waveform acceptance in case `0-0-0-24000`, sample 19275
(`0.0001914204` versus `0.00019121692`), after tag 1 returned. The source was
restored byte-for-byte. All 34 generated artifacts reproduced identical SHA-256
hashes on regeneration.

Validation: restored offline media/player run passed 14 tests across dependent PNS absence, dependent PNS, Main/PS and Main tools; zero failures.

### AAC Main PS dependent short-window PNS acceptance (2026-10-09)

The owned `generate_aac_main_ps_dependent_pns_short_fixtures.py` authors
16 synthetic MP4s with shared long/start/eight-short/stop transitions for target
and two dependent CCE sources. It spans coupling points 0/1, 24/48 kHz output,
and directional source/target TNS on non-short packets. Both sources use distinct
spectral residuals; global PNS consumes each coded short window in wire order.
The scalar oracle computes Main prediction, per-source short-window history
reset, PNS, f32 spectral mixing, optional TNS, direct short IMDCT/windowing and
overlap. Its negative control omits short-window predictor reset and resumes
with the old long-window history. Target PS uses the separately qualified owned
PS composition stage; this is not independent numerical PS qualification.

Acceptance checks waveform, checkpoint/rollback, reset/EOF, root export,
probe replay, interval crop, rewind and seek. The short windows form one group;
other grouping patterns, short-window TNS, source absence during dependent
window transitions and wider profiles remain separate qualification work.
No private source media, foreign decoder, FFmpeg or network is used, and
fixture generation remains separate from ordinary tests.

Sensitivity control: temporarily omitting `bank.short_window()` in the owned
Main channel decoder failed dependent waveform acceptance in case
`0-0-0-24000`, sample 13834 (`-0.0009294358` versus `-0.00092922425`),
after long-window prediction resumed. Shared production source was restored
byte-for-byte before final verification.

Validation: 34 generated artifacts reproduced identical SHA-256 hashes. The restored offline media/player run passed 14 tests across dependent short PNS, dependent PNS absence, Main/PS and Main tools; zero failures.

### AAC Main PS dependent grouped short PNS (2026-10-09)

`generate_aac_main_ps_dependent_pns_grouped_fixtures.py` authors 16 synthetic
MP4 videos with unequal short-window groups in target and dependent CCEs.
The first short packet uses tag 1 groups [1,3,4], tag 15 [2,1,2,3] and target
[3,2,3]; the next uses eight singleton windows, [4,4], and [8], respectively.
Each source alternates ordinary/PNS and PNS/ordinary band layouts between
groups. The own writer serializes group/band/window order and independent
scalar noise reconstruction maps each group into physical window spectra.
Main reset, optional non-short source/target TNS, f32 spectral mixing and direct
IMDCT overlap compose the mono core oracle. Target PS uses the separately
qualified owned stage; the oracle is independent for the core, not PS DSP.

The matrix spans coupling points 0/1 and 24/48 kHz, with checkpoint/rollback,
EOF/reset, export, probe replay, interval crop, rewind and seek acceptance.
These specific nonuniform layouts do not qualify all 128 grouping masks or
short-window TNS. Fixture generation remains separate from tests; no FFmpeg,
foreign decoder, network or private media is used.

Sensitivity: reversing all physical windows was rejected by a different special
band layout and is not waveform evidence. Reversing windows within each group
kept band layouts valid and failed PCM acceptance in case `0-0-0-24000`,
sample 8466 (`0.00026189064` versus `0.0002621318`). Production ICS source was
restored byte-for-byte. Regeneration reproduced identical hashes for all 34
artifacts.

Validation: restored offline media/player run passed 14 tests across grouped dependent PNS, dependent short PNS, Main/PS and Main tools; zero failures.

### AAC Main PS dependent PNS all grouping masks (2026-10-09)

The owned `generate_aac_main_ps_dependent_pns_all_groups_fixtures.py` writes
four synthetic MP4s spanning coupling points 0/1 and 24/48 kHz. Each has 136
core packets: long/start, 128 consecutive short packets, stop and long-window
prediction return. Tag 1 visits masks 0–127, tag 15 their complement, and target
a cyclic permutation. Alternating ordinary/PNS band layouts make group order
visible in PCM. A separate Rust test reads ICS at each actual coded bit offset,
reconstructs its grouping mask, and asserts all 128 masks for each element.

An independent scalar oracle reconstructs noise in group/band/window order,
Main short-window reset, f32 dependent spectral addition and direct IMDCT
window overlap. The target PS stage remains a composition of the separately
qualified owned DSP rather than an independent PS numerical oracle. Acceptance
covers whole-flow PCM, checkpoints/rollback, EOF/reset, MP4 export, probe replay,
interval cropping, rewind and seek. The negative core control retains old Main
prediction during short windows. Fixture generation is separate from tests and
uses no private media, foreign decoder, FFmpeg or network.

This supersedes the grouping-mask count gap for this two-band, two-source Main/PS
dependent PNS setup. It does not qualify every band count, source absence/window
transition combination, short TNS, profile or layout, or complete codec parity.

Sensitivity: reversing window order within each physical group in production
ICS deinterleave failed waveform acceptance in case `0-0-0-24000`, sample
8466 (`0.00026192036` versus `0.00026214647`). The shared source was restored
byte-for-byte. All 10 generated artifacts reproduced identical SHA-256 hashes.

Validation: restored offline media/player run passed 14 tests across all grouping masks, nonuniform groups, Main/PS and Main tools; zero failures.

### AAC Main PS dependent short-window TNS/PNS (2026-10-09)

The owned `generate_aac_main_ps_dependent_pns_short_tns_fixtures.py` authors
16 synthetic MP4s spanning source/target TNS on/off, coupling points 0/1 and
24/48 kHz. Each of eight short windows carries a first-order filter with a
three-bit coefficient and direction alternating by window; tag 15 reverses
tag 1's direction pattern. Nonuniform target/source groups and alternating
ordinary/PNS band layouts remain independent of TNS windows. The scalar core
oracle computes separate recurrence histories per physical short window,
source TNS before coupling, target TNS at the correct coupling stage, f32 sums,
Main reset, direct IMDCT and overlap. Target PS still uses the separately
qualified owned PS DSP; this is not an independent numerical PS oracle.

Waveform, checkpoint/rollback, EOF/reset, MP4 export, probe replay, interval
crop, rewind and seek acceptance cover the matrix. Generation remains separate
from tests with no private media, FFmpeg, foreign decoder or network. Higher
short TNS orders/resolutions/compression, all group/TNS combinations and wider
AAC profiles/layouts remain separate qualification work.

Sensitivity: bypassing only short-window TNS in production channel reconstruction
failed target-only TNS case `0-1-0-24000` at sample 8442 (`0.00020383533`
versus `0.00020408037`). Non-short TNS remained enabled. Shared production
source was restored byte-for-byte. All 34 artifacts reproduced identical hashes.

Validation: restored offline media/player run passed 14 tests across short-window TNS/PNS, grouped dependent PNS, Main/PS and Main tools; zero failures.

### AAC Main PS short TNS orders/resolution/compression (2026-10-09)

The owned `generate_aac_main_ps_dependent_pns_tns_orders_fixtures.py` writes
16 short synthetic MP4s with four consecutive short packets covering TNS
orders 1–7, resolutions 3/4 and compression off/on: all 28 combinations.
Coefficients use both signs, with direction alternating by physical window
and opposite patterns in two dependent CCE sources. Source/target TNS on/off,
coupling points 0/1 and 24/48 kHz remain in the matrix, together with nonuniform
window groups and alternating PNS bands. The independent scalar core oracle
converts authored reflection coefficients to LPC, runs per-window recurrence,
performs f32 spectral addition and direct IMDCT/window overlap, with Main reset.
Target PS uses separately qualified owned DSP; this does not independently
qualify PS numerics.

A syntax companion and Rust test read the actual order/resolution/compression
bits and assert all 28 combinations, finite parsed LPC and both coefficient
signs. Waveform, checkpoint/rollback, EOF/reset, export, probe replay, intervals,
rewind and seek cover the whole flow. All fixture generation stays separate
from ordinary tests and uses no private media, foreign decoder, FFmpeg or
network. Broader coefficient magnitudes, band lengths, profile/layouts and
all grouping/TNS Cartesian combinations remain separate qualification work.

Sensitivity: truncating short-window LPC to one coefficient after fully reading the syntax failed waveform acceptance: `"0-1-0-24000" sample 8451: -0.000034607547 vs -0.00003488504`. Production TNS source was restored byte-for-byte. All 36 generated artifacts reproduced identical hashes.

Validation: restored offline media/player run passed 15 tests across short TNS orders/resolution/compression, first-order short TNS/PNS, Main/PS and Main tools; zero failures.

### AAC LTP syntax foundation and playback gap (2026-10-09)

The owned `aac_ltp_syntax` module parses ordinary LTP side information after
its presence flag: lag/coefficient index and long-band or short-window usage.
It validates frame geometry and lag, and commits the bit cursor only on success.
It does not implement ER/LD lag-update syntax or change profile admission.
The independent authored fixture generator supplies 240 unaligned cases across
960/1024 frame lengths, eight coefficient indices, lag boundaries, band counts,
and short-window optional lag fields, plus malformed geometry/lag/truncation.

Two synthetic MP4s reproduce the existing exact AOT4 configuration refusal,
with active and inactive LTP flags. A passing refusal test is not playback
acceptance. The intended playback test is explicitly ignored until LTP signal
history, prediction, forward transform/TNS integration and profile admission
are implemented; independent PCM qualification will also be required.
Fixtures are generated separately from ordinary offline tests with no private
media, FFmpeg or external decoder execution.

Bit-field reference consulted: [FAAD2 Table 4.4.28 implementation](https://github.com/knik0/faad2/blob/master/libfaad/syntax.c)
and [40-band LTP syntax bound](https://github.com/knik0/faad2/blob/master/libfaad/structs.h).
These are syntax references, not code or runtime dependencies.

Validation: all five artifacts regenerate identically. Owned-library tests: 461 passed, 0 failed, 1 pre-existing ignored. Offline media/player integration: 8 passed, 0 failed, 1 explicitly pending LTP playback acceptance. Syntax acceptance and configuration-refusal reproduction do not establish LTP playback.

### AAC LTP owned time-domain history foundation (2026-10-09)

`aac_ltp_history::LtpHistory` owns a bounded four-frame signed-16-bit history
for 960/1024 core samples. Updates consume raw synthesis PCM/overlap before
normalization, using saturation and nearest-even integer rounding. Estimate
preparation uses lag and one of eight ordinary LTP gains to produce two-frame
long-window analysis input. The future tail remains zero. Reset and matching
checkpoint restore are allocation-free; invalid updates, estimates and restores
leave state/output intact. Short-window analysis explicitly refuses.

`SynthesisHistory::overlap_raw` exposes a borrowed raw overlap snapshot for
future decoder integration. It does not advance synthesis or normalize samples.
The independent Python oracle uses piecewise physical time rather than a
production buffer shift and covers past PCM, overlap, zero future, all gains,
clipping/ties and lag boundaries including 2047. A separate test feeds actual
owned synthesis PCM/overlap into the history. Generation is separate from tests
and uses no private media, foreign codec execution, FFmpeg or network.

This is a required LTP computation stage, not LTP playback acceptance. Forward
analysis/window selection, TNS analysis, band application, transactional decoder
history and AOT4 admission remain unintegrated; the two existing synthetic LTP
videos still reproduce configuration refusal and playback acceptance is ignored.
Gain/history layout reference consulted: [FAAD2 LTP reference](https://github.com/knik0/faad2/blob/master/libfaad/lt_predict.c).
No external decoder code or dependency is imported.

Sensitivity: replacing the previous-PCM carry with zeros failed the independent history oracle at n=960, lag=1920, coefficient=0. Source was restored. All three artifacts reproduce identical hashes and cover 216 full two-frame estimates, including the separately encoded 2047 lag boundary.

Validation: owned-library tests passed 461 with zero failures and one pre-existing ignored. Offline media/player integration passed 11 with zero failures and one pending LTP playback acceptance ignored. The chosen raw-signal scale and integer rounding still require end-to-end LTP PCM qualification when analysis/synthesis is integrated.

### AAC LTP owned FFT forward analysis foundation (2026-10-09)

`Imdct::forward_with_scratch` now folds the 2N input into the DCT-IV domain
and reuses the owned chirp convolution/FFT tables for an unnormalized cosine-sum
MDCT. It is O(N log N), accepts caller-owned scratch, and validates geometry,
finite input and transformed output before publishing any output samples.
An independent dense direct-cosine unit test covers all six supported transform
lengths (32,120,128,256,960,1024) and preserves output on bad scratch geometry.

`aac_ltp_analysis::LtpAnalysis` applies previous/current sine/KBD windows for
only-long, long-start and long-stop at 960/1024 geometry, then runs the forward
transform with retained scratch and no per-call allocations. It does not
advance decoder history; short-window analysis explicitly refuses. A separate
own generator creates 24 sparse boundary-signal full-spectrum references and
four dense harmonic selected-bin references using scalar windows/direct cosine
sums. Acceptance checks repeated calls, invalid input, overflow, short refusal
and recovery without corrupting caller output.

This remains a required computation stage, not LTP playback. TNS analysis,
selected-band application, profile/ICS admission and transactional decoder state
remain unconnected, as does final end-to-end scale/rounding qualification. The
existing synthetic LTP videos still reproduce the explicit AOT4 refusal and
playback acceptance remains ignored. Generation is separate from tests and uses
no private media, foreign decoder execution, FFmpeg or network.
Window-sequence reference consulted: [FAAD2 LTP filterbank](https://github.com/knik0/faad2/blob/master/libfaad/filtbank.c).
No external decoder code/dependency is imported.

Sensitivity: substituting current shape for previous shape failed the independent analysis oracle: `n=960 seq=OnlyLong bin=0: -1024.3450776386292 vs -1026.0142568988708`. Source was restored. All three generated artifacts reproduce identical hashes.

Validation: restored owned-library tests passed 462 with zero failures and one pre-existing ignored. Offline media/player integration passed 13 with zero failures and one pending LTP playback acceptance ignored. All six forward-transform sizes passed dense direct-cosine comparison.

### Owned AAC TNS analysis for LTP prediction spectra (2026-10-09)

`TnsData::analyze_owned` applies the all-zero/FIR analysis counterpart of the
existing TNS synthesis filter to an owned f64 spectrum. Each filter keeps
original-input history, resets at its own band/window boundary and respects
direction plus clipped spectral-band intervals. Geometry/finite LPC validation
precedes processing; overflow refuses. The buffer is consumed and reused with
no second frame allocation. This is a computation stage, not AOT4 admission.

The own generator supplies 96 full-spectrum direct-convolution references over
960/1024 geometry, long/eight-short windows, two filter intervals, both directions,
clipping limits 0/2/5 and orders through 20 (generic Main-capable TNS data;
profile-specific limits still belong to syntax admission). Tests compare FIR
results independently, check synthesis reversibility within f32 precision,
retained pointer/capacity and malformed/overflow refusal. They do not prove all
possible LPC coefficients or geometry. Generation is separate from tests and
uses no private source media, foreign decoder execution, FFmpeg or network.

LTP still needs selected-band application, profile/ICS admission, decoder state
integration and end-to-end scale/rounding/PCM acceptance. Existing synthetic LTP
videos retain exact configuration refusal; intended playback remains ignored.
Analysis recurrence reference consulted: [FAAD2 TNS analysis](https://github.com/knik0/faad2/blob/master/libfaad/tns.c).
No external code/dependency is imported.

Validation: 462 owned-library tests and 15 offline media/player integration tests passed, with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. All three generated artifacts retained identical SHA-256 hashes after regeneration. A temporary incorrect feedback-history mutation failed the independent FIR comparison at n=960, sample=0; the original implementation was restored before these final checks.

### Owned AAC LTP selected-band spectral addition (2026-10-09)

`apply_long_prediction` adds a previously TNS-analyzed prediction to only the
signaled long-window spectral bands. It checks frame geometry, strictly ordered
band offsets, finite operands and the ordinary LTP 40-band limit. All selected
sums are checked for f32 range before any residual write; a late overflow leaves
the caller's complete residual unchanged. No frame allocation occurs. Inputs
must already share normalization; this helper does not establish that contract
for the decoder's eventual LTP integration.

The own offline generator supplies 512 per-bin interval-union references: all
256 masks over eight unequal bands for both 960 and 1024 samples. Exact binary
fractions avoid rounding ambiguity. Tests include the first/last bin, untouched
bands, the 40-band ceiling, malformed geometry, nonfinite values, a late overflow
and reuse after refusal. These are spectral-stage acceptance tests, not video
playback acceptance. The existing own active/inactive LTP MP4 reproducers remain
configuration refusals and the playback acceptance remains pending.

Reference for ordering (analysis filterbank, TNS analysis, selected-band addition):
[FAAD2 LTP](https://github.com/knik0/faad2/blob/master/libfaad/lt_predict.c).
No external implementation or runtime dependency is imported. Profile admission,
channel-pair syntax, transactional decoder history, normalization/rounding and
independent end-to-end PCM qualification remain required.

Validation: 462 owned-library and 17 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. The own 512-case band fixture regenerates with an identical SHA-256 hash.

### Owned ordinary LTP ICS and common-window pair syntax (2026-10-09)

`LtpIcsInfo::read` parses the complete ordinary AOT4 ICS header transactionally,
including independent left/right LTP presence and data in a common-window pair.
Short windows consume only grouping, without long-window predictor flags.
Main predictor state remains separate. Geometry, reserved-bit, band-limit or
nested predictor failures leave the caller cursor unchanged. ER/LD syntax and
production configuration admission are not enabled by this parser.

The own generator authors 896 ICS cases over 960/1024, long/start/stop, sine/KBD,
0/1/40/63 bands, independent pair presence combinations, and every one of 128
short grouping masks for single/common-window modes. Tests verify exact cursor
and a following section sentinel, predictors, groups, all truncated byte prefixes
that end within the ICS, and band-limit rollback. Existing active/inactive own
LTP video packet ICS headers also parse correctly and leave the following
spectral-section codebook aligned. Their decoder playback remains a configuration
refusal; the ignored PCM acceptance is not enabled prematurely.

Syntax reference consulted: [FAAD2 ICS Table 4.4.6](https://github.com/knik0/faad2/blob/master/libfaad/syntax.c).
No external source implementation was copied or linked. Decoder channel-state
integration, normalization/rounding and independent end-to-end PCM qualification
remain required before AOT4 admission.

Validation: 462 owned-library and 20 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. Both ICS artifacts regenerate with identical SHA-256 hashes.

### Composed owned LTP spectral prediction pipeline (2026-10-09)

`LtpAnalysis::predict_long` composes the owned i16 history estimate, shaped forward
MDCT, optional TNS FIR analysis and selected-band addition. History is borrowed
and never advanced; all intermediate spectra are packet-local. A nested TNS or
geometry failure leaves the caller residual unchanged. This initial composition
allocates bounded 2N and N temporary arrays; retained scratch/accounting must be
handled at production decoder integration. The API explicitly uses matching raw
synthesis/MDCT units, not a claim of normative AAC PCM scale or AOT4 admission.

The own generator supplies 24 independent full-spectrum references across both
frame sizes, long/start/stop and all previous/current sine/KBD combinations.
It authors sparse physical previous/current PCM and overlap, quantizes independently,
uses direct cosine sums, original-input FIR convolution in two opposite-direction
intervals and an interval-union selection oracle. Tests check the combined result,
repeated-call identity, unchanged history, nested TNS refusal and mismatched
history geometry. Fixtures regenerate separately, without private media, external
codecs, FFmpeg or network. These spectral-stage tests do not replace the pending
video PCM acceptance or prove the history normalization/rounding contract.

Ordering reference: [FAAD2 LTP/filterbank](https://github.com/knik0/faad2/blob/master/libfaad/filtbank.c).
No external implementation is copied or linked. Production channel/pair state,
profile dispatch, memory accounting and independent end-to-end PCM qualification
remain required before enabling the existing LTP playback acceptance.

Validation: 462 owned-library and 22 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. All three composite-pipeline artifacts regenerated with identical SHA-256 hashes.

### Owned LTP channel histories and scalar PCM chain (2026-10-09)

`LtpChannel` joins spectral prediction, TNS synthesis, window synthesis and LTP
history updates. Checkpoints contain both synthesis overlap and quantized history
plus the previous window shape. Reset restores zero histories and sine shape;
mismatched checkpoint geometry refuses before mutation. Processing advances
histories only after all stages succeed, using temporary cloned state in this
initial adapter. It returns the existing owned 1/65536-normalized PCM convention.
This is not yet production AOT4 profile admission or proof of normative LTP scale.

The independent own scalar generator authors 12 sequential PCM frames across
960/1024, alternating sine/KBD, inactive then active LTP, signed large residuals,
clipped/rounded history and both TNS directions. It uses direct cosine sums and
scalar original-input FIR/feedback AR recurrences, distinct from the FFT adapter.
Tests compare full PCM, checkpoint replay, reset, refusal rollback and mismatched
restore. Generation is separate from ordinary offline tests and uses no private
media, FFmpeg, network or foreign decoder. The existing two own LTP MP4s remain
production configuration refusals, with intended video playback still pending.

Before decoder admission, normative history/PCM scaling and rounding require
external-reference qualification, followed by production channel/pair dispatch,
CCE/PCE interactions and memory accounting. The adapter currently clones channel
state and allocates temporary buffers; it does not claim production performance.

Validation: 462 owned-library and 24 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. Both channel artifacts regenerate with identical SHA-256 hashes.

### LTP floating-history external-reference correction (2026-10-09)

An explicit FFmpeg reference benchmark of the existing own inactive/active LTP
MP4s found inactive PCM peak error 2.6021e-11, but active error 5.2731e-6 with the
initial integer-history channel. The former validates base synthesis scaling on
this fixture; the latter is a specific active-prediction reproduction, not a
claim of general codec acceptance. The same own videos reproduce the issue;
no private source was added.

`LtpHistory::new_float` preserves fractional raw synthesis values without i16
rounding/saturation and is now used by `LtpChannel`. The original fixed-history
constructor and its independently qualified integer behavior remain separate.
Restore refuses a precision-mode mismatch. Nonfinite updates and floating
estimate overflow refuse before state/output mutation. The own channel scalar
oracle was updated explicitly to floating history, rather than retaining the
old integer convention as expected behavior. Fixed-history stage references
remain unchanged.

`scripts/benchmark_aac_ltp_reference.py` is the explicit external benchmark;
ordinary tests only read its saved f32 PCM and version/source/reference SHA-256
manifest. The acceptance covers the channel chain with the known authored
residuals of these two 24k/1024 mono long-window videos, not production packet
parse/dispatch, other tools/layouts or all LTP profiles. Full root playback
acceptance remains pending. External implementation details consulted:
[FFmpeg LTP float DSP](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/aac/aacdec_dsp_template.c).
No external source code was copied or runtime decoder dependency added.

Validation: after the floating-history fix, active fixture PCM peak error fell to 3.37394e-10 (inactive remains 2.60203e-11), passing the unchanged 1e-7 tolerance. 462 owned-library and 26 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending root LTP playback acceptance remain ignored. Reference source/PCM hashes were verified against the benchmark manifest; both own channel artifacts regenerated identically.

### Owned ordinary LTP individual-channel packet reader (2026-10-09)

`ChannelData::read_ltp` consumes global gain, ordinary AOT4 ICS/LTP, sections,
scalefactors, pulse, TNS, gain-control and spectral Huffman payload through the
shared channel-body parser. Whole-channel failures restore the original cursor;
non-AOT4 calls refuse. Geometry tables now accept AOT4 for this explicit reader
and reconstruction; production ASC/profile dispatch remains unchanged and gated.
The refactor shares the existing Main/LC/SSR body rather than duplicating tools.

The offline acceptance reads all 24 channel packets from the existing own active
and inactive LTP video fixtures, verifies element/tag and the following END code,
reconstructs the actual coded spectrum and runs owned channel synthesis against
the saved external PCM. Truncated channel prefixes and incorrect profile calls
must preserve the cursor. This replaces metadata-authored residuals with actual
packet spectral parsing for this path. Tests do not execute FFmpeg or use network.

This is individual-channel adapter acceptance, not root-container/player AOT4
acceptance. Common-window/non-common pairs, CCE/PCE state mapping, production
checkpoint dispatch and memory accounting remain required at integration. The
existing root LTP refusal and ignored root playback acceptance remain explicit.

### SSR regression helper EOF-drain correction (2026-10-09)

The expanded channel-body regression found the SSR playback helper returning
20480 bytes against the 24576-byte export: all common bytes were identical, with
only the final queued frame absent. The failure reproduced with the previous
channel/body implementation restored, so it was not introduced by LTP parsing.
Production `audio_thread` already calls `finish_packet` at EOF. The regression
helper now drains that same API and checks repeated EOF is empty before comparing
full, rewind and seek output. The existing short own SSR video supplies the exact
reproducer; no private media or replacement tolerance was introduced.

Validation: 462 owned-library tests, four root channel unit tests and 51 offline integration tests across 13 suites passed with zero failures. One pre-existing library test and 1 integration test(s) remain ignored, including pending root LTP playback acceptance. The previously failing SSR helper now passes full/rewind/seek/ranges with exact PCM equality after EOF drain.

### Owned LTP CPE packet parsing and independent channel state (2026-10-09)

`ChannelPair::read_ltp` parses ordinary AOT4 common-window or independent-window
pairs, returning independent LTP data for both channels. Common MS masks use the
same helper as existing Main/LC/SSR pair syntax; either stream's failure restores
the entire pair cursor. The prediction data is kept separate from common ICS
geometry, so different channel lag/coefficient/usage does not break stereo tools.
Production AOT4 ASC admission/dispatch remains gated.

The own generator authors four short 24k/1024 stereo LTP videos: common MS off,
explicit, all and independent windows. Twelve frames each cover inactive lead-in,
left/right presence changes, distinct lag/coefficient, and sine/KBD shape changes
(shared or independent). The explicit benchmark saves external interleaved PCM,
version and source/PCM hashes. Offline tests parse actual CPE payloads, reconstruct
MS spectra, run independent channel states, compare every sample to an own direct
cosine-sum scalar PCM oracle at 1e-7, verify
END alignment, checkpoint replay and whole-pair rollback for truncated streams,
reserved MS mode and incorrect profile. No private media/codec parameters or
runtime FFmpeg dependency is introduced. Generation and benchmark remain separate
from ordinary tests.

This qualifies these ordinary stereo packet/channel cases, not all LTP layouts,
intensity/PNS interactions, short prediction, 960 external-reference behavior,
CCE/PCE mapping or root player/MP4 admission. The original root LTP playback
acceptance still awaits production integration and memory accounting.

The three common-window cases also match every saved external PCM sample at
1e-7. Independent-window frames 0–4 and the complete left channel match that
reference; right-channel frame 5 diverges by 4.8556743e-4, and frame 6 by
4.4325036e-4. The offline regression retains and reproduces this disagreement
while requiring all independent-channel PCM to match the scalar oracle.
FFmpeg's [LTP dispatch](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/aac/aacdec.c)
gates both channels on the left predictor flag and does not clear LTP presence
when an independent ICS lacks prediction; this explains the observed right-channel
reference limitation. [FAAD2 stereo reconstruction](https://github.com/knik0/faad2/blob/master/libfaad/specrec.c)
applies separate channel predictors. FVid retains independent channel flags;
the saved external output is diagnostic evidence, not an acceptance oracle for
these divergent frames. The scalar reference uses authored residuals, direct
MDCT/IMDCT sums, float history, and independently constructed sine/KBD windows.

Validation: owned library 462 passed / 1 pre-existing ignored; root AAC pair
5 passed; fourteen offline AAC integration suites 53 passed / 1 pending root
LTP acceptance ignored. Own pair generation is deterministic and all saved
benchmark source/reference SHA-256 hashes are verified.

### LTP retained storage and shared analysis tables (2026-10-09)

`LtpChannel` now exposes retained-payload inspection, including its optional
packet-boundary checkpoint. Analysis sine/KBD windows and immutable forward
transform tables are shared by cloned channels; histories and analysis/synthesis
scratch remain independent. Both live and saved history capacities are counted.
Checkpoints retain five N-sized f64 buffers (four LTP history blocks plus one
synthesis overlap), without duplicating transform scratch or windows.

The internal visitor composes with native decoder accounting, and the bounded
shared-table workspace allows nine allocations per LTP channel/coupling state.
Tests cover 960/1024 geometry, exact incremental mutable storage for clones,
checkpoint storage, spare analysis/window capacity, independent history,
processing, restore and reset. Inspection excludes allocator/Arc control headers,
stack objects, packet temporaries and caller-owned PCM; this is retained heap
payload, not a new memory cap, peak decode budget or process RSS estimate.
Production AOT4 dispatch/admission and adding LTP fields to the native decoder's
checkpoint/visitor are still pending, as are remaining tool/layout qualification.

Validation: owned library 465 passed / 1 pre-existing ignored; eight offline
LTP/TNS integration suites 23 passed / 1 pending root LTP acceptance ignored.
All PCM oracles and tolerances remain unchanged.

### Native LTP SCE/CPE state and packet dispatch (2026-10-10)

The native AAC decoder now owns specialized per-output-channel LTP states,
reads SCE/LFE and common/independent CPE prediction data, and routes reconstructed
spectra through LTP analysis, TNS and synthesis. The generic pre-synthesis TNS
pass is bypassed for AOT4 to avoid applying TNS twice. Packet-boundary checkpoints
save only LTP/synthesis histories; reset, restore, retained-memory inspection and
late processing failure rollback include the new state. Initialized channels
share immutable tables and keep mutable history/scratch independent.

Tests invoke the same private parsed-metadata constructor used by `new`, with
own 24k/1024 LTP metadata, while public ASC admission remains unchanged. Both
mono streams and all four stereo streams pass actual raw packet dispatch against
saved external or independent scalar PCM at 1e-7, with exact checkpoint replay,
timed packet stamps, reset, END validation and stable retained storage. A controlled
internal right-lane geometry error verifies rollback after the left lane advances;
this is state-transaction qualification, not a reachable-input media reproducer.
The native joint checkpoint footprint is checked against lightweight histories
and existing metadata capacities.

This completes the native state/packet-dispatch stage for those fixtures, not
production LTP playback acceptance. Public AOT4 ASC admission remains gated;
LTP CCE syntax/history, dependent coupling order, PCE/layout qualification,
PNS/intensity interactions, short/transition and 960 packet cases, extension
profiles and end-to-end root playback still require integration/qualification.
Existing root refusal and pending acceptance expectations remain unchanged.

Validation: owned library 468 passed / 1 pre-existing ignored; root native AAC
15 passed, including the 72-packet compatibility bridge test; fourteen offline
AAC integration suites 53 passed / 1 pending root LTP acceptance ignored.
The final syntax suite also retains 5 passing tests and the pending acceptance;
its ignore reason now names ASC admission and remaining tool/layout qualification.
Root syntax metadata is explicitly adapted to owned LTP data; the library path
borrows it directly. Existing synthetic packets/PCM references are unchanged.

### Ordinary LTP CCE syntax and gain-list qualification (2026-10-10)

`Coupling::read_ltp` preserves AOT4 source prediction data separately from its
channel and target gain lists, while sharing the existing Main/LC/SSR CCE payload
parser. Whole-element cursor commit occurs only after all spectral/gain syntax
succeeds; an incorrect profile refuses without consuming bits.

The own offline generator creates seventeen 12-frame 24k/1024 synthetic videos
and their 204 raw CCE packets: points 0/1/3, mono targets, all four stereo target
selections, separate common gains and signed per-band gain lists. Wire order,
sine/KBD shape, prediction presence, lag/coefficient and band use vary. Both
root and library parsing are checked against authored metadata, exact residual
spectra, gain values, target selections and payload boundaries; truncated CCEs
must leave the complete element cursor unchanged. No private parameters/media,
foreign codec or FFmpeg/network dependency is used by generation or tests.

This is CCE parsing acceptance, not native coupling playback acceptance. A
separate private-metadata native test reproduces the existing specific LTP
coupling-state refusal on these same packets and verifies retained payload,
noise state and deterministic history probes are unchanged by the failed decode.
Checkpoint restoration may shrink spare metadata vector capacity, so refusal
memory inspection compares the state immediately before and after each attempt.
Public AOT4 ASC refusal and ignored root playback acceptance remain unchanged.

Native integration still needs source tag histories, prediction/TNS phases and
coupling ordering, history advancement rules, CCE snapshots/memory, and scalar
coupled-PCM qualification. [The primary dispatch reference](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/aac/aacdec.c)
places target point-0 mixing before prediction/TNS, and point-1 mixing after TNS;
the combined LTP channel adapter must be split to support those phases. No
reference decoder implementation was copied into production.

Validation: owned library 469 passed / 1 pre-existing ignored; fifteen offline
AAC integration suites 55 passed / 1 pending root LTP playback acceptance ignored.
The final two-test CCE suite also verifies that the legacy ICS path rejects the
same active-LTP source presence bit with its exact prediction diagnostic while
the new CCE reader accepts the packet. Nineteen generated artifacts are
deterministic; existing PCM references and tolerances are unchanged.

### LTP preparation/synthesis phases for coupling (2026-10-10)

`LtpChannel::prepare_spectrum` performs prediction analysis and TNS without
advancing PCM history or previous shape. `synthesize_spectrum` accepts the
prepared/coupled spectrum and commits both histories and shape only after all
synthesis stages succeed. The existing combined `process` delegates to these
stages. Preparation rejects non-finite raw spectra, including the tool-free path.
The caller retains the sequence/shape between stages and owns the spectral buffer.

Native target preparation now occurs between the existing point-0 and point-1
mixing loops; the final PCM loop only synthesizes the prepared spectrum, avoiding
a second TNS pass. All target preparation precedes PCM commits, so a right-lane
preparation failure leaves left history untouched. Native CCE source/tag state
is still refused, and public AOT4 ASC admission remains unchanged.

The own generator adds three 12-frame coupled-phase videos plus three distinct
target-only controls. Target lag/gain, window shape and directional TNS vary by
coupling configuration; independent source prediction and windows vary separately.
The independent scalar oracle uses direct sparse MDCT/IMDCT sums, sine/KBD,
float history and first-order FIR/AR TNS. Parsed packets exercise staged point-0,
point-1 and point-3 output against every saved PCM sample at 1e-7. A wrong point-1
pre-TNS control must differ by more than 1e-5; repeated preparation, invalid
synthesis, retained storage and checkpoint replay verify history boundaries.
Existing 960/1024 channel references also compare split and combined APIs exactly.
Native private-metadata target-only dispatch matches scalar PCM and checkpoints.

These are staged-channel and uncoupled native acceptance tests; coupled native
playback remains pending source tag histories, coupling dispatch and accounting,
additional layouts/tools and production ASC admission. This does not enable the
ignored root LTP playback acceptance. All media/parameters are authored, generation
is separate from ordinary offline tests, and no FFmpeg/network call is required.

Validation: owned library 470 passed / 1 pre-existing ignored; root native AAC
15 passed; sixteen offline AAC integration suites 56 passed / 1 pending root
LTP playback acceptance ignored. Eleven phase fixture artifacts are deterministic.
Existing PCM references and tolerances remain unchanged. Coupled native dispatch
and public AOT4 admission are still pending; their refusal expectations are kept.

### Ordinary AOT4 admission and long/short LTP transitions (2026-10-10)

Native CCE source tag histories, transactional decode, history checkpoints and
retained accounting were integrated in `6eec840ca`; coupled phase PCM is qualified
against the own direct scalar oracle. Ordinary AOT4 is now admitted by ASC parsing,
native decoders and PCE ASC serialization. Native MP4 playback acceptance replaces
the former exact configuration refusal, and the old ignored playback test is enabled.

Two new authored 12-frame MP4s use 960/1024 samples, long/start/eight-short/stop,
sine/KBD, varying prediction flags, lag, gains and band masks. All eight short
windows contain nonzero residuals; ordinary short ICS carries no prediction flags.
Independent sparse cosine transforms and float-history PCM match both native
parser paths. A stale short-history control must differ by >1e-6, proving the
fixture exercises prediction after the short-frame history transition.
Every frame replays exactly from checkpoints; reset and full rewind are exact.

Public MP4 tests compare every sample of both transition videos, three coupled
phase videos and four stereo pair videos to the saved scalar PCM at 1e-7.
Seventeen PCE/CCE gain-selection videos have admission/finite-output checks;
those checks do not independently qualify every stereo gain PCM combination.
The absent-target video now fails at its specific routing error through public
MP4 decoding, rather than being blocked by ASC. PCE roundtrip preserves AOT4,
layout and coupling. Owned MP4 budget refusal precedes PCM publication; admitted
256 MiB decoding equals the default path.

LTP admission additionally reserves one MiB per output/source state (channels+18),
covering float history, analysis tables/transforms, candidates, snapshots and
scratch above the existing generic reserve. This is conservative controlled
allocation admission, not process RSS or measured peak/60 fps qualification.
LTP SBR/PS combinations, ER/LD/ELD/USAC and broader layouts/tools still need
implementation/qualification; finite AOT4 fixtures do not establish full AAC
conformance. All generation and ordinary tests are offline and FFmpeg-free.

Ordinary short ICS/prediction separation was cross-checked against the
[primary syntax reference](https://github.com/knik0/faad2/blob/master/libfaad/syntax.c).
Only syntax/algorithm behavior was consulted; no foreign decoder code was copied.

Validation: owned library 474 passed / 1 pre-existing ignored; root native AAC
17 passed; seventeen offline AAC integration suites 62 passed / zero ignored.
The final seven-test public LTP suite additionally passed the new exact-range
check (six other tests repeated). Four fixture generators regenerate identically;
existing media, packets and PCM references are unchanged, with only three
provenance strings updated to reflect public admission. No tolerances relaxed.

### LTP SBR signalling/discovery and transport acceptance (2026-10-10)

A new authored implicit-SBR AOT4 packet reproduced the exact missing-discovery
failure at a fixed 24 kHz output clock: `AAC fill extension tool SBR requires
extension-aware stream signalling`. The reproduction passed before the fix.
AOT4 is now included in native fixed-clock/negotiated SBR discovery, ADTS
negotiation and unknown-signalling DSP admission. The refusal expectation is
replaced by actual PCM acceptance. Explicit SBR disable flags remain honored.

Six own six-frame MP4s cover explicit, sync and implicit signalling at 24/48 kHz.
The generator composes sparse direct LTP cosine transforms/float histories with
independent direct SBR QMF convolutions, using only saved numeric protocol tables.
All output samples match the independent float64 reference within 1e-9. A control
with LTP disabled differs by more than 1e-5; measured peaks are about 7.25e-5,
so the acceptance exercises active prediction through SBR rather than noise alone.
Packet checkpoints, reset and replay are exact through all six configurations.
Public root and owned MP4 PCM are identical; repeated/reversed ranges are exact
slices of full PCM. An own unprotected ADTS transport negotiates 48 kHz and
matches explicit MP4 PCM through both APIs, including the active-prediction range.

A separate short own CRC-error video and packet flip one SBR CRC bit while
preserving the payload. Public decoding reports precisely `SBR CRC mismatch`;
private-state tests verify retained storage/clock and subsequent valid PCM remain
unchanged across rollback. Unknown AOT4 admission now reserves SBR DSP before
FIL arrives, with the same estimate as explicit/sync signalling at both clocks.
Generation remains separate; ordinary tests use no FFmpeg, libav or network.

These fixtures qualify one mono 1024-frame LTP/SBR program with sine windows,
varying prediction lag/gain and 16-slot SBR geometry. Broader late-SBR
layouts, wider LTP/SBR
layouts/tools, LTP/PS, additional LTP ADTS combinations and ER/LD/ELD/USAC remain
separate gaps; this does not establish complete AAC conformance or performance.
Primary configuration/processing order was checked against the
[primary AAC dispatch reference](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/aac/aacdec.c).
No foreign decoder implementation was copied.

Validation: owned library 477 passed / 1 pre-existing ignored; root native AAC
17 passed; nineteen offline AAC integration suites 68 passed / 0 ignored.
All fifteen generated artifacts reproduce identical SHA-256 hashes. Existing
PCM references and tolerances are unchanged.

### Active LTP ADTS protection and multiplexed blocks (2026-10-10)

Eight own short companion videos and their protected single-block ADTS transports
reproduced `prediction is not allowed in AAC-LC` before the fix. This was a
transport scanner gap: CRC spans, raw-block boundaries and SBR discovery still
used legacy ICS readers for AOT4. The reproduction expectation is replaced by
actual playback acceptance.

The scanner now dispatches SCE/LFE, CPE and CCE to owned LTP readers. CPE parsing
returns the second ICS span without changing its independent predictor flags or
whole-pair rollback. AOT4 is also included in the bounded implicit-SBR rate probe.
No PCM/predictor state is advanced by CRC or block-boundary scanning.

`generate_adts_ltp_fixtures.py` authors all boundaries from bit-writer lengths and
computes CRC through independent GF(2) polynomial division. It never obtains its
expected spans from the decoder. Eight programs cover common-window stereo with
MS 0/1/2, independent stereo, mono/SBR, and PCE/CCE coupling points 0/1/3 with
alternating element order. The 78 ADTS variants use protected/unprotected groups
of 1/2/3/4 blocks, including mixed 1+2+1+4+4 and six-frame SBR 4+2. Root and owned indexed and
streaming readers expose the exact authored packets and core timestamps. Full
PCM matches companion MP4 and the previously qualified scalar references;
repeated/reversed ranges match exact full-output slices. No tolerance is relaxed.
Eight corrupt transports flip a CRC bit in the fourth, active-LTP block and
must report precisely `ADTS CRC mismatch`; streaming readers then remain poisoned.
SBR discovery is checked from an initial header-bearing block, not from payloads
that require retained header state.

This closes protected/multiplexed transport for these authored 1024-frame
programs, not all AAC profiles/tools/layouts. Broader late-SBR combinations,
LTP/PS, wider SBR geometry,
additional short-window/layout combinations and ER/LD/ELD/USAC remain separate
qualification gaps. Fixtures and ordinary tests need no FFmpeg, libav or network.

Validation: owned library 477 passed / 1 pre-existing ignored; eight offline
integration suites 26 passed / 0 ignored. The expanded 78-transport LTP suite
passed again after adding mixed groups. All 96 fixture artifacts have identical
SHA-256 hashes after regeneration; previously generated binary media and packets
are unchanged. No foreign codec or network is used by generation or tests.

### Late SBR after active LTP, with independent source histories (2026-10-10)

The previously unqualified late-SBR LTP path now has twelve own six-frame MP4s:
core controls and late FIL at fixed 24/48 kHz, direct mono, independent CCE with
silent target, and simultaneously active target/source LTP. Both active lanes
have already advanced prediction history before the first SBR FIL at packet 4.
The independent source receives SBR while the active target continues through
its own full-band QMF upsampler; final mixing preserves separate core histories.
The existing owned decoder passes these paths without production code changes.

Scalar references use direct cosine transforms and float LTP history, followed
by direct QMF convolutions. Each PCM sample agrees within 1e-9. Cold-QMF controls
preserve the current LTP core waveform but remove the pre-FIL QMF input history;
inactive-LTP controls keep the extension while disabling prediction. Both alter
post-transition PCM by more than 1e-7, so silence/noise-only or cold-history
playback cannot satisfy acceptance. Mixed controls apply the same f32 lane
rounding and final independent-coupling addition as the signal contract.

Three own ADTS inputs include indexed mono and in-band PCE bootstrap for each
coupled program. Full decode negotiates 48 kHz and reconstructs its complete
prefix; ranges ending before FIL, including an already-active LTP range, retain
24 kHz core PCM. Repeated/reversed ranges, public root/owned PCM, checkpoint,
reset, EOF, player rewind and seek before/after the first FIL are checked.

Six additional own short videos/packets corrupt only a transmitted SBR CRC bit
in the last frame, after valid core and extension history. Public refusal is
precisely `SBR CRC mismatch`; packet tests then decode the valid frame and
compare it with checkpoint replay and the complete scalar waveform. This
negative check is separate from actual PCM acceptance.

Generation is separate, uses no foreign codec/network, and copies no private
media/parameters. This qualifies ordinary mono 1024-frame sine-window LTP with
16-slot SBR and independent unit-gain CCE at these clocks. Wider layouts,
short-window/other SBR geometry, additional coupling combinations, LTP/PS and
ER/LD/ELD/USAC remain qualification/implementation gaps. No broad conformance or
performance claim follows from these fixtures.

Validation: five offline integration suites passed 23 tests / 0 ignored, including
five new late-LTP-SBR acceptance/refusal tests. All 45 new fixture artifacts
reproduce identical SHA-256 hashes. Production decoder sources are unchanged;
this delivers concrete qualification of existing owned paths, not a new fallback.

### Owned LTP/SBR/PS dispatch and coupling (2026-10-10)

Twenty-four own MP4 configurations reproduced precisely `AAC parametric stereo
core profile is not implemented` before this fix. The previous refusal test is
replaced with actual PCM acceptance. Native PS now uses owned LTP syntax and
channel states for AOT4, with no LC synthesis fallback. The target prediction
is applied between coupling point 0 and inverse TNS, then coupling point 1.
Core synthesis advances its floating LTP history before SBR/PS. Independent
CCE sources retain separate tag-keyed LTP histories and contribute only to final
left-channel PCM; dependent sources remain spectral and do not advance a PCM
history. New source states share immutable target tables but reset all semantic
history. Whole-decoder transactions/checkpoints and reset include these states.
The syntax-only PS probe consumes LTP SCE/CCE metadata without synthesis.
ADTS PS negotiation and bounded rate probing now include AOT4.

Fixtures cover 1024 long/start/short/stop with sine/KBD, three coupling points,
directional source/target TNS, alternating CCE/target wire order, and explicit,
sync and implicit signalling at 24/48 kHz. Original scalar LTP/CCE bit writers
are replayed and checked against the previously qualified core packets.
The output reference composes independent scalar core PCM with the separately
qualified owned SBR/PS and source-QMF stages. This is a dispatch/composition
oracle, not an independent end-to-end PS oracle. Source point 3 is mixed after
PS and aligned with the delayed target frame; right PCM remains untouched.

All samples, original frame indices and delayed EOF are checked, along with
checkpoint replay/reset, root/owned MP4 equivalence, repeated/reversed ranges,
four mono/PCE ADTS stereo-discovery streams and syntax-probe rollback. Corrupt
SBR CRC packets exercise failure after core synthesis, then valid packet replay;
four short own corrupt videos reproduce exactly `SBR CRC mismatch`. Player
rewind/seek and low controlled-memory admission are additional acceptance gates.
Generation stays separate and uses neither FFmpeg nor network/private media.

Ordering was checked against the primary
[AAC dispatch reference](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/aac/aacdec.c)
(LTP/TNS/core history before SBR and final independent coupling). No foreign
implementation is copied or linked. These finite fixtures do not qualify 960
PS geometry, all CCE tags/absence/roster changes, source SBR/PS combinations,
all PNS/layout combinations or ER/LD/ELD/USAC.


The separate ADTS playback adapter also excluded AOT4 from its implicit PS/SBR
negotiation. Before the fix, all four authored LTP/PS ADTS programs opened as
mono 24 kHz and decoding refused with `SBR extended audio/PS synthesis is not yet
implemented`. The adapter now includes AOT4 in both detection paths. The
acceptance suite checks stereo 48 kHz LTP/PS and mono 48 kHz late LTP/SBR,
complete PCM, rewind, repeated seeks, and delayed EOF against owned decoding.

Validation: owned library 477 passed / one pre-existing ignored; ten PS
integration suites 51 passed; three core LTP suites 11 passed. After the
ADTS playback adapter fix, five selected suites passed all 19 tests, including
the two new player acceptance tests (541 distinct selected tests overall).
All runs were offline, without FFmpeg. The 39 generated artifacts were
byte-identical on regeneration. This is finite qualification, not complete
AAC profile/tool parity.

### AAC Main implicit PS in ADTS (2026-10-10)

Owned Main/PS decoding was admitted in MP4 but three ADTS negotiation guards
still excluded AOT1. Two original streams (PS immediately, or after four Main
core frames including active prediction) reproduced precisely
`SBR extended audio/PS synthesis is not yet implemented`. Their companion
MP4 videos accept the same core and extension packets. The refusal test was
replaced by PCM acceptance after adding Main to owned prefix negotiation,
syntax-only SBR rate probing and the ADTS player adapter.

The generator reuses original Main residual/prediction/window bit writers and
authored PS payloads. The late extension starts with its own SBR header;
a delta-only initial extension is not accepted as a reproducer. No private
media, external codec or FFmpeg/network is used. Acceptance checks complete
PCM against explicit MP4 companions, repeated/reversed ranges, player rewind,
seek and delayed EOF. Prefix limits and intervals before PS retain the mono
core clock; a selected prefix containing PS negotiates stereo double-rate.
This transport/dispatch qualification does not extend codec geometry/layout
coverage or establish an independent full PS oracle.

Validation: 477 owned library tests passed (one pre-existing ignored),
11 distinct selected integration tests passed, all offline without FFmpeg.
All five new fixture artifacts were byte-identical on regeneration.

### Owned 960-frame LTP/PS qualification (2026-10-10)

Six own MP4s combine the independently qualified 960-sample LTP transition
packets with authored 30-slot PS payloads: long/start/eight-short/stop,
sine/KBD, active prediction with varied lag/gain/used bands, explicit/sync/
implicit signalling at 24/48 kHz. Each underlying core packet is checked
byte-for-byte against the original transition writer; scalar core PCM is
preserved independently. The PCM oracle composes that core with the separately
qualified owned SBR/PS stage; it is not an independent full PS implementation.

All final stereo samples match within 1e-7, frame identities and delayed EOF
are exact, checkpoints/reset replay identically. Trailing bytes and corrupted
SBR CRC after active history must fail transactionally, followed by valid
packet replay. A short corrupt companion video reports exactly `SBR CRC
mismatch`. Public root/owned decode, repeated/reversed ranges and player
rewind/seek/EOF accept all six positive videos. This qualifies an existing
owned path; production decoder sources are unchanged.

This closes the mono 960 LTP/PS transition qualification gap at these clocks.
960 CCE, other SBR/PS payload geometries, wider source/tag histories, layout/
PNS combinations and ER/LD/ELD/USAC remain distinct gaps. Generation is offline
and separate from tests, with no private media, FFmpeg or network.

Validation: three new acceptance tests and three existing full PS native
regressions passed offline without FFmpeg; all ten new fixture artifacts
were byte-identical on regeneration. Production decoder code is unchanged.

### Owned 960 LTP/PS coupling qualification (2026-10-10)

The 960 LTP/PS fixture set now includes 18 additional MP4 configurations:
CCE points 0/1/3, explicit/sync/implicit signalling at 24/48 kHz. Independent
scalar LTP history and directional TNS run separately for target and source,
with sine/KBD and alternating element order. Dependent sources are mixed at
the corresponding spectral boundary; independent source PCM passes through
its own QMF history and is added only to the final left PS lane. The reference
composes scalar core/source PCM with separately qualified owned PS/QMF stages;
it is not an independent complete PS oracle. Production decoder is unchanged.

A source-prediction-disabled control changes left PCM by more than 1e-7 while
leaving every right sample unchanged. Three additional CRC videos and an
absent-target video after active histories reproduce precisely `SBR CRC
mismatch` and `AAC coupling target is absent`. Packet failures are followed
by valid decode/checkpoint replay, preserving source, core and delayed PS state.
All samples, ranges, reset, delayed EOF and player rewind/seek are acceptance
checks across all 24 configurations (six mono and 18 coupled).

Validation: expanded 960 suite passed four tests and existing 1024 LTP/PS suite
passed six tests, offline without FFmpeg. Parameterizing the original scalar
phase oracle preserved every original 1024 artifact byte-for-byte; the expanded
960 set is deterministic on regeneration. Wider CCE tags/absence/roster/source
extensions and additional PNS/window/layout combinations remain separate gaps,
as do ER/LD/ELD/USAC. These finite checks do not establish universal codec parity.

### LTP/PS independent-source roster histories (2026-10-10)

Seventy-two original MP4s cover 960/1024-frame active LTP targets and two
independent CCE tags (1 and 15) with distinct window/shape, lag/gain/usage and
coded histories. Fixtures vary 24/48 kHz output, source SBR, single/both-source
arrival or disappearance/return and static/dynamic PCE coupling rosters.
Source clocks pause when absent; returning tags resume their own LTP and QMF
state. Target LTP and PS continue independently across empty rosters.

The reference uses independent scalar float LTP/IMDCT for target and each
source and composes their PCM through separately qualified owned PS/QMF/SBR
stages. This is a composition oracle, not an independent full PS implementation.
Controls disable source LTP, reassign source FIL and discard QMF state during
absence; each must alter PCM observably. Right target PCM remains uncoupled,
and absent sources contribute no left PCM. Static and dynamic roster versions
must produce identical audio for the same coded schedule. Packet checkpoint,
trailing-byte rollback, reset, delayed frame identity/EOF, syntax probe,
public PCM/ranges and playback rewind/seek are additional acceptance gates.

Generation is offline and separate from tests, with no FFmpeg/network/private
media. Production decoder is unchanged. These fixtures qualify explicit PS
with the selected PCE/SBR/window/tool combinations; wider tags and source
extension geometry, dependent-source absence, arbitrary gains/layout/PNS,
ER/LD/ELD/USAC and broad conformance/performance remain distinct gaps.

Validation: all six expanded acceptance/control tests passed offline without
FFmpeg (72 configurations, 261.65 s debug run). All 86 fixture artifacts
were byte-identical on regeneration. No production decoder changes.

### Owned ER AAC-LC baseline syntax and playback (2026-10-10)

Forty original MP4s reproduced the former core-profile admission refusal.
They now use actual owned AOT17 decoding, preserving the signaled core type.
ER-LC reads the configured sequence of tagged elements without ordinary
raw-element IDs/END, and defers TNS data until after the gain-control field.
The original baseline admitted epConfig0 and zero resilience flags, with
extensionFlag false/true. Section-only resilience is now admitted below;
RVLC/HCR, nonzero epConfig and extensionFlag3 remain specific refusals.
The syntax order was checked against the primary
[AAC reference](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/aac/aacdec.c).
No foreign decoder code is copied, linked or executed.

Own bit writers cover 960/1024, long/start/short/stop, sine/KBD, directional
long TNS, mono/stereo/3.0/5.1, and common-window MS modes0/1/2. Independent
scalar MS/TNS/sparse IMDCT/window synthesis and channel mapping qualify all
PCM samples within 1e-7. Packet checkpoint/reset and truncated/trailing-packet
rollback precede valid replay. Root/owned MP4 PCM, repeated/reversed ranges,
player channels/rate, rewind and seeks are acceptance gates. Alignment-bit
variants preserve PCM; no unsupported zero-padding restriction is introduced.

Eight remaining short videos check nonzero epConfig or future
extensionFlag3 refusal, separately from playback acceptance. Two trailing-byte
videos fail specifically after seven valid frames. Two alignment variants
accept and match scalar PCM. Generation is offline and separate from tests;
no private media, codec parameters, FFmpeg or network is used.

The original baseline implemented ER-LC without error-resilience tools; the
remaining unimplemented tools are
HCR/RVLC, epConfig protection, PCE, ER SBR/PS, ER-LTP,
LD/ELD/USAC or broad profile conformance. Additional indexed layouts/tools
need qualification. The full codec-gap objective remains open.

Validation: 477 owned library tests passed (one pre-existing ignored);
five final ER-LC acceptance/refusal tests and ten Main/SSR/LTP regressions
passed offline without FFmpeg (492 distinct selected tests). All 69 ER-LC
fixture artifacts were byte-identical on regeneration.

### Owned ER AAC-LC section-data resilience (2026-10-10)

Section resilience now retains five-bit codebook indices, uses implicit single
bands for book11 and virtual books16..31, and keeps ordinary escaped lengths
for the other books. Virtual indices select physical escape book11 with the
individual magnitude limits from ISO/IEC 14496-3 table4.95. Scalefactors,
reconstruction and pulse eligibility recognize the virtual spectral bands.
Malformed out-of-limit coefficients reject the packet transactionally; this
strict decoder does not implement concealment of corrupted spectral lines.

The offline generator `scripts/generate_aac_er_sections_fixtures.py` authors
32 acceptance videos and 32 exact LAV-overflow videos. Every virtual book is
exercised at its largest legal signed magnitude, mixed with explicit book1
and implicit book11, for 960/1024 sine/KBD and long/start/short/stop frames.
The independent direct IMDCT/window oracle checks PCM; ordinary tests read
committed assets and never generate them or require FFmpeg/network access.
The former section-only ASC refusal fixtures are replaced by acceptance;
RVLC, HCR, epConfig protection, PCE and ER SBR/PS remain unsupported.

Validation: 477 library tests passed (one pre-existing ignored), five baseline
ER-LC regressions, two section-syntax tests and four virtual-codebook native/MP4/
player/rollback acceptance tests passed offline. All 67 section fixture artifacts
regenerated byte-identically. These finite cases do not prove broad ER profile
conformance or close the remaining codec gaps.

### Owned ER AAC-LC RVLC scalefactors (2026-10-10)

ER-LC now admits scalefactor resilience with or without section resilience.
The owned reader separates the class1 RVLC header from class2 codewords after
pulse/TNS/gain presence flags and before deferred ER TNS payload. It uses the
15 base words, eight forbidden words and all 54 escape words from ISO/IEC
14496-3 tables4.113-115. Forward DPCM keeps spectral, PNS and intensity units
separate; first-noise PCM, terminal intensity, reverse gain and reverse noise
seeds are checked. Base codewords are also decoded backwards and compared.
Both bit regions have exact bounds; failures preserve bit cursors and native
history. Corrupted data is refused, without speculative concealment.

`scripts/generate_aac_rvlc_fixtures.py` is an offline own bit writer and scalar
PCM oracle. Sixteen acceptance videos cover mono, stereo, intensity and PNS,
960/1024 long/start/short/stop sine/KBD, section flags0/1, signed scalefactor
escapes and deferred directional TNS. Thirty-six corrupted videos isolate
forbidden words, missing/unused escapes, wrong region lengths, first-noise
length underflow and inconsistent reverse gain/noise/intensity seeds. The
length-mismatch reproducer has no escapes, so it cannot fail on a shifted
escape region instead. The four former RVLC-only/section+RVLC ASC refusal
fixtures are replaced by these acceptance videos; HCR refusals remain.

Unit qualification traverses every delta -60..60 (including all 54 escape
words), eight independent short groups, forbidden words, unaligned adjacent
bits and every header/payload truncation. Video PCM is compared with scalar
PNS/intensity/TNS/direct IMDCT/window synthesis. Ordinary tests read fixtures;
they do not run their generator or require FFmpeg/network access.

HCR, protected epConfig, ER PCE/SBR/PS, ER-LTP, LD/ELD/USAC and broader codec
coverage remain incomplete. This is finite RVLC qualification, not full AAC
profile conformance or completion of the codec-gap objective.

Validation: 480 library tests passed (one pre-existing ignored), plus 15 ER
baseline/section/RVLC integration tests. All 55 RVLC fixture artifacts regenerated
byte-identically. Four LTP test targets compile with the new configuration flag;
all checks were offline and used no FFmpeg.

### Owned ER AAC-LC Huffman codeword reordering (2026-10-10)

ER-LC now admits HCR independently and with section resilience and RVLC.
The owned parser reads HCR class1 lengths before RVLC class2 words/deferred
TNS, uses element-specific SCE/CPE length maxima and adapts reserved values.
A transactional Huffman-prefix API distinguishes unfinished words without
examining error strings. The HCR decoder sorts four-line units by codebook
priority, line and window, preserves paired-word order, instantiates variable
segments and merges their tail. It decodes PCWs forwards and subsequent sets
in alternating directions with modulo-shifted trials, retaining unfinished
words across segments. Group/band/window coefficients and virtual LAV checks
are shared with the ordinary spectrum path.

The independent own generator `scripts/generate_aac_hcr_fixtures.py` writes
16 acceptance videos and 52 malformed videos, without FFmpeg/network or
private media. Positive cases cover 960/1024, mono/common-window stereo MS0/2,
all physical books1..11 and virtual books16..31, zero bands, an initial
all-zero HCR frame, sine/KBD and
long/start/short/stop. Short windows use groups8 and1/3/4. Fixture coverage
contains up to eight codeword sets, 1192 split placements and 3520 shifted
placements; tail merging is also witnessed. Deferred TNS and all combinations
of HCR with section/RVLC flags are exercised.

Raw region/quantized artifacts gate exact reconstruction, separately from
scalar inverse-quantization/MS/TNS/direct IMDCT/window PCM. Malformed cases
isolate zero/too-small longest words, missing segments, unused data, truncated
regions, incomplete nonpriority words and virtual-LAV overflow. Failure is
transactional and checked again after valid packet history. The eight former
HCR ASC-refusal videos are replaced by acceptance. The remaining baseline
refusals concern epConfig protection and future extensionFlag3.

This is finite ER-LC qualification. Protected epConfig, ER PCE/SBR/PS, ER-LTP,
LD/ELD/USAC, additional tool/layout combinations and broader codec profiles
remain incomplete; the full codec-gap objective stays open.

Validation: 481 library tests passed (one pre-existing ignored), 20 ER
baseline/section/RVLC/HCR integration tests and three original Huffman regressions
passed offline. HCR acceptance was repeated with an initial all-zero region;
all 73 final HCR artifacts regenerated byte-identically. Four LTP test targets
compile with the new flag. No FFmpeg was used.

## ER AAC-LTP baseline (AOT19)

Own AOT19/epConfig0 parsing now reaches the existing owned LTP analysis,
float history, synthesis and checkpoint/reset paths. ER elements use their
configured order without ordinary element IDs/END. With a common window,
ICS only signals predictor presence: left LTP data follows the MS mask and
right LTP data follows the left channel payload. Independent windows retain
per-channel prediction data in ICS. ER TNS retains its deferred syntax.

Twenty authored videos (including extensionFlag zero/one with zero resilience flags) cover 960/1024 mono, independent stereo and common-window
stereo with MS modes 0/1/2. Twelve frames each alternate long/start/eight-short/
stop and sine/KBD windows, switch predictors independently, vary lag, all eight
prediction coefficients and band usage. A scalar direct cosine/window/history
oracle gates every PCM sample. Separate malformed videos exercise out-of-range
960 lag and truncated 1024 spectral payload with transactional channel/pair
cursors and decoder history. Public MP4 export, repeated intervals, rewind and
seek use the same acceptance corpus.

This qualifies baseline mono/stereo on authored streams, not complete ER-LTP.
Nonzero resilience flags, protected epConfig, PCE and SBR/PS remain explicit
refusals. LD/ELD/USAC and broader codec profiles/tools remain incomplete.
Generator: `scripts/generate_aac_er_ltp_fixtures.py`; normal tests do not execute
it or require FFmpeg/network. No private media or codec parameters were used.

Validation: 481 owned library tests and 31 selected AAC integration tests passed
offline (512 total; one pre-existing library test ignored). All 33 ER-LTP
artifacts regenerate byte-identically. The initial AOT19 profile refusal was
reproduced before the fix and replaced with five acceptance/rollback tests.
No FFmpeg was executed.

## ER AAC-LTP resilience and independent-window HCR

AOT19/epConfig0 now admits all seven nonzero section/RVLC/HCR flag combinations.
This supersedes the baseline resilience refusal above. Existing owned resilience
readers feed the owned LTP/TNS/synthesis path. Independent-window CPE channel
parsing now carries element context, so both channels retain the CPE HCR
12288-bit allowance instead of the SCE 6144-bit allowance. This changes no
ordinary AOT4 spectral syntax.

Seventy authored twelve-frame videos cover 960/1024 mono, independently coded
stereo and common-window MS0/1/2, all seven combinations, independent predictor
switching, all eight LTP coefficients, lag and band-use variation, sine/KBD and
long/start/eight-short/stop transitions. They include initial zero HCR regions,
virtual book17, RVLC signed/escape deltas and directional deferred TNS. An own
scalar inverse-quantization/MS/FIR prediction/AR TNS/direct cosine/window/float
history oracle qualifies every PCM sample at absolute tolerance 1e-7.

Four additional three-frame independent-window CPE videos alternate the large
channel: one HCR region is 10192 bits, the other is small, and the entire CPE
stays within its 12288-bit input buffer. Reusing the former SCE context
reproduces `AAC HCR incomplete nonpriority codeword`; the fixed direct pair
parsing, native/public/owned PCM, rewind and seek pass. Seventy separate corrupt
videos isolate HCR zero-longest, RVLC reverse-gain and virtual-LAV failures.
Channel/pair cursors and decoder histories roll back after failures; every
byte prefix of the seventy regular cases is also checked; the four large CPE
cases check selected truncations. Public MP4 export, repeated ranges and seeks
are acceptance tests, replacing the ASC-refusal reproduction.

This is finite ER-LTP tool-combination qualification, not complete AAC or codec
conformance. Protected epConfig, ER PCE/SBR/PS, wider tools/layout combinations,
LD/ELD/USAC and broader video codec profiles/tools remain incomplete. Generator:
`scripts/generate_aac_er_ltp_resilient_fixtures.py`; fixtures are authored offline
and ordinary tests require no FFmpeg or network. No private media or codec
parameter sets were used.

Validation: 481 owned library tests and 37 selected AAC integration tests passed
offline (518 total; one pre-existing library test ignored). The six new tests
passed on the final 70-combination/four-valid-CPE corpus. All 147 final
artifacts regenerate byte-identically. No FFmpeg was executed.

## AAC-LD filterbank foundation (AOT23 admission still open)

Owned IMDCT/MDCT now accepts 480/512 coefficients. The new `LdSynthesis`
implements sine and the LD low-overlap window from ISO14496-3 4.6.17.2.3;
LD shape1 is not the ordinary KBD window. Leading and trailing halves use
previous/current shapes. Synthesis has packet-boundary checkpoint/reset,
transactional input/history checks, retained-allocation inspection and no
per-frame allocation. Windowed forward MDCT is available for subsequent LD-LTP
integration and does not advance PCM history.

Two authored AOT23 ep0/no-prediction videos carry 48 spectral frames with all
four window-shape transitions and silence/overlap tails. Direct scalar cosine,
window and overlap PCM verifies the standalone filterbank; eight independent
forward-MDCT cases cover every shape pair. Window support/symmetry/power
complementarity, checkpoint replay and invalid-input rollback are tested.
The videos currently retain a specifically labelled public AOT23 profile-refusal
test. This is a gap reproduction, not public playback acceptance.

Required integration remains: LD band tables and frame geometry, LD LTP
lag-update/history semantics, native decoder/checkpoint/budget dispatch,
resilience/TNS/stereo combinations and MP4/player timing/seek acceptance. The
AOT23 gate is intentionally unchanged until those paths are implemented. This
milestone does not establish LD/ELD/USAC or complete codec conformance.

Validation: 483 owned library tests passed, including expanded forward/inverse
transform references at 480/512 and two LD window/checkpoint unit tests (one
pre-existing ignored). Two standalone LD acceptance tests and one specifically
labelled public-profile refusal test passed offline. All six authored artifacts
regenerate byte-identically. No FFmpeg was executed.


AAC-LD AOT23 is now connected to the owned raw decoder and MP4 PCM export with
480/512 frames, LD windows/LTP and ER TNS ordering. The six authored mono videos
now have intended PCM acceptance instead of profile-refusal expectations.
Pair/resilience combinations remain to be qualified; PCE, protected epConfig,
ELD and USAC are not covered by this milestone. See CODEC_PROGRESS.md.


AAC-LD qualification now includes 64 authored stereo/resilience combinations:
480/512, independent/common windows, MS 0/1/2, section/RVLC/HCR flags, independent
lag histories, window-shape switches and bidirectional TNS. PCM, malformed input
rollback and player range/rewind/seek are checked against synthetic references.
Additional rates/layouts/tools and protected epConfig/PCE/ELD/USAC remain open.
