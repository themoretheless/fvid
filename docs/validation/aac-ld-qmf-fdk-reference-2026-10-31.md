# Independent external LD-QMF cross-check

This optional reference workflow is separate from production and ordinary tests.
It requires Python3.12+,CMake,a C++11 clang++ compiler,and the owned native probe.
It has been executed on macOS/arm64. It does not claim platform or whole-codec
conformance. FFmpeg/libav are not used.

Fetch references separately:

```sh
python3 scripts/fetch_iso_ld_mps_conformance.py /tmp/fvid-iso-references
curl --fail --location 'https://android.googlesource.com/platform/external/aac/+archive/ee76460efbdb147e26d804c798949c23f174460b.tar.gz' --output /tmp/fvid-fdk-reference.tar.gz
```

The [pinned primary AOSP source](https://android.googlesource.com/platform/external/aac/+/ee76460efbdb147e26d804c798949c23f174460b/)
archive SHA-256 must be
`4f41fba1e55dc554ccd0166f550d7c0c83ae0f22d7f2ed76001537f4b62f7325`.
The validator checks this and the previously qualified ISO member hashes before
building. It generates external build metadata without editing decoder math.

Build the native probe and run the offline cross-check:

```sh
cargo build --offline --no-default-features --features player --example aac_spatial_reference
python3 scripts/validate_aac_ld_mps_fdk_reference.py \
  --source-archive /tmp/fvid-fdk-reference.tar.gz \
  --vectors-dir /tmp/fvid-iso-references \
  --native-probe target/debug/examples/aac_spatial_reference \
  --output-dir /tmp/fvid-fdk-crosscheck
```

For a custom Cargo target directory,pass its actual example path. Output contains
reference logs,raw PCM and `report.json`;reference source/build scratch is
removed when the run ends. Normal tests never fetch,compile or execute FDK.

`aac_spatial_reference packets INPUT.mp4 OUTPUT.records` writes `FVIDAAC1`,
u32le ASC length,ASC,u32le AU count,then u32le-length/AU pairs. The external
driver checks the header,length limits,whole-packet consumption,decoder status
and EOF. Decoder errors remain failures; they are not replaced with accepted PCM.

Observed results for `er_eld_ldmps_ldqmf_2`:

| Comparison | Entire stream RMS | RMS after frame2048 |
| --- | ---: | ---: |
| FDK vs ISO PCM16 WAV | 0.096575389 | 0.097453408 |
| FVid float vs FDK PCM16 | 0.001770325 | 0.000013925 |

All comparisons use the same sample positions and channel order,without a
delay/phase search. Full startup errors are retained. The later windows are
diagnostic measurements,not a normative permission to ignore an initial region.
FDK is an external fixed-point implementation,not a normative complete-codec
oracle. Its metadata reports outputDelay256 for both selected inputs;this does
not by itself justify shifting another decoder's output by256 samples.

Pinned FDK's320-window is the adjacent-pair interpolation of its640-window
within single-precision table rounding. After accounting for the published
table arrangement/reversal,the native640 numbers differ by at most5e-12 and
the320 derived numbers by3e-8. Neither these facts nor the cross-check resolve
the ISO reference's synthesis twiddle/coupling convention. The original ISO
`sac_dec.c` can read external fi/beta twiddles for the QMF synthesis interface;
the correct reference procedure still needs independent confirmation.

No production phase/window correction follows from this experiment. A confirmed
decoder defect still requires an authored short reproducer and enabled playback/
seek acceptance. Public ISO media,PCM and codec parameter sets are not copied
into repository fixtures;only provenance and numerical observations are recorded.
