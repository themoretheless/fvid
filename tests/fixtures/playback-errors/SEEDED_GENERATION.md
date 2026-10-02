# Offline playback fixture regeneration

Run `python3 scripts/generate_playback_error_samples.py` to regenerate the
28-file MP4/MOV/WebM/PCM regression corpus. Use `--output-dir PATH` for review
without changing checked-in fixtures. Generation uses only Python's standard
library and never invokes FFmpeg, libopus, another encoder or the network.

`synthetic-playback-seeds.zip` preserves the existing synthetic control clips
and independently produced Opus PCM references. These are the original public
96x64 test patterns and tones, not private recordings or parameter sets. This
is seed-based reproduction, not a new FVid encoding implementation: encoded
controls and reference PCM remain fixed; MP4 edit lists, sample entries and
composition timestamps are constructed by the generator itself. Keeping fixed
seeds preserves the original failures and packet/pixel/PCM expectations across
machines, including the explicitly forced Opus hybrid mode.

The seed JSON records SHA-256 for every archived file. All seeds are validated
before output creation. The generated JSON pins all 28 outputs including
metadata mutations. Run `python3 scripts/test_seeded_playback_generator.py`
explicitly to verify exact reproduction and corruption rejection. Ordinary
Cargo acceptance tests consume existing fixtures and never run the generator.

The prior FFmpeg/libopus-based generator was replaced; reference provenance is
retained here rather than presenting reference PCM as independently computed
by FVid. Other fixture generators/validation workflows remain separate work.
