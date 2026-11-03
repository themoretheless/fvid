# LD-MPS free-source search — 2026-11-03

This is a research receipt, not codec acceptance or a completed implementation.
No production guard was removed and no new playback support is claimed.

## Newly checked primary material

- MPEG N10876, report of the 90th meeting, 234 pages:
  <https://www.algopra.com/publications/pdfs/reports/Rana09_M16977.pdf>.
  The filename is misleading: the document identifies itself as N10876.
  It identifies contribution M16985, **Correction of time-alignment in MPEG
  Surround reference software and conformance**, and M16973, **Technical
  provisions for low-delay multi-channel rendering**. The report describes
  discussion/proposals, not complete algorithms for the outstanding cases.
  It must not be used alone to change filter-bank delay or source association.
- US9728194 cites M16656, **Technical Provisions for Low-Delay SAOC
  Multi-Channel Rendering**, June 2009:
  <https://patentimages.storage.googleapis.com/43/20/55/76b43ae13c8c63/US9728194.pdf>.
  The cited contribution itself has not been obtained. Public patent
  non-patent-literature attachments are a remaining research lead.
- The contributing author's public publication catalogue was inspected:
  <https://heikopurnhagen.net/sigproc/index.html>. It supplies article references,
  but no retrieved complete LD-MPS specification or replacement software.

## Retrieval results

Direct retrieval of both indexed full-standard candidates returned HTTP 403:

- <https://iso.ieclist.ink/iec/ISO%20IEC%2023003-2-2018%20PDF.pdf>
- <https://www.ieclist.ink/iso/ISO%20IEC%2023003-2-2010%20PDF.pdf>

Search-index excerpts from the first URL include SAOC clauses. Those are not a
retrieved, complete Annex B and cannot qualify LD-MPS matrix/history behavior.
No access controls were bypassed and no purchase was made.

The M16656 patent-family lead was followed to European application EP13706500
(publication EP2817802). The public document-list URL
<https://register.epo.org/application?number=EP13706500&tab=doclist> returned
HTTP 403 to the direct client. An ordinary browser displayed Cloudflare
security verification rather than the document list; no challenge was solved
or bypassed. USPTO PatentCenter and Global Dossier links also failed retrieval
through the web tool. No non-patent-literature attachment was obtained.

GitHub repository searches for `mpeg surround`, `SAOC decoder`, and
`mp4spatialdec` returned no additional relevant implementation. This only
records those searches; it is not proof that no public implementation exists.

## Additional primary algorithm sources checked

- **US8054981B2, Energy dependent quantization for efficient coding of spatial
  audio parameters**, description of Figures 9b–9d:
  <https://patents.google.com/patent/US8054981B2/en>.
  This newly located source explicitly describes root-to-leaf local-energy
  reconstruction and gives 5151/5152 examples. Its logarithmic split agrees
  structurally with `energy_split` and the existing tree-0 traversal. However,
  the document deliberately offers alternative dequantizers and alternative
  treatment of the center/LFE split. It does not identify the selected LD-MPS
  behavior for ArbitraryTree, the remaining TTT modes, or multi-input trees.
  These alternatives prevent treating a plausible generalization as normative
  qualification. No new codec acceptance follows from this comparison.
- **Breebaart, Villemoes, Kjorling, Binaural Rendering in MPEG Surround**,
  EURASIP 2008, DOI 10.1155/2008/732895, author-uploaded full article:
  <https://www.researchgate.net/profile/Jeroen-Breebaart/publication/26512784_Binaural_rendering_in_MPEG_surround/links/55ee0bfc08ae199d47bed4a4/Binaural-rendering-in-MPEG-surround.pdf>.
  Section 4.2 describes adapting spatial parameters to QMF subbands for the
  morphed-filter approach. This is not a specification of LD-MPS signalled
  HRTF-grid conversion or its phase/history rules. No Low Delay specification
  was found in this article. It cannot qualify removing the active-HRTF guard.

## Remaining implementation qualification

An additional implementation was checked independently of `fdk-aac-rs`:
<https://github.com/penguin425/fdk-aac-rust/blob/main/crates/fdk-aac/src/sac.rs>.
The inspected parser rejects every `tree_config` except `TREE_212` (lines
64–66 in the retrieved main-branch snapshot). Thus this implementation does not
provide an oracle for the remaining multi-tree/ArbitraryTree/HRTF cases.
Repository existence or a general AAC-parity claim must not be taken as proof
of full LD-MPS coverage. It was read only; no dependency or runtime was added.

Obtain rules or an independently executable, defined reference for unequal HRTF
grids, energy-dependent quantization in remaining tree/TTT/ArbitraryTree cases,
and replaced-fragment source association. Then implement each admitted behavior
with authored synthetic media and acceptance tests for decode and navigation.
Existing refusals and passing refusal tests do not establish acceptance.
