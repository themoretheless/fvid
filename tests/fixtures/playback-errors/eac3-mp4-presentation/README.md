# Expanded E-AC-3 in MP4

Regenerate separately with `python3 scripts/generate_eac3_mp4_presentation.py`.
Twenty owned 48 kHz 8/10-channel cases and one 44.1 kHz 7.1 witness reuse synthetic conventional base/dependent
spectra from eac3-presentation, including changing/conflicting gains and final
clamp headroom. No private media, external codec, subprocess or network.
Each MP4 includes neutral VP9 video and three six-block audio samples. Each
audio sample bundles two three-block programmes, for 4608 decoded samples.
The ec-3 sample entry uses placeholder channelcount=2; dec3 declares base 5.1
plus one or two dependents with chan_loc=2 (Lrs/Rrs) or 66
(Lrs/Rrs and height pair). Front replacement retains only additional locations
in dec3. Bit ordering is documented in
ETSI TS 102 366 F.6.2.13/Table F.6.1 and Chromium's primary implementation:
https://chromium.googlesource.com/chromium/src/media/+/refs/heads/main/formats/mp4/ac3_constants.h

`mp4_six_block_samples_infer_expanded_channels_and_preserve_pcm` in
`tests/eac3_raw_presentation.rs` failed before the reader change with 2 versus
expected 8 channels (`/tmp/fvid-eac3-mp4-before.log`). It now accepts channels,
mask, exact PCM after accounting for the raw decoder's 256-sample priming,
full output size and rewind. PCM is compared with the already qualified native
presentation decoder; this is container acceptance, not a new independent
codec oracle or physical-device test. Twenty-three generated artifacts reproduce
byte-for-byte with subprocess/socket creation denied.

Rates beyond the qualified 48/44.1 kHz cases, expanded AHT/SPX/coupling and 16-channel MP4 remain unqualified.

`mp4_expanded_worker_preserves_partial_sample_seeks_and_rewind` accepts all
21 cases through backend capture, rewind and 20/40/80 ms partial-sample seeks.
It compares byte-exact PCM, each PTS, layout mask and final worker clock.
The sequence test also checks three 1536-sample outputs at 0/32/64 ms and a
96 ms total duration at 48 kHz (sample-rational clocks at 44.1 kHz). This remains synthetic conventional-tools acceptance.

The 44.1 kHz owned video exposed an actual seek-history error: sequential PCM
passed while worker generation2 packet0 had equal byte counts but different PCM
(`/tmp/fvid-eac3-mp4-44100-before.log`). Native MP4 AC-3/E-AC-3 seeks now preroll
from the beginning to restore decoder state before the requested PCM is exposed.
The corrected 21-case run passes (`/tmp/fvid-eac3-mp4-44100-after.log`). This is
currently linear-time preroll; bounded checkpoints/performance are not qualified.
The 44.1 kHz sample entry, mdhd, dec3 fscod, bitrate and rounded movie/track
headers are generated from the actual rate; audio duration is 4608/44100 seconds.

Track-description regression: `two-expanded-audio-tracks.mp4` adds a second
valid track sharing the same owned 7.1 samples, with a distinct track ID. Both
sample entries retain placeholder channelcount=2. Before the metadata fix,
`audio_tracks()` reported 2 while decoded output was 8
(`/tmp/fvid-eac3-track-metadata-before.log`). The reader now caches native Dolby
geometry for all valid tracks, not only the selected one. Tests select either
track and compare both descriptions, complete PCM, PTS and rewind. Unreadable
unselected tracks retain original metadata and cannot prevent opening a valid
selected track. All 23 artifacts / 1,859,944 bytes reproduce with process/socket
denied; the prior 21 media artifacts remain byte-identical (manifest extended).
