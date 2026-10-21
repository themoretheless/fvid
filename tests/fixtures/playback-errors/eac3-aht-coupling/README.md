# Coupled E-AC-3 AHT and phase-coordinate reuse

Generate separately with `python3 scripts/generate_eac3_aht_coupling.py`.
Generation uses owned bit writers and analytic spectra, without external codecs,
network, private source media or generated input files. Ordinary tests consume
the checked-in media; generation is never part of test execution.

The 192 cases cross native/converted streams, coupling starts at bins 37/61,
phase restoration off/on, all eight independent/coupling AHT flag combinations,
and DC, AC basis 3, and mixed spectra. Each has three six-block syncframes at
48 kHz in raw and short VP9/Matroska form, with conventional controls.
Controls resend equivalent coupling coordinates and phase flags in every block;
test streams reuse them after the first block. The phase flags alternate by band.
Coordinate gains are 1/2 and 3/8. Coupling uses scalar hebap 19 at exponent 10.

`tests/eac3_aht_coupling.rs` requires exact PCM for representable DC/basis-3
spectra and an analytic L1 quantization-error bound for mixed spectra. It checks
conditional AHT flags (including the coupling plane), nonzero PCM, both readers,
duration and rewind. All 128 exact cases additionally exercise audio-worker
PCM/PTS, rewind and a 20 ms seek inside the first packet on both reader paths.

Before the fix, numeric and worker acceptance tests failed on phase-enabled
streams: the common coupling-coordinate routine cleared `phsflginu` when both
channels reused coordinates. `t0-b0-p1-m1-dc` failed exact PCM although the
preceding phase-disabled cases passed. The fix leaves this strategy state intact
until the stream actually updates it. This is playback acceptance, not an
unsupported refusal test.

The 769 generated artifacts total 14,352,308 bytes, excluding this README.
All regenerate identically with PATH empty and process/socket creation denied.
The optional benchmark/reference script `benches/check_eac3_aht_reference.py`
accepts `--fixtures eac3-aht-coupling`. All 192 comparisons pass its unchanged
absolute threshold 1e-5; maximum observed error is 8.279035682789981e-6.

This finite matrix covers ordinary coupling with fixed geometry, scalar AHT,
coordinate reuse and phase restoration. It does not qualify enhanced coupling,
changing coupling regions, VQ/GAQ in the coupling plane, zero-allocation noise,
dependent substreams, hardware output or performance.
