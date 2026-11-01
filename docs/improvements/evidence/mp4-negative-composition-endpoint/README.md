# FVID-0407 follow-up: signed composition endpoint

Count delta: zero. This is another variant of the existing unedited native
composition-endpoint correction, not an independent new improvement.

The final nine authored files contain three independently decodable 16x16 VP9
keyframes at 50Hz. CTTS version1 shifts -2,-1,0,+1 cover finite and all-ones
unknown movie/track durations. An additional -1 case explicitly declares five
media ticks against three indexed decode ticks, preserving a stated empty tail.
The manifest's `cases` fields identify these header/tail controls; its compact
provenance sentence describes the original finite-header witnesses.

The unchanged final test baseline passes exact signed sample metadata and all
nine decoded YUV controls, but fails duration: both known and unknown -2/-1
cases report60ms instead of20/40ms. Actual native sequential playback, EOF,
rewind, seeks within the final frame and past EOF also run in this test.
The four original witnesses/baseline are retained separately.

The first unaccepted candidate removed the decode-duration minimum only in the
finite-header cap. It passed the original six related suites but truncated an
existing two-moof stream from120ms to60ms. Its source, full failure log and
results are retained. No test expectation was weakened.

The final candidate distinguishes indexed decode coverage from a genuine
stated tail. An indexed decode endpoint cannot impose a longer composition
minimum; explicitly declared time beyond indexed decoding remains intact.
The same adjusted minimum protects appended fragments against stale init
headers. Lazy caching, raw media timestamps and exact pixel decoding remain.
There is no new allocation, public interface change or performance claim.

Ordinary tests read committed fixtures and need no generator, encoder, FFmpeg
or network. The generator is separate and exact offline regeneration is checked.
Qualification runs actual native software decoding and seeking. No physical GPU,
audio-device or new native-window duration qualification is claimed.
