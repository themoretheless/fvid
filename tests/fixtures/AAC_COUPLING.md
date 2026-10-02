# Synthetic AAC coupling regression oracles

Regenerate all 16 streams (including two absent-target refusals) and the 14
accepted PCM references with:

```sh
python3 scripts/generate_aac_coupling_sample.py --all
```

The generator writes AAC-LC bits directly. Its independent PCM oracle evaluates
sparse inverse MDCT cosine sums, sine windows and overlap-add in Python, without
calling FVid's packet decoder, FFT or synthesis. Spectra contain known +/-1
four-tuples at scalefactor 140. Explicit gain-code arithmetic covers shared,
left/right and separate targets, signed multiband gains and split short-window
groups. Before-TNS cases apply the encoded first-order forward predictor before
synthesis. Long-start/eight-short/long-stop transitions preserve overlap history.
No FFmpeg, reference executable or network is needed for generation or tests.

`tests/native_aac_coupling.rs` checks native PCM, reset/checkpoint restoration,
channel selection and atomic refusal when a target is absent. The negative
before-TNS case is stereo; its filename intentionally omits that suffix, so use
`--all` rather than deriving options from filenames.

Run the optional external comparison only as an explicit benchmark:

```sh
python3 scripts/benchmark_aac_coupling_reference.py
```

That benchmark invokes FFmpeg, counts PCM samples, checks peak absolute error
below 1e-7 and reports elapsed time including process startup. Production and
ordinary tests use the analytical references and do not invoke it.
