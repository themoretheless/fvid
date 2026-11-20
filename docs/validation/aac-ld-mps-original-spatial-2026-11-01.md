# Original ISO spatial decoder on identical LD5151 core PCM

This audit separates the open original LD5151 waveform residual from the
spatial reconstruction stage. The preceding core-isolation audit established
byte-identical recomposition with the full owned AAC decode, for both modes.
The exact same 1443840 mono core samples now feed the complete original ISO
spatial decoder, including its original configuration/frame parser, matrix
calculation, interpolation, temporal processing, decorrelation and QMF banks.
No owned matrix endpoints or processed spatial parameters are fed to ISO.

The original source archive is fetched separately from:
https://standards.iso.org/iso-iec/23003/-2/ed-2/en/SAOC_IS7_ReferenceSoftware__20160603.zip

Archive SHA256:
`b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`.
All archived source files remain unchanged. The harness supplies raw float file
IO in place of libtsp file writing; no codec, spatial or temporal algorithm is
stubbed. A declaration header supplies the missing legacy `myexit` prototype.
The experiment does not add foreign code to production or ordinary tests.

The original record pin and all core bit boundaries are checked by the existing
isolation tool. The source-specific transport adapter then preserves the five
SSC bytes and one complete frame-only spatial message per original AU. Fill
bytes are validated, not interpreted as spatial data. It refuses fragmentation,
other message types and layouts outside this pinned input. This is a research
adapter, not a general transport implementation.

## Results

All four experiments emit exactly 2820×512×6 float scalars. Each comparison is
whole-stream, zero-lag, without trimming, delay search, gain fit or correction.

| ISO polyphase control | Decorrelation | Worst-channel RMS vs FVid | Max absolute difference vs FVid |
| --- | --- | ---: | ---: |
| Original float PI | Real (`decType=0`) | 1.741283982e-8 | 4.768371582e-7 |
| Original float PI | Complex (`decType=1`) | 1.788999885e-8 | 4.619359970e-7 |
| Separate precise PI | Real | 1.138280979e-8 | 2.309679985e-7 |
| Separate precise PI | Complex | 1.239312688e-8 | 1.905718818e-7 |

The precise-PI experiment overrides only the PI macro in a separate polyphase
translation unit. It is a numerical control, not a replacement of the original
source result or a fitted filter. Original results are retained in full.

Against the original PCM16 WAV, the original Real spatial decoder on this core
still has worst-channel RMS 1.087456865e-5 and max error 5.794106983e-5.
FVid's preceding whole-chain measurement was RMS 1.087456455e-5 and max error
5.794526078e-5. Thus the remaining waveform residual is reproduced by the
original spatial implementation on the same core input. Changing spatial math
to fit that WAV is not supported by this evidence. Original AAC core arithmetic
and the reference-generation procedure remain to be checked independently.
This does not establish an AAC core defect or certify the entire profile.

Complex mode remains a valid separate renderer control; the original WAV uses
Real mode and is not a Complex-mode acceptance target. Both original mode
outputs and their full WAV measurements are retained in the JSON report.

No new production parsing, decoding, timestamp or seek failure is established
by this experiment. No production math, media fixtures or fixture golds change.
The earlier production regression/device gates are not claimed as newly run.

## Reproduction

Build and run the owned core-isolation procedure documented in
`aac-ld-mps-core-isolation-2026-11-01.md`, retaining both full owned outputs.
Run the explicit offline audit (a C compiler is required):

```sh
python3 scripts/validate_aac_ld_mps_original_spatial.py \
  --archive "$ARCHIVE" --records "$RECORDS" --syntax "$SYNTAX" \
  --core-pcm "$CORE_PCM" --owned-real "$OWNED_REAL" \
  --owned-complex "$OWNED_COMPLEX" --reference-wav "$ORIGINAL_WAV" \
  --output-dir "$RESEARCH_OUTPUT" --report "$REPORT"
```

The output directory must be separate from all inputs and repository fixtures.
Compilation and decoding logs, output PCM, input/source/tool hashes and complete
per-channel comparisons are preserved. Fixture generation and ordinary tests
do not call this tool, FFmpeg or network services.
