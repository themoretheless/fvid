# AAC-LD nearest-defined frequency tables

Generate explicitly with `python3 scripts/generate_aac_ld_nearest_rate_fixtures.py`.
Ordinary tests never generate fixtures or use FFmpeg/network access.

360 authored eight-AU programmes, each in MP4 and Matroska, cover core480/512,
7.35/8/11.025/12/16/18.782/55.426/64/88.2/96 kHz (indexed and explicit),
ten configured channel layouts and all eight spatial trees. The nonzero core
includes first/final spectral bands, sine/low-overlap windows and LTP updates.
At128 QMF bins,480-sample cores span four AUs per spatial frame. Navigation
checks include either side of core/spatial boundaries, backwards/repeated
seeks, live-history replacement, EOF, rewind and presentation timestamps.

Core packet bits are regenerated and asserted equal to existing own24/48-kHz
`aac-ld-layout` controls. The frozen independent sparse-cosine reference remains
in `../aac-ld-layout-reference.f32le`; changing Fs does not change those spectral
values when the nearest-defined tables are selected. Spatial acceptance compares
with an explicit PCM spatial composition of this frozen core reference. This
isolates rate-table admission; it is not a new independent whole-MPS oracle.

Before the fix,40 nominal controls pass and both new core/spatial acceptance
tests fail specifically with `unsupported AAC LD band geometry`. After the fix,
the same movies are accepted. No private or ISO media/audio/parameter sets are
copied. External ISO inputs are used separately, outside the fixture directory.

Normative basis: ISO/IEC14496-3:2009 section4.5.1.1,Table4.82 specifies nominal
frequency grouping and nearest-defined tables when a table lacks that nominal
entry. LD section4.6.17 defines outer band geometries at22.05/24 and44.1/48 kHz.
LD TNS limits use the same mapping; no new numeric filter tables are invented.
