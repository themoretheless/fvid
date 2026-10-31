# LD-MPS profile boundary

LD-MPS excludes residual coding. The authors state this in section3.3,page3 of
[AES8099](https://www.iis.fraunhofer.de/content/dam/iis/de/doc/ame/conference/AES-128-Convention_ANewParametricStereo-AndMultichannelExtension_AES8099.pdf).

ISO/IEC 14496-26:2010/Amd.3:2014, clauses 7.26.1.2.1.5 and 7.26.1.2.1.26,
confirm that configuration/frame extension types must be 2–15 (see the pinned
source audit). Types 0/1 are forbidden even with absent or zero residual bands.

Native AAC validates initial SSC and replacement headers. A residual extension
configuration returns an invalid-profile error; it is not a pending LD decoder
feature. Raw syntax and numerical components remain available separately.

Earlier residual64 synthetic playback was incorrectly interpreted as profile
qualification. Its mathematical results are retained,but its movies are negative
profile tests. Residual32/128 ignored LD acceptance was likewise invalid and has
been replaced with an executable refusal test.

Ordinary MPEG Surround/HQ residual decoding is a different integration and remains
unfinished. This boundary check does not prove all Annex B/profile constraints,
whole-codec conformance or completion of the no-codec-gaps goal.
