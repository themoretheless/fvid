# E-AC-3 AHT LFE and speaker assignment

Generate separately with `python3 scripts/generate_eac3_aht_lfe.py`.
All media is authored locally; no private input, external codec or network is used.
Ordinary tests consume checked-in fixtures and never regenerate them.

The 48 cases cross native/converted streams, all eight left/right/LFE AHT
flag combinations, and DC, exact AC basis 3, and mixed spectra. Every stream
has three 4096-byte syncframes at 48 kHz, six blocks per frame, stereo plus LFE.
Raw, short VP9/Matroska and VP9/MP4 versions have matching conventional controls.
The MP4 EC3SpecificBox follows
[ETSI TS 102 366 Annex F.6](https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.03.01_60/ts_102366v010301p.pdf).

`tests/eac3_aht_lfe.rs` checks nonzero LFE PCM, exact representable controls,
analytic spectral quantization bounds for mixtures, channel flags, duration,
rewind, and all three readers. Sixteen exact active-LFE cases also pass through
the audio worker on each reader, including a 20 ms seek inside the first packet.
The test backend requires FL/FR/LFE mask 0x0b. Stereo router impulses verify
that an LFE sample is not mistaken for a center sample.

Before the assignment fix, the three reader layout tests failed with `None`
instead of `Some(11)`; numeric PCM already passed. These are acceptance tests,
not tests accepting an unsupported refusal. The fix derives AC-3/E-AC-3 speaker
assignment from encoded headers and preserves it through the reader API.

The 289 generated artifacts total 6,859,668 bytes (excluding this README).
All regenerate byte-identically with PATH empty and process/socket creation
denied. Optional reference comparison belongs to `benches/` only:

```
python3 benches/check_eac3_aht_reference.py --probe /path/to/audio_probe --fixtures eac3-aht-lfe
```

Observed reference result: 48/48 at unchanged absolute-error threshold 1e-5;
maximum 7.235532393679023e-6. This covers scalar hebap 19 and fixed initial
layout only. It does not qualify coupling, VQ LFE, dependent streams, in-band
layout changes, hardware output or performance.
