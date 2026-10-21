# E-AC-3 coupling updates and tool transitions

Generate separately with `python3 scripts/generate_eac3_coupling_updates.py`.
All packets, conventional controls and VP9/Matroska envelopes are authored in
process, without external codecs, network, private input or generated inputs.
Ordinary tests never regenerate fixtures.

The 448 cases span layouts 2 through 7, with/without LFE, native/converted
streams and default/explicit enhanced banding. Stereo uses six blocks and the
other layouts three blocks, within the syntax's maximum syncframe size. Cases
include amplitude updates on selected channels, participant leave/join, moving
coupling start frequency, E-to-O, O-to-E and E-to-O-to-E transitions. Tool-only
transitions are tested both with new and reused exponent sets. Twenty-four wide
stereo cases reach ordinary default merged bands and test ordinary banding
initialization when the frame begins with enhanced coupling.

Controls code independently reconstructed spectra without coupling. Scalar
control rounding is checked under an analytic L1 bound, fixed from the authored
bandwidth: `4 * maximum_bins / (32768 * 1024)`. No tolerance is chosen after a
failure. The original 1,696 media artifacts from the first 424-case matrix remain
byte-identical when the 24 wide cases are added.

`tests/eac3_coupling_updates.rs` checks nonzero PCM, raw/Matroska playback,
headers, duration, channel masks and rewind. Ninety-six selections on both
readers additionally exercise exact audio-worker PCM/PTS, speaker assignment,
rewind and a 5 ms seek inside the first packet. The transport oracle is the
full-precision direct decode; the separate bounded numeric test uses uncoupled
controls to establish signal correctness.

Before the fix, `l2-f0-t0-e-o-x0-r0` failed actual PCM with maximum error
0.7034164, versus 1.4524324035644532e-5 bound. Switching tools incorrectly
reset the coordinate-exists history, causing the explicit update bit to be read
as coordinate data. Treating a mode switch as a new coupling region also muted
valid exponent reuse. The fix retains participant history, compares actual bin
ranges, and initializes ordinary default banding independently of the first tool.
This follows
[ETSI TS 102 366 E.1.2.3/E.1.2.4](https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.03.01_60/ts_102366v010301p.pdf).

All 1,793 artifacts / 33,539,996 bytes regenerate identically with PATH empty
and process/socket creation denied (excluding this README). All 448 uncoupled
controls also pass the optional independent FFmpeg comparison at unchanged
1e-5, maximum error 2.9910734156146646e-6. This verifies the controls, not direct
enhanced decoding in FFmpeg, which remains unavailable.

This finite matrix covers high-allocation scalar data with the specified
transitions. AHT combined with tool/participant changes, SPX combinations,
low-allocation noise, dependent presentations, physical output and performance
remain separately unqualified. It is not codec-wide completion.
