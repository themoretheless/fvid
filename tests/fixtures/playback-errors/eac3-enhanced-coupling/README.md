# Owned enhanced E-AC-3 coupling

Generate separately with `python3 scripts/generate_eac3_enhanced_coupling.py`.
The generator authors all packets and VP9/Matroska envelopes in process. It
does not read private media or generated inputs, run external codecs or use
network access. Ordinary tests never regenerate fixtures.

The 432 cases cross native/converted streams, nine enhanced coupling begin
codes (0/1/2/3/4/5/6/8/11), four amplitude seeds, default/explicit banding,
ordinary scalar DC, scalar AHT basis 3, and scalar AHT mixed spectra. Together
they exercise all 32 normative amplitude codes, including unity and silence,
the six-bin low-frequency bands, merged higher bands, conditional reserved
fields, and coordinate reuse after the first block. Each has three six-block
48 kHz stereo syncframes, as elementary EC3 and short VP9/Matroska video.

Controls are conventional streams with no coupling at all. The author applies
the normative amplitude to each source spectrum before quantizing the resulting
independent channel coefficient. Numeric comparisons use the conservative
analytic bound `4 * coupled_bins / (32768 * 1024)`: at most one mantissa LSB per
bin between the two scalar representations, the transform factor and overlap.
They do not compare a decoder against a copy of its own enhanced output.

`tests/eac3_enhanced_coupling.rs` checks nonzero numeric PCM, conditional header
flags, raw/Matroska playback, duration and rewind for all cases. Thirty-six
basis-3/seed-0 cases on both readers also test audio-worker PCM/PTS, rewind and
a 20 ms partial-packet seek. Worker transport uses the stream's full-precision
PCM as its exact reference; the uncoupled representation is used by the separate
bounded numeric check. Before implementation, playback failed specifically with
`Unsupported("an E-AC-3 block that couples by enhanced coupling")`.

All 1,729 generated artifacts / 32,313,452 bytes regenerate identically with
PATH empty and process/socket creation denied (excluding this README).

The installed FFmpeg 9.0.2 is not an enhanced-coupling playback oracle: it emits
audio-block decode errors on the enhanced stream. Its decoder source explicitly
lists enhanced coupling as unsupported. Optional comparison of the *uncoupled
controls* passes 392/432 at unchanged 1e-5; 40 mixed cases exceed it (maximum
1.3727025361731648e-5). These are not silently counted as passing.

`benches/check_eac3_enhanced_control_precision.py` diagnoses all 40 differences
by authoring temporary controls with mantissas floored to multiples of four at
exponent 10, reproducing the reference's Q23 coefficient truncation. Those
diagnostic comparisons all pass, maximum error 3.5157427191734314e-8. Production
decoding and checked-in fixtures retain full float precision. The primary gate
remains 392/432; no threshold is raised. Relevant reference source:
[FFmpeg n9.0.2 ac3dec.c](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/ac3dec.c).

The implementation follows
[ETSI TS 102 366 E.1.2.4, E.2.3 and E.2.5](https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.03.01_60/ts_102366v010301p.pdf).
This matrix qualifies fixed stereo geometry, scalar/AHT coupling and initial
parameters with reuse. Explicit mid-frame parameter updates, geometry/mode
changes, multichannel presentations, SPX combinations, coupling VQ/GAQ,
zero-allocation noise and physical audio output remain separately unqualified.
