# Audio Implementation Status

Ownership status: AAC currently uses Symphonia behind `codec::aac_decoder`.
Its working MP4/Matroska playback is not evidence of an FVid-owned AAC codec.
Replacing that implementation remains required by the own-codecs objective.
Malformed AAC packets now return errors and require decoder reset; they are
not reported as successful empty output. Out-of-range unsigned timestamps
are rejected before touching codec state instead of wrapping to negative PTS.

## What works

| Source | Codec | Path | Verified |
| --- | --- | --- | --- |
| MP4 | AAC-LC (`mp4a`) | `playback_mp4_audio` → `codec::aac_decoder` → cpal | yes, headless |
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
boundaries, complete/aligned band coverage and ASC-to-ICS validation. 960/120
band tables are still pending and are rejected explicitly, even though the
transform/synthesis components already support those sizes.

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
