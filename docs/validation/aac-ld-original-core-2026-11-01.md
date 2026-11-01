# Independent original AAC-LD core and complete LD5151 chain

The independent comparison now includes original AAC decoding, not only the
spatial renderer on owned core PCM. All 2820 isolated core AUs are decoded by
the original ISO AAC-LD implementation and generic decoder frontend from the
pinned SAOC reference archive. The entire stream contains 1443840 mono samples
at 48kHz, with 512 samples/AU, no PNS and no LTP.

## Original implementation and adapters

Archive:
https://standards.iso.org/iso-iec/23003/-2/ed-2/en/SAOC_IS7_ReferenceSoftware__20160603.zip

SHA256 `b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`.

The nested `audio.zip` contains the `mp4AudVm_Rewrite` source and original Unix
makefiles. The explicit research builder uses their selection of 118 decoder
translation units, adds the original generic `mp4ifc.c` frontend, and builds
the original epTool library. Archived AAC syntax, transform, dequantization,
prediction, windowing and TNS sources are not edited or substituted.

The build selects AAC-LD, not ELD. Original LPC and parametric dummy modules
remain unselected by this mono AAC input. SBR compilation is enabled because
the legacy generic transport structures require those fields; these core AUs
do not contain SBR. The external MP4 library is unnecessary: its transport
constructor is supplied by an explicit refusal adapter, while the original
generic decoder frontend reads LATM normally. The old GNU-sed linker-path
helper fails on BSD sed, so the script executes the compile commands selected
by `make -n` and links the resulting objects directly. No foreign decoder enters
production, fixtures or ordinary tests.

The isolated records are the pinned transform established in
`aac-ld-mps-core-isolation-2026-11-01.md`. The research LATM wrapper retains all
core AU bytes and carries the exact 22-bit core ASC, omitting only its disabled
spatial sync extension and padding. This avoids a legacy transport issue: the
original ASC parser skips unconsumed declared bits, and its LATM caller skips
them again. No AAC parser workaround or production change is applied. The
legacy `.mp4\nASS\0` bit-file header is outside the LOAS stream and is not an
MP4 container. This input is a source-specific research transform, not an
unchanged original file or a repository acceptance fixture.

The float capture wraps only `AudioWriteData`: it records the original mono
float samples divided by32768, then invokes the unchanged original file writer.
A second executable uses the original file writer without capture. Their full
PCM16 AU files are byte-identical; sample rate, channel count, encoding and
whole-stream sample count are checked. The captured PCM is taken before
integer file rounding. Process exit alone is insufficient: warnings/errors,
missing EOF or incorrect output geometry cause the audit to fail.

## Measurements

| Comparison | Worst-channel RMS | Maximum absolute difference |
| --- | ---: | ---: |
| Original AAC-LD float core vs owned core | 1.503630779e-9 | 2.980232239e-8 |
| Original core + original Real spatial chain vs FVid | 1.743758784e-8 | 4.842877388e-7 |
| Original core + original Complex spatial chain vs FVid | 1.786426530e-8 | 4.768371582e-7 |
| Original core + precise-PI Real spatial control vs FVid | 1.141244622e-8 | 2.309679985e-7 |
| Original core + precise-PI Complex spatial control vs FVid | 1.251531622e-8 | 1.923181117e-7 |

Core comparison covers every scalar. Each complete chain comparison covers all
8663040 interleaved output scalars. No gain fitting, delay search, trimming or
noise-dependent exclusions are used. Source and tool hashes and full per-channel
measurements are in the two JSON reports.

Even the complete original Real chain retains worst-channel RMS
1.087456673e-5 and max error5.794269964e-5 against the original PCM16 WAV.
The formerly open residual is thus reproduced independently by both original
core and original spatial code; it does not establish a defect in FVid's core
or spatial math on this input. Exact historical reference-generation settings
or software revision remain unverified. This is not a whole-profile conformance
certificate and does not close other unsupported codec tools/profiles.

The original AAC frontend documents PCM16 as its default file output. A separate
research control rounding the owned core at that boundary reduced the WAV
residual but did not eliminate it (worst-channel RMS8.16e-6, max4.79e-5).
That observation is insufficient to identify the historical generation path;
no such rounding is added to FVid's production float output.

No new production media failure is established, and no production arithmetic,
synthetic fixtures or fixture golds are changed. The preceding production
regression/device results are not presented as a fresh test run here.

## Reproduction outside ordinary tests

After the existing core-isolation procedure:

```sh
python3 scripts/validate_aac_ld_original_core.py \
  --archive "$ARCHIVE" --core-records "$CORE_RECORDS" \
  --owned-core "$OWNED_CORE_PCM" --output-dir "$CORE_RESEARCH_OUTPUT" \
  --report "$CORE_REPORT"
```

Then run `validate_aac_ld_mps_original_spatial.py` as documented in its audit,
using `$CORE_RESEARCH_OUTPUT/original-core.f32le` for `--core-pcm`. The resulting
`aac-ld-original-chain-2026-11-01.json` compares the complete independent chain
with FVid and retains its measurements against the historical WAV.
