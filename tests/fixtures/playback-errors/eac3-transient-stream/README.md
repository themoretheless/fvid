# Transient PCM frame-streaming layer

Regenerate separately: `python3 scripts/generate_eac3_transient_stream.py`.
Five owned PCM/video scenarios use regular, short and mixed 1/2/3/6-block frame
sizes, future events announced before nearer events, negative initial history,
an EOF-intersecting future window and an event entirely beyond EOF. Independent
Python whole-buffer processing sorts events by sample position, snapshots before
each event, and clips only unavailable EOF output. It does not share Rust queue
or retention logic. Half-Hann arithmetic is rounded to f32 after each event;
3e-6 is a fixed conservative allowance for up to five overlapping operations
on input bounded by 0.5, not fitted to observed errors.

`tests/eac3_transient_stream.rs` compares actual video PCM with the source,
streams frames through `transient::Stream`, and checks contiguous raw sample
positions, exact output length, scalar PCM, EOF, repeated EOF and reset replay.
Invalid frame/window inputs must not poison subsequent streaming. Two 1000-frame
runs (no events and a future event on every frame) retain at most 2304 PCM samples
and return exactly the input duration. This is a storage/correctness invariant,
not a throughput benchmark. The bounded initial buffer is zero-filled prehistory.

16 artifacts / 263,562 bytes excluding README reproduce with subprocess/socket
denied. The older 27 standalone-window artifacts remain byte-identical. No
private media, encoder, FFmpeg or network; generation is separate from tests.

This qualifies the single-channel streaming PCM component only. The manifest
explicitly sets decoder_integration_accepted=false. E-AC-3 decoding still does
not call it. The adapter must align channel readiness, queue folding/gain/source
window metadata, account for priming and EOF, and support reset/seek and expanded
programmes before decoder transient support can be claimed.
