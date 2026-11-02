# AVI G.711 bounded packet buffers

The PCM branch already bounded packet windows independently of physical AVI
chunks, while G.711 copied the complete record and decoded it in one call.
This is a measured resource opportunity; the original valid waveform, clocks
and packet bytes are not claimed defective. Three baseline executions observe
identical capacity metrics for all20 authored inputs.

`before.log` passes the three waveform/seek/worker/resource controls and fails
only the new packet-window admission check. The two `before-repeat` logs repeat
that memory check against the same original executable. `before-source.json`
records the baseline commit and exact source/fixture hashes.

The existing PCM window mechanism is now used for G.711 too: no more than2048
frames per packet, including after partial-record seek. A-law and mu-law are
stateless across these boundaries. No packet index or source buffering strategy
changes. All file bytes and index entries remain retained/charged as before.
The implementation only changes G.711's private window selection and shares the
constant with PCM. Existing ADPCM block framing is unchanged.

## Measurement scope and tradeoff

The Rust regression records `Vec::capacity()` of the currently live encoded
packet and its decoded float PCM packet, while both objects actually exist.
It also checks decoded lengths against encoded duration and channel count.
This is an observable packet-buffer capacity measure, not whole-player peak
memory, allocator overhead or RSS. It does not count unrelated retained data or
claim time/latency improvements. Larger chunks now entail more packet/decoder
calls; the comparison retains packet counts explicitly.

The8193-frame mono control originally retains8193+32772=40965 bytes for these
two packet vectors;stereo retains81930 bytes. The2048-frame window targets
10240/20480 bytes respectively. Short records retain their previous boundaries.
`measurement.json` records repeated exact before/after results per file.

## Regression qualification

The generator authors20 classic AVI files with embedded short neutral VP9 plus
G.711 audio and20 independent float32 PCM oracles. It covers both laws,all256
codes,8k mono/48k stereo,8193-frame chunks,grouped1/8193/3 frames,nonzero native
start73,and small1/2047/1-frame controls. An independently playable Matroska
video control is included;AVI video playback support is not claimed.

```sh
python3 scripts/generate_avi_g711_windows.py
cargo test --locked --offline --no-default-features --features player --test avi_g711_windows -- --nocapture
```

Generation is separate,standard Python only,no private media,FFmpeg or network.
Tests use checked-in files. Exact PCM/native stamps are compared before/after
raw and wall-time seek,2048 boundaries,EOF/MAX and rewind. Actual audio workers
are captured independently and must preserve PCM and queue anchors. Full-file
resource refusal remains strict. Related AVI PCM/ADPCM/G.711,header/index/RIFF,
seek/EOF and MP4 G.711 tests are recorded in `qualification-results.json`.

## Packet-boundary compatibility

`AudioStream::next_packet` may now split one physical G.711 AVI record into
several timestamped packets. Callers should use packet `pts`/`duration`,not assume
a one-to-one mapping to physical AVI records. Concatenated coded bytes,all decoded
samples,native timestamps,duration,seek landings and public signatures remain
unchanged. No decoder/container validation is weakened.

## Final acceptance

FVID-0520,batch531,total520/1000. All16 large profiles show4.000488x lower
live packet capacity;the four small profiles remain identical. Three baseline
runs and four final candidate observations are identical per file. Five final
gates pass106 test executions,zero ignores,plus core/player checks. Three
additional repeats pass4 tests each;4 tests pass without FFmpeg. All42 files
regenerate byte-identically and2825 source/fixture hashes remain unchanged.
An incidental rustfmt-only helper edit is restored;the complete final
qualification is repeated after restoration. Earlier results are retained under
`formatting-pilot` and are not added to accepted execution totals.
