# AV1 reserved-OBU playback qualification

The owned decoder already ignores types0/9..14 and padding15. This check
confirms the required forward-compatible decoder behavior with pictures,
not merely successful OBU header parsing. No production decoder fix was needed.
The AV1 specification states that reserved units are ignored by the decoder:
https://aomediacodec.github.io/av1-spec/#obu-header-semantics
This is decoder robustness qualification, not a claim that arbitrary future
payload bytes establish encoder bitstream conformance.

Sixteen authored programmes cross eight types with extension headers present
or absent. Blocks are inserted before every known OBU and after each of three
packets, including between separate frame headers and tile groups. The control
has a hidden16x16 reference and two shown32x32 frames with independent saved
pixels from the existing authored separate-frame corpus. Known CLL metadata
sets MaxCLL1000/MaxFALL200. The inserted payloads resemble other OBU headers so
accidental parsing as ordinary sequence/frame payload cannot satisfy the check.

Native whole-stream and packet-by-packet decoding preserves frame publication,
dimensions, all planes, segment maps and HDR metadata. Reset/replay gives the
same results. Display pixels also match the saved independent authored YUV.
The WebM writer preserves every inserted byte; the test demuxes all three
samples and compares their lengths and concatenated payload with the raw stream.
This prevents a fixture wrapper from silently removing the condition being
qualified. Software playback, rewind, forward/backward seeks and source frame
intervals match the unchanged control exactly.

Both tests pass, zero failures/ignored tests. All36 corpus files regenerate
byte-identically with subprocess and socket creation blocked. The generator
reuses the authored parent's saved codec-private record via bounded EBML
parsing; it does not build or run a codec helper. Ordinary tests never generate
fixtures or require FFmpeg/network.

```sh
python3 scripts/generate_av1_reserved_obus.py
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline \
  --no-default-features --features player --test av1_reserved_obus
```

This rules out the suspected unknown-OBU admission gap. It does not establish
all AV1 tool/profile conformance or close the remaining LD-MPS HRTF/EBQ and
inherited-prefix-loss restrictions, or the overall codec-gap objective.
