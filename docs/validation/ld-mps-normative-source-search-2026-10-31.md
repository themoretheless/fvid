# LD-MPS normative source retrieval, 2026-10-31

Status: full ISO/IEC 23003-2:2010 or :2018 has not been retrieved.
This research does not add codec acceptance or extend playback/seek coverage.

## Located publications

- Official 2018 catalogue: <https://www.iso.org/standard/73122.html>.
- 2010 preview: <https://cdn.standards.iteh.ai/samples/51827/12e1684e3e784c6c9ac44da8b923a2e4/ISO-IEC-23003-2-2010.pdf>.
  Its contents locate Annex B at printed page 109; the preview does not
  provide the required annex and window tables.
- 2018 preview: <https://cdn.standards.iteh.ai/samples/73122/3b090c9c05dc459d91aedef0addbcf18/ISO-IEC-23003-2-2018.pdf>.
  Its contents locate Annex B at printed page 150. Page numbers and table
  numbering must not be assumed identical across editions.
- Original reference software remains available from ISO:
  <https://standards.iso.org/iso-iec/23003/-2/ed-2/en/SAOC_IS7_ReferenceSoftware__20160603.zip>.
  SHA-256: `b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`.

## Failed full-document retrievals

- `https://iso.ieclist.ink/iec/ISO%20IEC%2023003-2-2018%20PDF.pdf`:
  HTTP 404. A search index still exposes document fragments; these are not
  a locally verified full publication.
- `https://www.ieclist.ink/iso/ISO%20IEC%2023003-2-2010%20PDF.pdf`:
  HTTP 403. Indexed results identify Table A.21 as the QMF LD 256 window,
  but do not supply a verified complete source.
- IEC lists ISO/IEC 23003-1:2007/COR1:2008 as a free 56-page corrigendum:
  <https://webstore.iec.ch/en/publication/11007>.
  The guessed public preview path
  `https://webstore.iec.ch/preview/info_isoiec23003-1-cor1%7Bed1.0%7Den.pdf`
  returns HTML despite HTTP 200, not a PDF. It has not been used as
  normative evidence. No account, checkout or purchase was performed.
- The former public ITTF listing now redirects users to ISO/IEC webstores:
  <https://standards.iso.org/ittf/PubliclyAvailableStandards/>.

## Unequal HRTF grid qualification boundary

In the extracted original `mp4spatialdec/src/sac_bitdec.c`, lines 2969–2981
load the signalled HRTF band count and that many directional values.
`sac_calcM1andM2.c`'s `Calculate3Dinv` accesses those values by the current
parameter-band index. This path does not itself establish a conversion
between different HRTF and spatial parameter grids.

The existing C component probe supplies HRTF values directly at the queried
band; it does not qualify unequal-grid parsing and conversion. Therefore
removing the owned unequal-grid guard, zero-filling missing cells or assuming
a remapping rule would not be a validated fix. Keep the explicit refusal
until the conversion is supported by authoritative evidence and authored
playback/seek acceptance fixtures. Existing equal-grid acceptance remains
distinct from full Annex B conformance.
