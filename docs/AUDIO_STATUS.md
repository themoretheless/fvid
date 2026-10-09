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
