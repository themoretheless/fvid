# ISO LD-SBR admission and reconstructed noise-range investigation

Historical strict-default audit. The current bounded Q31 policy closes these
six refusals;see [current acceptance](aac-ld-sbr-q31-2026-10-31.md). The original
checksums,strict failures and source findings below are retained as evidence.

A separate audit covers all26 `er_eld_sbr_*` compressed MP4 inputs from the
original ISO checksum inventory.20 decode completely through the normal audio
reader with presented,finite PCM. Six inverse-filter inputs fail with exactly
`SBR noise level exceeds normative range: 31`:
`er_eld_sbr_i_{1,2}_{32,44,48}.mp4`.
This remains failed strict admission, not six new passing conformance cases.

The earlier numerical spatial probe refuses edited tracks before decoding.
All26 have edits,so those initial probe refusals were not codec failures. The
fresh `audio_probe` audit instead uses the ordinary MP4 playback reader,edits
and source timestamps. It does not claim hardware output,independent sample
accuracy or reference phase/timing qualification.

## Authoritative conflict and current policy

ISO/IEC14496-3:2009 section4.6.18.3.6,printed page237 constrains reconstructed
uncoupled noise levels and coupled level-channel noise to0..30. Its five-bit
absolute syntax alone can express31; syntax representability is insufficient
to establish a valid reconstructed noise value. The directly observed value in
each of the six ISO inputs is31. The LD-SBR modifications in section4.6.19 do
not specify a widened noise range. The diagnostic now includes the actual value.
No production range is widened and no parsed parameter is silently clamped.

Full AAC source PDF SHA256:
`913b72c340ba90bc9f4fc3bbc2edad8005369076c78a51f147bbbbbe6b5cf20d`.
Source: <https://ossrs.io/lts/zh-cn/assets/files/ISO_IEC_14496-3-AAC-2009-90cda427e845cc176293bffee3f92f99.pdf>.
The conformance standard identifies `er_eld_sbr_i` as inverse-filter tests
(ISO/IEC14496-26:2010,Table77); it is not thereby proof of every encoded
noise value satisfying the older normative requirement. Applicable edition,
bitstream provenance and intended range/error handling need further resolution.

Independently pinned original AOSP FDK takes a different robustness policy:
`limitNoiseLevels` clamps reconstructed noise to0..35 before storing the last
noise envelope. That protection is documented as bounding delta accumulation;
it does not establish0..35 as the normative range. Source:
<https://android.googlesource.com/platform/external/aac/+/ee76460efbdb147e26d804c798949c23f174460b/libSBRdec/src/env_dec.cpp>.
Do not change the native formula/limits merely to match this foreign policy.
An independent run of that original pinned FDK decodes every AU of all six
inputs,without concealment/flush flags or replacement output on any decoder
error. This is raw decoder admission,not waveform conformance or edited output.
The six inputs contain602/402/402 AUs per channel-configuration family. Evidence:
`aac-eld-sbr-q31-fdk-admission-2026-10-31.json`.

For this check,`aac_spatial_reference packets` now explicitly exports original
ASC/AUs even when container edits are present. It reports ignored edit count;
raw compressed records contain no presented PCM/timeline. An authored edited
AAC control and its copy with only the enclosing edit-box tag changed to `free`
produce byte-identical records. Numerical Real/Complex PCM modes still refuse
edited input,and constant-description admission still refuses actual changes.
The exporter does not rewrite compressed data,apply priming or fit delays.

This investigation neither proves malformed original ISO media nor fixes those
six playback failures. Their acceptance and waveform qualification remain open.

## Own regressions

`aac-eld-sbr-noise-range` contains two eight-AU stereo480/512 controls with
inverse-filter modes0/1/2/3,dual-rate output and CRC. Their packets equal the
existing authored `aac-eld-sbr` inverse-filter controls and their PCM uses the
same saved independent direct-convolution reference. Both MP4 and Matroska
controls pass PCM,playback,live-history seek,rewind and timestamp checks.

Each has an authored counterexample changing only fourth-frame noise absolute
values from7 to31 and recomputing CRC. Both containers reach the exact Q31
error. Repeated late rejection followed by valid retry preserves core,QMF,
noise,chirp/CRC and coefficient histories,including subsequent frames and reset.
These are refusal/reproduction tests,not enabled acceptance for the intended
original ISO playback. No private/ISO media,frames,audio or codec parameters
are copied. Generation is deterministic across two runs with sockets and child
processes disabled. Ordinary tests require no generators,FFmpeg or network.

Scoped checks:601 pass,0 fail,1 pre-existing ignored (`fvid-media --lib`,
`aac_eld_sbr`,new range tests and the new audio-worker navigation test).
Evidence: `aac-eld-sbr-noise-range-tests-2026-10-31.json`.

```sh
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline -p fvid-media --lib
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline --no-default-features --features player --test aac_eld_sbr_noise_range --test aac_eld_sbr
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline --no-default-features --features player --lib inverse_filter_noise_range_controls_keep_worker_audio_and_navigation
```

Use `CARGO_TARGET_DIR=../target` for this worktree. The independent FDK build
uses the [previous pinned reference procedure](aac-ld-qmf-fdk-reference-2026-10-31.md).
Export a local original MP4 with `aac_spatial_reference packets INPUT OUTPUT.records`
and pass those records to the original `aac_ld_mps_fdk_reference_probe` driver.

## Reproduction

Build the current `audio_probe` with the player feature. The optional validator
fetches only explicitly selected original members by bounded HTTP ranges and
verifies the pinned official checksum inventory before decoding. Source media
must stay outside synthetic fixtures. Existing inputs support offline reruns:

```sh
python3 scripts/validate_iso_eld_admission.py --suite eld-sbr --inputs /tmp/fvid-iso-eld-sbr --probe ../target/debug/examples/audio_probe --report /tmp/fvid-iso-eld-sbr/report.json --fetch
# Repeat the same command without --fetch for a fully offline audit.
```

The validator exits1 for these six failures. The saved report
`aac-eld-sbr-iso-admission-2026-10-31.json` retains all26 statuses,stdout/stderr,
checksums and a completion marker; no expected-failure waiver is applied.
A separate `--suite eld-core` covers the128 non-MPS/non-LD-SBR ELD inputs.
All128 decode completely in both the discovery run and a second offline run
using one frozen `audio_probe` executable. The second run uses four independent
processes;per-file wall times are not performance benchmarks. All input MD5s
match the pinned inventory and every file has finite,presented output. This
qualifies decoder admission,not sample-accurate reference PCM or device playback.
Evidence: `aac-eld-core-iso-admission-2026-10-31.json`.

```sh
python3 scripts/validate_iso_eld_admission.py --suite eld-core --jobs 4 --inputs /tmp/fvid-iso-eld-core --probe ../target/debug/examples/audio_probe --report /tmp/fvid-iso-eld-core/report.json --fetch
```

As with the LD-SBR suite,omit `--fetch` for a local-only rerun. Complete reports
have a true completion marker;partial files are not a complete qualification.
