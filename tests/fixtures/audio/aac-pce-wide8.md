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
These parser tests do not yet establish PCM decoding of PCE layouts. The legacy
standard-layout ASC entrypoint still rejects a PCE rather than dropping its tags.
