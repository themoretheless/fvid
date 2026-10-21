# Native base-layer hev1 bootstrap

Generate separately with `python3 scripts/generate_hevc_inband_base_bootstrap.py`.
Generation and ordinary Rust tests use no FFmpeg, external codec, network or
private media. The six inputs and their YUV oracles are existing owned synthetic
fixtures in the parent directory; coded samples are preserved when remuxing.

The manifest contains 48 acceptance movies: six coded seeds across eight
parameter layouts. They cover empty hvcC with inline, combined-prefix,
separate-prefix and reverse-order PPS/SPS/VPS reception; VPS-only, SPS-only,
PPS-only and SPS/PPS recorded configurations. Seeds include dependent segments,
separate colour planes, mixed component depths, tiles/WPP and SCC ACT.

`tests/hevc_inband_base_bootstrap.rs` verifies the recorded configuration,
complete decoded Y/Cb/Cr samples, source sample indices, PTS, 40 ms durations,
EOF, rewind and sync seek. Direct decoder controls exercise reset and parameter
readiness. Original fully configured seeds remain strict-decoder controls.
Before the fix, empty-inline playback failed at open with
`HEVC configuration has no parameter-set pair`.

`from_configuration` retains its complete-pair requirement. The separate
`from_inband_configuration` API accepts an incomplete initial bank and exposes
`try_parameters` until a pair is available. PPS reception can precede its
missing SPS. This does not establish arbitrary replacement ordering when an
incompatible older SPS with the same ID is already present.

`unresolved-pps-budget.mp4` is a distinct aggregate-budget refusal fixture,
not an accepted playback movie: many individually small PPS packets reference
an absent SPS. The test checks aggregate refusal and reset-required error
state. `recovery.packet` contains the original seed's parameter-only packet to
verify recovery after reset; it does not claim malformed PPS playback support.
