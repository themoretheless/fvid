# AIFF/AIFC aligned storage padding — FVID-0514

Accepted in batch 525. One independent cause, ten positive codec/offset variants.

Apple AIFF 1.3, Sound Data Chunk and Block-Aligning Sound Data, defines COMM
numSampleFrames as valid audio and permits SSND soundData to extend to the end
of a storage block. Primary source:
https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-1.3.pdf
The SSND offset skips leading unused bytes; the nonzero blockSize explicitly
signals storage alignment. The sentinel tail is deliberately audible if decoded.

Original `32badf386d` passes the independent unblocked audio and neutral-video
control, and the malformed/limited-input controls. It fails the two acceptance
tests: int16-offset0 reports 2080 rather than 2051 frames, and the actual worker
emits an extra sample at 2051/8000 seconds. See baseline.log and baseline-source.json.
The fixture-specific audio failure is distinct from the companion VP9 video,
which AIFF/AIFC cannot contain. Neither fixture generation nor Rust tests need
FFmpeg, a network, private media or private codec parameters.

The fix uses checked declared-frame × coded-stride arithmetic for explicitly
block-aligned PCM/G711. The full stored sample run is charged before trimming.
Truncated declared audio still fails EOF; a genuinely incomplete channel frame
still fails parsing. Unblocked files and MACE keep their prior conventions.
PCM16 mono, PCM24 stereo, float32 stereo, ALaw mono and MuLaw stereo each have
offset0/3 variants, independent waveform oracles and a nonzero padding tail.
Raw/native-time seeks cover the 2048-frame packet boundary, EOF, MAX and rewind.
The actual worker captures exact native timestamps, PCM and flush anchors.

The candidate suite passes4/4. Five final gates pass81 Rust test executions,
zero ignored, and both core/player build checks. Three further repeats pass9
tests each. qualification-results.json contains exact commands and timings;
all logs, including initial before failures, are retained. All2425 hashes in
candidate-source.json stayed unchanged throughout qualification. Regeneration
reproduces all24 fixture files byte-for-byte. No physical output device, visual
UI or performance improvement is claimed.
