# Indexed audio packet EOF

The public payload accessors Wav/Aiff/Au/Ac3/Flac/Mp3/Aac/Module::packet now
return empty bytes when index >= packets(). Signatures and all valid packet
bytes are preserved. Root and owned ADTS entrypoints share the same correction.
This prevents an indexed consumer that stops on empty payload from repeating
the last audio packet or synthesizing XM release tails indefinitely.

Previously an out-of-range index implicitly selected the last packet (or an
XM tail). A caller deliberately relying on that undocumented selection must
select the last valid index explicitly, for example:

```rust
let last = audio.packets().checked_sub(1).map(|n| audio.packet(n));
```

XM packets() already includes the one valid release tail. That packet remains
nonempty;only indices beyond the declared count become empty. Existing rules
for incomplete coded tails remain unchanged. Seek/time-coordinate APIs and the
private AAC timestamp planner are outside this payload-accessor change.

No new public type,method or signature is introduced. This is one common
indexed-EOF correction across format adapters,not an ID per codec or fixture.
