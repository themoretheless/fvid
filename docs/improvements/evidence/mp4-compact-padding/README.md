# Compact sample-size padding: uncounted FVID-0432 follow-up

ISO/IEC14496-12:2015 section8.7.3.3 (PDF page62) requires zero padding for an incomplete final byte of4-bit sample-size entries:
https://b.goeswhere.com/ISO_IEC_14496-12_2015.pdf#page=62
No standard prose or source implementation is copied. The initial FVID-0432 parser ignores those unused low bits; originale4ca760ec5 passes1 functional control and fails1 precise refusal test. This is an introduced format-integrity gap, so it adds no ID.

Two synthetic corrupted movies change only one byte of the earlier authored three-caption control, retaining its meaningful high nibble and every media packet/clock. Manifest byte locations demonstrate the intended difference. Four valid controls preserve odd4-bit zero padding, even4-bit nonzero meaningful low nibbles, and odd8/16-bit records whose final low bits are actual sample-size data. Both parsers read exact text packets; all valid videos decode the three exact16x16 pictures/intervals/EOF and perform actual native caption seeks at exact cue clocks. The ordinary regression and separate Python generator require no FFmpeg or network; all media and parameters remain authored.

After exact table-length validation, reject a nonzero unused low nibble only for compact4-bit odd-count tables. Existing size/packet budgets and field-width rules remain. All2 padding and6 parent format tests pass in the focused check; wider qualification is green. No performance or physical-device claim. The parent-owned target is reused only while this follow-up is actively qualifying, then removed together with the archived raw logs and staging backup.

Qualification:12 integration suites pass70 checks, root MP4 passes57 and owned-media547. Exclude3 common included clock units:671 distinct passes. Eight focused repeats add no count. Locked/offline core/player checks pass. One physical Metal/VideoToolbox test and the existing pending RGB compatibility acceptance remain ignored. All23 new files regenerate identically; all58 parent files are unchanged. Raw log hashes precede trailing whitespace/blank-line normalization in archived copies.
