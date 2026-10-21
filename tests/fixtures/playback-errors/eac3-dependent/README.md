# Native dependent-substream reconstruction

496 owned cases: 480 layout/LFE/ID/map/AHT/basis combinations plus 16 single
location-bit witnesses covering every Table E.1.4 channel-map entry. Each case
includes extracted dependent frames, a conventional independent scalar control,
a VP9/Matroska video carrying the complete independent/dependent ordering, and
a base-only control video. Two malformed-map raw/video files exercise refusal.

Regenerate separately: `python3 scripts/generate_eac3_dependent.py`.
The author writes dependent syntax directly, including custom maps and omission
of independent converter fields. Only the last dependent stream carries compr.
ID-7 extraction videos contain dependent IDs 0..7 in order. Generation needs
no FFmpeg, subprocess or network and contains no private source media.

Full-band widths <=2 use six blocks, all eligible AHT-plane masks and optional
LFE; wider layouts use three conventional blocks. All acmod values 0..7, LFE
off/on, IDs 0/7, implicit/explicit maps and DC/basis-3/mixed spectra are included.
The 16 location witnesses select mono for singleton map bits and stereo for
pair bits. Header validation counts pairs as two coded channels.

`DependentDecoder` reconstructs one selected dependent stream into coded-order
PCM; `dependent_frame` retains its map and ID. Three integrations verify every
plane against independently authored conventional scalar controls, with exact
DC/basis-3 or a bound from actual spectral rounding error for mixed spectra.
The expected native-to-coded permutation is explicitly authored for each
layout, including LFE movement. Reset and extraction from all videos preserve
continuous PCM. Wrong map cardinality, mismatched identity and independent
input are refused; the malformed video reaches the exact map-count guard.
The ordinary primary probe intentionally still refuses dependent reconstruction.

Optional reference: `benches/check_eac3_aht_reference.py --fixtures eac3-dependent
--control --probe <audio_probe>` passes 496/496 at the unchanged 1e-5 threshold,
maximum error 7.175048267527018e-6. There are 48 unique independent control
payloads in those 496 case comparisons. This validates scalar controls, not
a direct FFmpeg dependent-presentation comparison. Report:
`/tmp/fvid-dependent-control-reference.json`.

This matrix remains a low-level reconstruction qualification. Fixed-layout
presentation assembly and 7.1/10-channel worker output are now qualified
separately by `../eac3-presentation/`. Full extended device maps, explicit
programme selection, heavy compression and dependent coupling/SPX/VQ/GAQ/
transient combinations remain open or unqualified.
