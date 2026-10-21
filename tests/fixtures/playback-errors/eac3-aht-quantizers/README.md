# E-AC-3 AHT VQ/GAQ playback

Generate separately with `python3 scripts/generate_eac3_aht_quantizers.py`.
No decoder, generated media, private input, external process, FFmpeg or network is
used. The static authoring codebooks are numeric ETSI TS 102 366 Tables E.3.1..7;
they share normative values with the decoder, so conventional-control equality
alone is not independent validation of those table values.

Four hundred cases cover every nonzero hebap (1..19), all four GAQ modes, native
and converted stream types, all 956 VQ codeword indices, signed small mantissas
and positive/negative large escapes. Flat exponent-8/10 spectra have a closed-form
mask for this 73-bin domain; its band PSD, knee, floor and SNR offset determine
payload widths without running a decoder. This is an authoring shortcut for these
fixtures, not a replacement for the production masking algorithm.

The 200 exponent-8 cases include output saturation. Another 200 retain exactly
the same coded VQ/GAQ vectors with exponent-10 headroom and adjusted SNR offsets
that preserve the allocation pointers. Conventional controls are authored from
reconstructed spectra, using sufficient exponent headroom to represent them.
The manifest's L1 spectral rounding bound propagates the control's quantization
error through exponent scaling and overlap/add. Tests add 1e-7 for floating
arithmetic; thresholds were not increased in response to failed comparisons.

`tests/eac3_aht_quantizers.rs` checks numeric PCM, geometry, flags, raw/Matroska
presentation, duration and rewind. Sixteen selected cases also exercise the audio
worker on both readers, with exact full-precision PCM preservation, rewind and
20 ms partial-packet seek. Separate numerical fault experiments (forcing VQ index
zero or removing GAQ large-mantissa bias while retaining payload consumption)
fail this playback test specifically on PCM.

The optional reference benchmark remains strict and is not wholly green:
196/400 pass direct unclipped float comparison. All 128 headroom cases without
VQ4 pass at 1e-5. The 72 headroom VQ4 cases disagree with the installed reference's
codebook. A separate optional experiment shows three reference decodes follow
its captured 31-row book plus implicit zero row, rather than the normative 32-row
book retained by FVid. Full-scale scalar cases additionally expose FVid's existing
saturating PCM policy versus float reference headroom; even after saturation,
two scalar cases exceed the unchanged 1e-5 reference threshold slightly. These
results are recorded, not silently treated as accepted reference comparisons.

Reference tools live only under benches/. Ordinary tests never run them.
Coupling/LFE, zero-allocation dithering, changing frame configurations, broader
codec families, real devices and performance remain separate qualification work.
