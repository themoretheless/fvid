# AHT with coupling updates and tool transitions

Generate separately with `python3 scripts/generate_eac3_aht_updates.py`.
Generation is in process from owned bit writers and analytic spectra, without
external codecs, network, private media or generated inputs. Ordinary tests read
the checked-in artifacts and do not regenerate them.

The 356 cases cover layouts 2..7, LFE on/off, native/converted streams,
amplitude updates, E-to-O-to-E tool changes, participant leave/join and changing
coupling start frequency. Only eligible planes carry AHT: their coded extent
and exponent region remain constant for six blocks. Ineligible planes transmit
conventional mantissas. LFE AHT remains eligible during full-band geometry
changes. Coupling AHT is exercised with amplitude updates and one coupling
region. The generator deduplicates activation sets rather than counting identical
all/alternating/LFE masks as distinct cases. It authors nonzero AC basis 3 and
mixed spectra at scalar hebap 19 / exponent 10.

Each stream has three six-block 48 kHz frames. Its independently coded uncoupled
control has six three-block frames, producing the same 96 ms timeline while
staying within the maximum 4096-byte syncframe size even at 5.1. Both elementary
EC3 and short VP9/Matroska versions are included. Matching total sample count,
numeric PCM and duration also check overlap continuity across different frame
boundaries. Controls apply channel amplitudes to the reconstructed spectra
before scalar quantization. The analytic numeric bound is
`4 * 73 / (32768 * 1024)` plus 1e-7 floating point slack.

`tests/eac3_aht_updates.rs` checks every AHT plane flag, nonzero PCM,
raw/Matroska decoding, speaker masks, duration and rewind for all 356 cases.
All 178 basis-3 cases on both readers additionally check exact audio-worker
PCM/PTS, backend assignment, rewind and a 5 ms partial-first-packet seek. The
transport reference is direct full-precision PCM, while the separate numeric
gate uses the independently quantized uncoupled controls.

The existing decoder passes this matrix without reconstruction changes. This
adds missing acceptance evidence, rather than claiming an invented prior bug.
All 1,425 generated artifacts / 39,880,272 bytes regenerate identically with
PATH empty and process/socket creation denied (excluding this README).

Optional independent comparison of the uncoupled controls passes 356/356 at
unchanged 1e-5, maximum absolute error 7.251590432133526e-6. This validates the
controls; it does not assert that FFmpeg decodes enhanced coupling itself.
Production and ordinary tests have no external-codec dependency.

This is a finite high-allocation, long-window, 48 kHz matrix. Low-allocation
noise, coupling VQ/GAQ, SPX combinations, dependent presentations, other sample
rates/window combinations and physical-device/performance acceptance remain
separately unqualified. In particular, comparing different legal random dither
sequences byte-for-byte is not a noise qualification gate.
