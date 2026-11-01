# Original AAC-LD core diagnostic, 2026-11-01

Two original AAC-LD controls were checked against their original signed PCM24
WAVs. Native probe is the unchanged production build from3468160247. The new
optional mono core inspector reads bounded FVIDAAC1 records and reports frame
size,LD window shape,PNS codebooks,LTP and TNS. It does not interpret ancillary
or spatial suffixes and is not a full-packet conformance oracle.

## Measurements and interpretation

| Original input | AU geometry | PNS AUs | Full-stream RMS | RMS without current/previous PNS |
| --- | --- | ---: | ---: | ---: |
| er_ad1020np_48_ep0 | 3336×512,48kHz | 2448 | 0.01116802226 | 3.401759382e-8 |
| er_ad1120np_24_ep0 | 1779×480,24kHz | 0 | 3.455342368e-8 | 3.455342368e-8 |

The48kHz diagnostic partition contains341504 scalars (667 complete AUs),max
error8.940696716e-8. Its remaining1366528 scalars have RMS0.01248576064,max
error0.4320798218. At24kHz,all853920 scalars are compared;max error1.341104507e-7.
Both sine and low-overlap windows occur. Neither core input uses LTP.

The large48kHz waveform difference is confined to the PNS-dependent portion.
AAC PNS codebook13 uses signed random vectors normalized to the signalled
spectral energy;the specification does not prescribe one unique PRNG/seed
sequence. ISO/IEC14496-3:2009,4.6.13.3,printed201–202,defines this procedure.
The owned generator uses a deterministic xorshift sequence and the required
squared spectral norm2^(noise_energy/2). Raw waveform identity between distinct
valid noise realizations is therefore not itself a codec acceptance condition.
This observation does not certify PNS statistics,correlation or whole-profile
conformance. It applies to AAC PNS,not SBR's fixed noise table/phase sequence.

Source PDF:
https://ossrs.io/lts/zh-cn/assets/files/ISO_IEC_14496-3-AAC-2009-90cda427e845cc176293bffee3f92f99.pdf
SHA256 `913b72c340ba90bc9f4fc3bbc2edad8005369076c78a51f147bbbbbe6b5cf20d`.

The partition is determined solely by parsed codebook13 in the current or
previous AU. Fixed2N LD overlap and absence of LTP/spatial filtering justify
that dependency boundary. It is a diagnostic,not an accuracy gate or permission
to discard failing samples:all full-stream measurements remain in the report.
The workflow refuses this partition for LTP/spatial input,where longer-lived
prediction/filter state would invalidate the assumption. No delay search,gain
fitting,trimming or decoder-math change occurs.

## Relation to the open LD5151 residual

The same inspector reads all2820 core prefixes of original
`er_ad_ldmps_tree_5151`:512 samples per core AU,no PNS,no LTP. Thus PNS does not
explain that input's much smaller full-output numerical residual. These bare
controls verify core behavior on additional original material;they do not
isolate the identical LD5151 downmix from the spatial renderer or close its
accuracy qualification. That comparison remains open.

No production defect is established by the new bare-control observations.
No decoder behavior,existing media fixtures or reference PCM is modified. The
preceding693-test production gate belongs to3468160247 and is not presented as
a fresh broad test run for this diagnostic addition.

## Reproduce outside ordinary tests

```sh
python3 scripts/fetch_iso_ld_mps_conformance.py --ld-core-controls /path/to/originals
cargo build --offline --no-default-features --features player \
  --example aac_spatial_reference --example aac_ld_core_inspect
python3 scripts/validate_aac_ld_core_original_pcm.py \
  --vectors-dir /path/to/originals \
  --inventory /path/to/mpeg4audio-conformance2ndEd_20230125.md5 \
  --native-probe /path/to/target/debug/examples/aac_spatial_reference \
  --inspect-probe /path/to/target/debug/examples/aac_ld_core_inspect \
  --output-dir /path/to/research-output \
  --mps-input /path/to/ld-mps-originals/er_ad_ldmps_tree_5151.mp4 \
  --report /path/to/report.json
```

`--mps-input` is optional and adds syntax evidence only. The default fetch mode
still obtains the four previous LD-MPS inputs;the new flag obtains exactly two
paired AAC-LD MP4/WAV controls. Fetch uses the pinned ISO inventory,member MD5
and ZIP CRC checks. The audit itself is offline,checks all supplied originals
before/after,and requires exact output rate,channels,frame/sample count. Original
media and syntax dumps remain external to synthetic repository fixtures.
Ordinary tests invoke neither fetch nor either reference workflow.

[Complete original metrics,input pins and syntax summaries](aac-ld-core-original-2026-11-01.json).
