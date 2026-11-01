# Already-admitted Matroska audio counted through metadata

Accepted FVID-0478. Both FLAC and Opus sources decode correctly and unbounded
concat plans succeed. A sufficient packet-work bound nevertheless causes the
counter to reopen source information through the restricted legacy AAC/PCM
exporter. The first source is rejected as unsupported,despite the configured
library decoder having already admitted it.

Final identical three-test baseline:two controls pass,one acceptance fails in
all four format/memory variants. The published counter and native selection
source were restored for before-published.log. The standalone counter candidate
was present in the library but is not called by these Matroska cases.

A private header-only audio index now shares the existing preferred/enabled
track selection. Work counting uses this index on its already-bounded reader.
The legacy exporter still applies its original codec gate after selection.
This adds no codec support and does not loosen parser or decoder validation.

Final candidate passes all three. Actual FLAC/Opus decoding,unbounded controls,
controlled plans and publication of both complete waveforms are verified.
Legacy codec refusals,zero limits and tight memory remain passing refusals.
The80ms neutral VP9/FLAC control displays both exact pictures,seeks to60ms,
rewinds and reaches EOF;its FLAC packet bytes match the audio-only witness.

An initial assumption about public MP4 EAC3 PCM export was not confirmed by
source admission. Its unaccepted stand input snapshots,generator and failure
log are preserved separately. Its zero oracle was an unverified expectation,
not accepted PCM evidence. That hypothesis and the stand correction add0IDs;
no EAC3 export acceptance is claimed in this change.

The generator separately authors short media and independent PCM. Ordinary
tests need no generation,FFmpeg or network. Real native software decoding and
file publication are exercised;no physical audio/GPU,new window-duration or
performance qualification is claimed.
