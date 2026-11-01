# FVID-0484 source-gap clock follow-up (zero new IDs)

Two authored AAC960/neutral VP9 videos expose 1ms of independent coded PCM followed by 1ms of a declared source gap, at 48kHz and 50Hz media scales. Public raw sample timing proves the gap: first packet PTS0/duration delta, second packet PTS2delta. The complete wave matches the existing owned synthetic reference slice plus literal zero silence. All three native video frames and EOF are checked.

The published aa587246f7 source passes this control. `before-final.log` fails two acceptances: direct stream navigation and the actual AudioPlayback backend flush. Six exact gap seeks use requested positions rather than the native silence lattice; two additional worker EOF positions remain1ns early despite no emitted PCM. All PCM slices and first output PTS already pass. `before.log` is an initial test-stand compile error from attempting to call a private helper; the final test instead checks public exact metadata. It adds no ID.

The change aligns exact non-recovered AAC edit gap anchors on the same lattice used to emit gap silence, bounded by both the gap and edit end. It keeps silence playable instead of snapping to the next coded packet. Recovered clocks and raw packet seeking retain their previous behavior. This is a conservative zero-count extension of FVID-0484, not another independently counted cause.

`after-focused.log` passes all13 new/current seek regressions. All seven final gates and three repeated thirteen-test runs pass. No performance or physical audio-device claim is made; the backend recorder observes the real worker's flush anchor and exact PCM without a hardware device.

Accepted in batch-482-validation.json with zero new IDs. All processes were terminal before deleting the owned build target.
