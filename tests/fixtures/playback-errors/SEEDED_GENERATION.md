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

The Y4M header grammar, bounded line reader and container probe are now owned by
`fvid-media::owned_y4m` / `owned_y4m_probe`. Frontend compatibility uses the same
parser source body and delegates metadata inspection; it does not introduce a
second independent grammar. Public library probing selects the owned route for
progressive 8-bit and 9/10/12/14/16-bit 420/422/444. Legacy-enabled probing retains
its prior backend for unsupported chroma/interlace modes.

`tests/native_y4m_library_probe.rs` checks 13 chroma/depth forms, frontend/library
metadata equality, reduced fractional frame rates, tagged frame markers, bounded
header lines and truncated payloads. Synthetic frame payloads are constructed in
the test itself; no private media, external codec or fixture generator is used.
Core Y4M processing and root/domain API compatibility tests remain enabled.

`fvid_media::decode_video` now selects owned Y4M raw decode-and-discard for
this grammar, including when legacy support is enabled. It consumes every
sample byte through 8 KiB scratch storage, reports frame geometry/pixel format
and rejects incomplete payloads; metadata-only seeks are not counted as decode.
The frame-rate parser is shared with owned probing, including positive rational
validation, reduction and the existing omitted-rate default. Public integration
tests require the owned backend for all 13 chroma/depth variants and additionally
exercise three-byte reads and truncated-tail refusal on the synthetic WAVE-probe
Y4M control. Transformed video decode and other containers remain migration work.
