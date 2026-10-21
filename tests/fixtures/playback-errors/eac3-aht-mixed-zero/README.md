# Mixed zero/VQ/GAQ allocation with disabled dither

Generate separately with `python3 scripts/generate_eac3_aht_mixed_zero.py`.
All media is authored in process from owned bit writers, analytic masking,
normative authoring tables and VP9/Matroska envelopes. No private media,
generated inputs, production decoder, external process or network is used.
Ordinary tests consume the checked-in artifacts without generating them.

The 272 cases cross 17 SNR offsets, two segment starts, four GAQ modes and
native/converted streams. Eight four-band delta-allocation segments alternate
-512/+512, with opposite patterns in the two channels. At fixed exponent 10,
this produces zero allocation next to coded coefficients in every channel.
The decoded allocation covers pointers 0..14; 120 cases contain zero, VQ and
GAQ simultaneously in both channels, while the rest combine zero and GAQ.
Seven VQ books contribute 852 sampled codewords. This does not replace the
earlier all-956-word qualification or independently re-audit shared table values.

Both full-band planes use six-block AHT with dither explicitly disabled.
Controls independently reconstruct the six spectra, set zero-bit bins to zero,
and encode them with conventional scalar mantissas. Oversized reconstructed
mantissas use an explicit power-of-two shift and adjusted exponent. PCM error
is bounded analytically from the actual scalar rounding error, rather than a
tolerance chosen after a failing comparison. Frames use 48 kHz stereo, long
windows, 73 coded bins and three syncframes per stream.

The private codec unit test validates the actual decoded allocation arrays for
every case against the analytic author. This prevents metadata claiming a
mixture that the decoder does not actually encounter.
`tests/eac3_aht_mixed_zero.rs` checks nonzero numeric PCM, headers, raw/Matroska
duration and rewind. Ten selected GAQ-mode-3 cases on both readers additionally
check audio-worker PCM/PTS, assignment, rewind and a 20 ms partial-packet seek.
Transport uses full-precision direct PCM as its exact reference; numeric signal
acceptance uses the independently quantized conventional controls.

The preceding selective dither fix already handles these cases; no production
reconstruction change was necessary. The shared authoring helper now accepts
zero pointers without emitting a codeword. All 1,601 artifacts in the prior
400-case quantizer matrix regenerate byte-identically.

All 1,089 new artifacts / 20,907,489 bytes regenerate identically with PATH empty
and process/socket creation denied (excluding this README). The optional
independent comparison passes 272/272 *conventional controls* at unchanged 1e-5,
maximum absolute error 1.1920928955078125e-6. It does not assert direct AHT
agreement with FFmpeg: prior disabled-dither/VQ4/reference discrepancies remain
recorded separately.

This finite matrix covers the stated flat-exponent, uncoupled mixtures.
Mixed zero with pointers 15..19 in one plane, enabled-noise mixtures, coupling,
LFE, SPX, other rates/windows and physical-device/performance acceptance remain
separately unqualified.
