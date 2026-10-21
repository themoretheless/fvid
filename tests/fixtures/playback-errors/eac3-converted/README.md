# AC-3-converted E-AC-3 stream type 2

Generate separately with `python3 scripts/generate_eac3_converted.py`.
The owned bit writer constructs fresh audio and VP9 video controls without reading
private or generated media. Ordinary tests never regenerate or invoke external
tools. Imported authoring functions do not call the legacy generator workflow.

ETSI TS 102 366 Annex E E.1.2.2/E.1.2.3/E.1.2.4 specify stream-type-dependent
mixing, original AC-3 frame-size, converter-exponent and converter-SNR syntax:
https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.03.01_60/ts_102366v010301p.pdf

The 66 positive cases cover 1/2/3/6 audio blocks per syncframe, BSID 11/16,
long/short/mixed transform windows, optional converted-stereo mixing metadata,
and both first/continuation (`blkid`) forms for short syncframes. Each consists
of three non-silent syncframes. Raw files and short Matroska videos have matching
independent-stream controls with identical audio coding, apart from stream-type
syntax. Reserved original AC-3 frame-size codes 38/63 are negative controls.

`tests/eac3_converted.rs` checks header geometry, exact direct-decoder PCM,
elementary/Matroska stream output, duration and rewind. These are acceptance tests
for type-2 reconstruction, not merely successful refusal tests. Removing type-2
acceptance fails specifically at converted header recognition. Dependent and
reserved stream types remain refused by codec unit tests. Prior type-2 refusal
expectations are replaced with type-3 refusal and positive type-2 playback.

PCM equality proves that independently coded and converted forms reconstruct the
same authored sound. It does not independently qualify every E-AC-3 tool, physical
audio playback or performance. Container clock precision follows Matroska's 1 ms
scale; the corresponding native syncframe sample counts remain exact.
