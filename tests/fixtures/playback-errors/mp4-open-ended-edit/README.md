# Exact open-ended MP4 media edits and finite fixed-audio edits

Accepted as FVID-0403/FVID-0404 (batch 397). No private media or codec parameters,
FFmpeg or network. Ordinary tests read checked-in files only.

Generate separately, in order:

1. `scripts/generate_mp4_open_ended_edit.py`: neutral 16x16 VP9, finite/open V0/V1
   edits, coarse/fractional movie clocks, trimmed starts and positive CTS.
2. `scripts/generate_mp4_open_ended_edit_audio.py`: owned nonzero AAC pulse PCM
   oracle, integer PCM and raw ALAC controls with VP9; hybrid fragments.
3. `scripts/generate_mp4_fixed_audio_fractional.py`: PCM/ALAC samples at 8kHz
   in a 44100Hz media clock, with a boundary between sample instants.

`manifest.json`, `manifest-audio.json` and `manifest-fractional.json` retain hashes.
Movie/track headers state consistent durations; open track duration remains
unstated. The elst duration-zero marker stays zero after parsing. Resolve its
presentation window with Track::resolved_edit_clock, using the returned clock.
A 25Hz movie clock must not turn one remaining 20ms video frame into 40ms.
Positive-CTS samples end at PTS 5 rather than the decoded-duration value 3.

Finite PCM/ALAC trims require their own presentation scheduler even with one
constant sample description. Native playback preserves nonzero scalar PCM,
exact duration, packet timestamps, seek suffixes, rewind and EOF. Fractional
boundaries keep the first retained sample at 102324ns, rather than losing the
sample phase. Existing finite AAC seek-return conventions remain covered;
output packet presentation timestamps and PCM are checked independently.

Public decode plans, PCM/WAVE export, atomic refusal without publication,
AAC-to-Matroska packet remux, owned video decode and CPU video schedules are
also covered. The NVIDIA schedule test needs no GPU; NVIDIA execution itself
was not qualified on this host. Existing flat/empty/multiple/rate-zero refusal
controls and the native leading-empty-edit normalization remain unchanged.

Reference: https://www.w3.org/TR/mse-byte-stream-format-isobmff/#initialization-segments
(single rate-one duration-zero media edit spans subsequent media).
