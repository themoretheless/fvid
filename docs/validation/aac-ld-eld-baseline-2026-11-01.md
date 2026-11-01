# AAC LD/ELD baseline regression result

The90-target run finished with exit0: 393 passed, zero failed or
ignored tests. It built on26b1d066fd and includes the indexed22.2 changes,
combined LD-SBR/RVLC/HCR/TNS and the prior local-replacement implementation.
The adjacent JSON records every target, command, elapsed time and log digest.

During execution,11646c5f45 added prefix-preserving inherited retransmission.
Its21-test qualification used a separate temporary executable, preserving
the baseline executables. Its four device runs pass16 playback/navigation
phases. This baseline is not claimed as a complete-current-tree run of that
new implementation. Five negative replacement MP4s and their packet blob
were updated to valid prefix-losing replacements while the baseline ran; the
old prefix-preserving pattern now has acceptance in the new45-programme corpus.
See `aac-ld-mps-retransmission-2026-11-01.md` for exact semantics and evidence.

A passing refusal or component test does not establish playback acceptance.
Unequal HRTF-grid processing, EBQ combinations outside the qualified trees,
inherited prefix loss and other codec/profile combinations remain open.
This result is regression evidence, not completion of the codec-gap goal.
