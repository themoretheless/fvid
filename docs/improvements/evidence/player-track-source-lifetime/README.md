# Selected audio source lifetime across track handoff

Accepted FVID-0472 after all nine gates and twenty repetitions passed. FVID-0472 is one source-admission/lifetime problem;rename,direction,pause,path-replacement and memory-backed variants add0IDs.

An authored1.5s VP9 video companion and600ms dual-PCM audio-only MP4 use exact scalar YUV/F32 oracles. A second valid audio-only file has deliberately different scalar samples under the same geometry. Both versions decode independently in the source control. No private media,codec parameter copies,encoder,FFmpeg or network. Generation is separate from ordinary tests.

Ordinary tests create uniquely named owned temporary directories and copy fixture bytes. They rename the open pathname or install the other authored file at that pathname. Fixture files are never modified. The active native audio-only reader is the last strong owner of its SharedFile descriptor. Closing it before opening a selected track loses an unlinked source or silently reopens another inode. Selected PCM prefixes and full native EOF suffixes distinguish those outcomes from metadata-only success.

A separate native MP4 stream on Cursor owns valid data but no registered file descriptor. A missing pathname must refuse new source admission without invoking the output factory or disposing of that playing decoder;the old stream must finish with exact original PCM. This passing refusal/continuation test is distinct from the successful moved-source playback acceptances. A no-live-source control must refuse without creating any device. Successful handoff independently checks the old backend has dropped before the new backend factory runs,preventing a naive device-overlap workaround.

The fix admits a reader before stopping the current handle,captures the latest transport phase,then joins the old device and installs the already admitted source. Metadata/worker installation is a private extraction with a byte-verified unchanged body(installer-equivalence.json). No public interface change or extra ID. This transaction covers source admission;it does not claim recovery of the previous device after a later asynchronous decoder/device initialization failure.

The initial audio-only stand accidentally inherited Default video buffering. It is preserved separately and excluded;the final stand explicitly matches open_sound_only with buffering=false. No assertion is removed or relaxed. Every temporary directory is removed by an ownership guard after Player drops.

Checks use real native decoders/audio workers and headless player state with controlled output. No physical audio,new native-window duration,hardware-codec or performance claim. The existing viewport-close limitation remains in ../player-ab-clock/native-smoke/finding.json.

Final identical eight-case baseline:3pass5fail,candidate:8pass0fail. Twenty repetitions pass160case executions. All9gates pass1623physical-source-distinct checks and both builds;five old ignores unchanged. The one initial stand correction is preserved in stand-admission.json and is not a media defect or separate improvement. Original open_shared weak-registry semantics and exclusive output ordering remain intact.
