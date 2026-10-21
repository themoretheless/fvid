# Legacy nonseparate 4:4:4 PCM and compressed acceptance

The manifest retains 132 progressive CABAC and MBAFF CAVLC/CABAC PCM cases.
Its historical field-signalled level-5.1 material remains compatibility coverage;
it is not a normative field-stream conformance claim. Reauthor separately with
`scripts/generate_avc_444_pcm.py`; ordinary offline tests use `tests/avc_444_pcm.rs`.

`accepted.json` now contains a valid progressive compressed CABAC example with
two complete macroblocks and all three component residuals. It replaces a
historical refusal sample that contained only luma syntax and incomplete frame
coverage. The positive test requires full native decode and software MP4 output.
`accepted-mbaff.json` replaces four historical MBAFF refusals with complete,
valid level-4.1 shared I16 residual syntax, independent component samples and
native/software MP4 acceptance. The original witnesses included incomplete or
non-4:4:4 syntax; their old malformed bytes are not claimed to decode. The legacy
basenames retain `crop0`, but the new SPS and metadata explicitly crop [1,1,2,2].
`unsupported.json` is empty. The broader `avc-444-mbaff-intra` suite covers
360 PCM/I16/I4/I8 cases; nonseparate inter is outside this acceptance scope.

Reauthoring on 2026-10-21 verified 542 other legacy artifacts byte for byte.
All parameter sets, samples and writers are owned synthetic data. No private
sources, FFmpeg or network are required for generation or ordinary tests.
