# Fragmented MP4 fixtures

Both files are synthetic and carry no private material. They are the two shapes of
ISO/IEC 14496-12 §8.8 sample-by-fragment indexing this reader was written against:
an init segment (`ftyp` plus a `moov` whose sample tables are empty and whose `mvex`
promises fragments) followed by `moof`/`mdat` pairs. FVid's runtime reads them with
no FFmpeg; `ffmpeg` here only writes them, and `ffprobe` only checks what the rows
say against what the reader indexes.

Both commands were run twice into two paths and the results compared byte for byte
(`cmp`), which is what lets them stand as reproducible documentation:

```sh
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1.2" \
  -c:a aac -b:a 128k \
  -movflags frag_keyframe+empty_moov+default_base_moof -f mp4 audio.mp4

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc=size=320x240:rate=25:duration=1" \
  -c:v libx264 -profile:v high -pix_fmt yuv420p -g 5 -sc_threshold 0 \
  -movflags frag_keyframe+empty_moov+default_base_moof -f mp4 video.mp4
```

`-sc_threshold 0` is what keeps the video take at five fragments of five rows each:
without it the scene-cut detector adds keyframes, and a keyframe is a fragment edge.
`-profile:v high -pix_fmt yuv420p` is what keeps it playable here: left to itself
x264 codes `testsrc` as High 4:4:4 (`avcC` begins `01 f4`), and FVid's own H.264
configuration reader refuses a profile it has no tools for (`codec/config.rs`,
"unexpected AVC profile extension"), so the take would index fine and never reach a
picture.

| File | Bytes | Tracks | Fragments | Rows | `ffprobe` says |
| --- | --- | --- | --- | --- | --- |
| `audio.mp4` | 20 085 | 1 mono AAC at 48 kHz | 1 | 58 | 1.221333 s, every packet `K__` |
| `video.mp4` | 19 495 | 1 H.264 High 320x240 at timescale 12 800 | 5 | 5 each, 25 total | 1.000000 s, `K__` at rows 0, 5, 10, 15, 20 |

What each one exercises, as read from its own bytes:

- `audio.mp4`: a `trun` with flags `0x301` — a data offset counted from the `moof`
  (`default_base_moof`, so the `tfhd` sets bit `0x20000` and states no absolute
  base), then one duration and one size per row. `tfdt` version 1 says 0.
  `tfhd` flags `0x20038` default the duration (1 024), the size (451) and the sample
  flags (`0x02000000`, whose "not restartable" bit is clear), so every row is a
  sync sample and none states flags of its own. The last row is the 5-byte, 256-tick
  fill frame the encoder ends on, and the 58 sizes add up to the 18 718 bytes of the
  one `mdat` — 1 300 to 20 018 — that they are measured from.
- `video.mp4`: five fragments, each with its own `mdat` (17 814 bytes of media
  between them), and a `trun` with flags `0xa05` — data offset (152 from each
  fragment's own `moof`, which is where its media begins), first-sample flags
  (`0x02000000`, the keyframe), then a size and a composition offset per row, so
  durations come from the `tfhd` default of 512 ticks. The offsets are what put the
  display order apart from the decode order (`pts - dts` of the first fragment is
  1 024, 2 560, 1 024, 0, 512), and the default sample flags (`0x01010000`, "not
  restartable" set) are what marks rows 1 to 4 of every fragment as non-sync. Each
  fragment states its `tfdt` base 2 560 ticks on from the one before, so the fifth
  begins at 10 240 and the take ends at 12 800.

`container::mp4::tests` asserts the whole 25-row video list and the audio list's
edges against these numbers, reads every packet of both takes back to check the
offsets, hides one fragment's `tfdt` (at byte 4 884) to check that the track
continues where the last sample ended, walks the audio take through the sound
reader and the decoder the dispatch answers, walks the video take through the
picture reader to check the five frames of the first fragment arrive in display
order, and rewrites the `tfhd` flags, the row count, a row size and the `moof`
itself to check that each refusal says so.
