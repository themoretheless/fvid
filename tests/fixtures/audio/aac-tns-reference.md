# Real encoder TNS regression

FFmpeg 9.0.2 commands (oracle/fixture generation only):

```sh
ffmpeg -v error -f lavfi -i 'anoisesrc=color=white:sample_rate=48000:duration=1:seed=1234' -af 'aeval=val(0)*exp(-mod(n\,2048)/256),aformat=channel_layouts=mono' -c:a aac -b:a 128k -aac_tns 1 -aac_pns 0 -f adts aac-tns.aac
ffmpeg -v error -i aac-tns.aac -f f32le aac-tns-reference.f32le
```

The test requires exactly two active TNS filters in the saved file, decodes all
packets with FVid, and compares the full mono PCM including decoder delay.
Observed RMS error: 1.5812e-8; peak: 1.7882e-7. Gates: RMS < 1e-7, peak < 1e-6.
PNS is disabled to remove random-sequence differences. FFmpeg is never called
by the test or the native decoder.
