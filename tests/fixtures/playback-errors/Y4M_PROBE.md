# Y4M probe dispatch regression

Regenerate `y4m-truncated-frame.y4m` with
`python3 scripts/generate_y4m440_fixture.py`; no external codec is used.
The synthetic 2x2 YUV420 frame contains five of its required six bytes.
`y4m_probe_dispatch` checks the specific owned truncation error through public
probe entrypoints, including a build with the temporary legacy feature.
Unknown signatures remain a non-selection, not a Y4M parse failure.
This is malformed-input refusal coverage, not playback acceptance.
