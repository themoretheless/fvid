# Correct original AIFC sample width without weakening coded-frame checks

Zero-count correction of the AIFC part of FVID-0193,batch186. The WAV extensible
and AU presentation repairs remain valid;their existing test bodies are byte
identical. The old8-only interpretation confused original sample precision with
coded G711 bytes. Apple AIFF-C section4.0 defines the metadata before compression
in1..32 bits. One coded byte per channel continues to determine actual stride.
Primary reference and the short17-word excerpt are retained in the companion
mp4-g711-seek/aifc-field-contract.json. No complete copyrighted document is copied.

The unchanged old g711_native test fails on both the original7c9462c78c source
and the MOV-only candidate with the same16-bit AIFC refusal. The original-source
run restores all nine temporarily saved candidate files and verifies their
hashes;see ../mp4-g711-seek/original-g711-native-baseline.json and its full log.
The new width stand has one passing8-bit/video control and three failures before
the AIFF correction. It passes all four tests afterward,and the old G711 test
passes unchanged. The MOV capability already passes independently before this
correction and adds just one independent ID,FVID-0513.

Sixty-four authored AIFC cases cover both laws,mono/stereo,lower/upper tags and
original widths16,1,8,12,13,14,24,32. Every coded value is checked against an
independent segment-level PCM oracle through complete playback,raw seeks,
rewind and actual playback-worker flush/packet capture. Bounds0,33,65535,
orphan channel data,truncated declared tail and physical data-byte limits have
specific passing refusal checks. Existing8-bit input remains an acceptance.

AIFC is audio-only. The authored short neutral-video companion is explicitly an
independent video control;the AIFC files reproduce the actual parser failure.
No claim is made that the video itself reproduces an AIFC parser error. Ordinary
tests read committed media and do not generate fixtures or use FFmpeg/network.

All seven original wav-aiff-au media files remain byte-identical. Their historic
invalid-named16/24-bit files are retained as positive controls;new0/33-bit files
provide correct negative controls. The updated old AIFC test additionally checks
actual decoded PCM against independent table fixtures and verifies coded seek
stride. No assertions are removed from unrelated WAV/AU tests. Legacy integrity,
regeneration and the four Python fixture units are retained here.

Qualification is shared with batch524:1114 Rust executions,four Python units,
two prior ignores,both builds,and three15-test repeats pass with2399 unchanged
source/fixture hashes. The counter advances only for MOV/G711 to513,next FVID-0514.
No speedup,physical audio-device or visual-player claim is made.
