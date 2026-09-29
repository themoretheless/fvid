# Rotated VP9 Matroska fixture

`vp9-rot90.mkv` copies the existing `../vp9/motion.webm` compressed packets and
adds a 90-degree clockwise rectangular display rotation. Generated with the
reference-only FFmpeg 9.0.2 command:

```sh
ffmpeg -v error -n -display_rotation -90 -i tests/fixtures/vp9/motion.webm \
  -map 0:v:0 -c copy tests/fixtures/display/vp9-rot90.mkv
```

The ordinary `native_matroska_metadata` regression reads/decodes/exports this
fixture entirely through FVid. It verifies display dimensions 96x128, planar
format detection and rotated Y4M sample bytes against the original fixture.
