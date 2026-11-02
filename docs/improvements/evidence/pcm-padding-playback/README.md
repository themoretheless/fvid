# Batch536 — native playback extension of FVID-0208,zero new IDs

The nonzero WAVE unused-bit failure was already counted in FVID-0208 for owned decode/export/loudness paths. This batch closes the same gap in native RIFF/WAVE and AVI readers,including newly supported AVI extensible PCM. Accepted count stays525,next FVID-0526.

## Original behavior and specific reproduction

Original production commit `f0cfb6c71405b32875be65ed228fc776d2cd332a` admits all36 authored WAVE/AVI files with nonzero unused bits and decodes all4099 native frames from each. `before.log` records all36 accepted names,not merely the first failing case. The same test stand passes two independent controls:exact good PCM/video/rewind,then72 actual-worker seek/EOF sessions. The sole failing check is the missing Invalid refusal with a padding-specific error. `before-source.json` and `before-results.json` preserve source hashes and exit101;both production adapters match the original commit.

The [WAVEFORMATEXTENSIBLE reference](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ksmedia/ns-ksmedia-waveformatextensible) describes valid precision left-aligned within storage with zero unused low bits. Existing FVID-0208/native processing already uses a padding-specific refusal policy. The player now applies that policy before publishing any PCM;it never discards nonzero bits silently.

## Fix and compatibility

The existing crate-private format validator carries a padding count alongside PCM geometry and speaker mask. A bounded stack-based helper inspects little-endian stored sample words. RIFF validates its admitted sample run before copying it to playable data;AVI validates each already framed physical PCM record before constructing a ready reader. No additional payload copy or heap index is introduced by this check. All reduced-precision data is scanned at admission,including final-record violations,so seeking cannot bypass the check.

Full-width integer,float and other codec paths have padding count0 and keep their existing behavior. The same nonzero low bits are explicitly tested as valid when the header declares full storage precision. Valid PCM bytes,geometry,timestamps,speaker masks,packet windows,seek/EOF and rewind are unchanged. External signatures and public Pcm construction are unchanged;the tuple change is crate-private.

Files declaring unused low bits while actually using them are now rejected as Invalid. Content intended to retain those bits must correctly declare full storage precision. This is an intentional strict-admission extension of the existing owned-processing policy. Reduced-precision opening now includes a full validation scan;no performance improvement,whole-reader memory reduction or physical-output claim is made.

## Authored media and qualification

`scripts/generate_pcm_padding_playback.py` creates analytic signed/unsigned8/16/24/32-bit PCM,including1-bit valid precision. Six profiles each produce a zero-padding declaration and a full-width low-bit control,with independent normalized float oracles and RIFF/AVI wrappers. Eighteen deliberately inconsistent declarations place a nonzero padding bit in the last channel of the first,middle-window-boundary or final frame;both wrappers give36 refusal cases. The companion three-frame16x16 authored VP9 MP4 has exact picture-plane controls. No private media,codec parameter copies,encoder or network are used. Generation stays separate from ordinary tests.

The new three-test suite checks24 successful files and36 exact refusals.72 real AudioPlayback worker sessions preserve native PCM,anchors and clock suffixes at start,after a window boundary and at EOF/MAX. Successful playback and correct refusal remain separate tests. Nine gates pass212 Rust executions,including original FVID-0208 public API regressions,AVI extended/classic formats,predictive-coded history,root readers,workers and owned Wave checks. Both core/player builds pass. Two existing hardware smoke tests remain ignored;none are newly ignored or weakened. Physical audio hardware and AVI video presentation are unqualified.

Three repeated three-test runs pass. The compiled suite also passes with nonexistent PATH/FFMPEG/FFPROBE. All77 fixture files regenerate byte-identically. `candidate-source.json` freezes3104 paths,stable throughout;commands and terminal statuses are in `qualification-results.json`. Complete logs and fixture hashes are preserved here. The registry adds this regression to FVID-0208 rather than creating another ID.
