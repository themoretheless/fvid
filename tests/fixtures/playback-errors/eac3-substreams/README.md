# Multiplexed E-AC-3 base-programme playback

88 owned stereo base cases: native/converted independent substream 0, conventional/AHT
full-band planes, coded/noise base signals, every additional independent ID
1..7, implicit/explicit dependent L/R maps, all seven extra programmes, and a
mixed bundle with dependent streams associated with programmes 0 and 7.
Each case includes three raw bundles, an owned VP9/Matroska video, both
base-only controls, and the isolated additional frames. Two extra files reproduce a truncated
unselected tail in raw and Matroska forms.

Regenerate separately: `python3 scripts/generate_eac3_substreams.py`.
The author writes dependent syntax explicitly: compr on the last dependent,
optional chanmap and no independent-only converter fields. It uses no FFmpeg,
subprocess or network and includes no private source media/parameters.

With only the new decoder routing branch disabled, the actual fixture fails
with `Unsupported("a second E-AC-3 substream of the same presentation")`:
`/tmp/fvid-substreams-before.log`. The branch was restored before qualification.
Five integration tests accept exact primary PCM/PTS, unchanged duration,
raw/Matroska rewind, worker assignment/rewind/20 ms partial seek, skipped-only
packets without consuming lead/RNG/history, reserved-type refusal and bounded
skipped lengths. A truncated raw skipped tail preserves three primary packets
but is still reported at EOF; the Matroska form refuses the truncated packet
with the specific skipped-substream length error. The initial reader integration
rejected the raw file as a false container because of its 90-percent coverage
heuristic; that failure is preserved in
`/tmp/fvid-substreams-truncated-tail-before.log`. The heuristic now distinguishes
a recognized incomplete skipped tail from unexplained trailing bytes. A private unit checks every authored envelope's
type, ID, 4096-byte size and CRC. Envelope skipping does not certify or decode
the skipped audio payload.

This implements required compatibility output for independent programme 0,
up to 5.1 channels (ETSI TS 102 366 E.2.8.1/E.2.8.2). It deliberately does
not claim reconstruction of dependent replacement/extension channels, output
above 5.1, explicit programme selection or acceptance of all container layouts.
The base reconstruction-header refusal test remains, with a clarified name:
that parser must not decode an extension as programme 0; the packet walker
now bypasses it for unselected frames. These are base-playback acceptance
tests, not extension-channel acceptance tests.


Authoring correction (2026-10-21): earlier `ind2`..`ind7` cases jumped directly
to the named independent ID, and the final dependent of programme 7 incorrectly
used dependent ID 7. E.1.3.1.2 requires independent IDs to be sequential and
dependent IDs to restart at 0 after each independent programme. Cases now
include all preceding independent IDs; the mixed case ends with dependent 0
after independent 7. A private regression checks that sequence. These correct
fixtures are requalified without changing the 88-case count. Old hashes remain
at `/tmp/fvid-substreams-prior-invalid-order-hashes.json`; corrected hashes:
`/tmp/fvid-corrected-routing-fixture-hashes.json`. Prior blanket claims that all
of the higher-ID fixtures were conforming are superseded by this correction.
