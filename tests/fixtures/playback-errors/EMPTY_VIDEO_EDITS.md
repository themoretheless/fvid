# Empty MP4 video edits

`edit-empty-spans.mov` is generated exclusively from the checked-in synthetic
`control.mp4` seed by `scripts/generate_playback_error_samples.py`.
Its rate-one edit list contains a leading one-second empty edit, two seconds of
media starting at zero, another one-second empty edit, and the same media range
again. Compressed payloads and parameter sets are synthetic.

The native MP4 input presentation test accepts two explicit blank events and
two appearances of source frame zero on the six-second movie clock. It also
checks the bounded plan and B-picture ordering with the existing I/P/B fixture.
This is acceptance of timeline planning, not hardware-rendered black pixels or
completed production export. The existing playback mapper's refusal for an
interior empty edit remains a separate capability expectation.

Generation is separate from test execution and requires no FFmpeg/network.
