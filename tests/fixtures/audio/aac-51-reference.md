# AAC 5.1 channel-order reference

FFmpeg 9.0.2, fixture/oracle generation only:

```sh
ffmpeg -v error -f lavfi -i 'aevalsrc=0.1*sin(2*PI*300*t)|0.1*sin(2*PI*500*t)|0.1*sin(2*PI*700*t)|0.1*sin(2*PI*60*t)|0.1*sin(2*PI*900*t)|0.1*sin(2*PI*1100*t):s=48000:d=0.2:c=5.1' -c:a aac -b:a 384k -aac_pns 0 -f adts aac-51-active.aac
ffmpeg -v error -i aac-51-active.aac -f f32le aac-51-reference.f32le
```

Every channel has a distinct nonzero tone. PCM order is FL FR FC LFE BL BR.
The test compares every channel independently, including decoder delay.
Observed RMS below 7.5e-9 and peak below 4.5e-8 on each channel; regression
gates are RMS 1e-7, peak 1e-6. Tests never execute FFmpeg.
