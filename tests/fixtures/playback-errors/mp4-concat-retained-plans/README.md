# Retained MP4 concat timing tables

Generated offline by `scripts/generate_mp4_concat_retained_plans.py` from owned AAC pulse syntax. One mono 48 kHz track has two packets, the other 512 (10.923 s); there is no private media or copied codec parameter set.

`tests/mp4_concat_retained_plans.rs` distinguishes resource refusal from acceptance:

* Two segments selecting track 0 must reject 40 KiB because timing tables for track 1 remain allocated.
* The identical input and selection with 2 MiB must succeed.
* Writing must emit four selected packets, and owned AAC decoding must produce PCM.

The pre-fix regression admitted the tight budget. Counts use the capacity of every retained `PacketTime` vector rather than the number of output-selected packets. This is controlled-memory admission, not a physical RSS claim. Fixture generation is separate from test execution; neither needs FFmpeg or a network.
