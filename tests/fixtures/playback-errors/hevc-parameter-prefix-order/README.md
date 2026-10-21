# HEVC replacement PPS before its new SPS

Generate with `python3 scripts/generate_hevc_parameter_prefix_order.py`.
The generator remuxes existing owned synthetic `hevc-pcm-reference-rext8`
and `hevc-scc-act-slice-y10-c10` samples. It never invokes FFmpeg, an external
codec or the network, and never reads private input. Tests consume saved files;
generation is separate.

Six acceptance movies place replacement parameters in the order VPS/SPS/PPS,
VPS/PPS/SPS or PPS/VPS/SPS, either before the new IDR in its sample or in one
parameter-only sample. VPS still precedes SPS. There are actual old-sequence
pictures before the transition, not merely stale unused configuration bytes.

The new ACT PPS is valid with the new interleaved 4:4:4 SPS and invalid with the
old SPS carrying the same ID. The regression asserts both parser controls.
Before the fix, `32-34-33-inline` failed at the transition with
`HEVC ACT requires interleaved 4:4:4`; the ordinary-order controls decoded the
same coded samples. This is contextual parameter reception, not missing ACT
reconstruction or an entropy error.

Acceptance compares all old/new Y/Cb/Cr samples against their existing YUV
oracles, sample indices, PTS, duration, EOF, rewind and seek to the new IDR.
The parameter-only sample extends the final old picture to 80 ms; other
pictures last 40 ms. Both public native constructors also decode the complete
sequence twice with reset and verify that slice-header queries leave parameter
state unchanged. The reader tests force the owned software path.

H.265 7.4.2.4.2 defines parameter availability before activation and applies
cross-parameter constraints to active sets. Its changed-VPS ordering constraint
is preserved by every acceptance layout:
https://www.itu.int/rec/T-REC-H.265-202601-I/en

This covers replacement parameters completed in one packet prefix. It does not
claim arbitrary reception across separate packets or relax malformed parameter,
post-picture update, random-access, or no-update guidance checks.
