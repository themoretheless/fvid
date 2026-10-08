# AV1 chroma deblocking fixture encoder

This encoder is a generation-only oracle tool. FVid and ordinary tests never
build, load or execute it. Stock encoders selected zero loopfilter levels on
these source patterns, so an enabled option was insufficient qualification.
The patch chooses legal nonzero directional/Y/UV levels and sharpness; unchanged
libaom and dav1d decoders independently verify the resulting stream pixels.

Use a separate clean libaom checkout pinned to
`44d0a57786f432d933ff64b653347c66f4d0fa1d` (v3.15.1):

```sh
git clone https://aomedia.googlesource.com/aom /tmp/fvid-aom-fixture-source
git -C /tmp/fvid-aom-fixture-source checkout --detach 44d0a57786f432d933ff64b653347c66f4d0fa1d
git -C /tmp/fvid-aom-fixture-source apply /path/to/fvid/scripts/fixture-tools/av1-chroma-forced-deblock.patch
cmake -S /tmp/fvid-aom-fixture-source -B /tmp/fvid-aom-fixture-build -DENABLE_TESTS=OFF -DENABLE_DOCS=OFF -DENABLE_TOOLS=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/fvid-aom-fixture-build --target aomenc -j 8
python3 /path/to/fvid/scripts/generate_av1_chroma_filter_samples.py --encoder /opt/homebrew/bin/aomenc --forced-deblock-encoder /tmp/fvid-aom-fixture-build/aomenc --oracle /opt/homebrew/bin/aomdec --second-oracle /opt/homebrew/bin/dav1d
```

Replace `/path/to/fvid` and the stock CLI paths for the generation host. Generation
requires the source checkout and reference tools; test execution only reads the
checked-in synthetic OBU/WebM/YUV assets and uses native FVid APIs.
