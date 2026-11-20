# AAC ER/MPS ancillary transport controls

224 own synthetic cases cover extension tags9/12, ancillary kinds0/1, direct
and EXT_DATA_LENGTH envelopes, four fragment sequences, and lengths around
short/escaped/long field boundaries through a 65805-byte child. Every case runs
at all eight bit offsets, with checkpoint replay and a following-extension
sentinel. 224 additional controlled sequences deliver 672 spatial frames in
start/continue/stop pieces to the existing PCM chain; all 1677312 saved PCM
samples remain unchanged. Inputs and packets are synthetic; no private media,
samples or codec parameter sets are copied.

Explicit generation, offline:

```sh
python3 scripts/generate_aac_ld_mps_transport_fixtures.py
```

Ordinary `aac_ld_mps_transport` tests load saved controls and PCM goldens without
network, FFmpeg, generation, reference software or a compiler. Header-and-frame
payload bytes remain opaque here; this is not in-band configuration/rendering
acceptance. The PCM test supplies decoded synthetic downmix and residual QMF;
it is not compressed AAC access-unit or movie playback acceptance.

Primary grammar facts were inspected in pinned AOSP FDK AAC commit
`ee76460efbdb147e26d804c798949c23f174460b`. Tags, flags and source hashes are in
the source report. No foreign implementation or dependency enters production.
Orphan continue/stop fragments are explicitly rejected instead of attempting
unsynchronized recovery. A complete start-stop message can replace a pending
sequence. Validation and allocation precede content changes; pending prefixes
are appended without cloning the entire accumulated message per fragment.
Malformed envelopes, reserved types, nested length wrappers and invalid order
retain state/cursor. `finish` reports incomplete messages without dropping them.
Reset explicitly discards pending state. There is no inherited 2048-byte
reference-buffer limit.

Native AAC extension dispatch, PCM/timestamp queue association, in-band spatial
configuration admission, coupled LD-SBR filtering and actual movie playback/seek
remain open. Native spatial renderer refusal stays enabled; zero accepted
spatial movies are counted by these component controls.
