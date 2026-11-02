# QuickTime G.711 native decoding, presentation and export

Candidate for one independent compatibility improvement,not one ID per law,
container layout,clock,edit,test or harness repair. Original source is
7c9462c78ca2e2cfb395a209e44955a9af4ff5f3. It has existing all-code G.711
wave/AVI/AU/AIFC decoding,but drops MOV `alaw`/`ulaw` before their audio tables
are built. Video and independent WAVE controls pass;MOV player/worker refuses
with `MP4 audio track coded alaw has no decoder here`,and owned export finds
no selected audio stream. `baseline.log` covers the initial18 movies;
`baseline-expanded.log` covers48 movies. Each is one pass,three failures.

The new capability routes existing Rust companders through the MP4 parser,
player,native description,owned float32/float64 export and public planning/WAVE
publication. Native packet counts use one encoded byte per channel,independent
of the description's16-bit uncompressed sample size. Sample-addressable tables
retain compact index budgeting;no law reconstruction arithmetic was changed.

The final stand covers48 authored v1/compressionID-2 movies:both laws,8k mono,
48k/44.1k stereo,dense/uniform/fragmented/hybrid tables,with/without aligned empty
and repeated media edits. It checks all256 codes against an independent segment
level oracle and WAVE decoder,exact rational PCM/sample timestamps,Duration seek
including native boundaries/EOF/MAX,raw reset,actual worker packets/flush anchors,
float32/float64 intervals,canonical public audio descriptors/plans and exact
float64 WAVE publication. Short neutral VP9 video remains playable in every
positive movie. The orphan-channel MOV is a specific refusal control,not a
successful playback fixture. A valid packet with a too-small byte budget refuses
before output;uniform768-frame audio costs exactly one index record under a16
record limit.

`candidate-expanded.log` preserves an incomplete native-description dispatch
and a stand-only mistake:SampleIndex.len is logical frame count,not allocated
record count. The corrected guard checks cost==1,len==768 with the same limit16.
`candidate-public.log` preserves a second stand-only mistake:the established
linear-PCM publication path deliberately writes float64 WAVE,not float32.
The final test requires float64 and compares every emitted byte against exactly
widened independent PCM levels. These repairs add zero IDs and do not change
old tests or the existing output precision policy.

`candidate-final.log`:seven passes. This focused result alone is not full
acceptance;all qualification gates,source hashes and repeats must pass first.
`regeneration.json` proves68 identical files and49 movies whose mvhd duration
covers every tkhd. Generation is standalone pure Python;normal tests read
committed fixtures,never generate media or require FFmpeg/network. Public export
tests own and remove their unique temporary directories,including on failure.
No private media,frames,audio or codec parameter sets are used.

Primary format references:
- https://developer.apple.com/documentation/quicktime-file-format/sound_sample_descriptions
- https://developer.apple.com/documentation/quicktime-file-format/sound_sample_description_version_1

The qualified corpus uses v1 redefined sample tables and zero optional packet
geometry fields. Other legacy table conventions and v2 descriptions are not
qualified by this stand. No speedup,physical audio-device,or visual-player claim.

Final acceptance: FVID-0513,batch524,one new ID. Initial qualification exposed
an old AIFC regression;that failure and frozen source are retained as initial-*
evidence. The unmodified original source reproduces the same failure. The
standards correction of FVID-0193 is separately documented and adds zero IDs.
Final nine gates pass1114 Rust executions,four Python fixture units,two old
ignores and both builds;three15-test repeats pass. Counter513,next FVID-0514.
