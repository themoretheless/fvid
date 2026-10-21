# E-AC-3 nonzero AHT AC spectra

Generate separately with `python3 scripts/generate_eac3_aht_ac.py`. Generation
uses owned bit writers and analytic spectra only: no generated media, private
input, external process, external codec or network. Ordinary tests use checked
fixtures and do not invoke the generator.

Thirty-six cases cross native/converted types, left/right/both AHT planes and
each of the five nonzero AC basis coordinates plus their mixture. Each contains
three six-block stereo frames at 48 kHz, as elementary audio and owned VP9 video.
The corresponding conventional scalar control is authored independently from
the inverse-DCT equation, not by decoding the AHT packet. Allocation remains
hebap 19 in this matrix; it does not qualify other VQ/GAQ allocation paths.

Basis 3 has exactly representable +/-1 inverse-DCT weights, so PCM must match
its conventional control byte for byte. For the irrational bases, conventional
16-bit mantissas round each analytic coefficient by at most half an LSB. The
manifest records a conservative sample-error bound from the largest per-block
L1 spectral rounding error: four times that error divided by 32768 and 256
(the overlap/add factor and exponent scale). The test adds 1e-7 for floating
arithmetic, rather than loosening the bound in response to a failed test.

`tests/eac3_aht_ac.rs` checks all basis spectra, header plane flags, raw/native
Matroska PCM, duration and rewind. The exact basis also runs through the audio
worker with rewind and a 20 ms seek inside the first packet, checking PCM and
presentation clocks. A sensitivity experiment which discards all AHT AC terms
leaves the previous DC matrix green but makes both new tests fail, establishing
that these fixtures exercise the missing temporal coverage. This is a test
sensitivity experiment, not a claim that the preceding implementation was broken.

The independent optional FFmpeg PCM oracle lives only in
`benches/check_eac3_aht_reference.py --fixtures eac3-aht-ac`; it does not generate
fixtures or run in ordinary tests. Normative transform source: ETSI TS 102 366
V1.3.1 Annex E E.2.4.5. VQ/GAQ playback, coupling/LFE, physical output and
performance are separate open qualification items.
