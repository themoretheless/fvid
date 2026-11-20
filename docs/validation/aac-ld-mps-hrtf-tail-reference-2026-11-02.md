# Active unequal HRTF grids: original-source tail dependence

This diagnostic narrows the remaining implementation dependency: the pinned original C `Calculate3Dinv` is not a sufficient oracle for all independently signalled spatial/HRTF resolutions. No production admission, PCM mapping or conformance improvement is claimed here.

The own wrapper creates two contexts with identical parameters in every signalled HRTF band. Only values in the untransmitted tail of the directional arrays differ. It calls the unchanged original function at the final spatial band, with an authored basis, asymmetric positive levels and nonzero phases. Both runs remain finite. Across all 49 spatial/HRTF count pairs, the 21 pairs with more spatial bands than HRTF bands produce different inverse matrices. The other 28 pairs produce exactly identical results. These controls demonstrate the direct-index dataflow dependency; they do not establish normative conversion semantics for either group.

The original parser initializes directional arrays only up to `hrtfParameterBands` (`sac_bitdec.c`, spatial initialization), while `Calculate3Dinv` indexes them directly by the spatial band (`sac_calcM1andM2.c`). The declared HRTF count does not constrain this access inside that function. Do not replace the production refusal with zero padding, repeated parameters or the output of this C routine under an arbitrary tail initialization.

This is an original-reference diagnostic, not a newly discovered FVid media failure. FVid already explicitly refuses the active unequal-grid case, covered by the own synthetic video corpora `aac-ld-mps-hrtf-grids` / `aac-ld-mps-hrtf-all-grids` and their existing automated tests. No external or private media is added. The previous exact-partition analysis remains useful, but neither it nor this diagnostic resolves the phase gate, matrix smoothing, cutoffs or in-band history semantics.

Reproduce explicitly, outside ordinary tests, with the pinned external archive:

```sh
python3 scripts/validate_aac_ld_mps_hrtf_tail_reference.py /path/to/SAOC_IS7_ReferenceSoftware__20160603.zip
```

The adjacent JSON records all 49 pairs, source/archive/harness hashes, compiler and output hash. Original source is not modified or committed. The wrapper and inputs are authored here; no copied codec parameter sets are used. Ordinary tests require neither the archive, a C compiler, FFmpeg nor network.

Further source search on 2026-11-02 found indexed ISO/IEC 23003-2:2018/2010 PDFs at `iso.ieclist.ink` / `www.ieclist.ink`, but direct access returned 404; inferred `previewnorm.com` candidates returned 403. Search snippets are not adequate evidence for equations or tables. Full Annex B and the relevant MPEG Surround mapping/phase rules remain a source dependency for this implementation.
