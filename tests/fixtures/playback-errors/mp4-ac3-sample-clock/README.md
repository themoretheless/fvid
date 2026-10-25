# AC-3 MP4 native sample-clock regressions

Generate separately: `python3 scripts/generate_mp4_ac3_sample_clock.py`.
Normal tests use the checked-in bytes without FFmpeg, a subprocess or a network.
These short movies reuse repository-authored neutral VP9 and synthetic AC-3 tone
from `ac3-mp4-channels`; no private media or parameter sets are copied.

The 45 variants use 900, 1000 and 1001 Hz media clocks with cumulative
floor/ceil/nearest endpoints. Five exact controls cover three single-frame
packets, a two-frame packet, a three-frame packet and damaged-first variants.
The manifest gives the independently known sample count of each packet.

The parent player stamps packet 1 of `ac3-900-floor.mp4` at 31.111111 ms instead
of 32 ms. A 1000 Hz clock represents all packet endpoints exactly but still
loses an intra-tick seek target. The damaged-first controls retain CRC failure
and verify concealment rather than trusting that header's channel mode.

`tests/mp4_ac3_sample_clock.rs` is playback acceptance, not a refusal-only test:
exact native PCM/time, 96 ms duration, six sub-tick seek targets, rewind and the
real worker with a capturing backend. Header recovery also walks syncframes
inside a grouped packet so a later CRC-checked frame can supply the setup.
Physical devices, E-AC-3 programme clocks and arbitrary AC-3 configurations are
not qualified by this finite matrix.
