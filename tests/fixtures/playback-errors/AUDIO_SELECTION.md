# Owned audio-selection fixture generation

Run `python3 scripts/generate_audio_selection_sample.py` separately from tests.
It uses committed synthetic packets and Python-owned MP4/ADTS/Matroska framing;
it does not invoke an encoder, decoder, FFmpeg, or another external media tool.

- `alac-two-tracks.m4a` duplicates the single synthetic `alac/stereo-24.m4a`
  track, retaining its sample description, packets and sample durations.
- `aac-two-tracks.m4a` combines synthetic 44.1 kHz mono ADTS with nine synthetic
  48 kHz stereo Matroska packets. The second track has 1024 priming samples and
  a 7200-sample presentation; explicit selection changes rate/channel geometry.
- `aac-rounded-two-tracks.m4a` retains 48 stereo packets with deliberately
  shortened durations: packet 1 is 1016 samples and packet 47 is 912. Its edit
  removes 1008 priming samples and presents exactly 48008 samples. Acceptance
  compares PCM against independently decoded packet slices, including a seek.
- `aac-no-edit.m4a` retains that stereo sample table without an edit. Its
  dependent gap/repeat fixtures are regenerated separately with
  `generate_aac_edit_timeline_sample.py`, optionally `--media-start-ticks 4800`.

A common movie clock of 7056000 represents both sample rates and millisecond
edits exactly. All six outputs regenerate deterministically. Tests consume
committed files without running these generators or requiring media tools.
`native_audio_plan` checks selected-track CLI decoding and the rounded-window
regression; `native_aac_media` checks repeated nonzero ranges against a continuous
PCM decode and verifies checkpoint reuse. The legacy-feature `native_audio_api`
checks that selected ALAC requests use the owned backend.
