# AAC-LC explicit eight-channel PCE fixtures

Generated with FFmpeg 9.0.2 as an independent encoder; production FVid does not
execute or link FFmpeg for PCE parsing. The source has distinct tones in every
channel. The encoder writes channel_configuration=0 with a Program Config
Element: front SCE 0, front CPE 0, front CPE 1, back CPE 2, LFE 0.

```sh
ffmpeg -v error -f lavfi -i 'aevalsrc=0.06*sin(2*PI*300*t)|0.06*sin(2*PI*400*t)|0.06*sin(2*PI*500*t)|0.06*sin(2*PI*60*t)|0.06*sin(2*PI*700*t)|0.06*sin(2*PI*800*t)|0.06*sin(2*PI*900*t)|0.06*sin(2*PI*1000*t):s=48000:d=0.12:c=7.1(wide)' -c:a aac -b:a 512k -aac_pns 0 -f adts aac-pce-wide8.aac
ffmpeg -v error -f lavfi -i 'aevalsrc=0.06*sin(2*PI*300*t)|0.06*sin(2*PI*400*t)|0.06*sin(2*PI*500*t)|0.06*sin(2*PI*60*t)|0.06*sin(2*PI*700*t)|0.06*sin(2*PI*800*t)|0.06*sin(2*PI*900*t)|0.06*sin(2*PI*1000*t):s=48000:d=0.12:c=7.1(wide)' -c:a aac -b:a 512k -aac_pns 0 aac-pce-wide8.m4a
```

The ADTS fixture tests raw_data_block PCE syntax. The MP4 fixture tests the PCE
embedded in AudioSpecificConfig, including alignment and trailing SBR signaling.
The metadata ASC entrypoint reports the explicit channel count; packet decoding
uses parse_with_program to retain tags and speaker positions.

Packet decoding reference (ignore edit-list priming, retain container packet durations):

```sh
ffmpeg -v error -ignore_editlist 1 -i aac-pce-wide8.m4a -f f32le aac-pce-wide8-mp4-reference.f32le
```

`NativeAacDecoder` now resolves the configured PCE tags and reconstructs all
8 channels. Each PCM speaker is compared independently with RMS < 1e-7 and
peak error < 1e-6. The last packet's shorter duration accounts for reference
suffix trimming; initial synthesis samples are compared without a guessed
offset. Reset and malformed-PCE state preservation are covered.
ADTS channel_configuration=0 ingestion is verified below.

Edited MP4 export reference:

```sh
ffmpeg -v error -i aac-pce-wide8.m4a -f f32le aac-pce-wide8-export-reference.f32le
```

Owned CLI and public media API exports retain all eight channels with WAVE mask
0xff (FL FR FC LFE BL BR FLC FRC), apply priming/edit scheduling, and compare
complete PCM samples with this independent edited reference (peak < 1e-6).

ADTS PCM reference: `aac-pce-wide8-adts-reference.f32le`, generated with
`ffmpeg -v error -i aac-pce-wide8.aac -f f32le aac-pce-wide8-adts-reference.f32le`.
Streaming and indexed owned decoding match every channel within 1e-6.
The reader requires PCE before audio in the first raw packet; preceding
data_stream_elements are accepted. Tests insert aligned DSE payloads of
0, 7, 255 and 510 bytes and verify byte-identical PCM and every truncation.

Owned MP4 indexed/streaming remux and MP4/Matroska concatenation retain the
complete ASC and every raw packet. Decoded output equals continuous owned
packet synthesis byte for byte, including state at segment boundaries.

`aac-config7-wide8.aac` derives from the independently encoded PCE fixture:
the 21-byte aligned PCE at the start of the first raw block is removed;
every ADTS header is rewritten with channel_configuration=7 and the first
frame length adjusted. All encoded audio element payloads are unchanged.
This is the standard 7.1 wide layout, not the common nonstandard 7.1 mapping.

```sh
ffmpeg -v error -strict strict -i aac-config7-wide8.aac -f f32le aac-config7-wide8-reference.f32le
```

Owned decode matches every reference sample within 1e-6 and MP4 remux
produces byte-identical owned PCM. WAVE speaker mask is 0xff.
Channel layout semantics verified against the primary reference tables:
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/aac/aacdec_tab.c
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/aac/aacdec.c

Preceding FIL tests cover lengths 0, 1, 14, 15 and 269 bytes, fill/fill-data
extensions, exact PCM, every first-frame truncation, and SBR rejection.

Indexed configurations 11/12 fixtures: `aac-config11-61.m4a` and
`aac-config12-71.m4a`, independently encoded using the distinct-tone source
above with seven/eight channels and `c=6.1(back)` / `c=7.1`, AAC 448k/512k,
PNS disabled. The encoder writes configuration 11/12 respectively.
References generated with:

```sh
ffmpeg -v error -ignore_editlist 1 -i aac-config11-61.m4a -f f32le aac-config11-61-reference.f32le
ffmpeg -v error -ignore_editlist 1 -i aac-config12-71.m4a -f f32le aac-config12-71-reference.f32le
```

Native raw packet synthesis matches each sample within 1e-6. Reference final
packet duration trims its suffix; no guessed priming offset is used.
WAV exports retain masks 0x13f / 0x63f, while configuration 7 remains 0xff.
